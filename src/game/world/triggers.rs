//! Triggers: what using a prop, or another trigger naming it, sets off.
//!
//! After Whizzardry `Trigger::Run`, `SelectAction` and `CommitActionResult`.
//! A run selects an action (initial, alternate or fallback), lets the level
//! script veto it, performs it and, unless the action failed, commits:
//! the trigger's sound, its recipients in order (synchronously), its
//! location variable and its `fired` flag. Effects that presentation shows
//! (sounds, camera shakes, messages) are returned, not played here. See
//! `docs/research/trigger-actions.md`.

use std::collections::HashMap;

use bevy::prelude::*;

use bevy::ecs::system::SystemParam;

use super::level::{Level, PropPart};
use super::props::{Prop, PropMotion};
use super::scripts::LevelScript;
use crate::content::items::{Dice, ItemCatalog, LootTables};
use crate::content::level::{Arrival, LevelGeometry, StateChange, TriggerDefinition};
use crate::game::characters::Character;
use crate::game::data::GameData;
use crate::game::inventory::{ItemIds, ItemInstance, PartyInventory, make_item};
use crate::game::loot::roll_loot;
use crate::game::rng::GameRng;

/// Retail trigger action codes performed so far.
pub mod action {
    pub const NOTHING: u32 = 0x00;
    pub const DOOR_OPEN: u32 = 0x01;
    pub const DOOR_TOGGLE: u32 = 0x02;
    pub const TOGGLE: u32 = 0x03;
    pub const CAMERA_PATH: u32 = 0x09;
    pub const RELAY: u32 = 0x0f;
    pub const TELEPORT: u32 = 0x34;
    pub const RANDOM_TELEPORT: u32 = 0x4b;
    pub const DOOR_TOGGLE_WITH_KEY: u32 = 0x2c;
    pub const START: u32 = 0x32;
    pub const STOP: u32 = 0x33;
    pub const SHAKE: u32 = 0x39;
    /// Hurt everyone in the party now: `data` count d sides + base.
    pub const PARTY_HEALTH: u32 = 0x24;
    /// Hurt everyone in the party every half second while it stays.
    pub const DAMAGE_ZONE: u32 = 0x23;
    pub const DIAL: u32 = 0x40;
}

/// Damage zones hurt this often, in seconds.
const DAMAGE_ZONE_PERIOD: f32 = 0.5;

/// Recipients naming each other in a loop stop here.
const MAX_DEPTH: usize = 16;

/// The live state of a level's triggers and location variables, on the
/// level root. Triggers are indexed as in [`LevelGeometry::triggers`].
#[derive(Component, Clone, Debug)]
pub struct LevelState {
    triggers: Vec<TriggerState>,
    /// By name; retail looks names up case-insensitively, newest first.
    index: HashMap<String, usize>,
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
    use_alternate: bool,
    /// It waits for its prop's animation to finish (retail `RUNNING`).
    awaits_animation: bool,
    /// The dial position or door state (retail `state_index`).
    pub position: usize,
    /// The party's eye is within its range (positioned triggers).
    inside: bool,
    /// A container's contents, rolled when it is first opened.
    pub contents: Option<Contents>,
    /// A damage zone the party stands in: seconds since it last hurt.
    zone: Option<f32>,
}

/// What a container holds.
#[derive(Clone, Debug, Default)]
pub struct Contents {
    pub items: Vec<ItemInstance>,
    pub gold: u32,
}

impl LevelState {
    pub fn new(geometry: &LevelGeometry, script: LevelScript) -> Self {
        let mut state = LevelState {
            triggers: vec![
                TriggerState {
                    on: true,
                    fired: false,
                    use_alternate: false,
                    awaits_animation: false,
                    position: 0,
                    inside: false,
                    contents: None,
                    zone: None,
                };
                geometry.triggers.len()
            ],
            index: HashMap::new(),
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
            let Some(trigger) = prop.trigger.as_ref().and_then(|name| state.find(name)) else {
                continue;
            };
            let definition = &geometry.triggers[trigger];
            if let (action::DIAL, Some((name, _))) = (definition.initial, &definition.state_change)
            {
                for position in 0..prop.slots.len() {
                    state.variables.entry(format!("{name}{position}")).or_insert(position == 0);
                }
            }
        }
        state
    }

