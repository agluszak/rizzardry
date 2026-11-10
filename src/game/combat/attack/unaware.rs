//! Catching the target unaware (`docs/rules/attack.md` section 3).

use super::{FighterReadOnlyItem, Surprise};
use crate::content::species::Reach;
use crate::game::actors::Arm;
use crate::game::characters::Condition;
use crate::game::party::formation::Row;
use crate::game::rng::GameRng;

/// How the blow stands toward its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caught {
    pub surprise: Surprise,
    /// The target turned to face the attacker's side.
    pub turns_to: Option<Row>,
    /// How often the target has tried to notice an attacker this round now.
    pub tries: u32,
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

/// Whether a blow from the side of its target the attacker is on
/// (`attacker_on`) catches it unaware (retail `IsPartyLookingAwayFrom`).
/// A seated member faces the way `facing` says, anyone else its own front.
/// Only a touch or short blow can; a member keeping watch (defending, alive,
/// not spent, nothing as bad as Webbed, not blind) who notices turns to face
/// it instead; anyone else is caught if the attacker is exactly across from
/// the way they face. An arm that ambushes strikes naturally whether or not
/// the target faces it.
pub fn catch_unaware(
    rng: &mut GameRng,
    arm: &Arm,
    (attacker_on, facing): (Row, Option<Row>),
    target: &FighterReadOnlyItem,
) -> Caught {
    let vitals = target.body.vitals;
    let tries = target.spotted.map(|spotted| spotted.0);
    let mut caught = Caught {
        surprise: Surprise { unaware: false, natural: arm.ambushes },
        turns_to: None,
        tries: tries.unwrap_or(0),
    };
    let facing = facing.unwrap_or(Row::Front);
    if arm.reach > Reach::Short || facing == attacker_on {
        return caught;
    }
    let watching = target.defending()
        && !vitals.is_dead()
        && vitals.stamina.current > 0
        && vitals.conditions.worst().is_none_or(|worst| worst < Condition::Webbed)
        && !target.has(Condition::Blind);
    let noticed = tries.filter(|_| watching).is_some_and(|tries| {
        caught.tries = tries + 1;
        notices(rng, target.body.attributes[crate::content::characters::Attribute::Senses], tries)
    });
    if noticed {
        caught.turns_to = Some(attacker_on);
    } else {
        caught.surprise.unaware = facing.opposite() == Some(attacker_on);
    }
    caught
}

#[cfg(test)]
mod tests {
    use super::super::arena::{arena, arm, duel, monster, party};
    use super::*;
    use crate::game::characters::{Lasting, Vitals};
    use crate::game::combat::turn::{Action, Ordered, Spotted};
    use crate::game::world::hostility::Stance;
    use bevy::prelude::{Entity, Vec3, World};

    /// What a blow of `reach` from `side` catches of a target facing front.
    fn caught(world: &mut World, target: Entity, reach: Reach, side: Row) -> Caught {
        let mut arm = arm(80);
        arm.reach = reach;
        duel(world, (target, target), |target, _| {
            catch_unaware(&mut GameRng::seeded(0), &arm, (side, Some(Row::Front)), target)
        })
    }

    #[test]
    fn only_the_side_across_from_the_way_it_faces_catches_it() {
        let mut world = arena();
        let target = monster(&mut world, Vec3::ZERO, Stance::Hostile);
        for (side, unaware) in
            [(Row::Front, false), (Row::Right, false), (Row::Left, false), (Row::Rear, true)]
        {
            let got = caught(&mut world, target, Reach::Touch, side);
            assert_eq!(got.surprise.unaware, unaware, "{side:?}");
        }
        // Facing right, the left is across.
        let got = duel(&mut world, (target, target), |target, _| {
            let facing = (Row::Left, Some(Row::Right));
            catch_unaware(&mut GameRng::seeded(0), &arm(80), facing, target)
        });
        assert!(got.surprise.unaware);
    }

    #[test]
    fn only_a_touch_or_short_blow_can_catch_it_unaware() {
        let mut world = arena();
        let target = monster(&mut world, Vec3::ZERO, Stance::Hostile);
        for (reach, unaware) in [
            (Reach::Touch, true),
            (Reach::Short, true),
            (Reach::Long, false),
            (Reach::Extreme, false),
        ] {
            let got = caught(&mut world, target, reach, Row::Rear);
            assert_eq!(got.surprise.unaware, unaware, "{reach:?}");
        }
    }

    /// A member keeping watch: defending, and yet to try this round.
    fn watchful() -> (World, Entity) {
        let mut world = arena();
        let member = party(&mut world, 1).1[0];
        world.entity_mut(member).insert((Ordered(Action::Defend), Spotted(0)));
        (world, member)
    }

    #[test]
    fn a_defending_member_notices_for_certain_the_first_time_and_turns() {
        let (mut world, member) = watchful();
        let got = caught(&mut world, member, Reach::Touch, Row::Rear);
        assert!(!got.surprise.unaware);
        assert_eq!((got.turns_to, got.tries), (Some(Row::Rear), 1));
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
    fn a_member_not_alert_does_not_notice() {
        let unnoticed = |world: &mut World, member| {
            let got = caught(world, member, Reach::Touch, Row::Rear);
            got.surprise.unaware && got.turns_to.is_none() && got.tries == 0
        };
        for condition in [Condition::Blind, Condition::Webbed] {
            let (mut world, member) = watchful();
            world.get_mut::<Vitals>(member).unwrap().afflict(condition, Lasting::Rounds(9));
            assert!(unnoticed(&mut world, member), "{condition:?}");
        }
        let (mut world, member) = watchful();
        world.get_mut::<Vitals>(member).unwrap().stamina.current = 0;
        assert!(unnoticed(&mut world, member), "spent");
        let (mut world, member) = watchful();
        world.entity_mut(member).insert(Ordered(Action::Attack { berserk: false }));
        assert!(unnoticed(&mut world, member), "not defending");
        // Nothing worse than Turncoat is alert still.
        let (mut world, member) = watchful();
        world.get_mut::<Vitals>(member).unwrap().afflict(Condition::Turncoat, Lasting::Rounds(9));
        assert_eq!(caught(&mut world, member, Reach::Touch, Row::Rear).turns_to, Some(Row::Rear));
    }
}
