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
use crate::game::characters::{Condition, Vitals};
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
    pub conditions: &'static mut crate::game::characters::Conditions,
    pub lucid: Has<crate::game::actors::Lucid>,
    pub can_guard: Has<crate::game::actors::CanGuard>,
    pub can_act: Has<crate::game::actors::CanAct>,
    pub can_fight: Has<crate::game::actors::CanFight>,
    pub side: &'static Side,
    pub loyalty: &'static Loyalty,
    pub attributes: &'static Attributes,
    pub skills: &'static crate::game::actors::Skills,
    pub level: &'static Level,
    pub resistances: &'static Resistances,
    pub armour: &'static Armour,
    pub attacks: &'static Attacks,
    /// What it is made of, where landing an effect changes or asks it.
    pub traits: &'static crate::game::actors::Traits,
    pub drained: &'static mut crate::game::actors::Drained,
    pub calling: Option<&'static crate::game::characters::Calling>,
    pub schooling: Option<crate::game::characters::Schooling>,
    pub cheated: Option<&'static crate::game::characters::CheatedDeathIn>,
    pub spells: Option<(&'static LearnedSpells, &'static mut SpellPoints)>,
    pub budget: Option<&'static mut SpellBudget>,
    pub burden: Option<&'static Burden>,
    pub member: Has<PartyMember>,
    pub untargetable: Has<Untargetable>,
}

impl BodyItem<'_, '_> {
    /// The side it is on now.
    pub fn stance(&self) -> Stance {
        self.side.now(&self.conditions)
    }

    /// As the formulas of training read it, if it trained.
    pub fn trained(&self) -> Option<crate::game::characters::Trained<'_>> {
        self.schooling.as_ref().map(|schooling| schooling.view())
    }

    /// The level a trait of its grows with.
    fn mastery(&self) -> u8 {
        self.calling.map_or(self.level.0 as u8, crate::game::characters::Calling::mastery)
    }
}

impl BodyReadOnlyItem<'_, '_> {
    /// The side it is on now.
    pub fn stance(&self) -> Stance {
        self.side.now(self.conditions)
    }

    /// The side it is on and the band it keeps to now.
    pub fn standing(&self) -> (Stance, Loyalty) {
        (self.stance(), self.loyalty.now(self.conditions))
    }

    /// As the formulas of training read it, if it trained.
    pub fn trained(&self) -> Option<crate::game::characters::Trained<'_>> {
        self.schooling.as_ref().map(|schooling| schooling.view())
    }
}

/// One thing landing: the rows it follows at a strength, a weapon's blow
/// behind them when it is one.
#[derive(Clone, Copy, Debug)]
pub struct Strike {
    pub rows: Rows,
    pub strength: Strength,
    pub blow: Option<Blow>,
    /// Who does it, and answers for a death it brings
    /// ([`Vitals::hurt`](crate::game::characters::Vitals::hurt)); nobody for
    /// a cloud's pulse, a trap or a device.
    pub by: Option<Entity>,
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
    /// The health plain harm would take, before a Guardian Angel, which a
    /// leeching spell gives back.
    pub harm: u32,
    /// What heat took of the party's items: an item, and how many of it.
    pub burnt: Vec<(Entity, u16)>,
    /// Who was swallowed, and by whom.
    pub swallowed: Vec<(Entity, Entity)>,
    /// Who cheated death for the first time in a fight, and the fight.
    pub cheated: Vec<(Entity, Entity)>,
}

impl Log {
    /// The items heat took are gone.
    pub fn scorch(&mut self, commands: &mut Commands) {
        for (item, amount) in self.burnt.drain(..) {
            commands.trigger(Lose { item, amount });
        }
    }

    /// Those swallowed are held by whoever swallowed them, and one who
    /// cheated death in a fight has done so in it.
    pub fn swallow(&mut self, commands: &mut Commands) {
        for (victim, by) in self.swallowed.drain(..) {
            commands.entity(victim).insert(crate::game::actors::Swallowed(by));
        }
        for (victim, fight) in self.cheated.drain(..) {
            commands.entity(victim).insert(crate::game::characters::CheatedDeathIn(fight));
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
                settle(body, alive, rules, rng, log);
                outcome
            }
            defences::Defended::Missed => Outcome::Missed,
            defences::Defended::Resisted { saved } => {
                if saved && body.calling.is_some() {
                    log.practised.push((victim, Skill::IronWill, 2));
                }
                log.effects.push(Effect::Resisted { victim });
                Outcome::Resisted
            }
        });
    }
}

