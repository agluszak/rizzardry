//! Putting a level's monsters and items in their places when it is built.

use std::collections::HashMap;

use bevy::prelude::*;

use super::level::{KeptMonster, LevelRecord};
use crate::content::items::ItemCatalog;
use crate::content::level::LevelGeometry;
use crate::content::monster::MonsterBody;
use crate::content::species::MonsterCatalog;
use crate::game::characters::{Condition, Lasting};
use crate::game::inventory::ItemIds;
use crate::game::rng::GameRng;
use crate::game::world::items::{WorldItem, placed_item};

/// A monster in the level: which one (its level-unique location id), what
/// it is, its body, and its health and stamina.
#[derive(Component, Clone, Debug)]
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
    pub body: ScaledBody,
}

impl Monster {
    pub fn is_dead(&self) -> bool {
        self.hp == 0
    }
}

/// A monster's effective gameplay body at the size it was rolled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScaledBody {
    pub scale: f32,
    /// Reach is measured from this far out from its centre (its scaled
    /// fight radius), in metres.
    pub radius: f32,
    /// Its eyes above its feet (its scaled target height).
    pub eye_height: f32,
    /// How fast it walks, in metres a second; 0 for those that do not
    /// (retail's navigator scales speed with size, `Navigator::SetScale`).
    pub speed: f32,
}

impl ScaledBody {
    /// The body of `definition` at retail's size roll (below 1000).
    pub fn new(definition: &MonsterBody, size_roll: u32) -> Self {
        let scale = definition.scale_for(size_roll);
        ScaledBody {
            scale,
            radius: definition.fight_radius * scale,
            eye_height: definition.target_height * scale,
            speed: definition.movement_rate * scale,
        }
    }
}

/// Required body definitions by species. Visual model construction is not a
/// gameplay prerequisite and is requested separately by presentation.
#[derive(Resource, Debug, Default)]
pub struct MonsterBodies(pub HashMap<u32, Handle<MonsterBody>>);

impl MonsterBodies {
    /// The bodies of the monsters a level places, loading any not yet
    /// requested.
    pub fn for_level(
        &mut self,
        geometry: &LevelGeometry,
        species: &MonsterCatalog,
        assets: &AssetServer,
    ) -> Vec<Handle<MonsterBody>> {
        geometry
            .monsters
            .iter()
            .filter_map(|placed| {
                let kind = species.0.get(placed.species as usize)?;
                let body = self.0.entry(placed.species).or_insert_with(|| {
                    assets.load(format!("retail://data/monsters/{}.mls", kind.model))
                });
                Some(body.clone())
            })
            .collect()
    }
}

/// Spawn the level's monsters and items under its root, as part of its
/// activation: on a first visit every placed monster, idling at its feet
/// and facing its heading (retail places them all at once on arrival), with
/// health and stamina rolled from its species in placement order, and the
/// items as authored; on a return, the monsters as the party left them
/// (the dead stay gone) and the items where they lay.
pub fn populate(
    commands: &mut Commands,
    root: Entity,
    geometry: &LevelGeometry,
    remembered: Option<LevelRecord>,
    (species, items): (&MonsterCatalog, &ItemCatalog),
    (handles, bodies): (&HashMap<u32, Handle<MonsterBody>>, &Assets<MonsterBody>),
    ids: &mut ItemIds,
    rng: &mut GameRng,
) {
    let (remembered_monsters, remembered_items, groups) = match remembered {
        Some(record) => (Some(record.monsters), Some(record.items), record.groups),
        None => (None, None, default()),
    };
    let monsters: Vec<KeptMonster> = match remembered_monsters {
        Some(monsters) => monsters,
        None => geometry
            .monsters
            .iter()
            .filter_map(|placed| {
                let kind = species.0.get(placed.species as usize)?;
                let body_definition = handles.get(&placed.species)?;
                let max_hp = rng.roll(kind.hit_points).max(1);
                let max_stamina = rng.roll(kind.stamina);
                // Retail draws Random(1000) for the size; its left-handed
                // mirror draw, presentation only, is not taken from
                // gameplay's stream.
                let size_roll = rng.below(1000);
                let body = ScaledBody::new(bodies.get(body_definition)?, size_roll);
                Some(KeptMonster {
                    monster: Monster {
                        location: placed.location,
                        group: placed.group,
                        species: placed.species,
                        hp: max_hp,
                        max_hp,
                        stamina: max_stamina,
                        max_stamina,
                        conditions: Vec::new(),
                        body,
                    },
                    feet: placed.position,
                    yaw: placed.yaw,
                })
            })
            .collect(),
    };
    let monster_count = monsters.iter().filter(|m| !m.monster.is_dead()).count();
    for kept in monsters {
        let monster = kept.monster;
        if monster.is_dead() {
            continue;
        }
        let Some(kind) = species.0.get(monster.species as usize) else { continue };
        commands.spawn((
            Name::new(format!("{} #{}", kind.name, monster.location)),
            monster,
            Transform::from_translation(kept.feet).with_rotation(Quat::from_rotation_y(kept.yaw)),
            ChildOf(root),
        ));
    }
    let lying: Vec<(WorldItem, Vec3)> = match remembered_items {
        Some(items) => items,
        None => geometry
            .items
            .iter()
            .filter_map(|placement| {
                let definition = items.get(placement.item)?;
                let item = placed_item(placement, definition, ids);
                let hidden = placement.hidden;
                Some((WorldItem { item, hidden, contents: vec![] }, placement.position))
            })
            .collect(),
    };
    info!("placed {monster_count} monsters and {} items", lying.len());
    for (item, at) in lying {
        let name = items.get(item.item.item).map_or_else(String::new, |d| d.name.clone());
        commands.spawn((Name::new(name), item, Transform::from_translation(at), ChildOf(root)));
    }
    commands.entity(root).insert(groups);
}
