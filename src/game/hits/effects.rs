//! What a strike does to the body it lands on: each effect of its
//! definition in turn, validated, saved against, resisted and applied, every
//! effect seeing what the one before it did. Plain functions over the state
//! their caller holds; nothing here is scheduled.

use bevy::ecs::query::QueryData;
use bevy::prelude::*;

use super::defences;
use super::definitions::{self, Blow, DefinitionId, Definitions, EffectKind, EffectSpec};
use super::{Damaged, Effect};
use crate::content::characters::{Attribute, Skill};
use crate::content::items::{EquipClass, ItemCatalog};
use crate::content::spells::SpellId;
use crate::game::actors::{Armour, Attacks, Attributes, Level, Resistances};
use crate::game::characters::{Character, Condition, Lasting, Vitals};
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::inventory::{Burden, Carried, Equipped, ItemStack, PartyInventory};
use crate::game::magic::enchantments::{self, Enchantment, Enchantments, PartyEffects};
use crate::game::magic::{LearnedSpells, SpellPoints, Strength};
use crate::game::party::roster::PartyMember;
use crate::game::rng::GameRng;
use crate::game::world::footing::Footing;
use crate::game::world::hostility::{Loyalty, Side, Stance};
use crate::game::world::population::{SpellBudget, Untargetable};

/// A body effects can land on, a member's or a monster's: what both carry,
/// and what only one of them does.
#[derive(QueryData)]
#[query_data(mutable)]
pub struct Body {
    pub entity: Entity,
    pub vitals: &'static mut Vitals,
    pub side: &'static Side,
    pub loyalty: &'static Loyalty,
    pub at: &'static Footing,
    pub attributes: &'static Attributes,
    pub level: &'static Level,
    pub resistances: &'static Resistances,
    pub armour: &'static Armour,
    pub attacks: &'static Attacks,
    pub character: Option<&'static mut Character>,
    pub spells: Option<(&'static LearnedSpells, &'static mut SpellPoints)>,
    pub budget: Option<&'static mut SpellBudget>,
    pub carried: Option<&'static mut Carried>,
    pub burden: Option<&'static Burden>,
    pub member: Has<PartyMember>,
    pub untargetable: Has<Untargetable>,
}

/// One thing landing: a definition's effects at a strength, a weapon's blow
/// behind them when it is one.
#[derive(Clone, Copy, Debug)]
pub struct Strike {
    pub definition: DefinitionId,
    pub strength: Strength,
    pub blow: Option<Blow>,
}

/// The rules a strike is settled by.
#[derive(Clone, Copy)]
pub struct Rules<'a> {
    pub content: &'a Catalogs,
    pub definitions: &'a Definitions,
    pub difficulty: Difficulty,
    /// The fight under way, in which a Valkyrie cheats death at full
    /// strength once.
    pub fight: Option<Entity>,
}

/// What strikes did, for whoever tells of it.
#[derive(Debug, Default)]
pub struct Log {
    pub effects: Vec<Effect>,
    pub improved: Vec<(Entity, Skill)>,
    pub damaged: Vec<Damaged>,
    /// The health plain harm took, which a leeching spell gives back.
    pub harm: u32,
}

/// How an effect was settled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Landed,
    Resisted,
    /// Nothing to act on: the body was dead, gone, or not of the kind.
    Missed,
}

/// The effects of a definition that apply, each with the weapon's chance
/// and poison when it stands for a weapon effect.
fn specs<'a>(
    strike: &'a Strike,
    rules: &Rules<'a>,
    party: bool,
) -> impl Iterator<Item = EffectSpec> + 'a {
    let blow = strike.blow;
    rules.definitions.get(strike.definition).effects.iter().filter_map(move |&spec| {
        if spec.party != party || (spec.by_monsters && !blow.is_some_and(|b| b.by_monster)) {
            return None;
        }
        let Some(effect) = spec.weapon else { return Some(spec) };
        let weapon = blow?.effects;
        let chance = weapon.chances[effect];
        let kind = match spec.kind {
            EffectKind::Afflict { condition: Condition::Poisoned, indefinite, .. } => {
                EffectKind::Afflict {
                    condition: Condition::Poisoned,
                    poison: weapon.strength,
                    indefinite,
                }
            }
            kind => kind,
        };
        (chance > 0).then_some(EffectSpec { chance: Some(chance), kind, ..spec })
    })
}

