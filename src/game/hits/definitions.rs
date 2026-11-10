//! Immutable spell and weapon descriptions, decoded once from retail catalogs.

use std::collections::HashMap;

use bevy::prelude::*;
use serde::Deserialize;

use super::defences::{CONDITION_SAVE, DEATH_SAVE, INDEFINITE, condition_save};
use crate::content::characters::Realm;
use crate::content::items::{AttackEffect, AttackEffects, Dice};
use crate::content::species::{MonsterKind, SpecialAttackKind};
use crate::content::spells::{SpellDefinition, SpellId};
use crate::game::characters::Condition;
use crate::game::data::Catalogs;
use crate::game::magic::Strength;
use crate::game::rng::GameRng;
use EffectKind::{DrainBudget, DrainPool, Harm, Sap, Slay, Tire};

/// What something does to each body it lands on: its effects in order, and
/// whether its damage comes back to its caster's side (`Some(true)` as
/// health, `Some(false)` as spell points).
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Definition {
    pub effects: Vec<EffectSpec>,
    pub leeches: Option<bool>,
    /// What an auto-powered cast must outlast: the conditions it lifts that
    /// it weighs in choosing its power.
    pub outlasts: Vec<Condition>,
    /// What an auto-powered cast restores, if it is a restoring spell.
    pub restores: Option<Restores>,
    /// A repeated cast goes on while a victim is short of stamina rather than
    /// of health.
    pub watches_stamina: bool,
    /// Laid, it takes damage in a victim's place; the damage it takes is
    /// rolled as it is laid.
    pub absorbs: bool,
    /// Laid, it first spends itself against a slowing.
    pub answers_slowing: bool,
    /// What each of its victims it killed bursts into.
    pub bursts_into: Option<SpellId>,
    /// Laid, it turns back the spells aimed at enemies that land on its
    /// bearer.
    pub turns_back: bool,
    /// Cast with Power Cast on a summoned elemental of this element, it may
    /// be laid on the elemental.
    pub buffs_elemental: Option<Realm>,
}

/// What a restoring spell mends, as its auto power measures it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum Restores {
    Health,
    Stamina,
    /// Health; stamina once health is full.
    HealthThenStamina,
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
///
/// The `Spell…` rolls are how a table row says "this spell's own": they are
/// written in the spell effects table, and become the plain roll when the
/// spell's effects are made ([`Definition::of_spell`]); none is ever rolled.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub enum Roll {
    /// This spell's dice (see [`Roll::Dice`]).
    SpellDice,
    /// This spell's dice, their count multiplied by `times`, without the
    /// cast's boost when `plain`, over `over`.
    SpellDiceWith {
        #[serde(default)]
        plain: bool,
        #[serde(default = "one::<i32>")]
        times: i32,
        #[serde(default = "one::<u32>")]
        over: u32,
    },
    /// This spell's rounds (see [`Roll::Duration`]).
    SpellRounds,
    /// This spell's dice as a gold share (see [`Roll::GoldShare`]).
    SpellGoldShare,
    Fixed(u32),
    /// A spell's dice times power, raised by the boost unless `plain`, over
    /// `over` (retail `RollEffectMagnitude`).
    Dice {
        dice: Option<Dice>,
        #[serde(default)]
        plain: bool,
        #[serde(default = "one::<u32>")]
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

fn one<T: From<u8>>() -> T {
    T::from(1)
}

/// What a blow's amount adds.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
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

/// Whom an effect lands on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
pub enum Whom {
    /// Every body the strike reaches, members and monsters.
    #[default]
    Everyone,
    Members,
    Monsters,
    /// The party itself (its shared spells, pool or purse), once, through
    /// the first member the strike reaches.
    Party,
}

/// One effect of a definition: what it does, how its magnitude is rolled,
/// what may stop it, and whom it is for.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EffectSpec {
    pub kind: EffectKind,
    pub on: Whom,
    pub roll: Option<Roll>,
    /// The percent chance it happens at all.
    pub chance: Option<u32>,
    /// The body may save against it outright, from this base.
    pub save: Option<i32>,
    /// The body's resistance in this realm cuts it; saves are made in it too.
    pub resist: Option<Realm>,
    /// Resisted in the realm of the spell it belongs to: how a table row
    /// says `resist` for its spell, made [`resist`](Self::resist) when the
    /// spell's effects are made, and false in every definition.
    pub spell_realm: bool,
    /// Saved against and resisted at half its strike's strength, and rolled
    /// for half the rounds.
    pub half: bool,
    /// Only if the effect before it landed.
    pub landed: bool,
    /// Only if the effect before it was resisted.
    pub resisted: bool,
    /// Only the monsters of these kinds.
    pub kinds: Option<Vec<MonsterKind>>,
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
    fn members(self) -> Self {
        Self { on: Whom::Members, ..self }
    }
    fn monsters(self) -> Self {
        Self { on: Whom::Monsters, ..self }
    }
    fn by_monsters(self) -> Self {
        Self { by_monsters: true, ..self }
    }
    fn weapon(self, effect: AttackEffect) -> Self {
        Self { weapon: Some(effect), ..self }
    }
}

