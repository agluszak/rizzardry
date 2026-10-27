//! Casting a learned spell or using an item's spell: one operation that
//! revalidates the source and target when it runs, captures the victims,
//! rolls the cast, pays for it and resolves its effect together (retail
//! `ExecuteCharacterSpellCast`, `UseItem`, `CastItemSpell`,
//! `CastSpellFromSource`).
//!
//! Choosing never spends anything. A refused attempt spends nothing; an
//! accepted one pays even when it fizzles or backfires: spell points for a
//! learned spell, one use for an item. The effect keeps its own copy of the
//! spell and victims, so using the last potion still heals.

use bevy::prelude::*;

use super::{Effect, LearnedSpells, Outcome, SpellPoints, Strength, Victim, World};
use crate::content::items::{EquipClass, QuantityKind};
use crate::content::spells::{SpellCatalog, SpellDefinition, SpellId, Targeting, Usable};
use crate::game::characters::{Character, Condition};
use crate::game::inventory::{Carried, ItemInstanceId, Slot, spend_use};
use crate::game::rng::GameRng;

/// What pays for a cast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// A learned spell at the chosen power; retail clamps it to what the
    /// caster can afford when it runs.
    Spell { spell: SpellId, power: u8 },
    /// An item the actor carries, identified by instance, not reserved.
    Item(ItemInstanceId),
}

/// Whom the actor aimed at; group, party and self spells widen or replace it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aim {
    Member(Entity),
    Monster(Entity),
}

/// An accepted magic action: the source and the aim, nothing resolved yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MagicChoice {
    pub source: Source,
    pub aim: Aim,
}

/// Why nothing was spent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    CannotAct,
    Silenced,
    NotLearned,
    NoSpellPoints,
    NoItem,
    NoCharges,
    CannotUse,
    /// Its effect is not implemented yet.
    Unsupported,
    /// Not now: only in combat, or only out of it.
    WrongTime,
    NoTarget,
    MissingContent,
}

/// What became of a magic action, for whoever tells the player.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum MagicNews {
    Done(Report),
    Refused { caster: Entity, refusal: Refusal },
}

/// What a cast did.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub caster: Entity,
    pub spell: SpellId,
    pub power: u8,
    /// The item it came from, already spent.
    pub item: Option<String>,
    pub outcome: Outcome,
    pub effects: Vec<Effect>,
}

/// Where the actor is and what it can reach, as the caller sees it.
pub struct Situation<'a> {
    pub in_combat: bool,
    /// The party's living members, in roster order.
    pub party: &'a [Entity],
    /// Monsters the actor may aim at: alive, in the fight if there is one,
    /// with each one's group mates.
    pub reachable: &'a dyn Fn(Entity, &SpellDefinition) -> bool,
    pub group_of: &'a dyn Fn(Entity) -> Vec<Entity>,
}

fn find_item(carried: &Carried, id: ItemInstanceId) -> Option<Slot> {
    let worn = carried
        .equipment
        .iter()
        .find(|(_, item)| item.as_ref().is_some_and(|i| i.id == id))
        .map(|(slot, _)| Slot::Equipment(slot));
    worn.or_else(|| {
        carried
            .backpack
            .iter()
            .position(|item| item.as_ref().is_some_and(|i| i.id == id))
            .map(Slot::Backpack)
    })
}

fn slot_mut(
    carried: &mut Carried,
    slot: Slot,
) -> &mut Option<crate::game::inventory::ItemInstance> {
    match slot {
        Slot::Equipment(slot) => &mut carried.equipment[slot],
        Slot::Backpack(index) => &mut carried.backpack[index],
    }
}

/// The victims a spell takes when it is cast, from its aim (retail
/// `PopulateSpellTargetMarkers`). Later movement adds nobody.
fn capture(
    caster: Entity,
    spell: &SpellDefinition,
    aim: Aim,
    situation: &Situation,
    forced_self: bool,
) -> Result<Vec<Victim>, Refusal> {
    if forced_self {
        return Ok(vec![Victim::Member(caster)]);
    }
    let monster = |aim| match aim {
        Aim::Monster(m) if (situation.reachable)(m, spell) => Ok(m),
        _ => Err(Refusal::NoTarget),
    };
    Ok(match spell.targeting {
        Targeting::Caster => vec![Victim::Member(caster)],
        Targeting::Ally => match aim {
            Aim::Member(member) if situation.party.contains(&member) => {
                vec![Victim::Member(member)]
            }
            _ => return Err(Refusal::NoTarget),
        },
        Targeting::Party => situation.party.iter().map(|&m| Victim::Member(m)).collect(),
        Targeting::Enemy => vec![Victim::Monster(monster(aim)?)],
        Targeting::EnemyGroup => {
            let first = monster(aim)?;
            let mut victims = vec![Victim::Monster(first)];
            for mate in (situation.group_of)(first) {
                if mate != first && (situation.reachable)(mate, spell) {
                    victims.push(Victim::Monster(mate));
                }
            }
            victims
        }
        _ => return Err(Refusal::Unsupported),
    })
}

