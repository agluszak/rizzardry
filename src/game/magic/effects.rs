//! What spells do to their victims (retail `ProcessSpellEffectTargets`,
//! `ResolveSpellMissileHit` and the effect functions of `Magic Effects.cpp`),
//! table-driven by spell. Each spell resolves in retail's passes: every
//! victim's damage, then every victim's conditions.

use bevy::prelude::*;

use super::{LearnedSpells, SpellPoints, Strength};
use crate::content::characters::{Attribute, Realm, Skill};
use crate::content::items::{Dice, EquipClass, ItemCatalog};
use crate::content::species::MonsterCatalog;
use crate::content::spells::{SpellCatalog, SpellDefinition, SpellId};
use crate::game::characters::{
    Character, Condition, Lasting, Vitals, inflict, resistance::resistances, wake_on_hit,
};
use crate::game::difficulty::{Difficulty, Side};
use crate::game::inventory::Carried;
use crate::game::party::DamageApplied;
use crate::game::party::formation::Formation;
use crate::game::rng::GameRng;
use crate::game::world::damage::{DamageOutcome, MonsterDamage};

/// What a spell does, by retail's effect functions.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    /// `ApplyDamageToTargets` or a missile hit: the magnitude less resistance.
    Damage,
    /// Damage to undead (and Banish's other kinds) only; others are untouched.
    DamageUndead {
        banish: bool,
    },
    Heal,
    Stamina,
    /// Healing followed by stamina with twice the dice count.
    RestoreHealth,
    RestoreMagic,
    GoldLoss,
    DestroyConsumables,
    DrainMagic,
    /// Heal, restore stamina and lift conditions 2..17 but infatuation.
    Restoration,
    /// `InflictConditionAttack` at a percent chance.
    Inflict(Condition, u32),
    /// Damage, then a condition attack on every victim.
    DamageThen(Condition, u32),
    /// Damage, then nausea at 90%, then unconsciousness at 25%.
    NoxiousFumes,
    /// Fear; a party member it lands on is attacked again for unconsciousness
    /// (Terror), a monster for more fear (Sonic Boom), at half strength.
    Fear {
        second: Condition,
    },
    /// Insanity; one who saves is afraid anyway.
    Pandemonium,
    /// Death on a failed save, unless resistance cuts the attempt to nothing.
    Slay,
    /// Half the time a weighted condition, otherwise damage.
    Affliction,
    Cure(&'static [Condition]),
    Resurrect,
    /// Drain health; 70% of it heals the most wounded party member.
    Lifesteal,
}

/// The spells implemented, by retail id (`spell_ids.h`), and what they do.
fn kind(spell: SpellId) -> Option<Kind> {
    use Condition::*;
    Some(match spell.0 {
        1 | 4 | 5 | 9 | 25 | 28 | 36 | 43 | 47 | 50 | 52 | 55 | 57 | 63 | 66 | 83 | 95 | 96
        | 97 | 98 | 101 => Kind::Damage,
        22 => Kind::DamageUndead { banish: false },
        77 => Kind::DamageUndead { banish: true },
        6 | 68 => Kind::Heal,
        13 | 44 => Kind::Stamina,
        100 => Kind::Restoration,
        7 => Kind::Inflict(Irritated, 100),
        11 | 37 | 67 => Kind::Inflict(Paralyzed, 100),
        12 => Kind::Inflict(Asleep, 100),
        15 => Kind::Inflict(Blind, 100),
        24 => Kind::Inflict(Insane, 100),
        29 => Kind::Inflict(Slowed, 100),
        31 => Kind::Inflict(Webbed, 100),
        46 => Kind::Inflict(Silenced, 100),
        69 => Kind::Inflict(Hexed, 100),
        10 | 71 => Kind::DamageThen(Insane, 25),
        78 => Kind::DamageThen(Blind, 25),
        91 => Kind::DamageThen(Insane, 75),
        92 => Kind::DamageThen(Unconscious, 50),
        42 => Kind::NoxiousFumes,
        14 => Kind::Fear { second: Unconscious },
        30 => Kind::Fear { second: Afraid },
        85 => Kind::Pandemonium,
        70 | 87 | 90 | 94 => Kind::Slay,
        86 | 99 => Kind::Affliction,
        16 => Kind::Cure(&[Irritated, Nauseated, Afraid, Asleep, Blind]),
        34 => Kind::Cure(&[Paralyzed, Webbed]),
        35 => Kind::Cure(&[Poisoned]),
        51 => Kind::Cure(&[Diseased]),
        58 => Kind::Cure(&[Hexed]),
        74 => Kind::Cure(&[Insane, Turncoat]),
        88 => Kind::Resurrect,
        82 => Kind::Lifesteal,
        124 => Kind::GoldLoss,
        125 => Kind::Damage,
        126 => Kind::DestroyConsumables,
        127 => Kind::DrainMagic,
        114 => Kind::RestoreHealth,
        115 => Kind::RestoreMagic,
        116 => Kind::Cure(&[Unconscious, Asleep]),
        _ => return None,
    })
}

