//! What a device's use does, one act each (retail `Trigger::RunAction`).
//! An act that comes off is [`Committed`]; one that does not declines, and
//! the player hears that nothing happened.

use bevy::prelude::*;

use super::walk::{ActiveDamageZone, WalkVolume};
use super::{
    Act, Activate, Committed, DeviceSound, DialState, Door, DoorState, HurtParty, LevelState,
    LightActivation, PartyMove, PendingPartyMove, Recipients, TriggerEffect, TriggerState,
};
use crate::content::items::Dice;
use crate::content::level::{Animation, Arrival, StateChange};
use crate::game::rng::GameRng;
use crate::game::world::level::LevelOwned;
use crate::game::world::particles::Emitter;
use crate::game::world::props::{PropAnimation, PropMotion};

/// An editor's placeholder: the original returns without committing.
#[derive(Clone, Copy, Debug)]
pub struct Idle;

/// The device's sound, without committing.
#[derive(Clone, Copy, Debug)]
pub struct Sound;

/// The camera distorted, or put right, by the source a relay passes on.
#[derive(Clone, Copy, Debug)]
pub struct Sway;

/// Hurt whoever stands in the device's place, every half second.
#[derive(Clone, Copy, Debug)]
pub struct Zone;

/// A dial turned to its next position.
#[derive(Clone, Copy, Debug)]
pub struct Dial;

/// Run the recipients, passing this source on.
#[derive(Clone, Copy, Debug)]
pub struct Relay(pub i32);

/// Named lights switched, each with its authored start.
#[derive(Clone, Debug)]
pub struct Lights(pub Vec<(String, bool)>, pub StateChange);

/// The level's particle emitters, by their place among its emitters.
#[derive(Clone, Debug)]
pub struct Particles(pub Vec<usize>, pub StateChange);

/// The level's named ambient sounds.
#[derive(Clone, Debug)]
pub struct AmbientSounds(pub Vec<String>, pub StateChange);

pub(super) fn plugin(app: &mut App) {
    app.add_observer(idle)
        .add_observer(sound)
        .add_observer(sway)
        .add_observer(zone)
        .add_observer(dial)
        .add_observer(door)
        .add_observer(animate)
        .add_observer(relay)
        .add_observer(set_devices)
        .add_observer(lights)
        .add_observer(particles)
        .add_observer(ambient_sounds)
        .add_observer(move_party)
        .add_observer(random_teleport)
        .add_observer(hurt)
        .add_observer(shake);
}

fn idle(act: On<Act<Idle>>, mut effects: MessageWriter<TriggerEffect>) {
    act.doing.decline(&mut effects);
}

fn sound(
    act: On<Act<Sound>>,
    sounds: Query<&DeviceSound>,
    mut effects: MessageWriter<TriggerEffect>,
) {
    if let Ok(DeviceSound { sound, at, .. }) = sounds.get(act.device) {
        let (path, volume) = (sound.path.clone(), sound.volume);
        effects.write(TriggerEffect::Sound { path, volume, at: *at });
    }
    act.doing.decline(&mut effects);
}

/// It answers, without committing.
fn sway(act: On<Act<Sway>>, owners: Query<&LevelOwned>, mut levels: Query<&mut LevelState>) {
    if let Ok(mut level) = owners.get(act.device).and_then(|owner| levels.get_mut(owner.0)) {
        level.camera_sway = act.doing.by.source() > 0;
    }
}

/// A place that hurts starts hurting; a device that is not a place does
/// nothing more.
fn zone(act: On<Act<Zone>>, places: Query<(), With<WalkVolume>>, mut commands: Commands) {
    if places.contains(act.device) {
        commands.entity(act.device).insert(ActiveDamageZone::new());
    }
    commands.trigger(Committed { device: act.device, doing: act.doing });
}

