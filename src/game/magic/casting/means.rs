//! What a cast is cast with (`docs/rules/casting.md` sections 2 to 4 and 7):
//! a learned spell, an item, or a monster's own casting. Each is a component
//! of the cast whose hook sets the observers of the steps that differ by it:
//! whether it may be cast, its power and how likely it fails, and what it
//! costs and trains.

use bevy::ecs::lifecycle::HookContext;
use bevy::ecs::world::DeferredWorld;
use bevy::prelude::*;

use super::aiming::{Released, Spent};
use super::{
    Aimed, Allowed, Ask, CastBy, CastSpell, Chosen, Failure, Pay, Pays, Power, Practice, Recast,
    Release, SeesBlind, Source, Spread, Worked, refuse_cast,
};
use crate::content::characters::{Skill, Trait};
use crate::content::items::UseType;
use crate::content::spells::{PowerChoice, SpellDefinition, SpellId, Working};
use crate::game::actors::{CanFight, KeepsAlchemy, Modifiers, Skills, Traits, UsableSkills};
use crate::game::camp::Camp;
use crate::game::characters::{
    Birth, Calling, Condition, Conditions, Practise, Profession, Training, Vitals,
};
use crate::game::combat::{Battle, Round};
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::items::{Burden, Casts, Items, Spend};
use crate::game::magic::may::{self, Refusal, UseContext};
use crate::game::magic::occasion::{Occupied, SpokenTo, Unsteady};
use crate::game::magic::reach::Aim;
use crate::game::magic::roll::{self, Pace};
use crate::game::magic::{LearnedSpells, SpellPoints, cost, power};
use crate::game::party::roster::{PartyMember, PartyRoster};
use crate::game::rng::GameRng;

/// A learned spell, at the power asked for or [`power::AUTO`], paid in points
/// of its realm.
#[derive(Component, Clone, Copy, Debug)]
#[component(on_add = cast_from_learning)]
pub struct LearnedSpell {
    pub asked: u32,
}

/// An item's spell, paid with one use of the item.
#[derive(Component, Clone, Copy, Debug)]
#[component(on_add = cast_from_item)]
pub struct FromItem(pub Entity);

/// A monster's own casting at the power it chose: its budget stands in for
/// skill, its level sets the strength, and it pays nothing.
#[derive(Component, Clone, Copy, Debug)]
#[component(on_add = cast_innately)]
pub struct Innate {
    pub power: u32,
    pub budget: u32,
    pub level: u8,
}

fn cast_from_learning(mut world: DeferredWorld, context: HookContext) {
    world
        .commands()
        .entity(context.entity)
        .observe(may_learned)
        .observe(power_learned)
        .observe(pay_learned)
        .observe(practise_learned)
        .observe(recast)
        .observe(again)
        .observe(trained_by_work);
}

fn cast_from_item(mut world: DeferredWorld, context: HookContext) {
    world
        .commands()
        .entity(context.entity)
        .observe(may_item)
        .observe(power_item)
        .observe(pay_item)
        .observe(practise_item)
        .observe(trained_by_work);
}

fn cast_innately(mut world: DeferredWorld, context: HookContext) {
    world
        .commands()
        .entity(context.entity)
        .observe(may_innate)
        .observe(power_innate)
        .observe(pay_innate);
}

/// The highest power of a spell the points pay for now.
pub fn affordable(pool: &SpellPoints, spell: &SpellDefinition) -> u32 {
    spell.realm.map_or(0, |realm| power::affordable(pool.available(realm), spell.cost))
}

/// Whether an auto cast's caster can go again: stamina past the spell's
/// level and points for it at power 1.
pub fn goes_again((stamina, pool): (u32, &SpellPoints), spell: &SpellDefinition) -> bool {
    spell.realm.is_some_and(|realm| {
        power::goes_again(stamina, spell.level, pool.available(realm), spell.cost)
    })
}

/// How the round paces a roll now: a moment late in it fails more often.
fn pace(round: Option<&Round>, difficulty: Difficulty) -> Pace {
    Pace { difficulty, moment: round.map_or(0, Round::moment) }
}