/// Cast or use as `caster`, now: revalidate, capture victims, roll, pay and
/// resolve. Nothing is spent on refusal.
pub fn perform(
    caster: Entity,
    choice: MagicChoice,
    situation: &Situation,
    spells: &SpellCatalog,
    points: &mut Query<(&LearnedSpells, &mut SpellPoints)>,
    rng: &mut GameRng,
    world: &mut World,
) -> Result<Report, Refusal> {
    let (character, vitals, carried) = world.members.get(caster).map_err(|_| Refusal::CannotAct)?;
    if vitals.is_dead() || vitals.worst_condition().is_some_and(|c| c >= Condition::Asleep) {
        return Err(Refusal::CannotAct);
    }
    // What the source is, before anything is spent.
    let (spell, power, item, forced_self) = match choice.source {
        Source::Spell { spell, power } => {
            if vitals.has(Condition::Silenced) {
                return Err(Refusal::Silenced);
            }
            let (learned, pool) = points.get(caster).map_err(|_| Refusal::NotLearned)?;
            if !learned.0.contains(&spell) {
                return Err(Refusal::NotLearned);
            }
            let definition = spells.get(spell).ok_or(Refusal::MissingContent)?;
            let power = power.min(pool.affordable_power(definition));
            if power == 0 {
                return Err(Refusal::NoSpellPoints);
            }
            (definition, power, None, false)
        }
        Source::Item(id) => {
            let slot = find_item(carried, id).ok_or(Refusal::NoItem)?;
            let instance = carried.slot(slot).and_then(Option::as_ref).ok_or(Refusal::NoItem)?;
            let definition = world.items.get(instance.item).ok_or(Refusal::MissingContent)?;
            let cast = definition.spell.ok_or(Refusal::CannotUse)?;
            if !character.can_use(definition) && !consumed_by_anyone(definition.class) {
                return Err(Refusal::CannotUse);
            }
            if matches!(instance.quantity, crate::game::inventory::Quantity::Uses(0)) {
                return Err(Refusal::NoCharges);
            }
            let spell = spells.get(cast.spell).ok_or(Refusal::MissingContent)?;
            // Drinking in combat is always done by the drinker (retail
            // `UseItem` aims potions at their user).
            let forced = situation.in_combat && consumed_by_anyone(definition.class);
            (spell, cast.power, Some((slot, definition.name.clone(), definition.quantity)), forced)
        }
    };
    if !super::modelled(spell.id) {
        return Err(Refusal::Unsupported);
    }
    match spell.usable {
        Usable::InCombat if !situation.in_combat => return Err(Refusal::WrongTime),
        Usable::OutOfCombat | Usable::Camped if situation.in_combat => {
            return Err(Refusal::WrongTime);
        }
        _ => {}
    }
    let victims = capture(caster, spell, choice.aim, situation, forced_self)?;
    // Accepted: roll, then pay whatever the roll.
    let chance = if item.is_some() { 0 } else { super::fail_chance(character, spell, power) };
    let strength = Strength::of(character, spell, power);
    let outcome = super::roll(rng, chance, spell);
    let fatigue = match item {
        None => super::cast_fatigue(spell, power),
        Some(_) => {
            let base = rng.below(2) + 1;
            if vitals.has(Condition::Diseased) { base * 2 } else { base }
        }
    };
    let item_name = match item {
        None => {
            let (_, mut pool) = points.get_mut(caster).map_err(|_| Refusal::NotLearned)?;
            let realm = spell.realm.ok_or(Refusal::MissingContent)?;
            pool.current[realm] = pool.current[realm].saturating_sub(spell.cost_at(power));
            None
        }
        Some((slot, name, quantity)) => {
            let (_, _, mut carried) = world.members.get_mut(caster).map_err(|_| Refusal::NoItem)?;
            let held = slot_mut(&mut carried, slot);
            // An emptied charged item stays, unusable; a drunk potion or a
            // read scroll is gone.
            if let Some(instance) = held.as_mut()
                && spend_use(instance, quantity == QuantityKind::Charges)
            {
                *held = None;
            }
            Some(name)
        }
    };
    {
        let (_, mut vitals, _) = world.members.get_mut(caster).map_err(|_| Refusal::CannotAct)?;
        vitals.spend_stamina(fatigue);
    }
    let effects = match outcome {
        Outcome::Cast => super::resolve(spell, strength, &victims, rng, world),
        // Retail re-aims a backfire; here it turns on the caster.
        Outcome::Backfired => {
            super::resolve(spell, strength, &[Victim::Member(caster)], rng, world)
        }
        Outcome::Fizzled => Vec::new(),
    };
    Ok(Report { caster, spell: spell.id, power, item: item_name, outcome, effects })
}

/// Potions, drinks and food: usable by anyone who holds them.
fn consumed_by_anyone(class: EquipClass) -> bool {
    matches!(class, EquipClass::Potion | EquipClass::Drink | EquipClass::Food)
}

/// The spells and items an actor could use: what menus offer. Execution
/// validates again.
pub fn usable_items(
    character: &Character,
    carried: &Carried,
    items: &crate::content::items::ItemCatalog,
) -> Vec<(ItemInstanceId, SpellId)> {
    carried
        .equipment
        .values()
        .chain(carried.backpack.iter())
        .flatten()
        .filter_map(|instance| {
            let definition = items.get(instance.item)?;
            let cast = definition.spell?;
            let usable = character.can_use(definition) || consumed_by_anyone(definition.class);
            let charged = !matches!(instance.quantity, crate::game::inventory::Quantity::Uses(0));
            (usable && charged && super::modelled(cast.spell)).then_some((instance.id, cast.spell))
        })
        .collect()
}
