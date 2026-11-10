//! What a strike does to the body it lands on: each effect of its
//! definition in turn, validated, saved against, resisted and applied, every
//! effect seeing what the one before it did. Plain functions over the state
//! their caller holds; nothing here is scheduled.

use bevy::ecs::query::QueryData;
use bevy::prelude::*;

use super::applied::{self, Outcome};
use super::defences;
use super::definitions::{self, Blow, EffectKind, EffectSpec, Rows};
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

/// One thing landing: the rows it follows at a strength, a weapon's blow
/// behind them when it is one.
#[derive(Clone, Copy, Debug)]
pub struct Strike {
    pub rows: Rows,
    pub strength: Strength,
    pub blow: Option<Blow>,
}

/// The rules a strike is settled by.
#[derive(Clone, Copy)]
pub struct Rules<'a> {
    pub content: &'a Catalogs,
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
    /// The health plain harm would take, before a Guardian Angel, which a
    /// leeching spell gives back.
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

/// Land a strike on a body (retail `ApplySpellEffectToTarget`,
/// `ApplyEffectConditions`): every effect for its kind in order, each asked of
/// the rules of [`super::defences`] and [`super::applied`] and made to happen
/// to the body.
pub fn land(
    strike: &Strike,
    body: &mut BodyItem,
    rules: &Rules,
    rng: &mut GameRng,
    log: &mut Log,
    laying: &mut Laying,
) {
    let mut previous = None;
    let spell = strike.rows.spell().map(|spell| &rules.content.spells[spell]);
    for spec in applied::rows(strike.rows.effects(rules.content), strike.blow, false) {
        let kind = body.resistances.kind;
        if !applied::is_for(&spec, body.member, kind) {
            continue;
        }
        if applied::waits(&spec, previous) {
            previous = Some(Outcome::Missed);
            continue;
        }
        let spec = spec.pick(rng);
        let rolled = spec.roll.map_or(0, |roll| {
            definitions::roll(roll, strike.strength, strike.blow.as_ref(), spell, rng)
        });
        let alive = !body.vitals.is_dead();
        let defended = defences::defend(
            rng,
            (&spec, spec.realm(spell)),
            rolled,
            strike.strength,
            rules.difficulty,
            (!alive, body.level.0, body.stance(), body.resistances),
        );
        let victim = body.entity;
        previous = Some(match defended {
            defences::Defended::Through(magnitude) => {
                let outcome = apply(&spec, magnitude, strike, body, rules, rng, log, laying);
                settle(body, alive, rules, rng);
                outcome
            }
            defences::Defended::Missed => Outcome::Missed,
            defences::Defended::Resisted { saved } => {
                if saved && body.character.is_some() {
                    log.practised.push((victim, Skill::IronWill, 2));
                }
                log.effects.push(Effect::Resisted { victim });
                Outcome::Resisted
            }
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

/// A member's spell point maxima as their sheet and attributes make them.
fn maxima(
    character: &Character,
    attributes: &Attributes,
    learned: &LearnedSpells,
    content: &Catalogs,
) -> enum_map::EnumMap<crate::content::characters::Realm, u32> {
    character.max_points(attributes, learned, &content.spells)
}

/// Damage dealt and its coupled consequences: it is told of, and plain harm
/// may wake a sleeper.
fn dealt(body: &mut BodyItem, amount: u32, harm: bool, rng: &mut GameRng, log: &mut Log) {
    let victim = body.entity;
    log.effects.push(Effect::Damaged { victim, amount });
    if amount > 0 {
        log.damaged.push(Damaged { body: victim, amount, died: body.vitals.is_dead() });
    }
    if harm && amount > 0 && !body.vitals.is_dead() {
        body.vitals.conditions.wake_on_hit(body.attributes[Attribute::Senses], rng);
    }
}

/// Apply one effect to a body, recording what it did.
fn apply(
    spec: &EffectSpec,
    magnitude: u32,
    strike: &Strike,
    body: &mut BodyItem,
    rules: &Rules,
    rng: &mut GameRng,
    log: &mut Log,
    laying: &mut Laying,
) -> Outcome {
    use EffectKind as K;
    let strength = strike.strength;
    let own = || strike.rows.spell().expect("only a spell's row is the spell's own");
    let victim = body.entity;
    let nothing_or = |amount: u32, effect: Effect| {
        if amount == 0 { Effect::Nothing { victim } } else { effect }
    };
    let effect = match spec.kind {
        K::Harm => {
            // A leech gives back the magnitude the victim's health could
            // bear, before a Guardian Angel takes any (retail
            // `DrainTargetsLife`, `Magic Effects.cpp:1626-1650`).
            log.harm += magnitude.min(body.vitals.health.current);
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
            let Some(lasting) = applied::afflicts(condition, body.stance(), indefinite, magnitude)
            else {
                return Outcome::Missed;
            };
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
            let cured =
                defences::cure(rng, &body.vitals.conditions, conditions, strength, outright);
            for &(condition, left) in &cured.shortened {
                if let Some(lasting) = body.vitals.conditions.lasting_mut(condition) {
                    *lasting = Lasting::Rounds(left);
                }
            }
            for &condition in &cured.lifted {
                body.vitals.conditions.lift(condition);
            }
            match cured.lifted.first() {
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
            let (health, stamina) = applied::raised(
                body.vitals.health.max,
                body.vitals.stamina.max,
                body.attributes[Attribute::Piety],
                body.attributes[Attribute::Vitality],
            );
            body.vitals.health.current = health;
            body.vitals.stamina.current = stamina;
            log.raised.push(victim);
            Effect::Raised { victim }
        }
        K::SpellPurify => {
            laying.purify(victim, magnitude + rules.content.spells[own()].level);
            Effect::Purified { victim }
        }
        K::SpellEnchant => {
            let spell = own();
            // Haste first spends itself on a slowing; wholly spent, it
            // changed something all the same.
            let answers_slowing =
                rules.content.spell_row(spell).is_some_and(|row| row.answers_slowing);
            let turns = if answers_slowing { body.vitals.hasted(magnitude) } else { magnitude };
            let dead = body.vitals.is_dead();
            let laid = !dead && lay(laying, victim, spell, turns, strength, rules, rng);
            // Wholly spent on a slowing, it changed something all the same.
            let spent = answers_slowing && turns == 0 && magnitude > 0;
            let changed = !dead && (laid || spent);
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
        K::SpellPrismic | K::AfflictAny(_) => {
            unreachable!("an either-or is picked before it is applied")
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
    let own = || strike.rows.spell().expect("only a spell's row is the spell's own");
    let spell = strike.rows.spell().map(|spell| &rules.content.spells[spell]);
    for spec in applied::rows(strike.rows.effects(rules.content), strike.blow, true) {
        if applied::waits(&spec, previous) {
            previous = Some(Outcome::Missed);
            continue;
        }
        let magnitude = spec.roll.map_or(0, |roll| {
            definitions::roll(roll, strike.strength, strike.blow.as_ref(), spell, rng)
        });
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
            K::SpellPurify => {
                laying.purify(party, magnitude + rules.content.spells[own()].level);
                log.effects.push(Effect::Purified { victim });
            }
            K::SpellEnchant => {
                let spell = own();
                let laid = lay(laying, party, spell, magnitude, strike.strength, rules, rng);
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
                // A percent of the purse, at most all of it.
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

/// Lay a spell on a bearer for `turns` at a cast's strength; the absorption of
/// a spell that absorbs damage is rolled as it is laid. Whether it took.
fn lay(
    laying: &mut Laying,
    bearer: Entity,
    spell: SpellId,
    turns: u32,
    strength: Strength,
    rules: &Rules,
    rng: &mut GameRng,
) -> bool {
    let absorbing = rules.content.spell_row(spell).is_some_and(|row| row.absorbs);
    let absorbs = applied::absorbs(
        rng,
        absorbing,
        rules.content.spells[spell].dice,
        strength.power,
        strength.boost,
    );
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
    let classes = [EquipClass::Potion, EquipClass::Scroll, EquipClass::Spellbook];
    let mut burnt = [0; 3];
    for item in items {
        let Some(class) = rules.held.flammable(item) else { continue };
        let lost = applied::burnt(rng, rules.held.units(item), chance);
        if lost > 0 {
            burnt[classes.iter().position(|&burns| burns == class).unwrap()] += u32::from(lost);
            log.burnt.push((item, lost));
        }
    }
    for (class, amount) in classes.into_iter().zip(burnt) {
        if amount != 0 {
            log.effects.push(Effect::Destroyed { class, amount });
        }
    }
}
