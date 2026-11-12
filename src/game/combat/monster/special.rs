//! A monster's special attack, a breath or a summoning (`docs/rules/monsters.md`
//! section 4, "The special attack").

use super::close::{closes_in, reaches_party};
use super::sight::MindItem;
use crate::content::species::Reach;
use crate::game::characters::Condition;
use crate::game::hits::effects::BodyReadOnlyItem;
use crate::game::hits::specials::special;
use crate::game::magic::reach::Aim;
use crate::game::rng::GameRng;
use crate::game::world::hostility::Stance;

/// A special attack made: at what, and the stamina it costs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Made {
    /// Where a breath is aimed; a summoning has no aim.
    pub aim: Option<Aim>,
    pub cost: u32,
}

/// Whether its special attack summons.
pub fn summons(mind: &MindItem) -> bool {
    mind.special.0.kind.is_some_and(|kind| special(kind).summons.is_some())
}

/// Whether a monster's special attack is ready, the party `gap` metres off:
/// it has one with a chance, not cooling down, a tenth of its stamina to
/// spend, and no silence on it that the attack honours. A breath needs one
/// of `aims`; a summoning needs a fight under way, the party within long
/// reach, and a summoner not friendly to it.
pub fn ready(
    own: &BodyReadOnlyItem,
    mind: &MindItem,
    gap: f32,
    fighting: bool,
    aims: &[Aim],
) -> bool {
    let Some(kind) = mind.special.0.kind else { return false };
    let stamina = own.vitals.stamina;
    let silenced = own.conditions.has(Condition::Silenced) && special(kind).silenced;
    let placed = if summons(mind) {
        fighting && gap <= Reach::Long.metres() && own.stance() != Stance::Friendly
    } else {
        !aims.is_empty()
    };
    mind.special.0.chance != 0
        && mind.cooldown.is_none_or(|cooldown| cooldown.0 == 0)
        && stamina.current >= stamina.max / 10
        && !silenced
        && placed
}

/// Whether a monster makes its special attack now: not if it would close in;
/// if it is ready, with its chance while an arm reaches the party and for
/// sure when none does, at one of the aims, for a tenth of its stamina.
pub fn makes(
    rng: &mut GameRng,
    own: &BodyReadOnlyItem,
    mind: &MindItem,
    gap: f32,
    fighting: bool,
    aims: &[Aim],
) -> Option<Made> {
    if closes_in(own, mind, gap) || !ready(own, mind, gap, fighting, aims) {
        return None;
    }
    let aim = (!summons(mind)).then(|| aims[rng.below(aims.len() as u32) as usize]);
    let chance =
        if reaches_party(own.attacks, gap) { u32::from(mind.special.0.chance) } else { 100 };
    (rng.below(100) < chance).then_some(Made { aim, cost: own.vitals.stamina.max / 10 })
}
