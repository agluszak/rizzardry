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
    /// What has been done to its spell budget since it was met: drained
    /// away, or raised by Might to Magic (retail `spell_points`, zero at
    /// first). Its casting stands on this and its species' budget together.
    pub spell_points: i32,
}

impl Monster {
    /// What its casting stands on now (retail `ChooseMonsterSpellPowerLevel`,
    /// `MonsterCastsSpell`): its species' budget and what has been done to
    /// it, never below nothing.
    pub fn spell_budget(&self, kind: &crate::content::species::Species) -> u32 {
        self.spell_points.saturating_add(kind.magic.budget).max(0) as u32
    }

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
    /// The bodies of every species a level may bring into play, loading any
    /// not yet requested, since a group called up in play must have its
    /// body ready: those it places, its random encounters and their
    /// companions, and whatever any of these can summon, in turn.
    pub fn for_level(
        &mut self,
        geometry: &LevelDefinition,
        species: &MonsterCatalog,
        assets: &AssetServer,
    ) -> Vec<Handle<MonsterBody>> {
        let encountered: Vec<u32> = geometry
            .encounters
            .generators
            .iter()
            .flat_map(|generator| generator.table.iter().flat_map(|table| &table.entries))
            .map(|entry| entry.species)
            .collect();
        let companions: Vec<u32> = encountered
            .iter()
            .filter_map(|&id| species.kind(id))
            .flat_map(|kind| kind.companions.iter().flatten().map(|&(id, _)| id))
            .collect();
        let mut spawnable: Vec<u32> =
            geometry.monsters.iter().map(|placed| placed.species).collect();
        spawnable.extend(encountered);
        spawnable.extend(companions);
        let mut next = 0;
        while let Some(&id) = spawnable.get(next) {
            if let Some(called) = species.kind(id).and_then(|kind| kind.special.kind?.summons())
                && !spawnable.contains(&called)
            {
                spawnable.push(called);
            }
            next += 1;
        }
        spawnable
            .into_iter()
            .filter_map(|id| {
                let kind = species.kind(id)?;
                let body = self
                    .0
                    .entry(id)
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

/// The name an actor goes by in the world's inspector.
fn actor_name(species: &MonsterCatalog, monster: &Monster) -> String {
    let kind = species.kind(monster.species).map_or("", |kind| kind.name.as_str());
    format!("{kind} #{}", monster.location)
}

/// Make a monster an actor of its level: standing at its feet and facing
/// its heading, in its group, and a walker if it walks.
fn embody(
    entity: &mut EntityCommands,
    root: Entity,
    (feet, yaw, body): (Vec3, f32, ScaledBody),
    group: Option<Entity>,
    (navigation, configs): (&LevelNavigation, &mut Assets<WalkerMotionConfig>),
) {
    let ScaledBody { radius, speed, .. } = body;
    entity.insert((
        Transform::from_translation(feet + Vec3::Y * super::navigation::float_height(speed))
            .with_rotation(Quat::from_rotation_y(yaw)),
        LevelOwned(root),
    ));
    if let Some(group) = group {
        entity.insert(GroupMember(group));
    }
    if speed > 0.0 {
        super::navigation::insert_walker(entity, (feet, yaw), (radius, speed), navigation, configs);
    }
}

/// Where a group about `origin` facing `yaw` may stand (retail
/// `FindScatterPositions`): in ranks going back from it, `spacing` apart,
/// three abreast (five for more than ten), each spot after the first
/// jittered by up to a fifth of a step. A spot counts where `ground`
/// settles it (none when the ground falls too far) and, when `clear`
/// allows it there and `reaches` finds a way to it from the first. Returns
/// at most `count` spots, fewer when the ground will not hold them.
pub fn scatter(
    (origin, yaw, spacing): (Vec3, f32, f32),
    count: usize,
    rng: &mut GameRng,
    ground: impl Fn(Vec3) -> Option<Vec3>,
    clear: impl Fn(Vec3) -> bool,
    reaches: impl Fn(Vec3, Vec3) -> bool,
) -> Vec<Vec3> {
    const COLUMNS: [f32; 5] = [0.0, 1.0, -1.0, 2.0, -2.0];
    let columns = if count > 10 { 5 } else { 3 };
    let rotation = Quat::from_rotation_y(yaw);
    let (back, side) = (rotation * Vec3::Z * spacing, rotation * Vec3::X * spacing);
    let mut source = origin;
    let mut spots: Vec<Vec3> = Vec::new();
    for rank in 0..10 {
        for &column in &COLUMNS[..columns] {
            if spots.len() >= count {
                return spots;
            }
            let (jitter_rank, jitter_column) = if spots.is_empty() {
                (0.0, 0.0)
            } else {
                let jitter = |rng: &mut GameRng| rng.below(1000) as f32 * 0.0004 - 0.2;
                let rank = jitter(rng);
                (rank, jitter(rng))
            };
            let candidate =
                source + back * (rank as f32 + jitter_rank) + side * (column + jitter_column);
            let Some(spot) = ground(candidate).filter(|&spot| clear(spot)) else { continue };
            if spots.is_empty() {
                source = spot;
                spots.push(spot);
            } else if reaches(spot, source) {
                spots.push(spot);
            }
        }
    }
    spots
}

/// A group come into the level (retail `CreateGroup`): its members as
/// rolled and placed, under the group id given, with the stance something
/// set on it. Returns the group and its members.
pub fn spawn_group(
    commands: &mut Commands,
    root: Entity,
    members: Vec<KeptMonster>,
    (authored, set): (i32, Option<super::hostility::SetStance>),
    species: &MonsterCatalog,
    (navigation, configs): (&LevelNavigation, &mut Assets<WalkerMotionConfig>),
) -> (Entity, Vec<Entity>) {
    let group =
        commands.spawn((MonsterGroup { authored, set, ..default() }, LevelOwned(root))).id();
    let members = members
        .into_iter()
        .map(|kept| {
            let name = actor_name(species, &kept.monster);
            let body = kept.monster.body;
            let mut entity = commands.spawn((Name::new(name), kept.monster));
            embody(
                &mut entity,
                root,
                (kept.feet, kept.yaw, body),
                Some(group),
                (navigation, configs),
            );
            entity.id()
        })
        .collect();
    (group, members)
}

/// On a level root: its monsters still unborn, as rolled with the rest
/// (retail `plsUnbornMonsterList`). They take no part in anything until
/// their group is born.
#[derive(Component, Clone, Debug, Default)]
pub struct Unborn(pub Vec<KeptMonster>);

/// Birth an unborn group of the played level, by the level's id for it
/// (retail `GiveBirthToMonster`): its members come into the world where
/// they were placed. A group already born, or with no unborn members, is
/// left as it is.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Birth(pub i32);

/// Bring unborn groups into the world as they are born.
pub fn give_birth(
    mut commands: Commands,
    mut births: MessageReader<Birth>,
    mut levels: Query<(Entity, &mut Unborn, &LevelNavigation), With<super::level::Level>>,
    groups: Query<(Entity, &MonsterGroup, &LevelOwned)>,
    mut configs: ResMut<Assets<WalkerMotionConfig>>,
    content: crate::game::data::Content,
) {
    let Ok((root, mut unborn, navigation)) = levels.single_mut() else {
        births.clear();
        return;
    };
    let Some(species) = content.species() else { return };
    for &Birth(authored) in births.read() {
        let (born, waiting): (Vec<_>, Vec<_>) =
            std::mem::take(&mut unborn.0).into_iter().partition(|k| k.group == Some(authored));
        unborn.0 = waiting;
        if born.is_empty() {
            continue;
        }
        let group = groups
            .iter()
            .find(|(_, group, owner)| owner.0 == root && group.authored == authored)
            .map(|(entity, ..)| entity)
            .unwrap_or_else(|| {
                commands.spawn((MonsterGroup { authored, ..default() }, LevelOwned(root))).id()
            });
        for kept in born {
            let name = actor_name(species, &kept.monster);
            let body = kept.monster.body;
            let mut entity = commands.spawn((Name::new(name), kept.monster));
            embody(
                &mut entity,
                root,
                (kept.feet, kept.yaw, body),
                Some(group),
                (navigation, &mut configs),
            );
        }
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
    (map, npcs): (&str, &mut crate::game::npcs::NpcPopulation),
    (handles, bodies): (&HashMap<u32, Handle<MonsterBody>>, &Assets<MonsterBody>),
    (navigation, configs): (&LevelNavigation, &mut Assets<WalkerMotionConfig>),
    ids: &mut ItemIds,
    rng: &mut GameRng,
) {
    let (remembered_monsters, remembered_items, groups, encounters) = match remembered {
        Some(record) => (
            Some((record.monsters, record.unborn)),
            Some(record.items),
            record.groups,
            Some(record.encounters),
        ),
        None => (None, None, default(), None),
    };
    // Every placed monster is rolled in placement order, the unborn too.
    let (monsters, unborn): (Vec<KeptMonster>, Vec<KeptMonster>) = match remembered_monsters {
        Some(monsters) => monsters,
        None => {
            let (unborn, born): (Vec<_>, Vec<_>) = geometry
                .monsters
                .iter()
                .filter(|placed| species.0[placed.species as usize].npc.is_none())
                .map(|placed| {
                    let kind = &species.0[placed.species as usize];
                    let body = bodies.get(&handles[&placed.species]).unwrap();
                    (placed.unborn, KeptMonster::placed(placed, kind, body, rng))
                })
                .partition(|&(unborn, _)| unborn);
            (
                born.into_iter().map(|(_, k)| k).collect(),
                unborn.into_iter().map(|(_, k)| k).collect(),
            )
        }
    };
    commands.entity(root).insert(Unborn(unborn));
    // The encounters' timers jitter after the placed monsters' rolls.
    let encounters = encounters
        .unwrap_or_else(|| super::encounters::EncounterState::authored(&geometry.encounters, rng));
    commands.entity(root).insert(encounters);
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
        let name = actor_name(species, &kept.monster);
        let entity = commands.spawn((Name::new(name), kept.monster)).id();
        actors.push((entity, kept.feet, kept.yaw, body, kept.group));
    }
    let average = npcs.average_level();
    let mut placed_actors = std::collections::HashSet::new();
    // Unborn NPCs are not placed; NPCs are never born.
    for placed in geometry.monsters.iter().filter(|placed| !placed.unborn) {
        let Some(kind) = species.kind(placed.species) else { continue };
        let Some(id) = kind.npc else { continue };
        let body = bodies.get(&handles[&placed.species]).unwrap();
        let place = crate::game::npcs::Placing { placed, id, map, average };
        let Some(actor) = npcs.place(commands, place, items, &mut placed_actors, rng, |rng| {
            KeptMonster::placed(placed, kind, body, rng)
        }) else {
            continue;
        };
        commands.entity(actor.0).insert(Name::new(format!("{} #{}", kind.name, placed.location)));
        actors.push(actor);
    }
    let monster_count = actors.len();
    for (actor, feet, yaw, body, group) in actors {
        let group = group.map(|authored| {
            *groups.entry(authored).or_insert_with(|| {
                commands.spawn((MonsterGroup { authored, ..default() }, LevelOwned(root))).id()
            })
        });
        embody(&mut commands.entity(actor), root, (feet, yaw, body), group, (navigation, configs));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::level::{EncounterEntry, EncounterTable, Generator, MonsterPlacement};
    use crate::content::species::{SpecialAttack, SpecialAttackKind, Species};

    /// A species only a generator brings, which summons a species nothing
    /// places, has that species' body readied too.
    #[test]
    fn an_encounters_summoning_has_its_body_ready() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default())).init_asset::<MonsterBody>();
        let mut catalog = vec![Species::default(); 13];
        catalog[3] = Species {
            special: SpecialAttack { kind: Some(SpecialAttackKind::SummonsSpecies12), ..default() },
            ..default()
        };
        let geometry = LevelDefinition {
            monsters: vec![MonsterPlacement {
                location: 1,
                group: 1,
                species: 1,
                position: Vec3::ZERO,
                yaw: 0.0,
                unborn: false,
            }],
            encounters: crate::content::level::Encounters {
                generators: vec![Generator {
                    table: Some(EncounterTable {
                        entries: vec![EncounterEntry { species: 3, ..default() }],
                        ..default()
                    }),
                    ..default()
                }],
                ..default()
            },
            ..default()
        };
        let mut bodies = MonsterBodies::default();
        let assets = app.world().resource::<AssetServer>().clone();
        bodies.for_level(&geometry, &MonsterCatalog(catalog), &assets);
        let mut readied: Vec<_> = bodies.0.keys().copied().collect();
        readied.sort();
        assert_eq!(readied, [1, 3, 12]);
    }
}
