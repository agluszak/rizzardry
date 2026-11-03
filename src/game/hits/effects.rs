//! What an act does to a body, one effect at a time. Each effect is an
//! entity: a [`HitOf`] its act that [`Strikes`] one body, with exactly one
//! effect component (`Harm`, `Afflict`, `Heal`, …) and the modifiers that
//! shape it:
//!
//! - [`InFlight`]: a missile on its way; nothing happens to it yet.
//! - [`Follows`]: it waits until the effect before it on the same body is
//!   settled, as retail applies a spell's or a blow's effects in turn.
//! - [`Chance`]: it may not happen at all.
//! - [`Save`]: the body may save against it outright.
//! - [`Resist`]: the body's resistance cuts its [`Magnitude`].
//! - [`OnlyIfLanded`], [`OnlyIfResisted`]: it happens only if the effect
//!   before it landed, or was resisted.
//! - [`ForParty`]: it also acts for the whole party, once.
//!
//! The systems below settle each effect with an [`Outcome`] and record what
//! it did on its act's [`Deed`]; [`release_followers`] then lets the next
//! effects go and the settled one goes away.

use bevy::prelude::*;

use super::body;
use super::defences::{Resistances, SaveLevel};
use super::{Deed, Effect, HitOf, Strikes};
use crate::content::characters::{Attribute, Realm, Skill, Trait};
use crate::content::items::{EquipClass, ItemCatalog};
use crate::content::spells::SpellId;
use crate::game::characters::{Character, Condition, Fate, Lasting, Vitals};
use crate::game::magic::{LearnedSpells, SpellPoints};
use crate::game::world::hostility::{Side, Stance};
use crate::game::world::population::Monster;
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::inventory::PartyInventory;
use crate::game::magic::Strength;
use crate::game::magic::enchantments::{Enchantment, Enchantments, PartyEffects};
use crate::game::party::DamageApplied;
use crate::game::rng::GameRng;
use crate::game::world::damage::DamageOutcome;

/// A missile still on its way.
#[derive(Component, Debug)]
pub struct InFlight(pub Timer);

/// The effect waits until the one before it is settled.
#[derive(Component, Debug)]
#[relationship(relationship_target = FollowedBy)]
pub struct Follows(pub Entity);

/// The effects waiting on this one.
#[derive(Component, Debug)]
#[relationship_target(relationship = Follows)]
pub struct FollowedBy(Vec<Entity>);

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

/// It happens only if the effect before it landed.
#[derive(Component, Clone, Debug)]
pub struct OnlyIfLanded;

/// It happens only if the effect before it was resisted.
#[derive(Component, Clone, Debug)]
pub struct OnlyIfResisted;

/// It acts on the whole party (its shared spells, pool and purse), through
/// the first member it reaches.
#[derive(Component, Clone, Debug)]
pub struct ForParty;

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

/// Health drained from a member for good.
#[derive(Component, Clone, Debug)]
pub struct Sap;

/// A condition; `indefinite` lasts until lifted, whatever its magnitude.
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

/// Spell points: a member's pool by the share retail takes, a monster's
/// budget.
#[derive(Component, Clone, Debug)]
pub struct DrainMagic;

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

/// Shorten the clouds hanging over a monster, or over the party; `level` is
/// the purifying spell's.
#[derive(Component, Clone, Copy, Debug)]
pub struct Purify {
    pub level: u32,
}

/// A spell laid on the body, or on the party, for its magnitude's rounds at
/// its act's strength.
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

/// Who a chain of effects lands on, as part of which act, and how long a
/// missile carrying them flies: each effect spawned follows the one before
/// it, and all arrive together.
pub struct Chain {
    act: Entity,
    victim: Entity,
    flight: Option<f32>,
    last: Option<Entity>,
}

impl Chain {
    pub fn new(act: Entity, victim: Entity) -> Self {
        Self { act, victim, flight: None, last: None }
    }

    /// The effects arrive by missile after `seconds`.
    pub fn flying(self, seconds: Option<f32>) -> Self {
        Self { flight: seconds, ..self }
    }