    /// The state kept from an earlier visit, for the party's return: it has
    /// not been seen here yet this time.
    pub fn revisited(&self) -> Self {
        LevelState { entered: false, ..self.clone() }
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
    /// The party, its possessions, items and randomness, for containers and
    /// traps; none while the databases load.
    pub party: Option<PartyContext<'a>>,
    /// Where a teleport or camera path sends the party.
    pub party_moves: Option<PartyMove>,
}

/// What containers and traps need: item definitions, loot tables,
/// randomness, the party's possessions and its members.
pub struct PartyContext<'a> {
    pub members: Vec<&'a mut Character>,
    pub catalog: &'a ItemCatalog,
    pub loot: &'a LootTables,
    pub rng: &'a mut GameRng,
    pub ids: &'a mut ItemIds,
    pub party: &'a mut PartyInventory,
}

/// Moving the party somewhere, which the party module carries out.
#[derive(Clone, Debug)]
pub enum PartyMove {
    Teleport(Arrival),
    /// Fly the party's eye along a camera path, by name (ladders, climbs).
    Fly(String),
}

impl Triggers<'_> {
    /// Whether the party may use a trigger at all (retail picking).
    pub fn usable(&self, index: usize) -> bool {
        let (definition, state) = (&self.definitions[index], &self.state.triggers[index]);
        definition.usable && !(definition.once && state.fired)
    }

    /// Run the positioned triggers the party's eye has just come within
    /// range of (retail `UpdateWorldTriggers`; retail reruns one each step
    /// while it is not running, here it runs on entering). The first time
    /// the party is seen in a level, triggers it stands in count as entered.
    pub fn walk(&mut self, eye: Vec3, seconds: f32) {
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
            match (inside, self.state.triggers[index].zone) {
                (false, Some(_)) => self.state.triggers[index].zone = None,
                (true, Some(elapsed)) => {
                    let elapsed = elapsed + seconds;
                    let due = elapsed >= DAMAGE_ZONE_PERIOD;
                    self.state.triggers[index].zone =
                        Some(if due { elapsed - DAMAGE_ZONE_PERIOD } else { elapsed });
                    if due {
                        self.damage_party(Dice { count: 1, sides: 6, base: 2 });
                    }
                }
                _ => {}
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

    fn run_index(&mut self, index: usize, depth: usize) -> bool {
        if depth > MAX_DEPTH {
            warn!("trigger {} recurses too deeply", self.definitions[index].name);
            return false;
        }
        let definitions = self.definitions;
        let definition = &definitions[index];
        let Some((action, use_index)) = self.select(index) else { return false };
        let script = self.state.script;
        if !script.before_run(&definition.name, self) {
            return false;
        }
        let mut changes_state = true;
        match action {
            action::NOTHING => {}
            action::DOOR_OPEN | action::DOOR_TOGGLE | action::DOOR_TOGGLE_WITH_KEY => {
                if definition.door.is_some_and(|door| door.locked) {
                    self.effects.push(TriggerEffect::Locked);
                    return false;
                }
                let Some(prop) = self.props.get_mut(&index) else { return false };
                if prop.playing() {
                    return false;
                }
                let state = &mut self.state.triggers[index];
                if action == action::DOOR_OPEN {
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
            action::TOGGLE | action::START | action::STOP => {
                if definition.loot_table.is_some() {
                    self.open_container(index);
                }
                let Some(prop) = self.props.get_mut(&index) else { return false };
                match action {
                    action::TOGGLE => prop.toggle(),
                    action::START if !prop.playing() => prop.start(),
                    action::STOP if prop.playing() => prop.motion.target = prop.motion.frame,
                    _ => {}
                }
                let state = &mut self.state.triggers[index];
                state.position ^= 1;
                state.awaits_animation = true;
            }
            action::RELAY => {
                let mut found = false;
                for recipient in &definition.recipients {
                    if let Some(target) = self.state.find(recipient) {
                        found = true;
                        self.run_index(target, depth + 1);
                    }
                }
                if !found {
                    return false;
                }
            }
            action::TELEPORT | action::RANDOM_TELEPORT => {
                let Some(arrival) = &definition.arrival else { return false };
                self.party_moves = Some(PartyMove::Teleport(arrival.clone()));
            }
            action::CAMERA_PATH => {
                let Some(path) = definition.recipients.first() else { return false };
                self.party_moves = Some(PartyMove::Fly(path.clone()));
            }
            action::PARTY_HEALTH => {
                let [count, sides, base] = definition.data;
                let or = |v: i32, default: i32| if v < 0 { default } else { v };
                self.damage_party(Dice {
                    count: or(count, 1) as u8,
                    sides: or(sides, 6) as u8,
                    base: or(base, 2) as i16,
                });
            }
            action::DAMAGE_ZONE => {
                self.state.triggers[index].zone = Some(0.0);
            }
            action::SHAKE => {
                let [intensity, _, duration] = definition.data;
                self.effects.push(TriggerEffect::Shake {
                    seconds: if duration == -1 {
                        1.0
                    } else {
                        duration.unsigned_abs() as f32 / 1000.0
                    },
                    intensity: if intensity == -1 { 800.0 } else { intensity as f32 },
                    fade_out: duration < -1,
                });
            }
            action::DIAL => {
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
            other => {
                debug!("trigger {}: action {other:#x} is not implemented", definition.name);
                return false;
            }
        }
        self.commit(index, use_index, changes_state, depth);
        true
    }

    /// Hurt each living member by their own roll (retail
    /// `ApplyRolledHealthChangeToParty`). Unconsciousness and death are not
    /// modelled yet: health stops at 0.
    fn damage_party(&mut self, dice: Dice) {
        let Some(party) = &mut self.party else { return };
        for member in party.members.iter_mut().filter(|m| m.hp > 0) {
            let amount = party.rng.roll(dice);
            member.hp = member.hp.saturating_sub(amount);
            self.effects.push(TriggerEffect::Hurt { member: member.name.clone(), amount });
        }
    }

    /// Open a container: roll its contents the first time (retail
    /// `GenerateItemGroup`), then give the party its gold and as many items
    /// as the pool takes. Retail puts a single item in the hand and opens a
    /// picker for several; with no inventory screen yet, all go to the pool.
    fn open_container(&mut self, index: usize) {
        let definitions = self.definitions;
        let Some(table_name) = &definitions[index].loot_table else { return };
        let Some(items) = &mut self.party else { return };
        let contents = self.state.triggers[index].contents.get_or_insert_with(|| {
            let Some(table) = items.loot.find(table_name) else {
                warn!("no loot table {table_name}");
                return Contents::default();
            };
            // The party's level once characters exist.
            let (found, gold) = roll_loot(table, items.catalog, 1, items.rng);
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

    /// Retail `SelectAction`: the action this run performs, and which of
    /// initial (0), alternate (1) and fallback (2) it is.
    fn select(&mut self, index: usize) -> Option<(u32, usize)> {
        let definition = &self.definitions[index];
        let running = self.state.triggers[index].awaits_animation
            && self.props.get(&index).is_some_and(TriggerProp::playing);
        let state = &self.state.triggers[index];
        if !state.on
            || (definition.once && state.fired)
            || (running && definition.initial != action::SHAKE)
        {
            return None;
        }
        if !definition.required_states.iter().all(|name| self.state.variable(name)) {
            return (definition.fallback != 0).then_some((definition.fallback, 2));
        }
        let state = &mut self.state.triggers[index];
        if !state.use_alternate {
            state.use_alternate = definition.alternate != 0;
            Some((definition.initial, 0))
        } else {
            state.use_alternate = !definition.alternate_toggles;
            Some((definition.alternate, 1))
        }
    }

    /// Retail `CommitActionResult`.
    fn commit(&mut self, index: usize, use_index: usize, changes_state: bool, depth: usize) {
        let definitions = self.definitions;
        let definition = &definitions[index];
        if let Some(sound) = &definition.sound {
            let at = self.props.get(&index).map(|prop| prop.position);
            self.effects.push(TriggerEffect::Sound {
                path: sound.path.clone(),
                volume: sound.volume,
                at,
            });
        }
        if definition.fire_linked {
            for recipient in &definition.recipients {
                if let Some(target) = self.state.find(recipient) {
                    self.run_index(target, depth + 1);
                }
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
        if definition.shows_message && definition.data[use_index] >= 0 {
            self.effects.push(TriggerEffect::Message(definition.data[use_index]));
        }
        self.state.triggers[index].fired = true;
    }
}

/// The current level's triggers, ready to run.
#[derive(SystemParam)]
pub struct LevelTriggers<'w, 's> {
    levels: Query<'w, 's, (&'static Level, &'static mut LevelState)>,
    geometry: Res<'w, Assets<LevelGeometry>>,
    props: Query<'w, 's, (&'static Prop, &'static mut PropMotion, &'static Children)>,
    parts: Query<'w, 's, &'static GlobalTransform, With<PropPart>>,
    effects: MessageWriter<'w, TriggerEffect>,
    party_move: ResMut<'w, PendingPartyMove>,
    data: Option<Res<'w, GameData>>,
    catalogs: Res<'w, Assets<ItemCatalog>>,
    loot: Res<'w, Assets<LootTables>>,
    rng: ResMut<'w, GameRng>,
    ids: ResMut<'w, ItemIds>,
    party: ResMut<'w, PartyInventory>,
    members: Query<'w, 's, &'static mut Character>,
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
    /// Nothing runs while no level is built.
    pub fn with<R>(&mut self, run: impl FnOnce(&mut Triggers) -> R) -> Option<R> {
        let Ok((level, state)) = self.levels.single_mut() else { return None };
        let geometry = self.geometry.get(&level.geometry)?;
        let state = state.into_inner();
        let mut props = HashMap::new();
        for (prop, motion, children) in &mut self.props {
            let Some(index) = prop.trigger.as_ref().and_then(|name| state.find(name)) else {
                continue;
            };
            // Sounds play from the middle of the prop's parts.
            let parts: Vec<Vec3> = children
                .iter()
                .filter_map(|c| self.parts.get(c).ok())
                .map(|t| t.translation())
                .collect();
            let position = parts.iter().sum::<Vec3>() / parts.len().max(1) as f32;
            props.insert(
                index,
                TriggerProp {
                    motion: motion.into_inner(),
                    last_frame: prop.last_frame as f32,
                    wraps: prop.wraps,
                    slots: &prop.slots,
                    position,
                },
            );
        }
        let members = self.members.iter_mut().map(|m| m.into_inner()).collect();
        let party = self.data.as_ref().and_then(|data| {
            Some(PartyContext {
                members,
                catalog: self.catalogs.get(&data.items)?,
                loot: self.loot.get(&data.loot)?,
                rng: self.rng.as_mut(),
                ids: self.ids.as_mut(),
                party: self.party.as_mut(),
            })
        });
        let mut triggers = Triggers {
            definitions: &geometry.triggers,
            state,
            props,
            effects: Vec::new(),
            party,
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
    use crate::content::level::{DoorLock, PropModel};

    fn trigger(name: &str, initial: u32) -> TriggerDefinition {
        TriggerDefinition {
            name: name.into(),
            usable: false,
            once: false,
            fire_linked: false,
            initial,
            alternate: 0,
            fallback: 0,
            alternate_toggles: false,
            recipients: vec![],
            data: [-1; 3],
            sound: None,
            shows_message: false,
            required_states: vec![],
            state_change: None,
            initial_state: None,
            range: (0.0, 10.0),
            door: None,
            place: None,
            arrival: None,
            loot_table: None,
        }
    }

    fn prop(trigger: &str, last_frame: f32, wraps: bool, slots: &[u16]) -> (PropModel, f32) {
        let model = PropModel {
            name: trigger.to_lowercase(),
            collidable: false,
            face_camera: false,
            frames_per_second: 10.0,
            parts: vec![],
            wraps,
            slots: slots.to_vec(),
            trigger: Some(trigger.into()),
        };
        (model, last_frame)
    }

    /// A level's triggers with their props held still or finished.
    struct Level {
        geometry: LevelGeometry,
        state: LevelState,
        last_frames: Vec<f32>,
        motions: Vec<PropMotion>,
    }

    impl Level {
        fn new(
            triggers: Vec<TriggerDefinition>,
            props: Vec<(PropModel, f32)>,
            script: LevelScript,
        ) -> Self {
            let (props, last_frames): (Vec<_>, Vec<_>) = props.into_iter().unzip();
            let motions = props.iter().map(|_| PropMotion::default()).collect();
            let geometry = LevelGeometry { triggers, props, ..default() };
            let state = LevelState::new(&geometry, script);
            Level { geometry, state, last_frames, motions }
        }

        /// Run a trigger as the party; returns whether it committed and the
        /// effects.
        fn run(&mut self, name: &str) -> (bool, Vec<TriggerEffect>) {
            let mut props = HashMap::new();
            for ((model, motion), &last_frame) in
                self.geometry.props.iter().zip(&mut self.motions).zip(&self.last_frames)
            {
                let index = self.state.find(model.trigger.as_ref().unwrap()).unwrap();
                props.insert(
                    index,
                    TriggerProp {
                        motion,
                        last_frame,
                        wraps: model.wraps,
                        slots: &model.slots,
                        position: Vec3::ZERO,
                    },
                );
            }
            let mut triggers = Triggers {
                definitions: &self.geometry.triggers,
                state: &mut self.state,
                props,
                effects: vec![],
                party: None,
                party_moves: None,
            };
            let committed = triggers.run(name);
            (committed, triggers.effects)
        }

        fn motion(&self, trigger: &str) -> &PropMotion {
            let index =
                self.geometry.props.iter().position(|p| p.trigger.as_deref() == Some(trigger));
            &self.motions[index.unwrap()]
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
        let mut door = trigger("DOOR", action::DOOR_OPEN);
        door.alternate = action::DOOR_TOGGLE;
        let mut locked = trigger("LOCKED", action::DOOR_OPEN);
        locked.door = Some(DoorLock { locked: true, key: None });
        let mut level = Level::new(
            vec![door, locked],
            vec![prop("DOOR", 15.0, false, &[]), prop("LOCKED", 15.0, false, &[])],
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

    /// The upper monastery's bells, as authored (`docs/research/trigger-actions.md`).
    fn bells() -> Level {
        let mut dial = trigger("BELLRINGSWITCH", action::DIAL);
        dial.usable = true;
        dial.state_change = Some(("RING".into(), StateChange::Set));
        dial.initial_state = Some(false);
        let mut button = trigger("BELL_BUTTON", action::TOGGLE);
        button.usable = true;
        button.fire_linked = true;
        button.recipients = ["BELLTRIGGER01", "BELLTRIGGERC", "BELL_ACTIVATE_2", "BELL_ACTIVATE_3"]
            .map(String::from)
            .to_vec();
        let mut windows = trigger("BELLTRIGGER01", action::NOTHING);
        windows.once = true;
        windows.fire_linked = true;
        windows.required_states = vec!["RING2".into()];
        windows.recipients = vec!["GLASS1".into(), "MONACAMSHAKE".into()];
        let relay = |name: &str, ring: &str, recipients: &[&str]| {
            let mut relay = trigger(name, action::RELAY);
            relay.fire_linked = true;
            relay.required_states = vec![ring.into()];
            relay.recipients = recipients.iter().map(|r| r.to_string()).collect();
            relay
        };
        let mut shake = trigger("MONACAMSHAKE", action::SHAKE);
        shake.data = [90, 40, -2400];
        Level::new(
            vec![
                dial,
                button,
                windows,
                relay("BELLTRIGGERC", "RING0", &["BELL_1"]),
                relay("BELL_ACTIVATE_2", "RING1", &["BELL_2"]),
                relay("BELL_ACTIVATE_3", "RING2", &["BELL_3", "MONACAMSHAKE"]),
                trigger("BELL_1", action::TOGGLE),
                trigger("BELL_2", action::TOGGLE),
                trigger("BELL_3", action::TOGGLE),
                trigger("GLASS1", action::TOGGLE),
                shake,
            ],
            vec![
                prop("BELLRINGSWITCH", 20.0, false, &[0, 1, 10, 20]),
                prop("BELL_BUTTON", 11.0, false, &[]),
                prop("BELL_1", 36.0, true, &[]),
                prop("BELL_2", 36.0, true, &[]),
                prop("BELL_3", 36.0, true, &[]),
                prop("GLASS1", 2.0, false, &[]),
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
        let button = level
            .geometry
            .props
            .iter()
            .position(|p| p.trigger.as_deref() == Some("BELL_BUTTON"))
            .unwrap();
        level.motions[button].frame = level.motions[button].target;
        let bell = level.motion("BELL_1").frame;
        assert_eq!(level.run("BELL_BUTTON").1, [bell_sound(1)]);
        assert_eq!(level.motion("BELL_1").frame, bell);
        assert_eq!(level.motion("BELL_1").target, 36.0);
    }

    #[test]
    fn walk_in_triggers_fire_on_entering_when_usable() {
        let teleport = |name: &str, x: f32, usable: bool| {
            let mut trigger = trigger(name, action::TELEPORT);
            trigger.usable = usable;
            trigger.place = Some(Vec3::new(x, 0.0, 0.0));
            trigger.range = (0.0, 2.0);
            trigger.arrival = Some(Arrival::Entrance { entrance: format!("{name}00"), ask: false });
            trigger
        };
        let geometry = LevelGeometry {
            triggers: vec![
                teleport("START", 0.0, true),
                teleport("EXIT", 10.0, true),
                teleport("OFF", 20.0, false),
            ],
            ..default()
        };
        let mut state = LevelState::new(&geometry, LevelScript::None);
        let mut walk = |eye: Vec3| {
            let mut triggers = Triggers {
                definitions: &geometry.triggers,
                state: &mut state,
                props: HashMap::new(),
                effects: vec![],
                party: None,
                party_moves: None,
            };
            triggers.walk(eye, 0.1);
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
        use crate::game::characters::{Gender, Profession, Race};
        let mut trap = trigger("TRAP", action::PARTY_HEALTH);
        trap.data = [2, 4, 0];
        let mut zone = trigger("ACID", action::DAMAGE_ZONE);
        zone.usable = true;
        zone.place = Some(Vec3::ZERO);
        zone.range = (0.0, 2.0);
        let geometry = LevelGeometry { triggers: vec![trap, zone], ..default() };
        let mut state = LevelState::new(&geometry, LevelScript::None);
        let (catalog, loot) = (ItemCatalog::default(), LootTables::default());
        let (mut rng, mut ids, mut inventory) =
            (GameRng::seeded(5), ItemIds::default(), PartyInventory::default());
        let mut a = Character::new("A", Race::Human, Gender::Male, Profession::Fighter);
        let mut b = Character::new("B", Race::Human, Gender::Male, Profession::Fighter);
        let full = a.hp;
        let mut triggers = Triggers {
            definitions: &geometry.triggers,
            state: &mut state,
            props: HashMap::new(),
            effects: vec![],
            party: Some(PartyContext {
                members: vec![&mut a, &mut b],
                catalog: &catalog,
                loot: &loot,
                rng: &mut rng,
                ids: &mut ids,
                party: &mut inventory,
            }),
            party_moves: None,
        };
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
        triggers.walk(Vec3::new(5.0, 0.0, 0.0), 0.1);
        triggers.walk(Vec3::ZERO, 0.1);
        for _ in 0..10 {
            triggers.walk(Vec3::ZERO, 0.1);
        }
        let zone_hits =
            triggers.effects.iter().filter(|e| matches!(e, TriggerEffect::Hurt { .. })).count();
        assert_eq!(zone_hits, 2 * 2, "two ticks for each of two members");
        triggers.walk(Vec3::new(5.0, 0.0, 0.0), 0.1);
        triggers.effects.clear();
        triggers.walk(Vec3::new(5.0, 0.0, 0.0), 1.0);
        assert!(triggers.effects.is_empty());
        drop(triggers);
        assert!(a.hp < full && b.hp < full);
    }
}
