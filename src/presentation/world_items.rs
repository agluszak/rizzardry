//! Showing items lying in the world: their models turning in place, lit as
//! retail lights them (`ActivateItem`, `W8Item::UpdateAnimation`).

use super::surfaces::ActorLight;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use avian3d::prelude::SpatialQuery;

use super::environment::LevelAppearance;
use crate::content::items::{ItemCatalog, ItemModel};
use crate::content::level::LevelGeometry;
use crate::game::data::GameData;
use crate::game::time::GameClock;
use crate::game::world::items::WorldItem;
use crate::presentation::monsters::sunlit;

/// Retail turns a lying item a sixteenth of a turn every 50 ms (the main
/// screen's world update); here it turns smoothly at that rate.
const TURN_PER_SECOND: f32 = std::f32::consts::PI / 16.0 / 0.05;

/// The shown model of a lying item.
#[derive(Component, Debug)]
pub struct ItemShown {
    model: Handle<ItemModel>,
    /// Where it was lit, the sky light it gets there (all in sight of the
    /// `SUN` light, none out of it: retail `light_scale`) and the level's
    /// fixed light there.
    lit_at: Option<(Vec3, f32, Vec3)>,
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
            None => catalog.get(item.appearance()).map(|d| d.model.as_str()),
            Some(shown) if assets.get_load_state(&shown.model).is_some_and(|s| s.is_failed()) => {
                Some("data/items3d/questionmark.itm")
            }
            Some(_) => None,
        };
        if let Some(model) = model {
            commands.entity(entity).insert(ItemShown {
                model: assets.load(format!("retail://{model}")),
                lit_at: None,
            });
        }
    }
}

/// Cosmetic pose belongs to the visual child, never the world-placement root.
#[derive(Component, Debug)]
pub struct ItemVisual;

pub fn build_world_items(
    mut commands: Commands,
    items: Query<(Entity, &ItemShown, Option<&Children>), With<WorldItem>>,
    visuals: Query<(), With<ItemVisual>>,
    models: Res<Assets<ItemModel>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (entity, shown, children) in &items {
        if children.is_some_and(|c| c.iter().any(|e| visuals.contains(e))) {
            continue;
        }
        let Some(model) = models.get(&shown.model) else { continue };
        commands.entity(entity).insert((Visibility::default(), ActorLight::default()));
        let root = commands
            .spawn((
                ItemVisual,
                Transform::from_rotation(Quat::from_rotation_y(
                    ((entity.to_bits() % 360) as f32).to_radians(),
                )),
                Visibility::default(),
                ChildOf(entity),
            ))
            .id();
        let mut instances = HashMap::new();
        for part in &model.parts {
            let Some(surface) = part.surface.lit_instance(entity, &mut instances, &mut materials)
            else {
                continue;
            };
            commands.spawn((Mesh3d(part.mesh.clone()), surface, ChildOf(root)));
        }
    }
}

pub fn turn_world_items(time: Res<Time>, mut items: Query<&mut Transform, With<ItemVisual>>) {
    for mut transform in &mut items {
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
    levels: Query<(Entity, &LevelAppearance)>,
    geometry: Res<Assets<LevelGeometry>>,
    spatial: SpatialQuery,
    mut items: Query<(Entity, &GlobalTransform, &mut ItemShown, &mut ActorLight)>,
) {
    let Some((level_entity, level)) =
        levels.iter().find_map(|(e, l)| geometry.get(&l.0).map(|g| (e, g)))
    else {
        return;
    };
    let sky = level.environment.as_ref().map_or(Vec3::ZERO, |e| e.sky_light(clock.phase()));
    // The hovered item pulses green: retail raises and lowers its level by
    // 0.05 every 50 ms between 0.25 and 1. How retail's renderer mixes the
    // highlight in is not recovered; here it adds green light.
    let phase = (time.elapsed_secs() / PULSE_SECONDS).fract();
    let pulse = 0.25 + 0.75 * (1.0 - (2.0 * phase - 1.0).abs());
    for (entity, transform, mut shown, mut light) in &mut items {
        let at = transform.translation();
        let (sky_scale, fixed) = match shown.lit_at {
            Some((lit, sky_scale, fixed)) if lit == at && !geometry.is_changed() => {
                (sky_scale, fixed)
            }
            _ => {
                let sky_scale = if sunlit(&spatial, level_entity, level, at) { 1.0 } else { 0.0 };
                let fixed = level.lighting.at(at, None);
                shown.lit_at = Some((at, sky_scale, fixed));
                (sky_scale, fixed)
            }
        };
        let highlight = if hovered.entity() == Some(entity) { Vec3::Y * pulse } else { Vec3::ZERO };
        light.set_if_neq(ActorLight { sky: sky * sky_scale, fixed: fixed + highlight });
    }
}

/// A full rise and fall of the highlight: 15 steps up and 15 down, 50 ms
/// each.
const PULSE_SECONDS: f32 = 1.5;
