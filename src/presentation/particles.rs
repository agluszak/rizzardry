//! Particle billboards observe game-owned scene simulation and birth history.
use super::{billboards::FaceCamera, environment::LevelAppearance};
use crate::{
    content::{
        level::{LevelDefinition, LevelGeometry},
        particles::*,
    },
    game::world::{
        level::{Level, LevelOwned},
        particles::{Emitter, Particle},
    },
};
use bevy::prelude::*;
#[derive(Component)]
struct ParticleSprite;
#[derive(Resource)]
struct ParticleMesh(Handle<Mesh>);
pub(super) fn plugin(app: &mut App) {
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(Rectangle::new(1.0, 1.0));
    app.insert_resource(ParticleMesh(mesh))
        .add_systems(Update, (attach_sprites, animate_particles).chain());
}
fn attach_sprites(
    levels: Query<&LevelAppearance>,
    geometry: Res<Assets<LevelGeometry>>,
    emitters: Query<(&LevelOwned, &Emitter)>,
    particles: Query<(Entity, &ChildOf, Option<&Children>), With<Particle>>,
    sprites: Query<(), With<ParticleSprite>>,
    mesh: Res<ParticleMesh>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    for (entity, parent, children) in &particles {
        if children.is_some_and(|children| children.iter().any(|child| sprites.contains(child))) {
            continue;
        }
        let Ok((owner, emitter)) = emitters.get(parent.parent()) else { continue };
        let Some(surface) = levels
            .get(owner.0)
            .ok()
            .and_then(|level| geometry.get(&level.0))
            .and_then(|level| level.particles.get(emitter.definition()))
        else {
            continue;
        };
        let Some(mut material) = materials.get(&surface.material).cloned() else { continue };
        material.cull_mode = None;
        if material.alpha_mode == AlphaMode::Opaque {
            material.alpha_mode = AlphaMode::Blend;
        }
        let material = materials.add(material);
        commands.spawn((
            ParticleSprite,
            FaceCamera,
            Mesh3d(mesh.0.clone()),
            MeshMaterial3d(material),
            Transform::default(),
            ChildOf(entity),
        ));
    }
}
fn animate_particles(
    time: Res<Time<Virtual>>,
    geometry: Res<Assets<LevelGeometry>>,
    levels: Query<(&Level, &LevelAppearance)>,
    definitions: Res<Assets<LevelDefinition>>,
    emitters: Query<(&LevelOwned, &Emitter)>,
    particles: Query<(&ChildOf, &Particle)>,
    mut sprites: Query<
        (&ChildOf, &MeshMaterial3d<StandardMaterial>, &mut Transform),
        With<ParticleSprite>,
    >,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (parent, material, mut transform) in &mut sprites {
        let Ok((emitter, particle)) = particles.get(parent.parent()) else { continue };
        let Ok((parent, emitter)) = emitters.get(emitter.parent()) else { continue };
        let Some(visual) = levels
            .get(parent.0)
            .ok()
            .and_then(|(_, appearance)| geometry.get(&appearance.0))
            .and_then(|level| level.particles.get(emitter.definition()))
        else {
            continue;
        };
        let Some(config) = levels
            .get(parent.0)
            .ok()
            .and_then(|(level, _)| definitions.get(&level.definition))
            .and_then(|level| level.particles.get(emitter.definition()))
        else {
            continue;
        };
        let age = time.elapsed().saturating_sub(particle.born);
        let fade = if age.is_zero() {
            1.0
        } else {
            match config.lifetime {
                ParticleLifetime::Timed(life) => {
                    (life.saturating_sub(age).as_secs_f32() / 0.5).min(1.0)
                }
                _ => 1.0,
            }
        };
        let frame = visual.animation.frame_at(age.as_secs_f32());
        let texture = visual.animation.frames.get(frame);
        let alpha =
            materials.get(&visual.material).map_or(1.0, |base| base.base_color.alpha()) * fade;
        if let Some(current) = materials.get(&material.0)
            && (current.base_color.alpha() != alpha
                || texture.is_some_and(|t| current.base_color_texture.as_ref() != Some(t)))
            && let Some(mut current) = materials.get_mut(&material.0)
        {
            current.base_color.set_alpha(alpha);
            if let Some(texture) = texture {
                current.base_color_texture = Some(texture.clone());
            }
        }
        let mut flutter = Vec3::ZERO;
        if let Some((amplitude, period)) = config.flutter
            && !period.is_zero()
            && particle.velocity.y < 0.0
        {
            let phase = (time.elapsed().as_secs_f32() % period.as_secs_f32())
                / period.as_secs_f32()
                * std::f32::consts::TAU;
            let amount = particle.velocity.y.max(-2.0) * -0.001 * amplitude * phase.sin();
            flutter = Quat::from_rotation_y(particle.flutter_angle) * Vec3::new(amount, 0.0, 0.0);
        }
        let scale = Vec3::splat(config.size);
        if transform.translation != flutter || transform.scale != scale {
            transform.translation = flutter;
            transform.scale = scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        content::surface::{Surface, TextureFrames},
        game::world::particles::tests::{scene, step},
    };

    #[test]
    fn rebuilding_particle_views_preserves_birth_motion_and_independent_texture_phase() {
        let mut scene = scene();
        scene.world.init_resource::<Assets<StandardMaterial>>();
        scene.world.init_resource::<Assets<LevelGeometry>>();
        scene.world.init_resource::<Assets<Image>>();
        scene.world.insert_resource(ParticleMesh(default()));
        let source = scene.world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
            base_color: Color::srgba(1.0, 1.0, 1.0, 0.5),
            alpha_mode: AlphaMode::Blend,
            ..default()
        });
        let frames: Vec<_> = (0..2)
            .map(|_| scene.world.resource_mut::<Assets<Image>>().add(Image::default()))
            .collect();
        let appearance = scene.world.resource_mut::<Assets<LevelGeometry>>().add(LevelGeometry {
            particles: vec![Surface {
                material: source.clone(),
                ambient: Vec3::ONE,
                emission: 0.0,
                animation: TextureFrames {
                    frames: frames.clone(),
                    frames_per_second: 10.0,
                    ..default()
                },
            }],
            ..default()
        });
        scene.world.entity_mut(scene.root).insert(LevelAppearance(appearance));
        assert!(scene.run("ON").0);
        step(&mut scene, 100);
        scene.world.run_system_cached(attach_sprites).unwrap();
        scene.world.flush();
        scene.world.run_system_cached(animate_particles).unwrap();
        let (sprite, parent) = scene
            .world
            .query_filtered::<(Entity, &ChildOf), With<ParticleSprite>>()
            .single(&scene.world)
            .unwrap();
        let particle = parent.parent();
        let birth = scene.world.get::<Particle>(particle).unwrap().born;
        let handle = scene.world.get::<MeshMaterial3d<StandardMaterial>>(sprite).unwrap().0.clone();
        assert_eq!(
            scene
                .world
                .resource::<Assets<StandardMaterial>>()
                .get(&handle)
                .unwrap()
                .base_color
                .alpha(),
            0.5
        );
        step(&mut scene, 100);
        scene.world.run_system_cached(animate_particles).unwrap();
        scene.world.despawn(sprite);
        scene.world.run_system_cached(attach_sprites).unwrap();
        scene.world.flush();
        scene.world.run_system_cached(animate_particles).unwrap();
        let child = scene.world.get::<Children>(particle).unwrap()[0];
        let handle = &scene.world.get::<MeshMaterial3d<StandardMaterial>>(child).unwrap().0;
        let material = scene.world.resource::<Assets<StandardMaterial>>().get(handle).unwrap();
        assert!((material.base_color.alpha() - 0.2).abs() < 1e-6);
        assert_eq!(material.base_color_texture.as_ref(), Some(&frames[1]));
        assert_eq!(scene.world.get::<Particle>(particle).unwrap().born, birth);
        assert!((scene.world.get::<Transform>(particle).unwrap().translation.y - 0.1).abs() < 1e-6);
        assert_eq!(
            scene
                .world
                .resource::<Assets<StandardMaterial>>()
                .get(&source)
                .unwrap()
                .base_color
                .alpha(),
            0.5
        );
        scene.visit(None);
        assert_eq!(
            scene.world.query_filtered::<Entity, With<ParticleSprite>>().iter(&scene.world).count(),
            0
        );
    }
}
