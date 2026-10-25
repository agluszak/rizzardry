//! Triggers: what using a prop, or another trigger naming it, sets off.
//!
//! After retail `Trigger::Run`, `SelectAction` and `CommitActionResult`.
//! A run selects an action (initial, alternate or fallback), lets the level
//! script veto it, performs it and, unless the action failed, commits:
//! the trigger's sound, its recipients in order (synchronously), its
//! location variable, its message and its `fired` flag. Effects that
//! presentation shows (sounds, camera shakes, messages) are returned, not
//! played here. See `docs/research/trigger-actions.md`.

use std::collections::HashMap;
use std::time::Duration;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::level::Level;
use super::props::{Prop, PropMotion};
use super::scripts::LevelScript;
use crate::content::items::{Dice, ItemCatalog, LootTables};
use crate::content::level::{
    Animation, Fountain, LevelGeometry, Restores, StateChange, TriggerAction, TriggerDefinition,
    TriggerUse,
};
use crate::game::characters::Character;
use crate::game::data::GameData;
use crate::game::inventory::{ItemIds, ItemInstance, PartyInventory, make_item};
use crate::game::loot::roll_loot;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::time::GameClock;

/// Damage zones hurt this often, in seconds.
const DAMAGE_ZONE_PERIOD: f32 = 0.5;

/// Recipients naming each other in a loop stop here.
const MAX_DEPTH: usize = 16;

/// The live state of a level's triggers and location variables, on the
/// level root. Triggers are indexed as in [`LevelGeometry::triggers`].
#[derive(Component, Clone, Debug)]
pub struct LevelState {
    triggers: Vec<TriggerState>,
    /// By name, for the level script; retail looks names up ignoring case,
    /// the newest trigger of a name winning.
    index: HashMap<String, usize>,
    /// Each container's loot table, by its place among the tables, resolved
    /// when the level is prepared. Two retail containers name tables there
    /// are none of; like retail (`GenerateItemGroup`), they generate
    /// nothing.
    loot: Vec<Option<usize>>,
    variables: HashMap<String, bool>,
    pub script: LevelScript,
    /// Whether the party has been seen in the level yet.
    entered: bool,
}

#[derive(Clone, Debug)]
pub struct TriggerState {
    /// Cleared to switch a trigger off for good (retail `ON`).
    pub on: bool,
    pub fired: bool,
    /// The next use runs the alternate action.
    use_alternate: bool,
    /// It waits for its prop's animation to finish (retail `RUNNING`).
    awaits_animation: bool,
    /// The dial position or door state (retail `state_index`).
    pub position: usize,
    /// A door's lock, until its key opens it.
    pub locked: bool,
    /// The party's eye is within its range (positioned triggers).
    inside: bool,
    /// A container's contents, rolled when it is first opened.
    pub contents: Option<Contents>,
    /// A damage zone hurting the party while it stands in it.
    zone: Option<Timer>,
    /// A fountain's uses left, and when (game milliseconds) it refills.
    fountain: Option<FountainState>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct FountainState {
    /// None for an endless fountain.
    left: Option<u32>,
    refills_at: Option<u64>,
}

/// What a level's triggers keep while the party is away (retail's `LOCK`
/// and `TRES` level state): each trigger's switches, position, lock,
/// contents and fountain, and the location variables. Proximity, animation
/// waits and damage zones belong to a visit and are not kept.
#[derive(Clone, Debug)]
pub struct TriggerMemory {
    triggers: Vec<KeptTrigger>,
    variables: HashMap<String, bool>,
}

#[derive(Clone, Debug)]
struct KeptTrigger {
    on: bool,
    fired: bool,
    use_alternate: bool,
    position: usize,
    locked: bool,
    contents: Option<Contents>,
    fountain: Option<FountainState>,
}

/// What a container holds.
#[derive(Clone, Debug, Default)]
pub struct Contents {
    pub items: Vec<ItemInstance>,
    pub gold: u32,
}

impl LevelState {
    /// A level's triggers as authored, their containers' loot tables
    /// looked up in `tables`.
    pub fn new(geometry: &LevelGeometry, script: LevelScript, tables: &LootTables) -> Self {
        let loot = geometry
            .triggers
            .iter()
            .map(|trigger| {
                let name = trigger.loot_table.as_ref()?;
                let table = tables.0.iter().position(|t| t.name.eq_ignore_ascii_case(name));
                if table.is_none() {
                    warn!("trigger {}: no loot table {name}; it holds nothing", trigger.name);
                }
                table
            })
            .collect();
        let mut state = LevelState {
            triggers: geometry
                .triggers
                .iter()
                .map(|trigger| TriggerState {
                    on: true,
                    fired: false,
                    use_alternate: false,
                    awaits_animation: false,
                    position: 0,
                    locked: trigger.door.is_some_and(|door| door.locked),
                    inside: false,
                    contents: None,
                    zone: None,
                    fountain: None,
                })
                .collect(),
            index: HashMap::new(),
            loot,
            variables: HashMap::new(),
            script,
            entered: false,
        };
        for (i, trigger) in geometry.triggers.iter().enumerate() {
            state.index.insert(trigger.name.to_uppercase(), i);
            if let (Some((name, _)), Some(value)) = (&trigger.state_change, trigger.initial_state) {
                state.variables.entry(name.clone()).or_insert(value);
            }
        }
        // A dial's position variables: `<state>0` is set, the rest clear.
        for prop in &geometry.props {
            let Some(definition) = prop.trigger.map(|index| &geometry.triggers[index]) else {
                continue;
            };
            if let (TriggerAction::Dial, Some((name, _))) =
                (&definition.initial.action, &definition.state_change)
            {
                for position in 0..prop.slots.len() {
                    state.variables.entry(format!("{name}{position}")).or_insert(position == 0);
                }
            }
        }
        state
    }

