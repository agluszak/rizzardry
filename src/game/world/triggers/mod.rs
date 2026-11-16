//! Devices: the levers, doors, chests, fountains and places of a level that
//! the party, the story or another device sets going (retail `Trigger`).
//!
//! Using one goes through events, in the order the original runs it: it is
//! asked to go ([`Activate`]), picks the use its state calls for, lets the
//! story have its say ([`Activating`]), carries out that use's act
//! ([`Act`]), and once the act is done sounds, runs what it is linked to
//! and settles ([`Committed`], [`Fired`]). Bevy carries a cascade of
//! events out depth-first, so a relay's recipients run before it settles,
//! as in the original.

mod acts;
mod spawn;
mod walk;

pub use spawn::spawn_devices;
pub(crate) use walk::walk;

use std::collections::HashMap;

use avian3d::prelude::{Collider, Position};
use bevy::prelude::*;

use super::interaction::{Confront, Guarded};
use super::level::{Level, LevelOwned};
use super::props::{PropMotion, PropPart};
use crate::content::items::{Dice, ItemId};
use crate::content::level::{
    Animation, Arrival, StateChange, TriggerAction, TriggerSound, TriggerUse, VariableName,
};
use crate::game::characters::{Calling, Vitals};
use crate::game::hits::Damaged;
use crate::game::items::Items;
use crate::game::npcs::NpcSpeaker;
use crate::game::npcs::devices::Gate;
use crate::game::party::body::PartyBody;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::level::InPlay;

/// How many links deep a device may run others before the original gives
/// up (retail's recursion guard).
const MAX_DEPTH: u8 = 16;

/// A device, by the name the level gives it. A prop's device lives on the
/// prop; one at a place stands alone.
#[derive(Component, Debug)]
#[require(TriggerState)]
pub struct Device {
    pub name: String,
}

/// What a device does: its first use, the one it alternates with, and the
/// one it falls back on when what it requires is missing.
#[derive(Component, Debug)]
pub struct Uses {
    initial: TriggerUse,
    alternate: Option<TriggerUse>,
    fallback: Option<TriggerUse>,
    /// Once used, it keeps to the alternate rather than going back.
    alternate_toggles: bool,
}

impl Uses {
    fn get(&self, slot: Slot) -> &TriggerUse {
        match slot {
            Slot::Initial => &self.initial,
            Slot::Alternate => self.alternate.as_ref().unwrap(),
            Slot::Fallback => self.fallback.as_ref().unwrap(),
        }
    }
}

/// Which of its uses a device carries out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Initial,
    Alternate,
    Fallback,
}

/// The player may use it.
#[derive(Component, Debug)]
pub struct Usable;

/// It runs once only.
#[derive(Component, Debug)]
pub struct Once;

/// Once it has done its act it runs what it is linked to.
#[derive(Component, Debug)]
pub struct FiresLinked;

/// The devices it is linked to, in order. A device may be linked from
/// several, so this is a list, not a relationship.
#[derive(Component, Debug)]
pub struct Recipients(pub Vec<Entity>);

/// The level variables that must be set for it to do its act; without them
/// it falls back.
#[derive(Component, Debug)]
pub struct Requires(Vec<VariableName>);

/// The level variable its act sets, clears or toggles.
#[derive(Component, Debug)]
pub struct ChangesState(VariableName, StateChange);

/// How near the party must stand to use it, from its prop's centre.
#[derive(Component, Debug)]
pub struct Reach {
    range: (f32, f32),
    centre: Vec3,
}

/// What it sounds like, and where; some sound as their act is done.
#[derive(Component, Debug)]
pub struct DeviceSound {
    sound: TriggerSound,
    at: Option<Vec3>,
    on_commit: bool,
}

/// The item that unlocks its door.
#[derive(Component, Debug)]
pub struct Key(ItemId);

/// Only level-wide variables and visit state live on the level.
#[derive(Component, Debug, Default)]
pub struct LevelState {
    pub camera_sway: bool,
    /// The level's variables: numbers, of which devices ask only whether
    /// one is set (retail's location variables).
    variables: HashMap<VariableName, i32>,
    ambient_sounds: HashMap<String, bool>,
    previous_eye: Option<Vec3>,
}

