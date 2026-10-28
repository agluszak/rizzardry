//! What spells do to their victims (retail `ProcessSpellEffectTargets`,
//! `ResolveSpellMissileHit` and the effect functions of `Magic Effects.cpp`),
//! table-driven by spell. Each spell resolves in retail's passes: every
//! victim's damage, then every victim's conditions.

use bevy::prelude::*;

use super::{LearnedSpells, SpellPoints, Strength};
use crate::content::characters::{Attribute, Realm, Skill};
use crate::content::items::{Dice, EquipClass, ItemCatalog};
use crate::content::species::{MonsterCatalog, MonsterKind};
use crate::content::spells::{SpellDefinition, SpellId};
use crate::game::characters::{Condition, Lasting, inflict, resistance::Resistances, wake_on_hit};
use crate::game::difficulty::{Difficulty, Side};
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
    Enchant,
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
    MightToMagic,
    Purify,
}

/// The spells implemented, by retail id (`spell_ids.h`), and what they do.
fn kind(spell: SpellId) -> Option<Kind> {
    use Condition::*;
    Some(match spell {
        SpellId::BOILING_BLOOD
        | SpellId::BOILING_BLOOD_EXPLOSION
        | SpellId::ACID_SPLASH
        | SpellId::FROST
        | SpellId::ENERGY_BLAST
        | SpellId::MAKE_WOUNDS
        | SpellId::MAGIC_MISSILES
        | SpellId::SHRILL_SOUND
        | SpellId::FIREBALL
        | SpellId::PSIONIC_FIRE
        | SpellId::WHIPPING_ROCKS
        | SpellId::CRUSH
        | SpellId::EGO_WHIP
        | SpellId::FIRE_BOMB
        | SpellId::ICEBALL
        | SpellId::WHIRLWIND
        | SpellId::DEHYDRATE
        | SpellId::LIGHTNING
        | SpellId::EARTHQUAKE
        | SpellId::FALLING_STARS
        | SpellId::MIND_FLAY
        | SpellId::NUCLEAR_BLAST
        | SpellId::TSUNAMI => Kind::Damage,
        SpellId::SHADOW_HOUND
        | SpellId::CHAMELEON
        | SpellId::ACID_BOMB
        | SpellId::DEATH_CLOUD
        | SpellId::TOXIC_CLOUD
        | SpellId::DRAINING_CLOUD
        | SpellId::FIRESTORM
        | SpellId::RAZOR_CLOAK
        | SpellId::ARMORMELT
        | SpellId::RING_OF_FIRE
        | SpellId::LIGHT
        | SpellId::X_RAY
        | SpellId::BLESS
        | SpellId::DETECT_SECRETS
        | SpellId::ENCHANTED_BLADE
        | SpellId::GUARDIAN_ANGEL
        | SpellId::MISSILE_SHIELD
        | SpellId::ARMORPLATE
        | SpellId::MAGIC_SCREEN
        | SpellId::ELEMENT_SHIELD
        | SpellId::HASTE
        | SpellId::SOUL_SHIELD
        | SpellId::SUPERMAN
        | SpellId::BODY_OF_STONE => Kind::Enchant,
        SpellId::HOLY_WATER => Kind::DamageUndead { banish: false },
        SpellId::BANISH => Kind::DamageUndead { banish: true },
        SpellId::HEAL_WOUNDS | SpellId::HEAL_ALL => Kind::Heal,
        SpellId::STAMINA | SpellId::REST_ALL => Kind::Stamina,
        SpellId::RESTORATION => Kind::Restoration,
        SpellId::ITCHING_SKIN => Kind::Inflict(Irritated, 100),
        SpellId::PARALYZE | SpellId::FREEZE_FLESH | SpellId::FREEZE_ALL => {
            Kind::Inflict(Paralyzed, 100)
        }
        SpellId::SLEEP => Kind::Inflict(Asleep, 100),
        SpellId::BLINDING_FLASH => Kind::Inflict(Blind, 100),
        SpellId::INSANITY => Kind::Inflict(Insane, 100),
        SpellId::SLOW => Kind::Inflict(Slowed, 100),
        SpellId::WEB => Kind::Inflict(Webbed, 100),
        SpellId::SILENCE => Kind::Inflict(Silenced, 100),
        SpellId::HEX => Kind::Inflict(Hexed, 100),
        SpellId::MIND_STAB | SpellId::PSIONIC_BLAST => Kind::DamageThen(Insane, 25),
        SpellId::BLIZZARD => Kind::DamageThen(Blind, 25),
        SpellId::CEREBRAL_HEMORRHAGE => Kind::DamageThen(Insane, 75),
        SpellId::CONCUSSION => Kind::DamageThen(Unconscious, 50),
        SpellId::NOXIOUS_FUMES => Kind::NoxiousFumes,
        SpellId::TERROR => Kind::Fear { second: Unconscious },
        SpellId::SONIC_BOOM => Kind::Fear { second: Afraid },
        SpellId::PANDEMONIUM => Kind::Pandemonium,
        SpellId::INSTANT_DEATH
        | SpellId::QUICKSAND
        | SpellId::ASPHYXIATION
        | SpellId::DEATH_WISH => Kind::Slay,
        SpellId::PRISMIC_RAY | SpellId::PRISMIC_CHAOS => Kind::Affliction,
        SpellId::CURE_LESSER_COND => Kind::Cure(&[Irritated, Nauseated, Afraid, Asleep, Blind]),
        SpellId::CURE_PARALYSIS => Kind::Cure(&[Paralyzed, Webbed]),
        SpellId::CURE_POISON => Kind::Cure(&[Poisoned]),
        SpellId::CURE_DISEASE => Kind::Cure(&[Diseased]),
        SpellId::REMOVE_CURSE => Kind::Cure(&[Hexed]),
        SpellId::SANE_MIND => Kind::Cure(&[Insane, Turncoat]),
        SpellId::RESURRECTION => Kind::Resurrect,
        SpellId::LIFESTEAL => Kind::Lifesteal,
        SpellId::MIGHT_TO_MAGIC => Kind::MightToMagic,
        SpellId::PURIFY_AIR => Kind::Purify,
        SpellId::PHILOSOPHERS_BANE => Kind::GoldLoss,
        SpellId::VAMPIRIC_VORTEX => Kind::Damage,
        SpellId::HEATWAVE => Kind::DestroyConsumables,
        SpellId::ANTI_MAGIC => Kind::DrainMagic,
        SpellId::RESTORE_HEALTH => Kind::RestoreHealth,
        SpellId::RESTORE_MAGIC => Kind::RestoreMagic,
        SpellId::SMELLING_SALTS => Kind::Cure(&[Unconscious, Asleep]),
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

impl Victim {
    pub fn entity(self) -> Entity {
        match self {
            Victim::Member(entity) | Victim::Monster(entity) => entity,
        }
    }
}

/// What a spell did to one victim.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    Purified {
        victim: Victim,
    },
    Enchanted {
        victim: Victim,
        spell: SpellId,
    },
    Healed {
        victim: Victim,
        amount: u32,
    },
    Refreshed {
        victim: Victim,
        amount: u32,
    },
    Replenished {
        victim: Victim,
        amount: u32,
    },
    Damaged {
        victim: Victim,
        amount: u32,
        outcome: DamageOutcome,
    },
    Inflicted {
        victim: Victim,
        condition: Condition,
    },
    Cured {
        victim: Victim,
        condition: Condition,
    },
    Raised {
        victim: Victim,
    },
    GoldLost {
        amount: u32,
    },
    Destroyed {
        class: EquipClass,
        amount: u32,
    },
    Drained {
        victim: Victim,
        amount: u32,
    },
    /// Stamina a blow drained.
    Tired {
        victim: Victim,
        amount: u32,
    },
    Resisted {
        victim: Victim,
    },
    Nothing {
        victim: Victim,
    },
}

use crate::game::inventory::PartyInventory;
use crate::game::party::Members;

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
    pub underwater: bool,
    pub visibility: Option<&'a crate::game::world::obstacles::WorldObstacles<'a, 'a>>,
    pub enchantments: &'a mut super::enchantments::Enchantments,
    pub members: &'a mut Members<'m, 'ms>,
    pub monsters: &'a mut MonsterDamage<'d, 'ds>,
    /// The party's pool and purse.
    pub inventory: &'a mut PartyInventory,
    pub formation: &'a mut Mut<'f, Formation>,
    pub party_damage: &'a mut MessageWriter<'p, DamageApplied>,
    pub items: &'a ItemCatalog,
    pub species: Option<&'a MonsterCatalog>,
    pub spells: Option<&'a crate::content::spells::SpellCatalog>,
    pub resistances: &'a Query<'a, 'a, &'static Resistances>,
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
pub(crate) fn magnitude(rng: &mut GameRng, dice: Option<Dice>, strength: Strength) -> u32 {
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

/// The monster kinds Dispel Undead harms.
const UNDEAD_KINDS: [MonsterKind; 2] = [MonsterKind::Undead, MonsterKind::Demon];
/// The kind only Banish harms besides them.
const BANISHED_KIND: MonsterKind = MonsterKind::Spirit;

/// Whether a monster of `kind` shrugs a condition off (retail
/// `g_condition_immunities`).
fn immune(kind: MonsterKind, condition: Condition) -> bool {
    use Condition::*;
    let immune: &[_] = match kind {
        MonsterKind::Undead => {
            &[Diseased, Poisoned, Unconscious, Irritated, Afraid, Insane, Asleep, Nauseated]
        }
        MonsterKind::Android => &[Diseased, Poisoned, Nauseated, Irritated, Afraid, Asleep],
        MonsterKind::Plant => &[Insane, Afraid, Nauseated],
        _ => &[],
    };
    immune.contains(&condition)
}

/// Rounds a condition spell inflicts (retail `RollEffectDuration`).
fn duration(rng: &mut GameRng, spell: &SpellDefinition, strength: Strength) -> u32 {
    let base = spell.duration_per_power * i32::from(strength.power) + spell.duration_base;
    if base == 9999 {
        return 9999;
    }
    let mut rounds = (base + 1).max(0) as u32;
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
            let (body, ..) = world.monsters.monsters().get(monster).ok()?;
            let kind = world.species?.kind(body.species)?;
            Some(Target {
                level: u32::from(kind.level),
                resistance: (i32::from(kind.resistances[realm])
                    + body.enchantments.resistance(realm))
                .min(100),
                side: Some(Side::Hostile),
                turncoat: false,
                immune: condition.is_some_and(|c| immune(kind.kind, c)),
                dead: body.is_dead(),
            })
        }
        Victim::Member(member) => {
            let (character, vitals, ..) = world.members.get(member).ok()?;
            Some(Target {
                level: u32::from(character.level),
                resistance: world.resistances.get(member).ok()?.0[realm].total,
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
    let amount = resisted_magnitude(rng, world, victim, realm, rolled, against)?;
    if amount == 0 {
        return Some(Effect::Resisted { victim });
    }
    hurt(rng, world, victim, amount)
}

/// The post-resistance magnitude remains a transaction input, independent
/// of absorption, overkill and the HP outcome.
fn resisted_magnitude(
    rng: &mut GameRng,
    world: &Field,
    victim: Victim,
    realm: Realm,
    rolled: u32,
    against: u32,
) -> Option<u32> {
    let target = target(world, victim, realm, None).filter(|target| !target.dead)?;
    Some(resist(rng, rolled, target.resistance, target.level, against))
}

/// Take `amount` health from a victim; a member who survives may wake.
pub fn hurt(rng: &mut GameRng, world: &mut Field, victim: Victim, amount: u32) -> Option<Effect> {
    match victim {
        Victim::Monster(monster) => {
            let outcome = world
                .monsters
                .apply(monster, amount, world.party_level, world.inventory, rng)
                .ok()?;
            Some(Effect::Damaged { victim, amount: outcome.damage, outcome })
        }
        Victim::Member(member) => {
            let (character, mut vitals, ..) = world.members.get_mut(member).ok()?;
            let lost = crate::game::party::damage_member(
                member,
                &mut vitals,
                world.formation,
                amount,
                world.party_damage,
            );
            if lost > 0 && !vitals.is_dead() {
                wake_on_hit(&mut vitals.conditions, character.attribute(Attribute::Senses), rng);
            }
            let outcome = DamageOutcome { damage: lost, died: vitals.is_dead(), experience: 0 };
            Some(Effect::Damaged { victim, amount: lost, outcome })
        }
    }
}

/// Haste and Slow cancel durations before either may remain.
fn counter_haste(conditions: &mut Vec<(Condition, Lasting)>, turns: u32) -> u32 {
    let mut remaining = turns;
    conditions.retain_mut(|(condition, lasting)| {
        if *condition != Condition::Slowed {
            return true;
        }
        match lasting {
            Lasting::Indefinite => {
                remaining = 0;
                true
            }
            Lasting::Rounds(left) => {
                remaining = turns.saturating_sub(u32::from(*left));
                *left = u32::from(*left).saturating_sub(turns) as u16;
                *left > 0
            }
        }
    });
    remaining
}

/// Give a victim a condition for `lasting`.
fn inflict_on(
    world: &mut Field,
    victim: Victim,
    condition: Condition,
    lasting: Lasting,
) -> Option<()> {
    match victim {
        Victim::Monster(monster) => {
            let lasting = if condition == Condition::Slowed {
                match lasting {
                    Lasting::Rounds(turns) => {
                        let remaining = world
                            .monsters
                            .enchantments_mut(monster)?
                            .counter_slow(u32::from(turns));
                        if remaining == 0 {
                            return Some(());
                        }
                        Lasting::Rounds(remaining as u16)
                    }
                    Lasting::Indefinite => {
                        world
                            .monsters
                            .enchantments_mut(monster)?
                            .0
                            .retain(|e| e.spell != crate::content::spells::SpellId::HASTE);
                        lasting
                    }
                }
            } else {
                lasting
            };
            inflict(&mut *world.monsters.conditions_mut(monster)?, condition, lasting)
        }
        Victim::Member(member) => world.members.get_mut(member).ok()?.1.inflict(condition, lasting),
    }
    Some(())
}

/// Retail `InflictConditionAttack` on one victim: the chance, a fresh
/// duration, then [`afflict`].
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
    // A victim that died or left is not attacked at all.
    target(world, victim, realm, Some(condition)).filter(|t| !t.dead)?;
    let rounds = duration(rng, spell, strength);
    afflict(rng, world, victim, Some(condition), realm, strength.against, Some(rounds))
}

/// Retail `ResolveAttackOnTarget`: a condition, or death when `condition` is
/// `None`, unless the victim is immune, saves, or shrugs it off by its
/// nature; then resistance cuts its duration, and nothing lands from none
/// (retail `InflictConditionOnTarget`). `rounds` of `None` lasts until
/// lifted.
fn afflict(
    rng: &mut GameRng,
    world: &mut Field,
    victim: Victim,
    condition: Option<Condition>,
    realm: Realm,
    against: u32,
    rounds: Option<u32>,
) -> Option<Effect> {
    let target = target(world, victim, realm, condition)?;
    if target.dead {
        return None;
    }
    let death_immune = match victim {
        Victim::Monster(monster) => {
            condition.is_none() && {
                let (body, ..) = world.monsters.monsters().get(monster).ok()?;
                world.species?.kind(body.species)?.death_immune
            }
        }
        Victim::Member(_) => false,
    };
    if target.immune || death_immune {
        return Some(Effect::Resisted { victim });
    }
    let base = condition.map_or(DEATH_SAVE, |c| CONDITION_SAVE[c as usize + 1]);
    if saves(rng, world, victim, &target, base, against) {
        return Some(Effect::Resisted { victim });
    }
    if condition.is_some_and(|condition| shrugs(world, victim, condition, rounds.is_none())) {
        return Some(Effect::Resisted { victim });
    }
    let lasting = rounds.unwrap_or(INDEFINITE);
    let lasting = match resist(rng, lasting, target.resistance, target.level, against) {
        0 => return Some(Effect::Resisted { victim }),
        _ if rounds.is_none() => Lasting::Indefinite,
        1 => Lasting::Rounds(2),
        rounds => Lasting::Rounds(rounds.min(u32::from(u16::MAX)) as u16),
    };
    let Some(condition) = condition else {
        let hp = match victim {
            Victim::Monster(monster) => world.monsters.monsters().get(monster).ok()?.0.hp,
            Victim::Member(member) => world.members.get(member).ok()?.1.hp,
        };
        return hurt(rng, world, victim, hp);
    };
    inflict_on(world, victim, condition, lasting)?;
    Some(Effect::Inflicted { victim, condition })
}

/// Constitution and profession immunities are checked after the save.
fn shrugs(world: &Field, victim: Victim, condition: Condition, indefinite: bool) -> bool {
    use crate::content::characters::Trait;
    let Victim::Member(member) = victim else { return false };
    let Ok((character, ..)) = world.members.get(member) else { return false };
    let android = character.has_trait(Trait::AndroidConditionImmunity);
    match condition {
        Condition::Afraid => character.has_trait(Trait::Fearless) || android,
        Condition::Diseased
        | Condition::Irritated
        | Condition::Nauseated
        | Condition::Poisoned
        | Condition::Asleep => android,
        Condition::Insane if indefinite => false,
        Condition::Insane | Condition::Turncoat => {
            character.has_trait(Trait::MentalConditionImmunity)
        }
        _ => false,
    }
}

/// Retail's marker for a condition without end (`W8_CONDITION_INDEFINITE`),
/// which resistance cuts like any other duration.
const INDEFINITE: u32 = 9999;

/// Rounds, or an amount, a blow's effect rolls (retail `RollEffectDuration`
/// on an empty definition, then `count`d2 + `base` and one more when that
/// came to under two).
fn blow_roll(rng: &mut GameRng, count: u32, sides: u8, base: u32) -> u32 {
    if rng.below(4) == 1 {
        return 2;
    }
    let dice = Dice { count: count.min(255) as u8, sides, base: base.min(i16::MAX as u32) as i16 };
    rng.roll(dice) + 1
}

/// Retail `ApplyEffectConditions`: what a weapon or a monster's blow that
/// drew blood does besides its damage. Each effect with a chance rolls for
/// it in turn until the victim dies; `power` is the attacker's effect
/// strength, and only a monster swallows or possesses.
pub fn blow(
    rng: &mut GameRng,
    world: &mut Field,
    points: &mut Query<(&LearnedSpells, &mut SpellPoints)>,
    victim: Victim,
    effects: &crate::content::items::AttackEffects,
    power: u32,
    by_monster: bool,
) -> Vec<Effect> {
    use crate::content::items::AttackEffect as E;
    let mut landed = Vec::new();
    for (effect, &chance) in &effects.chances {
        if target(world, victim, Realm::Fire, None).is_none_or(|t| t.dead) {
            break;
        }
        if chance == 0 || rng.below(100) >= u32::from(chance) {
            continue;
        }
        let count = power / 3 + 1;
        let rounds = blow_roll(rng, count, 2, 1).clamp(1, 12);
        let mut afflicts = |rng: &mut GameRng, condition, realm, rounds| {
            afflict(rng, world, victim, condition, realm, power, rounds)
        };
        let member = matches!(victim, Victim::Member(_));
        let outcome = match effect {
            E::Sleep => afflicts(rng, Some(Condition::Asleep), Realm::Air, Some(rounds)),
            E::Paralyze => afflicts(rng, Some(Condition::Paralyzed), Realm::Water, Some(rounds)),
            E::Poison => {
                let rounds = blow_roll(rng, count * 3, 2, u32::from(effects.strength));
                afflicts(rng, Some(Condition::Poisoned), Realm::Water, Some(rounds))
            }
            E::Hex => afflicts(rng, Some(Condition::Hexed), Realm::Divine, Some(rounds)),
            E::Disease => afflicts(rng, Some(Condition::Diseased), Realm::Water, None),
            E::Kill => afflicts(rng, None, Realm::Divine, None),
            E::KnockOut => afflicts(rng, Some(Condition::Unconscious), Realm::Earth, Some(rounds)),
            E::Blind => afflicts(rng, Some(Condition::Blind), Realm::Fire, Some(rounds)),
            E::Frighten => afflicts(rng, Some(Condition::Afraid), Realm::Mental, Some(rounds)),
            // Being swallowed (retail Missing) is not modelled.
            E::Swallow => None,
            E::Possess if by_monster && member => {
                afflicts(rng, Some(Condition::Turncoat), Realm::Divine, Some(rounds))
            }
            E::Possess => None,
            E::DrainHealth if member => {
                drain(rng, world, victim, Realm::Earth, power, (count, 2, 0))
                    .and_then(|amount| hurt(rng, world, victim, amount))
            }
            E::DrainHealth => None,
            E::DrainStamina => {
                drain(rng, world, victim, Realm::Water, power, (count * 2, 5, count * 3)).map(
                    |amount| {
                        let amount = match victim {
                            Victim::Member(member) => {
                                world.members.get_mut(member).map_or(0, |(_, mut vitals, ..)| {
                                    let before = vitals.stamina;
                                    vitals.spend_stamina(amount);
                                    before - vitals.stamina
                                })
                            }
                            Victim::Monster(monster) => world.monsters.fatigue(monster, amount),
                        };
                        Effect::Tired { victim, amount }
                    },
                )
            }
            E::DrainSpellPoints => match victim {
                Victim::Member(member) => {
                    drain(rng, world, victim, Realm::Divine, power, (count, 6, count)).and_then(
                        |amount| {
                            let (_, mut pool) = points.get_mut(member).ok()?;
                            let amount = pool.drain(amount, rng);
                            Some(Effect::Drained { victim, amount })
                        },
                    )
                }
                Victim::Monster(_) => None,
            },
            E::Nauseate => afflicts(rng, Some(Condition::Nauseated), Realm::Air, Some(rounds)),
            E::Insane => afflicts(rng, Some(Condition::Insane), Realm::Mental, Some(rounds)),
        };
        landed.extend(outcome);
    }
    landed
}

/// A blow's drain: the victim's save, then `dice` (count, sides, base),
/// one more, cut by resistance. `None` when nothing is left to take.
fn drain(
    rng: &mut GameRng,
    world: &mut Field,
    victim: Victim,
    realm: Realm,
    power: u32,
    (count, sides, base): (u32, u8, u32),
) -> Option<u32> {
    let target = target(world, victim, realm, None).filter(|t| !t.dead)?;
    if saves(rng, world, victim, &target, 0, power) {
        return None;
    }
    let amount = blow_roll(rng, count, sides, base);
    Some(resist(rng, amount, target.resistance, target.level, power)).filter(|&a| a != 0)
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
    kill(rng, world, victim)
}

/// Death bypasses absorption, while retaining the common death/loot transaction.
fn kill(rng: &mut GameRng, world: &mut Field, victim: Victim) -> Option<Effect> {
    let hp = match victim {
        Victim::Monster(monster) => world.monsters.monsters().get(monster).ok()?.0.hp,
        Victim::Member(member) => world.members.get(member).ok()?.1.hp,
    };
    // A failed death save inflicts Dead; it is not absorbable damage.
    match victim {
        Victim::Member(member) => world
            .members
            .get_mut(member)
            .ok()?
            .1
            .enchantments
            .0
            .retain(|e| e.spell != crate::content::spells::SpellId::GUARDIAN_ANGEL),
        Victim::Monster(monster) => world
            .monsters
            .enchantments_mut(monster)?
            .0
            .retain(|e| e.spell != crate::content::spells::SpellId::GUARDIAN_ANGEL),
    }
    hurt(rng, world, victim, hp)
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
    backfired: bool,
    spell: &SpellDefinition,
    strength: Strength,
    victims: &[Victim],
    rng: &mut GameRng,
    world: &mut Field,
    points: &mut Query<(&LearnedSpells, &mut SpellPoints)>,
) -> Vec<Effect> {
    if victims.is_empty() {
        return Vec::new();
    }
    let (Some(kind), Some(realm)) = (kind(spell.id), spell.realm) else { return Vec::new() };
    let kind = if backfired && super::enchantments::personal(spell.id) {
        if spell.id == SpellId::HASTE {
            Kind::Inflict(Condition::Slowed, 100)
        } else if spell.id == SpellId::RAZOR_CLOAK {
            Kind::Damage
        } else {
            return Vec::new();
        }
    } else {
        kind
    };
    let mut effects = Vec::new();
    let mut each = |f: &mut dyn FnMut(Victim, &mut GameRng, &mut Field) -> Option<Effect>| {
        for &victim in victims {
            if let Some(effect) = f(victim, rng, world) {
                effects.push(effect);
            }
        }
    };
    match kind {
        Kind::Purify => {
            let Some(catalog) = world.spells else { return effects };
            // Purify rolls from power alone, rather than its authored duration.
            let base = u32::from(strength.power) + 1;
            let amount = match rng.below(4) {
                0 if base > 1 => base + 1,
                1 if base < 3 => base + 1,
                _ => base,
            };
            let amount = amount + amount * strength.boost / 100;
            let mut party_done = false;
            for &victim in victims {
                let purify = |slots: &mut super::enchantments::Enchantments| {
                    slots.0.retain_mut(|slot| {
                        if matches!(
                            slot.spell,
                            SpellId::ACID_BOMB
                                | SpellId::TOXIC_CLOUD
                                | SpellId::DRAINING_CLOUD
                                | SpellId::FIRESTORM
                                | SpellId::DEATH_CLOUD
                        ) && let Some(cloud) = catalog.get(slot.spell)
                        {
                            let removed = (amount + u32::from(spell.level))
                                .saturating_sub(u32::from(cloud.level));
                            slot.turns = slot.turns.saturating_sub(removed);
                        }
                        slot.turns > 0
                    });
                };
                match victim {
                    Victim::Member(_) if !party_done => {
                        purify(world.enchantments);
                        party_done = true;
                    }
                    Victim::Member(_) => continue,
                    Victim::Monster(actor) => {
                        let Some(mut slots) = world.monsters.enchantments_mut(actor) else {
                            continue;
                        };
                        purify(&mut slots);
                    }
                }
                effects.push(Effect::Purified { victim });
            }
        }
        Kind::Enchant => {
            let turns = duration(rng, spell, strength);
            let mut party_applied = false;
            for &victim in victims {
                let personal = super::enchantments::personal(spell.id);
                let effect = super::enchantments::Enchantment {
                    spell: spell.id,
                    power: strength.power,
                    boost: strength.boost,
                    turns,
                    absorption: 0,
                };
                let mut apply = |enchantments: &mut super::enchantments::Enchantments,
                                 monster: bool| {
                    let accepts = if monster {
                        enchantments.monster_accepts(spell.id, strength.power)
                    } else {
                        enchantments.accepts(spell.id, strength.power)
                    };
                    if !accepts {
                        return false;
                    }
                    let absorption = if spell.id == SpellId::GUARDIAN_ANGEL {
                        let rolled =
                            spell.dice.map_or(0, |dice| rng.roll(dice)) * u32::from(strength.power);
                        rolled + rolled * strength.boost / 100
                    } else {
                        0
                    };
                    let effect = super::enchantments::Enchantment { absorption, ..effect };
                    if monster {
                        enchantments.apply_to_monster(effect)
                    } else {
                        enchantments.apply(effect)
                    }
                };
                let changed = match victim {
                    Victim::Member(member) if personal => {
                        world.members.get_mut(member).is_ok_and(|(profile, mut v, ..)| {
                            if v.is_dead() {
                                return false;
                            }
                            if spell.id == SpellId::HASTE {
                                let remaining = counter_haste(&mut v.conditions, turns);
                                if remaining == 0 {
                                    return true;
                                }
                                return v.enchantments.apply(super::enchantments::Enchantment {
                                    turns: remaining,
                                    ..effect
                                });
                            }
                            let maximum = world.spells.and_then(|spells| {
                                points.get(member).ok().map(|(known, _)| {
                                    super::max_points(
                                        &profile.stats(&v.enchantments),
                                        known,
                                        spells,
                                    )
                                })
                            });
                            let changed = apply(&mut v.enchantments, false);
                            if changed
                                && let (Some(before), Some(spells), Ok((known, mut pool))) =
                                    (maximum, world.spells, points.get_mut(member))
                            {
                                let after = super::max_points(
                                    &profile.stats(&v.enchantments),
                                    known,
                                    spells,
                                );
                                pool.adjust_limits(&before, &after);
                            }
                            changed
                        })
                    }
                    Victim::Member(_) => {
                        if party_applied {
                            continue;
                        }
                        party_applied = true;
                        apply(world.enchantments, false)
                    }
                    Victim::Monster(entity) => {
                        if spell.id == SpellId::HASTE {
                            let remaining = world
                                .monsters
                                .conditions_mut(entity)
                                .map(|mut c| counter_haste(&mut c, turns));
                            remaining.is_some_and(|turns| {
                                turns == 0
                                    || world.monsters.enchantments_mut(entity).is_some_and(
                                        |mut e| {
                                            e.apply_to_monster(super::enchantments::Enchantment {
                                                turns,
                                                ..effect
                                            })
                                        },
                                    )
                            })
                        } else {
                            world
                                .monsters
                                .enchantments_mut(entity)
                                .is_some_and(|mut e| apply(&mut e, true))
                        }
                    }
                };
                effects.push(if changed {
                    Effect::Enchanted { victim, spell: spell.id }
                } else {
                    Effect::Nothing { victim }
                });
            }
        }
        Kind::GoldLoss => {
            if victims.iter().any(|v| matches!(v, Victim::Member(_))) {
                // Retail's second roll when the first is below 100 is intentional.
                let first = magnitude(rng, spell.dice, strength);
                let percent = if first < 100 { magnitude(rng, spell.dice, strength) } else { 100 };
                let inventory = &mut world.inventory;
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
                let Ok((_, _, mut carried, ..)) = world.members.get_mut(member) else { continue };
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
                let pool = &mut world.inventory.pool.stacks;
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
                    if world.members.get(member).is_ok_and(|(_, v, ..)| !v.is_dead())
                        && let Ok((_, mut pool)) = points.get_mut(member)
                    {
                        let amount = pool.drain(amount, rng);
                        effects.push(Effect::Drained { victim: Victim::Member(member), amount });
                    }
                }
                // Anti-Magic clears persistent party slots, not combat or actor enchantments.
                world.enchantments.0.retain(|e| super::enchantments::combat_only(e.spell));
            }
        }
        Kind::RestoreMagic => each(&mut |v, rng, world| {
            let Victim::Member(member) = v else { return None };
            let (profile, vitals, ..) = world.members.get(member).ok()?;
            if vitals.is_dead() {
                return None;
            }
            // Retail rolls even when every realm is already full.
            let amount = magnitude(rng, spell.dice, strength);
            let Ok((learned, mut pool)) = points.get_mut(member) else {
                return Some(Effect::Nothing { victim: v });
            };
            let maximum =
                super::max_points(&profile.stats(&vitals.enchantments), learned, world.spells?);
            let amount = pool.restore_evenly(amount, &maximum);
            Some(if amount == 0 {
                Effect::Nothing { victim: v }
            } else {
                Effect::Replenished { victim: v, amount }
            })
        }),
        Kind::RestoreHealth => {
            // Separate passes preserve retail target and RNG order.
            each(&mut |v, rng, world| {
                let amount = restore(rng, world, v, Pool::Health, spell.dice, strength)?;
                Some(Effect::Healed { victim: v, amount })
            });
            let dice = spell.dice.map(|dice| Dice { count: dice.count.saturating_mul(2), ..dice });
            each(&mut |v, rng, world| {
                let amount = restore(rng, world, v, Pool::Stamina, dice, strength)?;
                Some(Effect::Refreshed { victim: v, amount })
            });
        }
        Kind::Damage => each(&mut |v, rng, world| {
            let rolled = magnitude(rng, spell.dice, strength);
            damage(rng, world, v, realm, rolled, strength.against)
        }),
        Kind::DamageUndead { banish } => each(&mut |v, rng, world| {
            let Victim::Monster(m) = v else { return None };
            let (body, ..) = world.monsters.monsters().get(m).ok()?;
            let kind = world.species?.kind(body.species)?.kind;
            if !(UNDEAD_KINDS.contains(&kind) || (banish && kind == BANISHED_KIND)) {
                return Some(Effect::Nothing { victim: v });
            }
            let rolled = magnitude(rng, spell.dice, strength);
            damage(rng, world, v, realm, rolled, strength.against)
        }),
        Kind::Heal | Kind::Stamina | Kind::Restoration => each(&mut |v, rng, world| {
            let Victim::Member(member) = v else { return None };
            if world.members.get(member).ok()?.1.is_dead() {
                return None;
            }
            let dice = if kind == Kind::Restoration { Some(RESTORATION_DICE) } else { spell.dice };
            let mut effect = Effect::Nothing { victim: v };
            if kind != Kind::Stamina
                && let Some(amount) = restore(rng, world, v, Pool::Health, dice, strength)
            {
                effect = Effect::Healed { victim: v, amount };
            }
            if kind != Kind::Heal
                && let Some(amount) = restore(rng, world, v, Pool::Stamina, dice, strength)
                && kind == Kind::Stamina
            {
                effect = Effect::Refreshed { victim: v, amount };
            }
            let (_, mut vitals, ..) = world.members.get_mut(member).ok()?;
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
            let attacks = match kind {
                Kind::DamageThen(condition, chance) => vec![(condition, chance)],
                _ => vec![(Condition::Nauseated, 90), (Condition::Unconscious, 25)],
            };
            for (condition, chance) in attacks {
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
                inflict_on(world, v, Condition::Afraid, lasting)?;
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
                    let (_, mut vitals, ..) = world.members.get_mut(m).ok()?;
                    cure(rng, &mut vitals.conditions, conditions, strength, false)
                }
                Victim::Monster(m) => {
                    cure(rng, &mut *world.monsters.conditions_mut(m)?, conditions, strength, false)
                }
            };
            Some(match lifted.first() {
                Some(&condition) => Effect::Cured { victim: v, condition },
                None => Effect::Nothing { victim: v },
            })
        }),
        Kind::Resurrect => each(&mut |v, _, world| {
            let Victim::Member(member) = v else { return None };
            let (character, mut vitals, ..) = world.members.get_mut(member).ok()?;
            if !vitals.is_dead() {
                return Some(Effect::Nothing { victim: v });
            }
            // Retail lifts Dead with health and stamina at
            // `max·((PIE + VIT) >> 2)/100`, at least 1.
            let share = (character.attribute(Attribute::Piety)
                + character.attribute(Attribute::Vitality))
                >> 2;
            vitals.hp = (character.stats(&vitals.enchantments).max_hp() * share / 100).max(1);
            vitals.stamina =
                (character.stats(&vitals.enchantments).max_stamina() * share / 100).max(1);
            world.formation.update_seat(member, false);
            Some(Effect::Raised { victim: v })
        }),
        Kind::Lifesteal | Kind::MightToMagic => {
            let mut drained = 0;
            each(&mut |v, rng, world| {
                let target = target(world, v, realm, None)?;
                if target.dead {
                    return None;
                }
                let rolled = magnitude(rng, spell.dice, strength);
                let effect = damage(rng, world, v, realm, rolled, strength.against)?;
                if let Effect::Damaged { amount, .. } = effect {
                    drained += amount;
                }
                Some(effect)
            });
            // Retail chooses once after every target has been drained. Do not
            // heal a different member after each hit or round each share separately.
            let recipient = world
                .party
                .iter()
                .copied()
                .filter_map(|m| {
                    let (c, v, ..) = world.members.get(m).ok()?;
                    if v.is_dead() {
                        return None;
                    }
                    let percent = if kind == Kind::Lifesteal {
                        v.hp * 100 / c.stats(&v.enchantments).max_hp().max(1)
                    } else {
                        let (learned, pool) = points.get(m).ok()?;
                        let maximum: u32 =
                            super::max_points(&c.stats(&v.enchantments), learned, world.spells?)
                                .values()
                                .sum();
                        if maximum == 0 {
                            return None;
                        }
                        pool.current.values().sum::<u32>() * 100 / maximum
                    };
                    (percent < 100).then_some((m, percent))
                })
                .min_by_key(|&(_, percent)| percent)
                .map(|(m, _)| m);
            if let Some(member) = recipient {
                let (c, mut v, ..) = world.members.get_mut(member).unwrap();
                if kind == Kind::Lifesteal {
                    let maximum = c.stats(&v.enchantments).max_hp();
                    let before = v.hp;
                    v.heal(drained * LIFESTEAL_SHARE / 100, maximum);
                    effects.push(Effect::Healed {
                        victim: Victim::Member(member),
                        amount: v.hp - before,
                    });
                } else if let (Ok((learned, mut pool)), Some(spells)) =
                    (points.get_mut(member), world.spells)
                {
                    let maximum = super::max_points(&c.stats(&v.enchantments), learned, spells);
                    let amount = pool.restore_evenly(drained, &maximum);
                    effects.push(Effect::Replenished { victim: Victim::Member(member), amount });
                }
            }
        }
    }
    effects
}

/// What Restoration restores health and stamina by, whatever its record says.
const RESTORATION_DICE: Dice = Dice { base: 0, count: 8, sides: 8 };

/// The share of Lifesteal's damage that heals the most wounded member, in
/// percent.
const LIFESTEAL_SHARE: u32 = 70;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pool {
    Health,
    Stamina,
}

/// Restore a living member's health or stamina by rolled `dice`, up to the
/// maximum: the amount, or `None` (nothing rolled) when the member lacks
/// none or cannot be restored.
fn restore(
    rng: &mut GameRng,
    world: &mut Field,
    victim: Victim,
    pool: Pool,
    dice: Option<Dice>,
    strength: Strength,
) -> Option<u32> {
    let Victim::Member(member) = victim else { return None };
    let (character, mut vitals, ..) = world.members.get_mut(member).ok()?;
    if vitals.is_dead() {
        return None;
    }
    let (current, maximum) = match pool {
        Pool::Health => (vitals.hp, character.stats(&vitals.enchantments).max_hp()),
        Pool::Stamina => (vitals.stamina, character.stats(&vitals.enchantments).max_stamina()),
    };
    let missing = maximum.saturating_sub(current);
    if missing == 0 {
        return None;
    }
    let amount = magnitude(rng, dice, strength).min(missing);
    match pool {
        Pool::Health => vitals.heal(amount, maximum),
        Pool::Stamina => vitals.restore_stamina(amount, maximum),
    }
    Some(amount)
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
        members.get(member).is_ok_and(|(character, vitals, ..)| {
            vitals.is_dead()
                || match kind(spell) {
                    Some(Kind::Stamina) => {
                        vitals.stamina >= character.stats(&vitals.enchantments).max_stamina()
                    }
                    _ => vitals.hp >= character.stats(&vitals.enchantments).max_hp(),
                }
        })
    })
}

