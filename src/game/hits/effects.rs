//! Immediate effect operations. Descriptions and rolled values are ordinary data;
//! only the act and independently delayed impacts have entity lifetimes.

use bevy::prelude::*;

use super::defences::{self, Immunities, Resistances, SaveLevel, Senses};
use super::definitions::{EffectKind, EffectSpec};
use super::{Deed, Effect};
use crate::content::characters::{Attribute, Realm, Skill, Trait};
use crate::content::items::{EquipClass, ItemCatalog};
use crate::content::spells::SpellId;
use crate::game::characters::{Character, Condition, Lasting, Vitals};
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::inventory::{Carried, PartyInventory};
use crate::game::magic::enchantments::{Enchantment, Enchantments, PartyEffects};
use crate::game::magic::{LearnedSpells, SpellPoints, Strength};
use crate::game::party::DamageApplied;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::hostility::{Side, Stance};
use crate::game::world::population::SpellBudget;

#[derive(Component, Debug)]
pub struct InFlight(pub Timer);

/// The percent chance the effect happens at all.
#[derive(Clone, Copy, Debug)]
pub struct Chance(pub u32);

/// The body may save against it outright, from this base.
#[derive(Clone, Copy, Debug)]
pub struct Save(pub i32);

/// The body's resistance in `realm` cuts it, against its act's strength;
/// saves against it are made in that realm too.
#[derive(Clone, Copy, Debug)]
pub struct Resist {
    pub realm: Realm,
}

/// It is saved against and resisted at half its act's strength.
#[derive(Clone, Debug)]
pub struct HalfStrength;

/// How an effect was settled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Landed,
    Resisted,
    /// Nothing to act on: the body was dead, gone, or not of the kind.
    Missed,
}

/// A condition; `indefinite` lasts until lifted, whatever its magnitude;
/// a poison bites by `poison`.
#[derive(Clone, Copy, Debug)]
pub struct Afflict {
    pub condition: Condition,
    pub poison: u32,
    pub indefinite: bool,
}

/// A member's spell points in one realm; in every realm when `None`.
#[derive(Clone, Copy, Debug)]
pub struct DrainRealm(pub Option<Realm>);

/// Lift these conditions, by its act's strength, or outright.
#[derive(Clone, Copy, Debug)]
pub struct Cure {
    pub conditions: &'static [Condition],
    pub outright: bool,
}

/// Shorten the clouds hanging over the body; `level` is the purifying
/// spell's.
#[derive(Clone, Copy, Debug)]
pub struct Purify {
    pub level: u32,
}

/// A spell laid on the body for its magnitude's rounds at its act's
/// strength.
#[derive(Clone, Copy, Debug)]
pub struct Enchant {
    pub spell: SpellId,
}

pub fn fly(mut commands: Commands, mut flying: Query<(Entity, &mut InFlight)>, time: Res<Time>) {
    for (reach, mut flight) in &mut flying {
        if flight.0.tick(time.delta()).is_finished() {
            commands.entity(reach).remove::<InFlight>();
        }
    }
}
fn record(acts: &mut Query<&mut Deed>, act: Entity, effect: Effect) {
    if let Ok(mut deed) = acts.get_mut(act) {
        deed.effects.push(effect);
    }
}
fn lasting(afflict: &Afflict, magnitude: u32) -> Lasting {
    match magnitude {
        _ if afflict.indefinite => Lasting::Indefinite,
        1 => Lasting::Rounds(2),
        rounds => Lasting::Rounds(rounds),
    }
}
fn purify_slots(
    slots: &mut Enchantments,
    amount: u32,
    level: u32,
    catalog: &crate::content::spells::SpellCatalog,
) {
    slots.0.retain_mut(|slot| {
        if crate::game::magic::enchantments::OFFENSIVE.contains(&slot.spell) {
            let cloud = &catalog[slot.spell];
            let removed = (amount + level).max(cloud.level) - cloud.level;
            slot.turns -= removed.min(slot.turns);
        }
        slot.turns > 0
    });
}
fn enchantment(
    enchant: &Enchant,
    turns: u32,
    strength: Strength,
    spells: &crate::content::spells::SpellCatalog,
    rng: &mut GameRng,
) -> Enchantment {
    let (spell, power) = (enchant.spell, strength.power);
    let absorption = if spell == SpellId::GUARDIAN_ANGEL {
        let rolled = spells[spell].dice.map_or(0, |dice| rng.roll(dice)) * power;
        rolled + rolled * strength.boost / 100
    } else {
        0
    };
    Enchantment { spell, power, boost: strength.boost, turns, absorption }
}
fn burn_stack(
    stack: crate::game::inventory::ItemStack,
    items: &ItemCatalog,
    chance: u32,
    rng: &mut GameRng,
    totals: &mut [u32; 3],
) -> Option<crate::game::inventory::ItemStack> {
    let category = match items[stack.item].class {
        EquipClass::Potion => 0,
        EquipClass::Scroll => 1,
        EquipClass::Spellbook => 2,
        _ => return Some(stack),
    };
    let lost = (0..stack.count()).filter(|_| rng.below(100) < chance).count() as u16;
    totals[category] += u32::from(lost);
    stack.remove_units(lost)
}
fn record_burnt(acts: &mut Query<&mut Deed>, act: Entity, totals: [u32; 3]) {
    let classes = [EquipClass::Potion, EquipClass::Scroll, EquipClass::Spellbook];
    for (class, amount) in classes.into_iter().zip(totals) {
        if amount != 0 {
            record(acts, act, Effect::Destroyed { class, amount });
        }
    }
}