/// Current named light activation, with its own rendering change boundary.
#[derive(Component, Clone, Debug, Default)]
pub struct LightActivation {
    lights: HashMap<String, bool>,
}

impl LightActivation {
    pub fn light_enabled(&self, name: &str, initial: bool) -> bool {
        self.lights.get(&name.to_uppercase()).copied().unwrap_or(initial)
    }
}

impl LevelState {
    pub fn ambient_sound_enabled(&self, name: &str) -> bool {
        self.ambient_sounds.get(&name.to_uppercase()).copied().unwrap_or(true)
    }

    /// Rebase walk-in detection after teleporting, without firing destination
    /// triggers or treating an unobserved level as already visited.
    pub(crate) fn arrive(&mut self, eye: Vec3) {
        if self.previous_eye.is_some() {
            self.previous_eye = Some(eye);
        }
    }

    /// Whether a variable was ever made.
    pub fn made(&self, name: &VariableName) -> bool {
        self.variables.contains_key(name)
    }

    /// What a variable holds; one never made holds nothing.
    pub fn value(&self, name: &VariableName) -> i32 {
        self.variables.get(name).copied().unwrap_or(0)
    }

    fn variable(&self, name: &VariableName) -> bool {
        self.value(name) != 0
    }

    fn set_variable(&mut self, name: &VariableName, value: bool) {
        self.variables.insert(name.clone(), i32::from(value));
    }
}

#[derive(Component, Clone, Debug)]
pub struct TriggerState {
    pub on: bool,
    pub fired: bool,
    use_alternate: bool,
    awaits_animation: bool,
}

impl Default for TriggerState {
    fn default() -> Self {
        Self { on: true, fired: false, use_alternate: false, awaits_animation: false }
    }
}

/// Logical door state is independent of its prop's animation progress.
#[derive(Component, Clone, Debug)]
pub struct DoorState {
    pub open: bool,
    pub locked: bool,
}

/// A dial: where it stands, and the level variable whose numbered copies
/// say so.
#[derive(Component, Clone, Debug, Default)]
pub struct DialState {
    pub position: usize,
    variable: Option<VariableName>,
}

/// On a level: its devices by their names. A name several share is the
/// last one's, as the original finds it.
#[derive(Component, Debug, Default)]
pub struct DeviceNames(HashMap<String, Entity>);

impl DeviceNames {
    pub fn named(&self, name: &str) -> Option<Entity> {
        self.0.get(&name.to_ascii_lowercase()).copied()
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
    Message {
        source: Option<Handle<crate::content::ui::TextTable>>,
        id: i32,
    },
    /// The party used something and nothing came of it.
    NothingHappened,
    /// A mandatory authored action has no implementation.
    Unsupported {
        device: String,
        code: u32,
    },
    Gold(u32),
    FountainUsed {
        at: Option<Vec3>,
    },
    Locked,
}

/// Moving the party somewhere, which the party module carries out.
#[derive(Clone, Debug)]
pub enum PartyMove {
    Teleport(Arrival),
    /// Fly the party's eye along a camera path, by name (ladders, climbs).
    Fly(usize),
}

/// On a level: a move one of its devices requested, carried out after the
/// trigger step. A later request replaces it; it goes with its level.
#[derive(Component, Clone, Debug)]
pub struct PendingPartyMove(pub PartyMove);

/// Who sets a device going.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum By {
    /// The player, who must be free, within reach of a device they may use;
    /// they are told when nothing comes of it.
    Party,
    /// The party walking into its place, unless already on the move.
    Walking,
    /// The level itself: the story, a monster, a lock overcome.
    Level,
    /// A device it is linked to, so many links deep; a relay passes its own
    /// source on (retail's call argument).
    Link { depth: u8, source: i32 },
}

impl By {
    fn depth(self) -> u8 {
        if let By::Link { depth, .. } = self { depth } else { 0 }
    }

    fn source(self) -> i32 {
        if let By::Link { source, .. } = self { source } else { -1 }
    }

    /// One link further on, with `source` passed along.
    fn linked(self, source: i32) -> Self {
        By::Link { depth: self.depth() + 1, source }
    }
}

/// A device's use being carried out: who set it going and which use.
#[derive(Clone, Copy, Debug)]
pub struct Doing {
    pub by: By,
    pub slot: Slot,
}

