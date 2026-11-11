//! What a monster is and what it makes out (`docs/rules/monsters.md`
//! sections 2 and 4): the components of its kind that its decisions rest on
//! ([`Mind`]), and who it sees from where it stands ([`Sight`]). The decisions
//! of [`super`] read both; nothing is copied out of them first.

use bevy::ecs::query::QueryData;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::game::actors::{Eyes, Hidden, Laid, Places};
use crate::game::combat::turn::{Fleeing, SpecialCooldown};
use crate::game::hits::effects::{BodyReadOnly, BodyReadOnlyItem};
use crate::game::npcs::NpcBody;
use crate::game::party::roster::PartyRoster;
use crate::game::world::hostility::{self, Stance};
use crate::game::world::identify::GroupMember;
use crate::game::world::level::PlayedLevel;
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::{Monster, Special, Spellcasting, Temper};

/// What a monster's kind and the fight so far make of it.
#[derive(QueryData)]
pub struct Mind {
    pub monster: &'static Monster,
    pub temper: &'static Temper,
    pub special: &'static Special,
    pub casting: &'static Spellcasting,
    /// Rounds before its next special attack.
    pub cooldown: Option<&'static SpecialCooldown>,
    pub npc: Has<NpcBody>,
    /// It ran in an earlier round.
    pub fleeing: Has<Fleeing>,
    pub group: Option<&'static GroupMember>,
}

/// The fight a monster is deciding in.
#[derive(Clone, Copy)]
pub struct Fray<'a> {
    /// Those who may take part in it: the monsters and the members.
    pub bodies: &'a [Entity],
    /// A fight is on.
    pub fighting: bool,
    /// The monster's eyes make out the party.
    pub sees_party: bool,
}

/// Everyone in the played level, where they stand, and who makes out whom.
#[derive(SystemParam)]
pub struct Sight<'w, 's> {
    pub bodies: Query<'w, 's, BodyReadOnly>,
    pub minds: Query<'w, 's, Mind>,
    pub places: Places<'w, 's>,
    pub laid: Laid<'w, 's>,
    pub obstacles: WorldObstacles<'w, 's>,
    party: Single<'w, 's, (Entity, &'static PartyRoster)>,
    level: PlayedLevel<'w, 's>,
}

impl Sight<'_, '_> {
    /// The party, which every member stands in.
    pub fn party(&self) -> Entity {
        self.party.0
    }

    pub fn roster(&self) -> &PartyRoster {
        self.party.1
    }

    /// The far clip, past which nobody sees anybody.
    /// How far the level lets one see: its far clip.
    pub fn far(&self) -> f32 {
        self.level.view_distance().unwrap_or(0.0)
    }

    /// A monster as its decisions read it.
    pub fn monster(&self, monster: Entity) -> Option<(BodyReadOnlyItem<'_, '_>, MindItem<'_, '_>)> {
        self.bodies.get(monster).ok().zip(self.minds.get(monster).ok())
    }

    pub fn alive(&self, body: Entity) -> bool {
        self.bodies.get(body).is_ok_and(|body| !body.vitals.is_dead())
    }

    /// The side a body's group or kind has it on, its own turning aside.
    pub fn side(&self, body: Entity) -> Option<Stance> {
        self.bodies.get(body).ok().map(|body| body.side.own())
    }

    /// How one living body stands toward another, itself a friend.
    pub fn toward(&self, own: &BodyReadOnlyItem, other: Entity) -> Option<Stance> {
        let their = self.bodies.get(other).ok().filter(|their| !their.vitals.is_dead())?;
        Some(if other == own.entity {
            Stance::Friendly
        } else {
            hostility::between(own.standing(), their.standing())
        })
    }

    /// How far a body is from the party.
    pub fn party_gap(&self, body: Entity) -> f32 {
        self.places.gap(body, self.party())
    }

    /// Whether the party has a clear line to a body within the far clip.
    pub fn in_party_view(&self, body: Entity) -> bool {
        let party = self.places.of(self.party());
        party.in_view(&self.places.of(body), self.far(), &self.obstacles)
    }

    /// Whether a body's eyes make out the party, which hides by its lowest
    /// able level and by Chameleon.
    pub fn sees_party(&self, watcher: Entity) -> bool {
        let Ok(own) = self.bodies.get(watcher) else { return false };
        let members = self
            .bodies
            .iter_many(self.roster().iter())
            .matched()
            .map(|member| (member.level, member.vitals));
        let hidden = Hidden::party(members, self.laid.bonus(self.party()).concealment);
        Eyes::of(own.vitals, own.attributes, own.attacks).see(
            &self.places.of(watcher),
            (&self.places.of(self.party()), hidden),
            self.far(),
            &self.obstacles,
        )
    }

    /// Whether a body's eyes make out a monster, which hides by its level.
    pub fn sees(&self, watcher: Entity, watched: Entity) -> bool {
        let (Ok(own), Ok(there)) = (self.bodies.get(watcher), self.bodies.get(watched)) else {
            return false;
        };
        Eyes::of(own.vitals, own.attributes, own.attacks).see(
            &self.places.of(watcher),
            (&self.places.of(watched), Hidden::alone(*there.level)),
            self.far(),
            &self.obstacles,
        )
    }

    /// Whether a body sees a living foe of its own among `others`: another
    /// monster it sees, or the party, if it makes it out and a member
    /// stands against it.
    pub fn sees_foe(&self, watcher: Entity, others: &[Entity]) -> bool {
        let Ok(own) = self.bodies.get(watcher) else { return false };
        let foe = |other: Entity| self.toward(&own, other) == Some(Stance::Hostile);
        let member = |other: &Entity| self.roster().contains(other);
        others.iter().any(|&other| !member(&other) && foe(other) && self.sees(watcher, other))
            || (others.iter().any(|&other| member(&other) && foe(other))
                && self.sees_party(watcher))
    }
}