/// The dial moves on a slot, back to the first after the last, and the
/// numbered variable of the slot it left is cleared and the next one's
/// set.
fn dial(
    act: On<Act<Dial>>,
    mut dials: Query<(&PropAnimation, &mut DialState, &mut PropMotion, &LevelOwned)>,
    mut levels: Query<&mut LevelState>,
    mut effects: MessageWriter<TriggerEffect>,
    mut commands: Commands,
) {
    let Ok((model, mut dial, mut motion, owner)) = dials.get_mut(act.device) else {
        return act.doing.decline(&mut effects);
    };
    let old = dial.position;
    let slots = &model.slots;
    let next = if slots.len() < 3 {
        0
    } else {
        let next = (old + 1) % (slots.len() - 1);
        motion.frame = f32::from(slots[next]);
        motion.target = f32::from(slots[next + 1]);
        next
    };
    if old != next {
        dial.position = next;
    }
    if let Some(name) = &dial.variable
        && let Ok(mut level) = levels.get_mut(owner.0)
    {
        level.set_variable(&name.numbered(old), false);
        level.set_variable(&name.numbered(next), true);
    }
    commands.trigger(Committed { device: act.device, doing: act.doing });
}

/// A locked door says so; one moving, or already as asked, does nothing.
fn door(
    act: On<Act<Door>>,
    mut doors: Query<(&PropAnimation, &mut DoorState, &mut PropMotion, &mut TriggerState)>,
    mut effects: MessageWriter<TriggerEffect>,
    mut commands: Commands,
) {
    let Ok((model, mut door, mut motion, mut state)) = doors.get_mut(act.device) else {
        return act.doing.decline(&mut effects);
    };
    if door.locked {
        effects.write(TriggerEffect::Locked);
        return act.doing.decline(&mut effects);
    }
    let open = act.what.open_only || !door.open;
    if motion.frame != motion.target || door.open == open {
        return act.doing.decline(&mut effects);
    }
    door.open = open;
    motion.target = if open { model.last_frame } else { 0.0 };
    state.awaits_animation = true;
    commands.trigger(Committed { device: act.device, doing: act.doing });
}

/// The prop plays; a door's prop playing opens or shuts it.
fn animate(
    act: On<Act<Animation>>,
    mut props: Query<(&PropAnimation, &mut PropMotion, &mut TriggerState, Option<&mut DoorState>)>,
    mut effects: MessageWriter<TriggerEffect>,
    mut commands: Commands,
) {
    let Ok((model, motion, mut state, door)) = props.get_mut(act.device) else {
        return act.doing.decline(&mut effects);
    };
    PropMotion::activate(motion, model, act.what);
    if let Some(mut door) = door {
        door.open = !door.open;
    }
    state.awaits_animation = true;
    commands.trigger(Committed { device: act.device, doing: act.doing });
}

/// The recipients run, a link deeper, with the source passed on.
fn relay(
    act: On<Act<Relay>>,
    recipients: Query<&Recipients>,
    mut effects: MessageWriter<TriggerEffect>,
    mut commands: Commands,
) {
    let Ok(Recipients(linked)) = recipients.get(act.device) else {
        return act.doing.decline(&mut effects);
    };
    for &recipient in linked {
        commands.trigger(Activate { device: recipient, by: act.doing.by.linked(act.what.0) });
    }
    commands.trigger(Committed { device: act.device, doing: act.doing });
}

/// The recipients are switched on, off, or over.
fn set_devices(
    act: On<Act<StateChange>>,
    recipients: Query<&Recipients>,
    mut states: Query<&mut TriggerState>,
    mut effects: MessageWriter<TriggerEffect>,
    mut commands: Commands,
) {
    let Ok(Recipients(linked)) = recipients.get(act.device) else {
        return act.doing.decline(&mut effects);
    };
    for &recipient in linked {
        if let Ok(mut state) = states.get_mut(recipient) {
            let on = act.what.apply(state.on);
            if state.on != on {
                state.on = on;
            }
        }
    }
    commands.trigger(Committed { device: act.device, doing: act.doing });
}

