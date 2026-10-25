//! The retail item database (`data/databases/items.dbs`).
//!
//! After Whizzardry `ItemDatabaseRecord` (`include/wiz8/retail_records.h`,
//! `include/wiz8/data/item_payloads.h`): a count, then 269-byte records.

use super::bytes::Bytes;
use super::pvl::{PvlError, invalid};

const RECORD: usize = 0x10d;

/// `count` d `sides` + `base`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dice {
    pub count: u8,
    pub sides: u8,
    pub base: i16,
}

/// A stat or skill an item asks a minimum of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requirement {
    pub id: u8,
    pub minimum: u8,
}

/// One item definition as retail stores it.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemRecord {
    pub name: String,
    /// The string table entry shown before the item is identified.
    pub unidentified_name: u16,
    pub equip_class: u8,
    /// Flag bits (`item_flag`).
    pub flags: u8,
    pub category: u8,
    /// The weapon skill it trains; none for non-weapons.
    pub weapon_skill: Option<u8>,
    pub range_category: u8,
    pub damage_bonus: i8,
    pub hit_bonus: i8,
    pub damage: Dice,
    pub armor_class_bonus: i8,
    pub spell: u8,
    pub spell_power: u8,
    pub quantity_kind: u8,
    pub initial_quantity: Dice,
    pub maximum_quantity: u8,
    pub profession_mask: u16,
    pub race_mask: u32,
    pub gender_mask: u8,
    pub attribute_requirements: Vec<Requirement>,
    pub skill_requirements: Vec<Requirement>,
    pub identify_difficulty: u8,
    pub value: u32,
    /// Tenths of a pound.
    pub weight: u16,
    pub binds_on_equip: bool,
    pub internal_name: String,
    /// The model shown when the item lies in the world.
    pub model: String,
}

/// Item flag bits (Whizzardry `W8ItemFlag`).
pub mod item_flag {
    pub const AUTO_IDENTIFY: u8 = 0x01;
    pub const NO_DISCARD: u8 = 0x02;
    pub const TWO_HANDED: u8 = 0x04;
    pub const OFF_HAND_ALLOWED: u8 = 0x08;
    pub const MERCHANT_CANNOT_SELL: u8 = 0x10;
    pub const CONTAINER: u8 = 0x20;
    pub const MUST_EQUIP_TO_USE: u8 = 0x40;
    pub const NEVER_DEPLETES: u8 = 0x80;
}

fn dice(bytes: &mut Bytes) -> Result<Dice, PvlError> {
    let base = bytes.u16()? as i16;
    Ok(Dice { base, count: bytes.u8()?, sides: bytes.u8()? })
}

fn requirements(bytes: &mut Bytes) -> Result<Vec<Requirement>, PvlError> {
    let mut found = Vec::new();
    for _ in 0..2 {
        let (id, minimum) = (bytes.u8()?, bytes.u8()?);
        // Retail stops at an id of 0xff.
        if id != 0xff {
            found.push(Requirement { id, minimum });
        }
    }
    Ok(found)
}

fn ascii(raw: &[u8]) -> String {
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end]).into_owned()
}

fn read_item(record: &[u8]) -> Result<ItemRecord, PvlError> {
    let units: Vec<u16> =
        record[..60].as_chunks::<2>().0.iter().map(|&pair| u16::from_le_bytes(pair)).collect();
    let name_end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    let name = String::from_utf16_lossy(&units[..name_end]);
    let mut bytes = Bytes::new(&record[0x3c..]);
    bytes.u16()?; // Wizardry 7 item number
    let equip_class = bytes.u8()?;
    let unidentified_name = bytes.u16()?;
    let flags = bytes.u8()?;
    let category = bytes.u8()?;
    bytes.take(3)?;
    let weapon_skill = match bytes.i8()? {
        skill if skill < 0 => None,
        skill => Some(skill as u8),
    };
    let range_category = bytes.u8()?;
    let damage_bonus = bytes.i8()?;
    let hit_bonus = bytes.i8()?;
    let damage = dice(&mut bytes)?;
    bytes.u16()?; // attack flags
    bytes.take(16 + 1 + 1)?; // missile modifiers, magnitude, slays
    let armor_class_bonus = bytes.i8()?;
    let spell = bytes.u8()?;
    let spell_power = bytes.u8()?;
    bytes.u8()?;
    let quantity_kind = bytes.u8()?;
    let initial_quantity = dice(&mut bytes)?;
    let maximum_quantity = bytes.u8()?;
    bytes.take(3 + 6 + 1)?; // regeneration, resistances, property
    let profession_mask = bytes.u16()?;
    let race_mask = bytes.u32()?;
    let gender_mask = bytes.u8()?;
    let attribute_requirements = requirements(&mut bytes)?;
    let skill_requirements = requirements(&mut bytes)?;
    let identify_difficulty = bytes.u8()?;
    let value = bytes.u32()?;
    let weight = bytes.u16()?;
    let binds_on_equip = bytes.u8()? != 0;
    Ok(ItemRecord {
        name,
        unidentified_name,
        equip_class,
        flags,
        category,
        weapon_skill,
        range_category,
        damage_bonus,
        hit_bonus,
        damage,
        armor_class_bonus,
        spell,
        spell_power,
        quantity_kind,
        initial_quantity,
        maximum_quantity,
        profession_mask,
        race_mask,
        gender_mask,
        attribute_requirements,
        skill_requirements,
        identify_difficulty,
        value,
        weight,
        binds_on_equip,
        internal_name: ascii(&record[0x8d..0x8d + 0x20]),
        model: ascii(&record[0xcd..0xcd + 0x18]),
    })
}

/// Every item definition, indexed by item number.
pub fn read_items(data: &[u8]) -> Result<Vec<ItemRecord>, PvlError> {
    let mut bytes = Bytes::new(data);
    let count = bytes.u32()? as usize;
    if data.len() - 4 < count * RECORD {
        return Err(invalid("item database shorter than its count", &bytes));
    }
    data[4..4 + count * RECORD]
        .as_chunks::<RECORD>()
        .0
        .iter()
        .map(|record| read_item(record))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a retail installation"]
    fn retail_items_decode() {
        let root =
            std::env::var("RIZZARDRY_RETAIL").expect("set RIZZARDRY_RETAIL to run this test");
        let files = crate::retail::files::RetailFiles::open(root).unwrap();
        let items = read_items(&files.read("data/databases/items.dbs").unwrap().unwrap()).unwrap();
        for (i, item) in items.iter().enumerate().take(8) {
            println!("{i}: {item:?}");
        }
        println!("{} items", items.len());
        assert!(items.len() > 500);
        assert!(items.iter().all(|item| item.equip_class <= 31), "equip classes in range");
        assert!(items.iter().filter(|item| !item.name.is_empty()).count() > 500);
    }
}
