//! A monster choosing a spell (`docs/rules/monsters.md` section 5): which of
//! its ten slots it may cast, at whom each may be aimed, whether it casts,
//! which spell, and how hard.
//!
//! A monster spends nothing to cast: its spell budget stands in for a caster's
//! skill in the failure roll, and the power it picks is the most that budget
//! comfortably covers.

use bevy::prelude::Entity;

use super::close::{closes_in, reaches_party};
use super::sight::{Fray, MindItem, Sight};
use crate::content::spells::{MAX_POWER, SpellDefinition, SpellId, Spellbook, Targeting};
use crate::game::characters::Condition;
use crate::game::data::Catalogs;
use crate::game::hits::effects::BodyReadOnlyItem;
use crate::game::magic::reach::Aim;
use crate::game::magic::roll::spell_failure;
use crate::game::rng::{GameRng, weighted};
use crate::game::world::hostility::Stance;

const SLOT_WEIGHTS: [u32; 10] = [5, 5, 5, 10, 10, 10, 10, 15, 15, 15];

/// Whether the spell may be cast by a monster at all (retail
/// `IsSpellUsableByMonster`): monsters may cast it, the game resolves its
/// effect, it is not aimed at the party, a point, an item or a lock, and a
/// silence does not stop it (an Alchemy spell by a kind that keeps its Alchemy
/// is not stopped).
pub fn castable(
    spell: &SpellDefinition,
    modelled: bool,
    silenced: bool,
    silent_alchemy: bool,
) -> bool {
    spell.monsters_cast
        && modelled
        && !matches!(
            spell.targeting,
            Targeting::Party | Targeting::Point | Targeting::Item | Targeting::LockOrTrap
        )
        && (!silenced || (spell.books.contains(Spellbook::Alchemy) && silent_alchemy))
}

/// A living body in the fight as a caster weighs it for one spell.
#[derive(Clone, Copy)]
struct Candidate {
    body: Entity,
    member: bool,
    /// The monster group it is of.
    group: Option<Entity>,
    /// The caster sees it: a member if it makes out the party, a monster by
    /// a clear line.
    seen: bool,
    /// How the caster stands toward it, itself a friend.
    stance: Stance,
    /// It is within the spell's reach.
    reached: bool,
    /// The spell would do something to it.
    worth: bool,
}

