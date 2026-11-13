//! A monster guarding a fellow, or holding back warily (`docs/rules/monsters.md`
//! section 4, "Guarding a fellow" and "Holding back warily").

use bevy::prelude::Entity;

use super::close::reaches_party;
use super::conduct::share;
use super::sight::{MindItem, Sight};
use crate::content::characters::Attribute;
use crate::game::hits::effects::BodyReadOnlyItem;
use crate::game::rng::GameRng;
use crate::game::world::hostility::Stance;

/// What a monster does instead of striking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Watching {
    Guards(Entity),
    HoldsBack,
}

/// Whether a monster with an arm that reaches the party, `gap` metres off,
/// guards a fellow or holds back, each with a chance of its Intelligence in
/// percent. A bodyguard able to guard takes one of its wards at random: the
/// living among `bodies` on its side, within reach of its first arm, of a
/// higher level than it and under 40 percent of their health. Failing that,
/// a wary kind holds back.
pub fn guards(
    rng: &mut GameRng,
    sight: &Sight,
    own: &BodyReadOnlyItem,
    mind: &MindItem,
    gap: f32,
    bodies: &[Entity],
) -> Option<Watching> {
    if !reaches_party(own.attacks, gap) {
        return None;
    }
    let (temper, intelligence) = (mind.temper.0, own.attributes[Attribute::Intelligence]);
    let reach = own.attacks.arms.first().map(|arm| arm.reach.metres());
    let wards: Vec<Entity> = match reach {
        Some(reach) if temper.bodyguard && own.can_guard && rng.below(100) < intelligence => bodies
            .iter()
            .copied()
            .filter(|&other| other != own.entity)
            .filter(|&other| sight.toward(own, other) == Some(Stance::Friendly))
            .filter(|&other| sight.places.gap(own.entity, other) <= reach)
            .filter(|&other| {
                sight.bodies.get(other).is_ok_and(|fellow| {
                    let health = fellow.vitals.health;
                    fellow.level > own.level && share(health.current, health.max) < 40
                })
            })
            .collect(),
        _ => Vec::new(),
    };
    if !wards.is_empty() {
        Some(Watching::Guards(wards[rng.below(wards.len() as u32) as usize]))
    } else if temper.wary && rng.below(100) < intelligence {
        Some(Watching::HoldsBack)
    } else {
        None
    }
}
