//! A monster's special attack at its moment (`docs/rules/monsters.md`
//! sections 4 and 7): a breath, or a summoning of its group.

use bevy::prelude::*;

use super::aim::{AimsMade, At, By, Spoiled, Weigh, Worth};
use super::slot::{Breath, Slots};
use super::{ClosesIn, Deliberating, PartyGap};
use crate::content::species::{Reach, SpecialAttack};
use crate::game::actors::arm::Furthest;
use crate::game::characters::{Condition, Conditions, Vitals};
use crate::game::combat::turn::{Breathes, Decided, Decision, SpecialCooldown};
use crate::game::hits::specials::special;
use crate::game::magic::reach::Aim;
use crate::game::rng::GameRng;
use crate::game::world::hostility::{Stance, Standing};
use crate::game::world::population::Special;

/// Its special attack may be made, as far as the monster itself goes (retail
/// `CanMonsterFlee`, `UpdateMonsterAI`): it has one with a chance, it is not
/// cooling down, its stamina is at least a tenth of its most, and a silence
/// does not stop it (only one that honours silence is stopped).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct SpecialReady;

/// Whether a special attack calls up its group rather than breathes.
pub fn summons(attack: &SpecialAttack) -> bool {
    attack.kind.is_some_and(|kind| special(kind).summons.is_some())
}

/// Being ready for its special attack follows the attack, its cooldown, its
/// stamina and its silence.
#[allow(clippy::type_complexity)]
pub(crate) fn ready(
    monsters: Query<(
        Entity,
        &Special,
        Option<&SpecialCooldown>,
        &Vitals,
        &Conditions,
        Has<SpecialReady>,
    )>,
    mut commands: Commands,
) {
    for (monster, attack, cooldown, vitals, conditions, was) in &monsters {
        let attack = attack.0;
        let silenced = attack
            .kind
            .is_some_and(|kind| conditions.has(Condition::Silenced) && special(kind).silenced);
        let now = attack.kind.is_some()
            && attack.chance != 0
            && cooldown.is_none_or(|cooldown| cooldown.0 == 0)
            && vitals.stamina.current >= vitals.stamina.max / 10
            && !silenced;
        match (now, was) {
            (true, false) => {
                commands.entity(monster).insert(SpecialReady);
            }
            (false, true) => {
                commands.entity(monster).remove::<SpecialReady>();
            }
            _ => {}
        }
    }
}

/// A deliberation that weighed the special attack and did not make it.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct NoSpecial;

/// A monster that fights and does not close in makes its special attack if
/// it is ready and placed: a summoning needs the party within long reach
/// and the summoner not friendly to it; a breath needs an aim, as a cone
/// spell's, so its breath's slot is weighed as a spell's is.
#[allow(clippy::type_complexity)]
pub(super) fn special_attack(
    weighed: On<Add<ClosesIn>>,
    mut commands: Commands,
    deliberations: Query<(&Deliberating, &PartyGap, &ClosesIn)>,
    monsters: Query<(&Special, &Standing, Option<&Slots>), With<SpecialReady>>,
    breaths: Query<(), With<Breath>>,
) {
    let deliberation = weighed.entity;
    let Ok((&Deliberating(monster), &PartyGap(gap), &ClosesIn(closes))) =
        deliberations.get(deliberation)
    else {
        return;
    };
    let (false, Ok((Special(attack), standing, slots))) = (closes, monsters.get(monster)) else {
        commands.entity(deliberation).insert(NoSpecial);
        return;
    };
    if summons(attack) {
        let placed = gap <= Reach::Long.metres() && standing.stance != Stance::Friendly;
        let placed = if placed { Placed::Summoning } else { Placed::Not };
        commands.trigger(Makes { deliberation, placed });
        return;
    }
    let breath = slots.and_then(|slots| slots.iter().find(|&slot| breaths.contains(slot)));
    if let Some(slot) = breath {
        commands.trigger(Weigh { slot, considering: deliberation });
    }
    commands.trigger(ConeWeighed { deliberation });
}

/// A breath's cone has been weighed.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct ConeWeighed {
    #[event_target]
    deliberation: Entity,
}

/// A breath with an aim worth making that spoils nothing is aimed at one of
/// them at random; one with none is not made.
fn aim_breath(
    breathes: On<ConeWeighed>,
    mut commands: Commands,
    made: Query<&AimsMade>,
    aims: Query<(&By, &At), (With<Worth>, Without<Spoiled>)>,
    breaths: Query<(), With<Breath>>,
    mut rng: ResMut<GameRng>,
) {
    let deliberation = breathes.deliberation;
    let made = made.get(deliberation).into_iter().flat_map(|made| made.iter());
    let aims: Vec<_> = aims
        .iter_many(made)
        .flatten()
        .filter(|(by, _)| breaths.contains(by.0))
        .map(|(_, at)| at.0)
        .collect();
    let placed = match aims.len() {
        0 => Placed::Not,
        count => Placed::Breath(aims[rng.below(count as u32) as usize]),
    };
    commands.trigger(Makes { deliberation, placed });
}

/// Where a special attack would go.
#[derive(Clone, Copy, Debug)]
enum Placed {
    /// A summoning, which needs no aim.
    Summoning,
    /// A breath, at this aim.
    Breath(Aim),
    /// Nowhere: it is not made.
    Not,
}

/// A special attack, placed or not.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct Makes {
    #[event_target]
    deliberation: Entity,
    placed: Placed,
}

/// A special attack placed is made if its chance comes up: the species'
/// while one of its attacks reaches the party, 100 when none does. It costs a
/// tenth of the monster's most stamina, and catches whoever stands in its
/// cone; a summoning calls up its group (section 7).
#[allow(clippy::type_complexity)]
fn make(
    makes: On<Makes>,
    mut commands: Commands,
    deliberations: Query<(&Deliberating, &PartyGap)>,
    monsters: Query<(&Special, &Furthest, &Vitals, &crate::game::actors::Level)>,
    places: crate::game::actors::Places,
    mut rng: ResMut<GameRng>,
) {
    let Makes { deliberation, placed } = *makes;
    let Ok((&Deliberating(monster), &PartyGap(gap))) = deliberations.get(deliberation) else {
        return;
    };
    let Ok((Special(attack), &reach, vitals, level)) = monsters.get(monster) else { return };
    let chance = if reach.reaches(gap) { u32::from(attack.chance) } else { 100 };
    let toward = match placed {
        Placed::Not => None,
        Placed::Summoning => Some(monster),
        Placed::Breath(aim) => Some(aim.body().unwrap_or(monster)),
    };
    let Some(toward) = toward.filter(|_| rng.below(100) < chance) else {
        commands.entity(deliberation).insert(NoSpecial);
        return;
    };
    let kind = attack.kind.expect("one that is ready has an attack");
    commands.trigger(Decided {
        actor: monster,
        decision: Decision::Special(Breathes {
            special: Some(kind),
            summons: special(kind).summons,
            strength: crate::game::hits::specials::strength(level.0 as u8, vitals.stamina),
            toward: places.of(toward).feet,
            cost: vitals.stamina.max / 10,
            cooldown: attack.cooldown,
        }),
        own_will: true,
    });
    commands.entity(deliberation).despawn();
}

pub(super) fn plugin(app: &mut App) {
    app.add_observer(special_attack).add_observer(aim_breath).add_observer(make);
}
