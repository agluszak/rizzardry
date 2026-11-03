//! What an act does to a body, one effect at a time. An act's [`Reach`] on a
//! body works through its definition's effects in turn, as retail applies a
//! spell's or a blow's effects; each is an entity that [`Strikes`] the body
//! and is a [`HitOf`] the act, with exactly one effect component (`Harm`,
//! `Afflict`, `Heal`, …) and the modifiers that shape it:
//!
//! - [`Chance`]: it may not happen at all.
//! - [`Save`]: the body may save against it outright.
//! - [`Resist`]: the body's resistance cuts its [`Magnitude`].
//! - [`HalfStrength`]: it is saved against and resisted at half strength.
//!
//! The systems below settle each effect with an [`Outcome`] and record what
//! it did on its act's [`Deed`]; the reach then goes on to the next.
//! Effects on the party's shared spells, pool and purse strike the party
//! itself ([`crate::game::party::roster::Party`]).

use bevy::prelude::*;

use super::defences::{self, Resistances, SaveLevel, Senses};
use super::{Deed, Effect, HitOf, Strikes};
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
use crate::game::world::population::{Monster, SpellBudget};

/// An act reaching a body: which of its definition's effects comes next,
/// and how the last one came out, for the ones that depend on it.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Reach {
    pub next: usize,
    pub last: Option<Outcome>,
}

/// A missile still on its way: its reach lands when it arrives.
#[derive(Component, Debug)]
pub struct InFlight(pub Timer);

/// The effect is the one its reach is settling now.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = Current)]
pub struct CurrentOf(pub Entity);

/// The effect a reach is settling, maintained by Bevy from [`CurrentOf`].
#[derive(Component, Debug)]
#[relationship_target(relationship = CurrentOf, linked_spawn)]
pub struct Current(Vec<Entity>);

impl Current {
    pub fn effect(&self) -> Option<Entity> {
        self.0.first().copied()
    }
}

/// The percent chance the effect happens at all.
#[derive(Component, Clone, Copy, Debug)]
pub struct Chance(pub u32);

/// The body may save against it outright, from this base.
#[derive(Component, Clone, Copy, Debug)]
pub struct Save(pub i32);

/// The body's resistance in `realm` cuts it, against its act's strength;
/// saves against it are made in that realm too.
#[derive(Component, Clone, Copy, Debug)]
pub struct Resist {
    pub realm: Realm,
}

/// It is saved against and resisted at half its act's strength.
#[derive(Component, Clone, Debug)]
pub struct HalfStrength;

/// How much: damage, a drain, a condition's rounds, a cure's percent.
#[derive(Component, Clone, Copy, Debug)]
pub struct Magnitude(pub u32);

/// How an effect was settled.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Landed,
    Resisted,
    /// Nothing to act on: the body was dead, gone, or not of the kind.
    Missed,
}

/// Damage.
#[derive(Component, Clone, Debug)]
pub struct Harm;

/// Health drained for good, from a body with a sheet to lose it from.
#[derive(Component, Clone, Debug)]
pub struct Sap;

/// A condition; `indefinite` lasts until lifted, whatever its magnitude;
/// a poison bites by `poison`.
#[derive(Component, Clone, Copy, Debug)]
pub struct Afflict {
    pub condition: Condition,
    pub poison: u32,
    pub indefinite: bool,
}

/// Death, as death spells deal it.
#[derive(Component, Clone, Debug)]
pub struct Slay;

/// Stamina.
#[derive(Component, Clone, Debug)]
pub struct Tire;

/// A member's spell points, by the share retail takes of their pool.
#[derive(Component, Clone, Debug)]
pub struct DrainPool;

/// A monster's spell budget.
#[derive(Component, Clone, Debug)]
pub struct DrainBudget;

/// A member's spell points in one realm; in every realm when `None`.
#[derive(Component, Clone, Debug)]
pub struct DrainRealm(pub Option<Realm>);

/// Health, up to the maximum.
#[derive(Component, Clone, Debug)]
pub struct Heal;

/// Stamina, up to the maximum.
#[derive(Component, Clone, Debug)]
pub struct Refresh;