    /// What the triggers keep while the party is away.
    pub fn remember(&self) -> TriggerMemory {
        TriggerMemory {
            triggers: self
                .triggers
                .iter()
                .map(|t| KeptTrigger {
                    on: t.on,
                    fired: t.fired,
                    use_alternate: t.use_alternate,
                    position: t.position,
                    locked: t.locked,
                    contents: t.contents.clone(),
                    fountain: t.fountain,
                })
                .collect(),
            variables: self.variables.clone(),
        }
    }

    /// A visit's state for a level the party has been to: its triggers as
    /// they were kept, nothing in progress, and the party not seen yet.
    pub fn returning(
        geometry: &LevelGeometry,
        script: LevelScript,
        tables: &LootTables,
        memory: &TriggerMemory,
    ) -> Self {
        let mut state = LevelState::new(geometry, script, tables);
        for (trigger, kept) in state.triggers.iter_mut().zip(&memory.triggers) {
            trigger.on = kept.on;
            trigger.fired = kept.fired;
            trigger.use_alternate = kept.use_alternate;
            trigger.position = kept.position;
            trigger.locked = kept.locked;
            trigger.contents = kept.contents.clone();
            trigger.fountain = kept.fountain;
        }
        state.variables = memory.variables.clone();
        state
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.index.get(&name.to_uppercase()).copied()
    }

    pub fn trigger(&self, index: usize) -> &TriggerState {
        &self.triggers[index]
    }

    pub fn trigger_mut(&mut self, index: usize) -> &mut TriggerState {
        &mut self.triggers[index]
    }

    pub fn variable(&self, name: &str) -> bool {
        self.variables.get(&name.to_uppercase()).copied().unwrap_or(false)
    }

    pub fn set_variable(&mut self, name: &str, value: bool) {
        self.variables.insert(name.to_uppercase(), value);
    }
}

/// What a trigger's prop can do: move along its path.
pub struct TriggerProp<'a> {
    pub motion: &'a mut PropMotion,
    pub last_frame: f32,
    pub wraps: bool,
    pub slots: &'a [u16],
    /// Where its sounds play.
    pub position: Vec3,
}

impl TriggerProp<'_> {
    fn playing(&self) -> bool {
        self.motion.frame != self.motion.target
    }

    /// Retail `SetRepresentationActive(!playing)`: stop a playing animation,
    /// or play it: from the start for a wrapping one, otherwise reversing
    /// the last play.
    fn toggle(&mut self) {
        if self.playing() {
            self.motion.target = self.motion.frame;
        } else {
            self.start();
        }
    }

    fn start(&mut self) {
        if self.wraps {
            self.motion.frame = 0.0;
            self.motion.target = self.last_frame;
        } else {
            self.motion.target =
                if self.motion.frame >= self.last_frame { 0.0 } else { self.last_frame };
        }
    }

    /// Advance to the next segment between slots and return its index;
    /// the last slot only ends the last segment (retail
    /// `AdvanceAnimationSegment`).
    fn advance_segment(&mut self, current: usize) -> usize {
        if self.slots.len() < 3 {
            return 0;
        }
        let next = (current + 1) % (self.slots.len() - 1);
        self.motion.frame = f32::from(self.slots[next]);
        self.motion.target = f32::from(self.slots[next + 1]);
        next
    }
}

/// Something presentation shows after triggers ran.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum TriggerEffect {
    /// A retail sound file, at a place or heard everywhere.
    Sound {
        path: String,
        volume: f32,
        at: Option<Vec3>,
    },
    Shake {
        seconds: f32,
        intensity: f32,
        fade_out: bool,
    },
    /// A message from the level's message file.
    Message(i32),
    /// The party used something and nothing came of it.
    NothingHappened,
    /// A member was hurt.
    Hurt {
        member: String,
        amount: u32,
    },
    /// An item went to the party ("Party takes %s").
    PartyTakes(String),
    Gold(u32),
    Locked,
}

/// Running triggers: their definitions, live state and props.
pub struct Triggers<'a> {
    pub definitions: &'a [TriggerDefinition],
    pub state: &'a mut LevelState,
    /// Props by the index of the trigger they carry.
    pub props: HashMap<usize, TriggerProp<'a>>,
    pub effects: Vec<TriggerEffect>,
    /// The party, its possessions, items and randomness, for containers,
    /// keys, traps and fountains.
    pub party: PartyContext<'a>,
    /// Where a teleport or camera path sends the party.
    pub party_moves: Option<PartyMove>,
}

/// What containers, keys, traps and fountains need: item definitions, loot
/// tables, randomness, the party's possessions and its members in order.
pub struct PartyContext<'a> {
    pub members: Vec<&'a mut Character>,
    /// Game time, for things that refill.
    pub now_ms: u64,
    pub catalog: &'a ItemCatalog,
    pub loot: &'a LootTables,
    pub rng: &'a mut GameRng,
    pub ids: &'a mut ItemIds,
    pub party: &'a mut PartyInventory,
}

/// Moving the party somewhere, which the party module carries out.
#[derive(Clone, Debug)]
pub enum PartyMove {
    Teleport(crate::content::level::Arrival),
    /// Fly the party's eye along a camera path, by name (ladders, climbs).
    Fly(String),
}

impl<'a> Triggers<'a> {
    /// Whether the party may use a trigger at all (retail picking).
    pub fn usable(&self, index: usize) -> bool {
        let (definition, state) = (&self.definitions[index], &self.state.triggers[index]);
        definition.usable && !(definition.once && state.fired)
    }