/// How likely a member's blind aim holds anyway: half, scaled by their
/// profession level, with the Effective While Blind trait.
fn sees_blind(traits: &Traits, calling: &Calling) -> SeesBlind {
    match traits.0.contains(Trait::EffectiveWhileBlind) {
        true => SeesBlind(calling.scale(50.0) as u32),
        false => SeesBlind(0),
    }
}

/// A learned spell may be cast (section 2, in its order): by a member able
/// to fight, not silenced but for Alchemy kept, asked at some power, known,
/// with points for power 1, a spell the game resolves, at a moment it suits.
#[allow(clippy::type_complexity)]
fn may_learned(
    ask: On<Ask>,
    casts: Query<(&CastBy, &CastSpell, &LearnedSpell)>,
    casters: Query<(
        Has<CanFight>,
        Has<PartyMember>,
        &Conditions,
        &LearnedSpells,
        &SpellPoints,
        Has<KeepsAlchemy>,
    )>,
    party: Single<(&UseContext, Has<Occupied>, Has<SpokenTo>, Has<Unsteady>), With<PartyRoster>>,
    camp: Option<Res<Camp>>,
    mut commands: Commands,
) -> Result {
    let (&CastBy(caster), CastSpell(spell), &LearnedSpell { asked }) = casts.get(ask.cast)?;
    let (&context, occupied, spoken_to, unsteady) = *party;
    let suits = |repeats| {
        may::suits(spell, context, occupied, camp.is_some(), spoken_to, unsteady, repeats)
    };
    let refusal = match casters.get(caster) {
        Ok((true, true, conditions, learned, points, keeps)) => {
            let repeats = asked == power::AUTO && spell.power == PowerChoice::ChosenOrRepeat;
            if conditions.has(Condition::Silenced) && may::silence_stops(spell, keeps) {
                Some(Refusal::Silenced)
            } else if asked == 0 {
                Some(Refusal::CannotUse)
            } else if !learned.knows(spell.id) {
                Some(Refusal::NotLearned)
            } else if affordable(points, spell) == 0 {
                Some(Refusal::NoSpellPoints)
            } else if !resolvable(spell) {
                Some(Refusal::Unsupported)
            } else if !suits(repeats) {
                Some(Refusal::WrongTime)
            } else {
                None
            }
        }
        _ => Some(Refusal::CannotAct),
    };
    match refusal {
        Some(refusal) => refuse_cast(&mut commands, (ask.cast, caster), refusal),
        None => commands.trigger(Allowed { cast: ask.cast }),
    }
    Ok(())
}

/// An item's spell may be cast (section 2, in its order): by a member able to
/// fight; out of the pool only outside a fight; an instrument not silenced but
/// for Alchemy kept; the item still there, carried by the caster or pooled;
/// casting a spell at some power, or played; one its user may set off; with a
/// charge left; a spell the game resolves; at a moment it suits.
#[allow(clippy::type_complexity)]
fn may_item(
    ask: On<Ask>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, &FromItem)>,
    casters: Query<(Has<CanFight>, Has<PartyMember>, &Conditions, Has<KeepsAlchemy>)>,
    users: Query<((&Calling, &Birth), &crate::game::actors::Attributes, &Skills)>,
    held: Items,
    party: Single<(&UseContext, Has<Occupied>, Has<SpokenTo>, Has<Unsteady>), With<PartyRoster>>,
    camp: Option<Res<Camp>>,
) -> Result {
    let (&CastBy(caster), CastSpell(spell), &FromItem(item)) = casts.get(ask.cast)?;
    let (&context, occupied, spoken_to, unsteady) = *party;
    let fighting = context == UseContext::Combat;
    let pooled = held.pooled(item);
    let there = (pooled || held.bearer(item) == Some(caster)) && held.id(item).is_some();
    let casts = held.casts(item).filter(|_| there);
    let refusal = match casters.get(caster) {
        Ok((true, true, conditions, keeps)) => {
            let instrument = casts.is_some_and(|casts| casts.by == UseType::Instrument);
            let worn = held.slot(item).is_some_and(|slot| slot.is_active());
            let user = |casts: &Casts| {
                users.get(caster).is_ok_and(|(user, attributes, skills)| {
                    may::activates(casts, worn, user, attributes, skills)
                })
            };
            if pooled && fighting {
                Some(Refusal::CannotUse)
            } else if conditions.has(Condition::Silenced)
                && (instrument || !there)
                && may::silence_stops(spell, keeps)
            {
                Some(Refusal::Silenced)
            } else if !there {
                Some(Refusal::NoItem)
            } else if let Some(casts) = casts
                && (casts.power != 0 || matches!(casts.by, UseType::Instrument | UseType::Gadget))
                && user(casts)
            {
                if held.charges(item) == Some(0) {
                    Some(Refusal::NoCharges)
                } else if !resolvable(spell) {
                    Some(Refusal::Unsupported)
                } else if !may::suits(
                    spell,
                    context,
                    occupied,
                    camp.is_some(),
                    spoken_to,
                    unsteady,
                    false,
                ) {
                    Some(Refusal::WrongTime)
                } else {
                    None
                }
            } else {
                Some(Refusal::CannotUse)
            }
        }
        _ => Some(Refusal::CannotAct),
    };
    if let Some(refusal) = refusal {
        refuse_cast(&mut commands, (ask.cast, caster), refusal);
        return Ok(());
    }
    commands.trigger(Allowed { cast: ask.cast });
    Ok(())
}

