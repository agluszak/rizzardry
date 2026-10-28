//! Source-aware finalization shared by immediate casts and accepted missiles.
use super::{
    LearnedSpells, SpellPoints, Strength,
    effects::{self, Effect, Field, Victim},
};
use crate::{
    content::spells::{SpellCatalog, SpellDefinition, SpellId},
    game::rng::GameRng,
};
use bevy::prelude::*;

/// Boiling Blood finishes with an explosion only if its original victim is
/// dead. Corpse poses stay on their actors; no second position is retained.
pub(super) fn resolve(
    caster: Entity,
    backfired: bool,
    spell: &SpellDefinition,
    strength: Strength,
    victims: &[Victim],
    party_at: Vec3,
    spells: &SpellCatalog,
    rng: &mut GameRng,
    world: &mut Field,
    points: &mut Query<(&LearnedSpells, &mut SpellPoints)>,
) -> Vec<Effect> {
    let mut effects = effects::resolve(backfired, spell, strength, victims, rng, world, points);
    if spell.id != SpellId::BOILING_BLOOD {
        return effects;
    }
    let Some(explosion) = spells.get(SpellId::BOILING_BLOOD_EXPLOSION) else { return effects };
    for &victim in victims {
        let centre = match victim {
            Victim::Member(member) => world
                .members
                .get(member)
                .ok()
                .filter(|(_, vitals, ..)| vitals.is_dead())
                .map(|_| party_at),
            Victim::Monster(monster) => world
                .monsters
                .monsters()
                .get(monster)
                .ok()
                .filter(|(body, ..)| body.is_dead())
                .map(|(_, at, ..)| at.translation()),
        };
        let Some(centre) = centre else { continue };
        // CastSpellFromSource recomputes difficulty for the secondary spell,
        // preserving the original power and Power Cast percentage.
        let figure = world.members.get(caster).map_or(0, |(profile, ..)| {
            super::total_caster_level(profile, explosion.books).max(0) as u32
        });
        let strength = Strength::at(figure, explosion, strength.power, strength.boost);
        let radius = explosion.radius + explosion.radius_per_power * f32::from(strength.power);
        let victims = super::targeting::radius(world, centre, centre, radius, Some(party_at));
        effects.extend(effects::resolve(false, explosion, strength, &victims, rng, world, points));
    }
    effects
}

pub(super) fn ready(spell: SpellId, spells: &SpellCatalog) -> bool {
    spell != SpellId::BOILING_BLOOD || spells.get(SpellId::BOILING_BLOOD_EXPLOSION).is_some()
}
