//! Catching the target unaware (`docs/rules/attack.md` section 3).

use super::Surprise;
use crate::content::species::Reach;
use crate::game::characters::Condition;
use crate::game::party::formation::Row;
use crate::game::rng::GameRng;

/// What a member who may notice an attack is doing and how well.
#[derive(Clone, Copy, Debug)]
pub struct Watch {
    /// Their order this round is Defend.
    pub defending: bool,
    pub alive: bool,
    pub stamina: u32,
    pub worst: Option<Condition>,
    pub blind: bool,
    pub senses: u32,
    /// How often they have tried to notice an attacker this round.
    pub tries: u32,
}

/// What catching a target unaware weighs.
#[derive(Clone, Copy, Debug)]
pub struct Unaware {
    pub reach: Reach,
    /// The side of the target the attacker is on.
    pub attacker_on: Row,
    /// The side a seated member faces; a monster has none, and faces its own
    /// front.
    pub facing: Option<Row>,
    /// The arm ambushes: its blows are natural attacks.
    pub ambushes: bool,
    /// A member under orders who may notice; a monster, or a member without an
    /// order, cannot.
    pub watch: Option<Watch>,
}

/// How the blow stands toward its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caught {
    pub surprise: Surprise,
    /// The target turned to face the attacker's side.
    pub turns_to: Option<Row>,
    /// How often the target has tried to notice an attacker now.
    pub tries: u32,
}

impl Watch {
    /// A defending member, awake enough and with sight, may notice.
    fn may_notice(&self) -> bool {
        self.defending
            && self.alive
            && self.stamina > 0
            && self.worst.is_none_or(|worst| worst < Condition::Webbed)
            && !self.blind
    }
}

/// Whether a member who has tried `tries` times already this round notices:
/// certain the first time, then with chance `Senses - 25 * tries` percent
/// (retail `CharacterNoticesAttacker`, which compares it as an unsigned
/// number: once it falls below 0, noticing is certain).
pub fn notices(rng: &mut GameRng, senses: u32, tries: u32) -> bool {
    if tries == 0 {
        return true;
    }
    let chance = senses as i32 - tries as i32 * 25;
    chance < 0 || rng.below(100) < chance as u32
}