impl Doing {
    /// Nothing came of it; the player is told so if it was theirs.
    pub(crate) fn decline(self, effects: &mut MessageWriter<TriggerEffect>) {
        if self.by == By::Party {
            effects.write(TriggerEffect::NothingHappened);
        }
    }
}

/// Set a device going.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Activate {
    #[event_target]
    pub device: Entity,
    pub by: By,
}

/// A device is about to do what it was made to; whoever has a say in it
/// (the story's [`Gate`]) may stop it, and otherwise it [`Perform`]s.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Activating {
    #[event_target]
    pub device: Entity,
    pub doing: Doing,
}

/// A device carries out its chosen use.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Perform {
    #[event_target]
    pub device: Entity,
    pub doing: Doing,
}

/// One act of a device, `what` being what its use says.
#[derive(EntityEvent, Clone, Debug)]
pub struct Act<T: Send + Sync + 'static> {
    #[event_target]
    pub device: Entity,
    pub doing: Doing,
    pub what: T,
}

/// The act a container's lid is: it is opened before it moves.
#[derive(Clone, Copy, Debug)]
pub struct Open(pub Animation);

/// A door opened, or opened or closed.
#[derive(Clone, Copy, Debug)]
pub struct Door {
    pub open_only: bool,
}

/// A device's act is done: it sounds and runs what it is linked to, then is
/// [`Fired`].
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Committed {
    #[event_target]
    pub device: Entity,
    pub doing: Doing,
}

/// A device settles after its act and its links: the level variable it
/// changes, its message, and that it has fired.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Fired {
    #[event_target]
    pub device: Entity,
    pub doing: Doing,
}

/// Hurt each living member by their own roll of the dice.
#[derive(Event, Clone, Copy, Debug)]
pub struct HurtParty(pub Dice);

/// Something the story does to a named device or variable of the level
/// the party is in (retail `FindTriggerByName`, `CreateLocationVar`). A name
/// the level does not have is nothing to it.
#[derive(Event, Clone, Debug, PartialEq, Eq)]
pub enum Bid {
    Run(String),
    /// Run it if it never has.
    RunOnce(String),
    SwitchOff(String),
    /// Switch it on and run it.
    SwitchOn(String),
    /// Switch it on, without running it.
    Enable(String),
    /// Overcome its lock or trap, as if it had been picked or disarmed.
    Unlock(String),
    Set(VariableName, i32),
    /// Set these bits of a variable, keeping the others.
    SetBits(VariableName, i32),
    /// Move a variable on by one, back to nothing at a count.
    Step(VariableName, i32),
}

pub(crate) fn plugin(app: &mut App) {
    app.add_observer(activate)
        .add_observer(ungated)
        .add_observer(perform)
        .add_observer(committed)
        .add_observer(fired)
        .add_observer(bid)
        .add_observer(hurt_party)
        .add_plugins(acts::plugin);
}

/// Which use a device's state calls for (retail `Trigger::SelectAction`).
enum Selected {
    /// `keyed` when the key in hand unlocks it, which overcomes its lock
    /// too (retail marks the lock completed); `next` is whether it
    /// alternates next time.
    Use { slot: Slot, keyed: bool, next: Option<bool> },
    /// A lock or trap must be overcome first.
    Guarded,
}

/// Whether a device it runs, or one those run, is mid-animation.
fn links_moving(
    device: Entity,
    depth: u8,
    recipients: &Query<&Recipients>,
    motions: &Query<&PropMotion>,
) -> bool {
    depth <= MAX_DEPTH
        && recipients.get(device).is_ok_and(|linked| {
            linked.0.iter().any(|&recipient| {
                motions.get(recipient).is_ok_and(|m| m.frame != m.target)
                    || links_moving(recipient, depth + 1, recipients, motions)
            })
        })
}

