//! The retail item databases: definitions (`data/databases/items.dbs`, a
//! count, then 269-byte records after Whizzardry `ItemDatabaseRecord`) and
//! loot tables (`itemtables.dbs`), decoded straight into the content types.

use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::reflect::TypePath;
use thiserror::Error;

use super::bytes::{Bytes, Truncated};
use super::pvl::{PvlError, invalid};
use crate::content::items::{
    Dice, EquipClass, ItemCatalog, ItemDefinition, ItemId, LootTable, LootTables, QuantityKind,
    Requirement,
};

const RECORD: usize = 0x10d;
const TABLE_RECORD: usize = 256 + 4 + 40 * 5 + 1 + 4 + 4 + 4 + 0x18;

#[derive(Debug, Error)]
pub enum ItemLoadError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Data(#[from] PvlError),
    #[error(transparent)]
    Truncated(#[from] Truncated),
    #[error("item {0}: unknown equipment class {1}")]
    EquipClass(usize, u8),
    #[error("item {0}: unknown quantity kind {1}")]
    QuantityKind(usize, u8),
    #[error("item {0}: kind {1} has no image")]
    Image(usize, u16),
}

/// Images for items without their own, by kind (unidentified name), from
/// Whizzardry's `g_item_video_object_fallback_names` (hard-coded in retail).
const KIND_IMAGES: [&str; 145] = [
    "dagger",
    "longsword",
    "bipennis",
    "battleaxe",
    "flail",
    "mace",
    "hammer",
    "shortstaff",
    "halberd",
    "spear",
    "bostaff",
    "longbow",
    "crossbow",
    "sling",
    "greatsword",
    "rapier",
    "katana",
    "longstaff",
    "wand",
    "magicstave",
    "questionmark",
    "shuriken",
    "arrow",
    "slingshot",
    "kiteshield",
    "basnet",
    "platetorso",
    "platelegs",
    "gauntletsteel",
    "sollerets",
    "amulet",
    "ring",
    "bluepotion",
    "scroll",
    "powderpouch",
    "key",
    "locket",
    "greatbow",
    "roundshield",
    "wizardcone",
    "leatherhelm",
    "cuirass",
    "leatherhauberk",
    "greaves",
    "leatherpants",
    "gloves",
    "sandals",
    "buskins",
    "boots",
    "book",
    "ankh",
    "ninjato",
    "warhammer",
    "flamberge",
    "bullwhip",
    "sai",
    "nunchuka",
    "glaive",
    "blacksword",
    "firesword",
    "robeupper",
    "robelower",
    "haltertop",
    "skirt",
    "skullcap",
    "featheredcap",
    "mitre",
    "questionmark",
    "kabuto",
    "do-maruupper",
    "tosei-gusokulower",
    "burgonet",
    "furlegs",
    "cloak",
    "questionmark",
    "questionmark",
    "garland",
    "catoninetails",
    "silvercross",
    "questionmark",
    "questionmark",
    "bracelet",
    "necklace",
    "quarrel",
    "questionmark",
    "coif",
    "hauberk",
    "chainlower",
    "mailmittens",
    "chainhosen",
    "lance",
    "armet",
    "ninjacowl",
    "ninjagarbupper",
    "ninjagarblower",
    "tabiboots",
    "heaume",
    "boxhelm",
    "diamondring",
    "bag",
    "redpotion",
    "greenpotion",
    "purplepotion",
    "yellowpotion",
    "questionmark",
    "t'rangstaff",
    "gauntletbronze",
    "questionmark",
    "lightsword",
    "questionmark",
    "questionmark",
    "questionmark",
    "dart",
    "powdershot",
    "musket",
    "questionmark",
    "stone",
    "stick",
    "shortsword",
    "questionmark",
    "bagpipes",
    "lyre",
    "lute",
    "horn",
    "questionmark",
    "bracelet2",
    "shuriken2",
    "questionmark",
    "wand2",
    "yellowpotion2",
    "mediumshield",
    "omnigun_1",
    "questionmark",
    "questionmark",
    "questionmark",
    "questionmark",
    "saxaphone",
    "violin",
    "drum",
    "bullroar",
    "questionmark",
    "emptybottle",
    "stix",
    "rocket",
    "rocketlauncher",
];

/// Item flag bits (Whizzardry `W8ItemFlag`).
mod flag {
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

/// The item's picture: its own (`video_object_name`), else its kind's
/// (retail `ItemSprites`).
fn image_path(own: &str, kind: u16) -> Option<String> {
    let name = match own {
        "" => (*KIND_IMAGES.get(usize::from(kind))?).to_owned(),
        own => own.to_lowercase(),
    };
    let name = name.strip_suffix(".sti").unwrap_or(&name);
    Some(format!("data/items/{name}.sti"))
}

fn read_item(index: usize, record: &[u8]) -> Result<ItemDefinition, ItemLoadError> {
    let units: Vec<u16> =
        record[..60].as_chunks::<2>().0.iter().map(|&pair| u16::from_le_bytes(pair)).collect();
    let name_end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    let mut bytes = Bytes::new(&record[0x3c..]);
    bytes.u16()?; // Wizardry 7 item number
    let equip_class = bytes.u8()?;
    let kind = bytes.u16()?;
    let flags = bytes.u8()?;
    let has = |bit| flags & bit != 0;
    let category = bytes.u8()?;
    bytes.take(3)?;
    let weapon_skill = u8::try_from(bytes.i8()?).ok();
    let range = bytes.u8()?;
    let damage_bonus = bytes.i8()?;
    let hit_bonus = bytes.i8()?;
    let damage = dice(&mut bytes)?;
    bytes.u16()?; // attack flags
    bytes.take(16 + 1 + 1)?; // effect chances, magnitude, slays
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
    let cursed = bytes.u8()? != 0;
    let own_image = ascii(&record[0xcd..0xcd + 0x18]);
    Ok(ItemDefinition {
        id: ItemId(index as u16),
        name: String::from_utf16_lossy(&units[..name_end]),
        internal_name: ascii(&record[0x8d..0x8d + 0x20]),
        kind,
        image: image_path(&own_image, kind).ok_or(ItemLoadError::Image(index, kind))?,
        class: EquipClass::from_retail(equip_class)
            .ok_or(ItemLoadError::EquipClass(index, equip_class))?,
        category,
        auto_identify: has(flag::AUTO_IDENTIFY),
        crucial: has(flag::NO_DISCARD),
        two_handed: has(flag::TWO_HANDED),
        off_hand: has(flag::OFF_HAND_ALLOWED),
        merchants_refuse: has(flag::MERCHANT_CANNOT_SELL),
        container: has(flag::CONTAINER),
        equip_to_use: has(flag::MUST_EQUIP_TO_USE),
        never_depletes: has(flag::NEVER_DEPLETES),
        weapon_skill,
        range,
        damage,
        damage_bonus,
        hit_bonus,
        armor_class_bonus,
        spell,
        spell_power,
        quantity: match quantity_kind {
            0 => QuantityKind::None,
            1 => QuantityKind::Stack,
            2 => QuantityKind::Charges,
            3 => QuantityKind::Uses,
            4 => QuantityKind::Shots,
            5 => QuantityKind::Unlabelled,
            other => return Err(ItemLoadError::QuantityKind(index, other)),
        },
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
        cursed,
    })
}

/// Every item definition, indexed by item number.
pub fn read_item_catalog(data: &[u8]) -> Result<ItemCatalog, ItemLoadError> {
    let mut bytes = Bytes::new(data);
    let count = bytes.u32()? as usize;
    if data.len() - 4 < count * RECORD {
        return Err(invalid("item database shorter than its count", &bytes).into());
    }
    let records = data[4..4 + count * RECORD].as_chunks::<RECORD>().0;
    Ok(ItemCatalog(
        records.iter().enumerate().map(|(i, r)| read_item(i, r)).collect::<Result<_, _>>()?,
    ))
}

/// The loot tables (Whizzardry `W8ItemTableRecord`), after their category
/// names.
pub fn read_loot_tables(data: &[u8]) -> Result<LootTables, PvlError> {
    let mut bytes = Bytes::new(data);
    let categories = bytes.u32()? as usize;
    bytes.take(categories * 256)?;
    let count = bytes.u32()? as usize;
    let tables = (0..count)
        .map(|_| {
            let record = bytes.take(TABLE_RECORD)?;
            let mut fields = Bytes::new(&record[256..]);
            fields.u32()?; // category
            let mut entries = Vec::new();
            for _ in 0..40 {
                let selector = fields.u16()?;
                let (item, weight) = (fields.u16()?, fields.u8()?);
                if selector != 0 {
                    entries.push((ItemId(item), weight));
                }
            }
            let level_scaled = fields.u8()? != 0;
            let item_count = dice(&mut fields)?;
            fields.take(4)?;
            let gold = dice(&mut fields)?;
            Ok(LootTable { name: ascii(&record[..256]), entries, level_scaled, item_count, gold })
        })
        .collect::<Result<_, PvlError>>()?;
    Ok(LootTables(tables))
}

#[derive(Default, TypePath)]
pub struct ItemCatalogLoader;

impl AssetLoader for ItemCatalogLoader {
    type Asset = ItemCatalog;
    type Settings = ();
    type Error = ItemLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<ItemCatalog, ItemLoadError> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).await?;
        read_item_catalog(&data)
    }

    fn extensions(&self) -> &[&str] {
        &["dbs"]
    }
}

