//! Immutable spell and weapon descriptions, decoded once from retail catalogs.

use std::collections::HashMap;

use bevy::prelude::*;

use super::defences::{CONDITION_SAVE, DEATH_SAVE, INDEFINITE, condition_save};
use super::effects::*;
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

/// An immutable entry in the decoded effect catalog. It never enters gameplay queries.
#[derive(Clone, Debug, Default)]
pub struct Definition {
    pub effects: Vec<EffectSpec>,
    pub leeches: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefinitionId(usize);

/// An act refers to shared data; the catalog owns its lifetime.
#[derive(Component, Debug)]
pub struct Uses(pub DefinitionId);

/// The damage a blow deals, its first effect.
#[derive(Component, Clone, Copy, Debug)]
pub struct BlowDamage(pub u32);

/// The act's weapon effects: each description of the blow definition lands at
/// the weapon's chance for it, a poison at the weapon's strength.
#[derive(Component, Debug)]
pub struct WeaponEffects {
    pub effects: AttackEffects,
    pub by_monster: bool,
}

/// It happens only if the effect before it landed.
#[derive(Clone, Debug)]
pub struct OnlyIfLanded;

/// It happens only if the effect before it was resisted.
#[derive(Clone, Debug)]
pub struct OnlyIfResisted;

/// It acts on the party itself (its shared spells, pool or purse), once,
/// through the first member the act reaches.
#[derive(Clone, Debug)]
pub struct OnParty;

/// The weapon effect a blow description stands for.
#[derive(Clone, Copy, Debug)]
pub struct FromWeapon(pub AttackEffect);

/// Only for a party member.
#[derive(Clone, Debug)]
pub struct OnlyMembers;

/// Only for a monster.
#[derive(Clone, Debug)]
pub struct OnlyMonsters;

/// Only for a monster of these kinds.
#[derive(Clone, Debug)]
pub struct OnlyKinds(pub &'static [MonsterKind]);

/// Only from a monster's blow.
#[derive(Clone, Debug)]
pub struct OnlyByMonsters;

/// Prismic Ray's either-or: half the time damage, otherwise a weighted
/// condition or death.
#[derive(Clone, Copy, Debug)]
pub struct Prismic {
    pub dice: Option<Dice>,
    pub base: i32,
    pub per_power: i32,
}

/// How an effect's magnitude is rolled from its act's [`Strength`].
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
    /// Three rolls under the act's bound (its power), summed, over
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
    /// A blow's damage, as its act's [`BlowDamage`] says.
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
    purposes: HashMap<SpellId, Vec<Purpose>>,
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

/// Definition construction is ordinary data composition, independent of World.
#[derive(Default)]
struct Definer {
    definition: Definition,
}
impl Definer {
    fn effect(&mut self, parts: impl EffectParts) -> &mut Self {
        let mut effect = EffectSpec::default();
        parts.describe(&mut effect);
        self.definition.effects.push(effect);
        self
    }
    fn finish(self, entries: &mut Vec<Definition>) -> DefinitionId {
        let id = DefinitionId(entries.len());
        entries.push(self.definition);
        id
    }
}

/// Immutable instructions. The previous result and rolled magnitude are locals
/// of resolution, never fields in this catalog or components in the world.
#[derive(Clone, Debug, Default)]
pub struct EffectSpec {
    pub kind: EffectKind,
    pub roll: Option<Roll>,
    pub chance: Option<Chance>,
    pub save: Option<Save>,
    pub resist: Option<Resist>,
    pub half: bool,
    pub landed: bool,
    pub resisted: bool,
    pub party: bool,
    pub members: bool,
    pub monsters: bool,
    pub kinds: Option<&'static [MonsterKind]>,
    pub by_monsters: bool,
    pub weapon: Option<AttackEffect>,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum EffectKind {
    #[default]
    Harm,
    Sap,
    Afflict(Afflict),
    Slay,
    Tire,
    DrainPool,
    DrainBudget,
    DrainRealm(DrainRealm),
    Heal,
    Refresh,
    Replenish,
    Cure(Cure),
    Renew,
    Raise,
    Purify(Purify),
    Enchant(Enchant),
    Burn,
    LoseGold,
    Dispel,
    Prismic(Prismic),
}

trait EffectParts {
    fn describe(self, effect: &mut EffectSpec);
}
macro_rules! tuples {
    ($($T:ident),+) => {
        impl<$($T: EffectParts),+> EffectParts for ($($T,)+) {
            #[allow(non_snake_case)]
            fn describe(self, effect: &mut EffectSpec) {
                let ($($T,)+) = self;
                $($T.describe(effect);)+
            }
        }
    }
}
tuples!(A, B);
tuples!(A, B, C);
tuples!(A, B, C, D);
tuples!(A, B, C, D, E);
tuples!(A, B, C, D, E, F);
/// A condition, rolled for so many rounds, saved against and resisted.
fn condition(condition: Condition, rounds: Roll, realm: Realm) -> impl EffectParts {
    (
        Afflict { condition, poison: 0, indefinite: false },
        rounds,
        Save(condition_save(condition)),
        Resist { realm },
    )
}

/// What a spell does to each victim, by retail's effect functions
/// (`Magic Effects.cpp`); `None` for one the game does not resolve.
fn define_spell(d: &mut Definer, spell: &SpellDefinition) -> Option<()> {
    use Condition::*;
    use SpellId as S;
    let realm = spell.realm?;
    let resist = Resist { realm };
    let dice = Roll::Dice { dice: spell.dice, plain: false, over: 1 };
    let rounds = Roll::Duration { base: spell.duration_base, per_power: spell.duration_per_power };
    let harm = (Harm, dice, resist);
    let afflict = |c| condition(c, rounds, realm);
    let personal = crate::game::magic::enchantments::personal(spell.id);
    match spell.id {
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
        | S::TSUNAMI => {
            d.effect(harm);
        }
        S::LIFESTEAL | S::MIGHT_TO_MAGIC => {
            d.effect(harm);
            let lifesteal = spell.id == S::LIFESTEAL;
            d.definition.leeches = Some(lifesteal);
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
            let enchant = Enchant { spell: spell.id };
            if personal {
                d.effect((enchant, rounds));
            } else {
                // The party's own slots hold for all its members at once.
                d.effect((enchant, rounds, OnlyMonsters));
                d.effect((enchant, rounds, OnParty));
            }
        }
        S::HOLY_WATER => {
            d.effect((harm, OnlyKinds(UNDEAD)));
        }
        S::BANISH => {
            d.effect((harm, OnlyKinds(BANISHED)));
        }
        S::HEAL_WOUNDS | S::HEAL_ALL => {
            d.effect((Heal, dice));
        }
        S::STAMINA | S::REST_ALL => {
            d.effect((Refresh, dice));
        }
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
            d.effect((Heal, restoring))
                .effect((Refresh, restoring))
                .effect(Cure { conditions: ALL, outright: true });
        }
        S::RESTORE_HEALTH => {
            let twice = spell.dice.map(|dice| Dice { count: dice.count * 2, ..dice });
            d.effect((Heal, dice))
                .effect((Refresh, Roll::Dice { dice: twice, plain: false, over: 1 }));
        }
        S::RESTORE_MAGIC => {
            d.effect((Replenish, dice));
        }
        S::ITCHING_SKIN => {
            d.effect(afflict(Irritated));
        }
        S::PARALYZE | S::FREEZE_FLESH | S::FREEZE_ALL => {
            d.effect(afflict(Paralyzed));
        }
        S::SLEEP => {
            d.effect(afflict(Asleep));
        }
        S::BLINDING_FLASH => {
            d.effect(afflict(Blind));
        }
        S::INSANITY => {
            d.effect(afflict(Insane));
        }
        S::SLOW => {
            d.effect(afflict(Slowed));
        }
        S::WEB => {
            d.effect(afflict(Webbed));
        }
        S::SILENCE => {
            d.effect(afflict(Silenced));
        }
        S::HEX => {
            d.effect(afflict(Hexed));
        }
        S::TURNCOAT => {
            d.effect(afflict(Turncoat));
        }
        S::MIND_STAB | S::PSIONIC_BLAST => {
            d.effect(harm).effect((afflict(Insane), Chance(25)));
        }
        S::BLIZZARD => {
            d.effect(harm).effect((afflict(Blind), Chance(25)));
        }
        S::CEREBRAL_HEMORRHAGE => {
            d.effect(harm).effect((afflict(Insane), Chance(75)));
        }
        S::CONCUSSION => {
            d.effect(harm).effect((afflict(Unconscious), Chance(50)));
        }
        S::NOXIOUS_FUMES => {
            d.effect(harm)
                .effect((afflict(Nauseated), Chance(90)))
                .effect((afflict(Unconscious), Chance(25)));
        }
        // A member it lands on may be knocked out too, a monster frightened
        // again, at half strength.
        S::TERROR => {
            d.effect(afflict(Afraid)).effect((
                afflict(Unconscious),
                OnlyMembers,
                OnlyIfLanded,
                HalfStrength,
            ));
        }
        S::SONIC_BOOM => {
            d.effect(afflict(Afraid)).effect((
                afflict(Afraid),
                OnlyMonsters,
                OnlyIfLanded,
                HalfStrength,
            ));
        }
        // One who keeps their mind is afraid anyway, with no save.
        S::PANDEMONIUM => {
            let afraid = Afflict { condition: Afraid, poison: 0, indefinite: false };
            d.effect(afflict(Insane)).effect((afraid, rounds, resist, OnlyIfResisted));
        }
        S::INSTANT_DEATH | S::QUICKSAND | S::ASPHYXIATION | S::DEATH_WISH => {
            d.effect((Slay, rounds, Save(DEATH_SAVE), resist));
        }
        S::PRISMIC_RAY | S::PRISMIC_CHAOS => {
            let prismic = Prismic {
                dice: spell.dice,
                base: spell.duration_base,
                per_power: spell.duration_per_power,
            };
            d.effect((prismic, resist));
        }
        S::CURE_LESSER_COND => {
            let conditions = &[Irritated, Nauseated, Afraid, Asleep, Blind];
            d.effect(Cure { conditions, outright: false });
        }
        S::CURE_PARALYSIS => {
            d.effect(Cure { conditions: &[Paralyzed, Webbed], outright: false });
        }
        S::CURE_POISON => {
            d.effect(Cure { conditions: &[Poisoned], outright: false });
        }
        S::CURE_DISEASE => {
            d.effect(Cure { conditions: &[Diseased], outright: false });
        }
        S::REMOVE_CURSE => {
            d.effect(Cure { conditions: &[Hexed], outright: false });
        }
        S::SANE_MIND => {
            d.effect(Cure { conditions: &[Insane, Turncoat], outright: false });
        }
        S::SMELLING_SALTS => {
            d.effect(Cure { conditions: &[Unconscious, Asleep], outright: false });
        }
        S::RESURRECTION => {
            d.effect(Raise);
        }
        // Purify rolls from power alone; the party's clouds hang over all
        // its members at once.
        S::PURIFY_AIR => {
            let purify = Purify { level: spell.level };
            d.effect((purify, Roll::Purify, OnlyMonsters)).effect((purify, Roll::Purify, OnParty));
        }
        S::PHILOSOPHERS_BANE => {
            d.effect((LoseGold, Roll::GoldShare(spell.dice), OnParty));
        }
        // Members lose the plain roll, without the cast's boost, for good.
        S::VAMPIRIC_VORTEX => {
            let plain = Roll::Dice { dice: spell.dice, plain: true, over: 1 };
            d.effect((Sap, plain, resist, OnlyMembers)).effect((harm, OnlyMonsters));
        }
        S::RENEWAL => {
            d.effect(Renew);
        }
        // Retail visits every slot a victim carries, and the party's pool
        // once.
        S::HEATWAVE => {
            d.effect((Burn, Roll::PerPower(10))).effect((Burn, Roll::PerPower(10), OnParty));
        }
        // Anti-Magic drains the whole living party, not only its victims,
        // and ends the party's lasting spells.
        S::ANTI_MAGIC => {
            d.effect((DrainPool, Roll::Fifty, OnParty)).effect((Dispel, OnParty));
        }
        _ => return None,
    }
    Some(())
}

/// What a personal spell turned on its caster's side does: Dracon Breath
/// nauseates, Haste slows, Razor Cloak harms; the others nothing.
fn define_backfire(d: &mut Definer, spell: &SpellDefinition) {
    let Some(realm) = spell.realm else { return };
    let rounds = Roll::Duration { base: spell.duration_base, per_power: spell.duration_per_power };
    match spell.id {
        SpellId::DRACON_BREATH => {
            d.effect(condition(Condition::Nauseated, rounds, realm));
        }
        SpellId::HASTE => {
            d.effect(condition(Condition::Slowed, rounds, realm));
        }
        SpellId::RAZOR_CLOAK => {
            d.effect((
                Harm,
                Roll::Dice { dice: spell.dice, plain: false, over: 1 },
                Resist { realm },
            ));
        }
        _ => {}
    }
}

/// An offensive cloud's pulse on whoever it hangs over (retail
/// `TickCombatEffectSlots`): Death Cloud makes a death save; the others
/// hurt, Draining Cloud then draining spell points and stamina by its own
/// roll, Toxic Cloud then nauseating, poisoning and knocking out.
fn define_pulse(d: &mut Definer, spell: &SpellDefinition) {
    let Some(realm) = spell.realm else { return };
    let resist = Resist { realm };
    let dice = |over| Roll::Dice { dice: spell.dice, plain: false, over };
    if spell.id == SpellId::DEATH_CLOUD {
        d.effect((Slay, Roll::Fixed(INDEFINITE), Save(DEATH_SAVE), resist));
        return;
    }
    d.effect((Harm, dice(1), resist));
    if spell.id == SpellId::DRAINING_CLOUD {
        d.effect((DrainRealm(None), dice(1), OnlyMembers, OnlyIfLanded))
            .effect((DrainBudget, dice(2), OnlyMonsters, OnlyIfLanded))
            .effect((Tire, dice(1), OnlyIfLanded));
    }
    if spell.id == SpellId::TOXIC_CLOUD {
        let rounds =
            Roll::Duration { base: spell.duration_base, per_power: spell.duration_per_power };
        for afflicted in [Condition::Nauseated, Condition::Poisoned, Condition::Unconscious] {
            d.effect(condition(afflicted, rounds, realm));
        }
    }
}

/// A special attack's effects: each of its two may be escaped (the first
/// one time in ten, the second one in two), each resisted by its realm; a
/// member and a monster roll them differently.
fn define_special(d: &mut Definer, definition: crate::game::hits::specials::SpecialDefinition) {
    use crate::game::hits::specials::SpecialEffect as E;
    let resist = Resist { realm: definition.realm };
    let three = |divide, plus, times, over| Roll::Three { divide, plus, times, over };
    for (slot, effect) in definition.effects.iter().enumerate() {
        let Some(effect) = *effect else { continue };
        let chance = Chance(if slot == 0 { 90 } else { 50 });
        match effect {
            E::Inflict(afflicted) => {
                let afflict = Afflict { condition: afflicted, poison: 0, indefinite: false };
                let save = Save(condition_save(afflicted));
                d.effect((afflict, Roll::ThirdOfBound, save, resist, chance, OnlyMonsters))
                    .effect((afflict, Roll::BelowBound, save, resist, chance, OnlyMembers));
            }
            E::Harm => {
                d.effect((Harm, three(3, 1, 1, 1), resist, chance));
            }
            // A member's realm six times over, a monster's budget by half.
            E::Drain => {
                d.effect((DrainBudget, three(6, 1, 1, 2), resist, chance, OnlyMonsters)).effect((
                    DrainRealm(Some(definition.realm)),
                    three(3, 1, 6, 1),
                    resist,
                    chance,
                    OnlyMembers,
                ));
            }
            E::Tire => {
                d.effect((Tire, three(1, 0, 1, 1), resist, chance, OnlyMonsters)).effect((
                    Tire,
                    three(1, 1, 1, 1),
                    resist,
                    chance,
                    OnlyMembers,
                ));
            }
            E::Wound if definition.resisted => {
                d.effect((Harm, three(3, 1, 1, 1), resist, chance));
            }
            E::Wound => {
                d.effect((Harm, three(3, 1, 1, 1), chance));
            }
        }
    }
}

/// What a blow does: its damage, then, once that drew blood, each weapon
/// effect in turn (retail `ApplyEffectConditions`), at the weapon's chance,
/// saved against and resisted at the attacker's effect power.
fn define_blow(d: &mut Definer) {
    use AttackEffect as A;
    use Condition::*;
    let condition = |afflicted: Condition, realm, effect| {
        (
            Afflict { condition: afflicted, poison: 0, indefinite: false },
            Roll::BlowRounds,
            Save(condition_save(afflicted)),
            Resist { realm },
            FromWeapon(effect),
        )
    };
    let indefinite = |afflicted: Condition, realm, effect| {
        (
            Afflict { condition: afflicted, poison: 0, indefinite: true },
            Roll::Fixed(INDEFINITE),
            Save(condition_save(afflicted)),
            Resist { realm },
            FromWeapon(effect),
        )
    };
    let drain = |times, sides, base, realm, effect| {
        (
            Roll::Blow { times, sides, base },
            Save(CONDITION_SAVE[0]),
            Resist { realm },
            FromWeapon(effect),
        )
    };
    d.effect((Harm, Roll::Damage))
        .effect(condition(Asleep, Realm::Air, A::Sleep))
        .effect(condition(Paralyzed, Realm::Water, A::Paralyze))
        .effect((
            Afflict { condition: Poisoned, poison: 0, indefinite: false },
            Roll::Blow { times: 3, sides: 2, base: BlowBase::Weapon },
            Save(condition_save(Poisoned)),
            Resist { realm: Realm::Water },
            FromWeapon(A::Poison),
        ))
        .effect(condition(Hexed, Realm::Divine, A::Hex))
        .effect(indefinite(Diseased, Realm::Divine, A::Disease))
        .effect((
            Slay,
            Roll::Fixed(INDEFINITE),
            Save(DEATH_SAVE),
            Resist { realm: Realm::Divine },
            FromWeapon(A::Kill),
        ))
        .effect(condition(Unconscious, Realm::Earth, A::KnockOut))
        .effect(condition(Blind, Realm::Fire, A::Blind))
        .effect(condition(Afraid, Realm::Mental, A::Frighten))
        .effect((condition(Turncoat, Realm::Divine, A::Possess), OnlyMembers, OnlyByMonsters))
        .effect((Sap, drain(1, 2, BlowBase::Zero, Realm::Earth, A::DrainHealth), OnlyMembers))
        .effect((Tire, drain(2, 5, BlowBase::Count(3), Realm::Water, A::DrainStamina)))
        .effect((
            DrainPool,
            drain(1, 6, BlowBase::Count(1), Realm::Divine, A::DrainSpellPoints),
            OnlyMembers,
        ))
        .effect(condition(Nauseated, Realm::Air, A::Nauseate))
        .effect(condition(Insane, Realm::Mental, A::Insane));
}

/// Decode the definitions once content has loaded, and what each spell's
/// effects are for.
pub fn define(world: &mut World) {
    use crate::game::hits::specials::{CHARACTER_BREATH, special};
    let content = world.resource::<Catalogs>().spells.0.clone();
    let mut entries = Vec::new();
    let mut spells = HashMap::new();
    let mut backfires = HashMap::new();
    let mut pulses = HashMap::new();
    for spell in content.iter() {
        let mut d = Definer::default();
        let defined = define_spell(&mut d, spell).is_some();
        if !defined {
            continue;
        }
        spells.insert(spell.id, d.finish(&mut entries));
        if crate::game::magic::enchantments::personal(spell.id) {
            let mut d = Definer::default();
            define_backfire(&mut d, spell);
            backfires.insert(spell.id, d.finish(&mut entries));
        }
        if crate::game::magic::enchantments::OFFENSIVE.contains(&spell.id)
            || spell.id == SpellId::RING_OF_FIRE
        {
            let mut d = Definer::default();
            define_pulse(&mut d, spell);
            pulses.insert(spell.id, d.finish(&mut entries));
        }
    }
    let mut specials = HashMap::new();
    for kind in (1..=u8::MAX).filter_map(|byte| SpecialAttackKind::try_from(byte).ok()) {
        let mut d = Definer::default();
        define_special(&mut d, special(kind));
        specials.insert(kind, d.finish(&mut entries));
    }
    let mut d = Definer::default();
    define_special(&mut d, CHARACTER_BREATH);
    let breath = d.finish(&mut entries);
    let mut d = Definer::default();
    define_blow(&mut d);
    let blow = d.finish(&mut entries);
    let purposes = spells
        .iter()
        .map(|(&spell, &definition)| (spell, purposes(&entries[definition.0])))
        .collect();
    world.insert_resource(Definitions {
        entries,
        spells,
        backfires,
        pulses,
        specials,
        breath,
        blow,
        purposes,
    });
}

/// What a definition's effects are for, in order, as a monster weighs them;
/// a follow-up that only comes if the one before landed is not counted.
fn purposes(definition: &Definition) -> Vec<Purpose> {
    definition
        .effects
        .iter()
        .filter_map(|effect| match &effect.kind {
            EffectKind::Heal => Some(Purpose::Heals),
            EffectKind::Refresh => Some(Purpose::Refreshes),
            EffectKind::Afflict(a) if !effect.landed => Some(Purpose::Afflicts(a.condition)),
            EffectKind::Enchant(e) => Some(Purpose::Enchants(e.spell)),
            EffectKind::Cure(c) => Some(Purpose::Cures(c.conditions)),
            _ => None,
        })
        .collect()
}

impl EffectSpec {
    pub fn pick(mut self, rng: &mut GameRng) -> Self {
        if let EffectKind::Prismic(ref prismic) = self.kind {
            let rounds = Roll::Duration { base: prismic.base, per_power: prismic.per_power };
            let roll = rng.below(100);
            if roll >= 50 {
                self.roll = Some(Roll::Dice { dice: prismic.dice, plain: false, over: 1 });
                self.kind = EffectKind::Harm;
            } else {
                let pick = crate::game::rng::weighted(roll, AFFLICTIONS.iter().map(|&(_, w)| w))
                    .and_then(|index| AFFLICTIONS[index].0);
                self.roll = Some(rounds);
                self.kind = match pick {
                    Some(condition) => {
                        self.save = Some(Save(condition_save(condition)));
                        EffectKind::Afflict(Afflict { condition, poison: 0, indefinite: false })
                    }
                    None => {
                        self.save = Some(Save(DEATH_SAVE));
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
    weapon: Option<&WeaponEffects>,
    damage: u32,
    rng: &mut GameRng,
) -> u32 {
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
        Roll::Damage => damage,
        Roll::BlowRounds => blow_roll(rng, count, 2, 1).clamp(1, 12),
        Roll::Blow { times, sides, base } => {
            let base = match base {
                BlowBase::Zero => 0,
                BlowBase::Count(times) => count * times,
                BlowBase::Weapon => weapon.map_or(0, |weapon| weapon.effects.strength),
            };
            blow_roll(rng, count * times, sides, base)
        }
    }
}

/// What one of a spell's effects is for, as a monster weighs whether the
/// spell is worth casting at someone.
#[derive(Clone, Copy, Debug)]
pub enum Purpose {
    Heals,
    Refreshes,
    Afflicts(Condition),
    Enchants(SpellId),
    Cures(&'static [Condition]),
}

impl Definitions {
    /// Whether a spell would do anything to `victim` (retail
    /// `MonsterSpellTargetOK`): healing only the hurt, rest only the tired,
    /// cures only what they cure, and no condition or enchantment it
    /// already has; a spell that does none of these always would. One the
    /// game does not resolve would not.
    pub fn worthwhile(&self, spell: SpellId, victim: &crate::game::characters::Vitals) -> bool {
        let Some(purposes) = self.purposes.get(&spell) else { return false };
        let has = |condition| victim.conditions.has(condition);
        purposes.is_empty()
            || purposes.iter().any(|purpose| match *purpose {
                Purpose::Heals => victim.health.current < victim.health.max,
                Purpose::Refreshes => victim.stamina.current < victim.stamina.max,
                Purpose::Afflicts(condition) => !has(condition),
                Purpose::Enchants(spell) => !victim.enchantments.has(spell),
                Purpose::Cures(conditions) => conditions.iter().any(|&c| has(c)),
            })
    }
}

impl EffectParts for Afflict {
    fn describe(self, e: &mut EffectSpec) {
        e.kind = EffectKind::Afflict(self);
    }
}

impl EffectParts for DrainRealm {
    fn describe(self, e: &mut EffectSpec) {
        e.kind = EffectKind::DrainRealm(self);
    }
}

impl EffectParts for Cure {
    fn describe(self, e: &mut EffectSpec) {
        e.kind = EffectKind::Cure(self);
    }
}

impl EffectParts for Purify {
    fn describe(self, e: &mut EffectSpec) {
        e.kind = EffectKind::Purify(self);
    }
}

impl EffectParts for Enchant {
    fn describe(self, e: &mut EffectSpec) {
        e.kind = EffectKind::Enchant(self);
    }
}

impl EffectParts for Prismic {
    fn describe(self, e: &mut EffectSpec) {
        e.kind = EffectKind::Prismic(self);
    }
}

impl EffectParts for Roll {
    fn describe(self, e: &mut EffectSpec) {
        e.roll = Some(self);
    }
}

impl EffectParts for Chance {
    fn describe(self, e: &mut EffectSpec) {
        e.chance = Some(self);
    }
}

impl EffectParts for Save {
    fn describe(self, e: &mut EffectSpec) {
        e.save = Some(self);
    }
}

impl EffectParts for Resist {
    fn describe(self, e: &mut EffectSpec) {
        e.resist = Some(self);
    }
}

impl EffectParts for HalfStrength {
    fn describe(self, e: &mut EffectSpec) {
        e.half = true;
    }
}

impl EffectParts for OnlyIfLanded {
    fn describe(self, e: &mut EffectSpec) {
        e.landed = true;
    }
}

impl EffectParts for OnlyIfResisted {
    fn describe(self, e: &mut EffectSpec) {
        e.resisted = true;
    }
}

impl EffectParts for OnParty {
    fn describe(self, e: &mut EffectSpec) {
        e.party = true;
    }
}

impl EffectParts for OnlyMembers {
    fn describe(self, e: &mut EffectSpec) {
        e.members = true;
    }
}

impl EffectParts for OnlyMonsters {
    fn describe(self, e: &mut EffectSpec) {
        e.monsters = true;
    }
}

impl EffectParts for OnlyByMonsters {
    fn describe(self, e: &mut EffectSpec) {
        e.by_monsters = true;
    }
}

impl EffectParts for OnlyKinds {
    fn describe(self, e: &mut EffectSpec) {
        e.kinds = Some(self.0);
    }
}

impl EffectParts for FromWeapon {
    fn describe(self, e: &mut EffectSpec) {
        e.weapon = Some(self.0);
    }
}

impl EffectParts for EffectKind {
    fn describe(self, e: &mut EffectSpec) {
        e.kind = self;
    }
}