/// A device is set going: for the player, if they are free and it is in
/// reach; walking in, unless the party is already being moved. Then the
/// use its state calls for is chosen and committed to: a key in hand
/// unlocks its door and overcomes its lock, and the next use is settled,
/// even if the act then comes to nothing (retail `Run` undoes neither). A
/// lock or trap in the way is confronted instead. A device that speaks for
/// someone is the story's.
#[allow(clippy::type_complexity)]
fn activate(
    activate: On<Activate>,
    mut devices: Query<(
        &Device,
        &Uses,
        &mut TriggerState,
        &LevelOwned,
        (Has<Usable>, Has<Once>, Has<Guarded>),
        (Option<&Requires>, Option<&Reach>, Option<&Children>),
        Has<NpcSpeaker>,
    )>,
    mut doors: Query<(&mut DoorState, Option<&Key>)>,
    (recipients, motions): (Query<&Recipients>, Query<&PropMotion>),
    (levels, moving): (Query<&LevelState>, Query<(), With<PendingPartyMove>>),
    party: Single<(Entity, &Position), With<PartyBody>>,
    parts: Query<(&Transform, &Collider), With<PropPart>>,
    (freedom, items): (crate::game::freedom::Freedom, Items),
    mut effects: MessageWriter<TriggerEffect>,
    mut commands: Commands,
) -> Result {
    let (device, by) = (activate.device, activate.by);
    let (
        named,
        uses,
        mut state,
        owner,
        (usable, once, guarded),
        (requires, reach, children),
        speaks,
    ) = devices.get_mut(device)?;
    // A device that speaks for someone is used by being spoken through.
    if speaks {
        return Ok(());
    }
    let ready = usable && state.on && !(once && state.fired);
    let (party, at) = *party;
    match by {
        By::Party => {
            // Reach is from the party's eye to the prop's nearest part, or
            // its centre for one with nothing solid.
            let eye = PartyBody::eye(at);
            let Some(reach) = reach else { return Ok(()) };
            let solid = children.into_iter().flatten().filter_map(|&c| parts.get(c).ok());
            let distance = solid
                .map(|(at, shape)| shape.distance_to_point(at.translation, at.rotation, eye, true))
                .min_by(f32::total_cmp)
                .unwrap_or_else(|| eye.distance(reach.centre));
            if !freedom.free() || !ready || !(reach.range.0..=reach.range.1).contains(&distance) {
                return Ok(());
            }
        }
        By::Walking if !ready || moving.contains(owner.0) => return Ok(()),
        _ => {}
    }
    let decline = |effects: &mut MessageWriter<TriggerEffect>| {
        if by == By::Party {
            effects.write(TriggerEffect::NothingHappened);
        }
    };
    if by.depth() > MAX_DEPTH {
        warn!("trigger {} recurses too deeply", named.name);
        return Ok(());
    }
    let mut door = doors.get_mut(device).ok();
    // A door waits while any device it runs is still moving (retail
    // `CanRunLinkedTriggers`), before a key is tried.
    if door.is_some() && links_moving(device, 0, &recipients, &motions) {
        decline(&mut effects);
        return Ok(());
    }
    let running = state.awaits_animation
        && motions.get(device).is_ok_and(|m| m.frame != m.target)
        && !matches!(uses.initial.action, TriggerAction::Shake { .. });
    if !state.on || (once && state.fired) || running {
        decline(&mut effects);
        return Ok(());
    }
    let level = levels.get(owner.0).unwrap();
    let fallback = || {
        uses.fallback.as_ref().map(|_| Selected::Use {
            slot: Slot::Fallback,
            keyed: false,
            next: None,
        })
    };
    let selected = if !requires.is_none_or(|r| r.0.iter().all(|name| level.variable(name))) {
        fallback()
    } else {
        let offered = items.held(party).and_then(|held| items.id(held));
        let keyed = match &door {
            Some((door, Some(Key(key)))) if door.locked => Some(offered == Some(*key)),
            _ => None,
        };
        match keyed {
            Some(false) => fallback(),
            None if guarded => Some(Selected::Guarded),
            _ => {
                let (slot, next) = match &uses.alternate {
                    Some(_) if state.use_alternate => (Slot::Alternate, !uses.alternate_toggles),
                    alternate => (Slot::Initial, alternate.is_some()),
                };
                Some(Selected::Use { slot, keyed: keyed == Some(true), next: Some(next) })
            }
        }
    };
    match selected {
        None => decline(&mut effects),
        Some(Selected::Guarded) => commands.trigger(Confront { device }),
        Some(Selected::Use { slot, keyed, next }) => {
            // A missing implementation is not the device declining: its
            // selection and lock stay as they were.
            if let TriggerAction::Unsupported(code) = uses.get(slot).action {
                effects.write(TriggerEffect::Unsupported { device: named.name.clone(), code });
                return Ok(());
            }
            if keyed && let Some((door, _)) = &mut door {
                door.locked = false;
                commands.entity(device).remove::<Guarded>();
            }
            if let Some(next) = next
                && state.use_alternate != next
            {
                state.use_alternate = next;
            }
            commands.trigger(Activating { device, doing: Doing { by, slot } });
        }
    }
    Ok(())
}

