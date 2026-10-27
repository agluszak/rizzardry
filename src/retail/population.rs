//! Who stands where in a level: the monster placements of its `.sts` file.

use super::pvl::{PvlError, invalid};
use crate::bytes::{Bytes, lenient_string};
use crate::content::level::TRAP_TYPES;

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

/// A device's authored lock or trap (a `W8LockState` record of the `LOCK`
/// chunk, retail `SaveTriggerRuntimeStates` as the level editor writes it).
#[derive(Clone, Debug, PartialEq)]
pub struct LockRecord {
    /// The trigger it belongs to, upper case.
    pub trigger: String,
    pub kind: LockKind,
    /// The editor's 0 to 10 grade.
    pub difficulty: i32,
    /// A trap's type (index into retail's trap table), or `None` to roll
    /// one when the party first meets it.
    pub trap: Option<u8>,
    /// The key item, `None` for none.
    pub key: Option<u16>,
}

/// Retail `lock_type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockKind {
    Pickable,
    Trap,
    Key,
}

/// The authored locks and traps of a level's `.sts` file. Level state files
/// carry the full records the editor writes; the shorter runtime records a
/// saved game uses (`LCKS`) are not authored content.
pub fn read_locks(data: &[u8]) -> Result<Vec<LockRecord>, PvlError> {
    let Some(payload) = level_chunk(data, b"LOCK")? else { return Ok(Vec::new()) };
    let mut bytes = Bytes::new(payload);
    let version = bytes.i32()?;
    let count = bytes.i32()?.max(0);
    let full = bytes.i32()? != 0;
    if !full && version > 1 {
        return Err(invalid("level locks hold runtime records only", &bytes));
    }
    let mut locks = Vec::new();
    for _ in 0..count {
        let trigger = lenient_string(bytes.take(0x80)?).to_uppercase();
        let record_version = bytes.i32()?;
        let kind = bytes.i32()?;
        let difficulty = bytes.i32()?;
        bytes.take(1 + 8)?; // completed, pins: reset when a level first loads
        let device = bytes.i32()?;
        let key = bytes.i32()?;
        if record_version > 1 {
            bytes.i32()?; // practice countdown, reset with the pins
        }
        let kind = match kind {
            1 => LockKind::Pickable,
            2 => LockKind::Trap,
            3 => LockKind::Key,
            _ => continue,
        };
        locks.push(LockRecord {
            trigger,
            kind,
            difficulty,
            trap: u8::try_from(device).ok().filter(|&t| usize::from(t) < TRAP_TYPES),
            key: u16::try_from(key).ok().filter(|&k| k != 0),
        });
    }
    Ok(locks)
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

    fn chunk(id: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut bytes = id.to_vec();
        bytes.extend([0, 0]);
        bytes.extend((payload.len() as u32).to_le_bytes());
        bytes.extend(payload);
        bytes
    }

    /// A level state file holding one chunk.
    fn level_state(id: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut level = [1u32.to_le_bytes(), 7u32.to_le_bytes()].concat();
        level.extend(chunk(id, payload));
        let mut root = 1u32.to_le_bytes().to_vec();
        root.extend(chunk(b"LVLS", &level));
        chunk(b"RIFF", &root)
    }

    #[test]
    fn authored_locks_name_their_trigger_kind_grade_trap_and_key() {
        let record = |name: &str, kind: i32, difficulty: i32, device: i32, key: i32| {
            let mut bytes = name.as_bytes().to_vec();
            bytes.resize(0x80, 0);
            for word in [2, kind, difficulty] {
                bytes.extend(word.to_le_bytes());
            }
            bytes.extend([1; 9]); // completed and pins, reset on load
            for word in [device, key, 9] {
                bytes.extend(word.to_le_bytes());
            }
            bytes
        };
        let mut payload = [2i32, 4, 1].map(i32::to_le_bytes).concat();
        payload.extend(record("Chest1", 2, 5, -1, -1));
        payload.extend(record("chest2", 2, 3, 6, -1));
        payload.extend(record("Door", 1, 4, -1, 612));
        payload.extend(record("Gate", 3, 0, -1, 77));
        let locks = read_locks(&level_state(b"LOCK", &payload)).unwrap();
        let summary: Vec<_> = locks
            .iter()
            .map(|l| (l.trigger.as_str(), l.kind, l.difficulty, l.trap, l.key))
            .collect();
        assert_eq!(
            summary,
            [
                ("CHEST1", LockKind::Trap, 5, None, None),
                ("CHEST2", LockKind::Trap, 3, Some(6), None),
                ("DOOR", LockKind::Pickable, 4, None, Some(612)),
                ("GATE", LockKind::Key, 0, None, Some(77)),
            ]
        );
        // A saved game's short records are not authored content.
        let runtime = [2i32, 0, 0].map(i32::to_le_bytes).concat();
        assert!(read_locks(&level_state(b"LOCK", &runtime)).is_err());
        assert!(read_locks(&level_state(b"MONS", &runtime)).unwrap().is_empty());
    }

    /// Every level's locks decode and name triggers of their level; the
    /// monastery's are listed.
    #[test]
    #[ignore = "requires a retail installation"]
    fn every_authored_lock_belongs_to_a_trigger_of_its_level() {
        let files = crate::retail::files::retail_files();
        for name in files.names().filter(|n| n.ends_with(".sts")) {
            let locks = read_locks(&files.read(name).unwrap().unwrap())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let level = name.replace(".sts", ".pvl");
            let Some(Ok(data)) = files.read(&level) else { continue };
            let geometry = super::super::pvl::read_level_geometry(&data).unwrap();
            let sections =
                super::super::sections::read_sections(&mut Bytes::new(&data[geometry.end..]))
                    .unwrap();
            for lock in &locks {
                assert!(sections.trigger(&lock.trigger).is_some(), "{name}: {lock:?}");
                if name.contains("monastery") {
                    println!("{name}: {lock:?}");
                }
            }
        }
    }

    #[test]
    #[ignore = "requires a retail installation"]
    fn the_monastery_has_items_lying_about() {
        let files = crate::retail::files::retail_files();
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
        let files = crate::retail::files::retail_files();
        let species = crate::retail::species::read_species(
            &files.read("data/databases/monsters.dbs").unwrap().unwrap(),
        )
        .unwrap();
        let models: Vec<&str> = species.iter().map(|s| s.model.as_str()).collect();
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
        // Every level's state decodes and places only monsters and items
        // that exist, so a level that fails preparation is a broken install.
        let catalog = crate::retail::items::retail_catalog(&files);
        for name in files.names().filter(|n| n.ends_with(".sts")) {
            let state = files.read(name).unwrap().unwrap();
            for placement in read_placements(&state).unwrap_or_else(|e| panic!("{name}: {e}")) {
                let model = models.get(placement.species as usize).unwrap_or_else(|| {
                    panic!("{name}: species {} is not in the catalog", placement.species)
                });
                assert!(files.contains(&format!("data/monsters/{model}.mls")), "{name}: {model}");
            }
            for item in read_world_items(&state).unwrap() {
                let id = crate::content::items::ItemId(item.item as u16);
                assert!(
                    catalog.get(id).is_some(),
                    "{name}: item {} is not in the catalog",
                    item.item
                );
            }
        }
    }
}
