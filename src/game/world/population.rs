//! Putting a level's monsters and items in their places when it is built.

use std::collections::HashMap;

use super::navigation::{LevelNavigation, WalkerMotionConfig};
use crate::game::world::level::LevelOwned;
use bevy::prelude::*;

use super::identify::{GroupMember, MonsterGroup};
use super::level::{KeptMonster, LevelRecord};
use crate::content::items::ItemCatalog;
use crate::content::level::LevelDefinition;
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
    pub enchantments: crate::game::magic::enchantments::Enchantments,
    pub location: i32,
    pub species: u32,
    pub hp: u32,
    pub max_hp: u32,
    pub stamina: u32,
    pub max_stamina: u32,
    pub conditions: Vec<(Condition, Lasting)>,
    pub body: ScaledBody,
}

/// A monster something has hurt: it fights whatever its authored
/// disposition (retail `MakeTargetGroupHostile`).
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct Provoked;

impl Monster {
    pub fn is_dead(&self) -> bool {
        self.hp == 0
    }

    pub fn can_fight(&self) -> bool {
        crate::game::characters::can_fight(self.hp, &self.conditions)
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
        geometry: &LevelDefinition,
        species: &MonsterCatalog,
        assets: &AssetServer,
    ) -> Vec<Handle<MonsterBody>> {
        geometry
            .monsters
            .iter()
            .filter_map(|placed| {
                let kind = species.kind(placed.species)?;
                let body = self
                    .0
                    .entry(placed.species)
                    .or_insert_with(|| assets.load(crate::content::paths::monster(&kind.model)));
                Some(body.clone())
            })
            .collect()
    }
}

/// Check that every monster and item the level places exists, as part of
/// its preparation: one missing is broken content, not something to leave
/// out of the level.
pub fn check(
    geometry: &LevelDefinition,
    species: &MonsterCatalog,
    items: &ItemCatalog,
) -> Result<(), String> {
    if let Some(placed) = geometry.monsters.iter().find(|p| species.kind(p.species).is_none()) {
        return Err(format!(
            "monster {} is of unknown species {}",
            placed.location, placed.species
        ));
    }
    if let Some(placed) = geometry.items.iter().find(|p| items.get(p.item).is_none()) {
        return Err(format!("an item lying about is unknown ({:?})", placed.item));
    }
    Ok(())
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
    geometry: &LevelDefinition,
    remembered: Option<LevelRecord>,
    (species, items, tables): (&MonsterCatalog, &ItemCatalog, &crate::content::items::LootTables),
    (map, npcs): (&str, &mut crate::game::npcs::NpcPopulation),
    (handles, bodies): (&HashMap<u32, Handle<MonsterBody>>, &Assets<MonsterBody>),
    (navigation, configs): (&LevelNavigation, &mut Assets<WalkerMotionConfig>),
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
            .filter(|placed| species.0[placed.species as usize].npc.is_none())
            .map(|placed| {
                let kind = &species.0[placed.species as usize];
                let body = bodies.get(&handles[&placed.species]).unwrap();
                KeptMonster::rolled(placed, kind, body, rng)
            })
            .collect(),
    };
    let mut groups: HashMap<i32, Entity> = groups
        .into_iter()
        .map(|group| (group.authored, commands.spawn((group, LevelOwned(root))).id()))
        .collect();
    let mut actors = Vec::new();
    for kept in monsters {
        if kept.monster.is_dead() {
            continue;
        }
        let body = kept.monster.body;
        let name =
            format!("{} #{}", species.0[kept.monster.species as usize].name, kept.monster.location);
        let mut entity = commands.spawn((Name::new(name), kept.monster));
        if kept.provoked {
            entity.insert(Provoked);
        }
        actors.push((entity.id(), kept.feet, kept.yaw, body, kept.group));
    }
    let average = npcs.average_level();
    let mut placed_actors = std::collections::HashSet::new();
    for placed in &geometry.monsters {
        let Some(kind) = species.kind(placed.species) else { continue };
        let Some(id) = kind.npc else { continue };
        let body = bodies.get(&handles[&placed.species]).unwrap();
        let place = crate::game::npcs::Placing { placed, tables, id, map, average };
        let Some(actor) = npcs.place(commands, place, items, &mut placed_actors, rng, |rng| {
            KeptMonster::rolled(placed, kind, body, rng)
        }) else {
            continue;
        };
        commands.entity(actor.0).insert(Name::new(format!("{} #{}", kind.name, placed.location)));
        actors.push(actor);
    }
    actors.extend(npcs.visitors(commands, map, &mut placed_actors));
    let monster_count = actors.len();
    for (actor, feet, yaw, body, group) in actors {
        let group = group.map(|authored| {
            *groups.entry(authored).or_insert_with(|| {
                commands.spawn((MonsterGroup { authored, ..default() }, LevelOwned(root))).id()
            })
        });
        let ScaledBody { radius, speed, .. } = body;
        let mut entity = commands.entity(actor);
        entity.insert((
            Transform::from_translation(feet + Vec3::Y * super::navigation::float_height(speed))
                .with_rotation(Quat::from_rotation_y(yaw)),
            LevelOwned(root),
        ));
        if let Some(group) = group {
            entity.insert(GroupMember(group));
        }
        if speed > 0.0 {
            super::navigation::insert_walker(
                &mut entity,
                (feet, yaw),
                (radius, speed),
                navigation,
                configs,
            );
        }
    }
    let lying: Vec<(WorldItem, Vec3)> = match remembered_items {
        Some(items) => items,
        None => geometry
            .items
            .iter()
            .map(|placement| {
                let definition = items.get(placement.item).expect("checked in preparation");
                let loot = if definition.container {
                    super::items::GroundLoot::Pile {
                        appearance: definition.id,
                        identified: placement.identified || definition.auto_identify,
                        contents: vec![],
                    }
                } else {
                    super::items::GroundLoot::Loose(placed_item(placement, definition, ids))
                };
                (WorldItem { loot, hidden: placement.hidden }, placement.position)
            })
            .collect(),
    };
    info!("placed {monster_count} monsters and {} items", lying.len());
    for (item, at) in lying {
        let name = items.get(item.appearance()).map_or_else(String::new, |d| d.name.clone());
        commands.spawn((Name::new(name), item, Transform::from_translation(at), LevelOwned(root)));
    }
}