#[derive(Clone, Debug)]
pub struct EffectInput {
    pub act: Entity,
    pub victim: Entity,
    pub strength: Strength,
    pub magnitude: u32,
    pub spec: EffectSpec,
}
#[derive(Debug)]
pub struct Applied {
    pub outcome: Outcome,
    pub dealt: Option<u32>,
}
impl Default for Applied {
    fn default() -> Self {
        Self { outcome: Outcome::Missed, dealt: None }
    }
}
fn harm(
    In(input): In<EffectInput>,
    mut bodies: Query<&mut Vitals>,
    mut acts: Query<&mut Deed>,
) -> Applied {
    let mut result = Applied::default();
    let victim = input.victim;
    let act = input.act;
    let magnitude = input.magnitude;

    let Ok(mut vitals) = bodies.get_mut(victim) else { return result };
    let lost = vitals.hurt(magnitude);
    record(&mut acts, act, Effect::Damaged { victim, amount: lost });
    result.outcome = Outcome::Landed;
    result.dealt = Some(lost);

    result
}
fn sap(
    In(input): In<EffectInput>,
    mut members: Query<(&mut Character, &mut Vitals)>,
    mut acts: Query<&mut Deed>,
) -> Applied {
    let mut result = Applied::default();
    let victim = input.victim;
    let act = input.act;
    let magnitude = input.magnitude;

    let Ok((mut character, mut vitals)) = members.get_mut(victim) else {
        result.outcome = Outcome::Missed;
        return result;
    };
    let lost = vitals.drain_health(&mut character, magnitude);
    record(&mut acts, act, Effect::Damaged { victim, amount: lost });
    result.outcome = Outcome::Landed;
    result.dealt = Some(lost);

    result
}
fn afflict(
    In(input): In<EffectInput>,
    mut bodies: Query<(&mut Vitals, &Side)>,
    mut acts: Query<&mut Deed>,
) -> Applied {
    let mut result = Applied::default();
    let victim = input.victim;
    let act = input.act;
    let magnitude = input.magnitude;
    let EffectKind::Afflict(afflict) = &input.spec.kind else {
        return result;
    };

    let Ok((mut vitals, side)) = bodies.get_mut(victim) else { return result };
    let condition = afflict.condition;
    if condition == Condition::Turncoat && side.0 == Stance::Neutral {
        result.outcome = Outcome::Missed;
        return result;
    }
    let lasting = lasting(afflict, magnitude);
    if condition == Condition::Poisoned {
        vitals.poison(lasting, afflict.poison);
    } else {
        vitals.inflict(condition, lasting);
    }
    record(&mut acts, act, Effect::Inflicted { victim, condition });
    result.outcome = Outcome::Landed;

    result
}
fn slay(
    In(input): In<EffectInput>,
    mut bodies: Query<&mut Vitals>,
    mut acts: Query<&mut Deed>,
) -> Applied {
    let mut result = Applied::default();
    let victim = input.victim;
    let act = input.act;

    let Ok(mut vitals) = bodies.get_mut(victim) else { return result };
    let lost = vitals.slay();
    record(&mut acts, act, Effect::Damaged { victim, amount: lost });
    result.outcome = Outcome::Landed;
    result.dealt = Some(lost);

    result
}
fn tire(
    In(input): In<EffectInput>,
    mut bodies: Query<&mut Vitals>,
    mut acts: Query<&mut Deed>,
) -> Applied {
    let mut result = Applied::default();
    let victim = input.victim;
    let act = input.act;
    let magnitude = input.magnitude;

    let Ok(mut vitals) = bodies.get_mut(victim) else { return result };
    let amount = vitals.tire(magnitude);
    record(&mut acts, act, Effect::Tired { victim, amount });
    result.outcome = Outcome::Landed;

    result
}
fn drain_pool(
    In(input): In<EffectInput>,
    mut members: Query<(&Character, &Vitals, &LearnedSpells, &mut SpellPoints)>,
    bodies: Query<(), With<Vitals>>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) -> Applied {
    let mut result = Applied::default();
    let victim = input.victim;
    let act = input.act;
    let magnitude = input.magnitude;

    let Ok((character, vitals, learned, mut pool)) = members.get_mut(victim) else {
        if bodies.contains(victim) {
            result.outcome = Outcome::Missed;
        }
        return result;
    };
    let maximum = character.stats(&vitals.enchantments).max_points(learned, &content.spells);
    let amount = pool.drain(magnitude, &maximum, &mut rng);
    record(&mut acts, act, Effect::Drained { victim, amount });
    result.outcome = Outcome::Landed;

    result
}
fn drain_budget(
    In(input): In<EffectInput>,
    mut budgets: Query<&mut SpellBudget>,
    mut acts: Query<&mut Deed>,
) -> Applied {
    let mut result = Applied::default();
    let victim = input.victim;
    let act = input.act;
    let magnitude = input.magnitude;

    let Ok(mut budget) = budgets.get_mut(victim) else {
        result.outcome = Outcome::Missed;
        return result;
    };
    let lost = budget.0.min(magnitude);
    budget.0 -= lost;
    record(&mut acts, act, Effect::Drained { victim, amount: lost });
    result.outcome = Outcome::Landed;

    result
}
fn drain_realm(
    In(input): In<EffectInput>,
    mut members: Query<(&Character, &Vitals, &LearnedSpells, &mut SpellPoints)>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
) -> Applied {
    let mut result = Applied::default();
    let victim = input.victim;
    let act = input.act;
    let magnitude = input.magnitude;
    let EffectKind::DrainRealm(drain) = &input.spec.kind else {
        return result;
    };

    let Ok((character, vitals, learned, mut pool)) = members.get_mut(victim) else {
        result.outcome = Outcome::Missed;
        return result;
    };
    let maximum = character.stats(&vitals.enchantments).max_points(learned, &content.spells);
    let realms = drain.0.map_or(enumset::EnumSet::all(), enumset::EnumSet::only);
    let amount: u32 = realms.iter().map(|realm| pool.drain_realm(realm, magnitude, &maximum)).sum();
    record(&mut acts, act, Effect::Drained { victim, amount });
    result.outcome = Outcome::Landed;

    result
}
fn heal(
    In(input): In<EffectInput>,
    mut bodies: Query<&mut Vitals>,
    mut acts: Query<&mut Deed>,
) -> Applied {
    let mut result = Applied::default();
    let victim = input.victim;
    let act = input.act;
    let magnitude = input.magnitude;

    let Ok(mut vitals) = bodies.get_mut(victim) else { return result };
    let amount = vitals.heal(magnitude);
    let landed =
        if amount == 0 { Effect::Nothing { victim } } else { Effect::Healed { victim, amount } };
    record(&mut acts, act, landed);
    result.outcome = Outcome::Landed;

    result
}
fn refresh(
    In(input): In<EffectInput>,
    mut bodies: Query<&mut Vitals>,
    mut acts: Query<&mut Deed>,
) -> Applied {
    let mut result = Applied::default();
    let victim = input.victim;
    let act = input.act;
    let magnitude = input.magnitude;

    let Ok(mut vitals) = bodies.get_mut(victim) else { return result };
    let amount = vitals.rest(magnitude);
    let landed =
        if amount == 0 { Effect::Nothing { victim } } else { Effect::Refreshed { victim, amount } };
    record(&mut acts, act, landed);
    result.outcome = Outcome::Landed;

    result
}
fn replenish(
    In(input): In<EffectInput>,
    mut pools: Query<&mut SpellPoints>,
    mut acts: Query<&mut Deed>,
) -> Applied {
    let mut result = Applied::default();
    let victim = input.victim;
    let act = input.act;
    let magnitude = input.magnitude;

    let Ok(mut pool) = pools.get_mut(victim) else {
        result.outcome = Outcome::Missed;
        return result;
    };
    let amount = pool.restore_evenly(magnitude);
    let landed = if amount == 0 {
        Effect::Nothing { victim }
    } else {
        Effect::Replenished { victim, amount }
    };
    record(&mut acts, act, landed);
    result.outcome = Outcome::Landed;

    result
}
fn cure(
    In(input): In<EffectInput>,
    mut bodies: Query<&mut Vitals>,
    mut acts: Query<&mut Deed>,
    mut rng: ResMut<GameRng>,
) -> Applied {
    let mut result = Applied::default();
    let victim = input.victim;
    let act = input.act;
    let EffectKind::Cure(cure) = &input.spec.kind else {
        return result;
    };

    let strength = input.strength;
    let Ok(mut vitals) = bodies.get_mut(victim) else { return result };
    if cure.conditions.contains(&Condition::Poisoned) {
        vitals.weaken_poison(defences::cure_rounds(strength), &mut rng);
    }
    let lifted =
        defences::cure(&mut rng, &mut vitals.conditions, cure.conditions, strength, cure.outright);
    let landed = match lifted.first() {
        Some(&condition) => Effect::Cured { victim, condition },
        None => Effect::Nothing { victim },
    };
    record(&mut acts, act, landed);
    result.outcome = Outcome::Landed;

    result
}
fn renew(
    In(input): In<EffectInput>,
    mut members: Query<(&mut Character, &mut Vitals)>,
    mut acts: Query<&mut Deed>,
) -> Applied {
    let mut result = Applied::default();
    let victim = input.victim;
    let act = input.act;

    let Ok((mut character, mut vitals)) = members.get_mut(victim) else {
        result.outcome = Outcome::Missed;
        return result;
    };
    let lifted = vitals.lift_drain(&mut character);
    let landed = if lifted {
        Effect::Cured { victim, condition: Condition::Drained }
    } else {
        Effect::Nothing { victim }
    };
    record(&mut acts, act, landed);
    result.outcome = Outcome::Landed;

    result
}
fn raise(
    In(input): In<EffectInput>,
    mut members: Query<(&Character, &mut Vitals)>,
    mut acts: Query<&mut Deed>,
) -> Applied {
    let mut result = Applied::default();
    let victim = input.victim;
    let act = input.act;

    let Ok((character, mut vitals)) = members.get_mut(victim) else {
        result.outcome = Outcome::Missed;
        return result;
    };
    if !vitals.is_dead() {
        record(&mut acts, act, Effect::Nothing { victim });
        result.outcome = Outcome::Missed;
        return result;
    }
    let stats = character.stats(&vitals.enchantments);
    let share = (stats.attribute(Attribute::Piety) + stats.attribute(Attribute::Vitality)) >> 2;
    vitals.health.current = (vitals.health.max * share / 100).max(1);
    vitals.stamina.current = (vitals.stamina.max * share / 100).max(1);
    record(&mut acts, act, Effect::Raised { victim });
    result.outcome = Outcome::Landed;

    result
}
fn purify(
    In(input): In<EffectInput>,
    mut bodies: Query<&mut Vitals>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
) -> Applied {
    let mut result = Applied::default();
    let victim = input.victim;
    let act = input.act;
    let magnitude = input.magnitude;
    let EffectKind::Purify(purify) = &input.spec.kind else {
        return result;
    };

    let Ok(mut vitals) = bodies.get_mut(victim) else { return result };
    purify_slots(&mut vitals.enchantments, magnitude, purify.level, &content.spells);
    record(&mut acts, act, Effect::Purified { victim });
    result.outcome = Outcome::Landed;

    result
}
fn purify_party(
    In(input): In<EffectInput>,
    mut parties: Query<&mut PartyEffects>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
) -> Applied {
    let mut result = Applied::default();
    let victim = input.victim;
    let act = input.act;
    let magnitude = input.magnitude;
    let EffectKind::Purify(purify) = &input.spec.kind else {
        return result;
    };

    let Ok(mut party) = parties.get_mut(victim) else { return result };
    purify_slots(&mut party.0, magnitude, purify.level, &content.spells);
    record(&mut acts, act, Effect::Purified { victim });
    result.outcome = Outcome::Landed;

    result
}
fn enchant(
    In(input): In<EffectInput>,
    mut bodies: Query<&mut Vitals>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) -> Applied {
    use crate::game::magic::enchantments::haste;

    let mut result = Applied::default();
    let victim = input.victim;
    let act = input.act;
    let turns = input.magnitude;
    let EffectKind::Enchant(enchant) = &input.spec.kind else {
        return result;
    };

    let Ok(mut vitals) = bodies.get_mut(victim) else { return result };
    let strength = input.strength;
    let rolled = enchantment(enchant, turns, strength, &content.spells, &mut rng);
    let vitals = &mut *vitals;
    let changed = if vitals.is_dead() {
        false
    } else if enchant.spell == SpellId::HASTE {
        haste(&mut vitals.conditions, &mut vitals.enchantments, rolled)
    } else {
        vitals.enchantments.accepts(enchant.spell, strength.power)
            && vitals.enchantments.apply(rolled)
    };
    let spell = enchant.spell;
    let landed =
        if changed { Effect::Enchanted { victim, spell } } else { Effect::Nothing { victim } };
    record(&mut acts, act, landed);
    result.outcome = Outcome::Landed;

    result
}
fn enchant_party(
    In(input): In<EffectInput>,
    mut parties: Query<&mut PartyEffects>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) -> Applied {
    let mut result = Applied::default();
    let victim = input.victim;
    let act = input.act;
    let turns = input.magnitude;
    let EffectKind::Enchant(enchant) = &input.spec.kind else {
        return result;
    };

    let Ok(mut party) = parties.get_mut(victim) else { return result };
    let strength = input.strength;
    let rolled = enchantment(enchant, turns, strength, &content.spells, &mut rng);
    let changed = party.0.accepts(enchant.spell, strength.power) && party.0.apply(rolled);
    let spell = enchant.spell;
    let landed =
        if changed { Effect::Enchanted { victim, spell } } else { Effect::Nothing { victim } };
    record(&mut acts, act, landed);
    result.outcome = Outcome::Landed;

    result
}
fn drain_party(
    In(input): In<EffectInput>,
    parties: Query<&PartyRoster>,
    mut members: Query<(&Character, &Vitals, &LearnedSpells, &mut SpellPoints)>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) -> Applied {
    let mut result = Applied::default();
    let victim = input.victim;
    let act = input.act;
    let magnitude = input.magnitude;

    let Ok(roster) = parties.get(victim) else { return result };
    for victim in roster.iter() {
        let Ok((character, vitals, learned, mut pool)) = members.get_mut(victim) else {
            continue;
        };
        if vitals.is_dead() {
            continue;
        }
        let maximum = character.stats(&vitals.enchantments).max_points(learned, &content.spells);
        let amount = pool.drain(magnitude, &maximum, &mut rng);
        record(&mut acts, act, Effect::Drained { victim, amount });
    }
    result.outcome = Outcome::Landed;

    result
}
fn burn(
    In(input): In<EffectInput>,
    mut carrying: Query<&mut Carried>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) -> Applied {
    let items = &content.items;

    let mut result = Applied::default();
    let victim = input.victim;
    let act = input.act;
    let chance = input.magnitude;

    let Ok(mut carried) = carrying.get_mut(victim) else { return result };
    let mut totals = [0; 3];
    for (_, slot) in &mut carried.equipment {
        if let Some(equipped) = slot.take() {
            *slot = burn_stack(equipped.stack, items, chance, &mut rng, &mut totals).map(|stack| {
                crate::game::inventory::Equipped { stack, curse_released: equipped.curse_released }
            });
        }
    }
    for slot in &mut carried.backpack {
        if let Some(stack) = slot.take() {
            *slot = burn_stack(stack, items, chance, &mut rng, &mut totals);
        }
    }
    record_burnt(&mut acts, act, totals);
    result.outcome = Outcome::Landed;

    result
}
fn burn_pool(
    In(input): In<EffectInput>,
    mut parties: Query<&mut PartyInventory>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) -> Applied {
    let items = &content.items;

    let mut result = Applied::default();
    let victim = input.victim;
    let act = input.act;
    let chance = input.magnitude;

    let Ok(mut inventory) = parties.get_mut(victim) else { return result };
    let mut totals = [0; 3];
    let pool = &mut inventory.pool.stacks;
    *pool = std::mem::take(pool)
        .into_iter()
        .filter_map(|stack| burn_stack(stack, items, chance, &mut rng, &mut totals))
        .collect();
    record_burnt(&mut acts, act, totals);
    result.outcome = Outcome::Landed;

    result
}
fn lose_gold(
    In(input): In<EffectInput>,
    mut parties: Query<&mut PartyInventory>,
    mut acts: Query<&mut Deed>,
) -> Applied {
    let mut result = Applied::default();
    let victim = input.victim;
    let act = input.act;
    let percent = input.magnitude;

    let Ok(mut inventory) = parties.get_mut(victim) else { return result };
    let amount = inventory.gold * percent.min(100) / 100;
    inventory.gold -= amount;
    record(&mut acts, act, Effect::GoldLost { amount });
    result.outcome = Outcome::Landed;

    result
}
fn dispel(In(input): In<EffectInput>, mut parties: Query<&mut PartyEffects>) -> Applied {
    let mut result = Applied::default();
    let victim = input.victim;

    let Ok(mut party) = parties.get_mut(victim) else { return result };
    party.0.0.retain(|e| crate::game::magic::enchantments::combat_only(e.spell));
    result.outcome = Outcome::Landed;

    result
}