/// A monster's own spell may be cast by one able to fight, not silenced but
/// for Alchemy kept by its kind, if the game resolves it; the moment does not
/// judge it.
fn may_innate(
    ask: On<Ask>,
    casts: Query<(&CastBy, &CastSpell)>,
    casters: Query<(Has<CanFight>, &Conditions, Has<KeepsAlchemy>)>,
    mut commands: Commands,
) -> Result {
    let (&CastBy(caster), CastSpell(spell)) = casts.get(ask.cast)?;
    let refusal = match casters.get(caster) {
        Ok((true, conditions, keeps)) => {
            if conditions.has(Condition::Silenced) && may::silence_stops(spell, keeps) {
                Some(Refusal::Silenced)
            } else if !resolvable(spell) {
                Some(Refusal::Unsupported)
            } else {
                None
            }
        }
        _ => Some(Refusal::CannotAct),
    };
    match refusal {
        Some(refusal) => refuse_cast(&mut commands, (ask.cast, caster), refusal),
        None => commands.trigger(Allowed { cast: ask.cast }),
    }
    Ok(())
}

/// A learned spell's power: the power asked for as far as the points reach,
/// or, left to the caster, worked out for the one aimed at; its strength
/// grows from the caster's total caster level, boosted by Power Cast; it
/// fails by the caster's skill and level.
#[allow(clippy::type_complexity)]
fn power_learned(
    aimed: On<Aimed>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, &LearnedSpell, &Chosen)>,
    casters: Query<(
        (&Calling, &Training, &Traits, &UsableSkills),
        (&Skills, &crate::game::actors::CasterLevels),
        &SpellPoints,
    )>,
    victims: Query<(&Vitals, &Conditions, Has<PartyMember>)>,
    held: Items,
    round: Option<Single<&Round, With<Battle>>>,
    difficulty: Res<Difficulty>,
) -> Result {
    let cast = aimed.cast;
    let (&CastBy(caster), CastSpell(spell), &LearnedSpell { asked }, &Chosen(choice)) =
        casts.get(cast)?;
    let ((calling, training, traits, usable), (skills, levels), points) = casters.get(caster)?;
    let trained = (calling, training);
    let pace = pace(round.as_deref().copied(), *difficulty);
    let caster_level = levels.caster_level(spell);
    let boost = power::boost(skills[Skill::PowerCast], usable.0.contains(Skill::PowerCast));
    let affordable = affordable(points, spell);
    let power = match asked {
        power::AUTO => {
            let identify = match (spell.facts.working, choice.aim) {
                (Some(Working::Identify), Aim::Item(target)) => {
                    held.obscurity(target).map(|difficulty| (difficulty, boost))
                }
                _ => None,
            };
            let victim = choice.aim.body().and_then(|aimed| victims.get(aimed).ok());
            let row = crate::game::hits::definitions::row(spell);
            let failure = std::array::from_fn(|power| {
                roll::learned(trained, skills, caster_level, spell, power as u32 + 1, pace, false)
            });
            power::auto(
                spell,
                (row.and_then(|row| row.restores), row.map_or(&[], |row| &row.outlasts)),
                victim,
                affordable,
                failure,
                identify,
            )
        }
        asked => power::chosen(asked, affordable, spell.power),
    };
    let on_oneself = choice.aim.body() == Some(caster);
    commands.entity(cast).insert((
        Power(power),
        power::strength(caster_level, power, boost, spell),
        Failure {
            chance: roll::learned(trained, skills, caster_level, spell, power, pace, on_oneself),
            quiet: false,
        },
        sees_blind(traits, calling),
    ));
    commands.trigger(Spread { cast });
    Ok(())
}

