//! Level trigger records (Whizzardry `Trigger::CreateAndLoadLevelTrigger`).
//!
//! Records appear in a level's trigger section and attached to props and
//! items. Retail ships types 3 (ambient sound) and 4; types 1 and 2 are
//! decoded from the reference code only.

use super::bytes::{Bytes, lenient_string};
use super::mesh_record::RECORD_SCALE;
use super::pvl::{PvlError, invalid, text};

const NAME: usize = 0x80;
const LONG_TEXT: usize = 0x100;

/// Retail trigger action codes used so far.
pub mod action {
    /// Move the party to the recipients: a local trigger or named position,
    /// or `<3-letter location><2-digit entrance>` on another level.
    pub const TELEPORT: u32 = 0x34;
    /// Like [`TELEPORT`] to a randomly chosen recipient.
    pub const RANDOM_TELEPORT: u32 = 0x4b;
    /// Play the named camera path; the party ends at its last key (ladders,
    /// ropes).
    pub const CAMERA_PATH: u32 = 0x09;
    /// Fire the recipients.
    pub const RELAY: u32 = 0x0f;
    /// Toggle a prop's animation; with a loot table, a container.
    pub const TOGGLE: u32 = 0x03;
}

/// A position in retail units with a rotation of `angle` radians about
/// `axis`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TriggerPlacement {
    pub position: [f32; 3],
    pub angle: f32,
    pub axis: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct DoorData {
    pub open: bool,
    pub lockable: bool,
    pub locked: bool,
    pub auto_locking: bool,
    pub one_way: bool,
    pub secret: bool,
    pub found: bool,
    pub jammable: bool,
    pub jammed: bool,
    /// The key item, 0 for none.
    pub item: u16,
    pub position: Option<[f32; 3]>,
    pub linked_trigger: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Trigger {
    /// Upper case, as retail compares names.
    pub name: String,
    pub kind: TriggerKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TriggerKind {
    /// Type 1: a switch fired by range.
    Switch { action: u32, recipients: String, door: Option<DoorData> },
    /// Type 2: a positioned prop trigger.
    Prop { placement: TriggerPlacement, action: u32, recipients: String },
    /// Type 3: an ambient sound emitter.
    AmbientSound(AmbientSound),
    /// Type 4: the general trigger used by retail levels.
    Item(ItemTrigger),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ItemTrigger {
    /// Packed flag bits (`TRIGGER_*`).
    pub flags: u8,
    /// The player may use it (click or walk into it); recipients run
    /// regardless.
    pub enabled: bool,
    /// Run the recipients when the action commits.
    pub fire_linked: bool,
    /// Uses go initial, alternate, initial, … rather than initial once and
    /// then alternate.
    pub alternate_toggles: bool,
    pub initial_action: u32,
    pub alternate_action: u32,
    pub fallback_action: u32,
    pub recipients: String,
    pub data: [i32; 3],
    /// Played on commit when `play_sound`: a file under `data/sound`.
    pub sound: String,
    pub play_sound: bool,
    /// Percent; retail reads 0 as 100.
    pub sound_volume: u8,
    /// Location variables that must all be set, or the fallback action runs.
    pub required_states: Vec<String>,
    /// The location variable the commit changes, and how.
    pub state_to_modify: String,
    pub state_mode: StateMode,
    /// The value `state_to_modify` starts with when the level first loads.
    pub initial_state: Option<bool>,
    pub action_data: String,
    /// The distance band, in retail units, in which a positioned trigger
    /// fires when the party is inside it.
    pub range: [f32; 2],
    /// Positioned triggers (representation kind 1), e.g. level entrances.
    pub placement: Option<TriggerPlacement>,
    pub door: Option<DoorData>,
}

/// Packed trigger flag bits (Whizzardry `World::loadTrigger`).
pub mod flag {
    /// Backed by a prop rather than a place.
    pub const ITEM: u8 = 1;
    /// Show the level message `data[use]` on commit.
    pub const SHOW_MESSAGE: u8 = 2;
    /// Runs at most once.
    pub const ONCE: u8 = 4;
}

/// How a commit changes its location variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateMode {
    Keep,
    Set,
    Clear,
    Toggle,
}

fn read_door(bytes: &mut Bytes) -> Result<DoorData, PvlError> {
    let _version = bytes.u8()?;
    let mut flags = [false; 9];
    for flag in &mut flags {
        *flag = bytes.u8()? & 1 != 0;
    }
    let [open, lockable, locked, auto_locking, one_way, secret, found, jammable, jammed] = flags;
    let item = bytes.u16()?;
    let has_position = bytes.u8()? != 0;
    let position = bytes.f32s::<3>()?.map(|v| v * RECORD_SCALE);
    Ok(DoorData {
        open,
        lockable,
        locked,
        auto_locking,
        one_way,
        secret,
        found,
        jammable,
        jammed,
        item,
        position: has_position.then_some(position),
        linked_trigger: text(bytes, NAME)?.to_uppercase(),
    })
}

/// Older records can carry a 36-vertex blocking shape; retail discards it.
fn skip_legacy_geometry(bytes: &mut Bytes) -> Result<(), PvlError> {
    bytes.take(1 + 36 * 12 + 2)?;
    Ok(())
}

fn placement(bytes: &mut Bytes) -> Result<TriggerPlacement, PvlError> {
    Ok(TriggerPlacement {
        position: bytes.f32s::<3>()?.map(|v| v * RECORD_SCALE),
        angle: bytes.f32()?,
        axis: bytes.f32s()?,
    })
}

pub fn read_trigger(bytes: &mut Bytes) -> Result<Trigger, PvlError> {
    let _record_version = bytes.u8()?;
    match bytes.u8()? {
        1 => read_switch(bytes),
        2 => read_prop_trigger(bytes),
        3 => read_ambient_sound(bytes),
        4 => read_item_trigger(bytes),
        _ => Err(invalid("invalid trigger type", bytes)),
    }
}

fn read_switch(bytes: &mut Bytes) -> Result<Trigger, PvlError> {
    let version = bytes.u8()?;
    // Cycle bounce, state count, animate states, range.
    bytes.take(16)?;
    let action = bytes.i32()? as u32;
    // Value, animate action, packed flags, enabled.
    bytes.take(4 + 4 + 2)?;
    let name = text(bytes, NAME)?.to_uppercase();
    let recipients = text(bytes, LONG_TEXT)?.to_uppercase();
    let _sound = text(bytes, NAME)?;
    if version > 1 {
        // Minimum range, surface id.
        bytes.take(4 + 0x40)?;
    }
    let mut door = None;
    if version > 2 && bytes.u8()? != 0 && bytes.u8()? == 1 {
        door = Some(read_door(bytes)?);
    }
    if version > 3 {
        bytes.i32()?;
    }
    Ok(Trigger { name, kind: TriggerKind::Switch { action, recipients, door } })
}

fn read_prop_trigger(bytes: &mut Bytes) -> Result<Trigger, PvlError> {
    let version = bytes.u8()?;
    let _range = bytes.f32()?;
    let position = bytes.f32s::<3>()?.map(|v| v * RECORD_SCALE);
    let action = bytes.i32()? as u32;
    // Searchable, fire linked, enabled.
    bytes.take(4 + 2)?;
    let name = text(bytes, NAME)?.to_uppercase();
    let recipients = text(bytes, LONG_TEXT)?.to_uppercase();
    if version > 1 {
        // Plane flag and four representation vectors.
        bytes.take(1 + 4 * 12)?;
    }
    let (mut angle, mut axis) = (0.0, [0.0, 1.0, 0.0]);
    if version > 2 {
        angle = bytes.f32()?;
        axis = bytes.f32s()?;
        bytes.u8()?;
        text(bytes, NAME)?;
    }
    if version > 3 {
        bytes.take(1 + 4)?;
    }
    if version > 4 && bytes.u8()? != 0 && bytes.u8()? == 2 {
        skip_legacy_geometry(bytes)?;
    }
    let placement = TriggerPlacement { position, angle, axis };
    Ok(Trigger { name, kind: TriggerKind::Prop { placement, action, recipients } })
}

/// A sound a place makes (type 3 record): looping, or now and then.
#[derive(Clone, Debug, PartialEq)]
pub struct AmbientSound {
    pub wave: String,
    /// Retail units.
    pub position: [f32; 3],
    pub radius: f32,
    pub looping: bool,
    pub volume: [i32; 2],
    pub speed: [i32; 2],
    /// Milliseconds between plays of a one-shot sound; -1 when unset.
    pub interval: [i32; 2],
}

fn read_ambient_sound(bytes: &mut Bytes) -> Result<Trigger, PvlError> {
    let version = bytes.u8()?;
    let volume = bytes.i32s::<2>()?;
    let speed = bytes.i32s::<2>()?;
    let interval = bytes.i32s::<2>()?;
    bytes.i32()?; // unbounded
    let radius = bytes.f32()? * RECORD_SCALE;
    let position = bytes.f32s::<3>()?.map(|v| v * RECORD_SCALE);
    // Region u and v.
    bytes.take(2 * 12)?;
    let wave = text(bytes, NAME)?;
    let mut looping = false;
    if version > 1 {
        bytes.u8()?;
        looping = bytes.u8()? != 0;
    }
    if version > 2 {
        // Region centre, angle, minimum, maximum.
        bytes.take(12 + 4 + 12 + 12)?;
    }
    let name = if version > 3 { text(bytes, NAME)?.to_uppercase() } else { String::new() };
    if version > 4 {
        bytes.u8()?;
    }
    Ok(Trigger {
        name,
        kind: TriggerKind::AmbientSound(AmbientSound {
            wave,
            position,
            radius,
            looping,
            volume,
            speed,
            interval,
        }),
    })
}

fn read_item_trigger(bytes: &mut Bytes) -> Result<Trigger, PvlError> {
    let version = bytes.u8()?;
    let name = text(bytes, NAME)?.to_uppercase();
    let packed_flags = bytes.u8()?;
    let enabled = bytes.u8()? != 0;
    let fire_linked = bytes.u8()? != 0;
    bytes.take(3)?; // link on deactivate, keep on finish, flag
    let alternate_toggles = bytes.u8()? != 0;
    let initial_action = bytes.i32()? as u32;
    let alternate_action = bytes.i32()? as u32;
    let fallback_action = bytes.i32()? as u32;
    let recipients = text(bytes, LONG_TEXT)?.to_uppercase();
    bytes.u8()?; // searchable
    text(bytes, LONG_TEXT)?; // required item
    bytes.take(1 + 4)?; // consume item, action value
    let play_sound = bytes.u8()? != 0;
    let sound = text(bytes, NAME)?;
    let mut data = [0; 3];
    let (mut initial_state, mut sound_volume) = (None, 100);
    if version > 1 {
        data = bytes.i32s()?;
        bytes.f32()?; // representation scale
        initial_state = match bytes.u8()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        };
        bytes.take(2)?; // alternate sound flag, sound mode
        sound_volume = match bytes.u8()? {
            0 => 100,
            volume => volume,
        };
        bytes.take(16)?; // unused
    }
    bytes.take(3)?; // state count, animate states, cycle bounce
    let required_states = text(bytes, LONG_TEXT)?
        .split(',')
        .map(|s| s.trim().to_uppercase())
        .filter(|s| !s.is_empty())
        .collect();
    let state_to_modify = text(bytes, LONG_TEXT)?.trim().to_uppercase();
    let state_mode = match bytes.u8()? {
        1 => StateMode::Set,
        2 => StateMode::Clear,
        3 => StateMode::Toggle,
        _ => StateMode::Keep,
    };
    let range = [bytes.f32()? * RECORD_SCALE, bytes.f32()? * RECORD_SCALE];
    let action_data = lenient_string(bytes.take(LONG_TEXT)?).to_uppercase();
    bytes.i32()?;
    if version > 2 {
        text(bytes, NAME)?; // alternate sound
    }
    let mut placement_record = None;
    if packed_flags & 1 == 0 {
        match bytes.u8()? {
            1 => placement_record = Some(placement(bytes)?),
            2 => {
                bytes.take(4 * 12)?;
            }
            _ => {}
        }
        if bytes.u8()? != 0 {
            // Legacy action: kind, value, sound.
            bytes.take(1 + 4)?;
            text(bytes, NAME)?;
        }
    }
    let mut door = None;
    if bytes.u8()? != 0 {
        match bytes.u8()? {
            1 => door = Some(read_door(bytes)?),
            2 => skip_legacy_geometry(bytes)?,
            _ => {}
        }
    }
    Ok(Trigger {
        name,
        kind: TriggerKind::Item(ItemTrigger {
            flags: packed_flags,
            enabled,
            fire_linked,
            alternate_toggles,
            initial_action,
            alternate_action,
            fallback_action,
            recipients,
            data,
            sound,
            play_sound,
            sound_volume,
            required_states,
            state_to_modify,
            state_mode,
            initial_state,
            action_data,
            range,
            placement: placement_record,
            door,
        }),
    })
}
