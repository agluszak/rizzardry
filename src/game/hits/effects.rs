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

use super::body::{self, Body, BodyItem, BodyReadOnlyItem, wound};
use super::{Deed, Effect, HitOf, Strikes};
use crate::content::characters::{Attribute, Realm};
use crate::content::items::{EquipClass, ItemCatalog};
use crate::content::species::MonsterCatalog;
use crate::content::spells::SpellId;
use crate::game::characters::{Condition, Lasting};
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

/// A condition, or death when `condition` is `None`; `indefinite` lasts
/// until lifted, whatever its magnitude.
#[derive(Component, Clone, Copy, Debug)]
pub struct Afflict {
    pub condition: Option<Condition>,
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

/// A body whose kind cannot suffer a condition, or cannot be slain, shrugs
/// it off before any save.
pub fn immunities(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &Afflict), (With<Ready>, Without<Outcome>)>,
    bodies: Query<Body>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
) {
    for (effect, strikes, hit_of, afflict) in &effects {
        let Ok(body) = bodies.get(strikes.0) else { continue };
        let immune = match afflict.condition {
            Some(condition) => body.immune(condition, &content.species),
            None => body.death_immune(&content.species),
        };
        if immune {
            record(&mut acts, hit_of.0, Effect::Resisted { victim: strikes.0 });
            commands.entity(effect).insert(Outcome::Resisted);
        }
    }
}

/// The body's defence against an effect in `realm`.
fn defence(
    body: &BodyReadOnlyItem,
    realm: Realm,
    items: &ItemCatalog,
    species: &MonsterCatalog,
    party: &Enchantments,
) -> body::Defence {
    body.defence(realm, items, species, party)
}

/// The strength an effect is saved against and resisted at: its act's,
/// halved for a weaker follow-up.
fn against(strengths: &Query<&Strength>, act: Entity, half: bool) -> u32 {
    let against = strengths.get(act).map_or(0, |strength| strength.against);
    if half { against / 2 } else { against }
}