/// An item's power is its own, but an instrument or gadget is played, as
/// high as its chance to fail (without the round's pace, and as if
/// identified) allows; one played too badly to sound tires and trains its
/// player all the same, and that is the end of it. Its strength grows from
/// its user's level, or their Bard or Gadgeteer levels for an instrument or
/// a gadget; it fails by the skill it is used with.
#[allow(clippy::type_complexity)]
fn power_item(
    aimed: On<Aimed>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, &FromItem)>,
    casters: Query<(&Calling, &Traits, &Skills)>,
    tiring: Query<(&Conditions, &Burden, &Modifiers)>,
    held: Items,
    round: Option<Single<&Round, With<Battle>>>,
    difficulty: Res<Difficulty>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let cast = aimed.cast;
    let (&CastBy(caster), CastSpell(spell), &FromItem(item)) = casts.get(cast)?;
    let (calling, traits, skills) = casters.get(caster)?;
    let casts = held.casts(item).ok_or("an allowed item casts")?;
    let identified = held.known(item).is_some_and(|known| known.identified);
    let pace = pace(round.as_deref().copied(), *difficulty);
    let power = match casts.by {
        UseType::Instrument | UseType::Gadget => {
            let unpaced = Pace { difficulty: Difficulty::Normal, moment: 0 };
            let chances = std::array::from_fn(|power| {
                roll::item(calling, skills, (casts, true), spell, power as u32 + 1, unpaced).0
            });
            power::played(&mut rng, chances)
        }
        _ => casts.power,
    };
    if power == 0 {
        let stamina = member_toll(&mut rng, tiring.get(caster)?, Some(casts.by), spell, 0);
        commands.trigger(Pays { caster, points: None, stamina });
        if let Some(skill) = casts.by.skill() {
            commands.trigger(Practise { member: caster, skill, usage: 1 });
        }
        refuse_cast(&mut commands, (cast, caster), Refusal::PlayedBadly);
        return Ok(());
    }
    let figure = match casts.by {
        UseType::Instrument => calling.levels[Profession::Bard],
        UseType::Gadget => calling.levels[Profession::Gadgeteer],
        _ => calling.level(),
    };
    let (chance, quiet) = roll::item(calling, skills, (casts, identified), spell, power, pace);
    commands.entity(cast).insert((
        Power(power),
        power::strength(i32::from(figure), power, 0, spell),
        Failure { chance, quiet },
        sees_blind(traits, calling),
    ));
    commands.trigger(Spread { cast });
    Ok(())
}

/// A monster's own spell goes off at the power it chose, as strong as its
/// level, failing by its budget alone.
fn power_innate(
    aimed: On<Aimed>,
    mut commands: Commands,
    casts: Query<(&CastSpell, &Innate)>,
) -> Result {
    let cast = aimed.cast;
    let (CastSpell(spell), &Innate { power, budget, level }) = casts.get(cast)?;
    commands.entity(cast).insert((
        Power(power),
        power::strength(i32::from(level), power, 0, spell),
        Failure { chance: roll::spell_failure(budget, spell, power), quiet: false },
        SeesBlind(0),
    ));
    commands.trigger(Spread { cast });
    Ok(())
}

