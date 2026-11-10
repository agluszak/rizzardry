//! What a strike does to the body it lands on: each effect of its
//! definition in turn, validated, saved against, resisted and applied, every
//! effect seeing what the one before it did. Plain functions over the state
//! their caller holds; nothing here is scheduled.

use bevy::ecs::query::QueryData;
use bevy::prelude::*;

use super::defences;
use super::definitions::{self, Blow, DefinitionId, Definitions, EffectKind, EffectSpec, Whom};
use super::{Damaged, Effect};
use crate::content::characters::{Attribute, Skill};
use crate::content::items::EquipClass;
use crate::content::spells::SpellId;
use crate::game::actors::spells::{Enchantment, Laying};
use crate::game::actors::{Armour, Attacks, Attributes, Level, Resistances};
use crate::game::characters::{Character, Condition, Lasting, Vitals};
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::inventory::PartyInventory;
use crate::game::items::{Burden, Items, Lose};
use crate::game::magic::{LearnedSpells, SpellPoints, Strength};
use crate::game::party::roster::PartyMember;
use crate::game::rng::GameRng;
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
    pub attributes: &'static Attributes,
    pub level: &'static Level,
    pub resistances: &'static Resistances,
    pub armour: &'static Armour,
    pub attacks: &'static Attacks,
    pub character: Option<&'static mut Character>,
    pub spells: Option<(&'static LearnedSpells, &'static mut SpellPoints)>,
    pub budget: Option<&'static mut SpellBudget>,
    pub burden: Option<&'static Burden>,
    pub member: Has<PartyMember>,
    pub untargetable: Has<Untargetable>,
}

impl BodyItem<'_, '_> {
    /// The side it is on now.
    pub fn stance(&self) -> Stance {
        self.side.now(&self.vitals)
    }
}

impl BodyReadOnlyItem<'_, '_> {
    /// The side it is on now.
    pub fn stance(&self) -> Stance {
        self.side.now(self.vitals)
    }

    /// The side it is on and the band it keeps to now.
    pub fn standing(&self) -> (Stance, Loyalty) {
        (self.stance(), self.loyalty.now(self.vitals))
    }
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
    /// What the party has, which heat burns.
    pub held: &'a Items<'a, 'a>,
}

/// What strikes did, for whoever tells of it.
#[derive(Debug, Default)]
pub struct Log {
    pub effects: Vec<Effect>,
    /// Skills a member used: who, which, and how often.
    pub practised: Vec<(Entity, Skill, u32)>,
    pub damaged: Vec<Damaged>,
    /// The dead brought back.
    pub raised: Vec<Entity>,
    /// The health plain harm took, which a leeching spell gives back.
    pub harm: u32,
    /// What heat took of the party's items: an item, and how many of it.
    pub burnt: Vec<(Entity, u16)>,
}

impl Log {
    /// The items heat took are gone.
    pub fn scorch(&mut self, commands: &mut Commands) {
        for (item, amount) in self.burnt.drain(..) {
            commands.trigger(Lose { item, amount });
        }
    }
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
    rules.definitions.get(strike.definition).effects.iter().filter_map(move |spec| {
        if (spec.on == Whom::Party) != party
            || (spec.by_monsters && !blow.is_some_and(|b| b.by_monster))
        {
            return None;
        }
        let Some(effect) = spec.weapon else { return Some(spec.clone()) };
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
            ref kind => kind.clone(),
        };
        (chance > 0).then_some(EffectSpec { chance: Some(chance), kind, ..spec.clone() })
    })
}

/// Whether an effect waits on the one before it, which did not come.
fn waits(spec: &EffectSpec, previous: Option<Outcome>) -> bool {
    (spec.landed && previous != Some(Outcome::Landed))
        || (spec.resisted && previous != Some(Outcome::Resisted))
}

/// Land a strike on a body (retail `ApplySpellEffectToTarget`,
/// `ApplyEffectConditions`): every effect for its kind in order.
pub fn land(
    strike: &Strike,
    body: &mut BodyItem,
    rules: &Rules,
    rng: &mut GameRng,
    log: &mut Log,
    laying: &mut Laying,
) {
    let mut previous = None;
    for spec in specs(strike, rules, false) {
        let kind = body.resistances.kind;
        if (spec.on == Whom::Members && !body.member)
            || (spec.on == Whom::Monsters && kind.is_none())
            || spec.kinds.as_ref().is_some_and(|kinds| !kind.is_some_and(|k| kinds.contains(&k)))
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
                let outcome =
                    apply(&spec, magnitude, strike.strength, body, rules, rng, log, laying);
                settle(body, alive, rules, rng);
                outcome
            }
            Err(outcome) => outcome,
        });
    }
}

