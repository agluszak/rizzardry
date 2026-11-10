//! A monster's special attack, a breath or a summoning (`docs/rules/monsters.md`
//! section 4, "The special attack").

use super::close::{Striking, closes_in, reaches_party};
use crate::content::species::Reach;
use crate::game::magic::casting::Aim;
use crate::game::rng::GameRng;
use crate::game::world::hostility::Stance;

/// What making a special attack weighs.
#[derive(Clone, Copy, Debug)]
pub struct Making<'a> {
    /// Its kind has one.
    pub has: bool,
    /// It summons rather than breathes.
    pub summons: bool,
    /// The chance (%) it makes it at its turn while it can strike someone.
    pub chance: u8,
    /// Its cooldown has rounds left.
    pub cooling: bool,
    pub stamina: u32,
    pub stamina_most: u32,
    pub silenced: bool,
    /// A silence stops this special attack.
    pub honours_silence: bool,
    /// What its arms reach of the party: it would close in rather than make
    /// it if it prefers blows and none does.
    pub striking: Striking<'a>,
    /// A fight is on: a summoning never opens one.
    pub fighting: bool,
    /// Its side toward the party.
    pub stance: Stance,
    /// The aims a breath may take, as for a cone spell.
    pub aims: &'a [Aim],
}

/// What a special attack comes to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Made {
    /// Where a breath is aimed; none for a summoning.
    pub aim: Option<Aim>,
    /// The stamina it costs: a tenth of its most.
    pub cost: u32,
}

/// Whether its special attack is ready (retail `CanMonsterFlee`,
/// `MonsterAI.cpp:2279-2326`), its chance and its wish to close in apart: it
/// has one with a chance above 0, it is not cooling down, it has a tenth of its
/// stamina, a silence does not stop it, and a summoning has the party within
/// long reach and the summoner not on the party's side and is never what opens
/// a fight, or a breath has an aim.
pub fn ready(facts: &Making) -> bool {
    let summons = !facts.summons
        || (facts.fighting
            && facts.striking.party_gap <= Reach::Long.metres()
            && facts.stance != Stance::Friendly);
    facts.has
        && facts.chance != 0
        && !facts.cooling
        && facts.stamina >= facts.stamina_most / 10
        && !(facts.silenced && facts.honours_silence)
        && summons
        && (facts.summons || !facts.aims.is_empty())
}

/// The special attack it makes, if it does: one that would close in to strike
/// does not; otherwise when it is ready, at its chance where one of its attacks
/// reaches the party and always where none does (retail `UpdateMonsterAI`,
/// `MonsterAI.cpp:817-945`), a breath at one of its aims at random.
pub fn makes(rng: &mut GameRng, facts: &Making) -> Option<Made> {
    if closes_in(&facts.striking) || !ready(facts) {
        return None;
    }
    let aim = (!facts.summons).then(|| facts.aims[rng.below(facts.aims.len() as u32) as usize]);
    let chance = if reaches_party(&facts.striking) { u32::from(facts.chance) } else { 100 };
    (rng.below(100) < chance).then_some(Made { aim, cost: facts.stamina_most / 10 })
}

#[cfg(test)]
mod tests {
    use bevy::prelude::Entity;

    use super::*;

    const AIMS: [Aim; 2] = [Aim::Body(Entity::PLACEHOLDER), Aim::Ahead];
    const CLAWS: [Reach; 1] = [Reach::Touch];

