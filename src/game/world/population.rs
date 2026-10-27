//! Putting a level's monsters and items in their places when it is built.

use std::collections::HashMap;

use super::STEP_HEIGHT;
use super::navigation::{LevelNavigation, Walker, WalkerMotion, WalkerMotionConfig};
use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::prelude::*;
use bevy_tnua::builtins::TnuaBuiltinWalkConfig;
use bevy_tnua::prelude::*;
use bevy_tnua_avian3d::prelude::*;

/// Half the height of a monster's body. Bodies are flat cylinders as wide
/// as the model's walk radius, however tall the model.
const HALF_HEIGHT: f32 = 0.5;
/// The ground sensor's radius: the ground under the body's centre decides
/// its height, not the highest point anywhere under a wide body.
const SENSOR_RADIUS: f32 = 0.3;
/// Wider monsters navigate the narrowest navmesh that still fits this much:
/// the wider navmeshes lose narrow ground (beaches, ledges) to erosion, so
/// big monsters may brush corners instead.
const MAX_NAVIGATION_RADIUS: f32 = 1.0;

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
    geometry: &LevelDefinition,
    remembered: Option<LevelRecord>,
    (species, items): (&MonsterCatalog, &ItemCatalog),
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
                        species: placed.species,
                        hp: max_hp,
                        max_hp,
                        stamina: max_stamina,
                        max_stamina,
                        conditions: Vec::new(),
                        body,
                    },
                    group: Some(placed.group),
                    feet: placed.position,
                    yaw: placed.yaw,
                })
            })
            .collect(),
    };
    let mut groups: HashMap<i32, Entity> = groups
        .into_iter()
        .map(|group| (group.authored, commands.spawn((group, ChildOf(root))).id()))
        .collect();
    let monster_count = monsters.iter().filter(|m| !m.monster.is_dead()).count();
    for kept in monsters {
        let monster = kept.monster;
        if monster.is_dead() {
            continue;
        }
        let Some(kind) = species.0.get(monster.species as usize) else { continue };
        let group = kept.group.map(|authored| {
            *groups.entry(authored).or_insert_with(|| {
                commands.spawn((MonsterGroup { authored, ..default() }, ChildOf(root))).id()
            })
        });
        let ScaledBody { radius, speed, .. } = monster.body;
        let float_height = if speed > 0.0 { STEP_HEIGHT + HALF_HEIGHT } else { 0.0 };
        let mut entity = commands.spawn((
            Name::new(format!("{} #{}", kind.name, monster.location)),
            monster,
            Transform::from_translation(kept.feet + Vec3::Y * float_height)
                .with_rotation(Quat::from_rotation_y(kept.yaw)),
            ChildOf(root),
        ));
        if let Some(group) = group {
            entity.insert(GroupMember(group));
        }
        if speed > 0.0 {
            let config = configs.add(WalkerMotionConfig {
                basis: TnuaBuiltinWalkConfig {
                    speed,
                    float_height,
                    max_slope: 50f32.to_radians(),
                    ..default()
                },
            });
            entity.insert((
                RigidBody::Dynamic,
                Position(kept.feet + Vec3::Y * float_height),
                Rotation(Quat::from_rotation_y(kept.yaw)),
                Collider::cylinder(radius, 2.0 * HALF_HEIGHT),
                LockedAxes::ROTATION_LOCKED,
                TnuaController::<WalkerMotion>::default(),
                TnuaConfig::<WalkerMotion>(config),
                TnuaAvian3dSensorShape(Collider::cylinder(SENSOR_RADIUS.min(radius), 0.0)),
                Walker { speed, float_height },
                AgentTarget3d::None,
                Velocity3d::default(),
            ));
            // Synthetic worlds can omit navigation; production activation
            // requires every requested navmesh before constructing actors.
            if !navigation.archipelagos.is_empty() {
                entity.insert(Agent3dBundle {
                    agent: default(),
                    settings: AgentSettings {
                        radius: radius.min(MAX_NAVIGATION_RADIUS),
                        desired_speed: speed * 0.8,
                        max_speed: speed,
                    },
                    archipelago_ref: ArchipelagoRef3d::new(
                        navigation.archipelago_for(radius.min(MAX_NAVIGATION_RADIUS)),
                    ),
                });
            }
        }
    }
    let lying: Vec<(WorldItem, Vec3)> = match remembered_items {
        Some(items) => items,
        None => geometry
            .items
            .iter()
            .filter_map(|placement| {
                let definition = items.get(placement.item)?;
                let loot = if definition.container {
                    super::items::GroundLoot::Pile {
                        appearance: definition.id,
                        identified: placement.identified || definition.auto_identify,
                        contents: vec![],
                    }
                } else {
                    super::items::GroundLoot::Loose(placed_item(placement, definition, ids))
                };
                Some((WorldItem { loot, hidden: placement.hidden }, placement.position))
            })
            .collect(),
    };
    info!("placed {monster_count} monsters and {} items", lying.len());
    for (item, at) in lying {
        let name = items.get(item.appearance()).map_or_else(String::new, |d| d.name.clone());
        commands.spawn((Name::new(name), item, Transform::from_translation(at), ChildOf(root)));
    }
}
