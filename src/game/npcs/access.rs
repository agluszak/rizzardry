//! NPC identity and world availability; scripts have independent read access.
use super::story::{StoryFacts, fact_named};
use crate::content::npcs::*;
use crate::game::{
    characters::{Condition, Vitals},
    combat::Combatant,
    data::Catalogs,
    factions::Factions,
    time::GameClock,
    world::{
        ActorPose,
        hostility::{self, Stance},
        identify::{GroupMember, MonsterGroup},
        level::{Level, LevelOwned},
        navigation::Walker,
        population::Monster,
    },
};
use bevy::{
    ecs::{query::QueryData, system::SystemParam},
    prelude::*,
};
const RFS81: NpcId = NpcId(0x20);
const RFS81_FIXED_NAME: &str = "RFS81B";
/// The NPCs the party may talk to: their actors and definitions, their
/// bodies, the catalogs and the level being played. Reads
/// only; what a conversation changes about an NPC, the NPC component holds.
#[derive(SystemParam)]
pub struct NpcActors<'w, 's> {
    actors: Query<'w, 's, (Entity, &'static super::ActorId, &'static super::NpcBodies)>,
    bodies: Query<'w, 's, NpcBodyData>,
    groups: Query<'w, 's, &'static MonsterGroup>,
    levels: Query<'w, 's, (Entity, &'static Level)>,
    /// The databases, which a level loads before any NPC stands in it.
    pub content: If<Res<'w, Catalogs>>,
    clock: Res<'w, GameClock>,
}

/// An NPC's body in the world, as dialogues read it.
#[derive(QueryData)]
struct NpcBodyData {
    vitals: &'static Vitals,
    monster: &'static Monster,
    pose: ActorPose,
    walker: Option<&'static Walker>,
    level: Option<&'static LevelOwned>,
    group: Option<&'static GroupMember>,
    fighting: Has<Combatant>,
}

impl NpcActors<'_, '_> {
    pub fn now(&self) -> u64 {
        self.clock.ms
    }

    pub fn id(&self, actor: Entity) -> NpcId {
        self.actors.get(actor).expect("an NPC actor").1.npc()
    }

    /// An NPC actor's authored definition. A level places only NPCs it has
    /// definitions for.
    pub fn definition(&self, actor: Entity) -> &NpcDefinition {
        &self.content.npcs[self.id(actor)]
    }

    /// The level being played.
    pub fn level(&self) -> Option<(Entity, &Level)> {
        self.levels.iter().next()
    }

    /// The NPC's living body in the level being played.
    pub fn active_body(&self, actor: Entity) -> Option<Entity> {
        let (_, _, bodies) = self.actors.get(actor).ok()?;
        bodies.iter().find(|&body| {
            self.bodies.get(body).is_ok_and(|b| {
                b.level.is_some_and(|owner| self.levels.contains(owner.0)) && !b.vitals.is_dead()
            })
        })
    }

    /// Where the NPC's body stands.
    pub fn position(&self, body: Entity) -> Option<Vec3> {
        self.bodies.get(body).ok().map(|b| b.pose.translation())
    }

    /// The body's vitals.
    pub fn vitals(&self, body: Entity) -> Option<&Vitals> {
        self.bodies.get(body).ok().map(|b| b.vitals)
    }

    /// Where the body's feet are and which way it faces, for standing it
    /// dormant.
    pub fn standing_pose(&self, body: Entity) -> Option<Transform> {
        let b = self.bodies.get(body).ok()?;
        let feet = crate::game::world::navigation::feet(b.pose.translation(), b.walker);
        Some(Transform::from_translation(feet).with_rotation(b.pose.rotation()))
    }

    /// The level of the NPC's body, which charms are measured against.
    pub fn species_level(&self, actor: Entity) -> u32 {
        self.active_body(actor)
            .and_then(|body| self.bodies.get(body).ok())
            .map_or(1, |b| u32::from(self.content.species[b.monster.species].level))
            .max(1)
    }

    /// Whether the NPC can be spoken to: its body in the level is not
    /// hostile. A body outside any group stands by its species' default.
    pub fn available(&self, actor: Entity, factions: &Factions) -> bool {
        let Some(b) = self.active_body(actor).and_then(|body| self.bodies.get(body).ok()) else {
            return false;
        };
        let species = &self.content.species[b.monster.species];
        let set = b.group.and_then(|g| self.groups.get(g.0).ok()).and_then(|g| g.set);
        hostility::monster_own(b.vitals, set, species, factions, self.clock.ms, b.fighting)
            != Stance::Hostile
    }

    /// Whether the NPC's body is turned to the party's side.
    pub fn turned(&self, actor: Entity) -> bool {
        self.active_body(actor)
            .and_then(|body| self.bodies.get(body).ok())
            .is_some_and(|b| b.vitals.conditions.has(Condition::Turncoat))
    }

    /// The actor of the NPC called `source_name` in the catalog.
    pub fn actor_named(&self, source_name: &str) -> Option<Entity> {
        let id = self
            .content
            .npcs
            .0
            .iter()
            .position(|npc| npc.source_name.eq_ignore_ascii_case(source_name))?;
        self.actors.iter().find(|(_, actor, _)| usize::from(actor.npc().0) == id).map(|(e, ..)| e)
    }

    /// The NPC a script names by its display name (retail
    /// `FindNpcStateByName`).
    pub fn actor_shown_as(&self, name: &str, story: &StoryFacts) -> Option<Entity> {
        let fixed = fact_named(story, &self.content.facts, "FACT_RFS81_HAS_BEEN_FIXED")
            .is_ok_and(|value| value != 0);
        self.actors.iter().find_map(|(entity, id, _)| {
            let id = id.npc();
            let shown = if id == RFS81 && fixed {
                RFS81_FIXED_NAME
            } else {
                self.content.npcs[id].display_name.as_str()
            };
            shown.eq_ignore_ascii_case(name).then_some(entity)
        })
    }
}

/// Script access is separate from physical NPC availability and recruitment.
#[derive(SystemParam)]
pub struct Scripts<'w, 's> {
    identities: Query<'w, 's, &'static super::ActorId>,
    scripts: Res<'w, Assets<DialogueScript>>,
    assets: Res<'w, super::NpcAssets>,
}
impl Scripts<'_, '_> {
    pub fn get(&self, actor: Entity) -> &DialogueScript {
        let id = self.identities.get(actor).expect("an NPC actor").npc();
        self.scripts.get(&self.assets.scripts[&id]).expect("a loaded NPC script")
    }
}
