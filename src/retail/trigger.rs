//! Level trigger records from a `.pvl` triggers section.
//!
//! Only the type 4 ("item") record is decoded so far, which is enough to read
//! level entrances such as `MO100`. Layout after Whizzardry's
//! `Trigger::CreateAndLoadLevelTrigger`.

use super::bytes::{Bytes, Truncated, c_string};

const NAME: usize = 0x80;
const LONG_TEXT: usize = 0x100;
/// Retail trigger coordinates are stored divided by 500.
const POSITION_SCALE: f32 = 500.0;

/// Where a positioned trigger sits and how it faces, in retail units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TriggerPlacement {
    pub position: [f32; 3],
    /// Rotation of `angle` radians about `axis`.
    pub angle: f32,
    pub axis: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct ItemTrigger {
    pub version: u8,
    pub name: String,
    pub placement: Option<TriggerPlacement>,
}

#[derive(Debug, thiserror::Error)]
pub enum TriggerError {
    #[error(transparent)]
    Truncated(#[from] Truncated),
    #[error("unterminated text field at offset {0}")]
    Unterminated(usize),
    #[error("record type {0} is not a type 4 trigger")]
    WrongType(u8),
}

fn text(bytes: &mut Bytes, len: usize) -> Result<String, TriggerError> {
    let offset = bytes.offset();
    c_string(bytes.take(len)?).ok_or(TriggerError::Unterminated(offset))
}

/// Read a type 4 trigger record, starting at its record version byte.
pub fn read_item_trigger(bytes: &mut Bytes) -> Result<ItemTrigger, TriggerError> {
    let _record_version = bytes.u8()?;
    let record_type = bytes.u8()?;
    if record_type != 4 {
        return Err(TriggerError::WrongType(record_type));
    }
    let version = bytes.u8()?;
    let name = text(bytes, NAME)?;
    let packed_flags = bytes.u8()?;
    bytes.take(6)?; // enabled, fire/deactivate links, keep, flag, alternate toggles
    bytes.take(12)?; // initial, alternate and fallback actions
    text(bytes, LONG_TEXT)?; // recipients
    bytes.u8()?; // searchable
    text(bytes, LONG_TEXT)?; // location variable
    bytes.take(1 + 4 + 1)?; // consume item, action value, animate action
    text(bytes, NAME)?; // sound
    if version > 1 {
        // Three data words, representation scale, initial location value,
        // flag, action data mode, sound volume, four unused words.
        bytes.take(12 + 4 + 4 + 16)?;
    }
    bytes.take(3)?; // state count, animate states, cycle bounce
    text(bytes, LONG_TEXT)?; // required states
    text(bytes, LONG_TEXT)?; // state to modify
    bytes.take(1 + 8)?; // modification mode, range minimum and maximum
    text(bytes, LONG_TEXT)?; // inline action data
    bytes.i32()?;
    if version > 2 {
        text(bytes, NAME)?; // alternate sound
    }
    let mut placement = None;
    if packed_flags & 1 == 0 && bytes.u8()? == 1 {
        let position = bytes.f32s::<3>()?.map(|v| v * POSITION_SCALE);
        let angle = bytes.f32()?;
        let axis = bytes.f32s()?;
        placement = Some(TriggerPlacement { position, angle, axis });
    }
    Ok(ItemTrigger { version, name, placement })
}

/// Find a type 4 trigger by name.
///
/// The sections before the triggers (lights, props, cameras, ...) are not
/// decoded yet, so this locates the record by its name field and accepts it
/// only if the bytes before it are a type 4 record header and the whole
/// record parses. Replace with a sequential section reader when one exists.
pub fn find_item_trigger(data: &[u8], name: &str) -> Option<ItemTrigger> {
    let mut needle = name.as_bytes().to_vec();
    needle.push(0);
    let mut from = 0;
    while let Some(found) =
        data[from..].windows(needle.len()).position(|w| w.eq_ignore_ascii_case(&needle))
    {
        let at = from + found;
        from = at + 1;
        let Some(start) = at.checked_sub(3) else { continue };
        if data[start + 1] != 4 {
            continue;
        }
        if let Ok(trigger) = read_item_trigger(&mut Bytes::new(&data[start..]))
            && trigger.name.eq_ignore_ascii_case(name)
        {
            return Some(trigger);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_field(text: &str, len: usize) -> Vec<u8> {
        let mut field = text.as_bytes().to_vec();
        field.resize(len, 0);
        field
    }

    fn record(name: &str, placement: Option<([f32; 3], f32, [f32; 3])>) -> Vec<u8> {
        let mut r = vec![1, 4, 3];
        r.extend(text_field(name, NAME));
        r.push(if placement.is_some() { 0 } else { 1 });
        r.extend([0; 6 + 12]);
        r.extend(text_field("", LONG_TEXT));
        r.push(0);
        r.extend(text_field("", LONG_TEXT));
        r.extend([0; 6]);
        r.extend(text_field("", NAME));
        r.extend([0; 36 + 3]);
        r.extend(text_field("", LONG_TEXT));
        r.extend(text_field("", LONG_TEXT));
        r.extend([0; 9]);
        r.extend(text_field("", LONG_TEXT));
        r.extend([0; 4]);
        r.extend(text_field("", NAME));
        if let Some((position, angle, axis)) = placement {
            r.push(1);
            for v in position.into_iter().chain([angle]).chain(axis) {
                r.extend(v.to_le_bytes());
            }
        }
        r
    }

    #[test]
    fn reads_a_positioned_entrance() {
        let mut data = vec![0xAB; 17];
        data.extend(record("MO100", Some(([2.0, 0.5, -1.0], 1.5, [0.0, -1.0, 0.0]))));
        let trigger = find_item_trigger(&data, "mo100").unwrap();
        let placement = trigger.placement.unwrap();
        assert_eq!(placement.position, [1000.0, 250.0, -500.0]);
        assert_eq!(placement.angle, 1.5);
        assert_eq!(placement.axis, [0.0, -1.0, 0.0]);
    }

    #[test]
    fn ignores_names_outside_type_4_records() {
        let mut data = b"xxMO100\0".to_vec();
        data.extend(record("OTHER", None));
        assert!(find_item_trigger(&data, "MO100").is_none());
    }

    /// Runs against a real installation when `RIZZARDRY_RETAIL` names one.
    #[test]
    fn finds_the_monastery_entrance() {
        let Ok(root) = std::env::var("RIZZARDRY_RETAIL") else { return };
        let files = crate::retail::files::RetailFiles::open(root).unwrap();
        let data = files.read("levels/monastery/monastery1.pvl").unwrap().unwrap();
        let placement = find_item_trigger(&data, "MO100").unwrap().placement.unwrap();
        let [x, _, z] = placement.position;
        assert!((75_000.0..77_000.0).contains(&x) && (52_000.0..53_000.0).contains(&z));
    }
}
