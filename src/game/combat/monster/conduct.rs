//! A monster's conduct at its moment in a fight (`docs/rules/monsters.md`
//! section 4): run, withdraw, hold back, or fight on.

use super::sight::MindItem;
use crate::content::species::Reach;
use crate::game::hits::effects::BodyReadOnlyItem;
use crate::game::rng::GameRng;
use crate::game::world::hostility::{Loyalty, Stance};

/// How a monster carries itself this moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conduct {
    /// Run for its life.
    Flee,
    /// Back away from the party.
    Withdraw,
    /// Stand where it is and do nothing.
    Hold,
    Fight,
}

/// `current` as a whole percent of `most`.
pub fn share(current: u32, most: u32) -> u32 {
    current * 100 / most.max(1)
}

/// The conduct chosen, and whether it is running for its life from now on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Conducted {
    pub conduct: Conduct,
    pub fleeing: bool,
}

/// A monster's conduct, the party `gap` metres off. A bystander only ever
/// holds. Otherwise it runs with a chance in percent: 100 for an NPC not
/// hostile at a third of its health or less; for one already running, 100
/// while the party is beyond short reach and none once it is cornered, which
/// ends the running; else its kind's own chance, a craven kind without one
/// always running. One that does not run withdraws if it is neutral (or
/// holds, if its kind keeps hidden) and fights otherwise.
pub fn conduct(rng: &mut GameRng, own: &BodyReadOnlyItem, mind: &MindItem, gap: f32) -> Conducted {
    let (temper, stance) = (mind.temper.0, own.stance());
    if *own.loyalty == Loyalty::Bystander {
        return Conducted { conduct: Conduct::Hold, fleeing: mind.fleeing };
    }
    let hurt = share(own.vitals.health.current, own.vitals.health.max) <= 33;
    let (chance, going_on) = if mind.npc && stance != Stance::Hostile && hurt {
        (100, mind.fleeing)
    } else if mind.fleeing {
        if gap > Reach::Short.metres() { (100, true) } else { (0, false) }
    } else if temper.flee_chance == 0 && temper.craven {
        (100, false)
    } else {
        (u32::from(temper.flee_chance), false)
    };
    if rng.below(100) < chance {
        return Conducted { conduct: Conduct::Flee, fleeing: true };
    }
    let conduct = match stance {
        Stance::Neutral if temper.camouflaged => Conduct::Hold,
        Stance::Neutral => Conduct::Withdraw,
        _ => Conduct::Fight,
    };
    Conducted { conduct, fleeing: going_on }
}