fn lights(
    act: On<Act<Lights>>,
    owners: Query<&LevelOwned>,
    mut levels: Query<&mut LightActivation>,
    mut effects: MessageWriter<TriggerEffect>,
    mut commands: Commands,
) {
    let Lights(sources, change) = &act.what;
    let level = owners.get(act.device).and_then(|owner| levels.get_mut(owner.0));
    let (Ok(mut lights), false) = (level, sources.is_empty()) else {
        return act.doing.decline(&mut effects);
    };
    for (name, initial) in sources {
        let next = change.apply(lights.light_enabled(name, *initial));
        lights.lights.insert(name.to_uppercase(), next);
    }
    commands.trigger(Committed { device: act.device, doing: act.doing });
}

fn particles(
    act: On<Act<Particles>>,
    owners: Query<&LevelOwned>,
    mut emitters: Query<(&mut Emitter, &LevelOwned)>,
    mut effects: MessageWriter<TriggerEffect>,
    mut commands: Commands,
) {
    let Particles(sources, change) = &act.what;
    let Ok(&LevelOwned(level)) = owners.get(act.device) else { return };
    let mut any = false;
    for (mut emitter, owner) in &mut emitters {
        if owner.0 == level && sources.contains(&emitter.definition()) {
            let on = change.apply(emitter.emitting());
            emitter.set_active(on);
            any = true;
        }
    }
    if !any {
        return act.doing.decline(&mut effects);
    }
    commands.trigger(Committed { device: act.device, doing: act.doing });
}

fn ambient_sounds(
    act: On<Act<AmbientSounds>>,
    owners: Query<&LevelOwned>,
    mut levels: Query<&mut LevelState>,
    mut effects: MessageWriter<TriggerEffect>,
    mut commands: Commands,
) {
    let AmbientSounds(names, change) = &act.what;
    let level = owners.get(act.device).and_then(|owner| levels.get_mut(owner.0));
    let (Ok(mut level), false) = (level, names.is_empty()) else {
        return act.doing.decline(&mut effects);
    };
    for name in names {
        let next = change.apply(level.ambient_sound_enabled(name));
        level.ambient_sounds.insert(name.to_uppercase(), next);
    }
    commands.trigger(Committed { device: act.device, doing: act.doing });
}

/// The party is to be moved, once the devices are done.
fn move_party(act: On<Act<PartyMove>>, owners: Query<&LevelOwned>, mut commands: Commands) {
    let Ok(&LevelOwned(level)) = owners.get(act.device) else { return };
    commands.entity(level).insert(PendingPartyMove(act.what.clone()));
    commands.trigger(Committed { device: act.device, doing: act.doing });
}

/// One of several places, chosen as it runs. The original takes the wall
/// clock's remainder; the game's random numbers stand in.
fn random_teleport(
    act: On<Act<Vec<Arrival>>>,
    owners: Query<&LevelOwned>,
    mut rng: ResMut<GameRng>,
    mut effects: MessageWriter<TriggerEffect>,
    mut commands: Commands,
) {
    let (Ok(&LevelOwned(level)), false) = (owners.get(act.device), act.what.is_empty()) else {
        return act.doing.decline(&mut effects);
    };
    let arrival = act.what[rng.below(act.what.len() as u32) as usize].clone();
    commands.entity(level).insert(PendingPartyMove(PartyMove::Teleport(arrival)));
    commands.trigger(Committed { device: act.device, doing: act.doing });
}

fn hurt(act: On<Act<Dice>>, mut commands: Commands) {
    commands.trigger(HurtParty(act.what));
    commands.trigger(Committed { device: act.device, doing: act.doing });
}

fn shake(
    act: On<Act<TriggerEffect>>,
    mut effects: MessageWriter<TriggerEffect>,
    mut commands: Commands,
) {
    effects.write(act.what.clone());
    commands.trigger(Committed { device: act.device, doing: act.doing });
}
