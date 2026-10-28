//! Telling what monsters are (retail `DetectMonsterGroups`). A group shows
//! by its look ("Crocodiles") until a member who can see recognises it with
//! Mythology, helped by how many of its kind they have seen; then by its
//! name ("Ironskin Crocodiles"). Every group seen adds its members to each
//! seer's count of that kind, once.

use crate::game::world::ActorPose;
use crate::game::world::level::LevelOwned;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::population::Monster;
use crate::content::characters::{Attribute, Skill};
use crate::game::characters::{Character, Condition, MonsterKnowledge, Vitals};
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::level::PlayedLevel;

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
fn can_look(member: &Vitals) -> bool {
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
pub fn health_known(
    party: &[(&Character, &Vitals)],
    hp: u32,
    max_hp: u32,
    display_level: u8,
) -> String {
    let mythology = party
        .iter()
        .filter(|(_, v)| v.can_act())
        .map(|(m, _)| m.skill(Skill::Mythology))
        .max()
        .unwrap_or(0);
    let average =
        party.iter().map(|(m, _)| f32::from(m.level)).sum::<f32>() / party.len().max(1) as f32;
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

/// The same recognition operation is used by clock ticks and battle boundaries.
/// It borrows only actual affected group/knowledge components, in authored order.
#[derive(SystemParam)]
pub struct GroupRecognition<'w, 's> {
    groups: Query<
        'w,
        's,
        (Entity, &'static mut MonsterGroup, Option<&'static GroupMembers>, &'static LevelOwned),
    >,
    monsters: Query<'w, 's, (&'static Monster, ActorPose)>,
    level: PlayedLevel<'w, 's>,
    roster: Single<'w, 's, &'static PartyRoster>,
    members: Query<'w, 's, (&'static Character, &'static Vitals, &'static mut MonsterKnowledge)>,
    content: crate::game::data::Content<'w>,
    pub(crate) factions: ResMut<'w, crate::game::factions::Factions>,
    news: MessageWriter<'w, Identified>,
}

impl GroupRecognition<'_, '_> {
    pub fn detect(
        &mut self,
        from: Vec3,
        forward: Vec3,
        combatants: Option<&[Entity]>,
        rng: &mut GameRng,
    ) {
        let (Some(level), Some(far)) = (self.level.entity(), self.level.view_distance()) else {
            return;
        };
        let Some(catalog) = self.content.species() else {
            return;
        };
        let (groups, monsters, roster, members, news) =
            (&mut self.groups, &self.monsters, &self.roster, &mut self.members, &mut self.news);
        // Authored group order determines gameplay RNG use. Membership itself is
        // maintained by Bevy, without rebuilding a second population map.
        let mut order: Vec<_> = groups
            .iter()
            .filter(|(_, _, _, owner)| owner.0 == level)
            .map(|(entity, group, _, _)| (group.authored, entity))
            .collect();
        order.sort_unstable_by_key(|&(authored, _)| authored);
        for (_, entity) in order {
            let (_, mut group, Some(membership), _) = groups.get_mut(entity).unwrap() else {
                continue;
            };
            let (known, counted) = (group.identified, group.counted);
            if known && counted {
                continue;
            }
            let (mut species_id, mut count, mut in_view) = (None, 0usize, false);
            for member in membership.iter() {
                let Ok((monster, at)) = monsters.get(member) else { continue };
                if monster.is_dead() || combatants.is_some_and(|c| !c.contains(&member)) {
                    continue;
                }
                species_id.get_or_insert(monster.species);
                count += 1;
                let to = at.translation() - from;
                in_view |= to.length() < far
                    && forward.angle_between(to.with_y(0.0)).to_degrees() < HALF_VIEW;
            }
            if !in_view {
                continue;
            }
            let Some(species_id) = species_id else { continue };
            let Some(species) = catalog.0.get(species_id as usize) else { continue };
            if members.iter_many(roster.iter()).matched().any(|(_, vitals, _)| can_look(vitals)) {
                self.factions.encounter(species.faction);
            }
            if !known {
                let mut best: Option<(String, i32)> = None;
                for (profile, vitals, knowledge) in members.iter_many(roster.iter()).matched() {
                    if !can_look(vitals) {
                        continue;
                    }
                    let score = recognition(profile, knowledge, species_id, species.level);
                    let roll = rng.below(100) as i32;
                    if roll < score
                        && best.as_ref().is_none_or(|(_, margin)| score - roll > *margin)
                    {
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
                    group.identified = true;
                }
            }
            if !counted {
                group.counted = true;
                for (_, vitals, mut knowledge) in
                    members.iter_many_unique_mut(roster.iter()).matched()
                {
                    if !can_look(vitals) {
                        continue;
                    }
                    let seen = knowledge.seen.entry(species_id).or_default();
                    *seen = seen.saturating_add(count.min(255) as u8);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::species::MonsterCatalog;
    use crate::game::characters::{Gender, Profession, Race};
    use crate::game::data::GameData;
    use crate::game::party::body::PartyBody;
    use avian3d::prelude::Position;

    #[test]
    fn mythology_tells_a_monsters_health() {
        let mut sage = Character::new("Sage", Race::Human, Gender::Male, Profession::Bishop);
        let vitals = Vitals::new(&sage);
        sage.skill_points[Skill::Mythology] = 0;
        assert_eq!(health_known(&[(&sage, &vitals)], 7, 20, 1), "?/?");
        sage.skill_points[Skill::Mythology] = 6;
        let known = sage.skill(Skill::Mythology);
        assert!(known > 4 && known < 10, "{known}");
        assert_eq!(health_known(&[(&sage, &vitals)], 7, 20, 1), "?/20");
        sage.skill_points[Skill::Mythology] = 12;
        assert_eq!(health_known(&[(&sage, &vitals)], 7, 20, 1), "7/20");
        // Three levels above the party's average cost nine points.
        let left = sage.skill(Skill::Mythology) - 9;
        let expected = if left >= 10 {
            "7/20"
        } else if left > 4 {
            "?/20"
        } else {
            "?/?"
        };
        assert_eq!(health_known(&[(&sage, &vitals)], 7, 20, 4), expected);
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
    fn recognition_uses_authored_visibility_and_excludes_nonmembers() {
        use crate::content::level::LevelDefinition;
        use crate::content::species::Species;
        use crate::game::world::level::Level;
        use crate::game::world::population::ScaledBody;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<crate::game::factions::Factions>()
            .init_resource::<Assets<LevelDefinition>>()
            .init_resource::<Assets<MonsterCatalog>>()
            .insert_resource(GameRng::seeded(4))
            .add_message::<Identified>()
            .add_systems(
                Update,
                |party: Query<(&Position, &PartyBody)>,
                 mut recognition: GroupRecognition,
                 mut rng: ResMut<GameRng>| {
                    let (at, body) = party.single().unwrap();
                    recognition.detect(PartyBody::eye(at), body.forward(), None, &mut rng);
                },
            );
        let geometry = app
            .world_mut()
            .resource_mut::<Assets<LevelDefinition>>()
            .add(LevelDefinition { sight_distance: Some(5.0), ..default() });
        let species =
            app.world_mut().resource_mut::<Assets<MonsterCatalog>>().add(MonsterCatalog(vec![
                Species {
                    name: "Crab".into(),
                    generic_name: "Crustacean".into(),
                    faction: 4,
                    ..default()
                },
            ]));
        app.insert_resource(GameData {
            items: default(),
            loot: default(),
            species,
            spells: default(),
        });
        let root = app.world_mut().spawn(Level::test(geometry.clone())).id();
        let group = app
            .world_mut()
            .spawn((MonsterGroup { authored: 7, ..default() }, LevelOwned(root)))
            .id();
        app.world_mut().spawn((
            Monster {
                enchantments: default(),
                location: 1,
                species: 0,
                hp: 10,
                max_hp: 10,
                stamina: 0,
                max_stamina: 0,
                mana: 0,
                conditions: vec![],
                body: ScaledBody { scale: 1.0, radius: 1.0, eye_height: 1.0, speed: 0.0 },
            },
            Transform::from_translation(Vec3::splat(1000.0)),
            GlobalTransform::from_translation(Vec3::splat(2000.0)),
            Position(Vec3::new(0.0, 0.0, -10.0)),
            GroupMember(group),
            LevelOwned(root),
        ));
        app.world_mut().spawn((PartyBody::default(), Position::default()));
        let mut sage = Character::new("Sage", Race::Human, Gender::Male, Profession::Bishop);
        sage.skill_points[Skill::Mythology] = 125;
        let member = app.world_mut().spawn((Vitals::new(&sage), sage.clone())).id();
        let outsider = app.world_mut().spawn((Vitals::new(&sage), sage)).id();
        crate::game::party::roster::test_party(app.world_mut(), &[member]);
        app.finish();
        app.cleanup();
        app.update();
        app.world_mut().clear_trackers();
        app.world_mut().run_schedule(Update);
        assert!(
            !app.world().get::<MonsterGroup>(group).unwrap().counted,
            "beyond the level's sight distance"
        );
        assert!(app.world().get::<MonsterKnowledge>(member).unwrap().seen.is_empty());
        assert!(!app.world().resource::<crate::game::factions::Factions>().encountered(4));
        app.world_mut()
            .resource_mut::<Assets<LevelDefinition>>()
            .get_mut(&geometry)
            .unwrap()
            .sight_distance = Some(20.0);
        app.world_mut().run_schedule(Update);
        let known = app.world().get::<MonsterGroup>(group).unwrap();
        assert!(known.identified && known.counted);
        assert!(app.world().resource::<crate::game::factions::Factions>().encountered(4));
        assert_eq!(app.world().get::<MonsterKnowledge>(member).unwrap().seen.get(&0), Some(&1));
        assert!(app.world().get::<MonsterKnowledge>(outsider).unwrap().seen.is_empty());
        // Aging and recognition settle in the same operation: blindness
        // expires during this interval and recognition can then succeed.
        app.world_mut().get_mut::<MonsterGroup>(group).unwrap().identified = false;
        let catalog = app.world().resource::<GameData>().species.clone();
        app.world_mut().resource_mut::<Assets<MonsterCatalog>>().get_mut(&catalog).unwrap().0[0]
            .level = 1;
        app.world_mut()
            .get_mut::<Vitals>(member)
            .unwrap()
            .inflict(Condition::Blind, crate::game::characters::Lasting::Rounds(2));
        let body = app
            .world_mut()
            .query_filtered::<Entity, With<PartyBody>>()
            .single(app.world())
            .unwrap();
        app.world_mut().entity_mut(body).insert(avian3d::prelude::LinearVelocity::ZERO);
        app.init_resource::<crate::game::time::GameClock>();
        app.init_resource::<Assets<crate::content::spells::SpellCatalog>>();
        app.add_message::<crate::game::camp::CampNews>();
        app.world_mut().resource_mut::<Time>().advance_by(std::time::Duration::from_secs(30));
        use bevy::ecs::system::RunSystemOnce;
        app.world_mut().run_system_once(crate::game::time::pass_time).unwrap();
        assert!(!app.world().get::<Vitals>(member).unwrap().has(Condition::Blind));
        assert!(app.world().get::<MonsterGroup>(group).unwrap().identified);
    }
}
