//! Monster species (`.mls`, text) and animation cycles (`.mon`, binary).

use super::keyframes::read_path;
use super::mesh_record::{MeshRecord, MeshVertices};
use super::prop::read_animation;
use super::pvl::{PvlError, invalid};
use crate::bytes::Bytes;
use crate::content::monster::MonsterBody;

/// One animation cycle: a mesh whose vertices move frame by frame.
#[derive(Clone, Debug, PartialEq)]
pub struct MonsterCycle {
    pub frames_per_second: f32,
    /// Topology, UVs and materials; vertex positions are in `frames`.
    pub mesh: MeshRecord,
    /// Vertex positions per frame, in retail units.
    pub frames: Vec<Vec<[f32; 3]>>,
}

const PARTICLE_RECORD: [usize; 4] = [0x216, 0x218, 0x21d, 0x225];

pub fn read_cycle(data: &[u8]) -> Result<MonsterCycle, PvlError> {
    let mut bytes = Bytes::new(data);
    if bytes.u8()? != 1 {
        return Err(invalid("unsupported monster cycle version", &bytes));
    }
    if bytes.u8()? != 0 {
        read_path(&mut bytes)?;
    }
    if bytes.u8()? != 0 {
        for _ in 0..bytes.i32()?.max(0) {
            let version = bytes.u8()? as usize;
            let size = *PARTICLE_RECORD
                .get(version.wrapping_sub(1))
                .ok_or_else(|| invalid("unknown particle version", &bytes))?;
            bytes.take(size)?;
        }
    }
    let animation = read_animation(&mut bytes, true)?;
    // The nearest level of detail is the highest non-empty channel.
    let part = animation
        .channels
        .iter()
        .rev()
        .find_map(|channel| channel.first())
        .ok_or_else(|| invalid("monster cycle has no mesh", &bytes))?;
    let records = &part.mesh.frames;
    let mesh = records[0].clone();
    let frames = match &mesh.vertices {
        MeshVertices::Frames(frames) => frames.clone(),
        // A few cycles store one record per frame.
        MeshVertices::Static(_) => records.iter().map(|r| r.vertices.first().to_vec()).collect(),
    };
    if frames.iter().any(|frame| frame.len() != frames[0].len()) {
        return Err(invalid("monster cycle frames differ in vertex count", &bytes));
    }
    Ok(MonsterCycle { frames_per_second: animation.frames_per_second, mesh, frames })
}

/// A monster species: its cycles and body measurements.
#[derive(Clone, Debug, PartialEq)]
pub struct MonsterSpecies {
    /// Cycle name (e.g. `IDLE`, `WALK`, `DIE`), cycle file stem and an
    /// optional frame-rate override. Only the first file per cycle is kept.
    pub cycles: Vec<(String, String, Option<f32>)>,
    pub body: MonsterBody,
}

/// The 27 retail cycle names (`g_cycle_names`), plus legacy aliases.
const CYCLES: [&str; 27] = [
    "BIRTH",
    "IDLE",
    "SPICE",
    "TRANSITION",
    "WALK",
    "DRAW",
    "ATTACK_CLOSE",
    "ATTACK_RANGED",
    "ATTACK_SPARE",
    "ATTACK_SWING",
    "ATTACK_THRUST",
    "ATTACK_BASH",
    "ATTACK_MELEE",
    "ATTACK_THROW",
    "ATTACK_PUNCH",
    "ATTACK_KICK",
    "ATTACK_LASH",
    "ATTACK_SHOOT",
    "ATTACK_SPECIAL",
    "DODGE",
    "GET_HIT",
    "DIE",
    "TURN",
    "TALK_SPICE",
    "TALK",
    "SPELL",
    "SPECIAL",
];

/// The cycle a `.mls` keyword names, by retail's prefix match (`IDLE2`
/// still names `IDLE`); `FLY` and `EXPLODE` are legacy names.
fn cycle_name(keyword: &str) -> Option<&'static str> {
    match keyword {
        k if k.starts_with("FLY") => return Some("BIRTH"),
        k if k.starts_with("EXPLODE") => return Some("IDLE"),
        _ => {}
    }
    // Longest match first, so ATTACK_SPECIAL is not read as ATTACK_SPARE.
    CYCLES.iter().filter(|name| keyword.starts_with(*name)).max_by_key(|name| name.len()).copied()
}

