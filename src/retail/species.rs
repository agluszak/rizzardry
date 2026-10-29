//! `data/databases/monsters.dbs`: a count followed by 0x297-byte species records.

use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::reflect::TypePath;

use super::record::{dice_at, reach};
use crate::bytes::{
    Bytes, DecodeError, f32_at, i16_at, i32_at, lenient_string, u16_at, u32_at, utf16_string,
};
use enum_map::EnumMap;
use enumset::EnumSet;

use crate::content::items::{AttackEffects, ItemId};
use crate::content::species::{
    AttackWord, Disposition, Drop, MonsterAttack, MonsterAttributes, MonsterCatalog, MonsterMagic,
    SpecialAttack, SpecialAttackKind, Species, Temper, Treasure,
};
use crate::content::spells::SpellId;

const RECORD: usize = 0x297;
const ATTACKS_AT: usize = 0xe7;
const ATTACK: usize = 0x22;

/// A name of UTF-16 text up to a NUL; retail drops anything after `#`.
fn name(raw: &[u8]) -> String {
    let name = utf16_string(raw);
    name.split('#').next().unwrap_or_default().trim().to_owned()
}

fn species(record: &[u8]) -> Result<Species, DecodeError> {
    let kind = crate::content::species::MonsterKind::try_from(record[0xcb])
        .map_err(|_| DecodeError::Invalid("unknown monster category", 0xcb))?;
    let bytes = |at: usize, n: usize| &record[at..at + n];
    let signed = |at: usize| record[at] as i8;
    let attacks = (0..3)
        .map(|i| &record[ATTACKS_AT + ATTACK * i..ATTACKS_AT + ATTACK * (i + 1)])
        .filter(|attack| attack[0] != 0)
        .map(|attack| {
            Ok(MonsterAttack {
                weapon: crate::content::species::MonsterWeapon::try_from(attack[1])
                    .map_err(|_| DecodeError::Invalid("unknown monster weapon", 1))?,
                word: if attack[2] == 13 {
                    AttackWord::Silent
                } else {
                    AttackWord::try_from(attack[2])
                        .map_err(|_| DecodeError::Invalid("unknown monster attack word", 2))?
                },
                reach: reach(attack[3]),
                score: attack[4],
                modes: EnumSet::from_repr_truncated(u16_at(attack, 0x15)?),
                damage: dice_at(attack, 0x17)?,
                effects: AttackEffects {
                    chances: EnumMap::from_array(attack[5..0x15].try_into().unwrap()),
                    strength: attack[0x1b],
                },
            })
        })
        .collect::<Result<_, DecodeError>>()?;
    let experience = match u32_at(record, 0x26b)? {
        0 => u32_at(record, 0x181)?,
        over => over,
    };
    Ok(Species {
        name: name(bytes(0, 48)),
        name_plural: name(bytes(0x30, 48)),
        generic_name: name(bytes(0x60, 48)),
        generic_plural: name(bytes(0x90, 48)),
        model: lenient_string(bytes(0x189, 48)).to_lowercase(),
        level_sight: kind == crate::content::species::MonsterKind::Ranger,
        npc: (record[0xd0] & 1 != 0)
            .then_some(crate::content::npcs::NpcId(u16::from(record[0xcd]))),
        kind,
        damage_reduction: record[0xcf],
        vulnerable_from_behind: record[0xd0] & 0x02 != 0,
        death_immune: record[0x1be] != 0,
        untargetable: record[0x24a] != 0,
        camouflage: record[0x248],
        attributes: MonsterAttributes {
            strength: record[0xd1],
            intelligence: record[0xd2],
            dexterity: record[0xd3],
            speed: record[0xd4],
            senses: record[0xd5],
        },
        hit_points: dice_at(record, 0xd6)?,
        stamina: dice_at(record, 0xda)?,
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
        gold: dice_at(record, 0x243)?,
        // Eight 10-byte slots; a zero count disables one.
        treasure: (0..8)
            .map(|slot| &record[0x1f3 + slot * 10..0x1f3 + slot * 10 + 10])
            .filter(|slot| i16_at(slot, 1).is_ok_and(|count| count != 0))
            .map(|slot| {
                let id = u16_at(slot, 3)?;
                Ok(Treasure {
                    drop: match slot[0] {
                        0 => Drop::Item(ItemId(id)),
                        1 => Drop::Table(id),
                        _ => Drop::Nothing,
                    },
                    chance: slot[5],
                    dice: dice_at(slot, 6)?,
                })
            })
            .collect::<Result<_, DecodeError>>()?,
        resistances: EnumMap::from_array(bytes(0x176, 6).try_into().unwrap()),
        // Metres (retail scales it by its world scale). Unused records hold
        // 0xcdcdcdcd filler, read as hostile.
        disposition: match i32_at(record, 0x25b)? {
            -1 => Disposition::Neutral,
            radius if radius > 0 => Disposition::HostileWithin(radius as f32),
            _ => Disposition::Hostile,
        },
        faction: i32_at(record, 0x25f)?,
        combat_move_range: f32_at(record, 0x1ba)?.max(0.0),
        pronouns: record[0xcc].min(3),
        group_size: dice_at(record, 0xc1)?,
        unborn: record[0x26a] != 0,
        companions: std::array::from_fn(|i| {
            let at = 0xc5 + 3 * i;
            let species = i16::from_le_bytes([record[at], record[at + 1]]);
            (species > 0).then(|| (species as u32, record[at + 2]))
        }),
        temper: Temper {
            flee_chance: record[0xe2],
            craven: record[0x1c0] == 0,
            camouflaged: record[0x249] != 0,
            prefers_range: record[0x1b9] != 0,
            wary: record[0x1c0] == 2 && record[0x1bf] != 3,
            bodyguard: record[0xd0] & 0x04 != 0,
        },
        magic: MonsterMagic {
            chance: record[0xe0],
            spells: std::array::from_fn(|slot| {
                Some(record[0x14d + slot]).filter(|&id| id != 0).map(SpellId)
            }),
            budget: i32_at(record, 0x26f)?,
        },
        special: SpecialAttack {
            kind: SpecialAttackKind::from_record(record[0xe3]),
            chance: record[0xe1],
            cooldown: record[0x15c],
        },
    })
}

/// Every species record.
pub fn read_species(data: &[u8]) -> Result<Vec<Species>, DecodeError> {
    let mut bytes = Bytes::new(data);
    let count = bytes.u32()? as usize;
    (0..count).map(|_| species(bytes.take(RECORD)?)).collect()
}

#[derive(Default, TypePath)]
pub struct MonsterCatalogLoader;

impl AssetLoader for MonsterCatalogLoader {
    type Asset = MonsterCatalog;
    type Settings = ();
    type Error = super::LoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<MonsterCatalog, super::LoadError> {
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
    use crate::content::items::Dice;

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
        assert_eq!(crocodile.disposition, Disposition::HostileWithin(15.0));
        assert_eq!(species[253].name, "Crab");
        assert_eq!(species[253].disposition, Disposition::HostileWithin(15.0));
        assert_eq!(slime.disposition, Disposition::Hostile);
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