/// What a member's cast tires them by: the toll scaled by what they carry,
/// a quarter less slowed, otherwise a quarter more hasted.
fn member_toll(
    rng: &mut GameRng,
    (conditions, burden, modifiers): (&Conditions, &Burden, &Modifiers),
    used: Option<UseType>,
    spell: &SpellDefinition,
    power: u32,
) -> u32 {
    let toll = cost::stamina(rng, used, spell, power, conditions.has(Condition::Diseased));
    burden.load.scale_fatigue(toll, conditions.has(Condition::Slowed), modifiers.laid.stride > 0.0)
}

/// A learned spell pays its points, whatever the roll, and tires its caster.
fn pay_learned(
    pay: On<Pay>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, &Power)>,
    casters: Query<(&Conditions, &Burden, &Modifiers)>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let cast = pay.cast;
    let (&CastBy(caster), CastSpell(spell), &Power(power)) = casts.get(cast)?;
    let stamina = member_toll(&mut rng, casters.get(caster)?, None, spell, power);
    let realm = spell.realm.ok_or("a cast spell has a realm")?;
    commands.trigger(Pays { caster, points: Some((realm, spell.cost * power)), stamina });
    commands.trigger(Release { cast });
    Ok(())
}

/// An item pays one use, whatever the roll, and tires its user; it is let go
/// before the use is spent, so that the last use is told by name.
fn pay_item(
    pay: On<Pay>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, &Power, &FromItem)>,
    casters: Query<(&Conditions, &Burden, &Modifiers)>,
    held: Items,
    mut rng: ResMut<GameRng>,
) -> Result {
    let cast = pay.cast;
    let (&CastBy(caster), CastSpell(spell), &Power(power), &FromItem(item)) = casts.get(cast)?;
    let used = held.casts(item).map(|casts| casts.by);
    let stamina = member_toll(&mut rng, casters.get(caster)?, used, spell, power);
    commands.trigger(Pays { caster, points: None, stamina });
    commands.trigger(Release { cast });
    commands.trigger(Spend { item, amount: 1 });
    Ok(())
}

/// A monster's own spell costs it no points, but tires it by its level and
/// power, unscaled.
fn pay_innate(
    pay: On<Pay>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, &Power)>,
    casters: Query<&Conditions>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let cast = pay.cast;
    let (&CastBy(caster), CastSpell(spell), &Power(power)) = casts.get(cast)?;
    let diseased = casters.get(caster)?.has(Condition::Diseased);
    let stamina = cost::stamina(&mut rng, None, spell, power, diseased);
    commands.trigger(Pays { caster, points: None, stamina });
    commands.trigger(Release { cast });
    Ok(())
}

/// A learned spell's caster practises its book, realm and Power Cast once
/// its deed has landed, if it affected anyone or a missile flew.
fn practise_learned(
    released: On<Released>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, &Power)>,
    casters: Query<(&Calling, &Training, &UsableSkills)>,
) -> Result {
    let (&CastBy(caster), CastSpell(spell), &Power(power)) = casts.get(released.cast)?;
    let (calling, training, usable) = casters.get(caster)?;
    let trained = (calling, training, usable.0.contains(Skill::PowerCast));
    let skills = cost::practice(trained, None, spell, power);
    commands
        .entity(released.deed)
        .insert(Practice { caster, skills, sure: released.flies })
        .observe(crate::game::magic::landed::practise);
    Ok(())
}

/// An item's user practises the skill it is used with once its deed has
/// landed, if it affected anyone or a missile flew.
fn practise_item(
    released: On<Released>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, &Power, &FromItem)>,
    casters: Query<(&Calling, &Training, &UsableSkills)>,
    held: Items,
) -> Result {
    let (&CastBy(caster), CastSpell(spell), &Power(power), &FromItem(item)) =
        casts.get(released.cast)?;
    let used = held.casts(item).map(|casts| casts.by);
    let (calling, training, usable) = casters.get(caster)?;
    let trained = (calling, training, usable.0.contains(Skill::PowerCast));
    let skills = cost::practice(trained, used, spell, power);
    commands
        .entity(released.deed)
        .insert(Practice { caster, skills, sure: released.flies })
        .observe(crate::game::magic::landed::practise);
    Ok(())
}