    /// Run the positioned triggers the party's eye has just come within
    /// range of (retail `UpdateWorldTriggers`; retail reruns one each step
    /// while it is not running, here it runs on entering). The first time
    /// the party is seen in a level, triggers it stands in count as entered.
    pub fn walk(&mut self, eye: Vec3, elapsed: Duration) {
        let first = !self.state.entered;
        self.state.entered = true;
        for index in 0..self.definitions.len() {
            let Some(place) = self.definitions[index].place else { continue };
            let (near, far) = self.definitions[index].range;
            let inside = (near..far).contains(&eye.distance(place));
            let entering = inside && !self.state.triggers[index].inside;
            self.state.triggers[index].inside = inside;
            // A damage zone hurts every half second until the party leaves
            // (retail: a 1d6+2 timed action, ended by `FinishAction`).
            if !inside {
                self.state.triggers[index].zone = None;
            } else if let Some(zone) = &mut self.state.triggers[index].zone
                && zone.tick(elapsed).just_finished()
            {
                self.damage_party(Dice { count: 1, sides: 6, base: 2 });
            }
            if entering && !first && self.party_moves.is_none() && self.usable(index) {
                self.run_index(index, 0);
            }
        }
    }

    /// The party arrived at `eye`: triggers there count as entered, so an
    /// exit beside an entrance does not send it straight back.
    pub fn arrive(&mut self, eye: Vec3) {
        for (index, definition) in self.definitions.iter().enumerate() {
            if let Some(place) = definition.place {
                let (near, far) = definition.range;
                self.state.triggers[index].inside = (near..far).contains(&eye.distance(place));
            }
        }
    }

    /// Run a trigger by name. Returns whether its action committed.
    pub fn run(&mut self, name: &str) -> bool {
        match self.state.find(name) {
            Some(index) => self.run_index(index, 0),
            None => false,
        }
    }

    pub fn run_index(&mut self, index: usize, depth: usize) -> bool {
        if depth > MAX_DEPTH {
            warn!("trigger {} recurses too deeply", self.definitions[index].name);
            return false;
        }
        let definitions = self.definitions;
        let definition = &definitions[index];
        let Some(chosen) = self.select(index) else { return false };
        let script = self.state.script;
        if !script.before_run(&definition.name, self) {
            return false;
        }
        let mut changes_state = true;
        match &chosen.action {
            TriggerAction::Nothing => {}
            action @ (TriggerAction::OpenDoor | TriggerAction::ToggleDoor) => {
                // A lock its key in hand has not opened (retail lock picking
                // is not ported).
                if self.state.triggers[index].locked {
                    self.effects.push(TriggerEffect::Locked);
                    return false;
                }
                let Some(prop) = self.props.get_mut(&index) else { return false };
                if prop.playing() {
                    return false;
                }
                let state = &mut self.state.triggers[index];
                if *action == TriggerAction::OpenDoor {
                    if state.position == 1 {
                        return false;
                    }
                    prop.motion.target = prop.last_frame;
                    state.position = 1;
                } else {
                    prop.motion.target = if state.position == 0 { prop.last_frame } else { 0.0 };
                    state.position ^= 1;
                }
                state.awaits_animation = true;
            }
            TriggerAction::Animate(animation) => {
                self.open_container(index);
                let Some(prop) = self.props.get_mut(&index) else { return false };
                match animation {
                    Animation::Toggle => prop.toggle(),
                    Animation::Start if !prop.playing() => prop.start(),
                    Animation::Stop if prop.playing() => prop.motion.target = prop.motion.frame,
                    _ => {}
                }
                let state = &mut self.state.triggers[index];
                state.position ^= 1;
                state.awaits_animation = true;
            }
            TriggerAction::Sound => {
                if let Some(sound) = &definition.sound {
                    let at = self.props.get(&index).map(|prop| prop.position);
                    self.effects.push(TriggerEffect::Sound {
                        path: sound.path.clone(),
                        volume: sound.volume,
                        at,
                    });
                }
                return false;
            }
            TriggerAction::Relay => {
                if definition.recipients.is_empty() {
                    return false;
                }
                for &recipient in &definition.recipients {
                    self.run_index(recipient, depth + 1);
                }
            }
            TriggerAction::Teleport(arrival) => {
                self.party_moves = Some(PartyMove::Teleport(arrival.clone()));
            }
            TriggerAction::CameraPath(path) => {
                self.party_moves = Some(PartyMove::Fly(path.clone()));
            }
            TriggerAction::HurtParty(dice) => self.damage_party(*dice),
            TriggerAction::Fountain(fountain) => {
                if !self.use_fountain(index, fountain) {
                    return false;
                }
            }
            TriggerAction::DamageZone => {
                self.state.triggers[index].zone =
                    Some(Timer::from_seconds(DAMAGE_ZONE_PERIOD, TimerMode::Repeating));
            }
            &TriggerAction::Shake { seconds, intensity, fade_out } => {
                self.effects.push(TriggerEffect::Shake { seconds, intensity, fade_out });
            }
            TriggerAction::Dial => {
                let Some(prop) = self.props.get_mut(&index) else { return false };
                let old = self.state.triggers[index].position;
                let new = prop.advance_segment(old);
                self.state.triggers[index].position = new;
                if let Some((name, _)) = &definition.state_change {
                    self.state.set_variable(&format!("{name}{old}"), false);
                    self.state.set_variable(&format!("{name}{new}"), true);
                }
                changes_state = false;
            }
            TriggerAction::Unsupported(code) => {
                debug!("trigger {}: action {code:#x} is not implemented", definition.name);
                return false;
            }
        }
        self.commit(index, chosen.message, changes_state, depth);
        true
    }

