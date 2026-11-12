//! The rows that say what a spell, a special attack or a blow does to each
//! body it lands on, and how a row's magnitude is rolled. The rows are
//! tables under `assets/tables/`; a row that says "the spell's own" is read
//! against the spell as it lands.

use std::collections::HashMap;
use std::sync::LazyLock;

use bevy::prelude::*;
use serde::Deserialize;

use super::defences::{DEATH_SAVE, INDEFINITE, condition_save};
use crate::content::characters::Realm;
use crate::content::items::{AttackEffect, AttackEffects, Dice};
use crate::content::species::{MonsterKind, SpecialAttackKind};
use crate::content::spells::{SpellDefinition, SpellId};
use crate::game::characters::Condition;
use crate::game::data::Catalogs;
use crate::game::magic::Strength;
use crate::game::rng::GameRng;

/// Whose rows a strike follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rows {
    Spell(SpellId),
    /// A spell turned on its caster's side.
    Backfire(SpellId),
    /// A spell laid that pulses as a round ends.
    Pulse(SpellId),
    Special(SpecialAttackKind),
    /// A weapon's blow.
    Blow,
}

/// What a blow does, from the table the game ships.
static WEAPON_EFFECTS: LazyLock<Vec<EffectSpec>> = LazyLock::new(|| {
    ron::from_str(include_str!("../../../assets/tables/weapon_effects.ron"))
        .expect("assets/tables/weapon_effects.ron is a table of effects")
});

impl Rows {
    /// The spell they are of.
    pub fn spell(self) -> Option<SpellId> {
        match self {
            Self::Spell(spell) | Self::Backfire(spell) | Self::Pulse(spell) => Some(spell),
            Self::Special(_) | Self::Blow => None,
        }
    }

    /// The rows themselves, in order; none for a spell the table does not
    /// name.
    pub fn effects(self, content: &Catalogs) -> &[EffectSpec] {
        let row = |spell| content.spell_row(spell);
        match self {
            Self::Spell(spell) => row(spell).map_or(&[], |row| &row.effects),
            Self::Backfire(spell) => row(spell).map_or(&[], |row| {
                if row.backfires_apart(&content.spells[spell]) {
                    &row.backfire
                } else {
                    &row.effects
                }
            }),
            Self::Pulse(spell) => row(spell).map_or(&[], |row| &row.pulse),
            Self::Special(kind) => &super::specials::special(kind).effects,
            Self::Blow => &WEAPON_EFFECTS,
        }
    }
}

/// What the damage of a leeching spell comes back to its caster's side as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum Leeches {
    /// Health, to the one of the caster's side most hurt (Lifesteal).
    Health,
    /// Spell points, to the one most spent; a monster's budget (Might to
    /// Magic).
    Points,
}

/// What an auto-powered restoring spell measures its victim's lack in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum Restores {
    Health,
    Stamina,
    /// Health; stamina once health is full.
    HealthThenStamina,
}

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
/// The `Spell…` rolls are how a table row says "this spell's own": they
/// are read against the spell the strike is of.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub enum Roll {
    /// The spell's dice times power, raised by the boost (retail
    /// `RollEffectMagnitude`).
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
    /// The spell's rounds (retail `RollEffectDuration`).
    SpellRounds,
    /// The spell's dice as a percent of the purse (Philosopher's Bane): a
    /// second roll when the first is under 100.
    SpellGoldShare,
    Fixed(u32),
    /// These dice times power, raised by the boost unless `plain`, over
    /// `over`.
    Dice {
        dice: Option<Dice>,
        #[serde(default)]
        plain: bool,
        #[serde(default = "one::<u32>")]
        over: u32,
    },
    /// Purify's, from power alone.
    Purify,
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
    /// Resisted, and saved against, in the realm of the spell it belongs
    /// to.
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
    /// The realm it is resisted and saved against in, as it lands of
    /// `spell`.
    pub fn realm(&self, spell: Option<&SpellDefinition>) -> Option<Realm> {
        if self.spell_realm { spell.and_then(|spell| spell.realm) } else { self.resist }
    }
}

/// What an effect does. The `Spell…` kinds are a spell's own, as the
/// [`Roll`]s are.
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
    /// Shorten the clouds hanging over the body, by its magnitude and the
    /// spell's level.
    SpellPurify,
    /// The spell, laid on the body for its magnitude's rounds.
    SpellEnchant,
    Burn,
    LoseGold,
    Dispel,
    /// Prismic Ray's either-or: half the time the spell's damage, otherwise
    /// a weighted condition for the spell's rounds, or death.
    SpellPrismic,
    /// A named spell, laid on the body for the magnitude's rounds.
    Enchant(SpellId),
    /// One of these, by its weight in a hundred, picked for each body.
    OneOf(Vec<Choice>),
}

/// One of an effect's either-ors: how likely it is in a hundred, what it
/// does and how its magnitude is rolled.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Choice {
    pub weight: u32,
    pub kind: EffectKind,
    pub roll: Roll,
}

/// One spell's row of the table: what it does when cast, and, for a personal
/// spell, what it does turned on its caster, and for one that pulses, what
/// each pulse does.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpellRow {
    pub effects: Vec<EffectSpec>,
    pub leeches: Option<Leeches>,
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
    /// Turned on its caster's side it follows rows of its own: a personal
    /// spell, or one whose row gives a backfire. Any other backfires as it
    /// is cast.
    pub fn backfires_apart(&self, spell: &SpellDefinition) -> bool {
        spell.facts.personal || !self.backfire.is_empty()
    }
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

