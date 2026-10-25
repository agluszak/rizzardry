//! Putting a level's monsters and items in their places when it is built.

use bevy::prelude::*;

use super::level::Level;
use crate::content::items::ItemCatalog;
use crate::content::level::LevelGeometry;
use crate::content::monster::MonsterModel;
use crate::content::species::MonsterCatalog;
use crate::game::characters::{Condition, Lasting};
use crate::game::data::GameData;
use crate::game::inventory::ItemIds;
use crate::game::rng::GameRng;
use crate::game::world::items::{WorldItem, placed_item};
use crate::presentation::monsters::MonsterAppearance;

/// A monster in the level: which one (its level-unique location id), what
/// it is, and its health and stamina.
#[derive(Component, Debug)]
pub struct Monster {
    pub location: i32,
    pub species: u32,
    pub hp: u32,
    pub max_hp: u32,
    pub stamina: u32,
    pub max_stamina: u32,
    pub conditions: Vec<(Condition, Lasting)>,
    /// Reach is measured from this far out from its centre (its scaled
    /// fight radius), in metres; 0 until its model is known.
    pub radius: f32,
    /// Its eyes above its feet (its scaled target height).
    pub eye_height: f32,
}

impl Monster {
    pub fn is_dead(&self) -> bool {
        self.hp == 0
    }
}

/// Note each monster's reach radius and eye height once its model is
/// known, scaled to its size.
pub fn measure_monsters(
    mut monsters: Query<(&mut Monster, &MonsterAppearance)>,
    models: Res<Assets<MonsterModel>>,
) {
    for (mut monster, appearance) in &mut monsters {
        if monster.radius > 0.0 {
            continue;
        }
        let Some(model) = models.get(&appearance.model) else { continue };
        let scale = appearance.scale(model);
        monster.radius = model.body.fight_radius * scale;
        monster.eye_height = model.body.target_height * scale;
    }
}

/// The level's monsters have been placed.
#[derive(Component, Debug)]
pub struct Populated;

/// Spawn every placed monster under the level root, idling, at its feet and
/// facing its heading (retail places them all at once on arrival), with
/// health and stamina rolled from its species, in placement order. They do
/// not move yet. A revisited level rolls them afresh: monsters are not
/// remembered yet.
pub fn populate_level(
    mut commands: Commands,
    levels: Query<(Entity, &Level), Without<Populated>>,
    geometry: Res<Assets<LevelGeometry>>,
    assets: Res<AssetServer>,
    data: Option<Res<GameData>>,
    catalogs: Res<Assets<MonsterCatalog>>,
    items: Res<Assets<ItemCatalog>>,
    mut ids: ResMut<ItemIds>,
    mut rng: ResMut<GameRng>,
) {
    let Some(data) = data else { return };
    let (Some(catalog), Some(items)) = (catalogs.get(&data.species), items.get(&data.items)) else {
        return;
    };
    for (root, level) in &levels {
        let Some(geometry) = geometry.get(&level.geometry) else { continue };
        for monster in &geometry.monsters {
            let Some(species) = catalog.0.get(monster.species as usize) else { continue };
            let max_hp = rng.roll(species.hit_points).max(1);
            let max_stamina = rng.roll(species.stamina);
            // Retail draws Random(1000) for the size; its left-handed mirror
            // draw, presentation only, is not taken from gameplay's stream.
            let size_roll = rng.below(1000);
            commands.spawn((
                Name::new(format!("{} #{}", species.name, monster.location)),
                Monster {
                    location: monster.location,
                    species: monster.species,
                    hp: max_hp,
                    max_hp,
                    stamina: max_stamina,
                    max_stamina,
                    conditions: Vec::new(),
                    radius: 0.0,
                    eye_height: 0.0,
                },
                MonsterAppearance {
                    model: assets.load(format!("retail://data/monsters/{}.mls", monster.model)),
                    float_height: 0.0,
                    // Retail starts each idle at a random frame; vary it by
                    // location instead.
                    phase: (monster.location.unsigned_abs() % 97) as f32 * 0.37,
                    size_roll: Some(size_roll),
                },
                Transform::from_translation(monster.position)
                    .with_rotation(Quat::from_rotation_y(monster.yaw)),
                ChildOf(root),
            ));
        }
        for placement in &geometry.items {
            let Some(definition) = items.get(placement.item) else { continue };
            commands.spawn((
                Name::new(definition.name.clone()),
                WorldItem {
                    item: placed_item(placement, definition, &mut ids),
                    hidden: placement.hidden,
                },
                Transform::from_translation(placement.position),
                ChildOf(root),
            ));
        }
        info!("placed {} monsters and {} items", geometry.monsters.len(), geometry.items.len());
        commands.entity(root).insert(Populated);
    }
}
