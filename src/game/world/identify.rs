//! Telling what monsters are (retail `DetectMonsterGroups`). A group shows
//! by its look ("Crocodiles") until a member who can see recognises it with
//! Mythology, helped by how many of its kind they have seen; then by its
//! name ("Ironskin Crocodiles"). Every group seen adds its members to each
//! seer's count of that kind, once.

use bevy::prelude::*;

use super::population::Monster;
use crate::content::characters::{Attribute, Skill};
use crate::content::species::MonsterCatalog;
use crate::game::characters::{Character, Condition, MonsterKnowledge};
use crate::game::combat::Battle;
use crate::game::data::GameData;
use crate::game::party::body::PartyBody;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::level::PlayedLevel;
use avian3d::prelude::Position;

/// Knowledge belongs to the identifiable group, not to parallel level tables.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct MonsterGroup {
    pub authored: i32,
    pub identified: bool,
    pub counted: bool,
}

/// Membership is independent of level lifetime ownership. Replacing this
/// immutable relationship updates the group's reverse collection automatically.
#[derive(Component, Debug)]
#[component(immutable)]
#[relationship(relationship_target = GroupMembers)]
pub struct GroupMember(pub Entity);

#[derive(Component, Debug)]
#[relationship_target(relationship = GroupMember)]
pub struct GroupMembers(Vec<Entity>);

/// Look the monsters over: as game time passes (each 120-second aging
/// tick), when a fight begins and each round (retail
/// `AdvanceTimedEffects`, `InitializeCombat`, `AdvanceCombatRound`).
#[derive(Message, Debug, Clone, Copy)]
pub struct DetectGroups;

/// "Ragnar identifies the Crocodiles:", "They're Ironskin Crocodiles!"
#[derive(Message, Debug, Clone, PartialEq)]
pub struct Identified {
    pub member: String,
    pub looks_like: String,
    pub is: String,
    pub count: usize,
}

/// Retail's base horizontal field of view; a group is seen when a member
/// is within it and the far clip (retail asks whether a member is drawn).
const HALF_VIEW: f32 = 85.0 / 2.0;

/// Whether a member can look (retail: alive, not insane or worse, not
/// blind).
fn can_look(member: &Character) -> bool {
    !member.is_dead()
        && member.worst_condition().is_none_or(|c| c < Condition::Insane)
        && !member.has(Condition::Blind)
}

/// How likely, in percent, a member is to recognise a species of `level`:
/// three for each of its kind they have seen beyond its level, a tenth of
/// their Intelligence, and their Mythology.
pub fn recognition(
    member: &Character,
    knowledge: &MonsterKnowledge,
    species: u32,
    level: u8,
) -> i32 {
    let seen = knowledge.seen.get(&species).copied().unwrap_or(0);
    (i32::from(seen) - i32::from(level)) * 3
        + member.attribute(Attribute::Intelligence) as i32 / 10
        + member.skill(Skill::Mythology) as i32
}

/// What the party can tell of a monster's health (retail
/// `FormatMonsterHealth`): its best Mythology, less three for each level the
/// monster shows above the party's average, tells the current health from
/// 10 and the most from 5; otherwise "?" ("12/20", "?/20", "?/?").
pub fn health_known(party: &[&Character], hp: u32, max_hp: u32, display_level: u8) -> String {
    let mythology = party
        .iter()
        .filter(|m| crate::game::camp::can_act(m))
        .map(|m| m.skill(Skill::Mythology))
        .max()
        .unwrap_or(0);
    let average = party.iter().map(|m| f32::from(m.level)).sum::<f32>() / party.len().max(1) as f32;
    let level = i32::from(display_level);
    let knowledge = if (average as i32) < level {
        (mythology as f32 - (level as f32 - average) * 3.0 + 0.5).max(0.0) as u32
    } else {
        mythology
    };
    let current = if knowledge >= 10 { hp.to_string() } else { "?".to_owned() };
    let most = if knowledge > 4 { max_hp.to_string() } else { "?".to_owned() };
    format!("{current}/{most}")
}