/// Whether a spell's effect is implemented; others are refused before
/// anything is spent.
pub fn modelled(spell: SpellId) -> bool {
    kind(spell).is_some()
}

/// Whether a spell restores what is missing, so that "auto" power and
/// repeating aim at what the victim lacks.
pub(super) fn restores(spell: SpellId) -> bool {
    matches!(kind(spell), Some(Kind::Heal | Kind::Stamina | Kind::Restoration))
}

/// Whom a spell lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Victim {
    Member(Entity),
    Monster(Entity),
}

/// What a spell did to one victim.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    Healed { victim: Victim, amount: u32 },
    Refreshed { victim: Victim, amount: u32 },
    Replenished { victim: Victim, amount: u32 },
    Damaged { victim: Victim, amount: u32, outcome: DamageOutcome },
    Inflicted { victim: Victim, condition: Condition },
    Cured { victim: Victim, condition: Condition },
    Raised { victim: Victim },
    GoldLost { amount: u32 },
    Destroyed { class: EquipClass, amount: u32 },
    Drained { victim: Victim, amount: u32 },
    Resisted { victim: Victim },
    Nothing { victim: Victim },
}

/// The party members' profile, vitals and belongings, as spells and combat
/// change them.
pub type Members<'w, 's> =
    Query<'w, 's, (&'static mut Character, &'static mut Vitals, &'static mut Carried)>;

/// Consume physical units, not charges, in place; remaining identity and
/// equipment binding metadata belong to the existing owner.
fn burn_stack(
    stack: crate::game::inventory::ItemStack,
    items: &ItemCatalog,
    chance: u32,
    rng: &mut GameRng,
    totals: &mut [u32; 3],
) -> Option<crate::game::inventory::ItemStack> {
    let Some(definition) = items.get(stack.item) else { return Some(stack) };
    let category = match definition.class {
        EquipClass::Potion => 0,
        EquipClass::Scroll => 1,
        EquipClass::Spellbook => 2,
        _ => return Some(stack),
    };
    let lost = (0..stack.count()).filter(|_| rng.below(100) < chance).count() as u16;
    totals[category] += u32::from(lost);
    stack.remove_units(lost)
}

/// What a spell's effects need from the world.
pub struct Field<'a, 'm, 'ms, 'd, 'ds, 'f, 'p> {
    pub members: &'a mut Members<'m, 'ms>,
    pub monsters: &'a mut MonsterDamage<'d, 'ds>,
    pub formation: &'a mut Mut<'f, Formation>,
    pub party_damage: &'a mut MessageWriter<'p, DamageApplied>,
    pub items: &'a ItemCatalog,
    pub species: &'a MonsterCatalog,
    /// What monster loot tables scale by.
    pub party_level: u32,
    pub difficulty: Difficulty,
    /// The living party, for spells that pick among it.
    pub party: &'a [Entity],
    /// Skills that improved while resolving, for the notices.
    pub improved: Vec<(Entity, Skill)>,
}

/// The magnitude a spell rolls at `power` (retail `RollEffectMagnitude`): its
/// dice count times power, then raised by the boost percent.
fn magnitude(rng: &mut GameRng, dice: Option<Dice>, strength: Strength) -> u32 {
    let Some(dice) = dice else { return 0 };
    let count = (u32::from(dice.count) * u32::from(strength.power)).min(255) as u8;
    let rolled = rng.roll(Dice { count, ..dice });
    rolled + rolled * strength.boost / 100
}

