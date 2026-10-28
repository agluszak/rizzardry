//! Campaign-lived NPC identity, memory and stock, linked to map-owned bodies.
pub mod conversation;
pub mod trade;
use crate::content::{npcs::*, species::MonsterCatalog};
use crate::game::{
    characters::Character,
    party::roster::PartyRoster,
    world::{ActorPose, level::KeptMonster, navigation::Walker, population::Monster},
};
use bevy::{ecs::system::SystemParam, prelude::*};
use std::collections::HashMap;

/// Named NPCs use their definition identity; repeatable monster-bound NPCs also
/// need the authored placement. Neither key uses a transient retail runtime slot.
#[derive(Component, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ActorId {
    Named(NpcId),
    Placed { npc: NpcId, map: String, location: i32 },
}
impl ActorId {
    pub fn for_placement(id: NpcId, def: &NpcDefinition, map: &str, location: i32) -> Self {
        if def.monster_bound {
            Self::Placed { npc: id, map: map.into(), location }
        } else {
            Self::Named(id)
        }
    }
}
/// A map-owned body points to its campaign-lived NPC. Despawning the map
/// removes the body and relationship, never the durable NPC.
#[derive(Component, Debug)]
#[component(immutable)]
#[relationship(relationship_target=NpcBodies)]
pub struct NpcBody(pub Entity);
#[derive(Component, Debug, Deref)]
#[relationship_target(relationship=NpcBody)]
pub struct NpcBodies(Vec<Entity>);
#[derive(Debug)]
pub struct NpcWorldRole {
    pub map: String,
    pub body: KeptMonster,
}
#[derive(Component, Debug)]
pub struct Npc {
    pub definition: NpcId,
    pub disposition: u8,
    pub greeting_pending: bool,
    pub paid_topics: Vec<u16>,
    /// Durable world role captured when the active body leaves its map.
    pub dormant: Option<NpcWorldRole>,
    pub stock: trade::MerchantStock,
}
#[derive(Resource, Default)]
pub struct NpcAssets {
    pub catalog: Handle<NpcCatalog>,
    pub facts: Handle<FactCatalog>,
    pub scripts: HashMap<NpcId, Handle<DialogueScript>>,
}
fn load_npcs(mut content: ResMut<NpcAssets>, assets: Res<AssetServer>) {
    if content.catalog == Handle::default() {
        content.catalog = assets.load(crate::content::paths::NPCS);
    }
    if content.facts == Handle::default() {
        content.facts = assets.load(crate::content::paths::FACTS);
    }
}
#[derive(SystemParam)]
pub struct NpcPopulation<'w, 's> {
    pub assets: ResMut<'w, NpcAssets>,
    pub definitions: Res<'w, Assets<NpcCatalog>>,
    pub facts: Res<'w, Assets<FactCatalog>>,
    pub scripts: Res<'w, Assets<DialogueScript>>,
    pub npcs:
        Query<'w, 's, (Entity, &'static ActorId, &'static mut Npc, Option<&'static NpcBodies>)>,
    bodies: Query<
        'w,
        's,
        (
            &'static NpcBody,
            &'static Monster,
            ActorPose,
            Option<&'static Walker>,
            Option<&'static crate::game::world::identify::GroupMember>,
            &'static crate::game::world::level::LevelOwned,
            Has<crate::game::world::population::Provoked>,
        ),
        Without<crate::game::party::body::PartyBody>,
    >,
    groups: Query<'w, 's, &'static crate::game::world::identify::MonsterGroup>,
    pub clock: Res<'w, crate::game::time::GameClock>,
    roster: Query<'w, 's, &'static PartyRoster>,
    profiles: Query<'w, 's, &'static Character>,
}
impl NpcPopulation<'_, '_> {
    pub fn average_level(&self) -> u32 {
        let levels: Vec<_> = self
            .profiles
            .iter_many(self.roster.iter().flat_map(|r| r.iter()))
            .matched()
            .map(|c| u32::from(c.level))
            .collect();
        if levels.is_empty() { 1 } else { levels.iter().sum::<u32>() / levels.len() as u32 }
    }
    /// Preparation changes only asset requests. Missing mandatory NPC content
    /// fails activation without touching the currently played world.
    pub fn prepare(
        &mut self,
        species: &MonsterCatalog,
        placements: &[crate::content::level::MonsterPlacement],
        items: &crate::content::items::ItemCatalog,
        server: &AssetServer,
    ) -> Result<bool, String> {
        let ids: Vec<_> =
            placements.iter().filter_map(|p| species.kind(p.species).and_then(|s| s.npc)).collect();
        if ids.is_empty() {
            return Ok(true);
        }
        if matches!(
            server.get_load_state(&self.assets.facts),
            Some(bevy::asset::LoadState::Failed(_))
        ) {
            return Err("fact database failed".into());
        }
        if !self.facts.contains(&self.assets.facts) {
            return Ok(false);
        }
        if matches!(
            server.get_load_state(&self.assets.catalog),
            Some(bevy::asset::LoadState::Failed(_))
        ) {
            return Err("NPC database failed".into());
        }
        let Some(catalog) = self.definitions.get(&self.assets.catalog) else { return Ok(false) };
        let mut ready = true;
        for id in ids {
            let def = catalog.get(id).ok_or_else(|| format!("missing NPC definition {id:?}"))?;
            if def.stock.iter().any(|r| items.get(r.item).is_none()) {
                return Err(format!("missing stock item for {id:?}"));
            }
            let script =
                self.assets.scripts.entry(id).or_insert_with(|| {
                    server.load(crate::content::paths::retail(&def.script_path()))
                });
            if matches!(server.get_load_state(&*script), Some(bevy::asset::LoadState::Failed(_))) {
                return Err(format!("NPC script failed: {}", def.script_path()));
            }
            ready &= self.scripts.contains(&*script);
        }
        Ok(ready)
    }
    pub fn depart(&mut self, root: Entity, map: &str) {
        for (body, monster, pose, walker, group, parent, provoked) in &self.bodies {
            if parent.0 != root {
                continue;
            }
            let Ok((_, _, mut npc, _)) = self.npcs.get_mut(body.0) else { continue };
            npc.dormant = Some(NpcWorldRole {
                map: map.into(),
                body: KeptMonster {
                    provoked,
                    monster: monster.clone(),
                    group: group.and_then(|g| self.groups.get(g.0).ok()).map(|g| g.authored),
                    feet: crate::game::world::navigation::feet(pose.translation(), walker),
                    yaw: pose.rotation().to_euler(EulerRot::YXZ).0,
                },
            });
        }
    }
}
pub struct NpcPlugin;
impl Plugin for NpcPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<NpcCatalog>()
            .init_asset::<DialogueScript>()
            .init_asset::<FactCatalog>()
            .init_resource::<NpcAssets>()
            .init_resource::<conversation::StoryFacts>()
            .add_message::<conversation::ConversationRequest>()
            .add_message::<conversation::LinePlayback>()
            .add_systems(Startup, load_npcs)
            .add_systems(
                Update,
                conversation::initialize_facts.before(crate::game::GameplaySet::LevelActivation),
            )
            .add_systems(
                Update,
                conversation::update_conversation.after(crate::game::GameplaySet::Combat),
            );
    }
}