pub fn apply(world: &mut World, input: EffectInput) -> Applied {
    let party = world.get::<PartyEffects>(input.victim).is_some();
    macro_rules! run {
        ($f:ident) => {
            world.run_system_cached_with($f, input).expect("effect inputs available")
        };
    }
    match input.spec.kind {
        EffectKind::Harm => run!(harm),
        EffectKind::Sap => run!(sap),
        EffectKind::Afflict(_) => run!(afflict),
        EffectKind::Slay => run!(slay),
        EffectKind::Tire => run!(tire),
        EffectKind::DrainBudget => run!(drain_budget),
        EffectKind::DrainRealm(_) => run!(drain_realm),
        EffectKind::Heal => run!(heal),
        EffectKind::Refresh => run!(refresh),
        EffectKind::Replenish => run!(replenish),
        EffectKind::Cure(_) => run!(cure),
        EffectKind::Renew => run!(renew),
        EffectKind::Raise => run!(raise),
        EffectKind::LoseGold => run!(lose_gold),
        EffectKind::Dispel => run!(dispel),
        EffectKind::DrainPool => {
            if party {
                run!(drain_party)
            } else {
                run!(drain_pool)
            }
        }
        EffectKind::Purify(_) => {
            if party {
                run!(purify_party)
            } else {
                run!(purify)
            }
        }
        EffectKind::Enchant(_) => {
            if party {
                run!(enchant_party)
            } else {
                run!(enchant)
            }
        }
        EffectKind::Burn => {
            if party {
                run!(burn_pool)
            } else {
                run!(burn)
            }
        }
        EffectKind::Prismic(_) => unreachable!("Prismic is chosen before application"),
    }
}