pub fn detect_monster_groups(
    mut requests: MessageReader<DetectGroups>,
    mut groups: Query<(Entity, &mut MonsterGroup, Option<&GroupMembers>)>,
    monsters: Query<(&Monster, &GlobalTransform)>,
    party: Query<(&Position, &PartyBody)>,
    level: PlayedLevel,
    battle: Option<Res<Battle>>,
    roster: Res<PartyRoster>,
    mut members: Query<(&Character, &mut MonsterKnowledge)>,
    (data, catalogs, mut rng): (
        Option<Res<GameData>>,
        Res<Assets<MonsterCatalog>>,
        ResMut<GameRng>,
    ),
    mut news: MessageWriter<Identified>,
) {
    if requests.read().count() == 0 {
        return;
    }
    let (Ok((position, body)), Some(far)) = (party.single(), level.view_distance()) else { return };
    let Some(catalog) = data.and_then(|d| catalogs.get(&d.species)) else { return };
    let (from, forward) = (PartyBody::eye(position), body.forward());
    // Authored group order determines gameplay RNG use. Membership itself is
    // maintained by Bevy, without rebuilding a second population map.
    let mut order: Vec<_> =
        groups.iter().map(|(entity, group, _)| (group.authored, entity)).collect();
    order.sort_unstable_by_key(|&(authored, _)| authored);
    for (_, entity) in order {
        let (_, group, Some(membership)) = groups.get(entity).unwrap() else { continue };
        let (known, counted) = (group.identified, group.counted);
        if known && counted {
            continue;
        }
        let (mut species_id, mut count, mut in_view) = (None, 0usize, false);
        for member in membership.iter() {
            let Ok((monster, at)) = monsters.get(member) else { continue };
            if monster.is_dead() || battle.as_ref().is_some_and(|b| !b.fights(member)) {
                continue;
            }
            species_id.get_or_insert(monster.species);
            count += 1;
            let to = at.translation() - from;
            in_view |=
                to.length() < far && forward.angle_between(to.with_y(0.0)).to_degrees() < HALF_VIEW;
        }
        if !in_view {
            continue;
        }
        let Some(species_id) = species_id else { continue };
        let Some(species) = catalog.0.get(species_id as usize) else { continue };
        if !known {
            let mut best: Option<(String, i32)> = None;
            for &member in &roster.0 {
                let Ok((profile, knowledge)) = members.get(member) else { continue };
                if !can_look(profile) {
                    continue;
                }
                let score = recognition(profile, knowledge, species_id, species.level);
                let roll = rng.below(100) as i32;
                if roll < score && best.as_ref().is_none_or(|(_, margin)| score - roll > *margin) {
                    best = Some((profile.name.clone(), score - roll));
                }
            }
            if let Some((member, _)) = best {
                news.write(Identified {
                    member,
                    looks_like: species.shown_name(false, count).to_owned(),
                    is: species.shown_name(true, count).to_owned(),
                    count,
                });
                groups.get_mut(entity).unwrap().1.identified = true;
            }
        }
        if !counted {
            groups.get_mut(entity).unwrap().1.counted = true;
            for &member in &roster.0 {
                let Ok((profile, _)) = members.get(member) else { continue };
                if !can_look(profile) {
                    continue;
                }
                let (_, mut knowledge) = members.get_mut(member).unwrap();
                let seen = knowledge.seen.entry(species_id).or_default();
                *seen = seen.saturating_add(count.min(255) as u8);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::characters::{Gender, Profession, Race};

    #[test]
    fn mythology_tells_a_monsters_health() {
        let mut sage = Character::new("Sage", Race::Human, Gender::Male, Profession::Bishop);
        sage.skill_points[Skill::Mythology] = 0;
        assert_eq!(health_known(&[&sage], 7, 20, 1), "?/?");
        sage.skill_points[Skill::Mythology] = 6;
        let known = sage.skill(Skill::Mythology);
        assert!(known > 4 && known < 10, "{known}");
        assert_eq!(health_known(&[&sage], 7, 20, 1), "?/20");
        sage.skill_points[Skill::Mythology] = 12;
        assert_eq!(health_known(&[&sage], 7, 20, 1), "7/20");
        // Three levels above the party's average cost nine points.
        let left = sage.skill(Skill::Mythology) - 9;
        let expected = if left >= 10 {
            "7/20"
        } else if left > 4 {
            "?/20"
        } else {
            "?/?"
        };
        assert_eq!(health_known(&[&sage], 7, 20, 4), expected);
    }

    #[test]
    fn seeing_more_of_a_kind_makes_it_easier_to_recognise() {
        let mut knowledge = MonsterKnowledge::default();
        let mut bishop = Character::new("Sage", Race::Human, Gender::Male, Profession::Bishop);
        bishop.attributes[Attribute::Intelligence] = 60;
        bishop.skill_points[Skill::Mythology] = 10;
        let mythology = bishop.skill(Skill::Mythology) as i32;
        // A level 8 kind never seen: 24 below, and 6 from Intelligence.
        assert_eq!(recognition(&bishop, &knowledge, 62, 8), mythology + 6 - 24);
        knowledge.seen.insert(62, 10);
        assert_eq!(recognition(&bishop, &knowledge, 62, 8), mythology + 6 + 6);
    }
    #[test]
    fn membership_updates_automatically_without_owning_member_lifetimes() {
        let mut world = World::new();
        let level = world.spawn_empty().id();
        let first = world.spawn((MonsterGroup { authored: 1, ..default() }, ChildOf(level))).id();
        let second = world.spawn((MonsterGroup { authored: 2, ..default() }, ChildOf(level))).id();
        let member = world.spawn((GroupMember(first), ChildOf(level))).id();
        assert_eq!(world.get::<GroupMembers>(first).unwrap().iter().collect::<Vec<_>>(), [member]);
        world.entity_mut(member).insert(GroupMember(second));
        assert!(world.get::<GroupMembers>(first).is_none_or(|m| m.iter().next().is_none()));
        assert_eq!(world.get::<GroupMembers>(second).unwrap().iter().collect::<Vec<_>>(), [member]);
        world.despawn(second);
        assert!(world.get_entity(member).is_ok());
        assert_eq!(world.get::<ChildOf>(member).unwrap().parent(), level);
        assert!(world.get::<GroupMember>(member).is_none());
        world.despawn(level);
        assert!(world.get_entity(member).is_err());
        assert!(world.get_entity(first).is_err());
    }

    #[test]
    fn recognition_uses_authored_visibility_and_changes_only_group_and_actor_knowledge() {
        use crate::content::level::{LevelEnvironment, LevelGeometry};
        use crate::content::species::Species;
        use crate::game::world::level::Level;
        use crate::game::world::population::ScaledBody;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<LevelGeometry>>()
            .init_resource::<Assets<MonsterCatalog>>()
            .init_resource::<PartyRoster>()
            .insert_resource(GameRng::seeded(4))
            .add_message::<DetectGroups>()
            .add_message::<Identified>()
            .add_systems(Update, detect_monster_groups);
        let geometry = app.world_mut().resource_mut::<Assets<LevelGeometry>>().add(LevelGeometry {
            environment: Some(LevelEnvironment {
                fog_range: (0.0, 1.0),
                view_distance: 5.0,
                intensity: 0.0,
                fog_colours: vec![Vec3::ZERO; 256],
                ambient_colours: vec![Vec3::ZERO; 256],
            }),
            ..default()
        });
        let species =
            app.world_mut().resource_mut::<Assets<MonsterCatalog>>().add(MonsterCatalog(vec![
                Species { name: "Crab".into(), generic_name: "Crustacean".into(), ..default() },
            ]));
        app.insert_resource(GameData { items: default(), loot: default(), species });
        let root = app
            .world_mut()
            .spawn(Level {
                path: "test".into(),
                geometry: geometry.clone(),
                navmeshes: vec![],
                sky: None,
                script: default(),
                messages: None,
            })
            .id();
        let group =
            app.world_mut().spawn((MonsterGroup { authored: 7, ..default() }, ChildOf(root))).id();
        app.world_mut().spawn((
            Monster {
                location: 1,
                species: 0,
                hp: 10,
                max_hp: 10,
                stamina: 0,
                max_stamina: 0,
                conditions: vec![],
                body: ScaledBody { scale: 1.0, radius: 1.0, eye_height: 1.0, speed: 0.0 },
            },
            GlobalTransform::from_translation(Vec3::new(0.0, 0.0, -10.0)),
            GroupMember(group),
            ChildOf(root),
        ));
        app.world_mut().spawn((PartyBody::default(), Position::default()));
        let mut sage = Character::new("Sage", Race::Human, Gender::Male, Profession::Bishop);
        sage.skill_points[Skill::Mythology] = 125;
        let member = app.world_mut().spawn(sage.clone()).id();
        let outsider = app.world_mut().spawn(sage).id();
        app.world_mut().resource_mut::<PartyRoster>().0.insert(member);
        let camera = app
            .world_mut()
            .spawn((
                crate::presentation::party_view::PartyCamera,
                Projection::Perspective(PerspectiveProjection { far: 1000.0, ..default() }),
                GlobalTransform::from_translation(Vec3::splat(1000.0)),
            ))
            .id();
        app.finish();
        app.cleanup();
        app.update();
        app.world_mut().clear_trackers();
        app.world_mut().write_message(DetectGroups);
        app.world_mut().run_schedule(Update);
        assert!(
            !app.world().get::<MonsterGroup>(group).unwrap().counted,
            "the rendering far plane does not make the group visible"
        );
        assert!(app.world().get::<MonsterKnowledge>(member).unwrap().seen.is_empty());
        app.world_mut()
            .resource_mut::<Assets<LevelGeometry>>()
            .get_mut(&geometry)
            .unwrap()
            .environment
            .as_mut()
            .unwrap()
            .view_distance = 20.0;
        app.world_mut()
            .entity_mut(camera)
            .insert(Projection::Perspective(PerspectiveProjection { far: 1.0, ..default() }));
        app.world_mut().write_message(DetectGroups);
        app.world_mut().run_schedule(Update);
        let known = app.world().get::<MonsterGroup>(group).unwrap();
        assert!(known.identified && known.counted);
        assert_eq!(app.world().get::<MonsterKnowledge>(member).unwrap().seen.get(&0), Some(&1));
        assert!(app.world().get::<MonsterKnowledge>(outsider).unwrap().seen.is_empty());
        let world = app.world_mut();
        assert_eq!(world.query_filtered::<Entity, Changed<Character>>().iter(world).count(), 0);
        world.clear_trackers();
        world.write_message(DetectGroups);
        world.run_schedule(Update);
        assert_eq!(world.query_filtered::<Entity, Changed<MonsterGroup>>().iter(world).count(), 0);
        assert_eq!(
            world.query_filtered::<Entity, Changed<MonsterKnowledge>>().iter(world).count(),
            0
        );
    }
}
