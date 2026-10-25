//! Loads a retail `.pvl` level's static geometry as [`LevelGeometry`].

use std::collections::HashMap;

use bevy::asset::{AssetLoader, AssetPath, LoadContext, RenderAssetUsages, io::Reader};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::reflect::TypePath;

use super::bytes::Bytes;
use super::links::{level_entrances, level_links, teleport_arrival};
use super::oct::read_collision;
use super::prop::{PROP_COLLIDABLE, Prop};
use super::prop_loader::{
    PropLighting, level_lighting, part_transforms, record_collision, record_meshes,
};
use super::pvl::{CompiledMesh, MaterialRecord, PvlError, read_level_geometry};
use super::sections::LevelSections;
use super::sections::{Environment, read_sections};
use super::space::UNITS_PER_METRE;
use super::space::{retail_direction, retail_position};
use super::texture_loader::file_has_alpha;
use super::trigger::{ItemTrigger, StateMode, TriggerKind, action as trigger_action, flag};
use crate::content::level::{
    DoorLock, LevelEnvironment, LevelGeometry, LevelPart, Playback, PropModel, PropModelPart,
    StateChange, TextureAnimation, TriggerDefinition, TriggerSound, VertexLighting,
};

/// Time-of-day phase used for baked lighting until the game clock exists:
/// noon, when a new game starts.
const NOON: u8 = 128;

fn level_environment(environment: &Environment) -> LevelEnvironment {
    let table = |colours: &Option<Box<[[u8; 3]; 256]>>| match colours {
        Some(colours) => colours.iter().map(|c| Vec3::from(c.map(f32::from)) / 255.0).collect(),
        // Retail's built-in default: grey, brightest at noon.
        None => (0..256)
            .map(|i: i32| Vec3::splat((if i < 128 { i } else { 255 - i }) as f32 / 127.0))
            .collect(),
    };
    LevelEnvironment {
        fog_range: (environment.fog_range[0], environment.fog_range[1]),
        view_distance: environment.view_distance / UNITS_PER_METRE,
        intensity: environment.intensity,
        fog_colours: table(&environment.fog_colours),
        ambient_colours: table(&environment.ambient_colours),
    }
}

#[derive(Default, TypePath)]
pub struct PvlLoader;

