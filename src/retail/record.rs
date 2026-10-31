//! Field encodings shared by the retail databases.

use crate::bytes::{Bytes, Truncated};
use crate::content::items::{Dice, UseType};
use crate::content::species::Reach;

/// Dice as retail packs them: a signed base, then the count and sides.
pub fn dice(bytes: &mut Bytes) -> Result<Dice, Truncated> {
    Ok(Dice { base: bytes.i16()? as i32, count: bytes.u8()? as i32, sides: bytes.u8()? as i32 })
}

/// Dice at `at` in a record.
pub fn dice_at(record: &[u8], at: usize) -> Result<Dice, Truncated> {
    dice(&mut Bytes::at(record, at)?)
}

/// A retail range category.
pub fn reach(category: u8) -> Reach {
    match category {
        0 => Reach::Touch,
        1 => Reach::Short,
        2 => Reach::Long,
        _ => Reach::Extreme,
    }
}

/// A retail item use type.
pub fn use_type(value: u8) -> Option<UseType> {
    Some(match value {
        0 => UseType::None,
        1 => UseType::Charged,
        2 => UseType::Scroll,
        3 => UseType::Spellbook,
        4 => UseType::Food,
        5 => UseType::Potion,
        6 => UseType::Instrument,
        7 => UseType::Thrown,
        8 => UseType::Gadget,
        9 => UseType::Flamethrower,
        _ => return None,
    })
}