/// Spell points, spread over the realms.
#[derive(Component, Clone, Debug)]
pub struct Replenish;

/// Lift these conditions, by its act's strength, or outright.
#[derive(Component, Clone, Debug)]
pub struct Cure {
    pub conditions: &'static [Condition],
    pub outright: bool,
}

/// Lift Drained, and what draining took.
#[derive(Component, Clone, Debug)]
pub struct Renew;

/// Raise the dead.
#[derive(Component, Clone, Debug)]
pub struct Raise;

/// Shorten the clouds hanging over the body; `level` is the purifying
/// spell's.
#[derive(Component, Clone, Copy, Debug)]
pub struct Purify {
    pub level: u32,
}

/// A spell laid on the body for its magnitude's rounds at its act's
/// strength.
#[derive(Component, Clone, Copy, Debug)]
pub struct Enchant {
    pub spell: SpellId,
}

/// Burn potions, scrolls and books, each unit at the magnitude's percent.
#[derive(Component, Clone, Debug)]
pub struct Burn;

/// Lose the magnitude's percent of the party's gold.
#[derive(Component, Clone, Debug)]
pub struct LoseGold;

/// End the party's lasting spells.
#[derive(Component, Clone, Debug)]
pub struct Dispel;

/// What a damaging effect took off its body: health, for good or not.
#[derive(Component, Clone, Copy, Debug)]
pub struct Dealt(pub u32);

/// The body saved against the effect.
#[derive(Component, Clone, Copy, Debug)]
pub struct Saved;

/// Fly each missile; one that arrives lands.
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

/// The strength an effect is saved against and resisted at: its act's,
/// halved for a weaker follow-up.
fn against(strengths: &Query<&Strength>, act: Entity, half: bool) -> u32 {
    let against = strengths.get(act).map_or(0, |strength| strength.against);
    if half { against / 2 } else { against }
}

/// What harms, afflicts, drains or restores a body does nothing once it is
/// dead.
pub fn miss_the_dead(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes),
        (
            Without<Outcome>,
            Or<(
                With<Harm>,
                With<Sap>,
                With<Afflict>,
                With<Slay>,
                With<Tire>,
                With<DrainPool>,
                With<DrainBudget>,
                With<DrainRealm>,
                With<Heal>,
                With<Refresh>,
                With<Replenish>,
            )>,
        ),
    >,
    vitals: Query<&Vitals>,
) {
    for (effect, strikes) in &effects {
        if vitals.get(strikes.0).is_ok_and(Vitals::is_dead) {
            commands.entity(effect).insert(Outcome::Missed);
        }
    }
}

/// An effect that happens only by chance may not.
pub fn roll_chances(
    mut commands: Commands,
    effects: Query<(Entity, &Chance), Without<Outcome>>,
    mut rng: ResMut<GameRng>,
) {
    for (effect, chance) in &effects {
        if rng.below(100) >= chance.0 {
            commands.entity(effect).insert(Outcome::Missed);
        }
    }
}

/// A monster whose kind cannot suffer a condition, or cannot be slain,
/// shrugs it off before any save (retail `g_condition_immunities`).
pub fn immunities(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, Option<&Afflict>, Has<Slay>), Without<Outcome>>,
    monsters: Query<&Monster>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
) {
    for (effect, strikes, hit_of, afflict, slays) in &effects {
        let Ok(monster) = monsters.get(strikes.0) else { continue };
        let kind = &content.species[monster.species];
        let immune = match afflict {
            Some(afflict) => defences::immune(kind.kind, afflict.condition),
            None => slays && kind.death_immune,
        };
        if immune {
            record(&mut acts, hit_of.0, Effect::Resisted { victim: strikes.0 });
            commands.entity(effect).insert(Outcome::Resisted);
        }
    }
}