/// Whether an effect waits on the one before it, which did not come.
fn waits(spec: &EffectSpec, previous: Option<Outcome>) -> bool {
    (spec.landed && previous != Some(Outcome::Landed))
        || (spec.resisted && previous != Some(Outcome::Resisted))
}

/// Land a strike on a body (retail `ApplySpellEffectToTarget`,
/// `ApplyEffectConditions`): every effect for its kind in order.
pub fn land(strike: &Strike, body: &mut BodyItem, rules: &Rules, rng: &mut GameRng, log: &mut Log) {
    let mut previous = None;
    for spec in specs(strike, rules, false) {
        let kind = body.resistances.kind;
        if (spec.members && !body.member)
            || (spec.monsters && kind.is_none())
            || spec.kinds.is_some_and(|kinds| !kind.is_some_and(|k| kinds.contains(&k)))
        {
            continue;
        }
        if waits(&spec, previous) {
            previous = Some(Outcome::Missed);
            continue;
        }
        let spec = spec.pick(rng);
        let rolled = spec
            .roll
            .map_or(0, |roll| definitions::roll(roll, strike.strength, strike.blow.as_ref(), rng));
        let alive = !body.vitals.is_dead();
        previous = Some(match defend(&spec, rolled, strike.strength, body, rules, rng, log) {
            Ok(magnitude) => {
                let outcome = apply(&spec, magnitude, strike.strength, body, rules, rng, log);
                settle(body, alive, rules, rng);
                outcome
            }
            Err(outcome) => outcome,
        });
    }
}

/// A member's maxima follow what the effect changed, and a Valkyrie whose
/// health it ran out cheats death before anything else lands on her.
fn settle(body: &mut BodyItem, was_alive: bool, rules: &Rules, rng: &mut GameRng) {
    let Some(character) = body.character.as_deref() else { return };
    if body.vitals.maxima_differ(character) {
        body.vitals.sync_maxima(character);
    }
    if was_alive && body.vitals.is_dead() {
        body.vitals.cheats_death(character, rules.fight, rng);
    }
}

/// Validate and resist one effect before it is applied (retail order:
/// the dead, its chance, immunity, the save, resistance): the magnitude it
/// lands with, or how it was stopped.
fn defend(
    spec: &EffectSpec,
    mut magnitude: u32,
    strength: Strength,
    body: &mut BodyItem,
    rules: &Rules,
    rng: &mut GameRng,
    log: &mut Log,
) -> Result<u32, Outcome> {
    use EffectKind as K;
    let victim = body.entity;
    let requires_life = matches!(
        spec.kind,
        K::Harm
            | K::Sap
            | K::Afflict { .. }
            | K::Slay
            | K::Tire
            | K::DrainPool
            | K::DrainBudget
            | K::DrainRealm(_)
            | K::Heal
            | K::Refresh
            | K::Replenish
    );
    if requires_life && body.vitals.is_dead() {
        return Err(Outcome::Missed);
    }
    if spec.chance.is_some_and(|chance| rng.below(100) >= chance) {
        return Err(Outcome::Missed);
    }
    let against = if spec.half { strength.against / 2 } else { strength.against };
    let affliction = match spec.kind {
        K::Afflict { condition, indefinite, .. } => Some((condition, indefinite)),
        _ => None,
    };
    let mut resisted = match affliction {
        Some((condition, _)) => body.resistances.immune.contains(&condition),
        None => matches!(spec.kind, K::Slay) && body.resistances.death_immune,
    };
    if !resisted && let (Some(save), Some(realm)) = (spec.save, spec.resist) {
        let resistance = body.resistances.realms[realm];
        let chance = defences::condition_chance(body.level.0, resistance, against, save);
        resisted = defences::save_roll(rng, rules.difficulty.scale(chance, body.side.0));
        if resisted
            && let Some(character) = body.character.as_deref_mut()
            && character.practise(Skill::IronWill, 2, rng)
        {
            log.improved.push((victim, Skill::IronWill));
        }
    }
    if !resisted
        && spec.roll.is_some()
        && let Some(realm) = spec.resist
    {
        let resistance = body.resistances.realms[realm];
        magnitude = defences::resist(rng, magnitude, resistance, body.level.0, against);
        resisted = magnitude == 0;
    }
    // What it shrugs off whenever a save is allowed at all.
    if !resisted
        && spec.save.is_some()
        && let Some((condition, _)) = affliction
    {
        resisted = body.resistances.wards.contains(&condition);
    }
    if resisted {
        log.effects.push(Effect::Resisted { victim });
        return Err(Outcome::Resisted);
    }
    Ok(magnitude)
}

