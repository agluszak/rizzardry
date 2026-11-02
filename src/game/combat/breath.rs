//! The racial or spell-granted breath action shares aim capture and damage
//! with casting; it spends stamina, never spell points or casting practice.
use crate::game::hits::{Act, Deed, specials};
use crate::{
    content::{characters::Trait, spells::SpellId},
    game::{
        characters::{Character, Vitals},
        magic::{
            casting::{self, Aim, Refusal, Situation},
            effects::Field,
        },
    },
};
use bevy::prelude::*;

pub fn available(profile: &Character, vitals: &Vitals) -> bool {
    vitals.can_fight()
        && (profile.has_trait(Trait::Breathe) || vitals.enchantments.has(SpellId::DRACON_BREATH))
        && vitals.stamina.current >= vitals.stamina.max / 5
}

/// Breathe at `target` and those beside it in the cone: a deed whose
/// effects the fight reports once they have landed.
pub(super) fn execute(
    commands: &mut Commands,
    actor: Entity,
    target: Entity,
    situation: &Situation,
    world: &mut Field,
) -> Result<(), Refusal> {
    let spells = world.spells;
    let (profile, vitals, ..) = world.members.get(actor).unwrap();
    if !available(profile, vitals) {
        return Err(Refusal::CannotAct);
    }
    let cone = &spells[SpellId::SPECIAL_ATTACK_CONE];
    let (mut victims, _) = casting::capture(actor, cone, Aim::Monster(target), situation, false)?;
    victims.retain(|&victim| world.monsters.targetable(victim));
    if victims.is_empty() {
        return Err(Refusal::NoTarget);
    }
    let deed = commands.spawn(Deed::new(Act::Fight)).id();
    let breather = (profile.level(), vitals.stamina);
    specials::breathe(commands, deed, specials::CHARACTER_BREATH, breather, &victims);
    let mut vitals = world.members.get_mut(actor).unwrap().1;
    let maximum = vitals.stamina.max;
    vitals.tire(maximum / 5);
    Ok(())
}