/// What an effect does. The `Spell…` kinds are how a table row says "this
/// spell's own", as the [`Roll`]s do; none is ever applied.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub enum EffectKind {
    #[default]
    Harm,
    /// A member's health drained for good.
    Sap,
    /// A condition; `indefinite` lasts until lifted, whatever its magnitude;
    /// a poison bites by `poison`.
    Afflict {
        condition: Condition,
        #[serde(default)]
        poison: u32,
        #[serde(default)]
        indefinite: bool,
    },
    /// One of these conditions, picked at random for each body, afflicted
    /// as [`EffectKind::Afflict`] is, saved against by its own base.
    AfflictAny(Vec<Condition>),
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
        conditions: Vec<Condition>,
        #[serde(default)]
        outright: bool,
    },
    Renew,
    Raise,
    /// Shorten the clouds hanging over the body; `level` is the purifying
    /// spell's.
    Purify {
        level: u32,
    },
    /// This spell's purifying (see [`EffectKind::Purify`]).
    SpellPurify,
    /// A spell laid on the body for its magnitude's rounds.
    Enchant(SpellId),
    /// This spell, laid on the body (see [`EffectKind::Enchant`]).
    SpellEnchant,
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
    /// This spell's either-or (see [`EffectKind::Prismic`]).
    SpellPrismic,
}

fn afflict(condition: Condition) -> EffectSpec {
    EffectSpec::of(EffectKind::Afflict { condition, poison: 0, indefinite: false })
}

/// A condition, rolled for so many rounds, saved against and resisted.
fn condition(afflicted: Condition, rounds: Roll, realm: Realm) -> EffectSpec {
    afflict(afflicted).roll(rounds).save(condition_save(afflicted)).resist(realm)
}

/// One spell's row of the table: what it does when cast, and, for a personal
/// spell, what it does turned on its caster, and for one that pulses, what
/// each pulse does.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpellRow {
    pub effects: Vec<EffectSpec>,
    pub leeches: Option<bool>,
    pub outlasts: Vec<Condition>,
    pub restores: Option<Restores>,
    pub watches_stamina: bool,
    pub absorbs: bool,
    pub answers_slowing: bool,
    pub bursts_into: Option<SpellId>,
    pub turns_back: bool,
    pub buffs_elemental: Option<Realm>,
    pub backfire: Vec<EffectSpec>,
    pub pulse: Vec<EffectSpec>,
}

/// What each spell does to each victim, by spell: the table in
/// `assets/tables/spell_effects.ron`, which ships with the game (retail keeps
/// it in its program, not its data). A spell it does not name does nothing
/// the game resolves.
#[derive(Debug, Deserialize)]
#[serde(transparent)]
pub struct SpellEffects(HashMap<SpellId, SpellRow>);

impl SpellEffects {
    pub fn get(&self, spell: SpellId) -> Option<&SpellRow> {
        self.0.get(&spell)
    }
}

/// The shipped table: it is part of the program's content, read where the
/// catalogs are made, so a catalog built by hand has it too.
impl Default for SpellEffects {
    fn default() -> Self {
        ron::from_str(include_str!("../../../assets/tables/spell_effects.ron"))
            .expect("assets/tables/spell_effects.ron is a spell effects table")
    }
}