    pub fn victim(&self) -> Entity {
        self.victim
    }

    /// Add a copy of a definition's effect after the ones before it.
    pub fn clone_of(&mut self, commands: &mut Commands, prototype: Entity) -> Entity {
        let effect = commands
            .entity(prototype)
            .clone_and_spawn_with_opt_out(|builder| {
                builder.deny::<super::prototypes::EffectOf>();
            })
            .id();
        let mut entity = commands.entity(effect);
        entity.insert((HitOf(self.act), Strikes(self.victim)));
        if let Some(last) = self.last {
            entity.insert(Follows(last));
        }
        if let Some(seconds) = self.flight {
            entity.insert(InFlight(Timer::from_seconds(seconds, TimerMode::Once)));
        }
        let id = entity.id();
        self.last = Some(id);
        id
    }
}

/// An effect that may be settled now: not in flight, not waiting on the
/// one before it.
#[derive(Component, Debug)]
pub struct Ready;

/// Effects become ready once they have arrived and the one before them is
/// settled.
pub fn ready(
    mut commands: Commands,
    effects: Query<
        Entity,
        (
            With<HitOf>,
            Without<super::prototypes::Reach>,
            Without<InFlight>,
            Without<Follows>,
            Without<Outcome>,
            Without<Ready>,
        ),
    >,
) {
    for effect in &effects {
        commands.entity(effect).insert(Ready);
    }
}

/// What harms, afflicts, drains or restores a body does nothing once it is
/// dead, or gone.
pub fn miss_the_dead(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes),
        (
            With<Ready>,
            Without<Outcome>,
            Or<(
                With<Harm>,
                With<Sap>,
                With<Afflict>,
                With<Slay>,
                With<Tire>,
                With<DrainMagic>,
                With<DrainRealm>,
                With<Heal>,
                With<Refresh>,
                With<Replenish>,
            )>,
        ),
    >,
    vitals: Query<&crate::game::characters::Vitals>,
) {
    for (effect, strikes) in &effects {
        if !vitals.get(strikes.0).is_ok_and(|vitals| !vitals.is_dead()) {
            commands.entity(effect).insert(Outcome::Missed);
        }
    }
}

/// Fly each missile; one that arrives lands.
pub fn fly(mut commands: Commands, mut flying: Query<(Entity, &mut InFlight)>, time: Res<Time>) {
    for (effect, mut flight) in &mut flying {
        if flight.0.tick(time.delta()).is_finished() {
            commands.entity(effect).remove::<InFlight>();
        }
    }
}

fn record(acts: &mut Query<&mut Deed>, act: Entity, effect: Effect) {
    if let Ok(mut deed) = acts.get_mut(act) {
        deed.effects.push(effect);
    }
}

/// An effect that happens only by chance may not.
pub fn roll_chances(
    mut commands: Commands,
    effects: Query<(Entity, &Chance), (With<Ready>, Without<Outcome>)>,
    mut rng: ResMut<GameRng>,
) {
    for (effect, chance) in &effects {
        if rng.below(100) >= chance.0 {
            commands.entity(effect).insert(Outcome::Missed);
        }
    }
}

/// What a damaging effect took off its body: health, for good or not.
#[derive(Component, Clone, Copy, Debug)]
pub struct Dealt(pub u32);

/// The body saved against the effect.
#[derive(Component, Clone, Copy, Debug)]
pub struct Saved;

/// The fight under way, whose deaths a member may cheat once.
fn fight(battles: &Query<Entity, With<crate::game::combat::Battle>>) -> Option<Entity> {
    battles.single().ok()
}