/// A Valkyrie whose health the effect ran out cheats death before anything
/// else lands on her.
fn settle(body: &mut BodyItem, was_alive: bool, rules: &Rules, rng: &mut GameRng) {
    if let Some(character) = body.character.as_deref()
        && was_alive
        && body.vitals.is_dead()
    {
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
        resisted = defences::save_roll(rng, rules.difficulty.scale(chance, body.stance()));
        if resisted && body.character.is_some() {
            log.practised.push((victim, Skill::IronWill, 2));
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
    laying: &mut Laying,
) -> Outcome {
    use EffectKind as K;
    let victim = body.entity;
    let nothing_or = |amount: u32, effect: Effect| {
        if amount == 0 { Effect::Nothing { victim } } else { effect }
    };
    let effect = match spec.kind {
        K::Harm => {
            let through = laying.absorb(victim, magnitude);
            let lost = body.vitals.hurt(through);
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
            if condition == Condition::Turncoat && body.stance() == Stance::Neutral {
                return Outcome::Missed;
            }
            let lasting = lasting(indefinite, magnitude);
            if condition == Condition::Poisoned {
                body.vitals.poison(lasting, poison);
            } else if condition != Condition::Slowed {
                body.vitals.afflict(condition, lasting);
            } else if let Some(left) = laying.slow(victim, lasting) {
                body.vitals.afflict(condition, left);
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
        K::Cure { ref conditions, outright } => {
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
            log.raised.push(victim);
            Effect::Raised { victim }
        }
        K::Purify { level } => {
            laying.purify(victim, magnitude + level);
            Effect::Purified { victim }
        }
        K::Enchant(spell) => {
            // Haste first spends itself on a slowing; wholly spent, it
            // changed something all the same.
            let turns = match spell {
                SpellId::HASTE => body.vitals.hasted(magnitude),
                _ => magnitude,
            };
            let changed = !body.vitals.is_dead()
                && (lay(laying, victim, spell, turns, strength, rules.content, rng)
                    || (spell == SpellId::HASTE && turns == 0 && magnitude > 0));
            if changed { Effect::Enchanted { victim, spell } } else { Effect::Nothing { victim } }
        }
        K::Burn => {
            if body.character.is_none() {
                return Outcome::Missed;
            }
            burn(rules.held.carried(victim), rules, magnitude, rng, log);
            return Outcome::Landed;
        }
        // The party's own: its purse, and its lasting spells.
        K::LoseGold | K::Dispel => return Outcome::Missed,
        K::Prismic { .. } => unreachable!("Prismic is picked before it is applied"),
        K::SpellPurify | K::SpellEnchant | K::SpellPrismic => {
            unreachable!("a spell's own effect is made plain with its effects")
        }
    };
    log.effects.push(effect);
    Outcome::Landed
}

/// Land a strike's party effects on the party itself, once (retail visits
/// the party's shared slots, pool and purse apart from its members).
pub fn land_on_party(
    strike: &Strike,
    party: Entity,
    inventory: &mut PartyInventory,
    members: &mut [BodyItem],
    rules: &Rules,
    rng: &mut GameRng,
    log: &mut Log,
    laying: &mut Laying,
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
                laying.purify(party, magnitude + level);
                log.effects.push(Effect::Purified { victim });
            }
            K::Enchant(spell) => {
                let laid =
                    lay(laying, party, spell, magnitude, strike.strength, rules.content, rng);
                log.effects.push(if laid {
                    Effect::Enchanted { victim, spell }
                } else {
                    Effect::Nothing { victim }
                });
            }
            K::Burn => {
                burn(rules.held.pool(party).iter().copied(), rules, magnitude, rng, log);
            }
            K::LoseGold => {
                let amount = inventory.gold * magnitude.min(100) / 100;
                inventory.gold -= amount;
                log.effects.push(Effect::GoldLost { amount });
            }
            K::Dispel => laying.dispel(party),
            _ => unreachable!("not an effect on the party itself"),
        }
        previous = Some(Outcome::Landed);
    }
}

/// Lay a spell on a bearer for `turns` at a cast's strength; Guardian
/// Angel's absorption is rolled as it is laid. Whether it took.
fn lay(
    laying: &mut Laying,
    bearer: Entity,
    spell: SpellId,
    turns: u32,
    strength: Strength,
    content: &Catalogs,
    rng: &mut GameRng,
) -> bool {
    let absorbs = if spell == SpellId::GUARDIAN_ANGEL {
        let rolled = content.spells[spell].dice.map_or(0, |dice| rng.roll(dice)) * strength.power;
        rolled + rolled * strength.boost / 100
    } else {
        0
    };
    let laid = Enchantment { spell, power: strength.power, boost: strength.boost };
    laying.lay(bearer, laid, turns, absorbs)
}

/// Heat takes each potion, scroll or book among `items` at `chance` percent,
/// each of a stack on its own, and tells what it took by class.
fn burn(
    items: impl IntoIterator<Item = Entity>,
    rules: &Rules,
    chance: u32,
    rng: &mut GameRng,
    log: &mut Log,
) {
    let mut burnt = [0; 3];
    for item in items {
        let Some(id) = rules.held.id(item) else { continue };
        let class = match rules.content.items[id].class {
            EquipClass::Potion => 0,
            EquipClass::Scroll => 1,
            EquipClass::Spellbook => 2,
            _ => continue,
        };
        let lost = (0..rules.held.units(item)).filter(|_| rng.below(100) < chance).count() as u16;
        if lost > 0 {
            burnt[class] += u32::from(lost);
            log.burnt.push((item, lost));
        }
    }
    record_burnt(log, burnt);
}

fn record_burnt(log: &mut Log, burnt: [u32; 3]) {
    let classes = [EquipClass::Potion, EquipClass::Scroll, EquipClass::Spellbook];
    for (class, amount) in classes.into_iter().zip(burnt) {
        if amount != 0 {
            log.effects.push(Effect::Destroyed { class, amount });
        }
    }
}
