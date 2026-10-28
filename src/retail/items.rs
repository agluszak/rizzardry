//! The retail item databases: definitions (`data/databases/items.dbs`, a
//! count, then 269-byte records after Whizzardry `ItemDatabaseRecord`) and
//! loot tables (`itemtables.dbs`), decoded straight into the content types.

use super::LoadError;
use super::record::{dice, reach, use_type as use_type_of};
use crate::bytes::{Bytes, DecodeError, lenient_string, utf16_string};
use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::reflect::TypePath;
use enumset::EnumSet;

use crate::content::characters::Skill;
use crate::content::items::{
    EquipClass, ItemCatalog, ItemDefinition, ItemId, LootTable, LootTables, QuantityKind, Recipe,
};

const RECORD: usize = 0x10d;
const TABLE_RECORD: usize = 256 + 4 + 40 * 5 + 1 + 4 + 4 + 4 + 0x18;

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

/// World models for items without their own, by kind, as retail names
/// them (`g_item_model_fallback_names`); file names are not case-sensitive.
const KIND_MODELS: [&str; 145] = [
    "dagger",
    "long sword",
    "bipennis",
    "battle axe",
    "flail",
    "mace",
    "hammer",
    "staff",
    "halberd",
    "spear",
    "bo stick",
    "bow",
    "crossbow",
    "sling",
    "great sword",
    "rapier",
    "katana",
    "long staff",
    "wand",
    "magic stave",
    "spellbook",
    "shuriken",
    "arrows",
    "bullets",
    "kite shield",
    "basnet",
    "plate torso",
    "plate leggings",
    "gauntlets",
    "sollerets",
    "amulet",
    "ring",
    "blue potion",
    "scroll",
    "powder",
    "key",
    "locket",
    "great bow",
    "round shield",
    "wizard cone",
    "leather helm",
    "cuirass",
    "leather top",
    "greaves",
    "leather leggings",
    "gloves",
    "sandals",
    "buskins",
    "boots",
    "book",
    "ankh",
    "ninjato",
    "war hammer",
    "flamberge",
    "bullwhip",
    "sai",
    "nunchuka",
    "glaive",
    "black sword",
    "fire sword",
    "upper robes",
    "lower robes",
    "halter",
    "skirt",
    "skullcap",
    "feathered cap",
    "mitre",
    "breast plate",
    "kabuto",
    "upper toseido",
    "lower toseido",
    "burgonet",
    "fur leggings",
    "cloak",
    "mantis gloves",
    "mantis boots",
    "garland",
    "cat 'o nine tails",
    "silver cross",
    "ruby talisman",
    "purple amulet",
    "bracelet",
    "necklace",
    "quarrels",
    "jade figure",
    "chain coif",
    "upper chain",
    "lower chain",
    "mail mittens",
    "chain hosen",
    "lance",
    "armet",
    "ninja cowl",
    "upper ninja garb",
    "lower ninja garb",
    "tabi boots",
    "heaume",
    "box helm",
    "diamond ring",
    "bag",
    "red potion",
    "green potion",
    "purple potion",
    "yellow potion",
    "power glove",
    "t'rang staff",
    "bronze gauntlets",
    "comm link",
    "light sword",
    "magic bag",
    "phaser",
    "id card",
    "darts",
    "powder shot",
    "musket",
    "coin",
    "stone",
    "stick",
    "short sword",
    "undefined",
    "bagpipes",
    "lyre",
    "lute",
    "horn",
    "map",
    "bracelet",
    "shuriken",
    "undefined",
    "wand",
    "yellow potion",
    "medium shield",
    "undefined",
    "undefined",
    "undefined",
    "undefined",
    "undefined",
    "saxophone",
    "violin",
    "drum",
    "bullroarer",
    "bomb",
    "emptybottle",
    "stix",
    "rocket",
    "rocketlauncher",
];

