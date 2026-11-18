//! A monster's conduct at its moment in a fight (`docs/rules/monsters.md`
//! section 4): run, withdraw, hold back, or fight on.

use bevy::prelude::*;

use super::{Begun, PartyGap};
use crate::content::species::Reach;
use crate::game::characters::Vitals;
use crate::game::combat::monster::aim::Considers;
use crate::game::combat::turn::Fleeing;
use crate::game::npcs::Embodies;
use crate::game::rng::GameRng;
use crate::game::world::hostility::{Loyalty, Stance, Standing};
use crate::game::world::population::Temper;

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

/// A deliberation's conduct.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Conducts(pub Conduct);

/// A monster's conduct, the party `gap` metres off (section 4). A bystander
/// only ever holds. Otherwise it runs with a chance in percent: 100 for an
/// NPC not hostile at a third of its health or less; for one already
/// running, 100 while the party is beyond short reach and none once it is
/// cornered, which ends the running; else its kind's own chance, a craven
/// kind without one always running. One that does not run withdraws if it
/// is neutral (or holds, if its kind keeps hidden) and fights otherwise.
/// Whether it runs for its life from now on is marked on it.
#[allow(clippy::type_complexity)]
pub(super) fn conduct(
    begun: On<Begun>,
    mut commands: Commands,
    deliberations: Query<(&Considers, &PartyGap)>,
    monsters: Query<(&Standing, &Vitals, &Temper, Has<Fleeing>, Has<Embodies>)>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let deliberation = begun.deliberation;
    let (&Considers(monster), &PartyGap(gap)) = deliberations.get(deliberation)?;
    let (standing, vitals, temper, fleeing, npc) = monsters.get(monster)?;
    let temper = &temper.0;
    let (conduct, runs_on) = if standing.loyalty == Loyalty::Bystander {
        (Conduct::Hold, fleeing)
    } else {
        let hurt = vitals.health.current * 100 / vitals.health.max.max(1) <= 33;
        let (chance, going_on) = if npc && standing.stance != Stance::Hostile && hurt {
            (100, fleeing)
        } else if fleeing {
            if gap > Reach::Short.metres() { (100, true) } else { (0, false) }
        } else if temper.flee_chance == 0 && temper.craven {
            (100, false)
        } else {
            (u32::from(temper.flee_chance), false)
        };
        if rng.below(100) < chance {
            (Conduct::Flee, true)
        } else {
            let conduct = match standing.stance {
                Stance::Neutral if temper.camouflaged => Conduct::Hold,
                Stance::Neutral => Conduct::Withdraw,
                _ => Conduct::Fight,
            };
            (conduct, going_on)
        }
    };
    match (fleeing, runs_on) {
        (false, true) => {
            commands.entity(monster).insert(Fleeing);
        }
        (true, false) => {
            commands.entity(monster).remove::<Fleeing>();
        }
        _ => {}
    }
    commands.entity(deliberation).insert(Conducts(conduct));
    Ok(())
}
