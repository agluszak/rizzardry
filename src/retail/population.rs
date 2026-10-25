//! Who stands where in a level: the monster placements of its `.sts` file,
//! and each species' model from `data/databases/monsters.dbs`. See
//! `docs/research/level-population.md`.

use super::bytes::Bytes;
use super::pvl::{PvlError, invalid};

/// One placed monster (a `MONS` monster record).
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// Unique within the level; what triggers and groups refer to.
    pub location: i32,
    pub group: i32,
    /// Index into `monsters.dbs`.
    pub species: u32,
    /// Retail units, at the feet.
    pub position: [f32; 3],
    /// Heading: the monster faces retail `(sin yaw, 0, cos yaw)`.
    pub yaw: f32,
    /// Frozen until a script births it.
    pub unborn: bool,
}

const GROUP_VERSION_AT: usize = 0xc4;

fn chunk<'a>(bytes: &mut Bytes<'a>) -> Result<([u8; 4], &'a [u8]), PvlError> {
    let id: [u8; 4] = bytes.take(4)?.try_into().unwrap();
    bytes.take(2)?; // grouped, at end
    let extent = bytes.u32()? as usize;
    Ok((id, bytes.take(extent)?))
}

/// The monsters placed in a level's `.sts` file.
pub fn read_placements(data: &[u8]) -> Result<Vec<Placement>, PvlError> {
    let mut bytes = Bytes::new(data);
    let (id, root) = chunk(&mut bytes)?;
    if &id != b"RIFF" {
        return Err(invalid("level state is not a RIFF chunk", &bytes));
    }
    let mut root = Bytes::new(root);
    root.u32()?; // children
    let (id, level) = chunk(&mut root)?;
    if &id != b"LVLS" {
        return Err(invalid("level state has no LVLS chunk", &root));
    }
    let mut level = Bytes::new(level);
    let children = level.u32()?;
    level.u32()?; // level id
    for _ in 0..children {
        let (id, payload) = chunk(&mut level)?;
        if &id == b"MONS" {
            return read_monsters(payload);
        }
    }
    Ok(Vec::new())
}

fn read_monsters(data: &[u8]) -> Result<Vec<Placement>, PvlError> {
    let mut bytes = Bytes::new(data);
    let groups = bytes.u32()?;
    let monsters = bytes.u32()?;
    for _ in 0..groups {
        let size = bytes.u32()? as usize;
        let image = bytes.take(size)?;
        let version = image
            .get(GROUP_VERSION_AT..GROUP_VERSION_AT + 4)
            .map_or(0, |v| u32::from_le_bytes(v.try_into().unwrap()));
        if version >= 2 {
            bytes.u8()?; // encounter
        }
    }
    (0..monsters)
        .map(|_| {
            let version = bytes.u32()?;
            let size = bytes.u32()? as usize;
            let image = bytes.take(size)?;
            let mut fields = Bytes::new(image);
            let location = fields.i32()?;
            let group = fields.i32()?;
            let species = fields.u32()?;
            let mut at = Bytes::new(&image[0x17..]);
            let position = at.f32s::<3>()?;
            let yaw = at.f32()?;
            if bytes.u8()? != 0 {
                bytes.take(64 + 4 + 4)?; // script name, wait, line
                let conditions = bytes.i32()?.max(0) as usize;
                bytes.take(conditions)?;
            }
            let unborn = version >= 5 && bytes.u8()? != 0;
            if version >= 2 && bytes.u8()? != 0 {
                bytes.take(4 + 4 + 12 + 12)?; // movement
            }
            if version >= 3 {
                bytes.take(4 + 4 + 4 + 1)?; // order flags and mode, patrol distance and variation, index
                let points = bytes.i32()?.max(0) as usize;
                bytes.take(points * 12)?;
                if version >= 4 {
                    bytes.take(12)?; // direction
                }
                if version >= 6 {
                    bytes.u8()?; // face the party
                }
                if version >= 7 {
                    bytes.u8()?; // stay home
                }
            }
            Ok(Placement { location, group, species, position, yaw, unborn })
        })
        .collect()
}

const MONSTER_RECORD: usize = 0x297;
const CYCLE_NAME_AT: usize = 0x189;

/// Each species' model: the `.mls` name under `data/monsters`.
pub fn read_monster_models(data: &[u8]) -> Result<Vec<String>, PvlError> {
    let mut bytes = Bytes::new(data);
    let count = bytes.u32()? as usize;
    (0..count)
        .map(|_| {
            let record = bytes.take(MONSTER_RECORD)?;
            let name = &record[CYCLE_NAME_AT..CYCLE_NAME_AT + 48];
            let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
            Ok(String::from_utf8_lossy(&name[..end]).to_lowercase())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a retail installation"]
    fn the_monastery_is_populated_as_authored() {
        let root =
            std::env::var("RIZZARDRY_RETAIL").expect("set RIZZARDRY_RETAIL to run this test");
        let files = crate::retail::files::RetailFiles::open(root).unwrap();
        let models =
            read_monster_models(&files.read("data/databases/monsters.dbs").unwrap().unwrap())
                .unwrap();
        assert_eq!(models.len(), 595);
        let placements =
            read_placements(&files.read("levels/monastery/monastery1.sts").unwrap().unwrap())
                .unwrap();
        assert_eq!(placements.len(), 28);
        let burz = placements.iter().find(|p| p.species == 218).unwrap();
        assert_eq!(burz.position.map(f32::round), [112152.0, 17354.0, 34302.0]);
        assert_eq!(models[218], "trynnie_npc");
        for placement in &placements {
            let model = &models[placement.species as usize];
            assert!(files.contains(&format!("data/monsters/{model}.mls")), "{model}");
        }
        // Every level's state decodes.
        for name in files.names().filter(|n| n.ends_with(".sts")) {
            read_placements(&files.read(name).unwrap().unwrap())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }
}
