//! The racial or spell-granted breath action shares aim capture and damage
//! with casting; it spends stamina, never spell points or casting practice.
use crate::{
    content::{
        characters::Trait,
        spells::{SpellCatalog, SpellId},
    },
    game::{
        characters::{Character, Vitals},
        magic::{
            LearnedSpells, SpellPoints,
            casting::{self, Aim, Refusal, Situation},
            effects::{self, Effect, Field},
        },
        rng::GameRng,
    },
};
use bevy::prelude::*;

pub fn available(profile: &Character, vitals: &Vitals) -> bool {
    vitals.can_fight()
        && (profile.has_trait(Trait::Breathe)
            || vitals.enchantments.get(SpellId::DRACON_BREATH).is_some())
        && vitals.stamina >= profile.stats(&vitals.enchantments).max_stamina() / 5
}

pub(super) fn execute(
    actor: Entity,
    target: Entity,
    situation: &Situation,
    spells: &SpellCatalog,
    rng: &mut GameRng,
    world: &mut Field,
    points: &mut Query<(&LearnedSpells, &mut SpellPoints)>,
) -> Result<Vec<Effect>, Refusal> {
    let (profile, vitals, ..) = world.members.get(actor).map_err(|_| Refusal::CannotAct)?;
    if !available(profile, vitals) {
        return Err(Refusal::CannotAct);
    }
    let cone = spells.get(SpellId::SPECIAL_ATTACK_CONE).ok_or(Refusal::MissingContent)?;
    let (mut victims, _) = casting::capture(actor, cone, Aim::Monster(target), situation, false)?;
    let species = world.species.ok_or(Refusal::MissingContent)?;
    victims.retain(|victim| match *victim {
        effects::Victim::Monster(entity) => {
            world.monsters.monsters().get(entity).is_ok_and(|(body, ..)| {
                species.kind(body.species).is_some_and(|kind| !kind.untargetable)
            })
        }
        _ => false,
    });
    if victims.is_empty() {
        return Err(Refusal::NoTarget);
    }
    let maximum = profile.stats(&vitals.enchantments).max_stamina();
    let breather =
        effects::Breather { level: profile.level(), stamina: vitals.stamina, max_stamina: maximum };
    let effects =
        effects::special_attack(effects::CHARACTER_BREATH, breather, &victims, rng, world, points);
    world.members.get_mut(actor).unwrap().1.spend_stamina(maximum / 5);
    Ok(effects)
}
