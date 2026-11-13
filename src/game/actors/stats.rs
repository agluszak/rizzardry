//! An actor's figures: the components rules read, a party member's and a
//! monster's alike. Each is written by its own system in
//! [`super::figures`], from what the actor is made of.

use bevy::prelude::*;
use enum_map::EnumMap;
use enumset::EnumSet;

use super::Place;
use super::sources::{Abilities, Traits};
use super::spells::Bonus;
use crate::content::characters::{Attribute, BodyPart, Realm, Skill};
use crate::content::items::{AttackEffects, AttackMode, Dice};
use crate::content::species::{AttackWord, MonsterKind, Reach};
use crate::content::spells::Spellbook;
use crate::game::characters::{
    Calling, Character, Condition, Conditions, Training, scale_by_level,
};
use crate::game::inventory::WeaponSlot;
use crate::game::party::formation::Formation;

/// An actor's attributes as they stand now, spells counted, each 1 to 125.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attributes(pub EnumMap<Attribute, u32>);

impl Default for Attributes {
    fn default() -> Self {
        Self(EnumMap::from_fn(|_| 1))
    }
}

impl std::ops::Index<Attribute> for Attributes {
    type Output = u32;
    fn index(&self, attribute: Attribute) -> &u32 {
        &self.0[attribute]
    }
}

/// An actor's skills as they stand now, each 0 to 125 (retail
/// `GetEffectiveSkillLevel`): the points on the sheet, a quarter more (at
/// least one) for the profession's bonus skill, and what its conditions
/// take off. A monster has none.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Skills(EnumMap<Skill, u32>);

impl std::ops::Index<Skill> for Skills {
    type Output = u32;
    fn index(&self, skill: Skill) -> &u32 {
        &self.0[skill]
    }
}

impl Skills {
    /// From the points trained, a quarter more (at least one) for the
    /// profession's bonus skill, and what conditions take off, held to
    /// 0-125.
    pub(super) fn affected(
        training: &Training,
        calling: &Calling,
        conditions: &Conditions,
    ) -> Self {
        let ailing: i32 = conditions.modifiers().map(|row| row.skills).sum();
        Self(EnumMap::from_fn(|skill| {
            let points = training.0[skill] as i32;
            let bonus =
                if calling.profession.row().bonus_skill == skill { (points / 4).max(1) } else { 0 };
            (points + bonus + ailing).clamp(0, 125) as u32
        }))
    }

    /// A character's, on paper: before they are anyone's actor.
    pub fn of_sheet(character: &Character) -> Self {
        Self::affected(&character.training, &character.calling, &Conditions::default())
    }
}

impl Attributes {
    /// From base values, with what the spells on the actor add (Superman to
    /// each, Haste to Speed) and what its conditions take off, held to
    /// 1-125 once all are counted (retail `ResetCharacterAttributes`). One
    /// who trained, with the trait that eases a condition, gets some of it
    /// back, scaled by their mastery.
    pub(super) fn affected(
        base: &Abilities,
        spells: &Bonus,
        conditions: &Conditions,
        eases: Option<(&Traits, u8)>,
    ) -> Self {
        let mut ailing = EnumMap::<Attribute, i32>::default();
        for row in conditions.modifiers() {
            for &(attribute, change) in &row.attributes {
                ailing[attribute] += change;
            }
            if let (Some(eased), Some((traits, mastery))) = (&row.eased_by, eases)
                && traits.has(eased.ability)
            {
                ailing[eased.attribute] += scale_by_level(mastery, eased.back) as i32;
            }
        }
        Self(EnumMap::from_fn(|a| {
            (base.0[a] as i32 + spells.attributes[a] as i32 + ailing[a]).clamp(1, 125) as u32
        }))
    }

    /// A character's, on paper: before they are anyone's actor.
    pub fn of_sheet(character: &Character) -> Self {
        Self(character.abilities.0)
    }

    /// These with a point of one lost for good, as disease takes it.
    pub fn weakened(&self, attribute: Attribute) -> Self {
        let mut less = *self;
        less.0[attribute] = (less.0[attribute] - 1).max(1);
        less
    }

    /// Senses + speed + dexterity + intelligence, which weighs for an
    /// actor's blows and against those at it (retail SSDI).
    pub fn agility(&self) -> i32 {
        use Attribute::*;
        [Senses, Speed, Dexterity, Intelligence].map(|a| self[a] as i32).iter().sum()
    }
}

/// The level an actor's saves, resistance and special attacks weigh.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Level(pub u32);

