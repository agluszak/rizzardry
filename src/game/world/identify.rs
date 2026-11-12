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
use crate::game::characters::{Identity, MonsterKnowledge, Vitals};
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::level::PlayedLevel;

/// Knowledge belongs to the identifiable group, not to parallel level tables,
/// and so does its stance toward the party ([`super::hostility`]).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct MonsterGroup {
    pub authored: i32,
    pub identified: bool,
    pub counted: bool,
    /// The stance last set on the group and when, if anything has set one.
    pub set: Option<super::hostility::SetStance>,
    /// A random encounter about the level, and the game second it came
    /// (retail `encounter_registered`, `spawn_time`).
    pub encounter: Option<u64>,
}

/// A companion group come along with a random encounter, which it goes
/// with: retired with it, but not killed with it (retail `leader_group_id`).
#[derive(Component, Debug)]
#[component(immutable)]
#[relationship(relationship_target = Escorts)]
pub struct EscortOf(pub Entity);

/// An encounter's companion groups, maintained by Bevy from [`EscortOf`].
/// Not `linked_spawn`: an encounter written off leaves its survivors.
#[derive(Component, Debug)]
#[relationship_target(relationship = EscortOf)]
pub struct Escorts(Vec<Entity>);

/// Membership is independent of level lifetime ownership. Replacing this
/// immutable relationship updates the group's reverse collection automatically.
#[derive(Component, Debug)]
#[component(immutable)]
#[relationship(relationship_target = GroupMembers)]
pub struct GroupMember(pub Entity);

#[derive(Component, Debug)]
#[relationship_target(relationship = GroupMember)]
pub struct GroupMembers(Vec<Entity>);

/// The member that leads a group (`docs/rules/monsters.md`, "A group's
/// leader"): the first to come into it, until it dies or goes; then the
/// fittest of the rest. Whoever asks for a group's leader asks which of
/// its members carries this.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[relationship(relationship_target = Leader)]
pub struct Leads(pub Entity);

/// A group's leader, maintained by Bevy from [`Leads`]: one at most, the
/// last to take the lead.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[relationship_target(relationship = Leads)]
pub struct Leader(Entity);

/// The first to come into a group leads it.
pub(super) fn first_leads(
    joined: On<Add<GroupMember>>,
    members: Query<&GroupMember>,
    mut commands: Commands,
) {
    let member = joined.entity;
    if let Ok(&GroupMember(group)) = members.get(member) {
        commands.queue(move |world: &mut World| {
            if world.get::<Leader>(group).is_none()
                && let Ok(mut member) = world.get_entity_mut(member)
            {
                member.insert(Leads(group));
            }
        });
    }
}

/// A group whose leader is gone takes a new one.
#[derive(SystemParam)]
pub(super) struct Succession<'w, 's> {
    groups: Query<'w, 's, &'static GroupMembers>,
    leading: Query<'w, 's, (), With<Leads>>,
    members: Query<'w, 's, &'static GroupMember>,
    fit: Query<
        'w,
        's,
        (&'static crate::game::actors::Level, &'static crate::game::characters::Conditions),
        Without<crate::game::characters::Dead>,
    >,
    scripts: Query<'w, 's, &'static super::bidden::Script>,
    commands: Commands<'w, 's>,
}

impl Succession<'_, '_> {
    /// If `gone` led its group, the living member of highest level that is
    /// not Turncoat or worse leads it now, the first of them among equals,
    /// or else the first still living; it takes up the script the old
    /// leader followed (retail `ElectGroupLeaderMember`).
    fn after(&mut self, gone: Entity) {
        let Ok(&GroupMember(group)) = self.members.get(gone) else { return };
        let Ok(members) = self.groups.get(group) else { return };
        if !self.leading.contains(gone) {
            return;
        }
        let living =
            || members.iter().filter(|&member| member != gone && self.fit.contains(member));
        let mut best: Option<(Entity, u32)> = None;
        for member in living() {
            let Ok((level, conditions)) = self.fit.get(member) else { continue };
            let turned = !conditions.below(crate::game::characters::Condition::Turncoat);
            if !turned && best.is_none_or(|(_, most)| level.0 > most) {
                best = Some((member, level.0));
            }
        }
        let Some(next) = best.map(|(member, _)| member).or_else(|| living().next()) else {
            return;
        };
        self.commands.entity(next).insert(Leads(group));
        if let Ok(script) = self.scripts.get(gone) {
            self.commands.entity(next).insert(script.clone());
        }
    }
}

