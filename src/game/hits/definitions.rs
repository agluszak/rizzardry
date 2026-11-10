//! Immutable spell and weapon descriptions, decoded once from retail catalogs.

use std::collections::HashMap;

use bevy::prelude::*;

use super::defences::{CONDITION_SAVE, DEATH_SAVE, INDEFINITE, condition_save};
use crate::content::characters::Realm;
use crate::content::items::{AttackEffect, AttackEffects, Dice};
use crate::content::species::{MonsterKind, SpecialAttackKind};
use crate::content::spells::{SpellDefinition, SpellId};
use crate::game::characters::Condition;
use crate::game::data::Catalogs;
use crate::game::magic::Strength;
use crate::game::rng::GameRng;
use EffectKind::{
    Burn, Dispel, DrainBudget, DrainPool, Harm, Heal, LoseGold, Raise, Refresh, Renew, Replenish,
    Sap, Slay, Tire,
};

/// What something does to each body it lands on: its effects in order, and
/// whether its damage comes back to its caster's side (`Some(true)` as
/// health, `Some(false)` as spell points).
#[derive(Clone, Debug, Default)]
pub struct Definition {
    pub effects: Vec<EffectSpec>,
    pub leeches: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefinitionId(usize);

/// A weapon blow behind a strike: the damage it deals and its weapon's
/// effects, each landing at the weapon's chance for it, a poison at the
/// weapon's strength.
#[derive(Clone, Copy, Debug)]
pub struct Blow {
    pub damage: u32,
    pub effects: AttackEffects,
    pub by_monster: bool,
}

/// How an effect's magnitude is rolled from its strike's [`Strength`].
#[derive(Clone, Copy, Debug)]
pub enum Roll {
    Fixed(u32),
    /// A spell's dice times power, raised by the boost unless `plain`, over
    /// `over` (retail `RollEffectMagnitude`).
    Dice {
        dice: Option<Dice>,
        plain: bool,
        over: u32,
    },
    /// A condition spell's rounds (retail `RollEffectDuration`).
    Duration {
        base: i32,
        per_power: i32,
    },
    /// Purify's, from power alone.
    Purify,
    /// Philosopher's Bane's percent: a second roll when the first is under
    /// 100.
    GoldShare(Option<Dice>),
    /// Anti-Magic's: fifty a power, raised by the boost.
    Fifty,
    /// So much a power.
    PerPower(u32),
    /// Three rolls under the strike's bound (its power), summed, over
    /// `divide`, plus `plus`, times `times`, over `over` (retail
    /// `RollSpecialAttackSum`).
    Three {
        divide: u32,
        plus: u32,
        times: u32,
        over: u32,
    },
    /// A monster's rounds from a special: a third of the bound.
    ThirdOfBound,
    /// A member's rounds from a special: under the bound.
    BelowBound,
    /// A blow's rounds: `count`d2 + 1, within 1 and 12, `count` being the
    /// effect power over 3, plus 1.
    BlowRounds,
    /// A blow's damage, as its [`Blow`] says.
    Damage,
    /// A blow's amount: (`count` × `times`)d`sides` + `base`, and one more.
    Blow {
        times: u32,
        sides: i32,
        base: BlowBase,
    },
}

/// What a blow's amount adds.
#[derive(Clone, Copy, Debug)]
pub enum BlowBase {
    Zero,
    /// So many times `count`.
    Count(u32),
    /// The weapon's strength.
    Weapon,
}

/// The conditions Prismic spells pick from, by weight (retail
/// `ME:251-253`); `None` is death.
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

/// The monster kinds Dispel Undead harms, and the one only Banish harms
/// besides them.
const UNDEAD: &[MonsterKind] = &[MonsterKind::Undead, MonsterKind::Demon];
const BANISHED: &[MonsterKind] = &[MonsterKind::Undead, MonsterKind::Demon, MonsterKind::Spirit];

/// What Restoration restores health and stamina by, whatever its record says.
const RESTORATION_DICE: Dice = Dice { base: 0, count: 8, sides: 8 };

/// The definitions, by what uses them; decoded once from content.
#[derive(Resource, Debug)]
pub struct Definitions {
    entries: Vec<Definition>,
    spells: HashMap<SpellId, DefinitionId>,
    backfires: HashMap<SpellId, DefinitionId>,
    pulses: HashMap<SpellId, DefinitionId>,
    specials: HashMap<SpecialAttackKind, DefinitionId>,
    breath: DefinitionId,
    blow: DefinitionId,
}

impl Definitions {
    pub fn get(&self, id: DefinitionId) -> &Definition {
        &self.entries[id.0]
    }
    pub fn spell(&self, spell: SpellId) -> Option<DefinitionId> {
        self.spells.get(&spell).copied()
    }
    pub fn backfire(&self, spell: SpellId) -> Option<DefinitionId> {
        self.backfires.get(&spell).or(self.spells.get(&spell)).copied()
    }
    pub fn pulse(&self, spell: SpellId) -> Option<DefinitionId> {
        self.pulses.get(&spell).copied()
    }
    pub fn special(&self, kind: SpecialAttackKind) -> Option<DefinitionId> {
        self.specials.get(&kind).copied()
    }
    pub fn breath(&self) -> DefinitionId {
        self.breath
    }
    pub fn blow(&self) -> DefinitionId {
        self.blow
    }
}

/// One effect of a definition: what it does, how its magnitude is rolled,
/// what may stop it, and whom it is for.
#[derive(Clone, Copy, Debug, Default)]
pub struct EffectSpec {
    pub kind: EffectKind,
    pub roll: Option<Roll>,
    /// The percent chance it happens at all.
    pub chance: Option<u32>,
    /// The body may save against it outright, from this base.
    pub save: Option<i32>,
    /// The body's resistance in this realm cuts it; saves are made in it too.
    pub resist: Option<Realm>,
    /// Saved against and resisted at half its strike's strength.
    pub half: bool,
    /// Only if the effect before it landed.
    pub landed: bool,
    /// Only if the effect before it was resisted.
    pub resisted: bool,
    /// It acts on the party itself (its shared spells, pool or purse), once,
    /// through the first member the strike reaches.
    pub party: bool,
    pub members: bool,
    pub monsters: bool,
    pub kinds: Option<&'static [MonsterKind]>,
    /// Only from a monster's blow.
    pub by_monsters: bool,
    /// The weapon effect a blow's effect stands for.
    pub weapon: Option<AttackEffect>,
}

impl EffectSpec {
    fn of(kind: EffectKind) -> Self {
        Self { kind, ..default() }
    }
    fn roll(self, roll: Roll) -> Self {
        Self { roll: Some(roll), ..self }
    }
    fn chance(self, percent: u32) -> Self {
        Self { chance: Some(percent), ..self }
    }
    fn save(self, base: i32) -> Self {
        Self { save: Some(base), ..self }
    }
    fn resist(self, realm: Realm) -> Self {
        Self { resist: Some(realm), ..self }
    }
    fn half(self) -> Self {
        Self { half: true, ..self }
    }
    fn if_landed(self) -> Self {
        Self { landed: true, ..self }
    }
    fn if_resisted(self) -> Self {
        Self { resisted: true, ..self }
    }
    fn on_party(self) -> Self {
        Self { party: true, ..self }
    }
    fn members(self) -> Self {
        Self { members: true, ..self }
    }
    fn monsters(self) -> Self {
        Self { monsters: true, ..self }
    }
    fn kinds(self, kinds: &'static [MonsterKind]) -> Self {
        Self { kinds: Some(kinds), ..self }
    }
    fn by_monsters(self) -> Self {
        Self { by_monsters: true, ..self }
    }
    fn weapon(self, effect: AttackEffect) -> Self {
        Self { weapon: Some(effect), ..self }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub enum EffectKind {
    #[default]
    Harm,
    /// A member's health drained for good.
    Sap,
    /// A condition; `indefinite` lasts until lifted, whatever its magnitude;
    /// a poison bites by `poison`.
    Afflict {
        condition: Condition,
        poison: u32,
        indefinite: bool,
    },
    Slay,
    Tire,
    DrainPool,
    DrainBudget,
    /// A member's spell points in one realm; in every realm when `None`.
    DrainRealm(Option<Realm>),
    Heal,
    Refresh,
    Replenish,
    /// Lift these conditions, by the strike's strength, or outright.
    Cure {
        conditions: &'static [Condition],
        outright: bool,
    },
    Renew,
    Raise,
    /// Shorten the clouds hanging over the body; `level` is the purifying
    /// spell's.
    Purify {
        level: u32,
    },
    /// A spell laid on the body for its magnitude's rounds.
    Enchant(SpellId),
    Burn,
    LoseGold,
    Dispel,
    /// Prismic Ray's either-or: half the time damage, otherwise a weighted
    /// condition or death.
    Prismic {
        dice: Option<Dice>,
        base: i32,
        per_power: i32,
    },
}

fn afflict(condition: Condition) -> EffectSpec {
    EffectSpec::of(EffectKind::Afflict { condition, poison: 0, indefinite: false })
}

/// A condition, rolled for so many rounds, saved against and resisted.
fn condition(afflicted: Condition, rounds: Roll, realm: Realm) -> EffectSpec {
    afflict(afflicted).roll(rounds).save(condition_save(afflicted)).resist(realm)
}

/// What a spell does to each victim, by retail's effect functions
/// (`Magic Effects.cpp`); `None` for one the game does not resolve.
fn define_spell(spell: &SpellDefinition) -> Option<Definition> {
    use Condition::*;
    use SpellId as S;
    let realm = spell.realm?;
    let dice = Roll::Dice { dice: spell.dice, plain: false, over: 1 };
    let rounds = Roll::Duration { base: spell.duration_base, per_power: spell.duration_per_power };
    let harm = EffectSpec::of(Harm).roll(dice).resist(realm);
    let inflict = |c| condition(c, rounds, realm);
    let cure = |conditions| EffectSpec::of(EffectKind::Cure { conditions, outright: false });
    let mut leeches = None;
    let effects = match spell.id {
        S::BOILING_BLOOD
        | S::BOILING_BLOOD_EXPLOSION
        | S::ACID_SPLASH
        | S::FROST
        | S::ENERGY_BLAST
        | S::MAKE_WOUNDS
        | S::MAGIC_MISSILES
        | S::SHRILL_SOUND
        | S::FIREBALL
        | S::PSIONIC_FIRE
        | S::WHIPPING_ROCKS
        | S::CRUSH
        | S::EGO_WHIP
        | S::FIRE_BOMB
        | S::ICEBALL
        | S::WHIRLWIND
        | S::DEHYDRATE
        | S::LIGHTNING
        | S::EARTHQUAKE
        | S::FALLING_STARS
        | S::MIND_FLAY
        | S::NUCLEAR_BLAST
        | S::TSUNAMI => vec![harm],
        S::LIFESTEAL | S::MIGHT_TO_MAGIC => {
            leeches = Some(spell.id == S::LIFESTEAL);
            vec![harm]
        }
        S::EYE_FOR_AN_EYE
        | S::DRACON_BREATH
        | S::SHADOW_HOUND
        | S::CHAMELEON
        | S::ACID_BOMB
        | S::DEATH_CLOUD
        | S::TOXIC_CLOUD
        | S::DRAINING_CLOUD
        | S::FIRESTORM
        | S::RAZOR_CLOAK
        | S::ARMORMELT
        | S::RING_OF_FIRE
        | S::LIGHT
        | S::X_RAY
        | S::BLESS
        | S::DETECT_SECRETS
        | S::ENCHANTED_BLADE
        | S::GUARDIAN_ANGEL
        | S::MISSILE_SHIELD
        | S::ARMORPLATE
        | S::MAGIC_SCREEN
        | S::ELEMENT_SHIELD
        | S::HASTE
        | S::SOUL_SHIELD
        | S::SUPERMAN
        | S::BODY_OF_STONE => {
            let enchant = EffectSpec::of(EffectKind::Enchant(spell.id)).roll(rounds);
            if crate::game::actors::spells::personal(spell.id) {
                vec![enchant]
            } else {
                // The party's own slots hold for all its members at once.
                vec![enchant.monsters(), enchant.on_party()]
            }
        }
        S::HOLY_WATER => vec![harm.kinds(UNDEAD)],
        S::BANISH => vec![harm.kinds(BANISHED)],
        S::HEAL_WOUNDS | S::HEAL_ALL => vec![EffectSpec::of(Heal).roll(dice)],
        S::STAMINA | S::REST_ALL => vec![EffectSpec::of(Refresh).roll(dice)],
        S::RESTORATION => {
            const ALL: &[Condition] = &[
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
            let restoring = Roll::Dice { dice: Some(RESTORATION_DICE), plain: false, over: 1 };
            vec![
                EffectSpec::of(Heal).roll(restoring),
                EffectSpec::of(Refresh).roll(restoring),
                EffectSpec::of(EffectKind::Cure { conditions: ALL, outright: true }),
            ]
        }
        S::RESTORE_HEALTH => {
            let twice = spell.dice.map(|dice| Dice { count: dice.count * 2, ..dice });
            vec![
                EffectSpec::of(Heal).roll(dice),
                EffectSpec::of(Refresh).roll(Roll::Dice { dice: twice, plain: false, over: 1 }),
            ]
        }
        S::RESTORE_MAGIC => vec![EffectSpec::of(Replenish).roll(dice)],
        S::ITCHING_SKIN => vec![inflict(Irritated)],
        S::PARALYZE | S::FREEZE_FLESH | S::FREEZE_ALL => vec![inflict(Paralyzed)],
        S::SLEEP => vec![inflict(Asleep)],
        S::BLINDING_FLASH => vec![inflict(Blind)],
        S::INSANITY => vec![inflict(Insane)],
        S::SLOW => vec![inflict(Slowed)],
        S::WEB => vec![inflict(Webbed)],
        S::SILENCE => vec![inflict(Silenced)],
        S::HEX => vec![inflict(Hexed)],
        S::TURNCOAT => vec![inflict(Turncoat)],
        S::MIND_STAB | S::PSIONIC_BLAST => vec![harm, inflict(Insane).chance(25)],
        S::BLIZZARD => vec![harm, inflict(Blind).chance(25)],
        S::CEREBRAL_HEMORRHAGE => vec![harm, inflict(Insane).chance(75)],
        S::CONCUSSION => vec![harm, inflict(Unconscious).chance(50)],
        S::NOXIOUS_FUMES => {
            vec![harm, inflict(Nauseated).chance(90), inflict(Unconscious).chance(25)]
        }
        // A member it lands on may be knocked out too, a monster frightened
        // again, at half strength.
        S::TERROR => {
            vec![inflict(Afraid), inflict(Unconscious).members().if_landed().half()]
        }
        S::SONIC_BOOM => vec![inflict(Afraid), inflict(Afraid).monsters().if_landed().half()],
        // One who keeps their mind is afraid anyway, with no save.
        S::PANDEMONIUM => {
            vec![inflict(Insane), afflict(Afraid).roll(rounds).resist(realm).if_resisted()]
        }
        S::INSTANT_DEATH | S::QUICKSAND | S::ASPHYXIATION | S::DEATH_WISH => {
            vec![EffectSpec::of(Slay).roll(rounds).save(DEATH_SAVE).resist(realm)]
        }
        S::PRISMIC_RAY | S::PRISMIC_CHAOS => {
            let prismic = EffectKind::Prismic {
                dice: spell.dice,
                base: spell.duration_base,
                per_power: spell.duration_per_power,
            };
            vec![EffectSpec::of(prismic).resist(realm)]
        }
        S::CURE_LESSER_COND => vec![cure(&[Irritated, Nauseated, Afraid, Asleep, Blind])],
        S::CURE_PARALYSIS => vec![cure(&[Paralyzed, Webbed])],
        S::CURE_POISON => vec![cure(&[Poisoned])],
        S::CURE_DISEASE => vec![cure(&[Diseased])],
        S::REMOVE_CURSE => vec![cure(&[Hexed])],
        S::SANE_MIND => vec![cure(&[Insane, Turncoat])],
        S::SMELLING_SALTS => vec![cure(&[Unconscious, Asleep])],
        S::RESURRECTION => vec![EffectSpec::of(Raise)],
        // Purify rolls from power alone; the party's clouds hang over all
        // its members at once.
        S::PURIFY_AIR => {
            let purify =
                EffectSpec::of(EffectKind::Purify { level: spell.level }).roll(Roll::Purify);
            vec![purify.monsters(), purify.on_party()]
        }
        S::PHILOSOPHERS_BANE => {
            vec![EffectSpec::of(LoseGold).roll(Roll::GoldShare(spell.dice)).on_party()]
        }
        // Members lose the plain roll, without the cast's boost, for good.
        S::VAMPIRIC_VORTEX => {
            let plain = Roll::Dice { dice: spell.dice, plain: true, over: 1 };
            vec![EffectSpec::of(Sap).roll(plain).resist(realm).members(), harm.monsters()]
        }
        S::RENEWAL => vec![EffectSpec::of(Renew)],
        // Retail visits every slot a victim carries, and the party's pool
        // once.
        S::HEATWAVE => {
            let burn = EffectSpec::of(Burn).roll(Roll::PerPower(10));
            vec![burn, burn.on_party()]
        }
        // Anti-Magic drains the whole living party, not only its victims,
        // and ends the party's lasting spells.
        S::ANTI_MAGIC => vec![
            EffectSpec::of(DrainPool).roll(Roll::Fifty).on_party(),
            EffectSpec::of(Dispel).on_party(),
        ],
        _ => return None,
    };
    Some(Definition { effects, leeches })
}

/// What a personal spell turned on its caster's side does: Dracon Breath
/// nauseates, Haste slows, Razor Cloak harms; the others nothing.
fn define_backfire(spell: &SpellDefinition) -> Vec<EffectSpec> {
    let Some(realm) = spell.realm else { return vec![] };
    let rounds = Roll::Duration { base: spell.duration_base, per_power: spell.duration_per_power };
    match spell.id {
        SpellId::DRACON_BREATH => vec![condition(Condition::Nauseated, rounds, realm)],
        SpellId::HASTE => vec![condition(Condition::Slowed, rounds, realm)],
        SpellId::RAZOR_CLOAK => vec![
            EffectSpec::of(Harm)
                .roll(Roll::Dice { dice: spell.dice, plain: false, over: 1 })
                .resist(realm),
        ],
        _ => vec![],
    }
}

/// An offensive cloud's pulse on whoever it hangs over (retail
/// `TickCombatEffectSlots`): Death Cloud makes a death save; the others
/// hurt, Draining Cloud then draining spell points and stamina by its own
/// roll, Toxic Cloud then nauseating, poisoning and knocking out.
fn define_pulse(spell: &SpellDefinition) -> Vec<EffectSpec> {
    let Some(realm) = spell.realm else { return vec![] };
    let dice = |over| Roll::Dice { dice: spell.dice, plain: false, over };
    if spell.id == SpellId::DEATH_CLOUD {
        return vec![
            EffectSpec::of(Slay).roll(Roll::Fixed(INDEFINITE)).save(DEATH_SAVE).resist(realm),
        ];
    }
    let mut effects = vec![EffectSpec::of(Harm).roll(dice(1)).resist(realm)];
    if spell.id == SpellId::DRAINING_CLOUD {
        effects.extend([
            EffectSpec::of(EffectKind::DrainRealm(None)).roll(dice(1)).members().if_landed(),
            EffectSpec::of(DrainBudget).roll(dice(2)).monsters().if_landed(),
            EffectSpec::of(Tire).roll(dice(1)).if_landed(),
        ]);
    }
    if spell.id == SpellId::TOXIC_CLOUD {
        let rounds =
            Roll::Duration { base: spell.duration_base, per_power: spell.duration_per_power };
        for afflicted in [Condition::Nauseated, Condition::Poisoned, Condition::Unconscious] {
            effects.push(condition(afflicted, rounds, realm));
        }
    }
    effects
}

/// A special attack's effects: each of its two may be escaped (the first
/// one time in ten, the second one in two), each resisted by its realm; a
/// member and a monster roll them differently.
fn define_special(definition: crate::game::hits::specials::SpecialDefinition) -> Vec<EffectSpec> {
    use crate::game::hits::specials::SpecialEffect as E;
    let realm = definition.realm;
    let three = |divide, plus, times, over| Roll::Three { divide, plus, times, over };
    let mut effects = Vec::new();
    for (slot, effect) in definition.effects.iter().enumerate() {
        let Some(effect) = *effect else { continue };
        let chance = if slot == 0 { 90 } else { 50 };
        let of = |kind, roll| EffectSpec::of(kind).roll(roll).resist(realm).chance(chance);
        match effect {
            E::Inflict(afflicted) => {
                let inflict =
                    afflict(afflicted).save(condition_save(afflicted)).resist(realm).chance(chance);
                effects.push(inflict.roll(Roll::ThirdOfBound).monsters());
                effects.push(inflict.roll(Roll::BelowBound).members());
            }
            E::Harm => effects.push(of(Harm, three(3, 1, 1, 1))),
            // A member's realm six times over, a monster's budget by half.
            E::Drain => {
                effects.push(of(DrainBudget, three(6, 1, 1, 2)).monsters());
                effects.push(of(EffectKind::DrainRealm(Some(realm)), three(3, 1, 6, 1)).members());
            }
            E::Tire => {
                effects.push(of(Tire, three(1, 0, 1, 1)).monsters());
                effects.push(of(Tire, three(1, 1, 1, 1)).members());
            }
            E::Wound if definition.resisted => effects.push(of(Harm, three(3, 1, 1, 1))),
            E::Wound => {
                effects.push(EffectSpec::of(Harm).roll(three(3, 1, 1, 1)).chance(chance));
            }
        }
    }
    effects
}

/// What a blow does: its damage, then, once that drew blood, each weapon
/// effect in turn (retail `ApplyEffectConditions`), at the weapon's chance,
/// saved against and resisted at the attacker's effect power.
fn define_blow() -> Vec<EffectSpec> {
    use AttackEffect as A;
    use Condition::*;
    let timed = |afflicted: Condition, realm, effect| {
        condition(afflicted, Roll::BlowRounds, realm).weapon(effect)
    };
    let drain = |kind, times, sides, base, realm, effect| {
        EffectSpec::of(kind)
            .roll(Roll::Blow { times, sides, base })
            .save(CONDITION_SAVE[0])
            .resist(realm)
            .weapon(effect)
    };
    vec![
        EffectSpec::of(Harm).roll(Roll::Damage),
        timed(Asleep, Realm::Air, A::Sleep),
        timed(Paralyzed, Realm::Water, A::Paralyze),
        condition(
            Poisoned,
            Roll::Blow { times: 3, sides: 2, base: BlowBase::Weapon },
            Realm::Water,
        )
        .weapon(A::Poison),
        timed(Hexed, Realm::Divine, A::Hex),
        EffectSpec::of(EffectKind::Afflict { condition: Diseased, poison: 0, indefinite: true })
            .roll(Roll::Fixed(INDEFINITE))
            .save(condition_save(Diseased))
            .resist(Realm::Divine)
            .weapon(A::Disease),
        EffectSpec::of(Slay)
            .roll(Roll::Fixed(INDEFINITE))
            .save(DEATH_SAVE)
            .resist(Realm::Divine)
            .weapon(A::Kill),
        timed(Unconscious, Realm::Earth, A::KnockOut),
        timed(Blind, Realm::Fire, A::Blind),
        timed(Afraid, Realm::Mental, A::Frighten),
        timed(Turncoat, Realm::Divine, A::Possess).members().by_monsters(),
        drain(Sap, 1, 2, BlowBase::Zero, Realm::Earth, A::DrainHealth).members(),
        drain(Tire, 2, 5, BlowBase::Count(3), Realm::Water, A::DrainStamina),
        drain(DrainPool, 1, 6, BlowBase::Count(1), Realm::Divine, A::DrainSpellPoints).members(),
        timed(Nauseated, Realm::Air, A::Nauseate),
        timed(Insane, Realm::Mental, A::Insane),
    ]
}

/// Decode the definitions once content has loaded.
pub fn define(mut commands: Commands, content: Res<Catalogs>) {
    use crate::game::hits::specials::{CHARACTER_BREATH, special};
    let mut entries = Vec::new();
    let mut add = |definition: Definition| {
        entries.push(definition);
        DefinitionId(entries.len() - 1)
    };
    let plain = |effects| Definition { effects, leeches: None };
    let mut spells = HashMap::new();
    let mut backfires = HashMap::new();
    let mut pulses = HashMap::new();
    for spell in content.spells.0.iter() {
        let Some(definition) = define_spell(spell) else { continue };
        spells.insert(spell.id, add(definition));
        if crate::game::actors::spells::personal(spell.id) {
            backfires.insert(spell.id, add(plain(define_backfire(spell))));
        }
        if crate::game::actors::spells::OFFENSIVE.contains(&spell.id)
            || spell.id == SpellId::RING_OF_FIRE
        {
            pulses.insert(spell.id, add(plain(define_pulse(spell))));
        }
    }
    let specials = (1..=u8::MAX)
        .filter_map(|byte| SpecialAttackKind::try_from(byte).ok())
        .map(|kind| (kind, add(plain(define_special(special(kind))))))
        .collect();
    let breath = add(plain(define_special(CHARACTER_BREATH)));
    let blow = add(plain(define_blow()));
    commands.insert_resource(Definitions {
        entries,
        spells,
        backfires,
        pulses,
        specials,
        breath,
        blow,
    });
}

/// What one of a spell's effects is for, as a monster weighs whether the
/// spell is worth casting at someone.
#[derive(Clone, Copy, Debug)]
enum Purpose {
    Heals,
    Refreshes,
    Afflicts(Condition),
    Enchants,
    Cures(&'static [Condition]),
}

impl Definitions {
    /// Whether a spell would do anything to `victim` (retail
    /// `MonsterSpellTargetOK`): healing only the hurt, rest only the tired,
    /// cures only what they cure, and no condition or enchantment it
    /// already has (`enchanted`: the spell is laid on it now); a spell
    /// that does none of these always would. One the
    /// game does not resolve would not. A follow-up that only comes if the
    /// effect before it landed is not weighed.
    pub fn worthwhile(
        &self,
        spell: SpellId,
        victim: &crate::game::characters::Vitals,
        enchanted: bool,
    ) -> bool {
        let Some(definition) = self.spell(spell) else { return false };
        let has = |condition| victim.conditions.has(condition);
        let mut purposes = self
            .get(definition)
            .effects
            .iter()
            .filter_map(|effect| match effect.kind {
                EffectKind::Heal => Some(Purpose::Heals),
                EffectKind::Refresh => Some(Purpose::Refreshes),
                EffectKind::Afflict { condition, .. } if !effect.landed => {
                    Some(Purpose::Afflicts(condition))
                }
                EffectKind::Enchant(_) => Some(Purpose::Enchants),
                EffectKind::Cure { conditions, .. } => Some(Purpose::Cures(conditions)),
                _ => None,
            })
            .peekable();
        purposes.peek().is_none()
            || purposes.any(|purpose| match purpose {
                Purpose::Heals => victim.health.current < victim.health.max,
                Purpose::Refreshes => victim.stamina.current < victim.stamina.max,
                Purpose::Afflicts(condition) => !has(condition),
                Purpose::Enchants => !enchanted,
                Purpose::Cures(conditions) => conditions.iter().any(|&c| has(c)),
            })
    }
}

impl EffectSpec {
    /// Prismic's either-or settled into the effect it is this time.
    pub fn pick(mut self, rng: &mut GameRng) -> Self {
        if let EffectKind::Prismic { dice, base, per_power } = self.kind {
            let roll = rng.below(100);
            if roll >= 50 {
                self.roll = Some(Roll::Dice { dice, plain: false, over: 1 });
                self.kind = EffectKind::Harm;
            } else {
                let pick = crate::game::rng::weighted(roll, AFFLICTIONS.iter().map(|&(_, w)| w))
                    .and_then(|index| AFFLICTIONS[index].0);
                self.roll = Some(Roll::Duration { base, per_power });
                self.kind = match pick {
                    Some(condition) => {
                        self.save = Some(condition_save(condition));
                        EffectKind::Afflict { condition, poison: 0, indefinite: false }
                    }
                    None => {
                        self.save = Some(DEATH_SAVE);
                        EffectKind::Slay
                    }
                };
            }
        }
        self
    }
}

/// Retail `RollEffectMagnitude`: the dice count times power, raised by the
/// boost percent.
pub fn magnitude(rng: &mut GameRng, dice: Option<Dice>, strength: Strength) -> u32 {
    let Some(dice) = dice else { return 0 };
    let count = dice.count * strength.power as i32;
    let rolled = rng.roll(Dice { count, ..dice });
    rolled + rolled * strength.boost / 100
}

/// Retail `RollEffectDuration`: a condition spell's rounds.
pub fn duration(rng: &mut GameRng, base: i32, per_power: i32, strength: Strength) -> u32 {
    let base = per_power * strength.power as i32 + base;
    if base == INDEFINITE as i32 {
        return INDEFINITE;
    }
    let mut rounds = (base + 1).max(0) as u32;
    match rng.below(4) {
        0 if rounds > 1 => rounds += 1,
        1 if rounds <= 2 => rounds += 1,
        _ => {}
    }
    rounds + rounds * strength.boost / 100
}

/// A blow's roll (retail `RollEffectDuration` on an empty definition, then
/// `count`d`sides` + `base` and one more).
fn blow_roll(rng: &mut GameRng, count: u32, sides: i32, base: u32) -> u32 {
    if rng.below(4) == 1 {
        return 2;
    }
    rng.roll(Dice { count: count as i32, sides, base: base as i32 }) + 1
}

/// Three rolls under `bound`, summed (retail `RollSpecialAttackSum`).
fn three(rng: &mut GameRng, bound: u32) -> u32 {
    rng.below(bound) + rng.below(bound) + rng.below(bound)
}

pub fn roll(roll: Roll, strength: Strength, blow: Option<&Blow>, rng: &mut GameRng) -> u32 {
    let count = strength.power / 3 + 1;
    match roll {
        Roll::Fixed(amount) => amount,
        Roll::Dice { dice, plain, over } => {
            let strength = if plain { Strength { boost: 0, ..strength } } else { strength };
            magnitude(rng, dice, strength) / over
        }
        Roll::Duration { base, per_power } => duration(rng, base, per_power, strength),
        Roll::Purify => {
            let base = strength.power + 1;
            let amount = match rng.below(4) {
                0 if base > 1 => base + 1,
                1 if base < 3 => base + 1,
                _ => base,
            };
            amount + amount * strength.boost / 100
        }
        // Retail's second roll when the first is below 100 is intentional.
        Roll::GoldShare(dice) => match magnitude(rng, dice, strength) {
            first if first < 100 => magnitude(rng, dice, strength).min(100),
            _ => 100,
        },
        Roll::Fifty => {
            let base = strength.power * 50;
            base + base * strength.boost / 100
        }
        Roll::PerPower(each) => strength.power * each,
        Roll::Three { divide, plus, times, over } => {
            (three(rng, strength.power) / divide + plus) * times / over
        }
        Roll::ThirdOfBound => (strength.power / 3).max(1),
        Roll::BelowBound => rng.below(strength.power),
        Roll::Damage => blow.map_or(0, |blow| blow.damage),
        Roll::BlowRounds => blow_roll(rng, count, 2, 1).clamp(1, 12),
        Roll::Blow { times, sides, base } => {
            let base = match base {
                BlowBase::Zero => 0,
                BlowBase::Count(times) => count * times,
                BlowBase::Weapon => blow.map_or(0, |blow| blow.effects.strength),
            };
            blow_roll(rng, count * times, sides, base)
        }
    }
}
