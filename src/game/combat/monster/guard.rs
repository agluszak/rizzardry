//! A monster guarding a fellow, or holding back warily (`docs/rules/monsters.md`
//! section 4, "Guarding a fellow" and "Holding back warily").

use bevy::prelude::Entity;

use super::close::{Striking, reaches_party};
use crate::game::rng::GameRng;

/// A living body in the fight other than the monster, as it weighs it for a ward.
#[derive(Clone, Copy, Debug)]
pub struct Fellow {
    pub body: Entity,
    /// It is on the monster's side (friendly toward it).
    pub friendly: bool,
    /// It is within the reach of the monster's first attack.
    pub in_reach: bool,
    pub level: u32,
    /// Its health as a share of its most, in percent (whole).
    pub health: u32,
}

/// What guarding weighs.
#[derive(Clone, Copy, Debug)]
pub struct Guarding<'a> {
    /// What its arms reach of the party: it has someone in reach if one does.
    pub striking: Striking<'a>,
    /// Its kind is a bodyguard.
    pub bodyguard: bool,
    /// It is not Blind or worse.
    pub able: bool,
    /// Its kind holds back warily.
    pub wary: bool,
    pub intelligence: u32,
    pub level: u32,
    pub fellows: &'a [Fellow],
}

/// What it does instead of striking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Watching {
    /// It guards this fellow for the round.
    Guards(Entity),
    /// It holds back and rests.
    HoldsBack,
}