/// What an actor is to the effects that land on it.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct Resistances {
    /// Percent resistance in each realm.
    pub realms: EnumMap<Realm, i32>,
    /// The conditions it cannot suffer at all (retail
    /// `g_condition_immunities`, by creature kind).
    pub immune: &'static [Condition],
    /// The conditions it shrugs off whenever a save against them is allowed
    /// (a member's traits: an Android's, the Fearless, a Psionic's mind).
    pub wards: Vec<Condition>,
    /// It cannot be slain outright.
    pub death_immune: bool,
    /// Its creature kind (undead, demon, plant, …), which holy and slaying
    /// things single out; a member has none.
    pub kind: Option<MonsterKind>,
    /// How strongly it casts from each book, which a spell turned back on
    /// it or from it grows from.
    pub caster: EnumMap<Spellbook, u32>,
}

impl Resistances {
    /// What a spell's strength from the actor grows from: its highest level
    /// among the spell's books.
    pub fn caster_level(&self, spell: &crate::content::spells::SpellDefinition) -> u32 {
        spell.books.iter().map(|book| self.caster[book]).max().unwrap_or(0)
    }
}

/// An actor's armour against blows: higher is better guarded, five off the
/// aim for each point (retail `GetTargetArmorClass`,
/// `TargetArmorClassAtLocation`, `GetTargetArmorClassModifier`).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Armour {
    /// Against being struck at all.
    pub total: i32,
    /// Where struck, by location.
    pub at: [i32; 7],
    /// What each mode finds besides.
    pub by_mode: EnumMap<AttackMode, i32>,
    /// What a surprise strips from its guard.
    pub exposed: i32,
    /// What bracing (defending) adds everywhere, and to what a surprise
    /// strips: two, and a shield's skill counting for more.
    pub braced: (i32, i32),
    /// Fatigue adds to its guard, as it does to a monster's.
    pub tires: bool,
    /// The percent cut off the damage of blows at it.
    pub reduction: i32,
    /// The percent chance what is shot or thrown at it is turned aside.
    pub missile_shield: u32,
    /// Where blows land on a monster, by its species' chances; a member
    /// has none of its own.
    pub hit_locations: Option<[u8; 7]>,
}

/// One thing an actor strikes with: one of a member's weapon slots as their gear
/// and stats make it now, or one of a monster's species' attacks. A
/// member's two slots are its two arms; a monster's attacks all belong to
/// its primary slot, which shares its attacks a round among them.
#[derive(Clone, Debug, PartialEq)]
pub struct Arm {
    pub slot: WeaponSlot,
    /// The weapon held, which an accepted attack is locked to.
    pub weapon: Option<Entity>,
    pub reach: Reach,
    pub modes: EnumSet<AttackMode>,
    /// The dice its blows roll; a punch rolls the off hand's (what it holds,
    /// else its fist), a kick the main hand's fist.
    pub dice: Dice,
    pub punch: Dice,
    pub kick: Dice,
    /// How well it strikes: a member's slot rating and five times its
    /// to-hit, a monster's attack score and five times its to-hit bonus.
    pub rating: i32,
    /// What its blows' dice are raised by, in percent.
    pub percent: i32,
    pub effects: AttackEffects,
    /// The monster kinds it slays, each a die more.
    pub slays: Vec<MonsterKind>,
    /// Attacks a round with it.
    pub attacks: u32,
    /// Its initiative, added to its body's.
    pub initiative: i32,
    /// Most swings an attack makes, before what the weapon adds.
    pub swings: u32,
    pub extra_swings: i32,
    /// Its blows are ranged combat: they train Eagle Eye and spend what is
    /// shot or thrown.
    pub ranged: bool,
    /// A launcher's: it shoots the ammunition in the other slot, which
    /// pierces by these modes.
    pub loaded: Option<EnumSet<AttackMode>>,
    /// A Ninja's throw, which gets through armour whenever it strikes.
    pub sure_throw: bool,
    /// The wielder's points in the skill whose training adds a kill chance
    /// to its blows.
    pub critical: Option<u32>,
    /// The skills a blow of it that does damage practises.
    pub practises: &'static [Skill],
    /// What swinging it weighs, for the stamina it costs; a natural attack
    /// costs none.
    pub weight: Option<u16>,
    /// Swinging it costs stamina.
    pub tires: bool,
    /// A modern weapon's: fatigue tells on its aim half as much.
    pub modern: bool,
    /// Fatigue tells on its penetration too.
    pub fatigue_pierces: bool,
    /// A sword, which a Samurai's Lightning Strike needs.
    pub sword: bool,
    /// It strikes naturally from hiding at arm's length: a Rogue's sword or
    /// dagger, a rogue kind's close attack.
    pub ambushes: bool,
    /// How a monster's attack is announced, and whether "at" follows.
    pub word: Option<(AttackWord, bool)>,
    /// Where its blows land on a member, by its species' chances.
    pub lands_on_party: Option<EnumMap<BodyPart, u8>>,
    /// How its blows sound (a row of the hit-sound table), if at all.
    pub sound_class: Option<u8>,
}