impl SpellRow {
    /// What the spell does when cast, made the spell's own: each `Spell…`
    /// roll, kind and realm in it resolved from the spell's record. `None`
    /// for a spell with no realm, which the game does not resolve.
    pub fn of_spell(&self, spell: &SpellDefinition) -> Option<Definition> {
        Some(Definition {
            effects: made(&self.effects, spell)?,
            leeches: self.leeches,
            outlasts: self.outlasts.clone(),
            restores: self.restores,
            watches_stamina: self.watches_stamina,
            absorbs: self.absorbs,
            answers_slowing: self.answers_slowing,
            bursts_into: self.bursts_into,
            turns_back: self.turns_back,
            buffs_elemental: self.buffs_elemental,
        })
    }

    /// What it does turned on its caster.
    pub fn backfire_of(&self, spell: &SpellDefinition) -> Option<Definition> {
        Some(Definition { effects: made(&self.backfire, spell)?, ..default() })
    }

    /// What each of its pulses does.
    pub fn pulse_of(&self, spell: &SpellDefinition) -> Option<Definition> {
        Some(Definition { effects: made(&self.pulse, spell)?, ..default() })
    }
}

/// A row's effects made the spell's own.
fn made(effects: &[EffectSpec], spell: &SpellDefinition) -> Option<Vec<EffectSpec>> {
    let realm = spell.realm?;
    Some(effects.iter().map(|effect| effect.of_spell(spell, realm)).collect())
}

