//! The `.pvl` sections after the compiled meshes (Whizzardry `ReadLevel`):
//! lights, props, cameras, environment, triggers, particles, named
//! positions and automap cells. Each section ends with an `i32` -1, except
//! the camera sway flag and the environment offset. Retail files end with 72
//! unread zero bytes.

use super::keyframes::{PathKey, read_path};
use super::mesh_record::RECORD_SCALE;
use super::prop::{Prop, read_prop};
use super::pvl::{PvlError, invalid, text};
use super::trigger::{Trigger, read_trigger};
use crate::bytes::{Bytes, lenient_string};

#[derive(Clone, Debug, PartialEq)]
pub struct Light {
    pub name: String,
    /// Retail units.
    pub location: [f32; 3],
    pub colour: [f32; 3],
    pub intensity: f32,
    pub range: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CameraPath {
    pub name: String,
    pub keys_per_second: f32,
    pub path: Vec<PathKey>,
}

/// The fog, ambient and view settings (Whizzardry `ReadWorldEnvironment`).
#[derive(Clone, Debug, PartialEq)]
pub struct Environment {
    pub fog_enabled: bool,
    /// Fog start and end as fractions of the far clip.
    pub fog_range: [f32; 2],
    /// Scales the ambient colour; 0 for interiors.
    pub intensity: f32,
    /// The far clip, in retail units.
    pub view_distance: f32,
    /// Fog and clear colour by time-of-day phase (0 midnight, 128 noon).
    pub fog_colours: Option<Box<[[u8; 3]; 256]>>,
    /// Ambient (sun and sky) light by time-of-day phase.
    pub ambient_colours: Option<Box<[[u8; 3]; 256]>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NamedPosition {
    pub name: String,
    pub position: [f32; 3],
    pub angle: f32,
    pub direction: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParticleSystem {
    pub name: String,
    pub location: [f32; 3],
}

/// A named horizontal clipping plane, at a height in retail units.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipPlane {
    pub name: String,
    pub height: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LevelSections {
    pub lights: Vec<Light>,
    pub props: Vec<Prop>,
    /// Billboard props: same records, drawn facing the camera.
    pub bitmaps: Vec<Prop>,
    pub cameras: Vec<CameraPath>,
    pub environment: Option<Environment>,
    pub triggers: Vec<Trigger>,
    pub camera_sway: bool,
    pub clip_planes: Vec<ClipPlane>,
    /// Added to every baked vertex colour.
    pub environment_offset: [f32; 3],
    pub particles: Vec<ParticleSystem>,
    pub named_positions: Vec<NamedPosition>,
    pub automap_cell_size: f32,
    pub automap_cells: Vec<u32>,
}

impl LevelSections {
    /// Retail registration order: props, billboards, then standalone triggers.
    pub fn all_triggers(&self) -> impl Iterator<Item = &Trigger> {
        self.props
            .iter()
            .chain(&self.bitmaps)
            .filter_map(|p| p.trigger.as_ref())
            .chain(&self.triggers)
    }

    pub fn trigger(&self, name: &str) -> Option<&Trigger> {
        self.all_triggers().filter(|t| t.name.eq_ignore_ascii_case(name)).last()
    }
}

/// Compiled levels (`.pvl`) end each section with `-1`; uncompiled ones
/// (`.lvl`, e.g. skies) do not.
fn sentinel(bytes: &mut Bytes, compiled: bool) -> Result<(), PvlError> {
    if compiled && bytes.i32()? != -1 {
        return Err(invalid("section has the wrong length", bytes));
    }
    Ok(())
}

fn count(bytes: &mut Bytes) -> Result<usize, PvlError> {
    let count = bytes.i32()?;
    if !(0..100_000).contains(&count) {
        return Err(invalid("invalid record count", bytes));
    }
    Ok(count as usize)
}

fn read_lights(bytes: &mut Bytes) -> Result<Vec<Light>, PvlError> {
    let count = bytes.i16()?.max(0);
    let mut lights = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let version = bytes.i16()?;
        bytes.take(2)?; // create, visible
        let flags = bytes.u32()?;
        let location = bytes.f32s::<3>()?.map(|v| v * RECORD_SCALE);
        let colour = bytes.f32s()?;
        let intensity = bytes.f32()?;
        let range = bytes.f32()? * RECORD_SCALE;
        let mut name = String::new();
        if version >= 2 {
            name = lenient_string(bytes.take(20)?);
            if flags & 2 != 0 {
                // Parametric animation: 15 words, path when its flags say so.
                let animation_flags = bytes.u32()?;
                bytes.take(14 * 4)?;
                if animation_flags & 0x10 != 0 {
                    read_path(bytes)?;
                }
            }
        }
        lights.push(Light { name, location, colour, intensity, range });
    }
    Ok(lights)
}

fn read_environment(bytes: &mut Bytes) -> Result<Option<Environment>, PvlError> {
    if bytes.i32()? == 0 {
        return Ok(None);
    }
    let fog_enabled = bytes.u8()? != 0;
    let [start, end, _] = bytes.f32s::<3>()?;
    let intensity = bytes.f32()?;
    let view_distance = bytes.f32()? * RECORD_SCALE;
    match bytes.u8()? {
        1 => {
            bytes.take(12)?;
        }
        2 => {
            bytes.take(12 + 4 + 12)?;
        }
        _ => {}
    }
    let table = |bytes: &mut Bytes| -> Result<Option<Box<[[u8; 3]; 256]>>, PvlError> {
        if bytes.u8()? == 0 {
            return Ok(None);
        }
        let raw = bytes.take(768)?;
        let mut colours = Box::new([[0u8; 3]; 256]);
        colours.copy_from_slice(raw.as_chunks::<3>().0);
        Ok(Some(colours))
    };
    let fog_colours = table(bytes)?;
    let ambient_colours = table(bytes)?;
    Ok(Some(Environment {
        fog_enabled,
        fog_range: [start, end],
        intensity,
        view_distance,
        fog_colours,
        ambient_colours,
    }))
}

const PARTICLE_RECORD: [usize; 4] = [0x216, 0x218, 0x21d, 0x225];

/// Read a compiled level's sections, starting where its meshes end.
pub fn read_sections(bytes: &mut Bytes) -> Result<LevelSections, PvlError> {
    read_sections_of(bytes, true)
}

/// Read an uncompiled level's sections, starting after its mesh record.
pub fn read_uncompiled_sections(bytes: &mut Bytes) -> Result<LevelSections, PvlError> {
    read_sections_of(bytes, false)
}

fn read_sections_of(bytes: &mut Bytes, compiled: bool) -> Result<LevelSections, PvlError> {
    let lights = read_lights(bytes)?;
    sentinel(bytes, compiled)?;
    for _ in 0..count(bytes)? {
        // Monster paths: unused by retail levels.
        text(bytes, 20)?;
        read_path(bytes)?;
    }
    sentinel(bytes, compiled)?;
    for _ in 0..count(bytes)? {
        // Items: unused by retail levels.
        bytes.take(20 + 12 + 16)?;
        if bytes.u8()? != 0 {
            read_trigger(bytes)?;
        }
        bytes.take(3 + 16)?;
    }
    sentinel(bytes, compiled)?;
    bytes.i32()?; // missile count, no records
    sentinel(bytes, compiled)?;
    let props = (0..count(bytes)?).map(|_| read_prop(bytes)).collect::<Result<_, _>>()?;
    sentinel(bytes, compiled)?;
    let bitmaps = (0..count(bytes)?).map(|_| read_prop(bytes)).collect::<Result<_, _>>()?;
    sentinel(bytes, compiled)?;
    let mut cameras = Vec::new();
    for _ in 0..count(bytes)? {
        bytes.take(8)?;
        let has_speed = bytes.u8()? != 0;
        let name = text(bytes, 20)?.to_uppercase();
        let keys_per_second = if has_speed { bytes.f32()? } else { 15.0 };
        cameras.push(CameraPath { name, keys_per_second, path: read_path(bytes)? });
    }
    sentinel(bytes, compiled)?;
    let environment = read_environment(bytes)?;
    sentinel(bytes, compiled)?;
    let triggers = (0..count(bytes)?).map(|_| read_trigger(bytes)).collect::<Result<_, _>>()?;
    sentinel(bytes, compiled)?;
    let camera_sway = bytes.i32()? != 0;
    let clip_count = count(bytes)?;
    let mut clip_planes = Vec::with_capacity(clip_count);
    if clip_count > 0 {
        bytes.u8()?; // The retail loader reads but does not branch on this version.
        for _ in 0..clip_count {
            let name = text(bytes, 64)?.to_uppercase();
            let position = bytes.f32s::<4>()?;
            clip_planes.push(ClipPlane { name, height: position[1] * RECORD_SCALE });
        }
    }
    sentinel(bytes, compiled)?;
    let environment_offset = bytes.f32s()?;
    let mut particles = Vec::new();
    for _ in 0..count(bytes)? {
        let version = bytes.u8()?;
        let size = *PARTICLE_RECORD
            .get((version as usize).wrapping_sub(1))
            .ok_or_else(|| invalid("unknown particle system version", bytes))?;
        let record = bytes.take(size)?;
        let mut location = Bytes::new(&record[0x40..0x4c]);
        particles.push(ParticleSystem {
            name: lenient_string(&record[..64]),
            location: location.f32s::<3>()?.map(|v| v * RECORD_SCALE),
        });
    }
    sentinel(bytes, compiled)?;
    let mut named_positions = Vec::new();
    for _ in 0..count(bytes)? {
        if bytes.u8()? != 1 {
            return Err(invalid("unknown named position version", bytes));
        }
        named_positions.push(NamedPosition {
            name: text(bytes, 0x80)?.to_uppercase(),
            position: bytes.f32s::<3>()?.map(|v| v * RECORD_SCALE),
            angle: bytes.f32()?,
            direction: bytes.f32s()?,
        });
    }
    sentinel(bytes, compiled)?;
    let automap_cell_size = bytes.f32()?;
    let cells = bytes.u32()? as usize;
    let automap_cells = bytes.records(cells, 4, |b| b.u32())?;
    sentinel(bytes, compiled)?;
    Ok(LevelSections {
        lights,
        props,
        bitmaps,
        cameras,
        environment,
        triggers,
        camera_sway,
        clip_planes,
        environment_offset,
        particles,
        named_positions,
        automap_cell_size,
        automap_cells,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retail::files::retail_files;
    use crate::retail::pvl::read_level_geometry;
    use crate::retail::trigger::TriggerKind;

    #[test]
    #[ignore = "requires a retail installation"]
    fn every_retail_level_parses_to_its_trailer() {
        let files = retail_files();
        let levels: Vec<String> =
            files.names().filter(|n| n.ends_with(".pvl")).map(str::to_owned).collect();
        assert_eq!(levels.len(), 34);
        for level in levels {
            let data = files.read(&level).unwrap().unwrap();
            let geometry = read_level_geometry(&data).unwrap_or_else(|e| panic!("{level}: {e}"));
            assert!(!geometry.meshes.is_empty(), "{level}");
            let mut bytes = Bytes::new(&data[geometry.end..]);
            let sections = read_sections(&mut bytes).unwrap_or_else(|e| panic!("{level}: {e}"));
            assert_eq!(bytes.remaining(), 72, "{level}");
            assert!(sections.environment.is_some(), "{level}");
        }
    }

    #[test]
    #[ignore = "requires a retail installation"]
    fn the_default_sky_parses() {
        let files = retail_files();
        let data = files.read("levels/test/defaultsky.lvl").unwrap().unwrap();
        let mut bytes = Bytes::new(&data);
        crate::retail::mesh_record::read_mesh_record(&mut bytes).unwrap();
        let sky = read_uncompiled_sections(&mut bytes).unwrap();
        let names: Vec<&str> =
            sky.props.iter().chain(&sky.bitmaps).map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"Sun") && names.contains(&"Moon"), "{names:?}");
    }

    #[test]
    #[ignore = "requires a retail installation"]
    fn the_monastery_bells_decode_their_trigger_logic() {
        use crate::retail::trigger::{StateMode, action, flag};
        let files = retail_files();
        let data = files.read("levels/monastery/monastery2.pvl").unwrap().unwrap();
        let start = read_level_geometry(&data).unwrap().end;
        let sections = read_sections(&mut Bytes::new(&data[start..])).unwrap();
        let item = |name: &str| match &sections.trigger(name).unwrap().kind {
            TriggerKind::Item(item) => item.clone(),
            other => panic!("{name}: {other:?}"),
        };
        let dial = item("BELLRINGSWITCH");
        assert_eq!(dial.initial_action, 0x40);
        assert!(dial.play_sound);
        assert_eq!(dial.sound.to_lowercase(), "ambients\\lock open small.wav");
        assert_eq!((dial.state_to_modify.as_str(), dial.state_mode), ("RING", StateMode::Set));
        let glass = item("BELLTRIGGER01");
        assert_eq!(glass.flags, flag::ITEM | flag::ONCE);
        assert!(glass.fire_linked && !glass.enabled);
        assert_eq!(glass.required_states, ["RING2"]);
        let relay = item("BELLTRIGGERC");
        assert_eq!(relay.initial_action, action::RELAY);
        assert_eq!(
            (relay.required_states.as_slice(), relay.recipients.as_str()),
            (&["RING0".to_owned()][..], "BELL_1")
        );
        let dial_prop =
            sections.props.iter().chain(&sections.bitmaps).find(|p| p.name == "dial1").unwrap();
        let frames: Vec<u16> = dial_prop.slots.iter().map(|s| s.frame).collect();
        assert_eq!(frames, [0, 1, 10, 20]);
    }
}
