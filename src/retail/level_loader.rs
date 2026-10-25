//! Loads a retail `.pvl` level's static geometry as [`LevelGeometry`].

use std::collections::HashMap;

use bevy::asset::{AssetLoader, AssetPath, LoadContext, RenderAssetUsages, io::Reader};
use bevy::image::{
    CompressedImageFormats, ImageAddressMode, ImageSampler, ImageSamplerDescriptor, ImageType,
};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::reflect::TypePath;

use super::pvl::{self, CompiledMesh, MaterialRecord, PvlError, read_level_geometry};
use super::space::{retail_direction, retail_position};
use crate::content::level::{LevelGeometry, LevelPart};

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

async fn load_texture(
    context: &mut LoadContext<'_>,
    directory: &str,
    name: &str,
) -> Option<(Handle<Image>, bool)> {
    let mut file = name.to_lowercase();
    if file.ends_with(".ifl") {
        // Animated texture list: one frame name per line. Playback is not
        // implemented yet; show the first frame.
        let list = context.read_asset_bytes(sibling(context, directory, &file)?).await.ok()?;
        let list = String::from_utf8_lossy(&list);
        file = list.lines().map(str::trim).find(|line| !line.is_empty())?.to_lowercase();
    }
    let label = format!("texture/{file}");
    let path = sibling(context, directory, &file)?;
    let bytes = match context.read_asset_bytes(path.clone()).await {
        Ok(bytes) => bytes,
        Err(error) => {
            warn!("{}: missing texture {path}: {error}", context.path());
            return None;
        }
    };
    let extension = file.rsplit('.').next().unwrap_or_default().to_owned();
    let sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    });
    let image = match Image::from_buffer(
        &bytes,
        ImageType::Extension(&extension),
        CompressedImageFormats::NONE,
        true,
        sampler,
        RenderAssetUsages::RENDER_WORLD,
    ) {
        Ok(image) => image,
        Err(error) => {
            warn!("{}: cannot decode texture {path}: {error}", context.path());
            return None;
        }
    };
    let alpha = has_alpha(&image);
    Some((context.add_labeled_asset(label, image), alpha))
}

async fn load_material(
    context: &mut LoadContext<'_>,
    directory: &str,
    textures: &mut HashMap<String, Option<(Handle<Image>, bool)>>,
    index: usize,
    record: &MaterialRecord,
) -> LoadedMaterial {
    let selected = if index == 0 { None } else { record.texture() };
    let mut texture = None;
    let mut alpha = false;
    let mut slot = 0;
    if let Some((texture_slot, name)) = selected {
        slot = texture_slot;
        // Retail: a texture found only in the fourth slot carries alpha.
        alpha = slot == 3;
        let key = name.to_lowercase();
        if !textures.contains_key(&key) {
            let loaded = load_texture(context, directory, name).await;
            textures.insert(key.clone(), loaded);
        }
        if let Some((handle, texture_alpha)) = textures[&key].clone() {
            texture = Some(handle);
            alpha |= texture_alpha;
        }
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
        base_color_texture: texture,
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
    LoadedMaterial { handle, emission: record.emission }
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

        let mut textures = HashMap::new();
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
        Ok(LevelGeometry { parts, collision_vertices, collision_triangles, bounds: (min, max) })
    }

    fn extensions(&self) -> &[&str] {
        &["pvl"]
    }
}
