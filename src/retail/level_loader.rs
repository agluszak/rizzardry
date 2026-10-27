//! Independent typed loaders for a retail level's gameplay and presentation.

use std::collections::HashMap;

use bevy::asset::{AssetLoader, AssetPath, LoadContext, RenderAssetUsages, io::Reader};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::reflect::TypePath;

use super::links::{level_entrances, level_links, teleport_arrival};
use super::oct::read_collision;
use super::population::{Placement, WorldItemRecord, read_placements, read_world_items};
use super::prop::{PROP_COLLIDABLE, Prop};
use super::prop_loader::{
    PropLighting, key_transform, level_lighting, part_transforms, record_collision, record_meshes,
};
use super::pvl::{CompiledMesh, MaterialRecord, PvlError, read_level_geometry};
use super::sections::LevelSections;
use super::sections::{Environment, read_sections};
use super::space::{retail_direction, retail_position, retail_yaw};
use super::texture_loader::file_has_alpha;
use super::trigger::{AmbientSound, ItemTrigger, StateMode, TriggerKind, action, flag};
use crate::bytes::Bytes;
use crate::content::UNITS_PER_METRE;
use crate::content::items::Dice;
use crate::content::items::ItemId;
use crate::content::level::{
    AmbientSoundModel, Animation, CameraPathModel, DoorLock, Fountain, ItemPlacement,
    LevelDefinition, LevelEnvironment, LevelGeometry, LevelPart, MonsterPlacement, Playback,
    PropModel, PropModelPart, PropVisual, PropVisualPart, Restores, StateChange, TextureAnimation,
    TriggerAction, TriggerDefinition, TriggerSound, TriggerUse, VertexLighting,
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

/// An authored `[low, high]` pair as a range, whichever way round it was
/// written.
fn ordered(a: f32, b: f32) -> std::ops::RangeInclusive<f32> {
    a.min(b)..=a.max(b)
}

/// A level's ambient sound, with Whizzardry `AddAmbientSound`'s defaults:
/// unset volumes are full (127 of 127), unset times 5 to 20 seconds.
fn ambient_sound(sound: &AmbientSound) -> AmbientSoundModel {
    let or = |value: i32, default: i32| if value == -1 { default } else { value };
    let volume = |v: i32| or(v, 127).clamp(0, 127) as f32 / 127.0;
    let seconds = |ms: i32| ms.max(0) as f32 / 1000.0;
    AmbientSoundModel {
        path: format!("data/sound/{}", sound.wave.replace('\\', "/")).to_lowercase(),
        position: retail_position(sound.position),
        radius: sound.radius / UNITS_PER_METRE,
        looping: sound.looping,
        volume: ordered(volume(sound.volume[0]), volume(sound.volume[1])),
        interval: ordered(
            seconds(or(sound.interval[0], 5000)),
            seconds(or(sound.interval[1], 20000)),
        ),
    }
}

/// Placed monsters with their models, in Bevy space. Unborn ones wait for a
/// script that is not implemented yet, so they are left out.
fn monster_placements(placements: &[Placement]) -> Vec<MonsterPlacement> {
    placements
        .iter()
        .filter(|placement| !placement.unborn)
        .map(|placement| MonsterPlacement {
            location: placement.location,
            group: placement.group,
            species: placement.species,
            position: retail_position(placement.position),
            yaw: retail_yaw(placement.yaw, [0.0, 1.0, 0.0]),
        })
        .collect()
}

fn item_placement(record: &WorldItemRecord) -> ItemPlacement {
    ItemPlacement {
        item: ItemId(record.item as u16),
        stack: record.stack,
        uses: record.uses,
        identified: record.identified,
        position: retail_position(record.position),
        hidden: record.hidden,
    }
}

/// Each embedded trigger has its own identity, even when names repeat.
/// Only named recipients resolve to the last registered trigger of that name.
fn level_triggers(sections: &LevelSections) -> (Vec<TriggerDefinition>, Vec<Option<usize>>) {
    let items: Vec<(&str, &ItemTrigger)> = sections
        .all_triggers()
        .filter_map(|trigger| match &trigger.kind {
            TriggerKind::Item(item) => Some((trigger.name.as_str(), item)),
            _ => None,
        })
        .collect();
    let index: HashMap<String, usize> =
        items.iter().enumerate().map(|(i, (name, _))| (name.to_uppercase(), i)).collect();
    let definitions =
        items.iter().map(|(name, item)| trigger_definition(sections, name, item, &index)).collect();
    let mut next = 0;
    let props = sections
        .props
        .iter()
        .chain(&sections.bitmaps)
        .map(|prop| {
            prop.trigger.as_ref().filter(|t| matches!(t.kind, TriggerKind::Item(_))).map(|_| {
                let index = next;
                next += 1;
                index
            })
        })
        .collect();
    (definitions, props)
}

fn trigger_definition(
    sections: &LevelSections,
    name: &str,
    item: &ItemTrigger,
    index: &HashMap<String, usize>,
) -> TriggerDefinition {
    let sound = (!item.sound.is_empty()).then(|| TriggerSound {
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
    let recipients: Vec<&str> =
        item.recipients.split(',').map(str::trim).filter(|r| !r.is_empty()).collect();
    // The message a commit shows is the data word of the use that ran.
    let shows_message = item.flags & flag::SHOW_MESSAGE != 0;
    let using = |code: u32, slot: usize| TriggerUse {
        action: decode_action(code, sections, item, recipients.first().copied()),
        message: (shows_message && item.data[slot] >= 0).then_some(item.data[slot]),
    };
    TriggerDefinition {
        name: name.to_owned(),
        usable: item.enabled,
        once: item.flags & flag::ONCE != 0,
        fire_linked: item.fire_linked,
        initial: using(item.initial_action, 0),
        alternate: (item.alternate_action != 0).then(|| using(item.alternate_action, 1)),
        fallback: (item.fallback_action != 0).then(|| using(item.fallback_action, 2)),
        alternate_toggles: item.alternate_toggles,
        recipients: recipients
            .iter()
            .filter_map(|name| index.get(&name.to_uppercase()).copied())
            .collect(),
        sound,
        sound_on_commit: item.play_sound,
        required_states: item.required_states.iter().map(|name| name.as_str().into()).collect(),
        state_change: state_change.map(|change| (item.state_to_modify.as_str().into(), change)),
        initial_state: item.initial_state,
        range: (item.range[0] / UNITS_PER_METRE, item.range[1] / UNITS_PER_METRE),
        door: item.door.as_ref().map(|door| DoorLock {
            locked: door.locked,
            key: (door.item != u16::MAX && door.item != 0).then_some(ItemId(door.item)),
        }),
        // Prop-backed triggers are used by clicking their prop.
        place: item
            .placement
            .filter(|_| item.flags & flag::ITEM == 0)
            .map(|placement| retail_position(placement.position)),
        loot_table: (item.initial_action == action::TOGGLE && !item.action_data.is_empty())
            .then(|| item.action_data.clone()),
    }
}

/// A retail action code with its parameters (`data`, the first recipient,
/// the teleport destination).
fn decode_action(
    code: u32,
    sections: &LevelSections,
    item: &ItemTrigger,
    first_recipient: Option<&str>,
) -> TriggerAction {
    let [first, second, third] = item.data;
    // -1 stands for retail's default.
    let or = |value: i32, default: i32| if value < 0 { default } else { value };
    match code {
        0x00 => TriggerAction::Nothing,
        0x01 => TriggerAction::OpenDoor,
        0x02 | 0x2c => TriggerAction::ToggleDoor,
        0x03 => TriggerAction::Animate(Animation::Toggle),
        0x32 => TriggerAction::Animate(Animation::Start),
        0x33 => TriggerAction::Animate(Animation::Stop),
        0x09 => first_recipient
            .and_then(|name| {
                sections.cameras.iter().position(|camera| camera.name.eq_ignore_ascii_case(name))
            })
            .map_or(TriggerAction::Unsupported(code), TriggerAction::CameraPath),
        0x0f => TriggerAction::Relay,
        action::TELEPORT | action::RANDOM_TELEPORT => teleport_arrival(sections, item)
            .map_or(TriggerAction::Unsupported(code), TriggerAction::Teleport),
        0x24 => TriggerAction::HurtParty(Dice {
            count: or(first, 1) as u8,
            sides: or(second, 6) as u8,
            base: or(third, 2) as i16,
        }),
        0x23 => TriggerAction::DamageZone,
        0x25 | 0x26 => TriggerAction::Fountain(Fountain {
            restores: if code == 0x26 { Restores::Health } else { Restores::Stamina },
            amount: third.max(0) as u32,
            uses: (first != -1).then_some(first.max(0) as u32),
            // Its second word counts retail's 720-second game hours.
            refills_after: (second > 0).then(|| second as u64 * 720 * 1000),
        }),
        0x39 => TriggerAction::Shake {
            seconds: if third == -1 { 1.0 } else { third.unsigned_abs() as f32 / 1000.0 },
            intensity: if first == -1 { 800.0 } else { first as f32 },
            fade_out: third < -1,
        },
        0x40 => TriggerAction::Dial,
        0x36 => TriggerAction::Sound,
        other => TriggerAction::Unsupported(other),
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
/// pose, with the identity of their own embedded trigger.
pub(super) async fn build_props(
    context: &mut LoadContext<'_>,
    source: impl Iterator<Item = &Prop>,
    directory: &str,
    textures: &mut LoadedTextures,
    lighting: &PropLighting<'_>,
    animated_textures: &mut Vec<TextureAnimation>,
) -> Vec<PropVisual> {
    let mut props = Vec::new();
    for (prop_index, prop) in source.enumerate() {
        let mut parts = Vec::new();
        for (part_index, (part, (transform, _))) in
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
            parts.push(PropVisualPart { transform, meshes });
        }
        props.push(PropVisual { name: prop.name.clone(), face_camera: prop.face_camera, parts });
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
        Ok(LevelGeometry {
            parts,
            environment,
            lighting: level_lighting,
            ambient_sounds: sections
                .all_triggers()
                .filter_map(|trigger| match &trigger.kind {
                    TriggerKind::AmbientSound(sound) => Some(ambient_sound(sound)),
                    _ => None,
                })
                .collect(),
            props,
            animated_textures,
        })
    }

    fn extensions(&self) -> &[&str] {
        &["pvl"]
    }
}

/// Gameplay loading is independent of textures, meshes, materials and skies.
#[derive(Default, TypePath)]
pub struct PvlDefinitionLoader;

impl AssetLoader for PvlDefinitionLoader {
    type Asset = LevelDefinition;
    type Settings = ();
    type Error = PvlLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        context: &mut LoadContext<'_>,
    ) -> Result<LevelDefinition, PvlLoadError> {
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
        // Collision comes from the level's octree file, not render polygons.
        let stem =
            context.path().path().file_stem().unwrap_or_default().to_string_lossy().into_owned();
        let octree = sibling(context, ".", &format!("{stem}.oct")).ok_or(PvlLoadError::Path)?;
        let octree = context.read_asset_bytes(octree).await?;
        let collision = read_collision(&octree)?.solid_mesh();

        // Who stands where and what lies about; levels without a state file
        // are empty.
        let (monsters, items) = match sibling(context, ".", &format!("{stem}.sts")) {
            Some(path) => match context.read_asset_bytes(path).await {
                Ok(state) => {
                    let items = read_world_items(&state)?.iter().map(item_placement).collect();
                    (monster_placements(&read_placements(&state)?), items)
                }
                Err(_) => (vec![], vec![]),
            },
            None => (vec![], vec![]),
        };

        let (triggers, prop_triggers) = level_triggers(&sections);
        let props = sections
            .props
            .iter()
            .chain(&sections.bitmaps)
            .zip(prop_triggers)
            .map(|(prop, trigger)| PropModel {
                name: prop.name.clone(),
                collidable: prop.flags & PROP_COLLIDABLE != 0,
                face_camera: prop.face_camera,
                frames_per_second: prop.frames_per_second,
                parts: prop
                    .detailed_parts()
                    .iter()
                    .zip(part_transforms(prop))
                    .map(|(part, (transform, keys))| PropModelPart {
                        transform,
                        keys,
                        collision: record_collision(&part.mesh.frames[part.mesh.first_frame]),
                    })
                    .collect(),
                wraps: prop.wraps,
                slots: prop.slots.iter().map(|slot| slot.frame).collect(),
                trigger,
            })
            .collect();
        Ok(LevelDefinition {
            collision,
            map_layers,
            sight_distance: sections
                .environment
                .as_ref()
                .map(|e| e.view_distance / UNITS_PER_METRE),
            entrances: level_entrances(&sections),
            triggers,
            monsters,
            items,
            camera_paths: sections
                .cameras
                .iter()
                .map(|camera| CameraPathModel {
                    name: camera.name.to_uppercase(),
                    keys: camera.path.iter().map(key_transform).collect(),
                    keys_per_second: camera.keys_per_second,
                })
                .collect(),
            props,
            links: level_links(&sections),
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
    fn prop_bindings_use_records_while_named_links_use_registration_order() {
        use crate::retail::trigger::Trigger;
        let item = ItemTrigger {
            flags: 0,
            enabled: true,
            fire_linked: false,
            alternate_toggles: false,
            initial_action: 1,
            alternate_action: 0,
            fallback_action: 0,
            recipients: "SAME".into(),
            data: [-1; 3],
            sound: String::new(),
            play_sound: false,
            sound_volume: 100,
            required_states: vec![],
            state_to_modify: String::new(),
            state_mode: StateMode::Keep,
            initial_state: None,
            action_data: String::new(),
            range: [0.0, 5000.0],
            placement: None,
            door: None,
        };
        let trigger = Trigger { name: "SAME".into(), kind: TriggerKind::Item(item) };
        let prop = Prop {
            name: "prop".into(),
            flags: 0,
            face_camera: false,
            wraps: false,
            slots: vec![],
            location: [0.0; 3],
            frames_per_second: 10.0,
            channels: Default::default(),
            path: None,
            trigger: Some(trigger.clone()),
        };
        let mut sections = LevelSections {
            lights: vec![],
            props: vec![prop.clone(), prop.clone()],
            bitmaps: vec![prop],
            cameras: vec![],
            environment: None,
            triggers: vec![],
            camera_sway: false,
            clip_planes: vec![],
            environment_offset: [0.0; 3],
            particles: vec![],
            named_positions: vec![],
            automap_cell_size: 0.0,
            automap_cells: vec![],
        };
        let (definitions, props) = level_triggers(&sections);
        assert_eq!(props, [Some(0), Some(1), Some(2)]);
        assert!(definitions.iter().all(|d| d.recipients == [2]));
        let mut standalone = trigger;
        if let TriggerKind::Item(item) = &mut standalone.kind {
            item.initial_action = 2;
        }
        sections.triggers.push(standalone);
        let (definitions, props) = level_triggers(&sections);
        assert_eq!(props, [Some(0), Some(1), Some(2)]);
        assert_eq!(definitions.len(), 4);
        assert!(definitions.iter().all(|d| d.recipients == [3]));
        assert!(matches!(definitions[0].initial.action, TriggerAction::OpenDoor));
        assert!(matches!(definitions[3].initial.action, TriggerAction::ToggleDoor));
        assert!(
            matches!(&sections.trigger("same").unwrap().kind, TriggerKind::Item(item) if item.initial_action == 2)
        );
    }

    #[test]
    fn frame_lists_end_at_the_first_short_line() {
        let list = "Candle-0000.tga\r\nCandle-0001.tga\r\n\r\nignored.tga\r\n";
        assert_eq!(ifl_frames(list), ["candle-0000.tga", "candle-0001.tga"]);
    }

    /// Every retail container names an existing loot table but two, which
    /// generate nothing (the camp's chest holds what the party leaves in it).
    #[test]
    #[ignore = "requires a retail installation"]
    fn retail_containers_name_existing_loot_tables_but_two() {
        let files = crate::retail::files::retail_files();
        let tables = crate::retail::items::read_loot_tables(
            &files.read("data/databases/itemtables.dbs").unwrap().unwrap(),
        )
        .unwrap();
        let levels: Vec<&str> = files.names().filter(|n| n.ends_with(".pvl")).collect();
        let (mut containers, mut missing) = (0, vec![]);
        for level in &levels {
            let data = files.read(level).unwrap().unwrap();
            let geometry = read_level_geometry(&data).unwrap();
            let sections = read_sections(&mut Bytes::new(&data[geometry.end..])).unwrap();
            for trigger in level_triggers(&sections).0 {
                let Some(name) = &trigger.loot_table else { continue };
                containers += 1;
                if tables.find(name).is_none() {
                    missing.push(format!("{level}: {name}"));
                }
            }
        }
        assert!(levels.len() > 20 && containers > 0);
        missing.sort();
        assert_eq!(
            missing,
            [
                "levels/camp/camp.pvl: AWAYCAMPCHEST",
                "levels/rapax/rapaxupperfloor.pvl: CONSTABLE CHEST"
            ]
        );
    }
}