    /// Drink from a fountain (retail actions 0x25-0x2b): each restores its
    /// amount to every member until it is dry, refilling some time after
    /// its first use. False when it is dry.
    fn use_fountain(&mut self, index: usize, fountain: &Fountain) -> bool {
        let now = self.party.now_ms;
        let state = self.state.triggers[index]
            .fountain
            .get_or_insert(FountainState { left: fountain.uses, refills_at: None });
        if state.refills_at.is_some_and(|at| at <= now) {
            *state = FountainState { left: fountain.uses, refills_at: None };
        }
        if let Some(left) = &mut state.left {
            if *left == 0 {
                return false;
            }
            *left -= 1;
            if state.refills_at.is_none() {
                state.refills_at = fountain.refills_after.map(|after| now + after);
            }
        }
        for member in self.party.members.iter_mut() {
            match fountain.restores {
                Restores::Health => member.heal(fountain.amount),
                Restores::Stamina => member.restore_stamina(fountain.amount),
            }
        }
        let at = self.props.get(&index).map(|prop| prop.position);
        self.effects.push(TriggerEffect::Sound {
            path: "data/sound/misc/fountain_magic.wav".into(),
            volume: 1.0,
            at,
        });
        true
    }

    /// Hurt each living member by their own roll (retail
    /// `ApplyRolledHealthChangeToParty`).
    fn damage_party(&mut self, dice: Dice) {
        let party = &mut self.party;
        for member in party.members.iter_mut().filter(|m| !m.is_dead()) {
            let amount = member.damage(party.rng.roll(dice));
            self.effects.push(TriggerEffect::Hurt { member: member.name.clone(), amount });
        }
    }

    /// Open a container, if the trigger is one: roll its contents the first
    /// time (retail `GenerateItemGroup`), then give the party its gold and
    /// as many items as the pool takes. Retail puts a single item in the
    /// hand and opens a picker for several; here all go to the pool.
    fn open_container(&mut self, index: usize) {
        let Some(table) = self.state.loot[index] else { return };
        let items = &mut self.party;
        let contents = self.state.triggers[index].contents.get_or_insert_with(|| {
            // The party's level once characters exist.
            let (found, gold) = roll_loot(&items.loot.0[table], items.catalog, 1, items.rng);
            let made = found
                .into_iter()
                .filter_map(|id| items.catalog.get(id))
                .map(|definition| make_item(definition, items.ids, items.rng, false, false))
                .collect();
            Contents { items: made, gold }
        });
        if contents.gold > 0 {
            items.party.gold += contents.gold;
            self.effects.push(TriggerEffect::Gold(contents.gold));
            contents.gold = 0;
        }
        let mut kept = Vec::new();
        for item in contents.items.drain(..) {
            let name = items.catalog.get(item.item).map(|d| d.name.clone()).unwrap_or_default();
            match items.party.place_in_pool(item, items.catalog) {
                Ok(()) => self.effects.push(TriggerEffect::PartyTakes(name)),
                Err(item) => kept.push(item),
            }
        }
        contents.items = kept;
    }

    /// Retail `SelectAction`: the use this run performs. Like retail, it
    /// moves on to the alternate as it selects, before the script or the
    /// action can still fail.
    fn select(&mut self, index: usize) -> Option<&'a TriggerUse> {
        let definitions = self.definitions;
        let definition = &definitions[index];
        let running = self.state.triggers[index].awaits_animation
            && self.props.get(&index).is_some_and(TriggerProp::playing);
        let state = &self.state.triggers[index];
        let shakes = matches!(definition.initial.action, TriggerAction::Shake { .. });
        if !state.on || (definition.once && state.fired) || (running && !shakes) {
            return None;
        }
        if !definition.required_states.iter().all(|name| self.state.variable(name)) {
            return definition.fallback.as_ref();
        }
        // A locked door with a key opens for the key in hand, which
        // unlocks it for good; without it the fallback runs.
        let state = &mut self.state.triggers[index];
        if state.locked
            && let Some(key) = definition.door.and_then(|door| door.key)
        {
            if self.party.party.held.as_ref().is_some_and(|held| held.item.item == key) {
                state.locked = false;
            } else {
                return definition.fallback.as_ref();
            }
        }
        match &definition.alternate {
            Some(alternate) if state.use_alternate => {
                state.use_alternate = !definition.alternate_toggles;
                Some(alternate)
            }
            alternate => {
                state.use_alternate = alternate.is_some();
                Some(&definition.initial)
            }
        }
    }

    /// Retail `CommitActionResult`.
    fn commit(&mut self, index: usize, message: Option<i32>, changes_state: bool, depth: usize) {
        let definitions = self.definitions;
        let definition = &definitions[index];
        if let Some(sound) = definition.sound.as_ref().filter(|_| definition.sound_on_commit) {
            let at = self.props.get(&index).map(|prop| prop.position);
            self.effects.push(TriggerEffect::Sound {
                path: sound.path.clone(),
                volume: sound.volume,
                at,
            });
        }
        if definition.fire_linked {
            for &recipient in &definition.recipients {
                self.run_index(recipient, depth + 1);
            }
        }
        if changes_state && let Some((name, change)) = &definition.state_change {
            let value = match change {
                StateChange::Set => true,
                StateChange::Clear => false,
                StateChange::Toggle => !self.state.variable(name),
            };
            self.state.set_variable(name, value);
        }
        if let Some(message) = message {
            self.effects.push(TriggerEffect::Message(message));
        }
        self.state.triggers[index].fired = true;
    }
}