/// A member's utility spell counts as an act for practice, whatever came of
/// it.
fn trained_by_work(
    worked: On<Worked>,
    mut commands: Commands,
    casts: Query<(&CastSpell, &Power, &Chosen)>,
    casters: Query<(&Calling, &Training, &UsableSkills)>,
    held: Items,
) -> Result {
    let (CastSpell(spell), &Power(power), &Chosen(choice)) = casts.get(worked.cast)?;
    let used = match choice.source {
        Source::Item(item) => held.casts(item).map(|casts| casts.by),
        _ => None,
    };
    let caster = worked.caster;
    let (calling, training, usable) = casters.get(caster)?;
    let trained = (calling, training, usable.0.contains(Skill::PowerCast));
    for (skill, usage) in cost::practice(trained, used, spell, power) {
        commands.trigger(Practise { member: caster, skill, usage });
    }
    Ok(())
}

/// A spell left to its caster at the "repeat until whole" choice is cast
/// again once its deed has landed, while its victims are not whole and
/// stamina and points last.
fn recast(
    released: On<Released>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, &LearnedSpell, &Chosen)>,
    deeds: Query<&crate::game::magic::landed::Spell>,
) -> Result {
    let (&CastBy(caster), CastSpell(spell), &LearnedSpell { asked }, &Chosen(choice)) =
        casts.get(released.cast)?;
    if asked != power::AUTO || spell.power != PowerChoice::ChosenOrRepeat {
        return Ok(());
    }
    let Some(row) = crate::game::hits::definitions::row(spell) else { return Ok(()) };
    let victims = deeds.get(released.deed)?.victims.clone();
    commands
        .entity(released.deed)
        .insert(Recast { caster, choice, victims, watches_stamina: row.watches_stamina })
        .observe(crate::game::magic::landed::recast);
    Ok(())
}

/// An auto cast that fizzled goes again at once, provoking nobody anew,
/// while stamina and points last.
fn again(
    spent: On<Spent>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, &LearnedSpell, &Chosen)>,
) -> Result {
    let (&CastBy(caster), CastSpell(spell), &LearnedSpell { asked }, &Chosen(choice)) =
        casts.get(spent.cast)?;
    if asked == power::AUTO {
        commands.trigger(super::Cast { caster, spell: spell.id, choice, again: true });
    }
    Ok(())
}

/// Whether the game resolves a spell it is asked about: it has a row, or is a
/// utility spell (what menus offer).
pub fn resolvable(spell: &SpellDefinition) -> bool {
    crate::game::hits::definitions::row(spell).is_some() || spell.facts.working.is_some()
}

/// The spells and items an actor could use: what menus offer. Execution
/// validates again.
pub fn usable_items(
    member: Entity,
    (user, attributes, skills): (
        (&Calling, &Birth),
        &crate::game::actors::Attributes,
        &crate::game::actors::Skills,
    ),
    held: &Items,
    content: &Catalogs,
) -> Vec<(Entity, SpellId)> {
    held.carried(member)
        .filter_map(|item| {
            let worn = held.slot(item).is_some_and(|slot| slot.is_active());
            Some((item, usable_item((user, attributes, skills), item, held, content, worn)?))
        })
        .collect()
}

/// The spell an item casts for `character`, if they can use it now: one
/// they may activate (worn, if it must be), with a charge left, whose
/// effect the game resolves.
pub fn usable_item(
    (user, attributes, skills): (
        (&Calling, &Birth),
        &crate::game::actors::Attributes,
        &crate::game::actors::Skills,
    ),
    item: Entity,
    held: &Items,
    content: &Catalogs,
    worn: bool,
) -> Option<SpellId> {
    let cast = held.casts(item)?;
    (may::activates(cast, worn, user, attributes, skills)
        && held.charges(item) != Some(0)
        && resolvable(content.spells.get(cast.spell)?))
    .then_some(cast.spell)
}