/// A monster whose kind cannot suffer a condition, or cannot be slain,
/// shrugs it off before any save (retail `g_condition_immunities`).
pub fn immunities(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes, &HitOf, Option<&Afflict>, Has<Slay>),
        (With<Ready>, Without<Outcome>),
    >,
    monsters: Query<&Monster>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
) {
    for (effect, strikes, hit_of, afflict, slays) in &effects {
        let Ok(monster) = monsters.get(strikes.0) else { continue };
        let kind = &content.species[monster.species];
        let immune = match afflict {
            Some(afflict) => body::immune(kind.kind, afflict.condition),
            None => slays && kind.death_immune,
        };
        if immune {
            record(&mut acts, hit_of.0, Effect::Resisted { victim: strikes.0 });
            commands.entity(effect).insert(Outcome::Resisted);
        }
    }
}

/// The strength an effect is saved against and resisted at: its act's,
/// halved for a weaker follow-up.
fn against(strengths: &Query<&Strength>, act: Entity, half: bool) -> u32 {
    let against = strengths.get(act).map_or(0, |strength| strength.against);
    if half { against / 2 } else { against }
}

/// A living body may save against an effect outright (retail
/// `TargetResistsCondition`): by its level and its resistance in the
/// effect's realm against the act's strength, scaled by difficulty for its
/// side, within 5..95.
pub fn saves(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes, &HitOf, &Save, &Resist, Has<HalfStrength>),
        (With<Ready>, Without<Outcome>),
    >,
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
            body::condition_chance(level.0, resistances.0[resist.realm], against, save.0);
        if body::save_roll(&mut rng, difficulty.scale(chance, side.0)) {
            record(&mut acts, hit_of.0, Effect::Resisted { victim: strikes.0 });
            commands.entity(effect).insert((Outcome::Resisted, Saved));
        }
    }
}

/// A member who saves practises Iron Will.
pub fn iron_will(
    effects: Query<(&Strikes, &HitOf), Added<Saved>>,
    mut characters: Query<&mut Character>,
    mut acts: Query<&mut Deed>,
    mut rng: ResMut<GameRng>,
) {
    for (strikes, hit_of) in &effects {
        if let Ok(mut character) = characters.get_mut(strikes.0)
            && character.practise(Skill::IronWill, 2, &mut rng)
            && let Ok(mut deed) = acts.get_mut(hit_of.0)
        {
            deed.improved.push((strikes.0, Skill::IronWill));
        }
    }
}

/// A living body's resistance cuts an effect's magnitude; nothing is left
/// of one cut to nothing.
pub fn resist(
    mut commands: Commands,
    mut effects: Query<
        (Entity, &Strikes, &HitOf, &Resist, &mut Magnitude, Has<HalfStrength>),
        (With<Ready>, Without<Outcome>),
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
        magnitude.0 = body::resist(&mut rng, magnitude.0, resistance, level.0, against);
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
    effects: Query<(Entity, &Strikes, &HitOf, &Afflict), (With<Save>, With<Ready>, Without<Outcome>)>,
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

/// Damage hurts a living member as retail `ApplyDamageToCharacter` does
/// (fatigue, Guardian Angel, cheating death), and may wake them.
pub fn harm_members(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes, &HitOf, &Magnitude),
        (With<Harm>, With<Ready>, Without<Outcome>),
    >,
    mut members: Query<(&Character, &mut Vitals)>,
    mut acts: Query<&mut Deed>,
    battles: Query<Entity, With<crate::game::combat::Battle>>,
    mut rng: ResMut<GameRng>,
) {
    let fight = fight(&battles);
    for (effect, strikes, hit_of, magnitude) in &effects {
        let Ok((character, mut vitals)) = members.get_mut(strikes.0) else { continue };
        let lost = vitals.damage(character, magnitude.0, &mut Fate { fight, rng: &mut rng });
        if lost > 0 && !vitals.is_dead() {
            let senses = character.stats(&vitals.enchantments).attribute(Attribute::Senses);
            vitals.conditions.wake_on_hit(senses, &mut rng);
        }
        let died = vitals.is_dead() && lost > 0;
        let outcome = DamageOutcome { damage: lost, died, experience: 0 };
        record(&mut acts, hit_of.0, Effect::Damaged { victim: strikes.0, outcome });
        commands.entity(effect).insert((Outcome::Landed, Dealt(lost)));
    }
}