    /// Claws, the party `party_gap` metres off.
    fn striking(party_gap: f32, prefers_range: bool) -> Striking<'static> {
        Striking { arms: &CLAWS, party_gap, prefers_range, blind: false }
    }

    /// A breath with a sure chance, ready, with an aim, in a fight with the party
    /// in reach (1 m, claws), from a monster with 100 stamina of 100.
    fn breath() -> Making<'static> {
        Making {
            has: true,
            summons: false,
            chance: 100,
            cooling: false,
            stamina: 100,
            stamina_most: 100,
            silenced: false,
            honours_silence: true,
            striking: striking(1.0, false),
            fighting: true,
            stance: Stance::Hostile,
            aims: &AIMS,
        }
    }

    fn summoning() -> Making<'static> {
        Making { summons: true, aims: &[], ..breath() }
    }

    #[test]
    fn a_ready_breath_with_a_sure_chance_is_made_at_one_of_its_aims_and_costs_a_tenth() {
        let mut rng = GameRng::seeded(5);
        let mut aimed = Vec::new();
        for _ in 0..40 {
            let made = makes(&mut rng, &breath()).unwrap();
            assert_eq!(made.cost, 10); // 100 / 10
            aimed.push(made.aim.unwrap());
        }
        assert!(aimed.contains(&AIMS[0]) && aimed.contains(&AIMS[1]), "either aim comes up");
    }

    #[test]
    fn the_cost_is_a_tenth_of_the_most_stamina_whole() {
        let made =
            makes(&mut GameRng::seeded(0), &Making { stamina_most: 57, stamina: 57, ..breath() });
        assert_eq!(made.unwrap().cost, 5); // 57 / 10
    }

    #[test]
    fn it_needs_a_special_attack_with_a_chance_above_nothing() {
        assert!(!ready(&Making { has: false, ..breath() }));
        assert!(!ready(&Making { chance: 0, ..breath() }));
        assert!(ready(&Making { chance: 1, ..breath() }));
    }

    #[test]
    fn one_that_would_close_in_does_not_make_it_but_it_is_ready_all_the_same() {
        // Claws reach 2 m: at 10 m a monster that prefers blows closes in.
        let closing = Making { striking: striking(10.0, false), ..breath() };
        assert!(ready(&closing));
        assert!(makes(&mut GameRng::seeded(0), &closing).is_none());
        // One that prefers range makes it from afar, for sure (no arm reaches).
        let ranged = Making { striking: striking(10.0, true), ..breath() };
        assert!(makes(&mut GameRng::seeded(0), &ranged).is_some());
    }

    #[test]
    fn a_cooling_special_attack_waits() {
        assert!(!ready(&Making { cooling: true, ..breath() }));
    }

    #[test]
    fn it_needs_a_tenth_of_its_stamina() {
        // Of 100: 10 is enough, 9 is not. Of 57: 5 (57 / 10 whole) is enough.
        assert!(ready(&Making { stamina: 10, ..breath() }));
        assert!(!ready(&Making { stamina: 9, ..breath() }));
        assert!(ready(&Making { stamina: 5, stamina_most: 57, ..breath() }));
        assert!(!ready(&Making { stamina: 4, stamina_most: 57, ..breath() }));
    }

    #[test]
    fn a_silence_stops_only_a_special_attack_that_honours_it() {
        assert!(!ready(&Making { silenced: true, honours_silence: true, ..breath() }));
        assert!(ready(&Making { silenced: true, honours_silence: false, ..breath() }));
        assert!(ready(&Making { silenced: false, honours_silence: true, ..breath() }));
    }

    #[test]
    fn a_breath_needs_an_aim() {
        assert!(!ready(&Making { aims: &[], ..breath() }));
        assert!(makes(&mut GameRng::seeded(0), &Making { aims: &[], ..breath() }).is_none());
    }

    #[test]
    fn a_summoning_needs_no_aim_but_the_party_within_long_reach() {
        let made = makes(&mut GameRng::seeded(0), &summoning()).unwrap();
        assert_eq!(made, Made { aim: None, cost: 10 });
        // 25 m is long reach; 25.1 is past it.
        assert!(ready(&Making { striking: striking(25.0, true), ..summoning() }));
        assert!(!ready(&Making { striking: striking(25.1, true), ..summoning() }));
    }

    #[test]
    fn a_summoning_is_never_made_by_a_summoner_friendly_to_the_party() {
        assert!(!ready(&Making { stance: Stance::Friendly, ..summoning() }));
        assert!(ready(&Making { stance: Stance::Neutral, ..summoning() }));
        assert!(ready(&Making { stance: Stance::Hostile, ..summoning() }));
    }

    #[test]
    fn a_summoning_never_opens_a_fight_but_a_breath_may() {
        assert!(!ready(&Making { fighting: false, ..summoning() }));
        assert!(ready(&Making { fighting: false, ..breath() }));
    }

    #[test]
    fn it_is_made_at_its_chance_where_an_attack_reaches_the_party() {
        // Chance 50 over 400 draws: made some of the time, not all.
        let half = Making { chance: 50, ..breath() };
        let mut rng = GameRng::seeded(3);
        let made = (0..400).filter(|_| makes(&mut rng, &half).is_some()).count();
        assert!((120..280).contains(&made), "{made} of 400 at 50%");
    }

    #[test]
    fn it_is_made_for_sure_where_no_attack_reaches_the_party() {
        let far = Making { chance: 1, striking: striking(10.0, true), ..breath() };
        let mut rng = GameRng::seeded(3);
        assert!((0..100).all(|_| makes(&mut rng, &far).is_some()), "chance 100 when out of reach");
        // In reach, a chance of 1 is made about once in a hundred.
        let near = Making { chance: 1, ..breath() };
        let made = (0..400).filter(|_| makes(&mut rng, &near).is_some()).count();
        assert!(made < 30, "{made} of 400 at 1%");
    }
}