/// Retail `ReduceMagnitudeByResistance`: resistance and the victim's level
/// over the cast's strength take a random share off.
pub(super) fn resist(
    rng: &mut GameRng,
    amount: u32,
    resistance: i32,
    level: u32,
    against: u32,
) -> u32 {
    let mut c = resistance + (level as i32 - against as i32) * 3;
    if c <= 0 {
        return amount;
    }
    c += 1;
    let half = c / 2;
    let share =
        (rng.below((c - half) as u32) as i32 + half + rng.below(half as u32) as i32).min(100);
    amount - amount * share as u32 / 100
}

/// Each condition's base save (retail `g_condition_resist_base`), by retail
/// number (Drained is 1, Dead 18).
const CONDITION_SAVE: [i32; 20] =
    [0, 1, 2, 3, 4, 4, 5, 6, 7, 8, 9, 10, 12, 14, 16, 20, 18, 22, 24, 22];
const DEATH_SAVE: i32 = CONDITION_SAVE[18];

/// Rounds a condition spell inflicts (retail `RollEffectDuration`).
fn duration(rng: &mut GameRng, spell: &SpellDefinition, strength: Strength) -> u32 {
    let mut rounds =
        (spell.duration_per_power * i32::from(strength.power) + spell.duration_base + 1).max(0)
            as u32;
    match rng.below(4) {
        0 if rounds > 1 => rounds += 1,
        1 if rounds <= 2 => rounds += 1,
        _ => {}
    }
    rounds + rounds * strength.boost / 100
}

/// The victim as saves and resistance see it.
struct Target {
    level: u32,
    resistance: i32,
    side: Option<Side>,
    turncoat: bool,
    immune: bool,
    dead: bool,
}

fn target(
    world: &Field,
    victim: Victim,
    realm: Realm,
    condition: Option<Condition>,
) -> Option<Target> {
    match victim {
        Victim::Monster(monster) => {
            let (body, ..) = world.monsters.monsters.get(monster).ok()?;
            let kind = world.species.0.get(body.species as usize)?;
            Some(Target {
                level: u32::from(kind.level),
                resistance: i32::from(kind.resistances[realm]),
                side: Some(Side::Hostile),
                turncoat: false,
                immune: condition.is_some_and(|c| kind.immune_to(c)),
                dead: body.is_dead(),
            })
        }
        Victim::Member(member) => {
            let (character, vitals, carried) = world.members.get(member).ok()?;
            Some(Target {
                level: u32::from(character.level),
                resistance: resistances(character, carried, world.items)[realm].total,
                side: None,
                turncoat: vitals.has(Condition::Turncoat),
                immune: false,
                dead: vitals.is_dead(),
            })
        }
    }
}

/// Whether a victim saves (retail `TargetResistsCondition`): `(level −
/// strength)·3 + base + resistance`, scaled by difficulty, within 5..95. A
/// party member who saves practises Iron Will.
fn saves(
    rng: &mut GameRng,
    world: &mut Field,
    victim: Victim,
    target: &Target,
    base: i32,
    against: u32,
) -> bool {
    let chance = (target.level as i32 - against as i32) * 3 + base + target.resistance;
    let chance = match target.side {
        Some(side) => world.difficulty.for_monster(chance, side),
        None => world.difficulty.for_character(chance, target.turncoat),
    };
    let saved = (rng.below(100) as i32) < chance.clamp(5, 95);
    if saved
        && let Victim::Member(member) = victim
        && let Ok((mut character, ..)) = world.members.get_mut(member)
        && character.practise(Skill::IronWill, 2, rng)
    {
        world.improved.push((member, Skill::IronWill));
    }
    saved
}