/// Damage hurts a living monster: its spells absorb what they can, it may
/// wake, and dying ends its spells. A kill is worth its species'
/// experience, unless it was summoned.
pub fn harm_monsters(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes, &HitOf, &Magnitude),
        (With<Harm>, With<Ready>, Without<Outcome>),
    >,
    mut monsters: Query<(&Monster, &mut Vitals, Has<crate::game::magic::pointcasts::SummonedBy>)>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    for (effect, strikes, hit_of, magnitude) in &effects {
        let Ok((monster, mut vitals, summoned)) = monsters.get_mut(strikes.0) else { continue };
        let kind = &content.species[monster.species];
        let lost = hurt_monster(&mut vitals, magnitude.0, kind, &mut rng);
        let died = vitals.is_dead() && lost > 0;
        let experience = if died && !summoned { kind.experience } else { 0 };
        let outcome = DamageOutcome { damage: lost, died, experience };
        record(&mut acts, hit_of.0, Effect::Damaged { victim: strikes.0, outcome });
        commands.entity(effect).insert((Outcome::Landed, Dealt(lost)));
    }
}

/// Take `amount` health from a monster: its spells absorb what they can, it
/// may wake, and dying ends its spells. The health lost.
fn hurt_monster(
    vitals: &mut Vitals,
    amount: u32,
    kind: &crate::content::species::Species,
    rng: &mut GameRng,
) -> u32 {
    if vitals.is_dead() || amount == 0 {
        return 0;
    }
    let amount = vitals.enchantments.absorb(amount);
    let lost = amount.min(vitals.health.current);
    vitals.health.current -= lost;
    if lost == 0 {
        return 0;
    }
    if vitals.is_dead() {
        vitals.enchantments.0.clear();
    } else {
        vitals.conditions.wake_on_hit(u32::from(kind.attributes.senses), rng);
    }
    lost
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
            notices.write(DamageApplied { member: strikes.0, amount: dealt.0, died: vitals.is_dead() });
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

/// A living member loses health for good; nothing wakes them.
pub fn sap(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes, &HitOf, &Magnitude),
        (With<Sap>, With<Ready>, Without<Outcome>),
    >,
    mut members: Query<(&mut Character, &mut Vitals)>,
    mut acts: Query<&mut Deed>,
    battles: Query<Entity, With<crate::game::combat::Battle>>,
    mut rng: ResMut<GameRng>,
) {
    let fight = fight(&battles);
    for (effect, strikes, hit_of, magnitude) in &effects {
        let Ok((mut character, mut vitals)) = members.get_mut(strikes.0) else {
            commands.entity(effect).insert(Outcome::Missed);
            continue;
        };
        let lost =
            vitals.drain_health(&mut character, magnitude.0, &mut Fate { fight, rng: &mut rng });
        let outcome = DamageOutcome { damage: lost, died: vitals.is_dead(), experience: 0 };
        record(&mut acts, hit_of.0, Effect::Damaged { victim: strikes.0, outcome });
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

/// A member's poison bites by its strength (retail
/// `InflictConditionOnTarget`); none without one.
pub fn poison_members(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes, &HitOf, &Afflict, &Magnitude),
        (With<Ready>, Without<Outcome>),
    >,
    mut members: Query<&mut Vitals, With<Character>>,
    mut acts: Query<&mut Deed>,
) {
    for (effect, strikes, hit_of, afflict, magnitude) in &effects {
        if afflict.condition != Condition::Poisoned {
            continue;
        }
        let Ok(mut vitals) = members.get_mut(strikes.0) else { continue };
        if vitals.poison(lasting(afflict, magnitude.0), afflict.poison) {
            record(&mut acts, hit_of.0, Effect::Inflicted { victim: strikes.0, condition: afflict.condition });
            commands.entity(effect).insert(Outcome::Landed);
        } else {
            commands.entity(effect).insert(Outcome::Missed);
        }
    }
}

/// A condition takes hold of a living body (retail
/// `InflictConditionOnTarget`); one standing neutral cannot be turned.
pub fn afflict(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes, &HitOf, &Afflict, &Magnitude),
        (With<Ready>, Without<Outcome>),
    >,
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
        vitals.inflict(condition, lasting(afflict, magnitude.0));
        record(&mut acts, hit_of.0, Effect::Inflicted { victim: strikes.0, condition });
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// A member dies outright, unless they cheat death (retail
/// `SetCharacterCondition` with Dead).
pub fn slay_members(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf), (With<Slay>, With<Ready>, Without<Outcome>)>,
    mut members: Query<(&Character, &mut Vitals)>,
    mut acts: Query<&mut Deed>,
    battles: Query<Entity, With<crate::game::combat::Battle>>,
    mut rng: ResMut<GameRng>,
) {
    let fight = fight(&battles);
    for (effect, strikes, hit_of) in &effects {
        let Ok((character, mut vitals)) = members.get_mut(strikes.0) else { continue };
        let had = vitals.health.current;
        vitals.perish(character, &mut Fate { fight, rng: &mut rng });
        let lost = had - vitals.health.current.min(had);
        let outcome = DamageOutcome { damage: lost, died: vitals.is_dead(), experience: 0 };
        record(&mut acts, hit_of.0, Effect::Damaged { victim: strikes.0, outcome });
        commands.entity(effect).insert((Outcome::Landed, Dealt(lost)));
    }
}