/// Whether it guards a fellow or holds back (retail `BuildMonsterActionQueue`,
/// `CanMonsterProtectCombatant`, `MonsterAI.cpp:1090-1135`,
/// `Combat Attack.cpp:406-456`). A monster with someone in reach of the
/// party that is a bodyguard, able and armed, with chance equal to its
/// Intelligence percent looks for a ward: a living fellow on its side, within
/// the reach of its first attack, of a higher level than it and at under 40
/// percent of its health, and guards one of them at random. Failing that, a wary
/// one with chance equal to its Intelligence percent holds back.
pub fn guards(rng: &mut GameRng, facts: &Guarding) -> Option<Watching> {
    if !reaches_party(&facts.striking) {
        return None;
    }
    let wards: Vec<Entity> = if facts.bodyguard
        && facts.able
        && !facts.striking.arms.is_empty()
        && rng.below(100) < facts.intelligence
    {
        facts
            .fellows
            .iter()
            .filter(|f| f.friendly && f.in_reach && f.level > facts.level && f.health < 40)
            .map(|f| f.body)
            .collect()
    } else {
        Vec::new()
    };
    if !wards.is_empty() {
        Some(Watching::Guards(wards[rng.below(wards.len() as u32) as usize]))
    } else if facts.wary && rng.below(100) < facts.intelligence {
        Some(Watching::HoldsBack)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::species::Reach;

    fn entity(index: u32) -> Entity {
        Entity::from_raw_u32(index).unwrap()
    }

    /// A fellow on its side, in reach, a level above, at 39 percent.
    fn ward(index: u32) -> Fellow {
        Fellow { body: entity(index), friendly: true, in_reach: true, level: 6, health: 39 }
    }

    const CLAWS: [Reach; 1] = [Reach::Touch];

    /// Claws, the party `party_gap` metres off.
    fn striking(arms: &[Reach], party_gap: f32) -> Striking<'_> {
        Striking { arms, party_gap, prefers_range: false, blind: false }
    }

    /// A bodyguard of level 5 and Intelligence 100 (so always clever enough)
    /// with someone in reach of the party (claws, 1 m), with the fellows given.
    fn guard(fellows: &[Fellow]) -> Guarding<'_> {
        Guarding {
            striking: striking(&CLAWS, 1.0),
            bodyguard: true,
            able: true,
            wary: false,
            intelligence: 100,
            level: 5,
            fellows,
        }
    }

    fn watch(facts: &Guarding) -> Vec<Option<Watching>> {
        let mut rng = GameRng::seeded(8);
        (0..60).map(|_| guards(&mut rng, facts)).collect()
    }

    #[test]
    fn a_bodyguard_guards_a_hurt_stronger_fellow_in_reach() {
        let fellows = [ward(10)];
        assert!(watch(&guard(&fellows)).iter().all(|w| *w == Some(Watching::Guards(entity(10)))));
    }

    #[test]
    fn a_ward_is_under_40_percent_of_its_health() {
        // 39 is under 40; 40 is not.
        let fellows = [Fellow { health: 40, ..ward(10) }];
        assert!(watch(&guard(&fellows)).iter().all(Option::is_none));
        let fellows = [Fellow { health: 0, ..ward(10) }];
        assert!(watch(&guard(&fellows)).iter().all(Option::is_some));
    }

    #[test]
    fn a_ward_is_of_a_higher_level_than_the_guard() {
        let fellows = [Fellow { level: 5, ..ward(10) }];
        assert!(watch(&guard(&fellows)).iter().all(Option::is_none), "equal is not higher");
        let fellows = [Fellow { level: 4, ..ward(10) }];
        assert!(watch(&guard(&fellows)).iter().all(Option::is_none));
        let fellows = [Fellow { level: 6, ..ward(10) }];
        assert!(watch(&guard(&fellows)).iter().all(Option::is_some));
    }

    #[test]
    fn a_ward_is_a_friend_within_the_reach_of_the_first_attack() {
        let foe = [Fellow { friendly: false, ..ward(10) }];
        assert!(watch(&guard(&foe)).iter().all(Option::is_none));
        let far = [Fellow { in_reach: false, ..ward(10) }];
        assert!(watch(&guard(&far)).iter().all(Option::is_none));
    }

    #[test]
    fn it_guards_one_of_several_wards_at_random() {
        let fellows = [ward(10), ward(11), Fellow { health: 90, ..ward(12) }];
        let chosen = watch(&guard(&fellows));
        assert!(chosen.contains(&Some(Watching::Guards(entity(10)))));
        assert!(chosen.contains(&Some(Watching::Guards(entity(11)))));
        assert!(!chosen.contains(&Some(Watching::Guards(entity(12)))), "well enough");
    }

    #[test]
    fn it_needs_to_be_a_bodyguard_able_armed_and_with_someone_in_reach_of_the_party() {
        let fellows = [ward(10)];
        let base = guard(&fellows);
        for facts in [
            Guarding { bodyguard: false, ..base },
            Guarding { able: false, ..base },
            Guarding { striking: striking(&[], 1.0), ..base },
            Guarding { striking: striking(&CLAWS, 10.0), ..base },
        ] {
            assert!(watch(&facts).iter().all(Option::is_none), "{facts:?}");
        }
    }

    #[test]
    fn it_guards_with_a_chance_equal_to_its_intelligence_percent() {
        // Intelligence 50: about half of 400 draws guard.
        let fellows = [ward(10)];
        let facts = Guarding { intelligence: 50, ..guard(&fellows) };
        let mut rng = GameRng::seeded(5);
        let guarded = (0..400).filter(|_| guards(&mut rng, &facts).is_some()).count();
        assert!((120..280).contains(&guarded), "{guarded} of 400");
        // Intelligence 0 never does.
        let witless = Guarding { intelligence: 0, ..guard(&fellows) };
        assert!(watch(&witless).iter().all(Option::is_none));
    }

    #[test]
    fn a_wary_monster_with_no_ward_holds_back_at_its_intelligence_percent() {
        let wary = Guarding { wary: true, bodyguard: false, ..guard(&[]) };
        assert!(watch(&wary).iter().all(|w| *w == Some(Watching::HoldsBack)));
        let half = Guarding { intelligence: 50, ..wary };
        let mut rng = GameRng::seeded(5);
        let held = (0..400).filter(|_| guards(&mut rng, &half).is_some()).count();
        assert!((120..280).contains(&held), "{held} of 400");
        // Someone in reach of the party is needed for this too.
        let alone = Guarding { striking: striking(&CLAWS, 10.0), ..wary };
        assert!(watch(&alone).iter().all(Option::is_none));
        // Not wary, nothing.
        assert!(watch(&Guarding { wary: false, ..wary }).iter().all(Option::is_none));
    }

    #[test]
    fn a_ward_found_comes_before_holding_back() {
        let fellows = [ward(10)];
        let facts = Guarding { wary: true, ..guard(&fellows) };
        assert!(watch(&facts).iter().all(|w| *w == Some(Watching::Guards(entity(10)))));
    }
}