/// How long an affliction of `magnitude` lasts.
fn lasting(indefinite: bool, magnitude: u32) -> Lasting {
    match magnitude {
        _ if indefinite => Lasting::Indefinite,
        1 => Lasting::Rounds(2),
        rounds => Lasting::Rounds(rounds),
    }
}

/// A member's spell point maxima as their sheet and attributes make them.
fn maxima(
    character: &Character,
    attributes: &Attributes,
    learned: &LearnedSpells,
    content: &Catalogs,
) -> enum_map::EnumMap<crate::content::characters::Realm, u32> {
    character.max_points(attributes, learned, &content.spells)
}

/// Damage dealt and its coupled consequences: it is told of, plain harm
/// counts toward a leech and may wake a sleeper.
fn dealt(body: &mut BodyItem, amount: u32, harm: bool, rng: &mut GameRng, log: &mut Log) {
    let victim = body.entity;
    log.effects.push(Effect::Damaged { victim, amount });
    if amount > 0 {
        log.damaged.push(Damaged { body: victim, amount, died: body.vitals.is_dead() });
    }
    if harm {
        log.harm += amount;
        if amount > 0 && !body.vitals.is_dead() {
            body.vitals.conditions.wake_on_hit(body.attributes[Attribute::Senses], rng);
        }
    }
}