/// Damage one victim by an already rolled magnitude, less resistance.
fn damage(
    rng: &mut GameRng,
    world: &mut Field,
    victim: Victim,
    realm: Realm,
    rolled: u32,
    against: u32,
) -> Option<Effect> {
    let target = target(world, victim, realm, None)?;
    if target.dead {
        return None;
    }
    let amount = resist(rng, rolled, target.resistance, target.level, against);
    if amount == 0 {
        return Some(Effect::Resisted { victim });
    }
    match victim {
        Victim::Monster(monster) => {
            let outcome = world.monsters.apply(monster, amount, world.party_level, rng).ok()?;
            Some(Effect::Damaged { victim, amount: outcome.damage, outcome })
        }
        Victim::Member(member) => {
            let (character, mut vitals, _) = world.members.get_mut(member).ok()?;
            let lost = crate::game::party::damage_member(
                member,
                &mut vitals,
                world.formation,
                amount,
                world.party_damage,
            );
            if !vitals.is_dead() {
                wake_on_hit(&mut vitals.conditions, character.attribute(Attribute::Senses), rng);
            }
            let outcome = DamageOutcome { damage: lost, died: vitals.is_dead(), experience: 0 };
            Some(Effect::Damaged { victim, amount: lost, outcome })
        }
    }
}

/// Retail `InflictConditionAttack` on one victim: the chance, a fresh
/// duration, immunity, the save, then the duration cut by resistance.
#[allow(clippy::too_many_arguments)]
fn attack(
    rng: &mut GameRng,
    world: &mut Field,
    spell: &SpellDefinition,
    victim: Victim,
    realm: Realm,
    condition: Condition,
    chance: u32,
    strength: Strength,
) -> Option<Effect> {
    if rng.below(100) >= chance {
        return None;
    }
    let target = target(world, victim, realm, Some(condition))?;
    if target.dead {
        return None;
    }
    let rounds = duration(rng, spell, strength);
    let base = CONDITION_SAVE[condition as usize + 1];
    if target.immune || saves(rng, world, victim, &target, base, strength.against) {
        return Some(Effect::Resisted { victim });
    }
    let rounds = match resist(rng, rounds, target.resistance, target.level, strength.against) {
        0 => return Some(Effect::Resisted { victim }),
        1 => 2,
        rounds => rounds,
    };
    let lasting = Lasting::Rounds(rounds.min(u32::from(u16::MAX)) as u16);
    match victim {
        Victim::Monster(monster) => {
            let (mut body, ..) = world.monsters.monsters.get_mut(monster).ok()?;
            inflict(&mut body.conditions, condition, lasting);
        }
        Victim::Member(member) => {
            let (_, mut vitals, _) = world.members.get_mut(member).ok()?;
            vitals.inflict(condition, lasting);
        }
    }
    Some(Effect::Inflicted { victim, condition })
}

/// Death spells: a failed save kills, unless resistance cuts the attempt's
/// duration to nothing (retail `InflictConditionOnTarget` for Dead).
fn slay(
    rng: &mut GameRng,
    world: &mut Field,
    spell: &SpellDefinition,
    victim: Victim,
    realm: Realm,
    strength: Strength,
) -> Option<Effect> {
    let target = target(world, victim, realm, None)?;
    if target.dead {
        return None;
    }
    let rounds = duration(rng, spell, strength);
    if saves(rng, world, victim, &target, DEATH_SAVE, strength.against)
        || resist(rng, rounds, target.resistance, target.level, strength.against) == 0
    {
        return Some(Effect::Resisted { victim });
    }
    let hp = match victim {
        Victim::Monster(monster) => world.monsters.monsters.get(monster).ok()?.0.hp,
        Victim::Member(member) => world.members.get(member).ok()?.1.hp,
    };
    match victim {
        Victim::Monster(monster) => {
            let outcome = world.monsters.apply(monster, hp, world.party_level, rng).ok()?;
            Some(Effect::Damaged { victim, amount: outcome.damage, outcome })
        }
        Victim::Member(member) => {
            let (_, mut vitals, _) = world.members.get_mut(member).ok()?;
            let lost = crate::game::party::damage_member(
                member,
                &mut vitals,
                world.formation,
                hp,
                world.party_damage,
            );
            let outcome = DamageOutcome { damage: lost, died: true, experience: 0 };
            Some(Effect::Damaged { victim, amount: lost, outcome })
        }
    }
}

