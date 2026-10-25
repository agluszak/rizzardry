//! Loads a retail `.pvl` level's static geometry as [`LevelGeometry`].

use std::collections::HashMap;

use bevy::asset::{AssetLoader, AssetPath, LoadContext, RenderAssetUsages, io::Reader};
use bevy::image::{
    CompressedImageFormats, ImageAddressMode, ImageFilterMode, ImageSampler,
    ImageSamplerDescriptor, ImageType,
};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::reflect::TypePath;

use super::pvl::{self, CompiledMesh, MaterialRecord, PvlError, read_level_geometry};
use super::space::{retail_direction, retail_position};
use crate::content::level::{LevelGeometry, LevelPart, Playback, TextureAnimation};
use crate::presentation::mipmaps::generate_mipmaps;

/// Light contributed by full sun exposure until the level environment's sun
/// colour is decoded.
const PROVISIONAL_SUN: f32 = 0.75;

#[derive(Default, TypePath)]
pub struct PvlLoader;

#[derive(Debug, thiserror::Error)]
pub enum PvlLoadError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Pvl(#[from] PvlError),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Blend {
    Opaque,
    Mask,
    Translucent,
}

struct LoadedMaterial {
    handle: Handle<StandardMaterial>,
    emission: f32,
    animation: Option<TextureAnimation>,
}

/// Whether any texel is not fully opaque. Retail treats such textures as
/// alpha-carrying whatever the file's channel layout.
fn has_alpha(image: &Image) -> bool {
    use bevy::render::render_resource::TextureFormat;
    let Some(data) = &image.data else { return false };
    match image.texture_descriptor.format {
        TextureFormat::Rgba8UnormSrgb | TextureFormat::Rgba8Unorm => {
            data.as_chunks::<4>().0.iter().any(|texel| texel[3] < 255)
        }
        _ => false,
    }
}

/// A path in `directory` next to the asset being loaded, in the same source.
fn sibling(context: &LoadContext, directory: &str, file: &str) -> Option<AssetPath<'static>> {
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

/// Images already decoded for this level, by file name, with whether they
/// carry alpha; `None` records a texture that failed to load.
#[derive(Default)]
struct LoadedTextures(HashMap<String, Option<(Handle<Image>, bool)>>);

/// One image, decoded with a repeat sampler and a full mip chain.
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
            // Retail: linear filtering, nearest mip selection, repeat.
            let sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: ImageAddressMode::Repeat,
                address_mode_v: ImageAddressMode::Repeat,
                mipmap_filter: ImageFilterMode::Nearest,
                ..ImageSamplerDescriptor::linear()
            });
            match Image::from_buffer(
                &bytes,
                ImageType::Extension(extension),
                CompressedImageFormats::NONE,
                true,
                sampler,
                RenderAssetUsages::RENDER_WORLD,
            ) {
                Ok(mut image) => {
                    let alpha = has_alpha(&image);
                    generate_mipmaps(&mut image);
                    Some((context.add_labeled_asset(format!("texture/{file}"), image), alpha))
                }
                Err(error) => {
                    warn!("{}: cannot decode texture {path}: {error}", context.path());
                    None
                }
            }
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

async fn load_material(
    context: &mut LoadContext<'_>,
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
    let blend = if record.opacity < 1.0 || alpha {
        // Slots 0-1 blend; slots 2-3 select the alpha-tested shader.
        if slot <= 1 { Blend::Translucent } else { Blend::Mask }
    } else {
        Blend::Opaque
    };
    let opacity = if index == 0 { 1.0 } else { record.opacity.clamp(0.0, 1.0) };
    let material = StandardMaterial {
        base_color: Color::srgba(
            1.0,
            1.0,
            1.0,
            if blend == Blend::Translucent { opacity.max(0.05) } else { 1.0 },
        ),
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
            Blend::Mask => AlphaMode::Mask(0.5),
            Blend::Translucent => AlphaMode::Blend,
        },
        ..default()
    };
    let handle = context.add_labeled_asset(format!("material/{index}"), material);
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
    LoadedMaterial { handle, emission: record.emission, animation }
}

/// Solid level triangles in Bevy space: every polygon whose material is not
/// the default and is fully opaque (alpha-tested foliage included; water and
/// other translucent surfaces excluded). Winding is reversed like the render
/// meshes, so normals face out.
pub fn collision_triangles(geometry: &pvl::LevelGeometry) -> (Vec<Vec3>, Vec<[u32; 3]>) {
    let solid: Vec<bool> = geometry
        .materials
        .iter()
        .enumerate()
        .map(|(index, material)| index != 0 && material.opacity >= 1.0)
        .collect();
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for mesh in &geometry.meshes {
        let base = vertices.len() as u32;
        vertices.extend(mesh.positions.iter().map(|&p| retail_position(p)));
        for polygon in mesh.polygons.iter().filter(|p| solid[p.material as usize]) {
            let [a, b, c] = polygon.vertices.map(|v| base + v);
            triangles.push([a, c, b]);
        }
    }
    (vertices, triangles)
}

/// Unweld one material's polygons from a compiled mesh into a Bevy mesh.
fn build_part(source: &CompiledMesh, material: u32, emission: f32) -> Option<Mesh> {
    let mut corners: HashMap<(u32, u32), u32> = HashMap::new();
    let (mut positions, mut normals, mut uvs, mut colors) = (vec![], vec![], vec![], vec![]);
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
                let sun = source.sunlight.as_ref().map_or(0.0, |sun| sun[v]) * PROVISIONAL_SUN;
                let light =
                    source.vertex_light[v].map(|channel| (channel + sun + emission).min(1.0));
                colors.push([light[0], light[1], light[2], 1.0]);
                positions.len() as u32 - 1
            });
            indices.push(index);
        }
    }
    if indices.is_empty() {
        return None;
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    Some(mesh)
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
        let directory = "bitmaps";

        let mut textures = LoadedTextures::default();
        let mut materials = Vec::with_capacity(geometry.materials.len());
        for (index, record) in geometry.materials.iter().enumerate() {
            materials.push(load_material(context, directory, &mut textures, index, record).await);
        }

        let mut parts = Vec::new();
        for (mesh_index, mesh) in geometry.meshes.iter().enumerate() {
            let mut used: Vec<u32> = mesh.polygons.iter().map(|p| p.material).collect();
            used.sort_unstable();
            used.dedup();
            for material in used {
                let loaded = &materials[material as usize];
                if let Some(part) = build_part(mesh, material, loaded.emission) {
                    let handle =
                        context.add_labeled_asset(format!("mesh/{mesh_index}/{material}"), part);
                    parts.push(LevelPart { mesh: handle, material: loaded.handle.clone() });
                }
            }
        }
        let (collision_vertices, collision_triangles) = collision_triangles(&geometry);
        let (min, max) = collision_vertices
            .iter()
            .fold((Vec3::MAX, Vec3::MIN), |(min, max), &p| (min.min(p), max.max(p)));
        let animated_textures = materials.into_iter().filter_map(|m| m.animation).collect();
        Ok(LevelGeometry {
            parts,
            collision_vertices,
            collision_triangles,
            bounds: (min, max),
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
