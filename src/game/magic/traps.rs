//! Indirect trap spells: preflight content, capture real party actors and
//! settle consequences synchronously, before the device may unlock.

use super::{
    LearnedSpells, MagicContext, SpellPoints, Strength, Victim, casting::MagicNews, effects,
};
use crate::content::spells::{SpellDefinition, SpellId};
use crate::game::party::Members;
use crate::game::world::{damage::MonsterDamage, interaction::Unable};
use bevy::prelude::*;

pub const SPELLS: [SpellId; 15] = [
    SpellId::DAGGER_SCATTER,
    SpellId::POISON_DARTS,
    SpellId::ALARM,
    SpellId::INSANITY,
    SpellId::WHIPPING_ROCKS,
    SpellId::FIREBALL,
    SpellId::FREEZE_FLESH,
    SpellId::SUMMON_ELEMENTAL,
    SpellId::INSTANT_DEATH,
    SpellId::PHILOSOPHERS_BANE,
    SpellId::VAMPIRIC_VORTEX,
    SpellId::HEATWAVE,
    SpellId::PRISMIC_RAY,
    SpellId::ANTI_MAGIC,
    SpellId::CEREBRAL_HEMORRHAGE,
];

/// The spell a trap type discharges, if the game can resolve it.
fn trap_spell(trap: u8) -> Result<SpellId, Unable> {
    let id = *SPELLS.get(usize::from(trap)).ok_or(Unable::UnsupportedTrap)?;
    if !effects::modelled(id) {
        return Err(Unable::UnsupportedTrap);
    }
    Ok(id)
}

/// No attempted inspection/disarm may roll if its failure cannot be resolved.
pub(crate) fn validate(trap: u8) -> Result<(), Unable> {
    trap_spell(trap).map(|_| ())
}

/// What a trap's discharge needs: the party and monsters it may strike,
/// their spell points, the shared magic context, the dice and the clock.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Discharge<'w, 's> {
    actors: (Members<'w, 's>, MonsterDamage<'w, 's>),
    points: Query<'w, 's, (&'static LearnedSpells, &'static mut SpellPoints)>,
    context: MagicContext<'w, 's>,
    rng: ResMut<'w, crate::game::rng::GameRng>,
    clock: Res<'w, crate::game::time::GameClock>,
}

impl Discharge<'_, '_> {
    /// Fire a trap of this type and authored grade at the party: retail
    /// deletes random slots from roster order until as many remain as the
    /// trap strikes, keeping the member who set it off (sleeping and
    /// unconscious members may be struck too). Returns whether the trap is
    /// destroyed.
    pub fn fire(&mut self, trap: u8, difficulty: u8, selected: Option<Entity>) -> bool {
        let Self { actors: (members, monsters), points, context, rng, clock } = self;
        let mut targets: Vec<Entity> = context
            .roster
            .iter()
            .filter(|&actor| !members.get(actor).unwrap().1.is_dead())
            .collect();
        let (spell, effects, improved, destroyed) =
            context.resolve(clock.ms, members, monsters, |field| {
                let spell: &SpellDefinition = &field.spells[SPELLS[usize::from(trap)]];
                let difficulty = difficulty.clamp(1, 7);
                let destroyed = rng.below(2) == 0;
                let mut count = u32::from(difficulty) - rng.below(u32::from(difficulty / 2));
                let extra = difficulty.saturating_sub(
                    crate::game::world::interaction::TRAP_GRADE[usize::from(trap)] as u8,
                );
                let power = (extra + 4).min(7);
                if extra > 3 {
                    count += rng.below(u32::from(extra - 3));
                }
                while targets.len() > count as usize {
                    let index = rng.below(targets.len() as u32) as usize;
                    if Some(targets[index]) != selected {
                        targets.remove(index);
                    }
                }
                let victims: Vec<_> = targets.into_iter().map(Victim::Member).collect();
                let effects = effects::resolve(
                    None,
                    false,
                    spell,
                    Strength::at(0, spell, power, 0),
                    &victims,
                    rng,
                    field,
                    points,
                );
                (spell.id, effects, std::mem::take(&mut field.improved), destroyed)
            });
        context.news.write(MagicNews::Triggered { spell, effects, improved });
        destroyed
    }
}