/// What an unidentified item is called, by kind: string 0x1e7 ("%s
/// (Unidentified)") around these strings (retail `g_generic_item_name_notice`,
/// `FormatItemDisplayName`).
const KIND_NAMES: [usize; 147] = [
    0x45a, 0x45b, 0x45c, 0x45d, 0x45e, 0x45f, 0x460, 0x461, 0x462, 0x463, 0x464, 0x465, 0x466,
    0x467, 0x468, 0x469, 0x46a, 0x46b, 0x46c, 0x46d, 0x46e, 0x46f, 0x470, 0x471, 0x472, 0x473,
    0x474, 0x475, 0x476, 0x477, 0x478, 0x479, 0x47a, 0x47b, 0x47c, 0x47d, 0x47e, 0x47f, 0x480,
    0x481, 0x482, 0x483, 0x484, 0x485, 0x486, 0x487, 0x488, 0x489, 0x48a, 0x48b, 0x48c, 0x48d,
    0x48e, 0x48f, 0x490, 0x491, 0x492, 0x493, 0x494, 0x495, 0x496, 0x497, 0x498, 0x499, 0x49a,
    0x49b, 0x49c, 0x49d, 0x49e, 0x49f, 0x4a0, 0x4a1, 0x4a2, 0x4a3, 0x4a4, 0x4a5, 0x4a6, 0x4a7,
    0x4a8, 0x4a9, 0x4aa, 0x4ab, 0x4ac, 0x4ad, 0x4ae, 0x4af, 0x4b0, 0x4b1, 0x4b2, 0x4b3, 0x4b4,
    0x4b5, 0x4b6, 0x4b7, 0x4b8, 0x4b9, 0x4ba, 0x4bb, 0x4bc, 0x4bd, 0x4be, 0x4bf, 0x4c0, 0x4c1,
    0x4c2, 0x4c3, 0x4c4, 0x4c5, 0x4c6, 0x4c7, 0x4c8, 0x4c9, 0x4ca, 0x4cb, 0x4cc, 0x4cd, 0x4ce,
    0x4cf, 0x4d0, 0x459, 0x4d1, 0x4d2, 0x4d3, 0x4d4, 0x4d5, 0x4ab, 0x46f, 0x459, 0x46c, 0x4c1,
    0x4d6, 0x4d7, 0x4d8, 0x4d9, 0x4da, 0x4db, 0x4dc, 0x4dd, 0x4de, 0x4df, 0x4e0, 0x4e1, 0x4e2,
    0x4e3, 0x4e4, 0x4e5, 0x4e6,
];
const UNIDENTIFIED: usize = 0x1e7;

/// Item flag bits (Whizzardry `W8ItemFlag`).
mod flag {
    pub const AUTO_IDENTIFY: u8 = 0x01;
    pub const NO_DISCARD: u8 = 0x02;
    pub const TWO_HANDED: u8 = 0x04;
    pub const OFF_HAND_ALLOWED: u8 = 0x08;
    pub const MERCHANT_CANNOT_SELL: u8 = 0x10;
    pub const CONTAINER: u8 = 0x20;
    pub const MUST_EQUIP_TO_USE: u8 = 0x40;
}

/// Two (id, minimum) pairs in the typed attribute or skill domain. Only
/// 0xff marks none; invalid constraints must not become unrestricted items.
fn minimums<K: enum_map::Enum + TryFrom<u8>>(
    bytes: &mut Bytes,
) -> Result<enum_map::EnumMap<K, u8>, DecodeError> {
    let mut minimums = enum_map::EnumMap::default();
    for _ in 0..2 {
        let (id, minimum) = (bytes.u8()?, bytes.u8()?);
        if id == u8::MAX {
            continue;
        }
        let of = K::try_from(id).map_err(|_| bytes.invalid("unknown item requirement"))?;
        minimums[of] = minimum;
    }
    Ok(minimums)
}