/// One whose health the effect ran out, and who cheats death, does so
/// before anything else lands on them.
fn settle(body: &mut BodyItem, was_alive: bool, rules: &Rules, rng: &mut GameRng, log: &mut Log) {
    if !was_alive || !body.vitals.is_dead() {
        return;
    }
    let victim = body.entity;
    let first = rules.fight.is_some_and(|fight| {
        body.cheated != Some(&crate::game::characters::CheatedDeathIn(fight))
            && !log.cheated.contains(&(victim, fight))
    });
    let by = (body.traits, body.mastery());
    if body.vitals.cheats_death(&mut body.conditions, by, first, rng)
        && let (true, Some(fight)) = (first, rules.fight)
    {
        log.cheated.push((victim, fight));
    }
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
        let senses = body.attributes[Attribute::Senses];
        body.conditions.wake_on_hit(senses, rng);
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
            let lost = body.vitals.hurt(through, strike.by);
            dealt(body, lost, true, rng, log);
            return Outcome::Landed;
        }
        K::Sap => {
            let lost = body.vitals.drain_health(
                &mut body.conditions,
                &mut body.drained,
                magnitude,
                strike.by,
            );
            dealt(body, lost, false, rng, log);
            return Outcome::Landed;
        }
        K::Slay => {
            let lost = body.vitals.slay(strike.by);
            dealt(body, lost, false, rng, log);
            return Outcome::Landed;
        }
        K::Afflict { condition, poison, indefinite } => {
            let Some(lasting) = applied::afflicts(condition, body.stance(), indefinite, magnitude)
            else {
                return Outcome::Missed;
            };
            if condition == Condition::Poisoned {
                body.conditions.poison(lasting, poison);
            } else if condition != Condition::Slowed {
                body.conditions.afflict(condition, lasting);
            } else if let Some(left) = laying.slow(victim, lasting) {
                body.conditions.afflict(condition, left);
            }
            Effect::Inflicted { victim, condition }
        }
        K::Swallow => {
            let Some(by) = strike.by else { return Outcome::Missed };
            body.conditions
                .afflict(Condition::Missing, crate::game::characters::Lasting::Indefinite);
            log.swallowed.push((victim, by));
            Effect::Inflicted { victim, condition: Condition::Missing }
        }
        K::Tire => Effect::Tired { victim, amount: body.vitals.tire(magnitude) },
        K::DrainPool | K::DrainRealm(_) => {
            let Some((learned, pool)) = body.spells.as_mut() else { return Outcome::Missed };
            let maximum = crate::game::magic::max_points(
                body.level.0,
                body.attributes,
                learned,
                &rules.content.spells,
                body.skills,
            );
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
                body.conditions.weaken_poison(defences::cure_rounds(strength), rng);
            }
            let cured = defences::cure(rng, &body.conditions, conditions, strength, outright);
            for &(condition, left) in &cured.shortened {
                body.conditions.shorten(condition, left);
            }
            for &condition in &cured.lifted {
                body.conditions.lift(condition);
            }
            match cured.lifted.first() {
                Some(&condition) => Effect::Cured { victim, condition },
                None => Effect::Nothing { victim },
            }
        }
        K::Renew => {
            if body.drained.lift(&mut body.conditions) {
                Effect::Cured { victim, condition: Condition::Drained }
            } else {
                Effect::Nothing { victim }
            }
        }
        K::Raise => {
            if body.calling.is_none() {
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
            body.vitals.raise(health, stamina);
            Effect::Raised { victim }
        }
        K::SpellPurify => {
            laying.purify(victim, magnitude + rules.content.spells[own()].level);
            Effect::Purified { victim }
        }
        K::SpellEnchant | K::Enchant(_) => {
            let spell = if let K::Enchant(spell) = &spec.kind { *spell } else { own() };
            // Haste first spends itself on a slowing; wholly spent, it
            // changed something all the same.
            let answers_slowing =
                rules.content.spell_row(spell).is_some_and(|row| row.answers_slowing);
            let turns = if answers_slowing { body.conditions.hasted(magnitude) } else { magnitude };
            let dead = body.vitals.is_dead();
            let laid = !dead && lay(laying, victim, spell, turns, strength, rules, rng);
            // Wholly spent on a slowing, it changed something all the same.
            let spent = answers_slowing && turns == 0 && magnitude > 0;
            let changed = !dead && (laid || spent);
            if changed { Effect::Enchanted { victim, spell } } else { Effect::Nothing { victim } }
        }
        K::Burn => {
            if body.calling.is_none() {
                return Outcome::Missed;
            }
            burn(rules.held.carried(victim), rules, magnitude, rng, log);
            return Outcome::Landed;
        }
        // The party's own: its purse, and its lasting spells.
        K::LoseGold | K::Dispel => return Outcome::Missed,
        K::SpellPrismic | K::AfflictAny(_) | K::OneOf(_) => {
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
                    let Some((learned, pool)) = body.spells.as_mut() else { continue };
                    let maximum = crate::game::magic::max_points(
                        body.level.0,
                        body.attributes,
                        learned,
                        &rules.content.spells,
                        body.skills,
                    );
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