/// Apply one effect to a body, recording what it did.
fn apply(
    spec: &EffectSpec,
    magnitude: u32,
    strength: Strength,
    body: &mut BodyItem,
    rules: &Rules,
    rng: &mut GameRng,
    log: &mut Log,
) -> Outcome {
    use EffectKind as K;
    let victim = body.entity;
    let nothing_or = |amount: u32, effect: Effect| {
        if amount == 0 { Effect::Nothing { victim } } else { effect }
    };
    let effect = match spec.kind {
        K::Harm => {
            let lost = body.vitals.hurt(magnitude);
            dealt(body, lost, true, rng, log);
            return Outcome::Landed;
        }
        K::Sap => {
            let Some(character) = body.character.as_deref_mut() else { return Outcome::Missed };
            let lost = body.vitals.drain_health(character, magnitude);
            dealt(body, lost, false, rng, log);
            return Outcome::Landed;
        }
        K::Slay => {
            let lost = body.vitals.slay();
            dealt(body, lost, false, rng, log);
            return Outcome::Landed;
        }
        K::Afflict { condition, poison, indefinite } => {
            if condition == Condition::Turncoat && body.side.0 == Stance::Neutral {
                return Outcome::Missed;
            }
            let lasting = lasting(indefinite, magnitude);
            if condition == Condition::Poisoned {
                body.vitals.poison(lasting, poison);
            } else {
                body.vitals.inflict(condition, lasting);
            }
            Effect::Inflicted { victim, condition }
        }
        K::Tire => Effect::Tired { victim, amount: body.vitals.tire(magnitude) },
        K::DrainPool | K::DrainRealm(_) => {
            let (Some(character), Some((learned, pool))) =
                (body.character.as_deref(), body.spells.as_mut())
            else {
                return Outcome::Missed;
            };
            let maximum = maxima(character, body.attributes, learned, rules.content);
            let amount = match spec.kind {
                K::DrainRealm(realm) => realm
                    .map_or(enumset::EnumSet::all(), enumset::EnumSet::only)
                    .iter()
                    .map(|realm| pool.drain_realm(realm, magnitude, &maximum))
                    .sum(),
                _ => pool.drain(magnitude, &maximum, rng),
            };
            Effect::Drained { victim, amount }
        }
        K::DrainBudget => {
            let Some(budget) = body.budget.as_mut() else { return Outcome::Missed };
            let lost = budget.0.min(magnitude);
            budget.0 -= lost;
            Effect::Drained { victim, amount: lost }
        }
        K::Heal => {
            let amount = body.vitals.heal(magnitude);
            nothing_or(amount, Effect::Healed { victim, amount })
        }
        K::Refresh => {
            let amount = body.vitals.rest(magnitude);
            nothing_or(amount, Effect::Refreshed { victim, amount })
        }
        K::Replenish => {
            let Some((_, pool)) = body.spells.as_mut() else { return Outcome::Missed };
            let amount = pool.restore_evenly(magnitude);
            nothing_or(amount, Effect::Replenished { victim, amount })
        }
        K::Cure { conditions, outright } => {
            if conditions.contains(&Condition::Poisoned) {
                body.vitals.weaken_poison(defences::cure_rounds(strength), rng);
            }
            let lifted =
                defences::cure(rng, &mut body.vitals.conditions, conditions, strength, outright);
            match lifted.first() {
                Some(&condition) => Effect::Cured { victim, condition },
                None => Effect::Nothing { victim },
            }
        }
        K::Renew => {
            let Some(character) = body.character.as_deref_mut() else { return Outcome::Missed };
            if body.vitals.lift_drain(character) {
                Effect::Cured { victim, condition: Condition::Drained }
            } else {
                Effect::Nothing { victim }
            }
        }
        K::Raise => {
            if body.character.is_none() {
                return Outcome::Missed;
            }
            if !body.vitals.is_dead() {
                log.effects.push(Effect::Nothing { victim });
                return Outcome::Missed;
            }
            let share =
                (body.attributes[Attribute::Piety] + body.attributes[Attribute::Vitality]) >> 2;
            body.vitals.health.current = (body.vitals.health.max * share / 100).max(1);
            body.vitals.stamina.current = (body.vitals.stamina.max * share / 100).max(1);
            Effect::Raised { victim }
        }
        K::Purify { level } => {
            purify(&mut body.vitals.enchantments, magnitude, level, rules.content);
            Effect::Purified { victim }
        }
        K::Enchant(spell) => {
            let rolled = enchantment(spell, magnitude, strength, rules.content, rng);
            let vitals = &mut *body.vitals;
            let changed = if vitals.is_dead() {
                false
            } else if spell == SpellId::HASTE {
                enchantments::haste(&mut vitals.conditions, &mut vitals.enchantments, rolled)
            } else {
                vitals.enchantments.apply(rolled)
            };
            if changed { Effect::Enchanted { victim, spell } } else { Effect::Nothing { victim } }
        }
        K::Burn => {
            let Some(carried) = body.carried.as_deref_mut() else { return Outcome::Missed };
            let items = &rules.content.items;
            let mut burnt = [0; 3];
            for (_, slot) in &mut carried.equipment {
                if let Some(equipped) = slot.take() {
                    *slot = burn(equipped.stack, items, magnitude, rng, &mut burnt)
                        .map(|stack| Equipped { stack, ..equipped });
                }
            }
            for slot in &mut carried.backpack {
                if let Some(stack) = slot.take() {
                    *slot = burn(stack, items, magnitude, rng, &mut burnt);
                }
            }
            record_burnt(log, burnt);
            return Outcome::Landed;
        }
        // The party's own: its purse, and its lasting spells.
        K::LoseGold | K::Dispel => return Outcome::Missed,
        K::Prismic { .. } => unreachable!("Prismic is picked before it is applied"),
    };
    log.effects.push(effect);
    Outcome::Landed
}