/// A leader that dies is succeeded.
pub(super) fn succeed_the_dead(
    died: On<Add<crate::game::characters::Dead>>,
    mut succession: Succession,
) {
    succession.after(died.entity);
}

/// A leader that leaves its group, or is gone from the world, is succeeded.
pub(super) fn succeed_the_gone(left: On<Remove<GroupMember>>, mut succession: Succession) {
    succession.after(left.entity);
}

/// "Ragnar identifies the Crocodiles:", "They're Ironskin Crocodiles!"
#[derive(Message, Debug, Clone, PartialEq)]
pub struct Identified {
    pub member: String,
    pub looks_like: String,
    pub is: String,
    pub count: usize,
}

/// How likely, in percent, a member is to recognise a species of `level`:
/// three for each of its kind they have seen beyond its level, a tenth of
/// their Intelligence, and their Mythology.
pub fn recognition(
    skills: &crate::game::actors::Skills,
    attributes: &crate::game::actors::Attributes,
    knowledge: &MonsterKnowledge,
    species: u32,
    level: u8,
) -> i32 {
    let seen = knowledge.seen.get(&species).copied().unwrap_or(0);
    (seen as i32 - i32::from(level)) * 3
        + attributes[Attribute::Intelligence] as i32 / 10
        + skills[Skill::Mythology] as i32
}

/// What the party can tell of a monster's health (retail
/// `FormatMonsterHealth`): its best Mythology, less three for each level the
/// monster shows above the party's average, tells the current health from
/// 10 and the most from 5; otherwise "?" ("12/20", "?/20", "?/?"). The
/// party is in roster order, each member's level with whether they can act
/// and whether they are a recruited NPC.
pub fn health_known(
    party: &[(u8, &crate::game::actors::Skills, bool, bool)],
    health: crate::game::characters::Pool,
    display_level: u8,
) -> String {
    // The best Mythology among those who can act (retail
    // `GetBestPartySkillLevel`).
    let mythology = party
        .iter()
        .filter(|(_, _, can_act, _)| *can_act)
        .map(|(_, skills, ..)| skills[Skill::Mythology])
        .max()
        .unwrap_or(0);
    let average = crate::game::party::mean_level(crate::game::party::averaged_places(
        party.iter().map(|&(level, _, _, npc)| (level, npc)),
    ));
    let level = i32::from(display_level);
    let knowledge = if (average as i32) < level {
        (mythology as f32 - (level as f32 - average) * 3.0 + 0.5).max(0.0) as u32
    } else {
        mythology
    };
    let current = if knowledge >= 10 { health.current.to_string() } else { "?".to_owned() };
    let most = if knowledge > 4 { health.max.to_string() } else { "?".to_owned() };
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
    monsters: Query<
        'w,
        's,
        (
            (&'static Monster, &'static Vitals),
            (&'static crate::game::actors::Level, Option<&'static super::population::Camouflage>),
            ActorPose,
            Option<&'static super::navigation::Walker>,
        ),
    >,
    level: PlayedLevel<'w, 's>,
    roster: Single<'w, 's, &'static PartyRoster>,
    members: Query<
        'w,
        's,
        (
            (&'static crate::game::actors::Traits, &'static Identity),
            (
                &'static crate::game::characters::Conditions,
                Has<crate::game::actors::CanFight>,
                Has<crate::game::actors::Lucid>,
            ),
            &'static crate::game::actors::Attributes,
            &'static crate::game::actors::Skills,
            &'static mut MonsterKnowledge,
        ),
    >,
    content: If<Res<'w, crate::game::data::Catalogs>>,
    obstacles: Option<super::obstacles::WorldObstacles<'w, 's>>,
    pub(crate) factions: ResMut<'w, crate::game::factions::Factions>,
    news: MessageWriter<'w, Identified>,
}