/// A living body may save against an effect outright (retail
/// `TargetResistsCondition`); a member who saves practises Iron Will.
pub fn saves(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes, &HitOf, &Save, &Resist, Has<HalfStrength>),
        (With<Ready>, Without<Outcome>),
    >,
    mut bodies: Query<Body>,
    mut acts: Query<&mut Deed>,
    strengths: Query<&Strength>,
    party: Single<&PartyEffects>,
    content: If<Res<Catalogs>>,
    difficulty: Res<Difficulty>,
    mut rng: ResMut<GameRng>,
) {
    let catalogs = (&content.items, &content.species, &party.0);
    for (effect, strikes, hit_of, save, resist, half) in &effects {
        let against = against(&strengths, hit_of.0, half);
        let Ok(body) = bodies.get(strikes.0) else { continue };
        let defence = defence(&body, resist.realm, catalogs.0, catalogs.1, catalogs.2);
        let mut body = bodies.get_mut(strikes.0).unwrap();
        let (saved, practised) = body.saves(&mut rng, &defence, save.0, against, *difficulty);
        if let Ok(mut deed) = acts.get_mut(hit_of.0) {
            deed.improved.extend(practised.map(|skill| (strikes.0, skill)));
        }
        if saved {
            record(&mut acts, hit_of.0, Effect::Resisted { victim: strikes.0 });
            commands.entity(effect).insert(Outcome::Resisted);
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
    bodies: Query<Body>,
    mut acts: Query<&mut Deed>,
    strengths: Query<&Strength>,
    party: Single<&PartyEffects>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    let catalogs = (&content.items, &content.species, &party.0);
    for (effect, strikes, hit_of, resist, mut magnitude, half) in &mut effects {
        let against = against(&strengths, hit_of.0, half);
        let Ok(body) = bodies.get(strikes.0) else { continue };
        let defence = defence(&body, resist.realm, catalogs.0, catalogs.1, catalogs.2);
        magnitude.0 =
            body::resist(&mut rng, magnitude.0, defence.resistance, defence.level, against);
        if magnitude.0 == 0 {
            record(&mut acts, hit_of.0, Effect::Resisted { victim: strikes.0 });
            commands.entity(effect).insert(Outcome::Resisted);
        }
    }
}

/// Damage wounds a living body; a drain's act counts what it took.
pub fn harm(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes, &HitOf, &Magnitude),
        (With<Harm>, With<Ready>, Without<Outcome>),
    >,
    mut bodies: Query<Body>,
    mut acts: Query<(&mut Deed, Option<&mut super::spells::Drains>)>,
    content: If<Res<Catalogs>>,
    fight: Query<Entity, With<crate::game::combat::Battle>>,
    mut rng: ResMut<GameRng>,
    mut notices: MessageWriter<DamageApplied>,
) {
    let fight = fight.single().ok();
    for (effect, strikes, hit_of, magnitude) in &effects {
        let Ok(mut body) = bodies.get_mut(strikes.0) else { continue };
        let hurt = wound(
            &mut body,
            magnitude.0,
            &mut rng,
            &content.species,
            fight,
            &mut commands,
            &mut notices,
        );
        if let Ok((mut deed, drains)) = acts.get_mut(hit_of.0) {
            if let (Effect::Damaged { outcome, .. }, Some(mut drains)) = (hurt, drains) {
                drains.drained += outcome.damage;
            }
            deed.effects.push(hurt);
        }
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// What slaying or sapping a body did: a member's notice, a monster's kill.
fn slain(
    body: &BodyItem,
    lost: u32,
    species: &MonsterCatalog,
    notices: &mut MessageWriter<DamageApplied>,
) -> Effect {
    let died = body.is_dead();
    if body.character.is_some() && lost > 0 {
        notices.write(DamageApplied { member: body.entity, amount: lost, died });
    }
    let experience = match &body.monster {
        Some(monster) if died && !body.summoned => species[monster.species].experience,
        _ => 0,
    };
    Effect::Damaged {
        victim: body.entity,
        outcome: DamageOutcome { damage: lost, died, experience },
    }
}

/// A living member loses health for good; nothing wakes them.
pub fn sap(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes, &HitOf, &Magnitude),
        (With<Sap>, With<Ready>, Without<Outcome>),
    >,
    mut bodies: Query<Body>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
    fight: Query<Entity, With<crate::game::combat::Battle>>,
    mut rng: ResMut<GameRng>,
    mut notices: MessageWriter<DamageApplied>,
) {
    let fight = fight.single().ok();
    for (effect, strikes, hit_of, magnitude) in &effects {
        let Ok(mut body) = bodies.get_mut(strikes.0) else { continue };
        let Some(lost) = body.sap(magnitude.0, &mut rng, fight) else {
            commands.entity(effect).insert(Outcome::Missed);
            continue;
        };
        record(&mut acts, hit_of.0, slain(&body, lost, &content.species, &mut notices));
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// A condition takes hold of a living body, unless its nature shrugs it off;
/// death slays it (retail `InflictConditionOnTarget`).
pub fn afflict(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes, &HitOf, &Afflict, &Magnitude, Has<Save>),
        (With<Ready>, Without<Outcome>),
    >,
    mut bodies: Query<Body>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
    fight: Query<Entity, With<crate::game::combat::Battle>>,
    mut rng: ResMut<GameRng>,
    mut notices: MessageWriter<DamageApplied>,
) {
    let (species, fight) = (&content.species, fight.single().ok());
    for (effect, strikes, hit_of, afflict, magnitude, saveable) in &effects {
        let victim = strikes.0;
        let Ok(read) = bodies.get(victim) else { continue };
        let shrugs =
            afflict.condition.is_some_and(|c| saveable && read.shrugs(c, afflict.indefinite));
        let side = read.side.0;
        let mut body = bodies.get_mut(victim).unwrap();
        let Some(condition) = afflict.condition else {
            let lost = body.slay(&mut rng, species, fight);
            record(&mut acts, hit_of.0, slain(&body, lost, species, &mut notices));
            commands.entity(effect).insert(Outcome::Landed);
            continue;
        };
        // What a body may save against, its nature may shrug off.
        if shrugs {
            record(&mut acts, hit_of.0, Effect::Resisted { victim });
            commands.entity(effect).insert(Outcome::Resisted);
            continue;
        }
        let lasting = match magnitude.0 {
            _ if afflict.indefinite => Lasting::Indefinite,
            1 => Lasting::Rounds(2),
            rounds => Lasting::Rounds(rounds),
        };
        if body.inflict(condition, afflict.poison, lasting, side) {
            record(&mut acts, hit_of.0, Effect::Inflicted { victim, condition });
            commands.entity(effect).insert(Outcome::Landed);
        } else {
            commands.entity(effect).insert(Outcome::Missed);
        }
    }
}

/// A death spell slays a living body (retail `InflictConditionOnTarget` for
/// Dead).
pub fn slay(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf), (With<Slay>, With<Ready>, Without<Outcome>)>,
    mut bodies: Query<Body>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
    fight: Query<Entity, With<crate::game::combat::Battle>>,
    mut rng: ResMut<GameRng>,
    mut notices: MessageWriter<DamageApplied>,
) {
    let (species, fight) = (&content.species, fight.single().ok());
    for (effect, strikes, hit_of) in &effects {
        let Ok(mut body) = bodies.get_mut(strikes.0) else { continue };
        let lost = body.slay(&mut rng, species, fight);
        record(&mut acts, hit_of.0, slain(&body, lost, species, &mut notices));
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// A living body tires.
pub fn tire(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes, &HitOf, &Magnitude),
        (With<Tire>, With<Ready>, Without<Outcome>),
    >,
    mut bodies: Query<Body>,
    mut acts: Query<&mut Deed>,
) {
    for (effect, strikes, hit_of, magnitude) in &effects {
        let Ok(mut body) = bodies.get_mut(strikes.0) else { continue };
        let amount = body.vitals.tire(magnitude.0);
        record(&mut acts, hit_of.0, Effect::Tired { victim: strikes.0, amount });
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// A living body loses spell points: a member by the share retail takes, a
/// monster from its budget.
pub fn drain_magic(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes, &HitOf, &Magnitude, Has<ForParty>),
        (With<DrainMagic>, With<Ready>, Without<Outcome>),
    >,
    mut bodies: Query<Body>,
    mut acts: Query<&mut Deed>,
    roster: Single<&crate::game::party::roster::PartyRoster>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    for (effect, strikes, hit_of, magnitude, for_party) in &effects {
        // Anti-Magic drains every living member, not only those it reached.
        if for_party {
            for member in roster.iter() {
                let mut body = bodies.get_mut(member).unwrap();
                if !body.is_dead() {
                    let amount =
                        body.drain_magic(magnitude.0, &mut rng, &content.spells, &content.species);
                    record(&mut acts, hit_of.0, Effect::Drained { victim: member, amount });
                }
            }
            commands.entity(effect).insert(Outcome::Landed);
            continue;
        }
        let Ok(mut body) = bodies.get_mut(strikes.0) else { continue };
        let amount = body.drain_magic(magnitude.0, &mut rng, &content.spells, &content.species);
        record(&mut acts, hit_of.0, Effect::Drained { victim: strikes.0, amount });
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// A living member loses spell points in one realm, or in each.
pub fn drain_realm(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes, &HitOf, &DrainRealm, &Magnitude),
        (With<Ready>, Without<Outcome>),
    >,
    mut bodies: Query<Body>,
    mut acts: Query<&mut Deed>,
    content: If<Res<Catalogs>>,
) {
    for (effect, strikes, hit_of, drain, magnitude) in &effects {
        let Ok(read) = bodies.get(strikes.0) else { continue };
        let maximum = read.mana_maximum(&content.spells);
        let mut body = bodies.get_mut(strikes.0).unwrap();
        let (Some(maximum), Some((_, pool))) = (maximum, body.magic.as_mut()) else {
            commands.entity(effect).insert(Outcome::Missed);
            continue;
        };
        let realms = drain.0.map_or(enumset::EnumSet::all(), enumset::EnumSet::only);
        let amount: u32 =
            realms.iter().map(|realm| pool.drain_realm(realm, magnitude.0, &maximum)).sum();
        record(&mut acts, hit_of.0, Effect::Drained { victim: strikes.0, amount });
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// A living body heals, up to its maximum.
pub fn heal(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes, &HitOf, &Magnitude),
        (With<Heal>, With<Ready>, Without<Outcome>),
    >,
    mut bodies: Query<Body>,
    mut acts: Query<&mut Deed>,
) {
    for (effect, strikes, hit_of, magnitude) in &effects {
        let victim = strikes.0;
        let Ok(mut body) = bodies.get_mut(victim) else { continue };
        let amount = body.vitals.heal(magnitude.0);
        let landed = if amount == 0 {
            Effect::Nothing { victim }
        } else {
            Effect::Healed { victim, amount }
        };
        record(&mut acts, hit_of.0, landed);
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// A living body gets its breath back, up to its maximum.
pub fn refresh(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes, &HitOf, &Magnitude),
        (With<Refresh>, With<Ready>, Without<Outcome>),
    >,
    mut bodies: Query<Body>,
    mut acts: Query<&mut Deed>,
) {
    for (effect, strikes, hit_of, magnitude) in &effects {
        let victim = strikes.0;
        let Ok(mut body) = bodies.get_mut(victim) else { continue };
        let amount = body.vitals.rest(magnitude.0);
        let landed = if amount == 0 {
            Effect::Nothing { victim }
        } else {
            Effect::Refreshed { victim, amount }
        };
        record(&mut acts, hit_of.0, landed);
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// A living member's spell points come back, spread over the realms.
pub fn replenish(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes, &HitOf, &Magnitude),
        (With<Replenish>, With<Ready>, Without<Outcome>),
    >,
    mut bodies: Query<Body>,
    mut acts: Query<&mut Deed>,
) {
    for (effect, strikes, hit_of, magnitude) in &effects {
        let victim = strikes.0;
        let Ok(mut body) = bodies.get_mut(victim) else { continue };
        let Some((_, pool)) = body.magic.as_mut() else {
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

/// Conditions lift; taking rounds off a member's poison weakens it as aging
/// does.
pub fn cure(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf, &Cure), (With<Ready>, Without<Outcome>)>,
    mut bodies: Query<Body>,
    mut acts: Query<&mut Deed>,
    strengths: Query<&Strength>,
    mut rng: ResMut<GameRng>,
) {
    for (effect, strikes, hit_of, cure) in &effects {
        let victim = strikes.0;
        let strength = strengths.get(hit_of.0).copied().unwrap_or_default();
        let Ok(mut body) = bodies.get_mut(victim) else { continue };
        if body.character.is_some() && cure.conditions.contains(&Condition::Poisoned) {
            body.vitals.weaken_poison(body::cure_rounds(strength), &mut rng);
        }
        let conditions = &mut body.vitals.conditions;
        let lifted = body::cure(&mut rng, conditions, cure.conditions, strength, cure.outright);
        let landed = match lifted.first() {
            Some(&condition) => Effect::Cured { victim, condition },
            None => Effect::Nothing { victim },
        };
        record(&mut acts, hit_of.0, landed);
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// Drained lifts: a member gets back what draining and disease took.
pub fn renew(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf), (With<Renew>, With<Ready>, Without<Outcome>)>,
    mut bodies: Query<Body>,
    mut acts: Query<&mut Deed>,
    strengths: Query<&Strength>,
    mut rng: ResMut<GameRng>,
) {
    for (effect, strikes, hit_of) in &effects {
        let strength = strengths.get(hit_of.0).copied().unwrap_or_default();
        let victim = strikes.0;
        let Ok(mut body) = bodies.get_mut(victim) else { continue };
        let lifted = match body.character.as_mut() {
            Some(character) => body.vitals.lift_drain(character),
            None => !body::cure(
                &mut rng,
                &mut body.vitals.conditions,
                &[Condition::Drained],
                strength,
                true,
            )
            .is_empty(),
        };
        let landed = if lifted {
            Effect::Cured { victim, condition: Condition::Drained }
        } else {
            Effect::Nothing { victim }
        };
        record(&mut acts, hit_of.0, landed);
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// A dead member rises with health and stamina at `max·((PIE + VIT) >> 2)/100`,
/// at least 1.
pub fn raise(
    mut commands: Commands,
    effects: Query<(Entity, &Strikes, &HitOf), (With<Raise>, With<Ready>, Without<Outcome>)>,
    mut bodies: Query<Body>,
    mut acts: Query<&mut Deed>,
) {
    for (effect, strikes, hit_of) in &effects {
        let victim = strikes.0;
        let Ok(mut body) = bodies.get_mut(victim) else { continue };
        let Some(character) = body.character.as_deref().cloned() else {
            commands.entity(effect).insert(Outcome::Missed);
            continue;
        };
        if !body.is_dead() {
            record(&mut acts, hit_of.0, Effect::Nothing { victim });
            commands.entity(effect).insert(Outcome::Missed);
            continue;
        }
        let stats = character.stats(&body.vitals.enchantments);
        let share = (stats.attribute(Attribute::Piety) + stats.attribute(Attribute::Vitality)) >> 2;
        let vitals = &mut body.vitals;
        vitals.health.current = (vitals.health.max * share / 100).max(1);
        vitals.stamina.current = (vitals.stamina.max * share / 100).max(1);
        record(&mut acts, hit_of.0, Effect::Raised { victim });
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// Shorten the offensive clouds among `slots`: by more the stronger the
/// purifying spell is than each cloud.
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
    mut bodies: Query<Body>,
    mut acts: Query<&mut Deed>,
    mut party: Single<&mut PartyEffects>,
    content: If<Res<Catalogs>>,
) {
    for (effect, strikes, hit_of, purify, magnitude, for_party) in &effects {
        let victim = strikes.0;
        let Ok(mut body) = bodies.get_mut(victim) else { continue };
        let slots = if for_party { &mut party.0 } else { &mut body.vitals.enchantments };
        purify_slots(slots, magnitude.0, purify.level, &content.spells);
        record(&mut acts, hit_of.0, Effect::Purified { victim });
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// A spell is laid on a living body, or on the party; a member's maxima
/// follow at once.
pub fn enchant(
    mut commands: Commands,
    effects: Query<
        (Entity, &Strikes, &HitOf, &Enchant, &Magnitude, Has<ForParty>),
        (With<Ready>, Without<Outcome>),
    >,
    mut bodies: Query<Body>,
    mut acts: Query<&mut Deed>,
    strengths: Query<&Strength>,
    mut party: Single<&mut PartyEffects>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    use crate::game::magic::enchantments::haste;
    for (effect, strikes, hit_of, enchant, turns, for_party) in &effects {
        let victim = strikes.0;
        let Ok(mut body) = bodies.get_mut(victim) else { continue };
        let strength = strengths.get(hit_of.0).copied().unwrap_or_default();
        let spell = enchant.spell;
        let power = strength.power;
        // Guardian Angel absorbs its rolled dice a power.
        let absorption = if spell == SpellId::GUARDIAN_ANGEL {
            let dice = content.spells[spell].dice;
            let rolled = dice.map_or(0, |dice| rng.roll(dice)) * power;
            rolled + rolled * strength.boost / 100
        } else {
            0
        };
        let rolled =
            Enchantment { spell, power, boost: strength.boost, turns: turns.0, absorption };
        let member = body.character.is_some();
        let changed = if for_party {
            party.0.accepts(spell, power) && party.0.apply(rolled)
        } else if body.is_dead() {
            false
        } else if spell == SpellId::HASTE {
            let vitals = &mut *body.vitals;
            haste(&mut vitals.conditions, &mut vitals.enchantments, rolled, !member)
        } else if member {
            body.vitals.enchantments.accepts(spell, power) && body.vitals.enchantments.apply(rolled)
        } else {
            body.vitals.enchantments.monster_accepts(spell, power)
                && body.vitals.enchantments.apply_to_monster(rolled)
        };
        if let Some(character) = body.character.as_deref().cloned() {
            body.vitals.sync_maxima(&character);
        }
        let landed =
            if changed { Effect::Enchanted { victim, spell } } else { Effect::Nothing { victim } };
        record(&mut acts, hit_of.0, landed);
        commands.entity(effect).insert(Outcome::Landed);
    }
}

/// Consume physical units, not charges, in place.
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
    mut bodies: Query<Body>,
    mut acts: Query<&mut Deed>,
    mut inventory: Single<&mut PartyInventory>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    let items = &content.items;
    for (effect, hit_of, strikes, chance, for_party) in &effects {
        let chance = chance.0;
        let mut totals = [0; 3];
        if !for_party
            && let Ok(mut body) = bodies.get_mut(strikes.0)
            && let Some(carried) = body.carried.as_mut()
        {
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