/// What an item is made from (retail `MergeItems`): the two ingredient
/// item numbers at 0xb9 and 0xbd, and the skill (0xc9, -1 for none) and
/// level (0xca) making it takes. A negative ingredient never matches an
/// item, so its record is made from nothing.
fn recipe(index: usize, record: &[u8]) -> Result<Option<Recipe>, LoadError> {
    let mut fields = Bytes::new(&record[0xb9..]);
    let (Ok(first), Ok(second)) = (u16::try_from(fields.i32()?), u16::try_from(fields.i32()?))
    else {
        return Ok(None);
    };
    let mut fields = Bytes::new(&record[0xc9..]);
    let (skill, level) = (fields.i8()?, fields.u8()?);
    let skill = match skill {
        -1 => None,
        skill => Some((
            u8::try_from(skill).ok().and_then(|s| Skill::try_from(s).ok()).ok_or_else(|| {
                LoadError::Invalid(format!("item {index}: unknown recipe skill {skill}"))
            })?,
            level,
        )),
    };
    Ok(Some(Recipe { ingredients: [ItemId(first), ItemId(second)], skill }))
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

/// The item's model lying in the world: its own (`internal_name`), else its
/// kind's (retail `ActivateItem`). Retail shows `questionmark` for a model
/// that is missing (four items, and kinds named "UNDEFINED").
fn model_path(own: &str, kind: u16) -> String {
    let name = match own {
        "" => KIND_MODELS.get(usize::from(kind)).copied().unwrap_or("questionmark"),
        own => own,
    };
    let name = name.to_lowercase();
    let name = name.strip_suffix(".itm").unwrap_or(&name);
    format!("data/items3d/{name}.itm")
}

fn read_item(index: usize, record: &[u8], strings: &[String]) -> Result<ItemDefinition, LoadError> {
    let mut bytes = Bytes::new(&record[0x3c..]);
    bytes.u16()?; // Wizardry 7 item number
    let equip_class = bytes.u8()?;
    let kind = bytes.u16()?;
    let flags = bytes.u8()?;
    let has = |bit| flags & bit != 0;
    let use_type = bytes.u8()?;
    let use_type = use_type_of(use_type)
        .ok_or_else(|| LoadError::Invalid(format!("item {index}: unknown use type {use_type}")))?;
    bytes.take(3)?; // not established
    let weapon_skill = u8::try_from(bytes.i8()?).ok().and_then(|s| Skill::try_from(s).ok());
    let reach = reach(bytes.u8()?);
    let damage_bonus = bytes.i8()?;
    let hit_bonus = bytes.i8()?;
    let damage = dice(&mut bytes)?;
    let attack_modes = EnumSet::from_repr_truncated(bytes.u16()?);
    bytes.take(16 + 1 + 1)?; // effect chances, magnitude, slays
    let armor_class_bonus = bytes.i8()?;
    let (spell, power) = (bytes.u8()?, bytes.u8()?);
    bytes.u8()?; // not established
    let spell = (spell != 0
        && (power != 0
            || matches!(
                use_type,
                crate::content::items::UseType::Instrument | crate::content::items::UseType::Gadget
            )))
    .then_some(crate::content::spells::ItemSpell {
        spell: crate::content::spells::SpellId(spell),
        power: power.min(crate::content::spells::MAX_POWER),
    });
    let quantity_kind = bytes.u8()?;
    let initial_quantity = dice(&mut bytes)?;
    let stack_limit = bytes.u8()?;
    let regeneration = bytes.array::<3>()?.map(|value| value as i8);
    let resistance_bonus =
        enum_map::EnumMap::from_array(bytes.array::<6>()?.map(|value| value as i8));
    bytes.take(1)?; // property
    let professions = EnumSet::from_repr_truncated(bytes.u16()?);
    let races = EnumSet::from_repr_truncated(bytes.u32()?);
    let genders = EnumSet::from_repr_truncated(bytes.u8()?);
    let attribute_minimums = minimums(&mut bytes)?;
    let skill_minimums = minimums(&mut bytes)?;
    let identify_difficulty = bytes.u8()?;
    let value = bytes.u32()?;
    let weight = bytes.u16()?;
    let cursed = bytes.u8()? != 0;
    let own_image = lenient_string(&record[0xcd..0xcd + 0x18]);
    let string = |index: usize| strings.get(index).map_or("", |s| s.as_str());
    let kind_name = KIND_NAMES.get(usize::from(kind)).map_or("", |&k| string(k));
    Ok(ItemDefinition {
        unidentified_name: string(UNIDENTIFIED).replace("%s", kind_name),
        kind,
        kind_name: kind_name.to_owned(),
        id: ItemId(index as u16),
        name: utf16_string(&record[..60]),
        image: image_path(&own_image, kind)
            .ok_or_else(|| LoadError::Invalid(format!("item {index}: kind {kind} has no image")))?,
        model: model_path(&lenient_string(&record[0x8d..0x8d + 0x20]), kind),
        class: EquipClass::try_from(equip_class).map_err(|_| {
            LoadError::Invalid(format!("item {index}: unknown equipment class {equip_class}"))
        })?,
        auto_identify: has(flag::AUTO_IDENTIFY),
        crucial: has(flag::NO_DISCARD),
        merchant_cannot_sell: has(flag::MERCHANT_CANNOT_SELL),
        identify_difficulty,
        two_handed: has(flag::TWO_HANDED),
        off_hand: has(flag::OFF_HAND_ALLOWED),
        container: has(flag::CONTAINER),
        weapon_skill,
        reach,
        damage,
        attack_modes,
        damage_bonus,
        hit_bonus,
        armor_class_bonus,
        spell,
        use_type,
        must_equip_to_use: has(flag::MUST_EQUIP_TO_USE),
        regeneration,
        resistance_bonus,
        quantity: match quantity_kind {
            0 => QuantityKind::None,
            1 => QuantityKind::Stack,
            2 => QuantityKind::Charges,
            3 => QuantityKind::Uses,
            4 => QuantityKind::Shots,
            5 => QuantityKind::Unlabelled,
            other => {
                return Err(LoadError::Invalid(format!(
                    "item {index}: unknown quantity kind {other}"
                )));
            }
        },
        initial_quantity,
        stack_limit,
        professions,
        races,
        genders,
        attribute_minimums,
        skill_minimums,
        value,
        weight,
        cursed,
        recipe: recipe(index, record)?,
    })
}

/// Every item definition, indexed by item number.
/// The item definitions, with their unidentified names from the string
/// table (`data/strings/stringdata.dat`).
pub fn read_item_catalog(data: &[u8], strings: &[String]) -> Result<ItemCatalog, LoadError> {
    let mut bytes = Bytes::new(data);
    let count = bytes.u32()? as usize;
    if data.len() - 4 < count * RECORD {
        return Err(bytes.invalid("item database shorter than its count").into());
    }
    let records = data[4..4 + count * RECORD].as_chunks::<RECORD>().0;
    Ok(ItemCatalog(
        records
            .iter()
            .enumerate()
            .map(|(i, r)| read_item(i, r, strings))
            .collect::<Result<_, _>>()?,
    ))
}

/// The loot tables (Whizzardry `W8ItemTableRecord`), after their category
/// names.
pub fn read_loot_tables(data: &[u8]) -> Result<LootTables, DecodeError> {
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
            Ok(LootTable {
                name: lenient_string(&record[..256]),
                entries,
                level_scaled,
                item_count,
                gold,
            })
        })
        .collect::<Result<_, DecodeError>>()?;
    Ok(LootTables(tables))
}