/// At whom the spell may be aimed (retail `CollectMonsterSpellTargets`,
/// `MonsterAI.cpp:1941-1960`), each one choice.
///
/// - the caster: itself, if the spell would do something to it;
/// - a friend or a foe: each one seen and in reach that it would do something
///   to, monsters first and then members;
/// - a group: each other monster group in the fight at least half of whose
///   hostile members would feel it and one of which is in reach, aimed at the
///   first such member; then the party, if at least half of the members seen
///   would feel it and one of them is in reach (a hostile one);
/// - a cone or a blast: each foe monster seen and in reach; and each hostile
///   member seen in reach once for each member seen, if at least half of the
///   party would feel it;
/// - all enemies: ahead, if any foe is seen and in reach.
pub fn aims(
    sight: &Sight,
    own: &BodyReadOnlyItem,
    spell: &SpellDefinition,
    content: &Catalogs,
    fray: Fray,
) -> Vec<Aim> {
    let (targeting, caster) = (spell.targeting, own.entity);
    let group_of = |body| sight.minds.get(body).ok().and_then(|mind| mind.group).map(|g| g.0);
    let own_group = group_of(caster);
    let eye = sight.places.of(caster).eye;
    let bodies: Vec<Candidate> = fray
        .bodies
        .iter()
        .filter_map(|&body| {
            let stance = sight.toward(own, body)?;
            let their = sight.bodies.get(body).ok()?;
            let member = sight.roster().contains(&body);
            let there = sight.places.of(body);
            Some(Candidate {
                body,
                member,
                group: group_of(body),
                seen: if member {
                    fray.sees_party
                } else {
                    body == caster || sight.obstacles.clear(eye, there.eye)
                },
                stance,
                reached: sight.places.gap(caster, body) <= spell.reach.metres(),
                worth: content.spell_row(spell.id).is_some_and(|row| {
                    row.worthwhile(their.vitals, sight.laid.has(body, spell.id))
                }),
            })
        })
        .collect();
    let bodies = &bodies[..];
    let seen = || bodies.iter().filter(|c| c.seen);
    let standing = |member: bool, stance: Stance| {
        seen()
            .filter(move |c| c.member == member && c.stance == stance && c.reached && c.worth)
            .map(|c| Aim::Body(c.body))
    };
    let party = seen().filter(|c| c.member).count();
    let half = || {
        let valid = seen().filter(|c| c.member && c.reached && c.worth).count();
        valid != 0 && party.div_ceil(2) <= valid
    };
    let hostile_members: Vec<Aim> = seen()
        .filter(|c| c.member && c.reached && c.stance == Stance::Hostile)
        .map(|c| Aim::Body(c.body))
        .collect();
    match targeting {
        Targeting::Caster => {
            if bodies.iter().any(|c| c.body == caster && c.worth) {
                vec![Aim::Body(caster)]
            } else {
                vec![]
            }
        }
        Targeting::Ally => {
            standing(false, Stance::Friendly).chain(standing(true, Stance::Friendly)).collect()
        }
        Targeting::Enemy => {
            standing(false, Stance::Hostile).chain(standing(true, Stance::Hostile)).collect()
        }
        Targeting::EnemyGroup => {
            let mut groups: Vec<Entity> = Vec::new();
            for group in bodies.iter().filter_map(|c| c.group).filter(|&g| Some(g) != own_group) {
                if !groups.contains(&group) {
                    groups.push(group);
                }
            }
            let mut aims: Vec<Aim> = groups
                .into_iter()
                .filter_map(|group| {
                    let foes = || {
                        bodies
                            .iter()
                            .filter(move |c| c.group == Some(group) && c.stance == Stance::Hostile)
                    };
                    let (eligible, valid) = (foes().count(), foes().filter(|c| c.worth).count());
                    let felt = valid != 0 && eligible.div_ceil(2) <= valid;
                    felt.then(|| foes().find(|c| c.reached)).flatten().map(|c| Aim::Body(c.body))
                })
                .collect();
            if half() {
                aims.extend(hostile_members.first().copied());
            }
            aims
        }
        Targeting::Cone | Targeting::Radius => {
            let mut aims: Vec<_> = standing(false, Stance::Hostile).collect();
            if half() {
                aims.extend(hostile_members);
            }
            aims
        }
        Targeting::AllEnemies => {
            let foes =
                standing(false, Stance::Hostile).count() + standing(true, Stance::Hostile).count();
            if foes != 0 { vec![Aim::Ahead] } else { vec![] }
        }
        _ => vec![],
    }
}

/// The spell it casts and how hard, and at whom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cast {
    pub spell: SpellId,
    pub aim: Aim,
    pub power: u32,
}

/// The spells in a monster's slots that it may cast now, each with its
/// slot's weight and the aims it has: castable, and with someone to aim at.
fn usable<'a>(
    sight: &Sight,
    own: &BodyReadOnlyItem,
    mind: &MindItem,
    content: &'a Catalogs,
    fray: Fray,
) -> Vec<(u32, &'a SpellDefinition, Vec<Aim>)> {
    let silenced = own.vitals.conditions.has(Condition::Silenced);
    mind.casting
        .spells
        .iter()
        .zip(SLOT_WEIGHTS)
        .filter_map(|(id, weight)| Some((weight, content.spells.get((*id)?)?)))
        .filter(|(_, spell)| {
            let modelled = content.spell_row(spell.id).is_some();
            castable(spell, modelled, silenced, mind.casting.silent_alchemy)
        })
        .map(|(weight, spell)| (weight, spell, aims(sight, own, spell, content, fray)))
        .filter(|(.., aims)| !aims.is_empty())
        .collect()
}

