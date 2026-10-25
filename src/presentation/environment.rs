//! Fog, clear colour and sky light from the level environment and the time
//! of day.

use bevy::prelude::*;

use crate::content::level::LevelGeometry;
use crate::game::party::body::PartyCamera;
use crate::game::time::GameClock;
use crate::game::world::level::Level;

/// The level and phase the presentation was last updated for.
#[derive(Default)]
pub struct Shown(Option<(AssetId<LevelGeometry>, u8)>);

/// Apply the environment when the level or the time-of-day phase changes:
/// linear fog toward the phase's fog colour (which is also the clear
/// colour), the far clip at the view distance, and vertex colours relit
/// with the phase's sky light.
pub fn apply_environment(
    mut shown: Local<Shown>,
    clock: Res<GameClock>,
    levels: Query<&Level>,
    geometry: Res<Assets<LevelGeometry>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut clear: ResMut<ClearColor>,
    mut camera: Query<(Entity, &mut Projection), With<PartyCamera>>,
    mut commands: Commands,
) {
    let Some(level) = levels.iter().next() else { return };
    let Some(level_geometry) = geometry.get(&level.geometry) else { return };
    let Ok((camera, mut projection)) = camera.single_mut() else { return };
    let phase = clock.phase();
    let key = (level.geometry.id(), phase);
    if shown.0 == Some(key) {
        return;
    }
    shown.0 = Some(key);
    let Some(environment) = &level_geometry.environment else { return };

    let fog = environment.fog_colours[phase as usize];
    clear.0 = Color::srgb(fog.x, fog.y, fog.z);
    let far = environment.view_distance;
    if let Projection::Perspective(perspective) = projection.as_mut() {
        perspective.far = far;
    }
    commands.entity(camera).insert(DistanceFog {
        color: Color::srgb(fog.x, fog.y, fog.z),
        falloff: FogFalloff::Linear {
            start: far * environment.fog_range.0,
            end: far * environment.fog_range.1,
        },
        ..default()
    });

    let sky = environment.sky_light(phase);
    let parts = level_geometry
        .parts
        .iter()
        .chain(level_geometry.props.iter().flat_map(|p| &p.parts).flat_map(|p| &p.meshes));
    for part in parts {
        if let Some(mut mesh) = meshes.get_mut(&part.mesh) {
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, part.lighting.colours(sky));
        }
    }
}