#[derive(Default, TypePath)]
pub struct ItemCatalogLoader;

impl AssetLoader for ItemCatalogLoader {
    type Asset = ItemCatalog;
    type Settings = ();
    type Error = LoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        context: &mut LoadContext<'_>,
    ) -> Result<ItemCatalog, LoadError> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).await?;
        let strings = context
            .read_asset_bytes("retail://data/strings/stringdata.dat")
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        read_item_catalog(&data, &super::strings::read_string_table(&strings)?)
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
    type Error = LoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<LootTables, LoadError> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).await?;
        Ok(read_loot_tables(&data)?)
    }

    fn extensions(&self) -> &[&str] {
        &["dbs"]
    }
}

/// The retail item catalog, for tests.
#[cfg(test)]
pub fn retail_catalog(files: &crate::retail::files::RetailFiles) -> ItemCatalog {
    let strings = files.read("data/strings/stringdata.dat").unwrap().unwrap();
    let strings = super::strings::read_string_table(&strings).unwrap();
    read_item_catalog(&files.read("data/databases/items.dbs").unwrap().unwrap(), &strings).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_flags_and_residual_regeneration_are_decoded() {
        let mut data = vec![0; 4 + RECORD];
        data[..4].copy_from_slice(&1_u32.to_le_bytes());
        data[4 + 0x41] = 0x40;
        data[4 + 0x6c..4 + 0x6f].copy_from_slice(&[1, 0xff, 2]);
        let catalog = read_item_catalog(&data, &[]).unwrap();
        assert!(catalog.0[0].must_equip_to_use);
        assert_eq!(catalog.0[0].regeneration, [1, -1, 2]);
        assert!(catalog.0[0].keeps_empty());
        data[4 + 0x42] = 6; // Instrument power comes from the player, not this byte.
        data[4 + 0x63] = 39;
        data[4 + 0x64] = 0;
        let catalog = read_item_catalog(&data, &[]).unwrap();
        assert_eq!(
            catalog.0[0].spell,
            Some(crate::content::spells::ItemSpell {
                spell: crate::content::spells::SpellId(39),
                power: 0
            })
        );
    }

    #[test]
    fn invalid_requirement_ids_cannot_import_as_unrestricted_items() {
        let mut data = vec![0; 4 + RECORD];
        data[..4].copy_from_slice(&1_u32.to_le_bytes());
        for (offset, invalid_id) in [(0x7d, 7), (0x81, 41)] {
            data[4 + offset] = invalid_id;
            data[4 + offset + 1] = 85;
            assert!(read_item_catalog(&data, &[]).is_err());
            data[4 + offset] = 0xff;
            assert!(
                read_item_catalog(&data, &[]).is_ok(),
                "the documented absent-requirement sentinel is valid"
            );
        }
    }

    #[test]
    #[ignore = "requires a retail installation"]
    fn retail_items_decode() {
        let files = crate::retail::files::retail_files();
        let catalog = retail_catalog(&files);
        assert_eq!(catalog.0.len(), 819);
        let dagger = catalog.get(ItemId(0)).unwrap();
        assert_eq!((dagger.name.as_str(), dagger.class), ("Dagger", EquipClass::ShortWeapon));
        assert_eq!(dagger.unidentified_name, "Dagger (Unidentified)");
        assert_eq!(dagger.kind_name, "Dagger");
        assert!(dagger.off_hand && !dagger.two_handed);
        assert!(catalog.0.iter().filter(|i| !i.name.is_empty()).count() > 500);
        assert!(catalog.0.iter().any(|i| i.recipe.is_some()), "items are made from others");
        assert!(
            (599..=610).all(|id| catalog.get(ItemId(id)).is_some_and(|i| i.kind == 0x83)),
            "the omnigun's twelve tiers"
        );
        let stacks: Vec<_> =
            catalog.0.iter().filter(|i| i.quantity == QuantityKind::Stack).collect();
        assert!(stacks.len() > 10 && stacks.iter().all(|i| i.stack_limit > 0));
        for item in &catalog.0 {
            assert!(files.contains(&item.image), "{}: {}", item.name, item.image);
        }
        // These show retail's question mark.
        let without_model: Vec<&str> = catalog
            .0
            .iter()
            .filter(|i| !i.name.is_empty() && !files.contains(&i.model))
            .map(|i| i.name.as_str())
            .collect();
        assert_eq!(
            without_model,
            ["Thermal Pineapple", "Fuel Tank", "Ivory Blade", "Beastslayer", "Jack in the Box"]
        );
    }

    #[test]
    #[ignore = "requires a retail installation"]
    fn retail_loot_tables_decode() {
        let files = crate::retail::files::retail_files();
        let tables =
            read_loot_tables(&files.read("data/databases/itemtables.dbs").unwrap().unwrap())
                .unwrap();
        let catalog = retail_catalog(&files);
        assert!(tables.0.len() > 50);
        assert!(
            tables.0.iter().flat_map(|t| &t.entries).all(|&(item, _)| catalog.get(item).is_some())
        );
    }
}
