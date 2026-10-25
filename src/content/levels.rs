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
}

const fn level(
    folder: &'static str,
    file: &'static str,
    location: &'static str,
    sky: i8,
) -> LevelInfo {
    LevelInfo { folder, file, location, sky: if sky < 0 { None } else { Some(sky as usize) } }
}

pub const LEVELS: [LevelInfo; 40] = [
    level("Arnika", "Arnika2", "ARN", 1),
    level("Ascension", "Ascension", "ASC", 0),
    level("Bayjin", "Bayjin1", "BA1", -1),
    level("Bayjin", "Bayjin2", "BA2", -1),
    level("Circle", "Circle", "CIR", 10),
    level("Marten", "Marten1", "MR1", -1),
    level("Marten", "Marten2", "MR2", 0),
    level("MountainPass", "MountainPass", "MNT", 2),
    level("Monastery", "Monastery1", "MO1", 0),
    level("Monastery", "Monastery2", "MO2", 0),
    level("MtGigas", "MtGigasWaterCaves", "MGW", -1),
    level("MtGigas", "MtGigasBelowCaves", "MGB", -1),
    level("MtGigas", "MtGigas1", "MG1", -1),
    level("MtGigas", "MtGigas2", "MG2", -1),
    level("MtGigas", "MtGigasOuter", "MGO", 0),
    level("MtGigas", "MtGigasTop", "MGT", 0),
    level("Camp", "Camp", "CMP", 7),
    level("Rapax", "RapaxCellar", "RAC", 1),
    level("Rapax", "RapaxMainFloor", "RAM", 1),
    level("Rapax", "RapaxUpperFloor", "RAU", 1),
    level("Rapax", "RapaxExterior", "RAE", 1),
    level("Rift", "Rift1", "RIF", 3),
    level("SeaCaves", "SeaCave1", "SC1", 0),
    level("SeaCaves", "SeaCave2", "SC2", 0),
    level("Swamp", "Swamp", "SWM", -1),
    level("Trynnie", "Trynnie1", "TY1", -1),
    level("Trynnie", "Trynnie2", "TY2", -1),
    level("ConnectiveTissue", "Arnika_Trynton", "CT1", 0),
    level("ConnectiveTissue", "Trynton_Swamp", "CT2", 0),
    level("ConnectiveTissue2", "SouthEastWilderness", "CT3", 0),
    level("ConnectiveTissue", "Rift_Peak", "CT4", 0),
    level("ConnectiveTissue2", "NorthEastWilderness", "CT5", 0),
    level("ConnectiveTissue", "NorthWilderness", "CT6", 0),
    level("ConnectiveTissue", "Arnika_StarterDungeon", "CT7", 0),
    level("ConnectiveTissue", "Peak_RapaxCastle", "CT8", 0),
    level("Footsteps", "Footsteps", "FS1", 0),
    level("SavantTower", "SavantTower", "SAV", -1),
    level("Trynnie", "Ratkin", "RTK", -1),
    level("Camp", "CampNoRapax", "CNR", 7),
    level("Dungeon", "Dungeon", "DUN", -1),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_monastery_uses_the_default_sky() {
        let monastery = LevelInfo::for_path("levels/monastery/monastery1.pvl").unwrap();
        assert_eq!(monastery.location, "MO1");
        assert_eq!(monastery.sky_path().unwrap(), DEFAULT_SKY);
        assert_eq!(
            LevelInfo::for_path("levels/circle/circle.pvl").unwrap().sky_path().unwrap(),
            CIRCLE_SKY
        );
        assert!(LevelInfo::for_path("levels/dungeon/dungeon.pvl").unwrap().sky_path().is_none());
    }

    #[test]
    fn entrances_name_their_level_by_location_code() {
        assert_eq!(
            LevelInfo::for_entrance("MO201").unwrap().path(),
            "levels/monastery/monastery2.pvl"
        );
        assert_eq!(LevelInfo::for_entrance("mo100").unwrap().location, "MO1");
        assert!(LevelInfo::for_entrance("XX").is_none());
    }
}