impl EffectSpec {
    fn of_spell(&self, spell: &SpellDefinition, realm: Realm) -> EffectSpec {
        let roll = self.roll.map(|roll| match roll {
            Roll::SpellDice => Roll::Dice { dice: spell.dice, plain: false, over: 1 },
            Roll::SpellDiceWith { plain, times, over } => Roll::Dice {
                dice: spell.dice.map(|dice| Dice { count: dice.count * times, ..dice }),
                plain,
                over,
            },
            Roll::SpellRounds => {
                Roll::Duration { base: spell.duration_base, per_power: spell.duration_per_power }
            }
            Roll::SpellGoldShare => Roll::GoldShare(spell.dice),
            roll => roll,
        });
        let kind = match self.kind {
            EffectKind::SpellPurify => EffectKind::Purify { level: spell.level },
            EffectKind::SpellEnchant => EffectKind::Enchant(spell.id),
            EffectKind::SpellPrismic => EffectKind::Prismic {
                dice: spell.dice,
                base: spell.duration_base,
                per_power: spell.duration_per_power,
            },
            _ => self.kind.clone(),
        };
        let resist = if self.spell_realm { Some(realm) } else { self.resist };
        EffectSpec { kind, roll, resist, spell_realm: false, ..self.clone() }
    }
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
                effects.push(inflict.clone().roll(Roll::ThirdOfBound).monsters());
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
    let plain = |effects| Definition { effects, ..default() };
    let mut spells = HashMap::new();
    let mut backfires = HashMap::new();
    let mut pulses = HashMap::new();
    for spell in content.spells.0.iter() {
        let Some(row) = content.spell_effects.get(spell.id) else { continue };
        let Some(definition) = row.of_spell(spell) else { continue };
        spells.insert(spell.id, add(definition));
        if (spell.facts.personal || !row.backfire.is_empty())
            && let Some(backfire) = row.backfire_of(spell)
        {
            backfires.insert(spell.id, add(backfire));
        }
        if spell.facts.pulse.is_some()
            && let Some(pulse) = row.pulse_of(spell)
        {
            pulses.insert(spell.id, add(pulse));
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
enum Purpose<'a> {
    Heals,
    Refreshes,
    Afflicts(Condition),
    Enchants,
    Cures(&'a [Condition]),
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
            .filter_map(|effect| match &effect.kind {
                EffectKind::Heal => Some(Purpose::Heals),
                EffectKind::Refresh => Some(Purpose::Refreshes),
                EffectKind::Afflict { condition, .. } if !effect.landed => {
                    Some(Purpose::Afflicts(*condition))
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
        if let EffectKind::AfflictAny(conditions) = &self.kind {
            let condition = conditions[rng.below(conditions.len() as u32) as usize];
            self.save = Some(condition_save(condition));
            self.kind = EffectKind::Afflict { condition, poison: 0, indefinite: false };
        }
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
        Roll::SpellDice | Roll::SpellDiceWith { .. } | Roll::SpellRounds | Roll::SpellGoldShare => {
            unreachable!("a spell's own roll is made plain with its effects")
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    const REALM: Realm = Realm::Air;
    const DICE: Dice = Dice { base: 1, count: 2, sides: 6 };

    /// A spell record with dice, level, realm and rounds of its own.
    fn spell(id: SpellId) -> SpellDefinition {
        SpellDefinition {
            id,
            realm: Some(REALM),
            level: 4,
            dice: Some(DICE),
            duration_base: 2,
            duration_per_power: 3,
            ..default()
        }
    }

    /// What the table makes of a spell, with its own numbers.
    fn made(id: SpellId) -> Definition {
        SpellEffects::default().get(id).unwrap().of_spell(&spell(id)).unwrap()
    }

    fn dice() -> Roll {
        Roll::Dice { dice: Some(DICE), plain: false, over: 1 }
    }

    fn rounds() -> Roll {
        Roll::Duration { base: 2, per_power: 3 }
    }

    #[test]
    fn a_plain_harm_is_its_dice_resisted_in_its_realm() {
        assert_eq!(
            made(SpellId::ENERGY_BLAST).effects,
            vec![EffectSpec { roll: Some(dice()), resist: Some(REALM), ..default() }]
        );
    }

    #[test]
    fn a_condition_is_its_rounds_saved_against_and_resisted() {
        assert_eq!(
            made(SpellId::SLEEP).effects,
            vec![EffectSpec {
                kind: EffectKind::Afflict {
                    condition: Condition::Asleep,
                    poison: 0,
                    indefinite: false
                },
                roll: Some(rounds()),
                save: Some(condition_save(Condition::Asleep)),
                resist: Some(REALM),
                ..default()
            }]
        );
    }

    #[test]
    fn a_follow_up_waits_on_the_effect_before_it() {
        let terror = made(SpellId::TERROR).effects;
        assert_eq!(terror.len(), 3);
        let knocked_out = &terror[1];
        assert_eq!(knocked_out.on, Whom::Members);
        assert!(knocked_out.landed && knocked_out.half);
        assert_eq!(
            knocked_out.kind,
            EffectKind::Afflict { condition: Condition::Unconscious, poison: 0, indefinite: false }
        );
        // A monster that did not resist is frightened a second time, at half
        // strength and for half as long (`Magic Effects.cpp:1923-1928`).
        let frightened = &terror[2];
        assert_eq!(frightened.on, Whom::Monsters);
        assert!(frightened.landed && frightened.half);
        assert_eq!(
            frightened.kind,
            EffectKind::Afflict { condition: Condition::Afraid, poison: 0, indefinite: false }
        );
    }

    #[test]
    fn a_cure_names_what_it_lifts() {
        assert_eq!(
            made(SpellId::CURE_PARALYSIS).effects,
            vec![EffectSpec {
                kind: EffectKind::Cure {
                    conditions: vec![Condition::Paralyzed, Condition::Webbed],
                    outright: false
                },
                ..default()
            }]
        );
    }

    #[test]
    fn a_party_wide_effect_lands_on_the_party_once() {
        assert_eq!(
            made(SpellId::ANTI_MAGIC).effects,
            vec![
                EffectSpec {
                    kind: EffectKind::DrainPool,
                    on: Whom::Party,
                    roll: Some(Roll::Fifty),
                    ..default()
                },
                EffectSpec { kind: EffectKind::Dispel, on: Whom::Party, ..default() },
            ]
        );
    }

    #[test]
    fn a_spell_laid_on_monsters_and_the_party_is_two_rows_and_a_personal_one_is_one() {
        let laid = |on| EffectSpec {
            kind: EffectKind::Enchant(SpellId::BLESS),
            on,
            roll: Some(rounds()),
            ..default()
        };
        assert_eq!(made(SpellId::BLESS).effects, vec![laid(Whom::Monsters), laid(Whom::Party)]);
        assert_eq!(made(SpellId::HASTE).effects.len(), 1);
    }

    #[test]
    fn a_spell_s_own_numbers_are_taken_from_its_record() {
        let restore = made(SpellId::RESTORE_HEALTH).effects;
        assert_eq!(restore[0].roll, Some(dice()));
        assert_eq!(
            restore[1].roll,
            Some(Roll::Dice {
                dice: Some(Dice { count: DICE.count * 2, ..DICE }),
                plain: false,
                over: 1
            })
        );
        assert_eq!(
            made(SpellId::PRISMIC_RAY).effects[0].kind,
            EffectKind::Prismic { dice: Some(DICE), base: 2, per_power: 3 }
        );
        assert_eq!(made(SpellId::PURIFY_AIR).effects[0].kind, EffectKind::Purify { level: 4 });
        assert_eq!(
            made(SpellId::PHILOSOPHERS_BANE).effects[0].roll,
            Some(Roll::GoldShare(Some(DICE)))
        );
        let vortex = made(SpellId::VAMPIRIC_VORTEX).effects;
        assert_eq!(vortex[0].roll, Some(Roll::Dice { dice: Some(DICE), plain: true, over: 1 }));
    }

    #[test]
    fn who_the_dead_and_the_undead_are_is_in_the_table() {
        assert_eq!(
            made(SpellId::BANISH).effects[0].kinds,
            Some(vec![MonsterKind::Undead, MonsterKind::Demon, MonsterKind::Spirit])
        );
        assert_eq!(made(SpellId::LIFESTEAL).leeches, Some(true));
        assert_eq!(made(SpellId::MIGHT_TO_MAGIC).leeches, Some(false));
    }

    #[test]
    fn every_spell_the_table_names_has_effects_and_resolves() {
        let table = SpellEffects::default();
        assert!(!table.0.is_empty());
        for (&id, rows) in &table.0 {
            assert_ne!(id, SpellId::NONE);
            assert!(!rows.effects.is_empty(), "{id:?}");
            let made = rows.of_spell(&spell(id)).unwrap();
            assert!(made.effects.iter().all(|effect| !effect.spell_realm), "{id:?}");
        }
    }

    /// What a personal spell does turned on its caster, and what a pulse
    /// does, are in the spell's row.
    #[test]
    fn backfires_and_pulses_are_rows() {
        let table = SpellEffects::default();
        let row = |id| table.get(id).unwrap();
        let backfire = |id| row(id).backfire_of(&spell_with_facts(id)).unwrap().effects;
        let pulse = |id| row(id).pulse_of(&spell_with_facts(id)).unwrap().effects;
        let afflicts = |effect: &EffectSpec| match effect.kind {
            EffectKind::Afflict { condition, .. } => Some(condition),
            _ => None,
        };
        assert_eq!(afflicts(&backfire(SpellId::DRACON_BREATH)[0]), Some(Condition::Nauseated));
        let slowed = &backfire(SpellId::HASTE)[0];
        assert_eq!((afflicts(slowed), slowed.roll), (Some(Condition::Slowed), Some(rounds())));
        assert_eq!(
            backfire(SpellId::RAZOR_CLOAK),
            vec![EffectSpec { roll: Some(dice()), resist: Some(REALM), ..default() }]
        );
        assert!(backfire(SpellId::GUARDIAN_ANGEL).is_empty());
        // The friendly spells that can backfire turn ill instead of well
        // (`Magic Effects.cpp:3246-3260, 3261-3283, 3378-3385, 3436-3441,
        // 3467-3475`): tiring, one of five afflictions, paralysis, insanity
        // and a hex.
        for id in [SpellId::STAMINA, SpellId::REST_ALL] {
            assert_eq!(
                backfire(id),
                vec![EffectSpec { kind: Tire, roll: Some(dice()), ..default() }],
                "{id:?}"
            );
        }
        let any = &backfire(SpellId::CURE_LESSER_COND)[0];
        assert_eq!(
            any.kind,
            EffectKind::AfflictAny(vec![
                Condition::Irritated,
                Condition::Nauseated,
                Condition::Afraid,
                Condition::Asleep,
                Condition::Blind
            ])
        );
        assert_eq!((any.roll, any.resist), (Some(rounds()), Some(REALM)));
        for (id, condition) in [
            (SpellId::CURE_PARALYSIS, Condition::Paralyzed),
            (SpellId::SANE_MIND, Condition::Insane),
            (SpellId::REMOVE_CURSE, Condition::Hexed),
        ] {
            let ill = &backfire(id)[0];
            assert_eq!(afflicts(ill), Some(condition), "{id:?}");
            assert_eq!(ill.save, Some(condition_save(condition)), "{id:?}");
            assert_eq!((ill.roll, ill.resist), (Some(rounds()), Some(REALM)), "{id:?}");
        }
        let death = pulse(SpellId::DEATH_CLOUD);
        assert_eq!(
            death,
            vec![EffectSpec {
                kind: EffectKind::Slay,
                roll: Some(Roll::Fixed(INDEFINITE)),
                save: Some(DEATH_SAVE),
                resist: Some(REALM),
                ..default()
            }]
        );
        let draining = pulse(SpellId::DRAINING_CLOUD);
        assert_eq!(draining.len(), 4);
        assert!(draining[1..].iter().all(|effect| effect.landed && effect.resist.is_none()));
        assert_eq!(draining[1].on, Whom::Members);
        assert_eq!(draining[2].roll, Some(Roll::Dice { dice: Some(DICE), plain: false, over: 2 }));
        let toxic = pulse(SpellId::TOXIC_CLOUD);
        assert_eq!(
            toxic[1..].iter().map(afflicts).collect::<Vec<_>>(),
            [Some(Condition::Nauseated), Some(Condition::Poisoned), Some(Condition::Unconscious)]
        );
        assert_eq!(
            toxic[2].save,
            Some(condition_save(Condition::Poisoned)),
            "a row's save is the condition's own"
        );
        assert_eq!(pulse(SpellId::RING_OF_FIRE).len(), 1);
    }

    /// A row has a backfire only for a spell that is personal or can backfire,
    /// a pulse only for one that pulses.
    #[test]
    fn a_backfire_or_a_pulse_is_for_a_spell_that_has_one() {
        let table = SpellEffects::default();
        for id in 0..=255 {
            let spell = spell_with_facts(SpellId(id));
            let Some(row) = table.get(spell.id) else { continue };
            assert!(
                spell.facts.personal || spell.can_backfire() || row.backfire.is_empty(),
                "{:?}",
                spell.id
            );
            assert_eq!(spell.facts.pulse.is_some(), !row.pulse.is_empty(), "{:?}", spell.id);
        }
    }

    /// What a spell is beyond its effects is in its row: what its auto power
    /// measures or outlasts, which it watches, what it absorbs, answers, bursts
    /// into, turns back or buffs.
    #[test]
    fn a_row_says_what_a_spell_is_beyond_its_effects() {
        let table = SpellEffects::default();
        let row = |id| table.get(id).unwrap().of_spell(&spell(id)).unwrap();
        assert_eq!(row(SpellId::HEAL_WOUNDS).restores, Some(Restores::Health));
        assert_eq!(row(SpellId::STAMINA).restores, Some(Restores::Stamina));
        assert_eq!(row(SpellId::RESTORATION).restores, Some(Restores::HealthThenStamina));
        assert!(row(SpellId::STAMINA).watches_stamina && row(SpellId::REST_ALL).watches_stamina);
        assert!(!row(SpellId::HEAL_ALL).watches_stamina);
        assert_eq!(
            row(SpellId::SANE_MIND).outlasts,
            [Condition::Insane, Condition::Turncoat, Condition::Asleep]
        );
        assert_eq!(row(SpellId::CURE_POISON).outlasts, [Condition::Poisoned]);
        assert!(row(SpellId::HEAL_WOUNDS).outlasts.is_empty());
        assert!(row(SpellId::GUARDIAN_ANGEL).absorbs && row(SpellId::HASTE).answers_slowing);
        assert_eq!(row(SpellId::BOILING_BLOOD).bursts_into, Some(SpellId::BOILING_BLOOD_EXPLOSION));
        assert!(row(SpellId::EYE_FOR_AN_EYE).turns_back);
        assert_eq!(
            [SpellId::HASTE, SpellId::SUPERMAN, SpellId::MISSILE_SHIELD, SpellId::ARMORPLATE]
                .map(|id| row(id).buffs_elemental),
            [Some(Realm::Fire), Some(Realm::Water), Some(Realm::Air), Some(Realm::Earth)]
        );
    }

    fn rolled(roll: Roll, power: u32, boost: u32, seed: u64) -> u32 {
        let strength = Strength { power, against: 0, boost };
        super::roll(roll, strength, None, &mut GameRng::seeded(seed))
    }

    /// Dice of 1 side are certain: 3 dice a power at power 2 are 6 dice, each 1,
    /// and the base 2 makes 8; raised by 50 percent that is 8 + 4 = 12; plain, 8;
    /// over 2, 4. No dice make nothing.
    #[test]
    fn a_roll_of_the_spells_dice_is_count_times_power_dice_plus_base_boosted() {
        let dice = Some(Dice { base: 2, count: 3, sides: 1 });
        let with = |plain, over| Roll::Dice { dice, plain, over };
        assert_eq!(rolled(with(false, 1), 2, 0, 0), 8);
        assert_eq!(rolled(with(false, 1), 2, 50, 0), 12);
        assert_eq!(rolled(with(true, 1), 2, 50, 0), 8);
        assert_eq!(rolled(with(false, 2), 2, 0, 0), 4);
        assert_eq!(rolled(Roll::Dice { dice: None, plain: false, over: 1 }, 2, 50, 0), 0);
    }

    /// Rounds are `per power * power + base` plus one, then below(4): on 0 one
    /// more if over 1; on 1 one more if 2 or under. Power 2 of 3 + 2 is 8, 9 with
    /// the one, so 9 or 10; of 0 + 0 it is 1 or 2; of 0 + 1 (2) it is 2 or 3. Raised
    /// by the boost: 50 percent of 9 is 4. 9999 is permanent.
    #[test]
    fn a_condition_lasts_its_rounds_with_a_little_luck_and_a_boost() {
        let rounds = |per_power, base, power, boost| -> std::collections::BTreeSet<u32> {
            (0..200)
                .map(|seed| rolled(Roll::Duration { base, per_power }, power, boost, seed))
                .collect()
        };
        assert_eq!(rounds(3, 2, 2, 0), [9, 10].into());
        assert_eq!(rounds(0, 0, 1, 0), [1, 2].into());
        assert_eq!(rounds(0, 1, 1, 0), [2, 3].into());
        assert_eq!(rounds(3, 2, 2, 50), [13, 15].into());
        assert_eq!(rounds(0, 9999, 1, 50), [9999].into());
        assert_eq!(rounds(0, -5, 1, 0), [0, 1].into(), "never under nothing, then the luck");
    }

    /// Purify: power + 1, then below(4): on 0 one more if over 1, on 1 one more
    /// if under 3. Power 1 (2): 3 on a 0 or 1; 2 otherwise. Power 3 (4): 5 or 4.
    /// Raised by the boost.
    #[test]
    fn purifying_rolls_from_the_power_alone() {
        let turns = |power, boost| -> std::collections::BTreeSet<u32> {
            (0..200).map(|seed| rolled(Roll::Purify, power, boost, seed)).collect()
        };
        assert_eq!(turns(1, 0), [2, 3].into());
        assert_eq!(turns(3, 0), [4, 5].into());
        assert_eq!(turns(3, 100), [8, 10].into());
    }

    /// Fifty a power raised by the boost; so much a power; a fixed number; a
    /// third of the power but one at least; below the power.
    #[test]
    fn the_simple_rolls_are_fixed_by_the_power() {
        assert_eq!(rolled(Roll::Fifty, 3, 0, 0), 150);
        assert_eq!(rolled(Roll::Fifty, 3, 20, 0), 180);
        assert_eq!(rolled(Roll::PerPower(10), 4, 99, 0), 40);
        assert_eq!(rolled(Roll::Fixed(7), 4, 99, 0), 7);
        assert_eq!(
            (rolled(Roll::ThirdOfBound, 1, 0, 0), rolled(Roll::ThirdOfBound, 7, 0, 0)),
            (1, 2)
        );
        assert!((0..50).all(|seed| rolled(Roll::BelowBound, 3, 0, seed) < 3));
    }

    /// A gold share is the dice at the power; if that is under 100, a second roll
    /// is taken in its place, held at 100; if it is 100 or over, 100.
    #[test]
    fn a_gold_share_is_the_dice_rolled_again_when_under_a_hundred_and_held_at_a_hundred() {
        let share = |base| Roll::GoldShare(Some(Dice { base, count: 1, sides: 1 }));
        assert_eq!(rolled(share(30), 1, 0, 0), 31, "the second roll, 1d1 + 30");
        assert_eq!(rolled(share(99), 1, 0, 0), 100, "the second, 100, held");
        assert_eq!(rolled(share(150), 1, 0, 0), 100, "not under 100, so 100");
        assert_eq!(rolled(Roll::GoldShare(None), 1, 0, 0), 0);
    }

    /// Three rolls below the power, summed, over `divide`, plus `plus`, times
    /// `times`, over `over`; at power 1 each is 0.
    #[test]
    fn a_special_attacks_sum_is_three_rolls_under_the_power() {
        let sum = Roll::Three { divide: 3, plus: 1, times: 4, over: 2 };
        assert_eq!(rolled(sum, 1, 0, 0), 2, "(0 / 3 + 1) * 4 / 2");
        assert!((0..100).all(|seed| (2..=6).contains(&rolled(sum, 3, 0, seed))));
    }

    /// Prismic: below(100) from 50 up is damage by the dice; under 50 a condition
    /// by weight (Nauseated 10, Afraid 10, Hexed 5, Insane 5, Blind 5, Turncoat 1,
    /// Asleep 10, Paralyzed 3) or death (1) of fifty, each with its own save.
    #[test]
    fn a_prismic_ray_is_damage_half_the_time_and_else_a_weighted_condition_or_death() {
        let ray = EffectSpec {
            kind: EffectKind::Prismic { dice: Some(DICE), base: 2, per_power: 3 },
            ..default()
        };
        let mut rng = GameRng::seeded(21);
        let picks: Vec<EffectSpec> = (0..20_000).map(|_| ray.clone().pick(&mut rng)).collect();
        let count = |f: &dyn Fn(&EffectSpec) -> bool| picks.iter().filter(|p| f(p)).count();
        let harm = count(&|p| p.kind == EffectKind::Harm);
        assert!((9_700..=10_300).contains(&harm), "{harm}");
        let with = |condition: Condition| {
            count(&|p| matches!(p.kind, EffectKind::Afflict { condition: c, .. } if c == condition))
        };
        // Of 20,000: 10 percent is 2,000; 5 percent 1,000; 1 percent 200.
        assert!((1_800..=2_200).contains(&with(Condition::Nauseated)));
        assert!((1_800..=2_200).contains(&with(Condition::Asleep)));
        assert!((800..=1_200).contains(&with(Condition::Hexed)));
        assert!((100..=300).contains(&with(Condition::Turncoat)));
        let death = count(&|p| p.kind == EffectKind::Slay);
        assert!((100..=300).contains(&death), "{death}");
        for pick in &picks {
            match pick.kind {
                EffectKind::Harm => assert_eq!(pick.roll, Some(dice())),
                EffectKind::Slay => assert_eq!(pick.save, Some(DEATH_SAVE)),
                EffectKind::Afflict { condition, .. } => {
                    assert_eq!(pick.save, Some(condition_save(condition)));
                    assert_eq!(pick.roll, Some(rounds()));
                }
                _ => panic!("{pick:?}"),
            }
        }
    }

    /// An either-or condition is one of its list, at random, saved against by its own base.
    #[test]
    fn an_either_or_condition_is_one_of_its_list_with_its_own_save() {
        let any = EffectSpec {
            kind: EffectKind::AfflictAny(vec![Condition::Blind, Condition::Asleep]),
            ..default()
        };
        let mut rng = GameRng::seeded(22);
        let picks: Vec<EffectSpec> = (0..400).map(|_| any.clone().pick(&mut rng)).collect();
        for pick in &picks {
            let EffectKind::Afflict { condition, .. } = pick.kind else { panic!() };
            assert!([Condition::Blind, Condition::Asleep].contains(&condition));
            assert_eq!(pick.save, Some(condition_save(condition)));
        }
        let blind = picks
            .iter()
            .filter(|p| matches!(p.kind, EffectKind::Afflict { condition: Condition::Blind, .. }))
            .count();
        assert!((140..=260).contains(&blind), "{blind}");
    }

    fn spell_with_facts(id: SpellId) -> SpellDefinition {
        crate::content::spells::SpellCatalog::new(vec![spell(id)]).0.remove(0)
    }
}