/// A monster dies outright; its Guardian Angel does not save it.
pub fn slay_monsters(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf), (With<Slay>, With<Ready>, Without<Outcome>)>,
    mut monsters: Query<(&Monster, &mut Vitals, Has<crate::game::magic::pointcasts::SummonedBy>)>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    for (effect, strikes, hit_of) in &effects {
        let Ok((monster, mut vitals, summoned)) = monsters.get_mut(strikes.0) else { continue };
        let kind = &content.species[monster.species];
        vitals.enchantments.0.retain(|e| e.spell != SpellId::GUARDIAN_ANGEL);
        let had = vitals.health.current;
        let lost = hurt_monster(&mut vitals, had, kind, &mut rng);
        let died = vitals.is_dead() && lost > 0;
        let experience = if died && !summoned { kind.experience } else { 0 };
        let outcome = DamageOutcome { damage: lost, died, experience };
        record(&mut acts, hit_of.0, Effect::Damaged { victim: strikes.0, outcome });
        commands.entity(effect).insert((Outcome::Landed, Dealt(lost)));
    }
}

pub fn tire(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes, &HitOf, &Magnitude),
        (With<Tire>, With<Ready>, Without<Outcome>),
    >,
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

/// A member's spell points, by the share retail takes of their pool; for
/// the party, every living member's, not only those it reached.
pub fn drain_members_magic(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes, &HitOf, &Magnitude, Has<ForParty>),
        (With<DrainMagic>, With<Ready>, Without<Outcome>),
    >,
    mut members: Query<(&Character, &Vitals, &LearnedSpells, &mut SpellPoints)>,
    mut acts: Query<&mut Deed>,
    roster: Single<&crate::game::party::roster::PartyRoster>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    for (effect, strikes, hit_of, magnitude, for_party) in &effects {
        if !for_party && !members.contains(strikes.0) {
            continue;
        }
        let victims: Vec<Entity> =
            if for_party { roster.iter().collect() } else { vec![strikes.0] };
        for victim in victims {
            let Ok((character, vitals, learned, mut pool)) = members.get_mut(victim) else { continue };
            if vitals.is_dead() {
                continue;
            }
            let maximum = character.stats(&vitals.enchantments).max_points(learned, &content.spells);
            let amount = pool.drain(magnitude.0, &maximum, &mut rng);
            record(&mut acts, hit_of.0, Effect::Drained { victim, amount });
        }
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// A monster's spell budget, directly.
pub fn drain_monsters_magic(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes, &HitOf, &Magnitude),
        (With<DrainMagic>, With<Ready>, Without<Outcome>),
    >,
    mut monsters: Query<&mut Monster>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
) {
    for (effect, strikes, hit_of, magnitude) in &effects {
        let Ok(mut monster) = monsters.get_mut(strikes.0) else { continue };
        let lost = monster.spell_budget(&content.species[monster.species]).min(magnitude.0);
        monster.spell_points -= lost as i32;
        record(&mut acts, hit_of.0, Effect::Drained { victim: strikes.0, amount: lost });
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// A member's spell points in one realm, or in every realm.
pub fn drain_realm(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes, &HitOf, &DrainRealm, &Magnitude),
        (With<Ready>, Without<Outcome>),
    >,
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
    effects: Query<
        (Entity, &Strikes, &HitOf, &Magnitude),
        (With<Heal>, With<Ready>, Without<Outcome>),
    >,
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
    effects: Query<
        (Entity, &Strikes, &HitOf, &Magnitude),
        (With<Refresh>, With<Ready>, Without<Outcome>),
    >,
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
    effects: Query<
        (Entity, &Strikes, &HitOf, &Magnitude),
        (With<Replenish>, With<Ready>, Without<Outcome>),
    >,
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

/// A cure of poison eases a member's poison as it goes.
pub fn ease_poison(
    effects: Query<(&Strikes, &HitOf, &Cure), (With<Ready>, Without<Outcome>)>,
    mut members: Query<&mut Vitals, With<Character>>,
    strengths: Query<&Strength>,
    mut rng: ResMut<GameRng>,
) {
    for (strikes, hit_of, cure) in &effects {
        if !cure.conditions.contains(&Condition::Poisoned) {
            continue;
        }
        let Ok(mut vitals) = members.get_mut(strikes.0) else { continue };
        let strength = strengths.get(hit_of.0).copied().unwrap_or_default();
        vitals.weaken_poison(body::cure_rounds(strength), &mut rng);
    }
}

pub fn cure(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &Cure), (With<Ready>, Without<Outcome>)>,
    mut bodies: Query<&mut Vitals>,
    mut acts: Query<&mut Deed>,
    strengths: Query<&Strength>,
    mut rng: ResMut<GameRng>,
) {
    for (effect, strikes, hit_of, cure) in &effects {
        let victim = strikes.0;
        let strength = strengths.get(hit_of.0).copied().unwrap_or_default();
        let Ok(mut vitals) = bodies.get_mut(victim) else { continue };
        let lifted =
            body::cure(&mut rng, &mut vitals.conditions, cure.conditions, strength, cure.outright);
        let landed = match lifted.first() {
            Some(&condition) => Effect::Cured { victim, condition },
            None => Effect::Nothing { victim },
        };
        record(&mut acts, hit_of.0, landed);
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// Renewal lifts a member's drain and gives back what it took.
pub fn renew_members(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf), (With<Renew>, With<Ready>, Without<Outcome>)>,
    mut members: Query<(&mut Character, &mut Vitals)>,
    mut acts: Query<&mut Deed>,
) {
    for (effect, strikes, hit_of) in &effects {
        let victim = strikes.0;
        let Ok((mut character, mut vitals)) = members.get_mut(victim) else { continue };
        let landed = if vitals.lift_drain(&mut character) {
            Effect::Cured { victim, condition: Condition::Drained }
        } else {
            Effect::Nothing { victim }
        };
        record(&mut acts, hit_of.0, landed);
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// Renewal lifts a monster's drain outright.
pub fn renew_monsters(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf), (With<Renew>, With<Ready>, Without<Outcome>)>,
    mut monsters: Query<&mut Vitals, With<Monster>>,
    mut acts: Query<&mut Deed>,
    strengths: Query<&Strength>,
    mut rng: ResMut<GameRng>,
) {
    for (effect, strikes, hit_of) in &effects {
        let victim = strikes.0;
        let Ok(mut vitals) = monsters.get_mut(victim) else { continue };
        let strength = strengths.get(hit_of.0).copied().unwrap_or_default();
        let drained = [Condition::Drained];
        let lifted = body::cure(&mut rng, &mut vitals.conditions, &drained, strength, true);
        let landed = if lifted.is_empty() {
            Effect::Nothing { victim }
        } else {
            Effect::Cured { victim, condition: Condition::Drained }
        };
        record(&mut acts, hit_of.0, landed);
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// A dead member rises with a share of their health and stamina by their
/// piety and vitality.
pub fn raise(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf), (With<Raise>, With<Ready>, Without<Outcome>)>,
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

/// A monster's clouds shorten, or the party's.
pub fn purify(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes, &HitOf, &Purify, &Magnitude, Has<ForParty>),
        (With<Ready>, Without<Outcome>),
    >,
    mut bodies: Query<&mut Vitals>,
    mut acts: Query<&mut Deed>,
    mut party: Single<&mut PartyEffects>,
    content: If<Res<Catalogs>>,
) {
    for (effect, strikes, hit_of, purify, magnitude, for_party) in &effects {
        let victim = strikes.0;
        let Ok(mut vitals) = bodies.get_mut(victim) else { continue };
        let slots = if for_party { &mut party.0 } else { &mut vitals.enchantments };
        purify_slots(slots, magnitude.0, purify.level, &content.spells);
        record(&mut acts, hit_of.0, Effect::Purified { victim });
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

/// A spell laid on the whole party.
pub fn enchant_party(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &Enchant, &Magnitude), (With<ForParty>, With<Ready>, Without<Outcome>)>,
    mut acts: Query<&mut Deed>,
    strengths: Query<&Strength>,
    mut party: Single<&mut PartyEffects>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    for (effect, strikes, hit_of, enchant, turns) in &effects {
        let strength = strengths.get(hit_of.0).copied().unwrap_or_default();
        let rolled = enchantment(enchant, turns.0, strength, &content.spells, &mut rng);
        let changed = party.0.accepts(enchant.spell, strength.power) && party.0.apply(rolled);
        let victim = strikes.0;
        let spell = enchant.spell;
        record(&mut acts, hit_of.0, if changed { Effect::Enchanted { victim, spell } } else { Effect::Nothing { victim } });
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// A spell laid on a living member, whose maxima follow it; Haste counters
/// Slow.
pub fn enchant_members(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &Enchant, &Magnitude), (Without<ForParty>, With<Ready>, Without<Outcome>)>,
    mut members: Query<(&Character, &mut Vitals)>,
    mut acts: Query<&mut Deed>,
    strengths: Query<&Strength>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    use crate::game::magic::enchantments::haste;
    for (effect, strikes, hit_of, enchant, turns) in &effects {
        let victim = strikes.0;
        let Ok((character, mut vitals)) = members.get_mut(victim) else { continue };
        let strength = strengths.get(hit_of.0).copied().unwrap_or_default();
        let rolled = enchantment(enchant, turns.0, strength, &content.spells, &mut rng);
        let vitals = &mut *vitals;
        let changed = if vitals.is_dead() {
            false
        } else if enchant.spell == SpellId::HASTE {
            haste(&mut vitals.conditions, &mut vitals.enchantments, rolled, false)
        } else {
            vitals.enchantments.accepts(enchant.spell, strength.power)
                && vitals.enchantments.apply(rolled)
        };
        vitals.sync_maxima(character);
        let spell = enchant.spell;
        record(&mut acts, hit_of.0, if changed { Effect::Enchanted { victim, spell } } else { Effect::Nothing { victim } });
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// A spell laid on a living monster, by the rules for monsters' spells.
pub fn enchant_monsters(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &Enchant, &Magnitude), (Without<ForParty>, With<Ready>, Without<Outcome>)>,
    mut monsters: Query<&mut Vitals, With<Monster>>,
    mut acts: Query<&mut Deed>,
    strengths: Query<&Strength>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    use crate::game::magic::enchantments::haste;
    for (effect, strikes, hit_of, enchant, turns) in &effects {
        let victim = strikes.0;
        let Ok(mut vitals) = monsters.get_mut(victim) else { continue };
        let strength = strengths.get(hit_of.0).copied().unwrap_or_default();
        let rolled = enchantment(enchant, turns.0, strength, &content.spells, &mut rng);
        let vitals = &mut *vitals;
        let changed = if vitals.is_dead() {
            false
        } else if enchant.spell == SpellId::HASTE {
            haste(&mut vitals.conditions, &mut vitals.enchantments, rolled, true)
        } else {
            vitals.enchantments.monster_accepts(enchant.spell, strength.power)
                && vitals.enchantments.apply_to_monster(rolled)
        };
        let spell = enchant.spell;
        record(&mut acts, hit_of.0, if changed { Effect::Enchanted { victim, spell } } else { Effect::Nothing { victim } });
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

/// Potions, scrolls and books burn: what a body carries in every slot,
/// alternate hands included, or the party's pool.
pub fn burn(
    mut commands: Commands,
    effects: Query<
        (Entity, &HitOf, &Strikes, &Magnitude, Has<ForParty>),
        (With<Burn>, With<Ready>, Without<Outcome>),
    >,
    mut carrying: Query<&mut crate::game::inventory::Carried>,
    mut acts: Query<&mut Deed>,
    mut inventory: Single<&mut PartyInventory>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    let items = &content.items;
    for (effect, hit_of, strikes, chance, for_party) in &effects {
        let chance = chance.0;
        let mut totals = [0; 3];
        if !for_party && let Ok(mut carried) = carrying.get_mut(strikes.0) {
            for (_, slot) in &mut carried.equipment {
                if let Some(equipped) = slot.take() {
                    *slot = burn_stack(equipped.stack, items, chance, &mut rng, &mut totals).map(
                        |stack| crate::game::inventory::Equipped {
                            stack,
                            curse_released: equipped.curse_released,
                        },
                    );
                }
            }
            for slot in &mut carried.backpack {
                if let Some(stack) = slot.take() {
                    *slot = burn_stack(stack, items, chance, &mut rng, &mut totals);
                }
            }
        }
        if for_party {
            let pool = &mut inventory.pool.stacks;
            *pool = std::mem::take(pool)
                .into_iter()
                .filter_map(|stack| burn_stack(stack, items, chance, &mut rng, &mut totals))
                .collect();
        }
        let classes = [EquipClass::Potion, EquipClass::Scroll, EquipClass::Spellbook];
        for (class, amount) in classes.into_iter().zip(totals) {
            if amount != 0 {
                record(&mut acts, hit_of.0, Effect::Destroyed { class, amount });
            }
        }
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// The party loses a share of its gold.
pub fn lose_gold(
    mut commands: Commands,
    effects: Query<(Entity, &HitOf, &Magnitude), (With<LoseGold>, With<Ready>, Without<Outcome>)>,
    mut acts: Query<&mut Deed>,
    mut inventory: Single<&mut PartyInventory>,
) {
    for (effect, hit_of, percent) in &effects {
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
    effects: Query<Entity, (With<Dispel>, With<Ready>, Without<Outcome>)>,
    mut party: Single<&mut PartyEffects>,
) {
    for effect in &effects {
        party.0.0.retain(|e| crate::game::magic::enchantments::combat_only(e.spell));
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// Settled effects go: a follower that needed the opposite outcome goes
/// with it; the others may now be settled in turn.
pub fn release_followers(
    mut commands: Commands,
    settled: Query<(Entity, &Outcome, Option<&FollowedBy>)>,
    conditional: Query<(Has<OnlyIfLanded>, Has<OnlyIfResisted>)>,
) {
    for (effect, &outcome, followers) in &settled {
        for &follower in followers.into_iter().flat_map(|f| f.iter()).collect::<Vec<_>>().iter() {
            let Ok((if_landed, if_resisted)) = conditional.get(follower) else { continue };
            if (if_landed && outcome != Outcome::Landed)
                || (if_resisted && outcome != Outcome::Resisted)
            {
                commands.entity(follower).insert(Outcome::Missed);
            } else {
                commands.entity(follower).remove::<(OnlyIfLanded, OnlyIfResisted)>();
            }
        }
        commands.entity(effect).despawn();
    }
}
