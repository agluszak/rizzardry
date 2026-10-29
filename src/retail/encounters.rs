//! A level's random encounters: its generators (the `.sts` `MONG` chunk,
//! retail `SaveEncounterState`, `MonGen::Save`), its row of `LEVELS.DBS`
//! and `EncounterTables.dbs` (retail `InitializeEncounterTables`).

use crate::bytes::{Bytes, DecodeError, lenient_string};
use crate::content::level::{
    EncounterEntry, EncounterLimits, EncounterTable, EncounterTime, Encounters, Generator,
};
#[cfg(test)]
use bevy::math::Vec3;

use super::space::retail_position;

/// The bytes of one `LEVELS.DBS` row.
const LEVEL_RECORD: usize = 0xd8;
/// Where a row's encounter bounds start.
const LEVEL_LIMITS_AT: usize = 0x3c;

/// A level's generators and running state as its `.sts` file holds them,
/// with their tables resolved from `tables`; `None` when it has no `MONG`
/// chunk.
pub fn read_generators(
    state: &[u8],
    tables: &[EncounterTable],
) -> Result<Option<Encounters>, DecodeError> {
    let Some(data) = super::population::level_chunk(state, b"MONG")? else { return Ok(None) };
    let mut bytes = Bytes::new(data);
    let version = bytes.i32()?;
    if version > 4 {
        bytes.i32()?; // the editor's generator-naming counter
    }
    let budget = bytes.i32()?;
    if version > 1 {
        bytes.i32()?; // lingering, which the level's row replaces
    }
    let (held, default_chance, default_interval) = if version > 2 {
        let held = bytes.u8()? != 0;
        let chance = bytes.i16()?;
        let interval = bytes.i16()?;
        bytes.i16()?; // unused
        (held, chance, interval)
    } else {
        (false, 100, 10)
    };
    let count = bytes.i32()?.max(0);
    let generators = (0..count)
        .map(|_| {
            let version = bytes.i8()?;
            let (name, enabled) = if version >= 3 {
                (lenient_string(bytes.take(32)?), bytes.u8()? != 0)
            } else {
                (String::new(), true)
            };
            let flags = bytes.u32()?;
            let chance = bytes.u8()?;
            let interval = bytes.u16()?;
            bytes.u16()?; // unknown
            let position = retail_position(bytes.f32s::<3>()?);
            let table = usize::try_from(bytes.i32()?).ok().and_then(|i| tables.get(i)).cloned();
            let progress = if version > 1 {
                let progress = bytes.f32()?;
                bytes.f32()?; // duration scale, reset on arrival
                progress
            } else {
                0.0
            };
            Ok(Generator {
                name,
                enabled,
                disabled: flags & 1 != 0,
                uses_defaults: flags & 8 != 0,
                // `SetEncounterTable` marks harassment tables so.
                harasses: flags & 0x20 != 0
                    || table.as_ref().is_some_and(|t: &EncounterTable| {
                        t.name.to_ascii_uppercase().starts_with("HARASSMENT")
                    }),
                chance,
                interval,
                position,
                progress: if progress.is_finite() { progress.clamp(0.0, 1.0) } else { 0.0 },
                table,
            })
        })
        .collect::<Result<_, DecodeError>>()?;
    Ok(Some(Encounters {
        limits: EncounterLimits::default(),
        budget,
        held,
        default_chance,
        default_interval,
        generators,
    }))
}

/// A level's bounds on random encounters, by its index in `LEVELS.DBS`.
pub fn read_limits(levels: &[u8], level: usize) -> Result<Option<EncounterLimits>, DecodeError> {
    let mut bytes = Bytes::new(levels);
    let count = bytes.u32()? as usize;
    if level >= count {
        return Ok(None);
    }
    let mut row = Bytes::new(bytes.take(LEVEL_RECORD * (level + 1))?);
    row.take(LEVEL_RECORD * level + LEVEL_LIMITS_AT)?;
    let [most, fewest, largest_budget, smallest_budget, period, lingers] = row.i32s::<6>()?;
    Ok(Some(EncounterLimits { most, fewest, largest_budget, smallest_budget, period, lingers }))
}