impl SpellRow {
    /// Whether a spell would do anything to `victim` (retail
    /// `MonsterSpellTargetOK`): healing only the hurt, rest only the tired,
    /// cures only what they cure, and no condition or enchantment it
    /// already has (`enchanted`: the spell is laid on it now); a spell
    /// that does none of these always would. A follow-up that only comes if the
    /// effect before it landed is not weighed.
    pub fn worthwhile(
        &self,
        (victim, conditions): (
            &crate::game::characters::Vitals,
            &crate::game::characters::Conditions,
        ),
        enchanted: bool,
    ) -> bool {
        let has = |condition| conditions.has(condition);
        let mut purposes = self
            .effects
            .iter()
            .filter_map(|effect| match &effect.kind {
                EffectKind::Heal => Some(Purpose::Heals),
                EffectKind::Refresh => Some(Purpose::Refreshes),
                EffectKind::Afflict { condition, .. } if !effect.landed => {
                    Some(Purpose::Afflicts(*condition))
                }
                EffectKind::SpellEnchant | EffectKind::Enchant(_) => Some(Purpose::Enchants),
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
        if let EffectKind::OneOf(choices) = &self.kind {
            let roll = rng.below(100);
            let chosen = crate::game::rng::weighted(roll, choices.iter().map(|c| c.weight));
            if let Some(choice) = chosen.map(|index| choices[index].clone()) {
                if let EffectKind::Afflict { condition, .. } = choice.kind {
                    self.save = Some(condition_save(condition));
                }
                self.roll = Some(choice.roll);
                self.kind = choice.kind;
            }
        }
        if self.kind == EffectKind::SpellPrismic {
            let roll = rng.below(100);
            if roll >= 50 {
                self.roll = Some(Roll::SpellDice);
                self.kind = EffectKind::Harm;
            } else {
                let pick = crate::game::rng::weighted(roll, AFFLICTIONS.iter().map(|&(_, w)| w))
                    .and_then(|index| AFFLICTIONS[index].0);
                self.roll = Some(Roll::SpellRounds);
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

pub fn roll(
    roll: Roll,
    strength: Strength,
    blow: Option<&Blow>,
    spell: Option<&SpellDefinition>,
    rng: &mut GameRng,
) -> u32 {
    let count = strength.power / 3 + 1;
    let own = || spell.expect("only a spell's row rolls the spell's own");
    match roll {
        Roll::SpellDice => magnitude(rng, own().dice, strength),
        Roll::SpellDiceWith { plain, times, over } => {
            let dice = own().dice.map(|dice| Dice { count: dice.count * times, ..dice });
            let strength = if plain { Strength { boost: 0, ..strength } } else { strength };
            magnitude(rng, dice, strength) / over
        }
        Roll::SpellRounds => duration(rng, own().duration_base, own().duration_per_power, strength),
        // Retail's second roll when the first is below 100 is intentional.
        Roll::SpellGoldShare => match magnitude(rng, own().dice, strength) {
            first if first < 100 => magnitude(rng, own().dice, strength).min(100),
            _ => 100,
        },
        Roll::Fixed(amount) => amount,
        Roll::Dice { dice, plain, over } => {
            let strength = if plain { Strength { boost: 0, ..strength } } else { strength };
            magnitude(rng, dice, strength) / over
        }
        Roll::Purify => {
            let base = strength.power + 1;
            let amount = match rng.below(4) {
                0 if base > 1 => base + 1,
                1 if base < 3 => base + 1,
                _ => base,
            };
            amount + amount * strength.boost / 100
        }
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

    #[test]
    fn every_spell_the_table_names_has_effects() {
        let table = SpellEffects::default();
        assert!(!table.0.is_empty());
        for (&id, rows) in &table.0 {
            assert_ne!(id, SpellId::NONE);
            assert!(!rows.effects.is_empty(), "{id:?}");
        }
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

    fn rolled(roll: Roll, power: u32, boost: u32, seed: u64) -> u32 {
        let strength = Strength { power, against: 0, boost };
        super::roll(roll, strength, None, None, &mut GameRng::seeded(seed))
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
                .map(|seed| {
                    let strength = Strength { power, against: 0, boost };
                    duration(&mut GameRng::seeded(seed), base, per_power, strength)
                })
                .collect()
        };
        assert_eq!(rounds(3, 2, 2, 0), [9, 10].into());
        assert_eq!(rounds(0, 0, 1, 0), [1, 2].into());
        assert_eq!(rounds(0, 1, 1, 0), [2, 3].into());
        assert_eq!(rounds(3, 2, 2, 50), [13, 15].into());
        assert_eq!(rounds(0, 9999, 1, 50), [9999].into());
        assert_eq!(rounds(0, -5, 1, 0), [0, 1].into(), "never under nothing, then the luck");
    }

    /// A gold share is the dice at the power; if that is under 100, a second roll
    /// is taken in its place, held at 100; if it is 100 or over, 100.
    #[test]
    fn a_gold_share_is_the_dice_rolled_again_when_under_a_hundred_and_held_at_a_hundred() {
        let share = |base| {
            let dice = Some(Dice { base, count: 1, sides: 1 });
            let spell = SpellDefinition { dice, ..default() };
            let strength = Strength { power: 1, against: 0, boost: 0 };
            roll(Roll::SpellGoldShare, strength, None, Some(&spell), &mut GameRng::seeded(0))
        };
        assert_eq!(share(30), 31, "the second roll, 1d1 + 30");
        assert_eq!(share(99), 100, "the second, 100, held");
        assert_eq!(share(150), 100, "not under 100, so 100");
    }

    fn spell_with_facts(id: SpellId) -> SpellDefinition {
        let spell = SpellDefinition { id, realm: Some(Realm::Air), ..default() };
        crate::content::spells::SpellCatalog::new(vec![spell]).0.remove(0)
    }
}
