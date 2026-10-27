//! Campaign-lived NPC actors. Active world components are removed on departure;
//! the retained entity owns memory and stock, never a second live character.
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
/// World presence is independent of despawn ownership. A map never owns the
/// lifetime of a named actor, even while its body is on that map.
#[derive(Component, Debug)]
#[component(immutable)]
#[relationship(relationship_target=PresentNpcs)]
pub struct NpcPresence(pub Entity);
#[derive(Component, Debug)]
#[relationship_target(relationship=NpcPresence)]
pub struct PresentNpcs(Vec<Entity>);
#[derive(Component, Debug)]
pub struct Npc {
    pub definition: NpcId,
    pub disposition: u8,
    pub greeting_pending: bool,
    pub paid_topics: Vec<u16>,
    /// Present only while there is no Monster/body on this entity.
    pub dormant: Option<(String, KeptMonster)>,
    pub stock: trade::MerchantStock,
}
#[derive(Resource, Default)]
pub struct NpcActors(pub HashMap<ActorId, Entity>);
#[derive(Resource, Default)]
pub struct NpcAssets {
    pub catalog: Handle<NpcCatalog>,
    pub facts: Handle<FactCatalog>,
    pub scripts: HashMap<NpcId, Handle<DialogueScript>>,
}
fn load_npcs(mut content: ResMut<NpcAssets>, assets: Res<AssetServer>) {
    if content.catalog == Handle::default() {
        content.catalog = assets.load("retail://data/databases/npc.dbs");
    }
    if content.facts == Handle::default() {
        content.facts = assets.load("retail://data/databases/fact.dbs");
    }
}
#[derive(SystemParam)]
pub struct NpcPopulation<'w, 's> {
    pub assets: ResMut<'w, NpcAssets>,
    pub definitions: Res<'w, Assets<NpcCatalog>>,
    pub facts: Res<'w, Assets<FactCatalog>>,
    pub scripts: Res<'w, Assets<DialogueScript>>,
    pub actors: ResMut<'w, NpcActors>,
    pub npcs: Query<
        'w,
        's,
        (
            Entity,
            &'static mut Npc,
            Option<&'static Monster>,
            Option<ActorPose>,
            Option<&'static Walker>,
            Option<&'static crate::game::world::identify::GroupMember>,
            Option<&'static Children>,
        ),
    >,
    groups: Query<'w, 's, &'static crate::game::world::identify::MonsterGroup>,
    pub clock: Res<'w, crate::game::time::GameClock>,
    roster: Res<'w, PartyRoster>,
    profiles: Query<'w, 's, &'static Character>,
}
impl NpcPopulation<'_, '_> {
    pub fn average_level(&self) -> u32 {
        let levels: Vec<_> =
            self.profiles.iter_many(&self.roster.0).matched().map(|c| u32::from(c.level)).collect();
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
        let ids: Vec<_> = placements
            .iter()
            .filter_map(|p| species.0.get(p.species as usize).and_then(|s| s.npc))
            .collect();
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
            let script = self
                .assets
                .scripts
                .entry(id)
                .or_insert_with(|| server.load(format!("retail://{}", def.script_path())));
            if matches!(server.get_load_state(&*script), Some(bevy::asset::LoadState::Failed(_))) {
                return Err(format!("NPC script failed: {}", def.script_path()));
            }
            ready &= self.scripts.contains(&*script);
        }
        Ok(ready)
    }
    pub fn depart(&mut self, commands: &mut Commands, map: &str) {
        use crate::game::world::{identify::GroupMember, navigation::WalkerMotion};
        use avian3d::prelude::*;
        use bevy_landmass::prelude::*;
        use bevy_tnua::prelude::*;
        for (entity, mut npc, monster, pose, walker, group, children) in &mut self.npcs {
            let (Some(monster), Some(pose)) = (monster, pose) else { continue };
            npc.dormant = Some((
                map.into(),
                KeptMonster {
                    monster: monster.clone(),
                    group: group.and_then(|g| self.groups.get(g.0).ok()).map(|g| g.authored),
                    feet: crate::game::world::navigation::feet(pose.translation(), walker),
                    yaw: pose.rotation().to_euler(EulerRot::YXZ).0,
                },
            ));
            if let Some(children) = children {
                for child in children.iter() {
                    commands.entity(child).despawn();
                }
            }
            commands
                .entity(entity)
                .remove::<NpcPresence>()
                .remove::<(Transform, GlobalTransform, Visibility, InheritedVisibility, ViewVisibility)>()
                .remove::<(
                    Monster,
                    GroupMember,
                    Walker,
                    RigidBody,
                    Position,
                    Rotation,
                    Collider,
                    LinearVelocity,
                    AngularVelocity,
                    LockedAxes,
                )>()
                .remove::<(
                    TnuaController<WalkerMotion>,
                    TnuaConfig<WalkerMotion>,
                    bevy_tnua::TnuaMotor,
                    bevy_tnua_avian3d::TnuaAvian3dSensorShape,
                )>()
                .remove::<(bevy_landmass::Agent3d, AgentTarget3d, AgentSettings, ArchipelagoRef3d, Velocity3d)>();
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
            .init_resource::<NpcActors>()
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