/// A body may save against an effect outright (retail
/// `TargetResistsCondition`): by its level and its resistance in the
/// effect's realm against the act's strength, scaled by difficulty for its
/// side, within 5..95.
pub fn saves(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &Save, &Resist, Has<HalfStrength>), Without<Outcome>>,
    bodies: Query<(&SaveLevel, &Resistances, &Side)>,
    mut acts: Query<&mut Deed>,
    strengths: Query<&Strength>,
    difficulty: Res<Difficulty>,
    mut rng: ResMut<GameRng>,
) {
    for (effect, strikes, hit_of, save, resist, half) in &effects {
        let Ok((level, resistances, side)) = bodies.get(strikes.0) else { continue };
        let against = against(&strengths, hit_of.0, half);
        let chance =
            defences::condition_chance(level.0, resistances.0[resist.realm], against, save.0);
        if defences::save_roll(&mut rng, difficulty.scale(chance, side.0)) {
            record(&mut acts, hit_of.0, Effect::Resisted { victim: strikes.0 });
            commands.entity(effect).insert((Outcome::Resisted, Saved));
        }
    }
}

/// A member who saves practises Iron Will.
pub fn iron_will(
    effects: Query<&Strikes, Added<Saved>>,
    mut characters: Query<&mut Character>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<crate::game::combat::CombatNews>,
) {
    for strikes in &effects {
        if let Ok(mut character) = characters.get_mut(strikes.0)
            && character.practise(Skill::IronWill, 2, &mut rng)
        {
            news.write(crate::game::combat::CombatNews::Improved {
                member: strikes.0,
                skill: Skill::IronWill,
            });
        }
    }
}

/// A body's resistance cuts an effect's magnitude; nothing is left of one
/// cut to nothing.
pub fn resist(
    mut commands: Commands,
    mut effects: Query<
        (Entity, &Strikes, &HitOf, &Resist, &mut Magnitude, Has<HalfStrength>),
        Without<Outcome>,
    >,
    bodies: Query<(&SaveLevel, &Resistances)>,
    mut acts: Query<&mut Deed>,
    strengths: Query<&Strength>,
    mut rng: ResMut<GameRng>,
) {
    for (effect, strikes, hit_of, resist, mut magnitude, half) in &mut effects {
        let Ok((level, resistances)) = bodies.get(strikes.0) else { continue };
        let against = against(&strengths, hit_of.0, half);
        let resistance = resistances.0[resist.realm];
        magnitude.0 = defences::resist(&mut rng, magnitude.0, resistance, level.0, against);
        if magnitude.0 == 0 {
            record(&mut acts, hit_of.0, Effect::Resisted { victim: strikes.0 });
            commands.entity(effect).insert(Outcome::Resisted);
        }
    }
}

/// What a member's constitution or profession shrugs off of a condition
/// they could save against, once the save is past.
pub fn shrug(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &Afflict), (With<Save>, Without<Outcome>)>,
    characters: Query<&Character>,
    mut acts: Query<&mut Deed>,
) {
    for (effect, strikes, hit_of, afflict) in &effects {
        let Ok(character) = characters.get(strikes.0) else { continue };
        let android = character.has_trait(Trait::AndroidConditionImmunity);
        let shrugs = match afflict.condition {
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
        if shrugs {
            record(&mut acts, hit_of.0, Effect::Resisted { victim: strikes.0 });
            commands.entity(effect).insert(Outcome::Resisted);
        }
    }
}

/// Damage hurts a living body ([`Vitals::hurt`]).
pub fn harm(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &Magnitude), (With<Harm>, Without<Outcome>)>,
    mut bodies: Query<&mut Vitals>,
    mut acts: Query<&mut Deed>,
) {
    for (effect, strikes, hit_of, magnitude) in &effects {
        let Ok(mut vitals) = bodies.get_mut(strikes.0) else { continue };
        let lost = vitals.hurt(magnitude.0);
        record(&mut acts, hit_of.0, Effect::Damaged { victim: strikes.0, amount: lost });
        commands.entity(effect).insert((Outcome::Landed, Dealt(lost)));
    }
}