/// A device nobody has a say in goes on.
fn ungated(activating: On<Activating>, gated: Query<(), With<Gate>>, mut commands: Commands) {
    if !gated.contains(activating.device) {
        commands.trigger(Perform { device: activating.device, doing: activating.doing });
    }
}

/// A device carries out its chosen use: the act it names. A container's
/// lid opens the container.
fn perform(
    perform: On<Perform>,
    devices: Query<(&Uses, Has<super::containers::Container>)>,
    mut commands: Commands,
) -> Result {
    let (device, doing) = (perform.device, perform.doing);
    let (uses, container) = devices.get(device)?;
    match &uses.get(doing.slot).action {
        TriggerAction::Nothing => commands.trigger(Committed { device, doing }),
        TriggerAction::NoRuntimeAction => commands.trigger(Act { device, doing, what: acts::Idle }),
        TriggerAction::Sound => commands.trigger(Act { device, doing, what: acts::Sound }),
        TriggerAction::CameraSway => commands.trigger(Act { device, doing, what: acts::Sway }),
        TriggerAction::DamageZone => commands.trigger(Act { device, doing, what: acts::Zone }),
        TriggerAction::Dial => commands.trigger(Act { device, doing, what: acts::Dial }),
        TriggerAction::OpenDoor => {
            commands.trigger(Act { device, doing, what: Door { open_only: true } });
        }
        TriggerAction::ToggleDoor => {
            commands.trigger(Act { device, doing, what: Door { open_only: false } });
        }
        &TriggerAction::Animate(animation) if container => {
            commands.trigger(Act { device, doing, what: Open(animation) });
        }
        &TriggerAction::Animate(animation) => {
            commands.trigger(Act { device, doing, what: animation });
        }
        &TriggerAction::CameraPath(path) => {
            commands.trigger(Act { device, doing, what: PartyMove::Fly(path) });
        }
        TriggerAction::Teleport(arrival) => {
            commands.trigger(Act { device, doing, what: PartyMove::Teleport(arrival.clone()) });
        }
        TriggerAction::RandomTeleport(arrivals) => {
            commands.trigger(Act { device, doing, what: arrivals.clone() });
        }
        &TriggerAction::Relay(source) => {
            commands.trigger(Act { device, doing, what: acts::Relay(source) });
        }
        &TriggerAction::SetDevices(change) => {
            commands.trigger(Act { device, doing, what: change });
        }
        TriggerAction::Lights { sources, change } => {
            let what = acts::Lights(sources.clone(), *change);
            commands.trigger(Act { device, doing, what });
        }
        TriggerAction::Particles { sources, change } => {
            let what = acts::Particles(sources.clone(), *change);
            commands.trigger(Act { device, doing, what });
        }
        TriggerAction::AmbientSounds { names, change } => {
            let what = acts::AmbientSounds(names.clone(), *change);
            commands.trigger(Act { device, doing, what });
        }
        &TriggerAction::HurtParty(dice) => commands.trigger(Act { device, doing, what: dice }),
        &TriggerAction::Fountain(fountain) => {
            commands.trigger(Act { device, doing, what: fountain });
        }
        &TriggerAction::Shake { seconds, intensity, fade_out } => {
            let what = TriggerEffect::Shake { seconds, intensity, fade_out };
            commands.trigger(Act { device, doing, what });
        }
        TriggerAction::Unsupported(_) => unreachable!("refused as the use is chosen"),
    }
    Ok(())
}

