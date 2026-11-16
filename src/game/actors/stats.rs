//! An actor's figures: the components rules read, a party member's and a
//! monster's alike. Each is written by its own system in
//! [`super::figures`], from what the actor is made of.

use bevy::prelude::*;
use enum_map::EnumMap;

use super::Place;
use super::sources::{Abilities, Traits};
use super::spells::Bonus;
use crate::content::characters::{Attribute, Realm, Skill};
use crate::content::species::{MonsterKind, Reach};
use crate::content::spells::Spellbook;
use crate::game::characters::{
    Calling, Character, Condition, Conditions, Training, scale_by_level,
};
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

/// Senses, Speed, Dexterity and Intelligence together, which weigh for an
/// actor's blows and against those at it (retail SSDI): a figure written
/// with its attributes.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Agility(pub i32);

/// What tiredness takes off an actor's aim and armour, by the share of its
/// stamina spent (retail `FatigueArmorPenalty`): a figure written as its
/// stamina changes.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fatigue(pub i32);

impl Fatigue {
    /// With this percent of stamina spent: 0 under half, then 5, 10, 20 and
    /// 40 from 50, 70, 85 and 95.
    pub fn of_spent(percent: u32) -> Self {
        Fatigue(match percent {
            0..50 => 0,
            50..70 => 5,
            70..85 => 10,
            85..=94 => 20,
            _ => 40,
        })
    }
}

/// How well an actor presses a blow past a guard, or guards against one: its
/// press less its fatigue, which tells half as much on one whose first
/// weapon tires little.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pressing(pub i32);

/// It is blind: its aim past 10 is lost unless it fights blind.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Blinded;

/// The level the strength of an actor's traits grows with: a monster's own
/// level, written as it is made; the level in the profession followed now
/// for one who trained (retail `ScaleValueByMonsterLevel`,
/// `ScaleValueByProfessionLevel`).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mastery(pub u8);

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

/// The percent cut off the damage of blows at it.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reduction(pub i32);

/// The percent chance what is shot or thrown at it is turned aside.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MissileShield(pub u32);

/// When an actor acts in a round, before its weapon's own initiative: its
/// nature's, and what its training adds.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Initiative(pub i32);

/// What training adds to when one acts in a round (retail `CalcInitiative`);
/// nothing for one who never trained.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrainedInitiative(pub i32);

/// It fights blind (a member with the trait, a Monk among monsters): its aim
/// past 10 scales by this level instead of being held to 10, and its action
/// is not lost to blindness.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlindFighting(pub u8);

/// Its blows knock out more often, by this level: a member with the trait's,
/// in their profession.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct KnockOut(pub u8);

/// The percent chance its natural strikes roll extra dice (retail
/// `RollBackstabExtraDice`), by its mastery.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Backstab(pub i32);

/// What an actor's spells and conditions add to its aim, five a point.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AimBonus(pub i32);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fatigue_grows_in_bands_of_the_share_of_stamina_spent() {
        let bands = [
            (0, 0),
            (49, 0),
            (50, 5),
            (69, 5),
            (70, 10),
            (84, 10),
            (85, 20),
            (94, 20),
            (95, 40),
            (100, 40),
        ];
        for (spent, lost) in bands {
            assert_eq!(Fatigue::of_spent(spent), Fatigue(lost), "{spent} spent");
        }
    }
}