/// Whether a blow from a side its target does not face catches it unaware
/// (retail `IsPartyLookingAwayFrom`): only a touch or short blow can; a member
/// who notices turns to face it instead; anyone else is caught if the attacker
/// is exactly across from the way they face. The arm's natural attack counts
/// as surprise whether or not the target faces the attacker.
pub fn catch_unaware(rng: &mut GameRng, facts: &Unaware) -> Caught {
    let mut caught = Caught {
        surprise: Surprise { unaware: false, natural: facts.ambushes },
        turns_to: None,
        tries: facts.watch.map_or(0, |watch| watch.tries),
    };
    let facing = facts.facing.unwrap_or(Row::Front);
    if facts.reach > Reach::Short || facing == facts.attacker_on {
        return caught;
    }
    let noticed = facts.watch.filter(Watch::may_notice).is_some_and(|watch| {
        caught.tries = watch.tries + 1;
        notices(rng, watch.senses, watch.tries)
    });
    if noticed {
        caught.turns_to = Some(facts.attacker_on);
    } else {
        caught.surprise.unaware = facing.opposite() == Some(facts.attacker_on);
    }
    caught
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watch() -> Watch {
        Watch {
            defending: true,
            alive: true,
            stamina: 10,
            worst: None,
            blind: false,
            senses: 50,
            tries: 0,
        }
    }

    /// An attacker at the target's rear while it faces front.
    fn from_behind(watch: Option<Watch>) -> Unaware {
        Unaware {
            reach: Reach::Touch,
            attacker_on: Row::Rear,
            facing: Some(Row::Front),
            ambushes: false,
            watch,
        }
    }

    fn caught(facts: &Unaware) -> Caught {
        catch_unaware(&mut GameRng::seeded(0), facts)
    }

    #[test]
    fn a_blow_from_behind_catches_a_monster_unaware() {
        let got = caught(&from_behind(None));
        assert!(got.surprise.unaware && !got.surprise.natural);
        assert_eq!(got.turns_to, None);
    }

    #[test]
    fn a_monster_with_no_seat_faces_its_own_front() {
        // Facing none is facing front: the attacker behind it catches it unaware,
        // one in front does not.
        let facts = Unaware { facing: None, ..from_behind(None) };
        assert!(caught(&facts).surprise.unaware);
        let facts = Unaware { facing: None, attacker_on: Row::Front, ..from_behind(None) };
        assert!(!caught(&facts).surprise.unaware);
    }

    #[test]
    fn only_the_side_across_from_the_way_it_faces_catches_it() {
        for (side, unaware) in
            [(Row::Front, false), (Row::Right, false), (Row::Left, false), (Row::Rear, true)]
        {
            let facts = Unaware { attacker_on: side, ..from_behind(None) };
            assert_eq!(caught(&facts).surprise.unaware, unaware, "{side:?}");
        }
        // Facing right, the left is across.
        let facts =
            Unaware { facing: Some(Row::Right), attacker_on: Row::Left, ..from_behind(None) };
        assert!(caught(&facts).surprise.unaware);
    }

    #[test]
    fn a_target_that_faces_the_attacker_sees_it_coming() {
        let facts =
            Unaware { facing: Some(Row::Rear), attacker_on: Row::Rear, ..from_behind(None) };
        let got = caught(&facts);
        assert!(!got.surprise.unaware);
        assert_eq!(got.turns_to, None);
    }

    #[test]
    fn only_a_touch_or_short_blow_can_catch_it_unaware() {
        for (reach, unaware) in [
            (Reach::Touch, true),
            (Reach::Short, true),
            (Reach::Long, false),
            (Reach::Extreme, false),
        ] {
            let facts = Unaware { reach, ..from_behind(None) };
            assert_eq!(caught(&facts).surprise.unaware, unaware, "{reach:?}");
        }
    }

    #[test]
    fn a_natural_attack_is_surprise_whether_or_not_the_target_faces_it() {
        for facing in [Row::Front, Row::Rear] {
            for reach in [Reach::Touch, Reach::Extreme] {
                let facts = Unaware {
                    ambushes: true,
                    facing: Some(facing),
                    reach,
                    attacker_on: Row::Rear,
                    watch: None,
                };
                assert!(caught(&facts).surprise.natural);
            }
        }
        assert!(!caught(&from_behind(None)).surprise.natural);
    }

    #[test]
    fn a_defending_member_notices_for_certain_the_first_time_and_turns() {
        for seed in 0..50 {
            let got = catch_unaware(&mut GameRng::seeded(seed), &from_behind(Some(watch())));
            assert!(!got.surprise.unaware);
            assert_eq!(got.turns_to, Some(Row::Rear));
            assert_eq!(got.tries, 1);
        }
    }

    #[test]
    fn later_tries_notice_with_senses_less_25_a_try_in_percent() {
        // Senses 50, one try already: 50 - 25 = 25 percent, so sometimes.
        let facts = from_behind(Some(Watch { tries: 1, ..watch() }));
        let noticed: Vec<_> = (0..200)
            .map(|seed| catch_unaware(&mut GameRng::seeded(seed), &facts).turns_to.is_some())
            .collect();
        assert!(noticed.iter().any(|&n| n) && noticed.iter().any(|&n| !n));
        // Senses 50, two tries: 50 - 50 = 0 percent, never; and not noticing
        // from behind is being caught.
        let facts = from_behind(Some(Watch { tries: 2, ..watch() }));
        for seed in 0..100 {
            let got = catch_unaware(&mut GameRng::seeded(seed), &facts);
            assert_eq!(got.turns_to, None);
            assert!(got.surprise.unaware);
            // The count of tries rises whether or not they notice.
            assert_eq!(got.tries, 3);
        }
    }

    #[test]
    fn a_negative_figure_is_certain_as_the_original_compares_it_unsigned() {
        // Senses 50, three tries: 50 - 75 = -25, which unsigned is huge.
        for seed in 0..100 {
            let mut rng = GameRng::seeded(seed);
            assert!(notices(&mut rng, 50, 3));
            assert!(notices(&mut rng, 0, 1));
            assert!(!notices(&mut rng, 25, 1), "25 - 25 = 0 percent");
        }
    }

    #[test]
    fn a_member_who_is_not_defending_does_not_try() {
        let facts = from_behind(Some(Watch { defending: false, tries: 4, ..watch() }));
        let got = caught(&facts);
        assert!(got.surprise.unaware);
        assert_eq!(got.turns_to, None);
        assert_eq!(got.tries, 4);
    }

    #[test]
    fn a_member_not_alert_does_not_notice() {
        let blind = Watch { blind: true, ..watch() };
        let spent = Watch { stamina: 0, ..watch() };
        let dead = Watch { alive: false, ..watch() };
        let webbed = Watch { worst: Some(Condition::Webbed), ..watch() };
        for member in [blind, spent, dead, webbed] {
            let got = caught(&from_behind(Some(member)));
            assert!(got.surprise.unaware, "{member:?}");
            assert_eq!(got.turns_to, None);
            assert_eq!(got.tries, 0);
        }
        // Nothing worse than Turncoat is alert still.
        let turned = Watch { worst: Some(Condition::Turncoat), ..watch() };
        assert_eq!(caught(&from_behind(Some(turned))).turns_to, Some(Row::Rear));
    }

    #[test]
    fn a_member_that_faces_the_attacker_does_not_try() {
        let facts = Unaware { facing: Some(Row::Rear), ..from_behind(Some(watch())) };
        assert_eq!(caught(&facts).tries, 0);
    }
}
