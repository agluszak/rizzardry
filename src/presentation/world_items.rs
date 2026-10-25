//! Showing items lying in the world: their models turning in place, lit as
//! retail lights them (`ActivateItem`, `W8Item::UpdateAnimation`).

use bevy::prelude::*;

use avian3d::prelude::SpatialQuery;

use crate::content::items::{ItemCatalog, ItemModel};
use crate::content::level::LevelGeometry;
use crate::game::data::GameData;
use crate::game::time::GameClock;
use crate::game::world::items::WorldItem;
use crate::game::world::level::Level;
use crate::presentation::monsters::sunlit;

/// Retail turns a lying item a sixteenth of a turn every 50 ms (the main
/// screen's world update); here it turns smoothly at that rate.
const TURN_PER_SECOND: f32 = std::f32::consts::PI / 16.0 / 0.05;

/// The shown model of a lying item.
#[derive(Component, Debug)]
pub struct ItemShown {
    model: Handle<ItemModel>,
    /// This item's copies of the model's meshes, once built.
    parts: Option<Vec<Handle<Mesh>>>,
    /// The sky light it gets: all in sight of the `SUN` light, none out of
    /// it, decided where it lies (retail `light_scale`).
    sky_scale: Option<f32>,
}

/// Start loading each visible item's model; a model that fails to load
/// shows retail's question mark.
pub fn show_world_items(
    mut commands: Commands,
    items: Query<(Entity, &WorldItem, Option<&ItemShown>)>,
    data: Option<Res<GameData>>,
    catalogs: Res<Assets<ItemCatalog>>,
    assets: Res<AssetServer>,
) {
    let Some(catalog) = data.and_then(|d| catalogs.get(&d.items)) else { return };
    for (entity, item, shown) in &items {
        let model = match shown {
            _ if item.hidden => continue,
            None => catalog.get(item.item.item).map(|d| d.model.as_str()),
            Some(shown) if assets.get_load_state(&shown.model).is_some_and(|s| s.is_failed()) => {
                Some("data/items3d/questionmark.itm")
            }
            Some(_) => None,
        };
        if let Some(model) = model {
            commands.entity(entity).insert(ItemShown {
                model: assets.load(format!("retail://{model}")),
                parts: None,
                sky_scale: None,
            });
        }
    }
}

pub fn build_world_items(
    mut commands: Commands,
    mut items: Query<(Entity, &WorldItem, &mut ItemShown, &mut Transform)>,
    models: Res<Assets<ItemModel>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    for (entity, item, mut shown, mut transform) in &mut items {
        if shown.parts.is_some() {
            continue;
        }
        let Some(model) = models.get(&shown.model) else { continue };
        let mut parts = Vec::new();
        for part in &model.parts {
            let Some(mesh) = meshes.get(&part.mesh).cloned() else { continue };
            let copy = meshes.add(mesh);
            commands.spawn((
                Mesh3d(copy.clone()),
                MeshMaterial3d(part.material.clone()),
                ChildOf(entity),
            ));
            parts.push(copy);
        }
        shown.parts = Some(parts);
        // Retail rolls a yaw below 360 degrees; presentation does not take
        // gameplay's random numbers, so it comes from the item's identity.
        transform.rotation = Quat::from_rotation_y(((item.item.id.0 % 360) as f32).to_radians());
    }
}

pub fn turn_world_items(time: Res<Time>, mut items: Query<&mut Transform, With<ItemShown>>) {
    for mut transform in &mut items {
        // Retail turns them clockwise seen from above.
        transform.rotate_y(-TURN_PER_SECOND * time.delta_secs());
    }
}

/// Retail lights item models as it does monsters: sky light by the
/// item's sky scale and each material's ambient, the level's offset and
/// nearby lights, and the material's own glow.
pub fn light_world_items(
    time: Res<Time>,
    hovered: Res<super::hover::Hover>,
    clock: Res<GameClock>,
    levels: Query<(Entity, &Level)>,
    geometry: Res<Assets<LevelGeometry>>,
    models: Res<Assets<ItemModel>>,
    spatial: SpatialQuery,
    mut items: Query<(Entity, &GlobalTransform, &mut ItemShown)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some((level_entity, level)) =
        levels.iter().find_map(|(e, l)| geometry.get(&l.geometry).map(|g| (e, g)))
    else {
        return;
    };
    let sky = level.environment.as_ref().map_or(Vec3::ZERO, |e| e.sky_light(clock.phase()));
    // The hovered item pulses green: retail raises and lowers its level by
    // 0.05 every 50 ms between 0.25 and 1. How retail's renderer mixes the
    // highlight in is not recovered; here it adds green light.
    let phase = (time.elapsed_secs() / PULSE_SECONDS).fract();
    let pulse = 0.25 + 0.75 * (1.0 - (2.0 * phase - 1.0).abs());
    for (entity, transform, mut shown) in &mut items {
        let at = transform.translation();
        let sky_scale = *shown.sky_scale.get_or_insert_with(|| {
            if sunlit(&spatial, level_entity, level, at) { 1.0 } else { 0.0 }
        });
        let fixed = level.lighting.at(at, None);
        let (Some(model), Some(parts)) = (models.get(&shown.model), &shown.parts) else { continue };
        for (part, copy) in model.parts.iter().zip(parts) {
            let mut light = sky * sky_scale * part.ambient + fixed + Vec3::splat(part.emission);
            if hovered.entity() == Some(entity) {
                light += Vec3::Y * pulse;
            }
            let colour = light.extend(1.0).to_array();
            let Some(mut mesh) = meshes.get_mut(copy) else { continue };
            let count = mesh.count_vertices();
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![colour; count]);
        }
    }
}

/// A full rise and fall of the highlight: 15 steps up and 15 down, 50 ms
/// each.
const PULSE_SECONDS: f32 = 1.5;
