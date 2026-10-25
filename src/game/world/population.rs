//! Putting a level's monsters in their places when it is built.

use bevy::prelude::*;

use super::level::Level;
use crate::content::level::LevelGeometry;
use crate::presentation::monsters::MonsterAppearance;

/// A monster standing in the level, by its level-unique location id.
#[derive(Component, Debug)]
pub struct PlacedMonster {
    pub location: i32,
    pub species: u32,
}

/// The level's monsters have been placed.
#[derive(Component, Debug)]
pub struct Populated;

/// Spawn every placed monster under the level root, idling, at its feet and
/// facing its heading (retail places them all at once on arrival). They do
/// not move or fight yet.
pub fn populate_level(
    mut commands: Commands,
    levels: Query<(Entity, &Level), Without<Populated>>,
    geometry: Res<Assets<LevelGeometry>>,
    assets: Res<AssetServer>,
) {
    for (root, level) in &levels {
        let Some(geometry) = geometry.get(&level.geometry) else { continue };
        for monster in &geometry.monsters {
            commands.spawn((
                Name::new(format!("{} #{}", monster.model, monster.location)),
                PlacedMonster { location: monster.location, species: monster.species },
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
