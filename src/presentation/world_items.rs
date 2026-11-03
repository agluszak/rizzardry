//! Showing items lying in the world: their models turning in place, lit as
//! retail lights them (`ActivateItem`, `W8Item::UpdateAnimation`).

use super::surfaces::{ActorLight, LightSample, SkyResponse};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use crate::content::items::ItemModel;
use crate::game::world::items::WorldItem;

/// Retail turns a lying item a sixteenth of a turn every 50 ms (the main
/// screen's world update); here it turns smoothly at that rate.
const TURN_PER_SECOND: f32 = std::f32::consts::PI / 16.0 / 0.05;

/// The shown model of a lying item.
#[derive(Component, Debug)]
pub struct ItemShown {
    model: Handle<ItemModel>,
}

/// Start loading each visible item's model; a model that fails to load
/// shows retail's question mark.
pub fn show_world_items(
    mut commands: Commands,
    items: Query<(Entity, &WorldItem, Option<&ItemShown>)>,
    content: If<Res<crate::game::data::Catalogs>>,
    assets: Res<AssetServer>,
) {
    let catalog = &content.items;
    for (entity, item, shown) in &items {
        let model = match shown {
            _ if item.hidden => continue,
            None => Some(catalog[item.appearance()].model.as_str()),
            Some(shown) if assets.get_load_state(&shown.model).is_some_and(|s| s.is_failed()) => {
                Some("data/items3d/questionmark.itm")
            }
            Some(_) => None,
        };
        if let Some(model) = model {
            commands
                .entity(entity)
                .insert(ItemShown { model: assets.load(crate::content::paths::retail(model)) });
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
        // Lit as retail lights item models: all the sky light in sight of
        // the `SUN` light at once, none out of it (retail `light_scale`).
        commands.entity(entity).insert((
            Visibility::default(),
            LightSample::default(),
            SkyResponse::Immediate,
            ActorLight::default(),
        ));
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