/// Land a strike's party effects on the party itself, once (retail visits
/// the party's shared slots, pool and purse apart from its members).
pub fn land_on_party(
    strike: &Strike,
    party: Entity,
    (effects, inventory): (&mut PartyEffects, &mut PartyInventory),
    members: &mut [BodyItem],
    rules: &Rules,
    rng: &mut GameRng,
    log: &mut Log,
) {
    use EffectKind as K;
    let victim = party;
    let mut previous = None;
    for spec in specs(strike, rules, true) {
        if waits(&spec, previous) {
            previous = Some(Outcome::Missed);
            continue;
        }
        let magnitude = spec
            .roll
            .map_or(0, |roll| definitions::roll(roll, strike.strength, strike.blow.as_ref(), rng));
        if spec.chance.is_some_and(|chance| rng.below(100) >= chance) {
            previous = Some(Outcome::Missed);
            continue;
        }
        match spec.kind {
            K::DrainPool => {
                for body in members.iter_mut().filter(|body| !body.vitals.is_dead()) {
                    let (Some(character), Some((learned, pool))) =
                        (body.character.as_deref(), body.spells.as_mut())
                    else {
                        continue;
                    };
                    let maximum = maxima(character, body.attributes, learned, rules.content);
                    let amount = pool.drain(magnitude, &maximum, rng);
                    log.effects.push(Effect::Drained { victim: body.entity, amount });
                }
            }
            K::Purify { level } => {
                purify(&mut effects.0, magnitude, level, rules.content);
                log.effects.push(Effect::Purified { victim });
            }
            K::Enchant(spell) => {
                let rolled = enchantment(spell, magnitude, strike.strength, rules.content, rng);
                log.effects.push(if effects.0.apply(rolled) {
                    Effect::Enchanted { victim, spell }
                } else {
                    Effect::Nothing { victim }
                });
            }
            K::Burn => {
                let mut burnt = [0; 3];
                let pool = &mut inventory.pool.stacks;
                *pool = std::mem::take(pool)
                    .into_iter()
                    .filter_map(|stack| {
                        burn(stack, &rules.content.items, magnitude, rng, &mut burnt)
                    })
                    .collect();
                record_burnt(log, burnt);
            }
            K::LoseGold => {
                let amount = inventory.gold * magnitude.min(100) / 100;
                inventory.gold -= amount;
                log.effects.push(Effect::GoldLost { amount });
            }
            K::Dispel => effects.0.0.retain(|e| enchantments::combat_only(e.spell)),
            _ => unreachable!("not an effect on the party itself"),
        }
        previous = Some(Outcome::Landed);
    }
}

/// Shorten the offensive clouds among `slots` (retail Purify Air).
fn purify(slots: &mut Enchantments, amount: u32, level: u32, content: &Catalogs) {
    slots.0.retain_mut(|slot| {
        if enchantments::OFFENSIVE.contains(&slot.spell) {
            let cloud = &content.spells[slot.spell];
            let removed = (amount + level).max(cloud.level) - cloud.level;
            slot.turns -= removed.min(slot.turns);
        }
        slot.turns > 0
    });
}

/// The enchantment a spell lays for `turns`; Guardian Angel's absorption is
/// rolled as it is laid.
fn enchantment(
    spell: SpellId,
    turns: u32,
    strength: Strength,
    content: &Catalogs,
    rng: &mut GameRng,
) -> Enchantment {
    let power = strength.power;
    let absorption = if spell == SpellId::GUARDIAN_ANGEL {
        let rolled = content.spells[spell].dice.map_or(0, |dice| rng.roll(dice)) * power;
        rolled + rolled * strength.boost / 100
    } else {
        0
    };
    Enchantment { spell, power, boost: strength.boost, turns, absorption }
}

/// What is left of a stack once heat has taken each potion, scroll or book
/// in it at `chance` percent, counted by class.
fn burn(
    stack: ItemStack,
    items: &ItemCatalog,
    chance: u32,
    rng: &mut GameRng,
    burnt: &mut [u32; 3],
) -> Option<ItemStack> {
    let class = match items[stack.item].class {
        EquipClass::Potion => 0,
        EquipClass::Scroll => 1,
        EquipClass::Spellbook => 2,
        _ => return Some(stack),
    };
    let lost = (0..stack.count()).filter(|_| rng.below(100) < chance).count() as u16;
    burnt[class] += u32::from(lost);
    stack.remove_units(lost)
}

fn record_burnt(log: &mut Log, burnt: [u32; 3]) {
    let classes = [EquipClass::Potion, EquipClass::Scroll, EquipClass::Spellbook];
    for (class, amount) in classes.into_iter().zip(burnt) {
        if amount != 0 {
            log.effects.push(Effect::Destroyed { class, amount });
        }
    }
}