/// Retail offensive slots follow their affected actors; they are not stationary
/// geometry. Tick before slot expiry, using indirect-source difficulty rather
/// than retaining a caster or borrowing a later caster's stats.
pub(crate) fn pulse_clouds(
    combatants: &[Entity],
    party_eye: Vec3,
    spells: &crate::content::spells::SpellCatalog,
    rng: &mut GameRng,
    world: &mut Field,
    points: &mut Query<(&LearnedSpells, &mut SpellPoints)>,
) -> Vec<(SpellId, Vec<Effect>)> {
    let mut reports = Vec::new();
    let party: Vec<_> = world.party.iter().copied().map(Victim::Member).collect();
    let targets = std::iter::once(None).chain(combatants.iter().copied().map(Some));
    for actor in targets {
        for id in [
            SpellId::ACID_BOMB,
            SpellId::TOXIC_CLOUD,
            SpellId::DRAINING_CLOUD,
            SpellId::FIRESTORM,
            SpellId::DEATH_CLOUD,
        ] {
            let effect = match actor {
                None => world.enchantments.get(id).copied(),
                Some(actor) => world
                    .monsters
                    .monsters()
                    .get(actor)
                    .ok()
                    .filter(|(monster, ..)| !monster.is_dead())
                    .and_then(|(monster, ..)| monster.enchantments.get(id).copied()),
            };
            let (Some(effect), Some(spell)) = (effect, spells.get(id)) else { continue };
            let Some(realm) = spell.realm else { continue };
            let strength = Strength::at(0, spell, effect.power, effect.boost);
            let victims = actor.map_or_else(|| party.clone(), |a| vec![Victim::Monster(a)]);
            let mut landed = Vec::new();
            for victim in victims {
                if target(world, victim, realm, None).is_none_or(|t| t.dead) {
                    continue;
                }
                if id == SpellId::DEATH_CLOUD {
                    // TickCombatEffectSlots makes a death save directly: no
                    // duration roll or duration resistance reduction.
                    let immune = match victim {
                        Victim::Monster(actor) => world
                            .monsters
                            .monsters()
                            .get(actor)
                            .ok()
                            .and_then(|(body, ..)| world.species?.kind(body.species))
                            .is_none_or(|kind| kind.death_immune),
                        Victim::Member(_) => false,
                    };
                    let target = target(world, victim, realm, None).unwrap();
                    let result = if immune
                        || saves(rng, world, victim, &target, DEATH_SAVE, strength.against)
                    {
                        Some(Effect::Resisted { victim })
                    } else {
                        kill(rng, world, victim)
                    };
                    landed.extend(result);
                } else {
                    let amount = magnitude(rng, spell.dice, strength);
                    let Some(amount) =
                        resisted_magnitude(rng, world, victim, realm, amount, strength.against)
                    else {
                        continue;
                    };
                    let hit = if amount == 0 {
                        Some(Effect::Resisted { victim })
                    } else {
                        hurt(rng, world, victim, amount)
                    };
                    if id == SpellId::DRAINING_CLOUD && amount > 0 {
                        match victim {
                            Victim::Member(member) => {
                                if let Ok((_, mut mana)) = points.get_mut(member) {
                                    for remaining in mana.current.values_mut() {
                                        *remaining = remaining.saturating_sub(amount);
                                    }
                                }
                                if let Ok((_, mut vitals, ..)) = world.members.get_mut(member) {
                                    vitals.spend_stamina(amount);
                                }
                            }
                            Victim::Monster(monster) => {
                                world.monsters.drain_magic(monster, amount / 2);
                                world.monsters.fatigue(monster, amount);
                            }
                        }
                    }
                    landed.extend(hit);
                    if id == SpellId::TOXIC_CLOUD {
                        let Some(victim_stats) =
                            target(world, victim, realm, None).filter(|t| !t.dead)
                        else {
                            continue;
                        };
                        let turns = duration(rng, spell, strength);
                        let turns = resist(
                            rng,
                            turns,
                            victim_stats.resistance,
                            victim_stats.level,
                            strength.against,
                        );
                        if turns == 0 {
                            continue;
                        }
                        let lasting = Lasting::Rounds(turns.max(2).min(u32::from(u16::MAX)) as u16);
                        // One duration reduction, followed by three separate saves.
                        for condition in
                            [Condition::Nauseated, Condition::Poisoned, Condition::Unconscious]
                        {
                            let Some(target) = target(world, victim, realm, Some(condition)) else {
                                continue;
                            };
                            if target.immune
                                || saves(
                                    rng,
                                    world,
                                    victim,
                                    &target,
                                    CONDITION_SAVE[condition as usize + 1],
                                    strength.against,
                                )
                                || shrugs(world, victim, condition, false)
                            {
                                landed.push(Effect::Resisted { victim });
                            } else if inflict_on(world, victim, condition, lasting).is_some() {
                                landed.push(Effect::Inflicted { victim, condition });
                            }
                        }
                    }
                }
            }
            reports.push((id, landed));
        }
        // Ring of Fire follows the party and samples current nearby actors on
        // every pulse, including actors that entered its radius after casting.
        if actor.is_none()
            && let (Some(effect), Some(spell)) = (
                world.enchantments.get(SpellId::RING_OF_FIRE).copied(),
                spells.get(SpellId::RING_OF_FIRE),
            )
            && let Some(realm) = spell.realm
        {
            let centre = party_eye - Vec3::Y * crate::game::party::body::EYE_ABOVE_CENTRE;
            let reach =
                crate::game::party::body::RADIUS + crate::content::species::Reach::Short.metres();
            let victims = super::targeting::radius(world, centre, centre, reach, None);
            let mut landed = Vec::new();
            for victim in victims {
                let amount = magnitude(
                    rng,
                    spell.dice,
                    Strength {
                        power: effect.power,
                        boost: 0,
                        against: u32::from(effect.power) * 3,
                    },
                );
                landed.extend(damage(
                    rng,
                    world,
                    victim,
                    realm,
                    amount,
                    u32::from(effect.power) * 3,
                ));
            }
            reports.push((spell.id, landed));
        }
    }
    reports
}