/// Retail `TryCureConditionOnTargets`: a timed condition loses `power²`
/// rounds (and goes if that is all of them); a lasting one goes with chance
/// `power·15`%.
fn cure(
    rng: &mut GameRng,
    conditions: &mut Vec<(Condition, Lasting)>,
    cured: &[Condition],
    strength: Strength,
    forced: bool,
) -> Vec<Condition> {
    let per = u32::from(strength.power).pow(2);
    let per = per + per * strength.boost / 100;
    let mut lifted = Vec::new();
    for &condition in cured {
        let Some(index) = conditions.iter().position(|&(c, _)| c == condition) else { continue };
        let gone = match &mut conditions[index].1 {
            _ if forced => true,
            Lasting::Rounds(left) if u32::from(*left) <= per => true,
            Lasting::Rounds(left) => {
                *left -= per as u16;
                false
            }
            Lasting::Indefinite => rng.below(100) < u32::from(strength.power) * 15,
        };
        if gone {
            conditions.remove(index);
            lifted.push(condition);
        }
    }
    lifted
}

/// Prismic spells' conditions and their weights (retail `ME:251-253`).
const AFFLICTIONS: [(Option<Condition>, u32); 9] = [
    (Some(Condition::Nauseated), 10),
    (Some(Condition::Afraid), 10),
    (Some(Condition::Hexed), 5),
    (Some(Condition::Insane), 5),
    (Some(Condition::Blind), 5),
    (Some(Condition::Turncoat), 1),
    (Some(Condition::Asleep), 10),
    (Some(Condition::Paralyzed), 3),
    (None, 1),
];