#[derive(Default, TypePath)]
pub struct LootTablesLoader;

impl AssetLoader for LootTablesLoader {
    type Asset = LootTables;
    type Settings = ();
    type Error = ItemLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<LootTables, ItemLoadError> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).await?;
        Ok(read_loot_tables(&data)?)
    }

    fn extensions(&self) -> &[&str] {
        &["dbs"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> crate::retail::files::RetailFiles {
        let root =
            std::env::var("RIZZARDRY_RETAIL").expect("set RIZZARDRY_RETAIL to run this test");
        crate::retail::files::RetailFiles::open(root).unwrap()
    }

    #[test]
    #[ignore = "requires a retail installation"]
    fn retail_items_decode() {
        let files = files();
        let catalog =
            read_item_catalog(&files.read("data/databases/items.dbs").unwrap().unwrap()).unwrap();
        assert_eq!(catalog.0.len(), 819);
        let dagger = catalog.get(ItemId(0)).unwrap();
        assert_eq!((dagger.name.as_str(), dagger.class), ("Dagger", EquipClass::ShortWeapon));
        assert!(dagger.off_hand && !dagger.two_handed);
        assert!(catalog.0.iter().filter(|i| !i.name.is_empty()).count() > 500);
        assert!(catalog.0.iter().any(|i| i.spell != 0), "some items cast spells");
        let stacks: Vec<_> =
            catalog.0.iter().filter(|i| i.quantity == QuantityKind::Stack).collect();
        assert!(stacks.len() > 10 && stacks.iter().all(|i| i.maximum_quantity > 0));
        for item in &catalog.0 {
            assert!(files.contains(&item.image), "{}: {}", item.name, item.image);
        }
    }

    #[test]
    #[ignore = "requires a retail installation"]
    fn retail_loot_tables_decode() {
        let files = files();
        let tables =
            read_loot_tables(&files.read("data/databases/itemtables.dbs").unwrap().unwrap())
                .unwrap();
        let catalog =
            read_item_catalog(&files.read("data/databases/items.dbs").unwrap().unwrap()).unwrap();
        assert!(tables.0.len() > 50);
        assert!(
            tables.0.iter().flat_map(|t| &t.entries).all(|&(item, _)| catalog.get(item).is_some())
        );
    }
}