/// Validate and resist one immediate effect before applying it. Rolls remain
/// in retail order: magnitude, dead check, chance, immunity, save, resistance.
pub fn defend(
    In(mut input): In<EffectInput>,
    bodies: Query<(&Vitals, &SaveLevel, &Resistances, &Immunities, &Side)>,
    mut characters: Query<&mut Character>,
    difficulty: Res<Difficulty>,
    mut rng: ResMut<GameRng>,
    mut acts: Query<&mut Deed>,
) -> (EffectInput, Option<Outcome>) {
    use EffectKind as K;
    let victim = input.victim;
    let body = bodies.get(victim).ok();
    let requires_life = matches!(
        input.spec.kind,
        K::Harm
            | K::Sap
            | K::Afflict(_)
            | K::Slay
            | K::Tire
            | K::DrainPool
            | K::DrainBudget
            | K::DrainRealm(_)
            | K::Heal
            | K::Refresh
            | K::Replenish
    );
    if requires_life && body.is_some_and(|(v, ..)| v.is_dead()) {
        return (input, Some(Outcome::Missed));
    }
    if input.spec.chance.is_some_and(|chance| rng.below(100) >= chance.0) {
        return (input, Some(Outcome::Missed));
    }
    let against = if input.spec.half { input.strength.against / 2 } else { input.strength.against };
    let affliction = match &input.spec.kind {
        K::Afflict(a) => Some(a),
        _ => None,
    };
    let mut resisted = body.is_some_and(|(_, _, _, immune, _)| match affliction {
        Some(a) => immune.conditions.contains(&a.condition),
        None => matches!(input.spec.kind, K::Slay) && immune.death,
    });
    if !resisted
        && let (Some(save), Some(resist), Some((_, level, resistance, _, side))) =
            (input.spec.save, input.spec.resist, body)
    {
        let chance =
            defences::condition_chance(level.0, resistance.0[resist.realm], against, save.0);
        resisted = defences::save_roll(&mut rng, difficulty.scale(chance, side.0));
        if resisted
            && let Ok(mut character) = characters.get_mut(victim)
            && character.practise(Skill::IronWill, 2, &mut rng)
            && let Ok(mut deed) = acts.get_mut(input.act)
        {
            deed.effects.push(Effect::Improved { victim, skill: Skill::IronWill });
        }
    }
    if !resisted
        && input.spec.roll.is_some()
        && let (Some(resist), Some((_, level, resistance, _, _))) = (input.spec.resist, body)
    {
        input.magnitude = defences::resist(
            &mut rng,
            input.magnitude,
            resistance.0[resist.realm],
            level.0,
            against,
        );
        resisted = input.magnitude == 0;
    }
    if !resisted
        && input.spec.save.is_some()
        && let Some(afflict) = affliction
        && let Ok(character) = characters.get(victim)
    {
        let android = character.has_trait(Trait::AndroidConditionImmunity);
        resisted = match afflict.condition {
            Condition::Afraid => character.has_trait(Trait::Fearless) || android,
            Condition::Diseased
            | Condition::Irritated
            | Condition::Nauseated
            | Condition::Poisoned
            | Condition::Asleep => android,
            Condition::Insane if afflict.indefinite => false,
            Condition::Insane | Condition::Turncoat => {
                character.has_trait(Trait::MentalConditionImmunity)
            }
            _ => false,
        };
    }
    if resisted {
        record(&mut acts, input.act, Effect::Resisted { victim });
    }
    (input, resisted.then_some(Outcome::Resisted))
}