/// The current level's triggers, ready to run.
#[derive(SystemParam)]
pub struct LevelTriggers<'w, 's> {
    levels: Query<'w, 's, (&'static Level, &'static mut LevelState)>,
    geometry: Res<'w, Assets<LevelGeometry>>,
    props: Query<'w, 's, (&'static Prop, &'static mut PropMotion)>,
    effects: MessageWriter<'w, TriggerEffect>,
    party_move: ResMut<'w, PendingPartyMove>,
    data: Option<Res<'w, GameData>>,
    catalogs: Res<'w, Assets<ItemCatalog>>,
    loot: Res<'w, Assets<LootTables>>,
    rng: ResMut<'w, GameRng>,
    ids: ResMut<'w, ItemIds>,
    party: ResMut<'w, PartyInventory>,
    members: Query<'w, 's, &'static mut Character>,
    roster: Res<'w, PartyRoster>,
    clock: Res<'w, GameClock>,
}

/// A trigger has sent the party somewhere; the party module moves it.
#[derive(Resource, Debug, Default)]
pub struct PendingPartyMove(pub Option<PartyMove>);

impl LevelTriggers<'_, '_> {
    /// Where a trigger has sent the party, if anywhere; taking it clears it.
    pub fn take_party_move(&mut self) -> Option<PartyMove> {
        self.party_move.0.take()
    }

    /// Run triggers with `run`, then send their effects to presentation.
    /// Nothing runs without an active level and the shared databases, which
    /// its activation guarantees: a trigger never runs half its action.
    pub fn with<R>(&mut self, run: impl FnOnce(&mut Triggers) -> R) -> Option<R> {
        let Ok((level, state)) = self.levels.single_mut() else { return None };
        let geometry = self.geometry.get(&level.geometry)?;
        let data = self.data.as_ref()?;
        let (catalog, loot) = (self.catalogs.get(&data.items)?, self.loot.get(&data.loot)?);
        let props = self
            .props
            .iter_mut()
            .filter_map(|(prop, motion)| {
                let prop_trigger = TriggerProp {
                    motion: motion.into_inner(),
                    last_frame: prop.last_frame as f32,
                    wraps: prop.wraps,
                    slots: &prop.slots,
                    position: prop.centre,
                };
                Some((prop.trigger?, prop_trigger))
            })
            .collect();
        // The party's members, in its order: order-sensitive rolls (each
        // member's trap damage) follow it.
        let members = self
            .members
            .iter_many_unique_mut(self.roster.0.iter())
            .filter_map(Result::ok)
            .map(Mut::into_inner)
            .collect();
        let mut triggers = Triggers {
            definitions: &geometry.triggers,
            state: state.into_inner(),
            props,
            effects: Vec::new(),
            party: PartyContext {
                members,
                now_ms: self.clock.ms,
                catalog,
                loot,
                rng: self.rng.as_mut(),
                ids: self.ids.as_mut(),
                party: self.party.as_mut(),
            },
            party_moves: None,
        };
        let result = run(&mut triggers);
        self.effects.write_batch(triggers.effects);
        if let Some(arrival) = triggers.party_moves {
            self.party_move.0 = Some(arrival);
        }
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::items::{ItemDefinition, ItemId, LootTable};
    use crate::content::level::{Arrival, DoorLock, PropModel, TriggerSound};
    use crate::game::characters::{Gender, Profession, Race};
    use crate::game::inventory::Held;

    fn act(action: TriggerAction) -> TriggerUse {
        TriggerUse { action, message: None }
    }

    fn trigger(name: &str, action: TriggerAction) -> TriggerDefinition {
        TriggerDefinition {
            name: name.into(),
            usable: false,
            once: false,
            fire_linked: false,
            initial: act(action),
            alternate: None,
            fallback: None,
            alternate_toggles: false,
            recipients: vec![],
            sound: None,
            sound_on_commit: false,
            required_states: vec![],
            state_change: None,
            initial_state: None,
            range: (0.0, 10.0),
            door: None,
            place: None,
            loot_table: None,
        }
    }

    /// Point each trigger's recipients at the named ones, as the decoder
    /// resolves them.
    fn link(triggers: &mut [TriggerDefinition], links: &[(&str, &[&str])]) {
        let index = |name: &str| triggers.iter().position(|t| t.name == name).unwrap();
        let resolved: Vec<(usize, Vec<usize>)> = links
            .iter()
            .map(|(from, to)| (index(from), to.iter().map(|t| index(t)).collect()))
            .collect();
        for (from, to) in resolved {
            triggers[from].recipients = to;
        }
    }

    /// A prop running `trigger`: its frames to the end, whether each play
    /// restarts, and its dial slots.
    struct PropSpec<'a>(&'a str, f32, bool, &'a [u16]);

    /// The party and the databases triggers act on.
    struct Party {
        catalog: ItemCatalog,
        loot: LootTables,
        rng: GameRng,
        ids: ItemIds,
        inventory: PartyInventory,
        members: Vec<Character>,
    }

    impl Party {
        fn new(members: usize) -> Self {
            let member = |n| Character::new(n, Race::Human, Gender::Male, Profession::Fighter);
            Party {
                catalog: ItemCatalog::default(),
                loot: LootTables::default(),
                rng: GameRng::seeded(5),
                ids: ItemIds::default(),
                inventory: PartyInventory::default(),
                members: ["A", "B", "C"].into_iter().take(members).map(member).collect(),
            }
        }

        fn context(&mut self, now_ms: u64) -> PartyContext<'_> {
            PartyContext {
                members: self.members.iter_mut().collect(),
                now_ms,
                catalog: &self.catalog,
                loot: &self.loot,
                rng: &mut self.rng,
                ids: &mut self.ids,
                party: &mut self.inventory,
            }
        }
    }

    /// A level's triggers with their props held still or finished.
    struct Level {
        geometry: LevelGeometry,
        state: LevelState,
        last_frames: Vec<f32>,
        motions: Vec<PropMotion>,
        party: Party,
    }

