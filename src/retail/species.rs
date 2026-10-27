//! `data/databases/monsters.dbs`: a count followed by 0x297-byte species records.

use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::reflect::TypePath;
use thiserror::Error;

use super::pvl::PvlError;
use crate::bytes::Bytes;
use enum_map::EnumMap;
use enumset::EnumSet;

use crate::content::items::{Dice, ItemId};
use crate::content::species::{
    AttackWord, Drop, MonsterAttack, MonsterAttributes, MonsterCatalog, Reach, Species, Treasure,
};

const RECORD: usize = 0x297;
const ATTACKS_AT: usize = 0xe7;
const ATTACK: usize = 0x22;

fn dice(record: &[u8], at: usize) -> Dice {
    Dice {
        base: i16::from_le_bytes([record[at], record[at + 1]]),
        count: record[at + 2],
        sides: record[at + 3],
    }
}

fn u32_at(record: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(record[at..at + 4].try_into().unwrap())
}

/// A name of UTF-16 text up to a NUL; retail drops anything after `#`.
fn name(raw: &[u8]) -> String {
    let units: Vec<u16> = raw
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&c| u16::from_le_bytes(c))
        .take_while(|&u| u != 0)
        .collect();
    let name = String::from_utf16_lossy(&units);
    name.split('#').next().unwrap_or_default().trim().to_owned()
}

fn species(record: &[u8]) -> Species {
    let bytes = |at: usize, n: usize| &record[at..at + n];
    let signed = |at: usize| record[at] as i8;
    let model = bytes(0x189, 48);
    let model_end = model.iter().position(|&b| b == 0).unwrap_or(model.len());
    let attacks = (0..3)
        .map(|i| &record[ATTACKS_AT + ATTACK * i..ATTACKS_AT + ATTACK * (i + 1)])
        .filter(|attack| attack[0] != 0)
        .map(|attack| MonsterAttack {
            named: attack[1] != 0,
            word: match attack[2] {
                1 => AttackWord::Verb,
                word => AttackWord::Word(word),
            },
            reach: Reach::from_retail(attack[3]),
            score: attack[4],
            modes: EnumSet::from_repr_truncated(u16::from_le_bytes([attack[0x15], attack[0x16]])),
            damage: dice(attack, 0x17),
        })
        .collect();
    let experience = match u32_at(record, 0x26b) {
        0 => u32_at(record, 0x181),
        over => over,
    };
    Species {
        name: name(bytes(0, 48)),
        name_plural: name(bytes(0x30, 48)),
        generic_name: name(bytes(0x60, 48)),
        generic_plural: name(bytes(0x90, 48)),
        model: String::from_utf8_lossy(&model[..model_end]).to_lowercase(),
        level_sight: record[0xcb] == 4,
        npc: (record[0xd0] & 1 != 0)
            .then_some(crate::content::npcs::NpcId(u16::from(record[0xcd]))),
        damage_reduction: record[0xcf],
        attributes: MonsterAttributes {
            strength: record[0xd1],
            intelligence: record[0xd2],
            dexterity: record[0xd3],
            speed: record[0xd4],
            senses: record[0xd5],
        },
        hit_points: dice(record, 0xd6),
        stamina: dice(record, 0xda),
        initiative: record[0xe4],
        attacks_per_round: record[0xe5],
        swings_per_round: record[0xe6],
        attacks,
        hits_on_party: EnumMap::from_array(bytes(0x157, 5).try_into().unwrap()),
        evasion: signed(0x15d),
        hit_locations: bytes(0x15f, 7).try_into().unwrap(),
        armour_by_location: std::array::from_fn(|i| signed(0x166 + i)),
        armour_by_mode: EnumMap::from_array(std::array::from_fn(|i| signed(0x16d + i))),
        experience,
        level: record[0x24f],
        display_level: record[0x251],
        gold: dice(record, 0x243),
        // Eight 10-byte slots; a zero count disables one.
        treasure: (0..8)
            .map(|slot| &record[0x1f3 + slot * 10..0x1f3 + slot * 10 + 10])
            .filter(|slot| i16::from_le_bytes([slot[1], slot[2]]) != 0)
            .map(|slot| {
                let id = u16::from_le_bytes([slot[3], slot[4]]);
                Treasure {
                    drop: match slot[0] {
                        0 => Drop::Item(ItemId(id)),
                        1 => Drop::Table(id),
                        _ => Drop::Nothing,
                    },
                    chance: slot[5],
                    dice: dice(slot, 6),
                }
            })
            .collect(),
    }
}

/// Every species record.
pub fn read_species(data: &[u8]) -> Result<Vec<Species>, PvlError> {
    let mut bytes = Bytes::new(data);
    let count = bytes.u32()? as usize;
    (0..count).map(|_| Ok(species(bytes.take(RECORD)?))).collect()
}

#[derive(Default, TypePath)]
pub struct MonsterCatalogLoader;

#[derive(Debug, Error)]
pub enum SpeciesLoadError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Data(#[from] PvlError),
}

impl AssetLoader for MonsterCatalogLoader {
    type Asset = MonsterCatalog;
    type Settings = ();
    type Error = SpeciesLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<MonsterCatalog, SpeciesLoadError> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).await?;
        Ok(MonsterCatalog(read_species(&data)?))
    }

    fn extensions(&self) -> &[&str] {
        &["dbs"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a retail installation"]
    fn retail_species_decode() {
        let files = crate::retail::files::retail_files();
        let species =
            read_species(&files.read("data/databases/monsters.dbs").unwrap().unwrap()).unwrap();
        assert_eq!(species.len(), 595);
        // The Slime lashes for 2+1d6 at score 130.
        let slime = &species[1];
        assert_eq!(slime.attacks[0].score, 130);
        assert_eq!(slime.attacks[0].damage, Dice { base: 2, count: 1, sides: 6 });
        assert_eq!(slime.evasion, 7);
        let crocodile = &species[62];
        assert_eq!(crocodile.name, "Ironskin Crocodile");
        // A Venom Crab may leave item 0x27e, one in four.
        let venom = &species[310];
        assert_eq!(venom.treasure[0].drop, Drop::Item(ItemId(0x27e)));
        assert_eq!(venom.treasure[0].chance, 25);
        assert_eq!(crocodile.shown_name(true, 2), "Ironskin Crocodiles");
        assert_eq!(crocodile.shown_name(false, 1), "Crocodile");
        assert_eq!(crocodile.attacks[0].damage, Dice { base: 2, count: 3, sides: 9 });
        assert!(species.iter().all(|s| s.attacks.len() <= 3));
    }
}
