//! Putting a level's monsters and items in their places when it is built.

use bevy::prelude::*;

use super::level::LevelRecord;
use crate::content::items::ItemCatalog;
use crate::content::level::LevelGeometry;
use crate::content::monster::MonsterModel;
use crate::content::species::MonsterCatalog;
use crate::game::characters::{Condition, Lasting};
use crate::game::inventory::ItemIds;
use crate::game::rng::GameRng;
use crate::game::world::items::{WorldItem, placed_item};
use crate::presentation::monsters::MonsterAppearance;

/// A monster in the level: which one (its level-unique location id), what
/// it is, and its health and stamina.
#[derive(Component, Debug)]
pub struct Monster {
    pub location: i32,
    /// Its group, by the level's group id.
    pub group: i32,
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

/// Spawn every placed monster under the level root, idling, at its feet and
/// facing its heading (retail places them all at once on arrival), with
/// health and stamina rolled from its species, in placement order; and the
/// items lying about, as the party left them or as authored. Part of a
/// level's activation. A revisited level rolls its monsters afresh:
/// monsters are not remembered yet.
#[allow(clippy::too_many_arguments)]
pub fn populate(
    commands: &mut Commands,
    root: Entity,
    geometry: &LevelGeometry,
    remembered: Option<&LevelRecord>,
    (species, items): (&MonsterCatalog, &ItemCatalog),
    assets: &AssetServer,
    ids: &mut ItemIds,
    rng: &mut GameRng,
) {
    for monster in &geometry.monsters {
        let Some(kind) = species.0.get(monster.species as usize) else { continue };
        let max_hp = rng.roll(kind.hit_points).max(1);
        let max_stamina = rng.roll(kind.stamina);
        // Retail draws Random(1000) for the size; its left-handed mirror
        // draw, presentation only, is not taken from gameplay's stream.
        let size_roll = rng.below(1000);
        commands.spawn((
            Name::new(format!("{} #{}", kind.name, monster.location)),
            Monster {
                location: monster.location,
                group: monster.group,
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
    let lying: Vec<(WorldItem, Vec3)> = match remembered {
        Some(record) => record.items.clone(),
        None => geometry
            .items
            .iter()
            .filter_map(|placement| {
                let definition = items.get(placement.item)?;
                let item = placed_item(placement, definition, ids);
                Some((WorldItem { item, hidden: placement.hidden }, placement.position))
            })
            .collect(),
    };
    info!("placed {} monsters and {} items", geometry.monsters.len(), lying.len());
    for (item, at) in lying {
        let name = items.get(item.item.item).map_or_else(String::new, |d| d.name.clone());
        commands.spawn((Name::new(name), item, Transform::from_translation(at), ChildOf(root)));
    }
    commands.entity(root).insert(super::identify::MonsterGroups::default());
}
