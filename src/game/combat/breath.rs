//! The racial or spell-granted breath action shares aim capture and damage
//! with casting; it spends stamina, never spell points or casting practice.
use crate::{
    content::{characters::Trait, spells::SpellId},
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
        && (profile.has_trait(Trait::Breathe) || vitals.enchantments.has(SpellId::DRACON_BREATH))
        && vitals.stamina.current >= vitals.stamina.max / 5
}

pub(super) fn execute(
    actor: Entity,
    target: Entity,
    situation: &Situation,
    rng: &mut GameRng,
    world: &mut Field,
    points: &mut Query<(&LearnedSpells, &mut SpellPoints)>,
) -> Result<Vec<Effect>, Refusal> {
    let spells = world.spells;
    let (profile, vitals, ..) = world.members.get(actor).unwrap();
    if !available(profile, vitals) {
        return Err(Refusal::CannotAct);
    }
    let cone = &spells[SpellId::SPECIAL_ATTACK_CONE];
    let (mut victims, _) = casting::capture(actor, cone, Aim::Monster(target), situation, false)?;
    victims.retain(|victim| match *victim {
        effects::Victim::Monster(entity) => world.monsters.targetable(entity),
        _ => false,
    });
    if victims.is_empty() {
        return Err(Refusal::NoTarget);
    }
    let maximum = vitals.stamina.max;
    let breather = effects::Breather {
        level: profile.level(),
        stamina: vitals.stamina.current,
        max_stamina: maximum,
    };
    let effects =
        effects::special_attack(effects::CHARACTER_BREATH, breather, &victims, rng, world, points);
    world.members.get_mut(actor).unwrap().1.tire(maximum / 5);
    Ok(effects)
}
