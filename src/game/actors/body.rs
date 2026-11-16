//! An actor as the rules that decide what it does read it. Nothing changes
//! an actor through this: whatever changes it queries what it changes.

use bevy::ecs::query::QueryData;
use bevy::prelude::*;

use super::{
    Armour, Attacks, Attributes, CanAct, CanFight, CanGuard, Level, Lucid, MissileShield,
    Reduction, Resistances, Skills, Traits,
};
use crate::game::characters::{Calling, Conditions, Schooling, Trained, Vitals};
use crate::game::items::Burden;
use crate::game::magic::{LearnedSpells, SpellPoints};
use crate::game::party::roster::PartyMember;
use crate::game::world::hostility::{Loyalty, Side, Stance};
use crate::game::world::population::{SpellBudget, Untargetable};

/// A member's or a monster's body: what both carry, and what only one of
/// them does.
#[derive(QueryData)]
pub struct Body {
    pub entity: Entity,
    pub vitals: &'static Vitals,
    pub conditions: &'static Conditions,
    pub lucid: Has<Lucid>,
    pub can_guard: Has<CanGuard>,
    pub can_act: Has<CanAct>,
    pub can_fight: Has<CanFight>,
    pub side: &'static Side,
    pub loyalty: &'static Loyalty,
    pub attributes: &'static Attributes,
    pub skills: &'static Skills,
    pub level: &'static Level,
    pub resistances: &'static Resistances,
    pub armour: &'static Armour,
    pub reduction: &'static Reduction,
    pub missile_shield: &'static MissileShield,
    pub attacks: &'static Attacks,
    pub traits: &'static Traits,
    pub calling: Option<&'static Calling>,
    pub schooling: Option<Schooling>,
    pub spells: Option<(&'static LearnedSpells, &'static SpellPoints)>,
    pub budget: Option<&'static SpellBudget>,
    pub burden: Option<&'static Burden>,
    pub member: Has<PartyMember>,
    pub untargetable: Has<Untargetable>,
}

impl BodyItem<'_, '_> {
    /// The side it is on now.
    pub fn stance(&self) -> Stance {
        self.side.now(self.conditions)
    }

    /// The side it is on and the band it keeps to now.
    pub fn standing(&self) -> (Stance, Loyalty) {
        (self.stance(), self.loyalty.now(self.conditions))
    }

    /// As the formulas of training read it, if it trained.
    pub fn trained(&self) -> Option<Trained<'_>> {
        self.schooling.as_ref().map(|schooling| schooling.view())
    }
}
