//! Read access to an NPC's current interaction endpoint.
use super::*;
use crate::game::{
    characters::Vitals,
    data::Catalogs,
    party::roster::PartyMember,
    time::GameClock,
    world::{
        ActorPose,
        hostility::{self, Stance},
        level::{Level, LevelOwned},
        population::Monster,
    },
};
use bevy::ecs::system::SystemParam;

/// The people there are, and where each can be met.
#[derive(SystemParam)]
pub struct NpcActors<'w, 's> {
    persons: Query<'w, 's, (Entity, &'static ActorId, Option<&'static NpcTrigger>)>,
    people: People<'w, 's>,
    bodies: Query<
        'w,
        's,
        (
            (&'static Vitals, &'static crate::game::characters::Conditions),
            ActorPose,
            &'static LevelOwned,
            &'static crate::game::actors::Level,
        ),
        With<Monster>,
    >,
    sides: Query<'w, 's, &'static hostility::Side>,
    members: Query<'w, 's, &'static Vitals, With<PartyMember>>,
    levels: Query<'w, 's, (Entity, &'static Level)>,
    pub content: If<Res<'w, Catalogs>>,
    clock: Res<'w, GameClock>,
}
impl NpcActors<'_, '_> {
    pub fn level(&self) -> Option<(Entity, &Level)> {
        self.levels.iter().next()
    }
    pub fn now(&self) -> u64 {
        self.clock.ms
    }
    pub fn actor_named(&self, name: &str) -> Option<Entity> {
        self.persons
            .iter()
            .find(|(_, id, _)| self.content.npcs[id.0].source_name.eq_ignore_ascii_case(name))
            .map(|r| r.0)
    }
    pub fn shown(&self, name: &str) -> Option<Entity> {
        self.persons
            .iter()
            .find(|(_, id, _)| self.content.npcs[id.0].display_name.eq_ignore_ascii_case(name))
            .map(|r| r.0)
    }
    pub fn definition(&self, person: Entity) -> Option<&crate::content::npcs::NpcDefinition> {
        self.persons.get(person).ok().map(|r| &self.content.npcs[r.1.0])
    }
    /// The body a person stands in, alive, in the level played.
    pub fn active_body(&self, person: Entity) -> Option<Entity> {
        self.people.body(person).filter(|&body| {
            self.bodies
                .get(body)
                .is_ok_and(|((v, _), _, l, _)| !v.is_dead() && self.levels.contains(l.0))
        })
    }
    pub fn position(&self, body: Entity) -> Option<Vec3> {
        self.bodies.get(body).ok().map(|r| r.1.translation())
    }
    /// The member a person is, while they have a seat in the party.
    pub fn member(&self, person: Entity) -> Option<Entity> {
        self.people.member(person)
    }
    /// Whether someone has a seat in the party.
    pub fn in_party(&self, person: Entity) -> bool {
        self.member(person).is_some()
    }
    pub fn available(&self, person: Entity) -> bool {
        let Ok((_, _, trigger)) = self.persons.get(person) else { return false };
        if let Some(member) = self.member(person) {
            return self.members.get(member).is_ok_and(|v| !v.is_dead());
        }
        if trigger.is_some_and(|t| self.levels.contains(t.0)) {
            return true;
        }
        let Some(body) = self.active_body(person) else { return false };
        self.sides.get(body).is_ok_and(|side| side.own() != Stance::Hostile)
    }
    pub fn turned(&self, person: Entity) -> bool {
        self.active_body(person).and_then(|b| self.bodies.get(b).ok()).is_some_and(
            |((_, conditions), ..)| conditions.has(crate::game::characters::Condition::Turncoat),
        )
    }
    pub fn species_level(&self, person: Entity) -> u32 {
        self.active_body(person).and_then(|b| self.bodies.get(b).ok()).map_or(1, |r| r.3.0.max(1))
    }
}
#[derive(SystemParam)]
pub struct Scripts<'w, 's> {
    ids: Query<'w, 's, &'static ActorId>,
    assets: Res<'w, NpcAssets>,
    scripts: Res<'w, Assets<DialogueScript>>,
}
impl Scripts<'_, '_> {
    pub fn get(&self, actor: Entity) -> Option<&DialogueScript> {
        self.scripts.get(self.assets.scripts.get(&self.ids.get(actor).ok()?.0)?)
    }
}
