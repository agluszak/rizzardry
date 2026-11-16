//! What a monster that fights and makes neither a special attack nor a spell
//! does (`docs/rules/monsters.md` section 4): withdraw when it can strike
//! nobody, guard a fellow, hold back warily, or strike.

use bevy::prelude::*;

use super::aim::{Toward, Viewed, Views};
use super::conduct::Conduct;
use super::spell::NoSpell;
use super::{Deliberating, PartyGap};
use crate::content::characters::Attribute;
use crate::game::actors::arm::{Furthest, GuardReach};
use crate::game::actors::{Attributes, BlindFighting, CanGuard, Level};
use crate::game::characters::{Condition, Conditions, Vitals};
use crate::game::combat::turn::{Decided, Decision, Fights, Retreats};
use crate::game::rng::GameRng;
use crate::game::world::hostility::Stance;
use crate::game::world::population::Temper;

/// One with no attack, or one blind (and not fighting blind) with nothing in
/// reach, withdraws. One with someone in reach of the party may guard, or
/// hold back, each with a chance of its Intelligence in percent: a bodyguard
/// able to guard takes one of its wards at random (the others it views on
/// its side, within reach of its first arm, of a higher level than it and
/// under 40 percent of their health); failing that, a wary kind holds back.
/// Otherwise it strikes.
#[allow(clippy::type_complexity)]
pub(super) fn strike_or_guard(
    spent: On<Add<NoSpell>>,
    mut commands: Commands,
    deliberations: Query<(&Deliberating, &PartyGap, &Views)>,
    views: Query<(&Viewed, &Toward)>,
    monsters: Query<(
        (&Temper, &Attributes, &Conditions, &Level),
        (&Furthest, &GuardReach, Has<BlindFighting>, Has<CanGuard>),
    )>,
    fellows: Query<(&Vitals, &Level)>,
    places: crate::game::actors::Places,
    mut rng: ResMut<GameRng>,
) {
    let deliberation = spent.entity;
    let Ok((&Deliberating(monster), &PartyGap(gap), viewed)) = deliberations.get(deliberation)
    else {
        return;
    };
    commands.entity(deliberation).despawn();
    let Ok(((temper, a, conditions, level), (reach, guard_reach, fights_blind, can_guard))) =
        monsters.get(monster)
    else {
        return;
    };
    let decided = |decision| Decided { actor: monster, decision, own_will: true };
    let blind = conditions.has(Condition::Blind) && !fights_blind;
    if reach.0.is_none() || (blind && !reach.reaches(gap)) {
        let retreats = Decision::Retreat(Retreats { conduct: Conduct::Withdraw, quiet: false });
        return commands.trigger(decided(retreats));
    }
    let fights = |waits| Decision::Fight(Fights { waits, berserk: false });
    if !reach.reaches(gap) {
        return commands.trigger(decided(fights(false)));
    }
    let temper = &temper.0;
    let intelligence = a[Attribute::Intelligence];
    let wards: Vec<Entity> = match guard_reach.0 {
        Some(guards) if temper.bodyguard && can_guard && rng.below(100) < intelligence => views
            .iter_many(viewed.iter())
            .flatten()
            .filter(|&(&Viewed(other), toward)| other != monster && toward.0 == Stance::Friendly)
            .map(|(&Viewed(other), _)| other)
            .filter(|&other| {
                fellows.get(other).is_ok_and(|(vitals, theirs_level)| {
                    let health = vitals.health;
                    places.gap(monster, other) <= guards.metres()
                        && theirs_level > level
                        && health.current * 100 / health.max.max(1) < 40
                })
            })
            .collect(),
        _ => Vec::new(),
    };
    commands.trigger(decided(if !wards.is_empty() {
        Decision::Protect(wards[rng.below(wards.len() as u32) as usize])
    } else {
        fights(temper.wary && rng.below(100) < intelligence)
    }));
}
