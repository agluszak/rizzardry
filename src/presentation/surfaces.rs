//! Asset surfaces become rendered materials; actor light is a uniform, not geometry.
use crate::content::surface::Surface;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

impl Surface {
    pub fn render(&self) -> impl Bundle {
        (MeshMaterial3d(self.material.clone()), self.animation.clone())
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
#[derive(Component, Default)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::surface::{Playback, TextureFrames};
    use crate::game::time::GameClock;

    #[test]
    fn light_and_animation_target_instance_materials_without_changing_opacity_or_source() {
        let mut app = App::new();
        app.init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<Image>>()
            .init_resource::<Time<Virtual>>()
            .init_resource::<GameClock>()
            .add_systems(Update, (super::super::textures::animate_textures, tint_surfaces));
        let frames: Vec<_> = (0..2)
            .map(|_| app.world_mut().resource_mut::<Assets<Image>>().add(Image::default()))
            .collect();
        let original =
            app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
                base_color: Color::linear_rgba(1., 1., 1., 0.4),
                base_color_texture: Some(frames[0].clone()),
                unlit: true,
                ..default()
            });
        let surface = Surface {
            material: original.clone(),
            ambient: Vec3::splat(0.5),
            emission: 0.2,
            animation: TextureFrames {
                frames: frames.clone(),
                playback: Playback::Loop,
                frames_per_second: 1.,
                start_frame: 0,
            },
        };
        let a = app.world_mut().spawn(ActorLight { sky: Vec3::ONE, fixed: Vec3::Y }).id();
        let b = app.world_mut().spawn(ActorLight { sky: Vec3::ZERO, fixed: Vec3::ZERO }).id();
        let mut instances = HashMap::new();
        let first = {
            let mut materials = app.world_mut().resource_mut::<Assets<StandardMaterial>>();
            surface.lit_instance(a, &mut instances, &mut materials).unwrap()
        };
        let a_part = app.world_mut().spawn(first).id();
        let second = {
            let mut materials = app.world_mut().resource_mut::<Assets<StandardMaterial>>();
            surface.lit_instance(a, &mut instances, &mut materials).unwrap()
        };
        let same_actor_part = app.world_mut().spawn(second).id();
        let other = {
            let mut materials = app.world_mut().resource_mut::<Assets<StandardMaterial>>();
            surface.lit_instance(b, &mut HashMap::new(), &mut materials).unwrap()
        };
        let b_part = app.world_mut().spawn(other).id();
        app.world_mut()
            .resource_mut::<Time<Virtual>>()
            .advance_by(std::time::Duration::from_secs(1));
        app.update();
        let handle = |e| &app.world().get::<MeshMaterial3d<StandardMaterial>>(e).unwrap().0;
        assert_eq!(handle(a_part), handle(same_actor_part));
        assert_ne!(handle(a_part), handle(b_part));
        let materials = app.world().resource::<Assets<StandardMaterial>>();
        let a = materials.get(handle(a_part)).unwrap();
        assert_eq!(a.base_color, Color::linear_rgba(0.7, 1.7, 0.7, 0.4));
        assert_eq!(a.base_color_texture, Some(frames[1].clone()));
        assert_eq!(
            materials.get(handle(b_part)).unwrap().base_color,
            Color::linear_rgba(0.2, 0.2, 0.2, 0.4)
        );
        assert_eq!(materials.get(&original).unwrap().base_color_texture, Some(frames[0].clone()));
    }
}
