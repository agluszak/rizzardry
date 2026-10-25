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
    AmbientSound { wave: String, position: [f32; 3], radius: f32, looping: bool },
    /// Type 4: the general trigger used by retail levels.
    Item(ItemTrigger),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ItemTrigger {
    pub enabled: bool,
    pub initial_action: u32,
    pub alternate_action: u32,
    pub fallback_action: u32,
    pub recipients: String,
    pub data: [i32; 3],
    pub action_data: String,
    /// Positioned triggers (representation kind 1), e.g. level entrances.
    pub placement: Option<TriggerPlacement>,
    pub door: Option<DoorData>,
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

fn read_ambient_sound(bytes: &mut Bytes) -> Result<Trigger, PvlError> {
    let version = bytes.u8()?;
    // Volume, speed and time ranges, unbounded.
    bytes.take(7 * 4)?;
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
    Ok(Trigger { name, kind: TriggerKind::AmbientSound { wave, position, radius, looping } })
}

fn read_item_trigger(bytes: &mut Bytes) -> Result<Trigger, PvlError> {
    let version = bytes.u8()?;
    let name = text(bytes, NAME)?.to_uppercase();
    let packed_flags = bytes.u8()?;
    let enabled = bytes.u8()? != 0;
    // Fire linked, link on deactivate, keep on finish, flag, alternate toggles.
    bytes.take(5)?;
    let initial_action = bytes.i32()? as u32;
    let alternate_action = bytes.i32()? as u32;
    let fallback_action = bytes.i32()? as u32;
    let recipients = text(bytes, LONG_TEXT)?.to_uppercase();
    bytes.u8()?; // searchable
    text(bytes, LONG_TEXT)?; // required item
    bytes.take(1 + 4 + 1)?; // consume item, action value, animate action
    text(bytes, NAME)?; // sound
    let mut data = [0; 3];
    if version > 1 {
        data = bytes.i32s()?;
        // Representation scale, initial location value, alternate action
        // flag, action data mode, sound volume, four unused words.
        bytes.take(4 + 4 + 16)?;
    }
    bytes.take(3)?; // state count, animate states, cycle bounce
    text(bytes, LONG_TEXT)?; // required states
    text(bytes, LONG_TEXT)?; // state to modify
    bytes.take(1 + 8)?; // modification mode, range minimum and maximum
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
            enabled,
            initial_action,
            alternate_action,
            fallback_action,
            recipients,
            data,
            action_data,
            placement: placement_record,
            door,
        }),
    })
}