/// Whether a monster could cast at its foes now: it has a chance to cast, and
/// a usable spell aimed at enemies.
pub fn casts_at_foes(
    sight: &Sight,
    own: &BodyReadOnlyItem,
    mind: &MindItem,
    content: &Catalogs,
    fray: Fray,
) -> bool {
    mind.casting.chance != 0
        && usable(sight, own, mind, content, fray)
            .iter()
            .any(|(_, spell, _)| spell.aimed_at_enemies())
}

/// Whether a monster casts now, the party `gap` metres off, and what: not
/// without a budget or a chance, nor if it would close in; with its chance
/// while an arm reaches the party and for sure when none does; one of its
/// usable spells by the weights of their slots, at one of its aims, at the
/// power its budget covers.
pub fn casts(
    rng: &mut GameRng,
    sight: &Sight,
    own: &BodyReadOnlyItem,
    mind: &MindItem,
    gap: f32,
    content: &Catalogs,
    fray: Fray,
) -> Option<Cast> {
    let budget = own.budget?.0;
    if mind.casting.chance == 0 || closes_in(own, mind, gap) {
        return None;
    }
    let chance = if reaches_party(own.attacks, gap) { u32::from(mind.casting.chance) } else { 100 };
    let usable = usable(sight, own, mind, content, fray);
    if usable.is_empty() || rng.below(100) >= chance {
        return None;
    }
    let total: u32 = usable.iter().map(|(weight, ..)| weight).sum();
    let which = weighted(rng.below(total), usable.iter().map(|(weight, ..)| *weight))
        .unwrap_or(usable.len() - 1);
    let (_, spell, aims) = &usable[which];
    let aim = aims[rng.below(aims.len() as u32) as usize];
    Some(Cast { spell: spell.id, aim, power: power(rng, budget, spell) })
}

/// How hard it casts (retail `ChooseMonsterSpellPowerLevel`): up from 1 while
/// the spell's failure chance at that power with the budget as skill is under 9
/// percent and the power is under 8; then one less if above 1; then below(4): on
/// 0 one less if above 2, on 1 one more if under 7.
pub fn power(rng: &mut GameRng, budget: u32, spell: &SpellDefinition) -> u32 {
    let mut power = 1;
    while power < 8 && spell_failure(budget, spell, power) < 9 {
        power += 1;
    }
    if power > 1 {
        power -= 1;
    }
    match rng.below(4) {
        0 if power > 2 => power - 1,
        1 if power < MAX_POWER => power + 1,
        _ => power,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::species::Reach;

    fn spell(id: u8, targeting: Targeting) -> SpellDefinition {
        SpellDefinition {
            id: SpellId(id),
            targeting,
            level: 1,
            cost: 2,
            monsters_cast: true,
            reach: Reach::Long,
            ..Default::default()
        }
    }

    // Power: failure of the test spell (cost 2, level 1: band 2, threshold 50) at
    // power p is need = 50 * p / 7 = 7, 14, 21, 28, 35, 42, 50 for 1 to 7, and
    // (need - budget) * 70 / need when the budget is short of it.

    fn powers(budget: u32) -> Vec<u32> {
        let blast = spell(1, Targeting::Enemy);
        let mut rng = GameRng::seeded(4);
        let mut got: Vec<u32> = (0..200).map(|_| power(&mut rng, budget, &blast)).collect();
        got.sort_unstable();
        got.dedup();
        got
    }

    #[test]
    fn a_middling_budget_casts_one_below_the_last_power_it_covers_give_or_take_one() {
        // Budget 30: powers 1-4 need 7, 14, 21, 28, all met; power 5 needs 35:
        // (35 - 30) * 70 / 35 = 10, not under 9. So 5, then 4; roll 0 -> 3,
        // roll 1 -> 5, else 4.
        assert_eq!(powers(30), [3, 4, 5]);
        // Budget 31: (35 - 31) * 70 / 35 = 8 is under 9: power 6 is tried; it
        // needs 42: (42 - 31) * 70 / 42 = 18: stops at 6, then 5; 4, 5 or 6.
        assert_eq!(powers(31), [4, 5, 6]);
    }
}
