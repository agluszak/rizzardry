//! Who stands where in a level: the monster placements of its `.sts` file.
//! See `docs/research/level-population.md`.

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

/// A level state chunk's payload, if the file has it.
fn level_chunk<'a>(data: &'a [u8], wanted: &[u8; 4]) -> Result<Option<&'a [u8]>, PvlError> {
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
        if &id == wanted {
            return Ok(Some(payload));
        }
    }
    Ok(None)
}

/// The monsters placed in a level's `.sts` file.
pub fn read_placements(data: &[u8]) -> Result<Vec<Placement>, PvlError> {
    level_chunk(data, b"MONS")?.map_or(Ok(Vec::new()), read_monsters)
}

/// One item lying in a level (a `W8WorldItem` record of the `ITEM` chunk,
/// retail `LoadItem`).
#[derive(Clone, Debug, PartialEq)]
pub struct WorldItemRecord {
    /// Index into `items.dbs`.
    pub item: u32,
    /// A stack's count, or charges or uses, by the item's quantity kind.
    pub stack: u8,
    pub uses: u8,
    pub identified: bool,
    /// Retail units: the item's origin, about 125 units above the ground.
    pub position: [f32; 3],
    /// Unseen until found by searching (`W8_WORLD_ITEM_HIDDEN`).
    pub hidden: bool,
}

const WORLD_ITEM_SIZE: usize = 0xad;
const WORLD_ITEM_HIDDEN: u32 = 1;

/// The items lying in a level's `.sts` file. An entry may chain more items
/// into a pile (retail builds those at runtime for its item picker); no
/// retail level has one, and the chained items are skipped.
pub fn read_world_items(data: &[u8]) -> Result<Vec<WorldItemRecord>, PvlError> {
    let Some(payload) = level_chunk(data, b"ITEM")? else { return Ok(Vec::new()) };
    let mut bytes = Bytes::new(payload);
    let count = bytes.u32()?;
    (0..count)
        .map(|_| {
            let image = bytes.take(WORLD_ITEM_SIZE)?;
            let mut at = Bytes::new(&image[0x09..]);
            let item = at.u32()?;
            let (stack, uses, identified) = (at.u8()?, at.u8()?, at.u8()? != 0);
            let position = Bytes::new(&image[0x15..]).f32s::<3>()?;
            let mut at = Bytes::new(&image[0x29..]);
            let hidden = at.u32()? & WORLD_ITEM_HIDDEN != 0;
            at.u32()?; // saved marker
            let mut chained = at.u32()? != 0;
            while chained {
                let next = bytes.take(WORLD_ITEM_SIZE)?;
                chained = Bytes::new(&next[0x31..]).u32()? != 0;
            }
            Ok(WorldItemRecord { item, stack, uses, identified, position, hidden })
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a retail installation"]
    fn the_monastery_has_items_lying_about() {
        let root =
            std::env::var("RIZZARDRY_RETAIL").expect("set RIZZARDRY_RETAIL to run this test");
        let files = crate::retail::files::RetailFiles::open(root).unwrap();
        let items =
            read_world_items(&files.read("levels/monastery/monastery1.sts").unwrap().unwrap())
                .unwrap();
        assert_eq!(items.len(), 58);
        // The first: item 350 lying in the open; the second hidden.
        assert_eq!((items[0].item, items[0].hidden), (350, false));
        assert_eq!(items[0].position.map(f32::round), [89732.0, 6192.0, 83747.0]);
        assert!(items[1].hidden);
        // A stack of 21 of item 132.
        assert_eq!((items[2].item, items[2].stack), (132, 21));
    }

    #[test]
    #[ignore = "requires a retail installation"]
    fn the_monastery_is_populated_as_authored() {
        let root =
            std::env::var("RIZZARDRY_RETAIL").expect("set RIZZARDRY_RETAIL to run this test");
        let files = crate::retail::files::RetailFiles::open(root).unwrap();
        let species = crate::retail::species::read_species(
            &files.read("data/databases/monsters.dbs").unwrap().unwrap(),
        )
        .unwrap();
        let models: Vec<&str> = species.iter().map(|s| s.model.as_str()).collect();
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
