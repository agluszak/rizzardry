//! Asset surfaces become rendered materials; actor light is a uniform, not geometry.
use crate::content::surface::Surface;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

impl Surface {
    pub fn render(&self) -> impl Bundle {
        (MeshMaterial3d(self.material.clone()), self.animation.clone())
    }

    /// Per-level render instances share their source and bake policy. The
    /// dynamic contribution is sampled at the live GPU world pose.
    pub(super) fn dynamic_instance(
        &self,
        compiled: bool,
        lights: &Handle<bevy::render::storage::ShaderBuffer>,
        instances: &mut HashMap<
            (AssetId<StandardMaterial>, bool),
            Handle<super::lighting::SceneMaterial>,
        >,
        source: &Assets<StandardMaterial>,
        materials: &mut Assets<super::lighting::SceneMaterial>,
    ) -> Option<impl Bundle> {
        let handle = match instances.get(&(self.material.id(), compiled)) {
            Some(handle) => handle.clone(),
            None => {
                let handle = materials.add(super::lighting::SceneMaterial {
                    base: source.get(&self.material)?.clone(),
                    extension: super::lighting::SceneLighting {
                        compiled: if compiled { Vec4::X } else { Vec4::ZERO },
                        lights: lights.clone(),
                    },
                });
                instances.insert((self.material.id(), compiled), handle.clone());
                handle
            }
        };
        Some((MeshMaterial3d(handle), self.animation.clone()))
    }

    /// Each independently lit visual owns its materials, sharing textures/meshes.
    /// Matching parts within that visual reuse one instance of their source.
    pub fn lit_instance(
        &self,
        source: Entity,
        instances: &mut HashMap<AssetId<StandardMaterial>, Handle<StandardMaterial>>,
        materials: &mut Assets<StandardMaterial>,
    ) -> Option<impl Bundle> {
        let material = match instances.get(&self.material.id()) {
            Some(material) => material.clone(),
            None => {
                let material = materials.add(materials.get(&self.material)?.clone());
                instances.insert(self.material.id(), material.clone());
                material
            }
        };
        Some((
            MeshMaterial3d(material),
            self.animation.clone(),
            LightFrom(source),
            SurfaceResponse { ambient: self.ambient, emission: self.emission },
        ))
    }
}

#[derive(Component)]
#[relationship(relationship_target = LitSurfaces)]
pub struct LightFrom(pub Entity);

#[derive(Component, Default)]
#[relationship_target(relationship = LightFrom)]
pub struct LitSurfaces(Vec<Entity>);

#[derive(Component)]
pub struct SurfaceResponse {
    pub ambient: Vec3,
    pub emission: f32,
}

/// Sampled at the actor by its own sunlight/hover policy.
#[derive(Component, Default, PartialEq)]
pub struct ActorLight {
    pub sky: Vec3,
    pub fixed: Vec3,
}

pub fn tint_surfaces(
    sources: Query<&ActorLight>,
    surfaces: Query<(&LightFrom, &SurfaceResponse, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (source, response, handle) in &surfaces {
        let Ok(light) = sources.get(source.0) else { continue };
        let rgb = light.sky * response.ambient + light.fixed + Vec3::splat(response.emission);
        let Some(material) = materials.get(&handle.0) else { continue };
        let tint = Color::linear_rgba(rgb.x, rgb.y, rgb.z, material.base_color.alpha());
        if material.base_color != tint {
            materials.get_mut(&handle.0).unwrap().base_color = tint;
        }
    }
}