/// Damage has coupled consequences; none needs a temporary effect entity.
pub fn hurt(
    In((act, victim, amount, harm)): In<(Entity, Entity, u32, bool)>,
    mut commands: Commands,
    mut bodies: Query<(
        &mut Vitals,
        Option<&Senses>,
        Has<Character>,
        Has<crate::game::combat::Combatant>,
    )>,
    mut lured: Query<(
        &crate::game::magic::pointcasts::LureInfluence,
        Option<&mut bevy_landmass::AgentTarget3d>,
    )>,
    mut drains: Query<&mut super::spells::Drains>,
    mut notices: MessageWriter<DamageApplied>,
    mut rng: ResMut<GameRng>,
) {
    let Ok((mut vitals, senses, member, fighting)) = bodies.get_mut(victim) else { return };
    if amount > 0 && member {
        notices.write(DamageApplied { member: victim, amount, died: vitals.is_dead() });
    }
    if !harm {
        return;
    }
    if let Ok(mut drains) = drains.get_mut(act) {
        drains.drained += amount;
    }
    if amount == 0 {
        return;
    }
    if let Ok((lure, goal)) = lured.get_mut(victim) {
        commands.entity(victim).insert(crate::game::magic::pointcasts::LureInfluence {
            response: crate::game::magic::pointcasts::LureResponse::Resisted,
            ..*lure
        });
        if !fighting && let Some(mut goal) = goal {
            *goal = bevy_landmass::AgentTarget3d::None;
        }
    }
    if !vitals.is_dead()
        && let Some(senses) = senses
    {
        vitals.conditions.wake_on_hit(senses.0, &mut rng);
    }
}
