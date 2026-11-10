//! A monster choosing a spell (`docs/rules/monsters.md` section 5): which of
//! its ten slots it may cast, at whom each may be aimed, whether it casts,
//! which spell, and how hard.
//!
//! A monster spends nothing to cast: its spell budget stands in for a caster's
//! skill in the failure roll, and the power it picks is the most that budget
//! comfortably covers.

use bevy::prelude::Entity;

use super::close::{Striking, closes_in, reaches_party};
use crate::content::spells::{MAX_POWER, SpellDefinition, SpellId, Spellbook, Targeting};
use crate::game::magic::reach::Aim;
use crate::game::magic::roll::spell_failure;
use crate::game::rng::{GameRng, weighted};
use crate::game::world::hostility::Stance;

/// How much each of the ten slots weighs in the choice (retail
/// `g_spell_cast_weights`, `MonsterAI.cpp:141`).
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
#[derive(Clone, Copy, Debug)]
pub struct Candidate {
    pub body: Entity,
    /// One of the party's members.
    pub member: bool,
    /// The monster group it is in.
    pub group: Option<Entity>,
    /// The caster sees it (a member only if its eyes make out the party).
    pub seen: bool,
    /// How the caster stands toward it; itself, friendly.
    pub stance: Stance,
    /// The gap is within the spell's reach.
    pub reached: bool,
    /// The spell would do something to it: a heal if it is hurt, a refresh if
    /// it is tired, a condition if it does not have it, an enchantment if it is
    /// not laid on it, a cure if it bears what is cured (retail
    /// `MonsterSpellTargetOK`); a spell that does none of these always would.
    pub worth: bool,
}

/// What aiming one spell weighs.
#[derive(Clone, Copy, Debug)]
pub struct Aiming<'a> {
    pub targeting: Targeting,
    pub caster: Entity,
    /// The monster group the caster is in, whose own it does not aim at.
    pub own_group: Option<Entity>,
    /// The living bodies of the fight, the caster among them.
    pub bodies: &'a [Candidate],
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
pub fn aims(facts: &Aiming) -> Vec<Aim> {
    let Aiming { targeting, caster, own_group, bodies } = *facts;
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

/// One of a monster's ten slots with a spell in it.
#[derive(Clone, Debug)]
pub struct Slot<'a> {
    pub spell: &'a SpellDefinition,
    /// The game resolves its effect.
    pub modelled: bool,
    /// At whom it may be aimed ([`aims`]).
    pub aims: Vec<Aim>,
}

/// What casting weighs.
#[derive(Clone, Debug)]
pub struct Casting<'a> {
    /// The chance (%) its kind casts at its turn while it can strike someone.
    pub chance: u8,
    /// What its arms reach of the party: it would close in rather than cast if
    /// it prefers blows and none does.
    pub striking: Striking<'a>,
    pub silenced: bool,
    /// Its kind keeps its Alchemy through a silence.
    pub silent_alchemy: bool,
    /// What its casting stands on; none, it does not cast.
    pub budget: Option<u32>,
    /// Its slots in order, up to ten.
    pub slots: &'a [Option<Slot<'a>>],
}

/// The spell it casts and how hard, and at whom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cast {
    pub spell: SpellId,
    pub aim: Aim,
    pub power: u32,
}

/// The slots it may cast from now: a castable spell with at least one aim,
/// with the weight of the slot.
fn usable<'a>(facts: &'a Casting) -> impl Iterator<Item = (u32, &'a Slot<'a>)> {
    facts
        .slots
        .iter()
        .zip(SLOT_WEIGHTS)
        .filter_map(|(slot, weight)| Some((weight, slot.as_ref()?)))
        .filter(|(_, slot)| {
            castable(slot.spell, slot.modelled, facts.silenced, facts.silent_alchemy)
                && !slot.aims.is_empty()
        })
}

/// Whether it could cast at a foe now (retail `MonsterGroupCanEngage`'s spell
/// test): it casts at all, and one of its spells aimed at enemies is castable
/// with an aim.
pub fn casts_at_foes(facts: &Casting) -> bool {
    facts.chance != 0 && usable(facts).any(|(_, slot)| slot.spell.aimed_at_enemies())
}

