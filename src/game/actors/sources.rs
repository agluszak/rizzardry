//! What an actor is made of: the components its figures are written from.
//! Each is written when the actor is made, and afterwards only by the one
//! thing that changes it. No rule that resolves anything reads these; rules
//! read the figures.

use bevy::prelude::*;
use enum_map::EnumMap;
use enumset::EnumSet;

use super::Arm;
use crate::content::characters::{Attribute, Realm, Resist, Trait};
use crate::content::items::AttackMode;
use crate::content::species::MonsterKind;

/// The seven attributes before anything is laid on them or ails them.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Abilities(pub EnumMap<Attribute, u32>);

impl Default for Abilities {
    fn default() -> Self {
        Self(EnumMap::from_fn(|_| 1))
    }
}

/// What an actor can do that others cannot: a character's by race and the
/// profession followed now, a monster's by its kind.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Traits(pub EnumSet<Trait>);

impl Traits {
    pub fn has(&self, ability: Trait) -> bool {
        self.0.contains(ability)
    }
}

/// A creature's kind (undead, demon, plant, ...), which holy and slaying
/// things single out and which says what it cannot suffer.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kind(pub MonsterKind);

/// What an actor is guarded by, resists and mends by with nothing trained,
/// worn or laid on it: a monster's from its species, a character's from
/// their race.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct Nature {
    /// Its guard against being struck at all.
    pub armour: i32,
    /// What covers each of its places besides.
    pub armour_at: [i32; 7],
    pub armour_by_mode: EnumMap<AttackMode, i32>,
    /// Where blows land on it, by chance, if it has places of its own.
    pub hit_locations: Option<[u8; 7]>,
    /// What a surprise strips from its guard.
    pub exposed: i32,
    /// Fatigue adds to its guard.
    pub fatigue_guards: bool,
    /// Percent off every blow that lands on it.
    pub reduction: i32,
    pub resistance: EnumMap<Realm, Option<Resist>>,
    pub initiative: i32,
    /// Health and stamina it gains a minute, or loses when negative.
    pub mending: (i32, i32),
    /// Nothing kills it outright.
    pub death_immune: bool,
    /// What it can carry for each point of `VIT + 2 x STR`.
    pub carry_factor: u32,
    /// What its kind adds to its Senses when it looks about.
    pub keen_sight: u32,
    /// How far it may walk in one combat move, in metres.
    pub stride: Option<f32>,
}

impl Nature {
    /// What a race gives one born of it.
    pub fn of_race(race: crate::content::characters::Race) -> Self {
        let row = race.row();
        Self {
            armour: row.armour,
            resistance: row.resistance,
            carry_factor: row.carry_factor,
            ..default()
        }
    }
}

impl Default for Nature {
    fn default() -> Self {
        Self {
            armour: 0,
            armour_at: [0; 7],
            armour_by_mode: EnumMap::default(),
            hit_locations: None,
            exposed: 0,
            fatigue_guards: false,
            reduction: 0,
            resistance: EnumMap::default(),
            initiative: 0,
            mending: (0, 0),
            death_immune: false,
            carry_factor: 12,
            keen_sight: 0,
            stride: None,
        }
    }
}

/// What an actor strikes with while it holds nothing: a species' attacks,
/// each made once, when the actor is. A character has none; its fists are
/// what its hands make of being empty.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct NaturalArms(pub Vec<Arm>);

/// The health and stamina an actor was born with, rolled once.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub health: u32,
    pub stamina: u32,
}

/// Health and stamina an actor's maxima have lost to draining and disease,
/// until the drain is lifted (retail `hp_adjustment`, `fatigue_penalty`).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Drained {
    pub health: u32,
    pub stamina: u32,
}

impl Drained {
    /// Lift Drained (retail `RemoveCharacterCondition`): the maxima get
    /// back what draining and disease took, and what is left rises with
    /// them. Whether it was there to lift.
    pub fn lift(&mut self, conditions: &mut crate::game::characters::Conditions) -> bool {
        use crate::game::characters::Condition;
        if !conditions.has(Condition::Drained) {
            return false;
        }
        conditions.lift(Condition::Drained);
        *self = Self::default();
        true
    }
}
