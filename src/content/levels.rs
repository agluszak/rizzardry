//! The retail level table (Whizzardry `g_level_folders`, from the
//! executable): where each level's files live, the location code its
//! triggers use, and its sky.

/// One retail level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LevelInfo {
    pub folder: &'static str,
    pub file: &'static str,
    /// The three letters entrance triggers start with, e.g. `MO1` + `00`.
    pub location: &'static str,
    /// Index into [`SKIES`]; `None` for levels without a sky.
    pub sky: Option<usize>,
    /// The region the level belongs to (retail `level_band`), which NPC
    /// services are keyed by; 0 for none.
    pub band: u8,
}

const fn level(
    folder: &'static str,
    file: &'static str,
    location: &'static str,
    sky: i8,
    band: u8,
) -> LevelInfo {
    LevelInfo { folder, file, location, sky: if sky < 0 { None } else { Some(sky as usize) }, band }
}

pub const LEVELS: [LevelInfo; 40] = [
    level("Arnika", "Arnika2", "ARN", 1, 2),
    level("Ascension", "Ascension", "ASC", 0, 12),
    level("Bayjin", "Bayjin1", "BA1", -1, 8),
    level("Bayjin", "Bayjin2", "BA2", -1, 8),
    level("Circle", "Circle", "CIR", 10, 14),
    level("Marten", "Marten1", "MR1", -1, 5),
    level("Marten", "Marten2", "MR2", 0, 5),
    level("MountainPass", "MountainPass", "MNT", 2, 6),
    level("Monastery", "Monastery1", "MO1", 0, 1),
    level("Monastery", "Monastery2", "MO2", 0, 1),
    level("MtGigas", "MtGigasWaterCaves", "MGW", -1, 15),
    level("MtGigas", "MtGigasBelowCaves", "MGB", -1, 11),
    level("MtGigas", "MtGigas1", "MG1", -1, 11),
    level("MtGigas", "MtGigas2", "MG2", -1, 11),
    level("MtGigas", "MtGigasOuter", "MGO", 0, 11),
    level("MtGigas", "MtGigasTop", "MGT", 0, 11),
    level("Camp", "Camp", "CMP", 7, 13),
    level("Rapax", "RapaxCellar", "RAC", 1, 9),
    level("Rapax", "RapaxMainFloor", "RAM", 1, 9),
    level("Rapax", "RapaxUpperFloor", "RAU", 1, 9),
    level("Rapax", "RapaxExterior", "RAE", 1, 9),
    level("Rift", "Rift1", "RIF", 3, 10),
    level("SeaCaves", "SeaCave1", "SC1", 0, 7),
    level("SeaCaves", "SeaCave2", "SC2", 0, 7),
    level("Swamp", "Swamp", "SWM", -1, 4),
    level("Trynnie", "Trynnie1", "TY1", -1, 3),
    level("Trynnie", "Trynnie2", "TY2", -1, 3),
    level("ConnectiveTissue", "Arnika_Trynton", "CT1", 0, 16),
    level("ConnectiveTissue", "Trynton_Swamp", "CT2", 0, 0),
    level("ConnectiveTissue2", "SouthEastWilderness", "CT3", 0, 17),
    level("ConnectiveTissue", "Rift_Peak", "CT4", 0, 0),
    level("ConnectiveTissue2", "NorthEastWilderness", "CT5", 0, 0),
    level("ConnectiveTissue", "NorthWilderness", "CT6", 0, 0),
    level("ConnectiveTissue", "Arnika_StarterDungeon", "CT7", 0, 0),
    level("ConnectiveTissue", "Peak_RapaxCastle", "CT8", 0, 0),
    level("Footsteps", "Footsteps", "FS1", 0, 0),
    level("SavantTower", "SavantTower", "SAV", -1, 2),
    level("Trynnie", "Ratkin", "RTK", -1, 3),
    level("Camp", "CampNoRapax", "CNR", 7, 0),
    level("Dungeon", "Dungeon", "DUN", -1, 0),
];

/// Sky worlds by index (Whizzardry `g_sky_names`). Retail ships only
/// `DefaultSky` (in `levels/test`) and `CircleSky`; a missing sky falls back
/// to the default.
pub const SKIES: [&str; 11] = [
    "DefaultSky",
    "RapaxSky",
    "MountainPassSky",
    "RiftSky1",
    "TrynnieSky1",
    "TrynnieSky2",
    "RatkinSky1",
    "CampSky",
    "BluffSky1",
    "BluffSky2",
    "CircleSky",
];

/// Each level's name in the game string table, by [`LEVELS`] index.
const LEVEL_NAMES: [u16; 40] = [
    0x6f7, 0x6f8, 0x6f9, 0x6fa, 0x6fb, 0x6fd, 0x6fc, 0x6fe, 0x6ff, 0x700, 0x701, 0x719, 0x702,
    0x703, 0x704, 0x705, 0x706, 0x707, 0x708, 0x709, 0x70a, 0x70b, 0x70c, 0x719, 0x70d, 0x70e,
    0x70f, 0x710, 0x719, 0x711, 0x719, 0x712, 0x713, 0x714, 0x719, 0x715, 0x716, 0x717, 0x706,
    0x718,
];

pub const DEFAULT_SKY: &str = "levels/test/defaultsky.lvl";
const CIRCLE_SKY: &str = "levels/circle/circlesky.lvl";

impl LevelInfo {
    /// The retail path of the level's `.pvl`, e.g. `levels/monastery/monastery1.pvl`.
    pub fn path(&self) -> String {
        format!("levels/{}/{}.pvl", self.folder, self.file).to_lowercase()
    }

    /// The retail path of the level's sky world. Retail ships only the
    /// default sky and the Circle's, and falls back to the default when a
    /// level's own sky is missing.
    pub fn sky_path(&self) -> Option<String> {
        let own = format!("levels/{}/{}.lvl", self.folder, SKIES[self.sky?]).to_lowercase();
        Some(if own == CIRCLE_SKY { own } else { DEFAULT_SKY.to_owned() })
    }

    /// The game string table entry naming the level (Whizzardry
    /// `g_level_name_indices`).
    pub fn name_string(&self) -> Option<usize> {
        let index = LEVELS.iter().position(|level| level == self)?;
        LEVEL_NAMES.get(index).map(|&string| usize::from(string))
    }

    /// The level an entrance name such as `MO201` leads to.
    pub fn for_entrance(entrance: &str) -> Option<&'static LevelInfo> {
        let code = entrance.get(..3)?;
        LEVELS.iter().find(|level| level.location.eq_ignore_ascii_case(code))
    }

    /// The level whose files are at `path` (a normalized retail path).
    pub fn for_path(path: &str) -> Option<&'static LevelInfo> {
        LEVELS.iter().find(|level| level.path() == path)
    }
}