    impl Level {
        fn new(triggers: Vec<TriggerDefinition>, props: &[PropSpec], script: LevelScript) -> Self {
            let index = |name: &str| triggers.iter().position(|t| t.name == name).unwrap();
            let models = props
                .iter()
                .map(|&PropSpec(trigger, _, wraps, slots)| PropModel {
                    name: trigger.to_lowercase(),
                    collidable: false,
                    face_camera: false,
                    frames_per_second: 10.0,
                    parts: vec![],
                    wraps,
                    slots: slots.to_vec(),
                    trigger: Some(index(trigger)),
                })
                .collect();
            let last_frames = props.iter().map(|p| p.1).collect();
            let motions = props.iter().map(|_| PropMotion::default()).collect();
            let geometry = LevelGeometry { triggers, props: models, ..default() };
            let party = Party::new(1);
            let state = LevelState::new(&geometry, script, &party.loot);
            Level { geometry, state, last_frames, motions, party }
        }

        fn triggers(&mut self, now_ms: u64) -> Triggers<'_> {
            let props = self
                .geometry
                .props
                .iter()
                .zip(&mut self.motions)
                .zip(&self.last_frames)
                .map(|((model, motion), &last_frame)| {
                    let prop = TriggerProp {
                        motion,
                        last_frame,
                        wraps: model.wraps,
                        slots: &model.slots,
                        position: Vec3::ZERO,
                    };
                    (model.trigger.unwrap(), prop)
                })
                .collect();
            Triggers {
                definitions: &self.geometry.triggers,
                state: &mut self.state,
                props,
                effects: vec![],
                party: self.party.context(now_ms),
                party_moves: None,
            }
        }

        /// Run a trigger as the party; returns whether it committed and the
        /// effects.
        fn run(&mut self, name: &str) -> (bool, Vec<TriggerEffect>) {
            let mut triggers = self.triggers(0);
            let committed = triggers.run(name);
            (committed, triggers.effects)
        }

        fn motion(&self, trigger: &str) -> &PropMotion {
            let index = self.state.find(trigger);
            let prop = self.geometry.props.iter().position(|p| p.trigger == index);
            &self.motions[prop.unwrap()]
        }

