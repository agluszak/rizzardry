//! Telling what monsters are (retail `DetectMonsterGroups`). A group shows
//! by its look ("Crocodiles") until a member who can see recognises it with
//! Mythology, helped by how many of its kind they have seen; then by its
//! name ("Ironskin Crocodiles"). Every group seen adds its members to each
//! seer's count of that kind, once.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;

use super::population::Monster;
use crate::content::characters::{Attribute, Skill};
use crate::content::species::MonsterCatalog;
use crate::game::characters::{Character, Condition};
use crate::game::combat::Battle;
use crate::game::data::GameData;
use crate::game::party::body::PartyCamera;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;

/// What the party knows of a level's monster groups, on the level root
/// (retail `alternate_name`, and `group_state[0]` for counted).
#[derive(Component, Clone, Debug, Default)]
pub struct MonsterGroups {
    identified: HashSet<i32>,
    counted: HashSet<i32>,
}

impl MonsterGroups {
    pub fn identified(&self, group: i32) -> bool {
        self.identified.contains(&group)
    }
}

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
pub fn recognition(member: &Character, species: u32, level: u8) -> i32 {
    let seen = member.monster_awareness.get(&species).copied().unwrap_or(0);
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
    mut levels: Query<&mut MonsterGroups>,
    monsters: Query<(Entity, &Monster, &GlobalTransform)>,
    camera: Query<(&GlobalTransform, &Projection), With<PartyCamera>>,
    battle: Option<Res<Battle>>,
    roster: Res<PartyRoster>,
    mut members: Query<&mut Character>,
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
    let (Ok(mut groups), Ok((eye, projection))) = (levels.single_mut(), camera.single()) else {
        return;
    };
    let Some(catalog) = data.and_then(|d| catalogs.get(&d.species)) else { return };
    let far = match projection {
        Projection::Perspective(p) => p.far,
        _ => return,
    };
    let (from, forward) = (eye.translation(), eye.forward().with_y(0.0).normalize_or_zero());
    // Each group's species, living members, and whether one is seen; in a
    // fight only the groups fighting count. Ordered by group for a
    // reproducible order of rolls.
    let mut seen: HashMap<i32, (u32, usize, bool)> = HashMap::new();
    for (entity, monster, at) in &monsters {
        if monster.is_dead() || battle.as_ref().is_some_and(|b| !b.fights(entity)) {
            continue;
        }
        let to = at.translation() - from;
        let in_view =
            to.length() < far && forward.angle_between(to.with_y(0.0)).to_degrees() < HALF_VIEW;
        let entry = seen.entry(monster.group).or_insert((monster.species, 0, false));
        entry.1 += 1;
        entry.2 |= in_view;
    }
    let mut order: Vec<i32> = seen.keys().copied().collect();
    order.sort_unstable();
    for group in order {
        let (species_id, count, in_view) = seen[&group];
        let known = groups.identified.contains(&group);
        if !in_view || (known && groups.counted.contains(&group)) {
            continue;
        }
        let Some(species) = catalog.0.get(species_id as usize) else { continue };
        if !known {
            // The member who beats their roll by the most recognises it:
            // (seen − level) × 3 + intelligence / 10 + Mythology.
            let mut best: Option<(String, i32)> = None;
            for &member in &roster.0 {
                let Ok(member) = members.get(member) else { continue };
                if !can_look(member) {
                    continue;
                }
                let score = recognition(member, species_id, species.level);
                let roll = rng.below(100) as i32;
                if roll < score && best.as_ref().is_none_or(|(_, margin)| score - roll > *margin) {
                    best = Some((member.name.clone(), score - roll));
                }
            }
            if let Some((member, _)) = best {
                news.write(Identified {
                    member,
                    looks_like: species.shown_name(false, count).to_owned(),
                    is: species.shown_name(true, count).to_owned(),
                    count,
                });
                groups.identified.insert(group);
            }
        }
        if groups.counted.insert(group) {
            for &member in &roster.0 {
                let Ok(mut member) = members.get_mut(member) else { continue };
                if can_look(&member) {
                    let seen = member.monster_awareness.entry(species_id).or_default();
                    *seen = seen.saturating_add(count.min(255) as u8);
                }
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
        let mut bishop = Character::new("Sage", Race::Human, Gender::Male, Profession::Bishop);
        bishop.attributes[Attribute::Intelligence] = 60;
        bishop.skill_points[Skill::Mythology] = 10;
        let mythology = bishop.skill(Skill::Mythology) as i32;
        // A level 8 kind never seen: 24 below, and 6 from Intelligence.
        assert_eq!(recognition(&bishop, 62, 8), mythology + 6 - 24);
        bishop.monster_awareness.insert(62, 10);
        assert_eq!(recognition(&bishop, 62, 8), mythology + 6 + 6);
    }
}
