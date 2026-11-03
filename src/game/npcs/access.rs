//! Read access to an NPC's current interaction endpoint.
use super::*;
use crate::game::{
    characters::Vitals,
    data::Catalogs,
    factions::Factions,
    party::roster::PartyMember,
    time::GameClock,
    world::{
        ActorPose,
        hostility::{self, Stance},
        identify::{GroupMember, MonsterGroup},
        level::{Level, LevelOwned},
        population::Monster,
    },
};
use bevy::ecs::system::SystemParam;

#[derive(SystemParam)]
pub struct NpcActors<'w, 's> {
    records: Query<
        'w,
        's,
        (
            Entity,
            &'static ActorId,
            Option<&'static NpcBodies>,
            Option<&'static NpcTrigger>,
            Has<PartyMember>,
        ),
    >,
    bodies: Query<
        'w,
        's,
        (
            &'static Vitals,
            &'static Monster,
            ActorPose,
            &'static LevelOwned,
            Option<&'static GroupMember>,
            Has<crate::game::combat::Combatant>,
        ),
    >,
    groups: Query<'w, 's, &'static MonsterGroup>,
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
        self.records
            .iter()
            .find(|(_, id, _, _, _)| self.content.npcs[id.0].source_name.eq_ignore_ascii_case(name))
            .map(|r| r.0)
    }
    pub fn shown(&self, name: &str) -> Option<Entity> {
        self.records
            .iter()
            .find(|(_, id, _, _, _)| {
                self.content.npcs[id.0].display_name.eq_ignore_ascii_case(name)
            })
            .map(|r| r.0)
    }
    pub fn definition(&self, actor: Entity) -> Option<&crate::content::npcs::NpcDefinition> {
        self.records.get(actor).ok().map(|r| &self.content.npcs[r.1.0])
    }
    pub fn active_body(&self, actor: Entity) -> Option<Entity> {
        self.records.get(actor).ok()?.2?.iter().find(|&b| {
            self.bodies
                .get(b)
                .is_ok_and(|(v, _, _, l, _, _)| !v.is_dead() && self.levels.contains(l.0))
        })
    }
    pub fn position(&self, body: Entity) -> Option<Vec3> {
        self.bodies.get(body).ok().map(|r| r.2.translation())
    }
    pub fn available(&self, actor: Entity, factions: &Factions) -> bool {
        let Ok((_, _, _, trigger, member)) = self.records.get(actor) else { return false };
        if member {
            return self.members.get(actor).is_ok_and(|v| !v.is_dead());
        }
        if trigger.is_some_and(|t| self.levels.contains(t.0)) {
            return true;
        }
        let Some(body) = self.active_body(actor) else { return false };
        let Ok((v, m, _, _, g, fighting)) = self.bodies.get(body) else { return false };
        let set = g.and_then(|g| self.groups.get(g.0).ok()).and_then(|g| g.set);
        hostility::monster_own(
            v,
            set,
            &self.content.species[m.species],
            factions,
            self.clock.ms,
            fighting,
        ) != Stance::Hostile
    }
    pub fn turned(&self, actor: Entity) -> bool {
        self.active_body(actor)
            .and_then(|b| self.bodies.get(b).ok())
            .is_some_and(|r| r.0.conditions.has(crate::game::characters::Condition::Turncoat))
    }
    pub fn species_level(&self, actor: Entity) -> u32 {
        self.active_body(actor)
            .and_then(|b| self.bodies.get(b).ok())
            .map_or(1, |r| u32::from(self.content.species[r.1.species].level).max(1))
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