/// Resolve a spell on the victims captured when it was cast, in their
/// captured order. Victims that died or left since are skipped; nobody new
/// is added.
pub fn resolve(
    spell: &SpellDefinition,
    strength: Strength,
    victims: &[Victim],
    rng: &mut GameRng,
    world: &mut Field,
    (spells, points): (&SpellCatalog, &mut Query<(&LearnedSpells, &mut SpellPoints)>),
) -> Vec<Effect> {
    let (Some(kind), Some(realm)) = (kind(spell.id), spell.realm) else { return Vec::new() };
    let mut effects = Vec::new();
    let mut each = |f: &mut dyn FnMut(Victim, &mut GameRng, &mut Field) -> Option<Effect>| {
        for &victim in victims {
            if let Some(effect) = f(victim, rng, world) {
                effects.push(effect);
            }
        }
    };
    match kind {
        Kind::GoldLoss => {
            if victims.iter().any(|v| matches!(v, Victim::Member(_))) {
                // Retail's second roll when the first is below 100 is intentional.
                let first = magnitude(rng, spell.dice, strength);
                let percent = if first < 100 { magnitude(rng, spell.dice, strength) } else { 100 };
                let inventory = &mut world.monsters.inventory;
                // Malformed/modded percentages cannot wrap the unsigned purse.
                let lost = (u64::from(inventory.gold) * u64::from(percent.min(100)) / 100) as u32;
                inventory.gold -= lost;
                effects.push(Effect::GoldLost { amount: lost });
            }
        }
        Kind::DestroyConsumables => {
            let mut totals = [0; 3];
            let chance = u32::from(strength.power) * 10;
            for &victim in victims {
                let Victim::Member(member) = victim else { continue };
                let Ok((_, _, mut carried)) = world.members.get_mut(member) else { continue };
                // Retail visits all twelve equipped slots, including alternate hands.
                for (_, slot) in &mut carried.equipment {
                    if let Some(equipped) = slot.take() {
                        *slot = burn_stack(equipped.stack, world.items, chance, rng, &mut totals)
                            .map(|stack| crate::game::inventory::Equipped {
                                stack,
                                curse_released: equipped.curse_released,
                            });
                    }
                }
                for slot in &mut carried.backpack {
                    if let Some(stack) = slot.take() {
                        *slot = burn_stack(stack, world.items, chance, rng, &mut totals);
                    }
                }
            }
            if victims.iter().any(|v| matches!(v, Victim::Member(_))) {
                let pool = &mut world.monsters.inventory.pool.stacks;
                *pool = std::mem::take(pool)
                    .into_iter()
                    .filter_map(|stack| burn_stack(stack, world.items, chance, rng, &mut totals))
                    .collect();
            }
            for (class, amount) in [EquipClass::Potion, EquipClass::Scroll, EquipClass::Spellbook]
                .into_iter()
                .zip(totals)
            {
                if amount != 0 {
                    effects.push(Effect::Destroyed { class, amount });
                }
            }
        }
        Kind::DrainMagic => {
            if victims.iter().any(|v| matches!(v, Victim::Member(_))) {
                let base = u32::from(strength.power) * 50;
                let amount = base + base * strength.boost / 100;
                // Anti-Magic drains the whole living party, not only captured victims.
                for &member in world.party {
                    if world.members.get(member).is_ok_and(|(_, v, _)| !v.is_dead())
                        && let Ok((_, mut pool)) = points.get_mut(member)
                    {
                        let amount = pool.drain(amount, rng);
                        effects.push(Effect::Drained { victim: Victim::Member(member), amount });
                    }
                }
                // Party enchantment slots are not modelled; no active buffs can exist yet.
            }
        }
        Kind::RestoreMagic => each(&mut |v, rng, world| {
            let Victim::Member(member) = v else { return None };
            let (character, vitals, _) = world.members.get(member).ok()?;
            if vitals.is_dead() {
                return None;
            }
            // Retail rolls even when every realm is already full.
            let amount = magnitude(rng, spell.dice, strength);
            let Ok((learned, mut pool)) = points.get_mut(member) else {
                return Some(Effect::Nothing { victim: v });
            };
            let maximum = super::max_points(character, learned, spells);
            let amount = pool.restore_evenly(&maximum, amount);
            Some(if amount == 0 {
                Effect::Nothing { victim: v }
            } else {
                Effect::Replenished { victim: v, amount }
            })
        }),
        Kind::RestoreHealth => {
            // Separate passes preserve retail target and RNG order.
            each(&mut |v, rng, world| {
                let Victim::Member(member) = v else { return None };
                let (character, mut vitals, _) = world.members.get_mut(member).ok()?;
                if vitals.is_dead() {
                    return None;
                }
                let missing = character.max_hp().saturating_sub(vitals.hp);
                if missing == 0 {
                    return None;
                }
                let amount = magnitude(rng, spell.dice, strength).min(missing);
                vitals.heal(amount, character.max_hp());
                Some(Effect::Healed { victim: v, amount })
            });
            each(&mut |v, rng, world| {
                let Victim::Member(member) = v else { return None };
                let (character, mut vitals, _) = world.members.get_mut(member).ok()?;
                if vitals.is_dead() {
                    return None;
                }
                let missing = character.max_stamina().saturating_sub(vitals.stamina);
                if missing == 0 {
                    return None;
                }
                let dice =
                    spell.dice.map(|dice| Dice { count: dice.count.saturating_mul(2), ..dice });
                let amount = magnitude(rng, dice, strength).min(missing);
                vitals.restore_stamina(amount, character.max_stamina());
                Some(Effect::Refreshed { victim: v, amount })
            });
        }
        Kind::Damage => each(&mut |v, rng, world| {
            let rolled = magnitude(rng, spell.dice, strength);
            damage(rng, world, v, realm, rolled, strength.against)
        }),
        Kind::DamageUndead { banish } => each(&mut |v, rng, world| {
            let Victim::Monster(m) = v else { return None };
            let (body, ..) = world.monsters.monsters.get(m).ok()?;
            let kind = world.species.0.get(body.species as usize)?.kind;
            if !(kind == 0x14 || kind == 0x15 || (banish && kind == 0x1c)) {
                return Some(Effect::Nothing { victim: v });
            }
            let rolled = magnitude(rng, spell.dice, strength);
            damage(rng, world, v, realm, rolled, strength.against)
        }),
        Kind::Heal | Kind::Stamina | Kind::Restoration => each(&mut |v, rng, world| {
            let Victim::Member(member) = v else { return None };
            let (character, mut vitals, _) = world.members.get_mut(member).ok()?;
            if vitals.is_dead() {
                return None;
            }
            let mut effect = Effect::Nothing { victim: v };
            if kind != Kind::Stamina {
                let dice = if kind == Kind::Restoration {
                    Some(Dice { base: 0, count: 8, sides: 8 })
                } else {
                    spell.dice
                };
                let missing = character.max_hp().saturating_sub(vitals.hp);
                if missing > 0 {
                    let amount = magnitude(rng, dice, strength).min(missing);
                    vitals.heal(amount, character.max_hp());
                    effect = Effect::Healed { victim: v, amount };
                }
            }
            if kind != Kind::Heal {
                let dice = if kind == Kind::Restoration {
                    Some(Dice { base: 0, count: 8, sides: 8 })
                } else {
                    spell.dice
                };
                let missing = character.max_stamina().saturating_sub(vitals.stamina);
                if missing > 0 {
                    let amount = magnitude(rng, dice, strength).min(missing);
                    vitals.restore_stamina(amount, character.max_stamina());
                    if kind == Kind::Stamina {
                        effect = Effect::Refreshed { victim: v, amount };
                    }
                }
            }
            if kind == Kind::Restoration {
                use Condition::*;
                let all = [
                    Diseased,
                    Irritated,
                    Nauseated,
                    Slowed,
                    Afraid,
                    Poisoned,
                    Silenced,
                    Hexed,
                    Insane,
                    Blind,
                    Turncoat,
                    Webbed,
                    Asleep,
                    Paralyzed,
                    Unconscious,
                ];
                cure(rng, &mut vitals.conditions, &all, strength, true);
            }
            Some(effect)
        }),
        Kind::Inflict(condition, chance) => each(&mut |v, rng, world| {
            attack(rng, world, spell, v, realm, condition, chance, strength)
        }),
        Kind::DamageThen(..) | Kind::NoxiousFumes => {
            each(&mut |v, rng, world| {
                let rolled = magnitude(rng, spell.dice, strength);
                damage(rng, world, v, realm, rolled, strength.against)
            });
            let attacks: &[(Condition, u32)] = match kind {
                Kind::DamageThen(..) => &[(condition_of(kind), chance_of(kind))],
                _ => &[(Condition::Nauseated, 90), (Condition::Unconscious, 25)],
            };
            for &(condition, chance) in attacks {
                each(&mut |v, rng, world| {
                    attack(rng, world, spell, v, realm, condition, chance, strength)
                });
            }
        }
        Kind::Fear { second } => each(&mut |v, rng, world| {
            let landed = attack(rng, world, spell, v, realm, Condition::Afraid, 100, strength);
            let again = matches!(
                (v, second),
                (Victim::Member(_), Condition::Unconscious)
                    | (Victim::Monster(_), Condition::Afraid)
            );
            if matches!(landed, Some(Effect::Inflicted { .. })) && again {
                let half = Strength { against: strength.against / 2, ..strength };
                attack(rng, world, spell, v, realm, second, 100, half);
            }
            landed
        }),
        Kind::Pandemonium => each(&mut |v, rng, world| match attack(
            rng,
            world,
            spell,
            v,
            realm,
            Condition::Insane,
            100,
            strength,
        ) {
            Some(Effect::Resisted { .. }) => {
                let target = target(world, v, realm, Some(Condition::Afraid))?;
                let rounds = duration(rng, spell, strength);
                let rounds = resist(rng, rounds, target.resistance, target.level, strength.against);
                if rounds == 0 || target.immune {
                    return Some(Effect::Resisted { victim: v });
                }
                let lasting = Lasting::Rounds(rounds.max(2).min(u32::from(u16::MAX)) as u16);
                match v {
                    Victim::Monster(m) => inflict(
                        &mut world.monsters.monsters.get_mut(m).ok()?.0.conditions,
                        Condition::Afraid,
                        lasting,
                    ),
                    Victim::Member(m) => {
                        world.members.get_mut(m).ok()?.1.inflict(Condition::Afraid, lasting)
                    }
                }
                Some(Effect::Inflicted { victim: v, condition: Condition::Afraid })
            }
            other => other,
        }),
        Kind::Slay => each(&mut |v, rng, world| slay(rng, world, spell, v, realm, strength)),
        Kind::Affliction => each(&mut |v, rng, world| {
            let roll = rng.below(100);
            if roll >= 50 {
                let rolled = magnitude(rng, spell.dice, strength);
                return damage(rng, world, v, realm, rolled, strength.against);
            }
            let mut left = roll;
            let pick = AFFLICTIONS
                .iter()
                .find(|&&(_, weight)| {
                    if left < weight {
                        true
                    } else {
                        left -= weight;
                        false
                    }
                })
                .and_then(|&(c, _)| c);
            match pick {
                Some(condition) => attack(rng, world, spell, v, realm, condition, 100, strength),
                None => slay(rng, world, spell, v, realm, strength),
            }
        }),
        Kind::Cure(conditions) => each(&mut |v, rng, world| {
            let lifted = match v {
                Victim::Member(m) => {
                    let (_, mut vitals, _) = world.members.get_mut(m).ok()?;
                    cure(rng, &mut vitals.conditions, conditions, strength, false)
                }
                Victim::Monster(m) => {
                    let (mut body, ..) = world.monsters.monsters.get_mut(m).ok()?;
                    cure(rng, &mut body.conditions, conditions, strength, false)
                }
            };
            Some(match lifted.first() {
                Some(&condition) => Effect::Cured { victim: v, condition },
                None => Effect::Nothing { victim: v },
            })
        }),
        Kind::Resurrect => each(&mut |v, _, world| {
            let Victim::Member(member) = v else { return None };
            let (character, mut vitals, _) = world.members.get_mut(member).ok()?;
            if !vitals.is_dead() {
                return Some(Effect::Nothing { victim: v });
            }
            // Retail lifts Dead with health and stamina at
            // `max·((PIE + VIT) >> 2)/100`, at least 1.
            let share = (character.attribute(Attribute::Piety)
                + character.attribute(Attribute::Vitality))
                >> 2;
            vitals.hp = (character.max_hp() * share / 100).max(1);
            vitals.stamina = (character.max_stamina() * share / 100).max(1);
            world.formation.update_seat(member, false);
            Some(Effect::Raised { victim: v })
        }),
        Kind::Lifesteal => each(&mut |v, rng, world| {
            let target = target(world, v, realm, None)?;
            if target.dead {
                return None;
            }
            let rolled = magnitude(rng, spell.dice, strength);
            let effect = damage(rng, world, v, realm, rolled, strength.against)?;
            if let Effect::Damaged { amount, .. } = effect {
                let wounded = world.party.iter().copied().min_by_key(|&m| {
                    world.members.get(m).map_or(u32::MAX, |(c, v, _)| {
                        if v.is_dead() { u32::MAX } else { v.hp * 100 / c.max_hp().max(1) }
                    })
                });
                if let Some(member) = wounded
                    && let Ok((character, mut vitals, _)) = world.members.get_mut(member)
                {
                    vitals.heal(amount * 7 / 10, character.max_hp());
                }
            }
            Some(effect)
        }),
    }
    effects
}

fn condition_of(kind: Kind) -> Condition {
    match kind {
        Kind::DamageThen(condition, _) => condition,
        _ => Condition::Nauseated,
    }
}

fn chance_of(kind: Kind) -> u32 {
    match kind {
        Kind::DamageThen(_, chance) => chance,
        _ => 0,
    }
}

/// Whether anything the spell did counts as affecting a victim, which is
/// what practice needs (retail `SpellAffectedTarget`).
pub fn affected(effects: &[Effect]) -> bool {
    effects.iter().any(|e| !matches!(e, Effect::Nothing { .. }))
}

/// Whether every victim of a restoring spell is now whole, so a repeated
/// cast can stop.
pub fn all_restored(spell: SpellId, victims: &[Victim], members: &Members) -> bool {
    victims.iter().all(|&victim| {
        let Victim::Member(member) = victim else { return true };
        members.get(member).is_ok_and(|(character, vitals, _)| {
            vitals.is_dead()
                || match kind(spell) {
                    Some(Kind::Stamina) => vitals.stamina >= character.max_stamina(),
                    _ => vitals.hp >= character.max_hp(),
                }
        })
    })
}
