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
use crate::game::characters::{Character, MonsterKnowledge, Vitals};
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
    /// A companion group come along with the random encounter of this
    /// group id, which it goes with (retail `leader_group_id`).
    pub escort_of: Option<i32>,
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

/// How likely, in percent, a member is to recognise a species of `level`:
/// three for each of its kind they have seen beyond its level, a tenth of
/// their Intelligence, and their Mythology.
pub fn recognition(
    member: &crate::game::characters::CharacterStats<'_>,
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
/// 10 and the most from 5; otherwise "?" ("12/20", "?/20", "?/?"). The
/// party is in roster order, each member with whether it is a recruited NPC.
pub fn health_known(
    party: &[(&Character, &Vitals, bool)],
    hp: u32,
    max_hp: u32,
    display_level: u8,
) -> String {
    // The best Mythology among those who can act (retail
    // `GetBestPartySkillLevel`).
    let mythology = party
        .iter()
        .filter(|(_, vitals, _)| vitals.can_act())
        .map(|(character, ..)| character.skill(Skill::Mythology))
        .max()
        .unwrap_or(0);
    let average = crate::game::party::mean_level(
        crate::game::party::averaged_places(party.iter().map(|&(m, _, npc)| (m, npc)))
            .into_iter()
            .map(Character::level),
    );
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
    monsters:
        Query<'w, 's, (&'static Monster, ActorPose, Option<&'static super::navigation::Walker>)>,
    level: PlayedLevel<'w, 's>,
    roster: Single<'w, 's, &'static PartyRoster>,
    members: Query<'w, 's, (&'static Character, &'static Vitals, &'static mut MonsterKnowledge)>,
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
                let (monster, at, walker) = monsters.get(member).unwrap();
                if monster.is_dead() || combatants.is_some_and(|c| !c.contains(&member)) {
                    continue;
                }
                species_id.get_or_insert(monster.species);
                count += 1;
                let species = &catalog[monster.species];
                in_view |= view.sees(
                    members
                        .iter_many(roster.iter())
                        .matched()
                        .map(|(profile, vitals, _)| (profile, vitals)),
                    shadow_hound,
                    monster,
                    species,
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
            if members.iter_many(roster.iter()).matched().any(|(_, vitals, _)| vitals.lucid()) {
                self.factions.encounter(species.faction);
            }
            if !known {
                let mut best: Option<(String, i32)> = None;
                for (profile, vitals, knowledge) in members.iter_many(roster.iter()).matched() {
                    if !vitals.lucid() {
                        continue;
                    }
                    let stats = profile.stats(&vitals.enchantments);
                    let score = recognition(&stats, knowledge, species_id, species.level);
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
                    if !vitals.lucid() {
                        continue;
                    }
                    let seen = knowledge.seen.entry(species_id).or_default();
                    *seen = seen.saturating_add(count.min(255) as u8);
                }
            }
        }
    }
}