#[derive(Debug, thiserror::Error)]
pub enum PvlLoadError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Pvl(#[from] PvlError),
    #[error(transparent)]
    Read(#[from] bevy::asset::ReadAssetBytesError),
    #[error("cannot name the level's octree file")]
    Path,
    #[error("invalid automap layers: {0}")]
    MapLayers(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Blend {
    Opaque,
    Translucent,
    Additive,
}

pub(super) struct LoadedMaterial {
    pub(super) handle: Handle<StandardMaterial>,
    pub(super) emission: f32,
    /// The material's ambient reflectance, which scales sun and sky light.
    pub(super) ambient: Vec3,
    pub(super) animation: Option<TextureAnimation>,
}

/// A path in `directory` next to the asset being loaded, in the same source.
pub(super) fn sibling(
    context: &LoadContext,
    directory: &str,
    file: &str,
) -> Option<AssetPath<'static>> {
    context.path().resolve_embed_str(&format!("{directory}/{file}")).ok()
}

/// The frame files an `.ifl` lists. Retail stops at the first line of two
/// characters or fewer, so trailing blank lines end the list.
fn ifl_frames(list: &str) -> Vec<String> {
    list.lines()
        .map(str::trim_end)
        .take_while(|line| line.len() > 2)
        .map(str::to_lowercase)
        .collect()
}

/// Textures this load has already looked at, by file name, with whether they
/// carry alpha; `None` records a texture that is missing. The images
/// themselves are shared assets.
#[derive(Default)]
pub(super) struct LoadedTextures(HashMap<String, Option<(Handle<Image>, bool)>>);

/// One texture: a shared image handle, and whether retail treats it as
/// carrying alpha, which only needs the file header.
async fn load_image(
    context: &mut LoadContext<'_>,
    directory: &str,
    file: &str,
    textures: &mut LoadedTextures,
) -> Option<(Handle<Image>, bool)> {
    if let Some(loaded) = textures.0.get(file) {
        return loaded.clone();
    }
    let path = sibling(context, directory, file)?;
    let loaded = match context.read_asset_bytes(path.clone()).await {
        Err(error) => {
            warn!("{}: missing texture {path}: {error}", context.path());
            None
        }
        Ok(bytes) => {
            let extension = file.rsplit('.').next().unwrap_or_default();
            Some((context.load(path.with_label("image")), file_has_alpha(&bytes, extension)))
        }
    };
    textures.0.insert(file.to_owned(), loaded.clone());
    loaded
}

/// A texture's frames: one image, or every frame an `.ifl` lists.
async fn load_frames(
    context: &mut LoadContext<'_>,
    directory: &str,
    name: &str,
    textures: &mut LoadedTextures,
) -> Vec<(Handle<Image>, bool)> {
    let file = name.to_lowercase();
    if !file.ends_with(".ifl") {
        return load_image(context, directory, &file, textures).await.into_iter().collect();
    }
    let Some(path) = sibling(context, directory, &file) else { return vec![] };
    let Ok(list) = context.read_asset_bytes(path).await else {
        warn!("{}: missing frame list {file}", context.path());
        return vec![];
    };
    let mut frames = Vec::new();
    for frame in ifl_frames(&String::from_utf8_lossy(&list)) {
        frames.extend(load_image(context, directory, &frame, textures).await);
    }
    frames
}

pub(super) async fn load_material(
    context: &mut LoadContext<'_>,
    label: String,
    directory: &str,
    textures: &mut LoadedTextures,
    index: usize,
    record: &MaterialRecord,
) -> LoadedMaterial {
    let selected = if index == 0 { None } else { record.texture() };
    let mut frames = Vec::new();
    let mut alpha = false;
    let mut slot = 0;
    if let Some((texture_slot, name)) = selected {
        slot = texture_slot;
        // Retail: a texture found only in the fourth slot carries alpha.
        alpha = slot == 3;
        frames = load_frames(context, directory, name, textures).await;
        alpha |= frames.iter().any(|(_, frame_alpha)| *frame_alpha);
    }
    // Retail's packed shaders: slots 0-1 blend by alpha, slots 2-3 add
    // colour times alpha (glows, flares, sparks, beams; black adds nothing).
    let blend = if record.opacity < 1.0 || alpha {
        if slot <= 1 { Blend::Translucent } else { Blend::Additive }
    } else {
        Blend::Opaque
    };
    // Retail takes an opacity of 0 as 0.7.
    let opacity = match record.opacity {
        _ if index == 0 => 1.0,
        0.0 => 0.7,
        opacity => opacity.clamp(0.0, 1.0),
    };
    let material = StandardMaterial {
        base_color: Color::srgba(1.0, 1.0, 1.0, if blend == Blend::Opaque { 1.0 } else { opacity }),
        base_color_texture: frames.first().map(|(handle, _)| handle.clone()),
        unlit: true,
        double_sided: record.two_sided(),
        cull_mode: if record.two_sided() {
            None
        } else {
            Some(bevy::render::render_resource::Face::Back)
        },
        alpha_mode: match blend {
            Blend::Opaque => AlphaMode::Opaque,
            Blend::Translucent => AlphaMode::Blend,
            Blend::Additive => AlphaMode::Add,
        },
        ..default()
    };
    let handle = context.add_labeled_asset(label, material);
    let animation = (frames.len() > 1).then(|| TextureAnimation {
        material: handle.clone(),
        frames: frames.into_iter().map(|(frame, _)| frame).collect(),
        playback: match record.animation_mode {
            1 => Playback::PingPong,
            2 => Playback::Once,
            3 => Playback::Manual,
            _ => Playback::Loop,
        },
        frames_per_second: record.animation_rate,
        start_frame: record.animation_frame.max(0) as usize,
    });
    LoadedMaterial {
        handle,
        emission: record.emission,
        ambient: Vec3::from(record.ambient),
        animation,
    }
}

/// Every type 4 trigger of the level, attached to a prop or not.
fn level_triggers(sections: &LevelSections) -> Vec<TriggerDefinition> {
    sections
        .all_triggers()
        .filter_map(|trigger| match &trigger.kind {
            TriggerKind::Item(item) => Some(trigger_definition(sections, &trigger.name, item)),
            _ => None,
        })
        .collect()
}

fn trigger_definition(
    sections: &LevelSections,
    name: &str,
    item: &ItemTrigger,
) -> TriggerDefinition {
    let sound = (item.play_sound && !item.sound.is_empty()).then(|| TriggerSound {
        path: format!("data/sound/{}", item.sound.replace('\\', "/")).to_lowercase(),
        volume: f32::from(item.sound_volume) / 100.0,
    });
    let state_change = match item.state_mode {
        _ if item.state_to_modify.is_empty() => None,
        StateMode::Keep => None,
        StateMode::Set => Some(StateChange::Set),
        StateMode::Clear => Some(StateChange::Clear),
        StateMode::Toggle => Some(StateChange::Toggle),
    };
    TriggerDefinition {
        name: name.to_owned(),
        usable: item.enabled,
        once: item.flags & flag::ONCE != 0,
        fire_linked: item.fire_linked,
        initial: item.initial_action,
        alternate: item.alternate_action,
        fallback: item.fallback_action,
        alternate_toggles: item.alternate_toggles,
        recipients: item
            .recipients
            .split(',')
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(str::to_owned)
            .collect(),
        data: item.data,
        sound,
        shows_message: item.flags & flag::SHOW_MESSAGE != 0,
        required_states: item.required_states.clone(),
        state_change: state_change.map(|change| (item.state_to_modify.clone(), change)),
        initial_state: item.initial_state,
        range: (item.range[0] / UNITS_PER_METRE, item.range[1] / UNITS_PER_METRE),
        door: item.door.as_ref().map(|door| DoorLock {
            locked: door.locked,
            key: (door.item != u16::MAX && door.item != 0).then_some(door.item),
        }),
        // Prop-backed triggers are used by clicking their prop.
        place: item
            .placement
            .filter(|_| item.flags & flag::ITEM == 0)
            .map(|placement| retail_position(placement.position)),
        arrival: matches!(
            item.initial_action,
            trigger_action::TELEPORT | trigger_action::RANDOM_TELEPORT
        )
        .then(|| teleport_arrival(sections, item))
        .flatten(),
    }
}

/// Unweld one material's polygons from a compiled mesh into a Bevy mesh,
/// with retail's baked lighting: static vertex light, the environment
/// offset and the material's emission, plus sky light scaled by the
/// material's ambient reflectance and the vertex's sun exposure.
fn build_part(
    source: &CompiledMesh,
    material: u32,
    emission: f32,
    reflectance: Vec3,
    offset: Vec3,
    sky: Vec3,
) -> Option<(Mesh, VertexLighting)> {
    let mut corners: HashMap<(u32, u32), u32> = HashMap::new();
    let (mut positions, mut normals, mut uvs) = (vec![], vec![], vec![]);
    let mut lighting = VertexLighting { reflectance, ..default() };
    let mut indices = vec![];
    for polygon in source.polygons.iter().filter(|p| p.material == material) {
        // Mirroring Z also mirrors winding; emit corners reversed so retail
        // front faces stay front faces.
        for corner in [0, 2, 1] {
            let (vertex, uv) = (polygon.vertices[corner], polygon.uvs[corner]);
            let index = *corners.entry((vertex, uv)).or_insert_with(|| {
                let v = vertex as usize;
                positions.push(retail_position(source.positions[v]).to_array());
                normals.push(retail_direction(source.normals[v]).to_array());
                uvs.push(source.uvs[uv as usize]);
                lighting.sun.push(source.sunlight.as_ref().map_or(1.0, |sun| sun[v]));
                lighting.fixed.push(Vec3::from(source.vertex_light[v]) + offset + emission);
                positions.len() as u32 - 1
            });
            indices.push(index);
        }
    }
    if indices.is_empty() {
        return None;
    }
    // Kept in the main world too, so the colours can be redone.
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, lighting.colours(sky));
    mesh.insert_indices(Indices::U32(indices));
    Some((mesh, lighting))
}

/// Build placed props: their part meshes and materials, lit for their rest
/// pose, and what using them does.
pub(super) async fn build_props(
    context: &mut LoadContext<'_>,
    source: impl Iterator<Item = &Prop>,
    directory: &str,
    textures: &mut LoadedTextures,
    lighting: &PropLighting<'_>,
    animated_textures: &mut Vec<TextureAnimation>,
) -> Vec<PropModel> {
    let mut props = Vec::new();
    for (prop_index, prop) in source.enumerate() {
        let mut parts = Vec::new();
        for (part_index, (part, (transform, keys))) in
            prop.detailed_parts().iter().zip(part_transforms(prop)).enumerate()
        {
            let record = &part.mesh.frames[part.mesh.first_frame];
            let mut part_materials = Vec::with_capacity(record.materials.len());
            for (index, material) in record.materials.iter().enumerate() {
                let label = format!("prop/{prop_index}/{part_index}/material/{index}");
                part_materials.push(
                    load_material(context, label, directory, textures, index, material).await,
                );
            }
            let mut meshes = Vec::new();
            for (material, mesh, lighting) in
                record_meshes(record, &transform, lighting, |m| part_materials[m].emission)
            {
                let label = format!("prop/{prop_index}/{part_index}/mesh/{material}");
                let mesh = context.add_labeled_asset(label, mesh);
                meshes.push(LevelPart {
                    mesh,
                    material: part_materials[material].handle.clone(),
                    lighting,
                });
            }
            animated_textures.extend(part_materials.into_iter().filter_map(|m| m.animation));
            parts.push(PropModelPart {
                transform,
                keys,
                meshes,
                collision: record_collision(record),
            });
        }
        let trigger = prop.trigger.as_ref().and_then(|trigger| match &trigger.kind {
            TriggerKind::Item(_) => Some(trigger.name.clone()),
            _ => None,
        });
        props.push(PropModel {
            name: prop.name.clone(),
            collidable: prop.flags & PROP_COLLIDABLE != 0,
            face_camera: prop.face_camera,
            frames_per_second: prop.frames_per_second,
            parts,
            wraps: prop.wraps,
            slots: prop.slots.iter().map(|slot| slot.frame).collect(),
            trigger,
        });
    }
    props
}

impl AssetLoader for PvlLoader {
    type Asset = LevelGeometry;
    type Settings = ();
    type Error = PvlLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        context: &mut LoadContext<'_>,
    ) -> Result<LevelGeometry, PvlLoadError> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).await?;
        let geometry = read_level_geometry(&data)?;
        let sections = read_sections(&mut Bytes::new(&data[geometry.end..]))?;
        let level_path = context.path().path().to_string_lossy();
        let level_index =
            crate::content::levels::LEVELS.iter().position(|level| level.path() == level_path);
        let filter_path = AssetPath::from("retail://data/automap/mapfilters.txt");
        let filters = match context.read_asset_bytes(filter_path).await {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(bevy::asset::ReadAssetBytesError::AssetReaderError(
                bevy::asset::io::AssetReaderError::NotFound(_),
            )) => String::new(),
            Err(error) => return Err(error.into()),
        };
        let map_layers =
            super::automap::layer_heights(&sections.clip_planes, &filters, level_index)
                .map_err(PvlLoadError::MapLayers)?;
        let directory = "bitmaps";
        let environment = sections.environment.as_ref().map(level_environment);
        let ambient = environment.as_ref().map_or(Vec3::ZERO, |e| e.sky_light(NOON));
        let offset = Vec3::from(sections.environment_offset);

        let mut textures = LoadedTextures::default();
        let mut materials = Vec::with_capacity(geometry.materials.len());
        for (index, record) in geometry.materials.iter().enumerate() {
            let label = format!("material/{index}");
            materials
                .push(load_material(context, label, directory, &mut textures, index, record).await);
        }

        let mut parts = Vec::new();
        for (mesh_index, mesh) in geometry.meshes.iter().enumerate() {
            let mut used: Vec<u32> = mesh.polygons.iter().map(|p| p.material).collect();
            used.sort_unstable();
            used.dedup();
            for material in used {
                let loaded = &materials[material as usize];
                if let Some((part, lighting)) =
                    build_part(mesh, material, loaded.emission, loaded.ambient, offset, ambient)
                {
                    let handle =
                        context.add_labeled_asset(format!("mesh/{mesh_index}/{material}"), part);
                    parts.push(LevelPart {
                        mesh: handle,
                        material: loaded.handle.clone(),
                        lighting,
                    });
                }
            }
        }
        // Collision comes from the level's octree file, not render polygons.
        let stem = context.path().path().file_stem().unwrap_or_default().to_string_lossy();
        let octree = sibling(context, ".", &format!("{stem}.oct")).ok_or(PvlLoadError::Path)?;
        let octree = context.read_asset_bytes(octree).await?;
        let collision = read_collision(&octree)?.solid_mesh();

        let level_lighting = level_lighting(&sections.lights, sections.environment_offset);
        let lighting = PropLighting { ambient, level: &level_lighting };
        let mut animated_textures: Vec<TextureAnimation> =
            materials.into_iter().filter_map(|m| m.animation).collect();
        let props = sections.props.iter().chain(&sections.bitmaps);
        let props = build_props(
            context,
            props,
            directory,
            &mut textures,
            &lighting,
            &mut animated_textures,
        )
        .await;
        let links = level_links(&sections);
        Ok(LevelGeometry {
            parts,
            collision,
            map_layers,
            environment,
            lighting: level_lighting,
            entrances: level_entrances(&sections),
            triggers: level_triggers(&sections),
            props,
            links,
            animated_textures,
        })
    }

    fn extensions(&self) -> &[&str] {
        &["pvl"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_lists_end_at_the_first_short_line() {
        let list = "Candle-0000.tga\r\nCandle-0001.tga\r\n\r\nignored.tga\r\n";
        assert_eq!(ifl_frames(list), ["candle-0000.tga", "candle-0001.tga"]);
    }
}
