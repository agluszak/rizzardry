//! Putting a level's monsters in their places when it is built.

use bevy::prelude::*;

use super::level::Level;
use crate::content::level::LevelGeometry;
use crate::content::species::MonsterCatalog;
use crate::game::characters::{Condition, Lasting};
use crate::game::data::GameData;
use crate::game::rng::GameRng;
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
}

impl Monster {
    pub fn is_dead(&self) -> bool {
        self.hp == 0
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
    mut rng: ResMut<GameRng>,
) {
    let Some(catalog) = data.and_then(|d| catalogs.get(&d.species)) else { return };
    for (root, level) in &levels {
        let Some(geometry) = geometry.get(&level.geometry) else { continue };
        for monster in &geometry.monsters {
            let Some(species) = catalog.0.get(monster.species as usize) else { continue };
            let max_hp = rng.roll(species.hit_points).max(1);
            let max_stamina = rng.roll(species.stamina);
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
                },
                MonsterAppearance {
                    model: assets.load(format!("retail://data/monsters/{}.mls", monster.model)),
                    float_height: 0.0,
                    // Retail starts each idle at a random frame; vary it by
                    // location instead.
                    phase: (monster.location.unsigned_abs() % 97) as f32 * 0.37,
                },
                Transform::from_translation(monster.position)
                    .with_rotation(Quat::from_rotation_y(monster.yaw)),
                ChildOf(root),
            ));
        }
        info!("placed {} monsters", geometry.monsters.len());
        commands.entity(root).insert(Populated);
    }
}