impl Arm {
    /// The dice a blow of `mode` rolls: a punch the off hand's, a kick the
    /// main hand's fist, anything else the arm's own.
    pub fn rolls(&self, mode: AttackMode) -> Dice {
        match mode {
            AttackMode::Punch => self.punch,
            AttackMode::Kick => self.kick,
            _ => self.dice,
        }
    }
}

/// What an actor strikes with, and how it fights.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Attacks {
    pub arms: Vec<Arm>,
    /// When it acts in a round, before its weapon's own initiative.
    pub initiative: i32,
    /// How well it presses a blow past a guard, or guards against one,
    /// before fatigue (retail `GetCharacterProtectionScore`,
    /// `GetMonsterProtectionScore`); fatigue tells half as much on one whose
    /// first weapon `tires_little`.
    pub press: i32,
    pub tires_little: bool,
    /// Effective while blind (a member with the trait, a Monk among
    /// monsters): its aim past 10 scales by this level instead of being
    /// held to 10, and its action is not lost to blindness.
    pub blind_fighting: Option<u8>,
    /// What its kind adds to its Senses when it looks about.
    pub keen_sight: u32,
    /// The percent chance its natural strikes roll extra dice (retail
    /// `RollBackstabExtraDice`).
    pub backstab: i32,
    /// The level of one whose blows knock out more often, by which the
    /// chance grows: a member with the trait's, in their profession.
    pub knock_out: Option<u8>,
    /// How far it may walk in one combat move, in metres; the party walks
    /// as one, so a member has none.
    pub stride: Option<f32>,
    /// The share of its stride Haste adds to a move: a tenth a power.
    pub hasted: f32,
}

impl Attacks {
    /// The furthest any of its arms reaches.
    pub fn reach(&self) -> Reach {
        self.arms.iter().map(|arm| arm.reach).max().unwrap_or(Reach::Touch)
    }

    /// The arms of a slot.
    pub fn of(&self, slot: WeaponSlot) -> impl Iterator<Item = &Arm> {
        self.arms.iter().filter(move |arm| arm.slot == slot)
    }

    /// The attacks a round the slot has: its arms' most.
    pub fn attacks(&self, slot: WeaponSlot) -> u32 {
        self.of(slot).map(|arm| arm.attacks).max().unwrap_or(0)
    }

    /// The slot's initiative: its arms' most.
    pub fn slot_initiative(&self, slot: WeaponSlot) -> i32 {
        self.of(slot).map(|arm| arm.initiative).max().unwrap_or(0)
    }

    /// The slot an attack opens with: the one with the most attacks, then
    /// the bigger initiative (retail `ChooseCharacterAttackHand`).
    pub fn opening(&self) -> Option<WeaponSlot> {
        [WeaponSlot::Primary, WeaponSlot::Secondary]
            .into_iter()
            .filter(|&slot| self.attacks(slot) > 0)
            .reduce(|best, slot| {
                let rank = |slot| (self.attacks(slot), self.slot_initiative(slot));
                // The main hand keeps it on a tie.
                if rank(slot) > rank(best) { slot } else { best }
            })
    }

    /// The weapons its slots hold.
    pub fn weapons(&self) -> EnumMap<WeaponSlot, Option<Entity>> {
        EnumMap::from_fn(|slot| self.of(slot).find_map(|arm| arm.weapon))
    }
}

/// Whether an actor reaches another with `reach` (retail `CloseFormationGap`,
/// `FrontRankScreens`): a member and someone outside the party by the
/// formation, which shortens touch and short reach by the occupied rows
/// between; two members unless the center screens a touch blow; anyone
/// else by the gap between their bodies.
pub fn reaches(
    formation: &Formation,
    (a, from): (Entity, &Place),
    (b, to): (Entity, &Place),
    reach: Reach,
) -> bool {
    let seated = |body| formation.row_of(body).is_some();
    let through_formation = |member, seat: &Place, other: &Place| {
        let side = seat.quadrant(other.feet);
        formation.reach(member, side, reach).is_some_and(|r| seat.gap(other) <= r.metres())
    };
    match (seated(a), seated(b)) {
        (true, true) => a == b || reach > Reach::Touch || !formation.screens(a, b),
        (true, false) => through_formation(a, from, to),
        (false, true) => through_formation(b, to, from),
        (false, false) => from.gap(to) <= reach.metres(),
    }
}