        /// Let every animation finish.
        fn settle(&mut self) {
            for motion in &mut self.motions {
                motion.frame = motion.target;
            }
        }
    }

    #[test]
    fn a_door_opens_then_toggles_and_a_locked_one_stays_shut() {
        let mut door = trigger("DOOR", TriggerAction::OpenDoor);
        door.alternate = Some(act(TriggerAction::ToggleDoor));
        let mut locked = trigger("LOCKED", TriggerAction::OpenDoor);
        locked.door = Some(DoorLock { locked: true, key: None });
        let mut level = Level::new(
            vec![door, locked],
            &[PropSpec("DOOR", 15.0, false, &[]), PropSpec("LOCKED", 15.0, false, &[])],
            LevelScript::None,
        );
        assert!(level.run("DOOR").0);
        assert_eq!(level.motion("DOOR").target, 15.0);
        // Busy until the door finishes opening.
        assert!(!level.run("DOOR").0);
        level.settle();
        assert!(level.run("DOOR").0);
        assert_eq!(level.motion("DOOR").target, 0.0);
        level.settle();
        assert!(level.run("DOOR").0);
        assert_eq!(level.motion("DOOR").target, 15.0);
        assert_eq!(level.run("LOCKED"), (false, vec![TriggerEffect::Locked]));
        assert_eq!(level.motion("LOCKED").target, 0.0);
    }

    #[test]
    fn a_key_in_hand_unlocks_its_door_for_good_and_otherwise_the_fallback_runs() {
        let key = ItemId(7);
        let mut door = trigger("DOOR", TriggerAction::OpenDoor);
        door.door = Some(DoorLock { locked: true, key: Some(key) });
        door.fallback = Some(TriggerUse { action: TriggerAction::Nothing, message: Some(3) });
        let mut level =
            Level::new(vec![door], &[PropSpec("DOOR", 15.0, false, &[])], LevelScript::None);
        // Without the key: the fallback, which shows its message.
        assert_eq!(level.run("DOOR"), (true, vec![TriggerEffect::Message(3)]));
        assert_eq!(level.motion("DOOR").target, 0.0);
        let definition = ItemDefinition { id: key, ..default() };
        let item = make_item(&definition, &mut level.party.ids, &mut level.party.rng, false, true);
        level.party.inventory.held = Some(Held { item, origin: None });
        assert!(level.run("DOOR").0);
        assert_eq!(level.motion("DOOR").target, 15.0);
        // Unlocked for good: kept while the party is away.
        let memory = level.state.remember();
        let tables = LootTables::default();
        let back = LevelState::returning(&level.geometry, LevelScript::None, &tables, &memory);
        assert!(!back.trigger(0).locked);
    }

    #[test]
    fn a_container_opens_its_table_once_and_one_without_a_table_holds_nothing() {
        let chest = |name: &str, table: &str| {
            let mut chest = trigger(name, TriggerAction::Animate(Animation::Toggle));
            chest.loot_table = Some(table.into());
            chest
        };
        let props = [PropSpec("CHEST", 5.0, false, &[]), PropSpec("LOST", 5.0, false, &[])];
        let mut level = Level::new(
            vec![chest("CHEST", "Monastery Chest"), chest("LOST", "No Such Table")],
            &props,
            LevelScript::None,
        );
        // The tables are looked up when the level is prepared.
        level.party.loot = LootTables(vec![LootTable {
            name: "MONASTERY CHEST".into(),
            entries: vec![],
            level_scaled: false,
            item_count: Dice::default(),
            gold: Dice { count: 0, sides: 0, base: 5 },
        }]);
        level.state = LevelState::new(&level.geometry, LevelScript::None, &level.party.loot);
        assert_eq!(level.run("CHEST"), (true, vec![TriggerEffect::Gold(5)]));
        level.settle();
        assert_eq!(level.run("CHEST"), (true, vec![]), "its gold was taken");
        assert_eq!(level.run("LOST"), (true, vec![]));
        let lost = level.state.find("LOST").unwrap();
        assert!(level.state.trigger(lost).contents.is_none(), "no contents are made up");
        assert_eq!(level.party.inventory.gold, 5);
    }

    #[test]
    fn sounds_play_on_commit_only_when_marked_and_always_for_the_sound_action() {
        let sound = TriggerSound { path: "data/sound/click.wav".into(), volume: 1.0 };
        let mut quiet = trigger("QUIET", TriggerAction::Nothing);
        quiet.sound = Some(sound.clone());
        let mut loud = trigger("LOUD", TriggerAction::Nothing);
        loud.sound = Some(sound.clone());
        loud.sound_on_commit = true;
        let mut only = trigger("ONLY", TriggerAction::Sound);
        only.sound = Some(sound);
        let mut level = Level::new(vec![quiet, loud, only], &[], LevelScript::None);
        let click =
            TriggerEffect::Sound { path: "data/sound/click.wav".into(), volume: 1.0, at: None };
        assert_eq!(level.run("QUIET"), (true, vec![]));
        assert_eq!(level.run("LOUD"), (true, vec![click.clone()]));
        assert_eq!(level.run("ONLY"), (false, vec![click]), "plays without committing");
    }

    /// The upper monastery's bells, as authored (`docs/research/trigger-actions.md`).
    fn bells() -> Level {
        let toggle = || TriggerAction::Animate(Animation::Toggle);
        let mut dial = trigger("BELLRINGSWITCH", TriggerAction::Dial);
        dial.usable = true;
        dial.state_change = Some(("RING".into(), StateChange::Set));
        dial.initial_state = Some(false);
        let mut button = trigger("BELL_BUTTON", toggle());
        button.usable = true;
        button.fire_linked = true;
        let mut windows = trigger("BELLTRIGGER01", TriggerAction::Nothing);
        windows.once = true;
        windows.fire_linked = true;
        windows.required_states = vec!["RING2".into()];
        let relay = |name: &str, ring: &str| {
            let mut relay = trigger(name, TriggerAction::Relay);
            relay.fire_linked = true;
            relay.required_states = vec![ring.into()];
            relay
        };
        let shake = TriggerAction::Shake { seconds: 2.4, intensity: 90.0, fade_out: true };
        let mut triggers = vec![
            dial,
            button,
            windows,
            relay("BELLTRIGGERC", "RING0"),
            relay("BELL_ACTIVATE_2", "RING1"),
            relay("BELL_ACTIVATE_3", "RING2"),
            trigger("BELL_1", toggle()),
            trigger("BELL_2", toggle()),
            trigger("BELL_3", toggle()),
            trigger("GLASS1", toggle()),
            trigger("MONACAMSHAKE", shake),
        ];
        link(
            &mut triggers,
            &[
                (
                    "BELL_BUTTON",
                    &["BELLTRIGGER01", "BELLTRIGGERC", "BELL_ACTIVATE_2", "BELL_ACTIVATE_3"],
                ),
                ("BELLTRIGGER01", &["GLASS1", "MONACAMSHAKE"]),
                ("BELLTRIGGERC", &["BELL_1"]),
                ("BELL_ACTIVATE_2", &["BELL_2"]),
                ("BELL_ACTIVATE_3", &["BELL_3", "MONACAMSHAKE"]),
            ],
        );
        Level::new(
            triggers,
            &[
                PropSpec("BELLRINGSWITCH", 20.0, false, &[0, 1, 10, 20]),
                PropSpec("BELL_BUTTON", 11.0, false, &[]),
                PropSpec("BELL_1", 36.0, true, &[]),
                PropSpec("BELL_2", 36.0, true, &[]),
                PropSpec("BELL_3", 36.0, true, &[]),
                PropSpec("GLASS1", 2.0, false, &[]),
            ],
            LevelScript::Monastery2,
        )
    }

    fn bell_sound(n: u32) -> TriggerEffect {
        TriggerEffect::Sound {
            path: format!("data/sound/ambients/mon2bell{n}.wav"),
            volume: 1.0,
            at: None,
        }
    }

    #[test]
    fn each_dial_position_rings_its_own_bell() {
        let mut level = bells();
        assert!(level.state.variable("RING0"));
        let (committed, effects) = level.run("BELL_BUTTON");
        assert!(committed);
        assert_eq!(effects, [bell_sound(1)]);
        assert_eq!(level.motion("BELL_1").target, 36.0);
        assert_eq!(level.motion("BELL_2").target, 0.0);
        level.settle();
        assert!(level.run("BELLRINGSWITCH").0);
        assert!(!level.state.variable("RING0") && level.state.variable("RING1"));
        assert_eq!(
            (level.motion("BELLRINGSWITCH").frame, level.motion("BELLRINGSWITCH").target),
            (1.0, 10.0)
        );
        level.settle();
        assert_eq!(level.run("BELL_BUTTON").1, [bell_sound(2)]);
        assert_eq!(level.motion("BELL_2").target, 36.0);
        assert_eq!(level.motion("GLASS1").target, 0.0, "the windows hold until the third bell");
    }

    #[test]
    fn the_third_bell_breaks_the_windows_once_and_ends_the_puzzle() {
        let mut level = bells();
        level.run("BELLRINGSWITCH");
        level.settle();
        level.run("BELLRINGSWITCH");
        level.settle();
        let (committed, effects) = level.run("BELL_BUTTON");
        assert!(committed);
        assert_eq!(effects[0], bell_sound(3));
        let shake = TriggerEffect::Shake { seconds: 2.4, intensity: 90.0, fade_out: true };
        assert!(effects.contains(&shake));
        assert_eq!(level.motion("GLASS1").target, 2.0);
        assert_eq!(level.motion("BELL_3").target, 36.0);
        level.settle();
        // Dial and button are off for good; the windows trigger has fired.
        assert_eq!(level.run("BELL_BUTTON"), (false, vec![]));
        assert_eq!(level.run("BELLRINGSWITCH"), (false, vec![]));
        let windows = level.state.find("BELLTRIGGER01").unwrap();
        assert!(level.state.trigger(windows).fired);
    }

    #[test]
    fn a_swinging_bell_is_not_restarted() {
        let mut level = bells();
        level.run("BELL_BUTTON");
        level.motions.iter_mut().for_each(|m| m.frame = (m.target / 2.0).floor());
        // The button finished; the bell is halfway.
        let button = level.state.find("BELL_BUTTON");
        let button = level.geometry.props.iter().position(|p| p.trigger == button).unwrap();
        level.motions[button].frame = level.motions[button].target;
        let bell = level.motion("BELL_1").frame;
        assert_eq!(level.run("BELL_BUTTON").1, [bell_sound(1)]);
        assert_eq!(level.motion("BELL_1").frame, bell);
        assert_eq!(level.motion("BELL_1").target, 36.0);
    }

    #[test]
    fn walk_in_triggers_fire_on_entering_when_usable() {
        let teleport = |name: &str, x: f32, usable: bool| {
            let arrival = Arrival::Entrance { entrance: format!("{name}00"), ask: false };
            let mut trigger = trigger(name, TriggerAction::Teleport(arrival));
            trigger.usable = usable;
            trigger.place = Some(Vec3::new(x, 0.0, 0.0));
            trigger.range = (0.0, 2.0);
            trigger
        };
        let triggers = vec![
            teleport("START", 0.0, true),
            teleport("EXIT", 10.0, true),
            teleport("OFF", 20.0, false),
        ];
        let mut level = Level::new(triggers, &[], LevelScript::None);
        let mut walk = |eye: Vec3| {
            let mut triggers = level.triggers(0);
            triggers.walk(eye, Duration::from_millis(100));
            triggers.party_moves
        };
        // Standing in a trigger when the level is first seen does not fire it.
        assert!(walk(Vec3::ZERO).is_none());
        assert!(walk(Vec3::new(5.0, 0.0, 0.0)).is_none());
        assert!(matches!(
            walk(Vec3::new(10.5, 0.0, 0.0)),
            Some(PartyMove::Teleport(Arrival::Entrance { entrance, .. })) if entrance == "EXIT00"
        ));
        // Staying inside does not fire it again; a trigger the party may not
        // use never fires.
        assert!(walk(Vec3::new(10.0, 0.0, 0.0)).is_none());
        assert!(walk(Vec3::new(20.0, 0.0, 0.0)).is_none());
    }

    #[test]
    fn traps_hurt_each_member_by_their_own_roll_and_zones_hurt_while_stood_in() {
        let trap = trigger("TRAP", TriggerAction::HurtParty(Dice { count: 2, sides: 4, base: 0 }));
        let mut zone = trigger("ACID", TriggerAction::DamageZone);
        zone.usable = true;
        zone.place = Some(Vec3::ZERO);
        zone.range = (0.0, 2.0);
        let mut level = Level::new(vec![trap, zone], &[], LevelScript::None);
        level.party = Party::new(2);
        let full = level.party.members[0].hp;
        let tick = Duration::from_millis(100);
        let mut triggers = level.triggers(0);
        assert!(triggers.run("TRAP"));
        let hurt: Vec<u32> = triggers
            .effects
            .iter()
            .filter_map(|e| match e {
                TriggerEffect::Hurt { amount, .. } => Some(*amount),
                _ => None,
            })
            .collect();
        assert_eq!(hurt.len(), 2);
        assert!(hurt.iter().all(|n| (2..=8).contains(n)));
        triggers.effects.clear();
        // Seen standing outside, then walking in: the zone starts; a second
        // inside hurts twice; leaving stops it.
        triggers.walk(Vec3::new(5.0, 0.0, 0.0), tick);
        triggers.walk(Vec3::ZERO, tick);
        for _ in 0..10 {
            triggers.walk(Vec3::ZERO, tick);
        }
        let zone_hits =
            triggers.effects.iter().filter(|e| matches!(e, TriggerEffect::Hurt { .. })).count();
        assert_eq!(zone_hits, 2 * 2, "two ticks for each of two members");
        triggers.walk(Vec3::new(5.0, 0.0, 0.0), tick);
        triggers.effects.clear();
        triggers.walk(Vec3::new(5.0, 0.0, 0.0), Duration::from_secs(1));
        assert!(triggers.effects.is_empty());
        assert!(level.party.members.iter().all(|m| m.hp < full));
    }

    #[test]
    fn fountains_restore_until_dry_and_refill_with_game_time() {
        let fountain = Fountain {
            restores: Restores::Health,
            amount: 5,
            uses: Some(1),
            refills_after: Some(2 * 720 * 1000),
        };
        let mut level = Level::new(
            vec![trigger("WELL", TriggerAction::Fountain(fountain))],
            &[],
            LevelScript::None,
        );
        let full = level.party.members[0].hp;
        level.party.members[0].hp = 1;
        let mut triggers = level.triggers(0);
        assert!(triggers.run("WELL"));
        assert!(!triggers.run("WELL"), "dry after its one use");
        // Two × 720 game seconds later it has refilled.
        triggers.party.now_ms = 2 * 720 * 1000;
        assert!(triggers.run("WELL"));
        assert_eq!(level.party.members[0].hp, 11.min(full));
    }
}