pub fn read_species(text: &str) -> MonsterSpecies {
    let mut species = MonsterSpecies { cycles: vec![], body: MonsterBody::default() };
    let (mut range_start, mut range_end) = (None, None);
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or_default();
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let Some(&keyword) = tokens.first() else { continue };
        let number = |i: usize| tokens.get(i).and_then(|t| t.parse::<f32>().ok());
        match keyword.to_lowercase().as_str() {
            "scalefactor" => species.body.scale = number(1).unwrap_or(1.0),
            "scalerangestart" => range_start = number(1),
            "scalerangeend" => range_end = number(1),
            // Retail keeps the default for a radius of 0.
            "walkradius" => {
                species.body.walk_radius = number(1).filter(|&r| r != 0.0).unwrap_or(1.0)
            }
            "fightradius" => {
                species.body.fight_radius = number(1).filter(|&r| r != 0.0).unwrap_or(1.0)
            }
            // At least 250 world units, half a metre.
            "targetheight" => {
                if let Some(height) = number(1).filter(|&h| h != 0.0) {
                    species.body.target_height = height.max(0.5);
                }
            }
            "movementrate" => {
                let rate = number(1).unwrap_or(3.0);
                species.body.movement_rate = if rate < 0.1 { 2.0 } else { rate };
            }
            _ => {
                if let (Some(cycle), Some(file)) = (cycle_name(keyword), tokens.get(1))
                    && !species.cycles.iter().any(|(c, _, _)| c == cycle)
                {
                    let fps = number(2).filter(|&fps| fps > 0.0);
                    species.cycles.push((cycle.to_owned(), file.to_lowercase(), fps));
                }
            }
        }
    }
    // A range sets the scale to its middle (retail `LoadMonsterDefinition`).
    if let (Some(low), Some(high)) = (range_start, range_end) {
        species.body.scale_range = Some((low, high));
        species.body.scale = (high - low) * 0.5 + low;
    }
    species
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn species_files_name_cycles_by_prefix() {
        let species = read_species(
            "# comment\nIDLE trooper_idle\nIDLE2 other\nATTACK_SPECIAL x 12\nscalefactor 1.25\nwalkradius 0.675\n",
        );
        assert_eq!(species.cycles[0], ("IDLE".into(), "trooper_idle".into(), None));
        assert_eq!(species.cycles[1], ("ATTACK_SPECIAL".into(), "x".into(), Some(12.0)));
        assert_eq!(species.cycles.len(), 2);
        assert_eq!((species.body.scale, species.body.walk_radius), (1.25, 0.675));
        assert_eq!(species.body.fight_radius, 1.0, "retail's default");
        let crab = read_species("scalerangestart .3\nscalerangeend .6\nfightradius 2.5\n");
        assert_eq!(crab.body.scale_range, Some((0.3, 0.6)));
        assert!((crab.body.scale - 0.45).abs() < 1e-6, "the middle of the range");
        assert_eq!(crab.body.scale_for(1000), 0.6);
    }

    #[test]
    #[ignore = "requires a retail installation"]
    fn every_retail_cycle_decodes() {
        let files = crate::retail::files::retail_files();
        let mut decoded = 0;
        for name in files.names().filter(|n| n.ends_with(".mon")) {
            let data = files.read(name).unwrap().unwrap();
            if data.is_empty() {
                continue; // nessie_attack_speed.mon, unused
            }
            read_cycle(&data).unwrap_or_else(|e| panic!("{name}: {e}"));
            decoded += 1;
        }
        assert!(decoded >= 1088, "{decoded}");
        let text = files.read("data/monsters/savant_trooper.mls").unwrap().unwrap();
        let species = read_species(&String::from_utf8_lossy(&text));
        let idle = &species.cycles.iter().find(|(c, _, _)| c == "IDLE").unwrap().1;
        let cycle = read_cycle(&files.read(&format!("data/monsters/{idle}.mon")).unwrap().unwrap())
            .unwrap();
        assert_eq!((cycle.frames.len(), cycle.frames[0].len()), (30, 199));
    }
}