/// A device whose act is done sounds if it sounds then, and runs what it is
/// linked to before it settles.
fn committed(
    committed: On<Committed>,
    devices: Query<(Option<&DeviceSound>, Has<FiresLinked>, Option<&Recipients>)>,
    mut effects: MessageWriter<TriggerEffect>,
    mut commands: Commands,
) -> Result {
    let (device, doing) = (committed.device, committed.doing);
    let (sound, links, recipients) = devices.get(device)?;
    if let Some(DeviceSound { sound, at, on_commit: true }) = sound {
        let (path, volume) = (sound.path.clone(), sound.volume);
        effects.write(TriggerEffect::Sound { path, volume, at: *at });
    }
    if links {
        for &recipient in recipients.into_iter().flat_map(|r| &r.0) {
            commands.trigger(Activate { device: recipient, by: doing.by.linked(-1) });
        }
    }
    commands.trigger(Fired { device, doing });
    Ok(())
}

/// A device settles: the variable it changes (a dial has set its own), the
/// message of the use it carried out, and that it has fired.
fn fired(
    fired: On<Fired>,
    mut devices: Query<(&Uses, &mut TriggerState, &LevelOwned, Option<&ChangesState>)>,
    mut levels: Query<(&Level, &mut LevelState)>,
    mut effects: MessageWriter<TriggerEffect>,
) -> Result {
    let (uses, mut state, owner, changes) = devices.get_mut(fired.device)?;
    let chosen = uses.get(fired.doing.slot);
    let (level, mut variables) = levels.get_mut(owner.0)?;
    if let Some(ChangesState(name, change)) = changes
        && !matches!(chosen.action, TriggerAction::Dial)
    {
        let value = change.apply(variables.variable(name));
        if variables.variable(name) != value {
            variables.set_variable(name, value);
        }
    }
    if let Some(id) = chosen.message {
        effects.write(TriggerEffect::Message { source: level.messages.clone(), id });
    }
    if !state.fired {
        state.fired = true;
    }
    Ok(())
}

/// Carry out the story's bidding on the level the party is in.
fn bid(
    bid: On<Bid>,
    levels: Single<(&DeviceNames, &mut LevelState), With<InPlay>>,
    mut states: Query<&mut TriggerState>,
    mut doors: Query<&mut DoorState>,
    mut commands: Commands,
) {
    let (names, mut variables) = levels.into_inner();
    let mut run = |name: &str| {
        if let Some(device) = names.named(name) {
            commands.trigger(Activate { device, by: By::Level });
        }
    };
    match bid.event() {
        Bid::Run(name) => run(name),
        Bid::RunOnce(name) => {
            let state = names.named(name).map(|d| states.get(d).expect("a device"));
            if state.is_some_and(|state| !state.fired) {
                run(name);
            }
        }
        Bid::SwitchOff(name) | Bid::SwitchOn(name) | Bid::Enable(name) => {
            if let Some(device) = names.named(name) {
                let mut state = states.get_mut(device).expect("a device");
                state.on = !matches!(bid.event(), Bid::SwitchOff(_));
            }
            if matches!(bid.event(), Bid::SwitchOn(_)) {
                run(name);
            }
        }
        Bid::Unlock(name) => {
            if let Some(device) = names.named(name) {
                commands.entity(device).remove::<Guarded>();
                if let Ok(mut door) = doors.get_mut(device) {
                    door.locked = false;
                }
            }
        }
        Bid::Set(name, value) => {
            variables.variables.insert(name.clone(), *value);
        }
        Bid::Step(name, count) => {
            let next = (variables.value(name) + 1) % count;
            variables.variables.insert(name.clone(), next);
        }
        Bid::SetBits(name, bits) => {
            let held = variables.value(name);
            variables.variables.insert(name.clone(), held | bits);
        }
    }
}

/// Hurt each living member by `dice`, in roster order.
fn hurt_party(
    hurt: On<HurtParty>,
    roster: Single<&PartyRoster>,
    mut members: Query<&mut Vitals, With<Calling>>,
    mut rng: ResMut<GameRng>,
    mut laying: crate::game::actors::Laying,
    mut damage: MessageWriter<Damaged>,
) {
    for entity in roster.iter() {
        let mut member = members.get_mut(entity).unwrap();
        if member.is_dead() {
            continue;
        }
        let rolled = rng.roll(hurt.0);
        if rolled > 0 {
            crate::game::party::damage_member(
                entity,
                &mut member,
                rolled,
                &mut laying,
                &mut damage,
            );
        }
    }
}