/// A body with a sheet loses health for good; nothing wakes it.
pub fn sap(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &Magnitude), (With<Sap>, Without<Outcome>)>,
    mut members: Query<(&mut Character, &mut Vitals)>,
    mut acts: Query<&mut Deed>,
) {
    for (effect, strikes, hit_of, magnitude) in &effects {
        let Ok((mut character, mut vitals)) = members.get_mut(strikes.0) else {
            commands.entity(effect).insert(Outcome::Missed);
            continue;
        };
        let lost = vitals.drain_health(&mut character, magnitude.0);
        record(&mut acts, hit_of.0, Effect::Damaged { victim: strikes.0, amount: lost });
        commands.entity(effect).insert((Outcome::Landed, Dealt(lost)));
    }
}

/// How long a condition lasts, by its magnitude.
fn lasting(afflict: &Afflict, magnitude: u32) -> Lasting {
    match magnitude {
        _ if afflict.indefinite => Lasting::Indefinite,
        1 => Lasting::Rounds(2),
        rounds => Lasting::Rounds(rounds),
    }
}

/// A condition takes hold of a living body (retail
/// `InflictConditionOnTarget`): a poison bites by its strength; one
/// standing neutral cannot be turned.
pub fn afflict(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &Afflict, &Magnitude), Without<Outcome>>,
    mut bodies: Query<(&mut Vitals, &Side)>,
    mut acts: Query<&mut Deed>,
) {
    for (effect, strikes, hit_of, afflict, magnitude) in &effects {
        let Ok((mut vitals, side)) = bodies.get_mut(strikes.0) else { continue };
        let condition = afflict.condition;
        if condition == Condition::Turncoat && side.0 == Stance::Neutral {
            commands.entity(effect).insert(Outcome::Missed);
            continue;
        }
        let lasting = lasting(afflict, magnitude.0);
        if condition == Condition::Poisoned {
            vitals.poison(lasting, afflict.poison);
        } else {
            vitals.inflict(condition, lasting);
        }
        record(&mut acts, hit_of.0, Effect::Inflicted { victim: strikes.0, condition });
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// A body dies outright ([`Vitals::slay`]); a Valkyrie may still cheat it.
pub fn slay(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf), (With<Slay>, Without<Outcome>)>,
    mut bodies: Query<&mut Vitals>,
    mut acts: Query<&mut Deed>,
) {
    for (effect, strikes, hit_of) in &effects {
        let Ok(mut vitals) = bodies.get_mut(strikes.0) else { continue };
        let lost = vitals.slay();
        record(&mut acts, hit_of.0, Effect::Damaged { victim: strikes.0, amount: lost });
        commands.entity(effect).insert((Outcome::Landed, Dealt(lost)));
    }
}

pub fn tire(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &Magnitude), (With<Tire>, Without<Outcome>)>,
    mut bodies: Query<&mut Vitals>,
    mut acts: Query<&mut Deed>,
) {
    for (effect, strikes, hit_of, magnitude) in &effects {
        let Ok(mut vitals) = bodies.get_mut(strikes.0) else { continue };
        let amount = vitals.tire(magnitude.0);
        record(&mut acts, hit_of.0, Effect::Tired { victim: strikes.0, amount });
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// A member's spell points, by the share retail takes of their pool; a
/// body without a pool has none to lose.
pub fn drain_pool(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &Magnitude), (With<DrainPool>, Without<Outcome>)>,
    mut members: Query<(&Character, &Vitals, &LearnedSpells, &mut SpellPoints)>,
    bodies: Query<(), With<Vitals>>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    for (effect, strikes, hit_of, magnitude) in &effects {
        let Ok((character, vitals, learned, mut pool)) = members.get_mut(strikes.0) else {
            if bodies.contains(strikes.0) {
                commands.entity(effect).insert(Outcome::Missed);
            }
            continue;
        };
        let maximum = character.stats(&vitals.enchantments).max_points(learned, &content.spells);
        let amount = pool.drain(magnitude.0, &maximum, &mut rng);
        record(&mut acts, hit_of.0, Effect::Drained { victim: strikes.0, amount });
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// A spell budget, directly.
pub fn drain_budget(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &Magnitude), (With<DrainBudget>, Without<Outcome>)>,
    mut budgets: Query<&mut SpellBudget>,
    mut acts: Query<&mut Deed>,
) {
    for (effect, strikes, hit_of, magnitude) in &effects {
        let Ok(mut budget) = budgets.get_mut(strikes.0) else {
            commands.entity(effect).insert(Outcome::Missed);
            continue;
        };
        let lost = budget.0.min(magnitude.0);
        budget.0 -= lost;
        record(&mut acts, hit_of.0, Effect::Drained { victim: strikes.0, amount: lost });
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// A member's spell points in one realm, or in every realm.
pub fn drain_realm(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &DrainRealm, &Magnitude), Without<Outcome>>,
    mut members: Query<(&Character, &Vitals, &LearnedSpells, &mut SpellPoints)>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
) {
    for (effect, strikes, hit_of, drain, magnitude) in &effects {
        let Ok((character, vitals, learned, mut pool)) = members.get_mut(strikes.0) else {
            commands.entity(effect).insert(Outcome::Missed);
            continue;
        };
        let maximum = character.stats(&vitals.enchantments).max_points(learned, &content.spells);
        let realms = drain.0.map_or(enumset::EnumSet::all(), enumset::EnumSet::only);
        let amount: u32 =
            realms.iter().map(|realm| pool.drain_realm(realm, magnitude.0, &maximum)).sum();
        record(&mut acts, hit_of.0, Effect::Drained { victim: strikes.0, amount });
        commands.entity(effect).insert(Outcome::Landed);
    }
}

pub fn heal(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &Magnitude), (With<Heal>, Without<Outcome>)>,
    mut bodies: Query<&mut Vitals>,
    mut acts: Query<&mut Deed>,
) {
    for (effect, strikes, hit_of, magnitude) in &effects {
        let victim = strikes.0;
        let Ok(mut vitals) = bodies.get_mut(victim) else { continue };
        let amount = vitals.heal(magnitude.0);
        let landed = if amount == 0 {
            Effect::Nothing { victim }
        } else {
            Effect::Healed { victim, amount }
        };
        record(&mut acts, hit_of.0, landed);
        commands.entity(effect).insert(Outcome::Landed);
    }
}

pub fn refresh(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &Magnitude), (With<Refresh>, Without<Outcome>)>,
    mut bodies: Query<&mut Vitals>,
    mut acts: Query<&mut Deed>,
) {
    for (effect, strikes, hit_of, magnitude) in &effects {
        let victim = strikes.0;
        let Ok(mut vitals) = bodies.get_mut(victim) else { continue };
        let amount = vitals.rest(magnitude.0);
        let landed = if amount == 0 {
            Effect::Nothing { victim }
        } else {
            Effect::Refreshed { victim, amount }
        };
        record(&mut acts, hit_of.0, landed);
        commands.entity(effect).insert(Outcome::Landed);
    }
}

pub fn replenish(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &Magnitude), (With<Replenish>, Without<Outcome>)>,
    mut pools: Query<&mut SpellPoints>,
    mut acts: Query<&mut Deed>,
) {
    for (effect, strikes, hit_of, magnitude) in &effects {
        let victim = strikes.0;
        let Ok(mut pool) = pools.get_mut(victim) else {
            commands.entity(effect).insert(Outcome::Missed);
            continue;
        };
        let amount = pool.restore_evenly(magnitude.0);
        let landed = if amount == 0 {
            Effect::Nothing { victim }
        } else {
            Effect::Replenished { victim, amount }
        };
        record(&mut acts, hit_of.0, landed);
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// Conditions lifted by the act's strength; a cure of poison eases a
/// poison as it goes.
pub fn cure(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &Cure), Without<Outcome>>,
    mut bodies: Query<&mut Vitals>,
    mut acts: Query<&mut Deed>,
    strengths: Query<&Strength>,
    mut rng: ResMut<GameRng>,
) {
    for (effect, strikes, hit_of, cure) in &effects {
        let victim = strikes.0;
        let strength = strengths.get(hit_of.0).copied().unwrap_or_default();
        let Ok(mut vitals) = bodies.get_mut(victim) else { continue };
        if cure.conditions.contains(&Condition::Poisoned) {
            vitals.weaken_poison(defences::cure_rounds(strength), &mut rng);
        }
        let lifted = defences::cure(
            &mut rng,
            &mut vitals.conditions,
            cure.conditions,
            strength,
            cure.outright,
        );
        let landed = match lifted.first() {
            Some(&condition) => Effect::Cured { victim, condition },
            None => Effect::Nothing { victim },
        };
        record(&mut acts, hit_of.0, landed);
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// Renewal lifts a drain from a body with a sheet to give back what
/// draining took; only such bodies are ever drained.
pub fn renew(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf), (With<Renew>, Without<Outcome>)>,
    mut members: Query<(&mut Character, &mut Vitals)>,
    mut acts: Query<&mut Deed>,
) {
    for (effect, strikes, hit_of) in &effects {
        let victim = strikes.0;
        let Ok((mut character, mut vitals)) = members.get_mut(victim) else {
            commands.entity(effect).insert(Outcome::Missed);
            continue;
        };
        let lifted = vitals.lift_drain(&mut character);
        let landed = if lifted {
            Effect::Cured { victim, condition: Condition::Drained }
        } else {
            Effect::Nothing { victim }
        };
        record(&mut acts, hit_of.0, landed);
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// A dead member rises with a share of their health and stamina by their
/// piety and vitality.
pub fn raise(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf), (With<Raise>, Without<Outcome>)>,
    mut members: Query<(&Character, &mut Vitals)>,
    mut acts: Query<&mut Deed>,
) {
    for (effect, strikes, hit_of) in &effects {
        let victim = strikes.0;
        let Ok((character, mut vitals)) = members.get_mut(victim) else {
            commands.entity(effect).insert(Outcome::Missed);
            continue;
        };
        if !vitals.is_dead() {
            record(&mut acts, hit_of.0, Effect::Nothing { victim });
            commands.entity(effect).insert(Outcome::Missed);
            continue;
        }
        let stats = character.stats(&vitals.enchantments);
        let share = (stats.attribute(Attribute::Piety) + stats.attribute(Attribute::Vitality)) >> 2;
        vitals.health.current = (vitals.health.max * share / 100).max(1);
        vitals.stamina.current = (vitals.stamina.max * share / 100).max(1);
        record(&mut acts, hit_of.0, Effect::Raised { victim });
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// Shorten the offensive clouds among `slots` by `amount` over the
/// purifying spell's level.
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

/// A body's clouds shorten.
pub fn purify(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &Purify, &Magnitude), Without<Outcome>>,
    mut bodies: Query<&mut Vitals>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
) {
    for (effect, strikes, hit_of, purify, magnitude) in &effects {
        let victim = strikes.0;
        let Ok(mut vitals) = bodies.get_mut(victim) else { continue };
        purify_slots(&mut vitals.enchantments, magnitude.0, purify.level, &content.spells);
        record(&mut acts, hit_of.0, Effect::Purified { victim });
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// The party's clouds shorten.
pub fn purify_party(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &Purify, &Magnitude), Without<Outcome>>,
    mut parties: Query<&mut PartyEffects>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
) {
    for (effect, strikes, hit_of, purify, magnitude) in &effects {
        let Ok(mut party) = parties.get_mut(strikes.0) else { continue };
        purify_slots(&mut party.0, magnitude.0, purify.level, &content.spells);
        record(&mut acts, hit_of.0, Effect::Purified { victim: strikes.0 });
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// The enchantment an effect lays, at its act's strength: Guardian Angel
/// absorbs its rolled dice a power.
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

/// A spell laid on a living body; Haste counters Slow.
pub fn enchant(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &Enchant, &Magnitude), Without<Outcome>>,
    mut bodies: Query<&mut Vitals>,
    mut acts: Query<&mut Deed>,
    strengths: Query<&Strength>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    use crate::game::magic::enchantments::haste;
    for (effect, strikes, hit_of, enchant, turns) in &effects {
        let victim = strikes.0;
        let Ok(mut vitals) = bodies.get_mut(victim) else { continue };
        let strength = strengths.get(hit_of.0).copied().unwrap_or_default();
        let rolled = enchantment(enchant, turns.0, strength, &content.spells, &mut rng);
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
        record(&mut acts, hit_of.0, landed);
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// A spell laid on the whole party.
pub fn enchant_party(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &Enchant, &Magnitude), Without<Outcome>>,
    mut parties: Query<&mut PartyEffects>,
    mut acts: Query<&mut Deed>,
    strengths: Query<&Strength>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    for (effect, strikes, hit_of, enchant, turns) in &effects {
        let victim = strikes.0;
        let Ok(mut party) = parties.get_mut(victim) else { continue };
        let strength = strengths.get(hit_of.0).copied().unwrap_or_default();
        let rolled = enchantment(enchant, turns.0, strength, &content.spells, &mut rng);
        let changed = party.0.accepts(enchant.spell, strength.power) && party.0.apply(rolled);
        let spell = enchant.spell;
        let landed =
            if changed { Effect::Enchanted { victim, spell } } else { Effect::Nothing { victim } };
        record(&mut acts, hit_of.0, landed);
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// Anti-Magic drains every living member's pool, not only those it
/// reached.
pub fn drain_party(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &Magnitude), (With<DrainPool>, Without<Outcome>)>,
    parties: Query<&PartyRoster>,
    mut members: Query<(&Character, &Vitals, &LearnedSpells, &mut SpellPoints)>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    for (effect, strikes, hit_of, magnitude) in &effects {
        let Ok(roster) = parties.get(strikes.0) else { continue };
        for victim in roster.iter() {
            let Ok((character, vitals, learned, mut pool)) = members.get_mut(victim) else {
                continue;
            };
            if vitals.is_dead() {
                continue;
            }
            let maximum =
                character.stats(&vitals.enchantments).max_points(learned, &content.spells);
            let amount = pool.drain(magnitude.0, &maximum, &mut rng);
            record(&mut acts, hit_of.0, Effect::Drained { victim, amount });
        }
        commands.entity(effect).insert(Outcome::Landed);
    }
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

/// What burnt, recorded by class.
fn record_burnt(acts: &mut Query<&mut Deed>, act: Entity, totals: [u32; 3]) {
    let classes = [EquipClass::Potion, EquipClass::Scroll, EquipClass::Spellbook];
    for (class, amount) in classes.into_iter().zip(totals) {
        if amount != 0 {
            record(acts, act, Effect::Destroyed { class, amount });
        }
    }
}

/// Potions, scrolls and books burn in every slot a body carries, alternate
/// hands included.
pub fn burn(
    mut commands: Commands,
    effects: Query<(Entity, &HitOf, &Strikes, &Magnitude), (With<Burn>, Without<Outcome>)>,
    mut carrying: Query<&mut Carried>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    let items = &content.items;
    for (effect, hit_of, strikes, chance) in &effects {
        let Ok(mut carried) = carrying.get_mut(strikes.0) else { continue };
        let mut totals = [0; 3];
        for (_, slot) in &mut carried.equipment {
            if let Some(equipped) = slot.take() {
                *slot = burn_stack(equipped.stack, items, chance.0, &mut rng, &mut totals).map(
                    |stack| crate::game::inventory::Equipped {
                        stack,
                        curse_released: equipped.curse_released,
                    },
                );
            }
        }
        for slot in &mut carried.backpack {
            if let Some(stack) = slot.take() {
                *slot = burn_stack(stack, items, chance.0, &mut rng, &mut totals);
            }
        }
        record_burnt(&mut acts, hit_of.0, totals);
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// Potions, scrolls and books burn in the party's pool.
pub fn burn_pool(
    mut commands: Commands,
    effects: Query<(Entity, &HitOf, &Strikes, &Magnitude), (With<Burn>, Without<Outcome>)>,
    mut parties: Query<&mut PartyInventory>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    let items = &content.items;
    for (effect, hit_of, strikes, chance) in &effects {
        let Ok(mut inventory) = parties.get_mut(strikes.0) else { continue };
        let mut totals = [0; 3];
        let pool = &mut inventory.pool.stacks;
        *pool = std::mem::take(pool)
            .into_iter()
            .filter_map(|stack| burn_stack(stack, items, chance.0, &mut rng, &mut totals))
            .collect();
        record_burnt(&mut acts, hit_of.0, totals);
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// The party loses a share of its gold.
pub fn lose_gold(
    mut commands: Commands,
    effects: Query<(Entity, &HitOf, &Strikes, &Magnitude), (With<LoseGold>, Without<Outcome>)>,
    mut parties: Query<&mut PartyInventory>,
    mut acts: Query<&mut Deed>,
) {
    for (effect, hit_of, strikes, percent) in &effects {
        let Ok(mut inventory) = parties.get_mut(strikes.0) else { continue };
        let amount = inventory.gold * percent.0.min(100) / 100;
        inventory.gold -= amount;
        record(&mut acts, hit_of.0, Effect::GoldLost { amount });
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// The party's lasting spells end; its combat spells and its members' own
/// stay.
pub fn dispel(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes), (With<Dispel>, Without<Outcome>)>,
    mut parties: Query<&mut PartyEffects>,
) {
    for (effect, strikes) in &effects {
        let Ok(mut party) = parties.get_mut(strikes.0) else { continue };
        party.0.0.retain(|e| crate::game::magic::enchantments::combat_only(e.spell));
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// A member hurt hears of it.
pub fn notify_hurt(
    effects: Query<(&Strikes, &Dealt), Added<Dealt>>,
    members: Query<&Vitals, With<Character>>,
    mut notices: MessageWriter<DamageApplied>,
) {
    for (strikes, dealt) in &effects {
        if dealt.0 > 0
            && let Ok(vitals) = members.get(strikes.0)
        {
            notices.write(DamageApplied {
                member: strikes.0,
                amount: dealt.0,
                died: vitals.is_dead(),
            });
        }
    }
}

/// A lured monster hurt breaks free of the lure, and one outside a fight
/// stops where it is.
pub fn break_lures(
    mut commands: Commands,
    effects: Query<(&Strikes, &Dealt), (Added<Dealt>, With<Harm>)>,
    mut lured: Query<(
        &crate::game::magic::pointcasts::LureInfluence,
        Option<&mut bevy_landmass::AgentTarget3d>,
        Has<crate::game::combat::Combatant>,
    )>,
) {
    use crate::game::magic::pointcasts::{LureInfluence, LureResponse};
    for (strikes, dealt) in &effects {
        let Ok((&lure, goal, fighting)) = lured.get_mut(strikes.0) else { continue };
        if dealt.0 == 0 {
            continue;
        }
        commands
            .entity(strikes.0)
            .insert(LureInfluence { response: LureResponse::Resisted, ..lure });
        if !fighting && let Some(mut goal) = goal {
            *goal = bevy_landmass::AgentTarget3d::None;
        }
    }
}

/// What a draining act's damage took counts toward what it drains.
pub fn count_drains(
    effects: Query<(&HitOf, &Dealt), (Added<Dealt>, With<Harm>)>,
    mut drains: Query<&mut super::spells::Drains>,
) {
    for (hit_of, dealt) in &effects {
        if let Ok(mut drains) = drains.get_mut(hit_of.0) {
            drains.drained += dealt.0;
        }
    }
}

/// A sleeping body that damage left alive may wake, by its senses (retail
/// `ApplyDamageToCharacter`, `MonsterReactsToBeingStruck`).
pub fn wake(
    effects: Query<(&Strikes, &Dealt), (Added<Dealt>, With<Harm>)>,
    mut bodies: Query<(&mut Vitals, &Senses)>,
    mut rng: ResMut<GameRng>,
) {
    for (strikes, dealt) in &effects {
        let Ok((mut vitals, senses)) = bodies.get_mut(strikes.0) else { continue };
        if dealt.0 > 0 && !vitals.is_dead() {
            vitals.conditions.wake_on_hit(senses.0, &mut rng);
        }
    }
}