/// The spell it casts, if it does, and at whom (retail `UpdateMonsterAI`):
/// one that casts and does not close in rolls its chance once, the species' if
/// one of its attacks reaches the party and 100 if none does, if it has a spell
/// it can cast with an aim; if that comes up it casts one of them by the weights
/// of their slots (retail `ChooseMonsterSpell`), at one of that spell's aims at
/// random, at the power its budget allows.
pub fn casts(rng: &mut GameRng, facts: &Casting) -> Option<Cast> {
    let budget = facts.budget?;
    if facts.chance == 0 || closes_in(&facts.striking) {
        return None;
    }
    let chance = if reaches_party(&facts.striking) { u32::from(facts.chance) } else { 100 };
    let usable: Vec<_> = usable(facts).collect();
    if usable.is_empty() || rng.below(100) >= chance {
        return None;
    }
    let total: u32 = usable.iter().map(|&(weight, _)| weight).sum();
    let which = weighted(rng.below(total), usable.iter().map(|&(weight, _)| weight))
        .unwrap_or(usable.len() - 1);
    let (_, slot) = usable[which];
    let aim = slot.aims[rng.below(slot.aims.len() as u32) as usize];
    Some(Cast { spell: slot.spell.id, aim, power: power(rng, budget, slot.spell) })
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
    use enumset::EnumSet;

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

    fn entity(index: u32) -> Entity {
        Entity::from_raw_u32(index).unwrap()
    }

    const CASTER: u32 = 1;

    /// A body the caster sees, in reach, that the spell would do something to.
    fn seen(index: u32, member: bool, stance: Stance) -> Candidate {
        Candidate {
            body: entity(index),
            member,
            group: None,
            seen: true,
            stance,
            reached: true,
            worth: true,
        }
    }

    fn itself() -> Candidate {
        seen(CASTER, false, Stance::Friendly)
    }

    fn aimed(targeting: Targeting, bodies: &[Candidate]) -> Vec<Aim> {
        aims(&Aiming { targeting, caster: entity(CASTER), own_group: None, bodies })
    }

    fn at(indices: &[u32]) -> Vec<Aim> {
        indices.iter().map(|&i| Aim::Body(entity(i))).collect()
    }

    #[test]
    fn a_silence_stops_all_but_alchemy_of_a_kind_that_keeps_it() {
        let blast = spell(1, Targeting::Enemy);
        let alchemy = SpellDefinition { books: Spellbook::Alchemy.into(), ..blast.clone() };
        assert!(!castable(&blast, true, true, true), "not Alchemy");
        assert!(!castable(&alchemy, true, true, false), "Alchemy of a kind that loses it");
        assert!(castable(&alchemy, true, true, true));
        assert!(castable(&alchemy, true, false, false), "no silence");
        let other: EnumSet<Spellbook> = Spellbook::Wizardry | Spellbook::Alchemy;
        assert!(castable(&SpellDefinition { books: other, ..blast }, true, true, true));
    }

    #[test]
    fn a_friend_is_each_friend_seen_in_reach_it_would_work_on_monsters_before_members() {
        let bodies = [
            seen(10, true, Stance::Friendly),
            itself(),
            seen(11, false, Stance::Friendly),
            Candidate { reached: false, ..seen(12, false, Stance::Friendly) },
            Candidate { worth: false, ..seen(13, false, Stance::Friendly) },
            Candidate { seen: false, ..seen(14, false, Stance::Friendly) },
            seen(15, false, Stance::Hostile),
        ];
        // Itself (friendly) and 11, then the member 10; not 12 (too far), 13
        // (nothing to do), 14 (unseen), 15 (a foe).
        assert_eq!(aimed(Targeting::Ally, &bodies), at(&[CASTER, 11, 10]));
    }

    #[test]
    fn a_party_is_aimed_at_by_a_group_spell_if_half_of_it_would_feel_it() {
        // Four members seen; need ceil(4 / 2) = 2 that are in reach and would feel it.
        let member = |i, worth| Candidate { worth, ..seen(i, true, Stance::Hostile) };
        let two = [member(10, true), member(11, true), member(12, false), member(13, false)];
        assert_eq!(aimed(Targeting::EnemyGroup, &two), at(&[10]), "the first hostile one");
        let one = [member(10, true), member(11, false), member(12, false), member(13, false)];
        assert_eq!(aimed(Targeting::EnemyGroup, &one), vec![], "1 of 4 is under half");
        // Three seen: ceil(3 / 2) = 2 are needed.
        let three_one = [member(10, true), member(11, false), member(12, false)];
        assert_eq!(aimed(Targeting::EnemyGroup, &three_one), vec![]);
        let three_two = [member(10, true), member(11, true), member(12, false)];
        assert_eq!(aimed(Targeting::EnemyGroup, &three_two), at(&[10]));
        // None feeling it is never enough.
        assert_eq!(aimed(Targeting::EnemyGroup, &[member(10, false)]), vec![]);
        // Those out of reach do not count as feeling it.
        let far = Candidate { reached: false, ..member(10, true) };
        assert_eq!(aimed(Targeting::EnemyGroup, &[far]), vec![]);
    }

    /// A foe monster of a group.
    fn grouped(index: u32, group: u32, stance: Stance) -> Candidate {
        Candidate { group: Some(entity(group)), ..seen(index, false, stance) }
    }

    #[test]
    fn a_group_is_aimed_at_even_when_unseen_and_comes_before_the_party() {
        // Retail asks the group's members no sight (`MonsterAI.cpp:1941-1960`),
        // and offers the groups before the party.
        let unseen = Candidate { seen: false, ..grouped(20, 50, Stance::Hostile) };
        let party = seen(10, true, Stance::Hostile);
        assert_eq!(aimed(Targeting::EnemyGroup, &[party, unseen]), at(&[20, 10]));
    }

    #[test]
    fn a_cone_is_each_foe_monster_and_each_hostile_member_if_half_the_party_would_feel_it() {
        let member = |i, worth| Candidate { worth, ..seen(i, true, Stance::Hostile) };
        let bodies =
            [itself(), seen(11, false, Stance::Hostile), member(10, true), member(12, true)];
        // The foe monster 11; both members feel it (2 of 2): each hostile member.
        assert_eq!(aimed(Targeting::Cone, &bodies), at(&[11, 10, 12]));
        assert_eq!(aimed(Targeting::Radius, &bodies), at(&[11, 10, 12]));
        // Only 1 of 2 members would feel it: ceil(2 / 2) = 1, still enough.
        let bodies = [seen(11, false, Stance::Hostile), member(10, true), member(12, false)];
        assert_eq!(aimed(Targeting::Cone, &bodies), at(&[11, 10, 12]));
        // None would: the members are left out, the monster foe remains.
        let bodies = [seen(11, false, Stance::Hostile), member(10, false), member(12, false)];
        assert_eq!(aimed(Targeting::Cone, &bodies), at(&[11]));
        // A member of the party that is not hostile (turned) is not aimed at.
        let turned = [Candidate { stance: Stance::Friendly, ..member(10, true) }];
        assert_eq!(aimed(Targeting::Cone, &turned), vec![]);
    }

    // Casting

    const CLAWS: [Reach; 1] = [Reach::Touch];

    /// Claws, the party `party_gap` metres off, and a kind that prefers range.
    fn striking(party_gap: f32, prefers_range: bool) -> Striking<'static> {
        Striking { arms: &CLAWS, party_gap, prefers_range, blind: false }
    }

    /// Casting from a monster with the party in reach (1 m, claws).
    fn casting<'a>(chance: u8, slots: &'a [Option<Slot<'a>>]) -> Casting<'a> {
        Casting {
            chance,
            striking: striking(1.0, false),
            silenced: false,
            silent_alchemy: false,
            budget: Some(30),
            slots,
        }
    }

    fn slot(spell: &SpellDefinition) -> Option<Slot<'_>> {
        Some(Slot { spell, modelled: true, aims: vec![Aim::Ahead] })
    }

    #[test]
    fn it_does_not_cast_with_no_chance_no_budget_no_usable_spell_or_a_wish_to_close_in() {
        let blast = spell(1, Targeting::Enemy);
        let slots = [slot(&blast)];
        let mut rng = GameRng::seeded(9);
        assert!(casts(&mut rng, &casting(0, &slots)).is_none());
        assert!(casts(&mut rng, &Casting { budget: None, ..casting(100, &slots) }).is_none());
        // Prefers blows and the party is out of reach of its claws: it closes in.
        let closing = Casting { striking: striking(10.0, false), ..casting(100, &slots) };
        assert!(casts(&mut rng, &closing).is_none());
        assert!(casts(&mut rng, &casting(100, &[])).is_none());
        // A slot with no aim, a spell it may not cast, and one silenced.
        let no_aim = [Some(Slot { spell: &blast, modelled: true, aims: vec![] })];
        assert!(casts(&mut rng, &casting(100, &no_aim)).is_none());
        let unmodelled = [Some(Slot { spell: &blast, modelled: false, aims: vec![Aim::Ahead] })];
        assert!(casts(&mut rng, &casting(100, &unmodelled)).is_none());
        assert!(casts(&mut rng, &Casting { silenced: true, ..casting(100, &slots) }).is_none());
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