/// Every encounter table, in file order (which generators index).
pub fn read_tables(data: &[u8]) -> Result<Vec<EncounterTable>, DecodeError> {
    let mut bytes = Bytes::new(data);
    let names = bytes.i32()?.max(0) as usize;
    bytes.take(names * 0x100)?;
    let count = bytes.i32()?.max(0);
    (0..count)
        .map(|_| {
            bytes.u8()?; // record kind
            let name = lenient_string(bytes.take(0x100)?);
            bytes.u32()?; // category, for the editor
            let version = bytes.u16()?;
            let entries = usize::from(bytes.u8()?);
            let species = bytes.take(entries * 2)?;
            let rarity = bytes.take(entries)?;
            let time = bytes.take(entries)?;
            let challenge = bytes.take(entries)?;
            bytes.take(entries * 64)?; // monster script names
            if version != 1 {
                bytes.u8()?; // flags
            }
            let entries = (0..entries)
                .map(|i| EncounterEntry {
                    species: u32::from(u16::from_le_bytes([species[2 * i], species[2 * i + 1]])),
                    rarity: rarity[i],
                    time: match time[i] {
                        0 => EncounterTime::Day,
                        1 => EncounterTime::Night,
                        _ => EncounterTime::Always,
                    },
                    challenge: challenge[i],
                })
                .collect();
            Ok(EncounterTable { name, entries })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str, width: usize) -> Vec<u8> {
        let mut bytes = text.as_bytes().to_vec();
        bytes.resize(width, 0);
        bytes
    }

    /// Two tables, the second a harassment one with two entries.
    fn tables() -> Vec<u8> {
        let mut bytes = 1i32.to_le_bytes().to_vec();
        bytes.extend(name("Ignored", 0x100));
        bytes.extend(2i32.to_le_bytes());
        for (table, entries) in [("Woods", 1u8), ("HARASSMENT Rattkin", 2)] {
            bytes.push(0);
            bytes.extend(name(table, 0x100));
            bytes.extend(4u32.to_le_bytes());
            bytes.extend(2u16.to_le_bytes());
            bytes.push(entries);
            for species in [12u16, 300].iter().take(usize::from(entries)) {
                bytes.extend(species.to_le_bytes());
            }
            bytes.extend([20, 70].iter().take(usize::from(entries)));
            bytes.extend([1, 2].iter().take(usize::from(entries)));
            bytes.extend([5, 9].iter().take(usize::from(entries)));
            bytes.extend(vec![0; 64 * usize::from(entries)]);
            bytes.push(0);
        }
        bytes
    }

    /// The encounter state of a level with one generator on the second
    /// table, using the defaults, 30% through its timer.
    fn mong() -> Vec<u8> {
        let mut bytes = Vec::new();
        for word in [5i32, 3, 14, 180] {
            bytes.extend(word.to_le_bytes());
        }
        bytes.push(0);
        for short in [60i16, 25, -1] {
            bytes.extend(short.to_le_bytes());
        }
        bytes.extend(1i32.to_le_bytes());
        bytes.push(3);
        bytes.extend(name("MonGen001", 32));
        bytes.push(1);
        bytes.extend(8u32.to_le_bytes());
        bytes.push(40);
        bytes.extend(90u16.to_le_bytes());
        bytes.extend(0xffffu16.to_le_bytes());
        for coordinate in [500.0f32, 0.0, 1000.0] {
            bytes.extend(coordinate.to_le_bytes());
        }
        bytes.extend(1i32.to_le_bytes());
        bytes.extend(0.3f32.to_le_bytes());
        bytes.extend(1.0f32.to_le_bytes());
        bytes
    }

    #[test]
    fn generators_resolve_their_tables_and_the_level_row_its_bounds() {
        let tables = read_tables(&tables()).unwrap();
        assert_eq!(tables.len(), 2);
        assert_eq!(
            tables[1].entries[1],
            EncounterEntry { species: 300, rarity: 70, time: EncounterTime::Always, challenge: 9 }
        );
        assert_eq!(tables[0].entries[0].time, EncounterTime::Night);
        let state = crate::retail::population::tests::level_state(b"MONG", &mong());
        let encounters = read_generators(&state, &tables).unwrap().unwrap();
        assert_eq!(encounters.budget, 14);
        assert_eq!((encounters.default_chance, encounters.default_interval), (60, 25));
        let generator = &encounters.generators[0];
        assert_eq!(generator.name, "MonGen001");
        assert!(generator.enabled && generator.uses_defaults && !generator.disabled);
        assert!(generator.harasses, "a harassment table marks its generator");
        assert_eq!((generator.chance, generator.interval), (40, 90));
        assert_eq!(generator.position, Vec3::new(1.0, 0.0, -2.0));
        assert!((generator.progress - 0.3).abs() < 1e-6);
        assert_eq!(generator.table.as_ref().unwrap().name, "HARASSMENT Rattkin");

        let mut levels = 3u32.to_le_bytes().to_vec();
        for level in 0..3i32 {
            let mut row = vec![0; LEVEL_RECORD];
            for (i, word) in [10i32, 2, 20, 5, 600, 180].iter().enumerate() {
                let at = LEVEL_LIMITS_AT + 4 * i;
                row[at..at + 4].copy_from_slice(&(word * (level + 1)).to_le_bytes());
            }
            levels.extend(row);
        }
        assert_eq!(
            read_limits(&levels, 1).unwrap(),
            Some(EncounterLimits {
                most: 20,
                fewest: 4,
                largest_budget: 40,
                smallest_budget: 10,
                period: 1200,
                lingers: 360,
            })
        );
        assert_eq!(read_limits(&levels, 3).unwrap(), None);
    }
}