impl GroupRecognition<'_, '_> {
    pub fn detect(
        &mut self,
        from: Vec3,
        forward: Vec3,
        combatants: Option<&[Entity]>,
        shadow_hound: bool,
        searching: bool,
        rng: &mut GameRng,
    ) {
        let (Some(level), Some(far)) = (self.level.entity(), self.level.view_distance()) else {
            return;
        };
        let catalog = &self.content.species;
        let Some(obstacles) = self.obstacles.as_ref() else { return };
        let view = super::perception::View { eye: from, forward, far, searching };
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
                let ((monster, vitals), (level, camouflage), at, walker) =
                    monsters.get(member).unwrap();
                if vitals.is_dead() || combatants.is_some_and(|c| !c.contains(&member)) {
                    continue;
                }
                species_id.get_or_insert(monster.species);
                count += 1;
                in_view |= view.sees(
                    members
                        .iter_many(roster.iter())
                        .matched()
                        .filter(|(_, (_, can_fight, _), ..)| *can_fight)
                        .map(|((traits, _), (conditions, ..), attributes, skills, _)| {
                            (traits, conditions, attributes, skills)
                        }),
                    shadow_hound,
                    (monster, vitals),
                    (*level, camouflage.copied()),
                    super::navigation::feet(at.translation(), walker),
                    combatants.is_some(),
                    obstacles,
                );
            }
            if !in_view {
                continue;
            }
            let Some(species_id) = species_id else { continue };
            let species = &catalog[species_id];
            if members.iter_many(roster.iter()).matched().any(|(_, (.., lucid), ..)| lucid) {
                self.factions.encounter(species.faction);
            }
            if !known {
                let mut best: Option<(String, i32)> = None;
                for ((_, identity), (.., lucid), attributes, skills, knowledge) in
                    members.iter_many(roster.iter()).matched()
                {
                    if !lucid {
                        continue;
                    }
                    let score =
                        recognition(skills, attributes, knowledge, species_id, species.level);
                    let roll = rng.below(100) as i32;
                    if roll < score
                        && best.as_ref().is_none_or(|(_, margin)| score - roll > *margin)
                    {
                        best = Some((identity.name.clone(), score - roll));
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
                for (_, (.., lucid), _, _, mut knowledge) in
                    members.iter_many_unique_mut(roster.iter()).matched()
                {
                    if !lucid {
                        continue;
                    }
                    let seen = knowledge.seen.entry(species_id).or_default();
                    *seen += count as u32;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::actors::Level;
    use crate::game::characters::{Condition, Conditions, Dead, Lasting};
    use crate::game::world::bidden::Script;

    /// Retail `ElectGroupLeaderMember`: the first to come leads; when it
    /// dies the living member of highest level that is not Turncoat leads,
    /// and follows the script the dead leader followed.
    #[test]
    fn a_dead_leader_is_succeeded_by_the_highest_fit_member() {
        let mut world = World::new();
        world.add_observer(first_leads);
        world.add_observer(succeed_the_dead);
        let group = world.spawn(MonsterGroup::default()).id();
        let member = |world: &mut World, level| {
            world.spawn((Vitals::full(10, 10), Level(level), GroupMember(group))).id()
        };
        let first = member(&mut world, 3);
        world.entity_mut(first).insert(Script("patrol.msf".into()));
        let low = member(&mut world, 2);
        let high = member(&mut world, 9);
        let turned = member(&mut world, 12);
        world
            .get_mut::<Conditions>(turned)
            .unwrap()
            .afflict(Condition::Turncoat, Lasting::Rounds(3));
        let leads = |world: &World, member| world.get::<Leads>(member).is_some();
        assert!(leads(&world, first));
        world.entity_mut(low).insert(Dead::default());
        assert!(leads(&world, first), "not the leader's death");
        world.entity_mut(first).insert(Dead::default());
        assert!(leads(&world, high) && !leads(&world, first));
        assert_eq!(world.get::<Script>(high), Some(&Script("patrol.msf".into())));
        // With only the turned one left, it leads all the same.
        world.entity_mut(high).insert(Dead::default());
        assert!(leads(&world, turned));
    }
}
