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
        && target.body.conditions.below(Condition::Webbed)
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
