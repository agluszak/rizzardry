//! Campaign-lived NPCs retain their world-role state while their level is dormant.
pub mod conversation;
pub mod recruit;
pub mod trade;
use crate::content::items::ItemId;
use crate::content::npcs::*;
use crate::game::GameplaySet;
use crate::game::data::Catalogs;
use crate::game::rng::GameRng;
use crate::game::world::level::LevelOwned;
use crate::game::{
    characters::Character,
    party::{body::PartyBody, roster::PartyRoster},
    world::{ActorPose, level::KeptMonster, navigation::Walker, population::Monster},
};
use bevy::{
    ecs::{entity_disabling::Disabled, system::SystemParam},
    prelude::*,
};
use std::collections::HashMap;

/// Named NPCs use their definition identity; repeatable monster-bound NPCs also
/// need the authored placement. Neither key uses a transient retail runtime slot.
/// The NPC catalog, for operations that hold the whole world.
pub(crate) fn catalog(world: &World) -> &NpcCatalog {
    &world.resource::<Catalogs>().npcs
}

/// An NPC actor's authored definition, for operations that hold the whole
/// world. A level places only NPCs it has definitions for.
pub(crate) fn definition(world: &World, actor: Entity) -> &NpcDefinition {
    &catalog(world)[world.get::<ActorId>(actor).expect("an NPC actor").npc()]
}

/// An NPC actor's dialogue script, loaded before the level that placed it
/// was entered.
pub(crate) fn script(world: &World, actor: Entity) -> AssetId<DialogueScript> {
    let id = world.get::<ActorId>(actor).expect("an NPC actor").npc();
    world.resource::<NpcAssets>().scripts[&id].id()
}

/// The party's members awake and well enough to fight, in roster order: the
/// population retail's NPC rules work with (`hp_current != 0 &&
/// highest_condition < ASLEEP`, as in `CanNpcJoinParty`, the charm and
/// dialogue practice).
pub(crate) fn able_members(world: &mut World) -> Vec<Entity> {
    let party = crate::game::party::roster::the_party(world);
    world
        .get::<crate::game::party::roster::PartyRoster>(party)
        .unwrap()
        .iter()
        .filter(|&m| world.get::<crate::game::characters::Vitals>(m).unwrap().can_fight())
        .collect()
}

/// The levels of [`able_members`], which retail's NPC rules average.
pub(crate) fn able_levels(world: &mut World) -> Vec<u8> {
    able_members(world)
        .into_iter()
        .filter_map(|m| world.get::<crate::game::characters::Character>(m))
        .map(|character| character.level())
        .collect()
}

#[derive(Component, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ActorId {
    Named(NpcId),
    Placed { npc: NpcId, map: String, location: i32 },
}
impl ActorId {
    /// The NPC this is.
    pub fn npc(&self) -> NpcId {
        match *self {
            Self::Named(npc) | Self::Placed { npc, .. } => npc,
        }
    }
    pub fn for_placement(id: NpcId, def: &NpcDefinition, map: &str, location: i32) -> Self {
        if def.monster_bound {
            Self::Placed { npc: id, map: map.into(), location }
        } else {
            Self::Named(id)
        }
    }
}
/// The durable world role of an NPC, separate from its retained party profile.
#[derive(Component, Debug)]
#[component(immutable)]
#[relationship(relationship_target = NpcBodies)]
pub struct NpcBody(pub Entity);
#[derive(Component, Debug, Deref)]
#[relationship_target(relationship = NpcBody, linked_spawn)]
pub struct NpcBodies(Vec<Entity>);

/// The map and authored group of an NPC's world role. Its Monster component
/// remains on the world-role entity; only its physical embodiment ends on departure.
#[derive(Component, Debug)]
pub struct NpcWorldRole {
    pub map: String,
    pub group: Option<i32>,
}
#[derive(Component, Debug, Default)]
pub struct Npc {
    pub disposition: u8,
    pub greeting_pending: bool,
    pub paid_topics: Vec<u16>,
    /// Successful Mindread attempts advance through the NPC's three responses.
    pub mindread_refusals: u8,
    pub stock: trade::MerchantStock,
    /// What it still takes in bribes before it warms to the party.
    pub trade_pool: u16,
    /// When the party last charmed it and made small talk with it; each
    /// waits half a day before the party may try again.
    pub charmed_at: Option<u64>,
    pub small_talk_at: Option<u64>,
    pub possessions: Possessions,
    /// Killed (retail `W8NpcState::spawned`): the NPC never comes back to
    /// the world, and story facts read its death from here.
    pub dead: bool,
}

/// What an NPC carries on its person, apart from any stock it trades
/// (retail `InitializeNpcItemTable`): its item table's entries with their
/// weights, how many of the weighted ones it gives up, and its purse.
#[derive(Clone, Debug, Default)]
pub struct Possessions {
    pub items: Vec<(ItemId, u8)>,
    pub count: crate::content::items::Dice,
    pub gold: u32,
}

impl Possessions {
    pub fn of(definition: &NpcDefinition, tables: &crate::content::items::LootTables) -> Self {
        let table = definition.item_table.and_then(|id| tables.0.get(usize::from(id)));
        Self {
            items: table.map_or_else(Vec::new, |table| table.entries.clone()),
            count: table.map_or_else(default, |table| table.item_count),
            gold: definition.gold,
        }
    }

    /// Everything the NPC leaves when it dies (retail `DropMonsterLoot`'s
    /// NPC branch): every item of weight 0, then as many weighted draws as
    /// its count rolls, and its purse. It keeps nothing.
    pub fn give_up(
        &mut self,
        catalog: &crate::content::items::ItemCatalog,
        rng: &mut GameRng,
    ) -> (Vec<ItemId>, u32) {
        let table = crate::content::items::LootTable {
            name: String::new(),
            entries: std::mem::take(&mut self.items),
            level_scaled: false,
            item_count: self.count,
            gold: default(),
        };
        let wanted = rng.roll(self.count);
        let items = crate::game::loot::table_items(&table, catalog, 0, wanted, rng);
        (items, std::mem::take(&mut self.gold))
    }
}
#[derive(Resource, Default)]
pub struct NpcAssets {
    pub scripts: HashMap<NpcId, Handle<DialogueScript>>,
}
#[derive(SystemParam)]
pub struct NpcPopulation<'w, 's> {
    pub assets: ResMut<'w, NpcAssets>,
    pub scripts: Res<'w, Assets<DialogueScript>>,
    pub npcs: Query<'w, 's, (Entity, &'static ActorId, &'static NpcBodies)>,
    // Activation owns both live and dormant roles. Departure and placement
    // happen in one commit, before deferred Disabled changes become visible.
    roles: Query<
        'w,
        's,
        (
            Entity,
            &'static mut NpcWorldRole,
            &'static Monster,
            ActorPose,
            Option<&'static Walker>,
            Option<&'static LevelOwned>,
            Option<&'static crate::game::world::identify::GroupMember>,
        ),
        (With<NpcBody>, Allow<Disabled>, Without<PartyBody>),
    >,
    groups: Query<'w, 's, &'static crate::game::world::identify::MonsterGroup>,
    pub clock: Res<'w, crate::game::time::GameClock>,
    roster: Query<'w, 's, &'static PartyRoster>,
    profiles: Query<'w, 's, &'static Character>,
    /// NPCs travelling with the party, whose world roles wait.
    members: Query<'w, 's, (), With<crate::game::party::roster::PartyMember>>,
    /// Whether each NPC has died, which outranks whatever its dormant body
    /// last recorded.
    states: Query<'w, 's, &'static Npc>,
}
/// An NPC's placement as a level's population meets it.
pub struct Placing<'a> {
    pub placed: &'a crate::content::level::MonsterPlacement,
    /// The item tables an NPC's possessions come from.
    pub tables: &'a crate::content::items::LootTables,
    pub id: NpcId,
    /// The level's map, which identifies the placement.
    pub map: &'a str,
    /// The party's average level, which stock scales by.
    pub average: u32,
}

impl NpcPopulation<'_, '_> {
    /// Reuse the actor and its gameplay components; create them only on first meeting.
    pub fn place(
        &mut self,
        commands: &mut Commands,
        Placing { placed, tables, id, map, average }: Placing,
        catalogs: &Catalogs,
        placed_actors: &mut std::collections::HashSet<ActorId>,
        rng: &mut GameRng,
        roll: impl FnOnce(&mut GameRng) -> KeptMonster,
    ) -> Option<(Entity, Vec3, f32, crate::game::world::population::ScaledBody, Option<i32>)> {
        let definition = catalogs.npcs.get(id).expect("NPC preparation validated definition");
        let key = ActorId::for_placement(id, definition, map, placed.location);
        if !placed_actors.insert(key.clone()) {
            return None;
        }
        if let Some((actor, _, bodies)) =
            self.npcs.iter().find(|(_, identity, _)| **identity == key)
        {
            if !self.may_stand(actor) {
                return None;
            }
            let entity = *bodies.first()?;
            let (_, role, monster, pose, walker, ..) = self.roles.get(entity).ok()?;
            if role.map != map || monster.is_dead() {
                return None;
            }
            commands.entity(entity).remove::<Disabled>();
            return Some((
                entity,
                crate::game::world::navigation::feet(pose.translation(), walker),
                pose.rotation().to_euler(EulerRot::YXZ).0,
                monster.body,
                role.group,
            ));
        }
        let mut stock = trade::MerchantStock::default();
        if definition.owns_stock {
            stock.maintain(definition, &catalogs.items, average, self.clock.ms, false, rng);
        }
        let actor = commands
            .spawn((
                key,
                Npc {
                    disposition: definition.disposition,
                    greeting_pending: true,
                    mindread_refusals: 0,
                    paid_topics: vec![],
                    stock,
                    trade_pool: definition.trade_pool,
                    charmed_at: None,
                    small_talk_at: None,
                    possessions: Possessions::of(definition, tables),
                    dead: false,
                },
            ))
            .id();
        let kept = roll(rng);
        let body = kept.monster.body;
        let entity = commands
            .spawn((
                NpcBody(actor),
                NpcWorldRole { map: map.into(), group: kept.group },
                kept.monster,
            ))
            .id();
        Some((entity, kept.feet, kept.yaw, body, kept.group))
    }

    pub fn average_level(&self) -> u32 {
        self.roster.single().map_or(1, |roster| {
            crate::game::party::average_level(
                self.profiles.iter_many(roster.iter()).matched().map(|c| c.level()),
            )
        })
    }
    /// Preparation changes only asset requests. Missing mandatory NPC content
    /// fails activation without touching the currently played world.
    pub fn prepare(
        &mut self,
        catalogs: &Catalogs,
        placements: &[crate::content::level::MonsterPlacement],
        server: &AssetServer,
    ) -> Result<bool, String> {
        let (species, items) = (&catalogs.species, &catalogs.items);
        let ids: Vec<_> =
            placements.iter().filter_map(|p| species.kind(p.species).and_then(|s| s.npc)).collect();
        if ids.is_empty() {
            return Ok(true);
        }
        let catalog = &catalogs.npcs;
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
    /// Whether the NPC's world role may stand in a level: not while it
    /// travels with the party, and never once it has died, wherever.
    fn may_stand(&self, actor: Entity) -> bool {
        !self.members.contains(actor) && self.states.get(actor).is_ok_and(|npc| !npc.dead)
    }
    /// NPCs whose world roles stand on `map` without being placed there,
    /// such as one that left the party on it, as the level meets them again.
    pub fn visitors(
        &mut self,
        commands: &mut Commands,
        map: &str,
        placed_actors: &mut std::collections::HashSet<ActorId>,
    ) -> Vec<(Entity, Vec3, f32, crate::game::world::population::ScaledBody, Option<i32>)> {
        let mut visitors = Vec::new();
        for (actor, key, bodies) in &self.npcs {
            if !self.may_stand(actor) || placed_actors.contains(key) {
                continue;
            }
            let Some(&entity) = bodies.first() else { continue };
            let Ok((_, role, monster, pose, walker, ..)) = self.roles.get(entity) else { continue };
            if role.map != map || monster.is_dead() {
                continue;
            }
            placed_actors.insert(key.clone());
            commands.entity(entity).remove::<Disabled>();
            visitors.push((
                entity,
                crate::game::world::navigation::feet(pose.translation(), walker),
                pose.rotation().to_euler(EulerRot::YXZ).0,
                monster.body,
                role.group,
            ));
        }
        visitors
    }
    pub fn depart(&mut self, commands: &mut Commands, root: Entity) {
        for (entity, mut role, _, pose, walker, parent, group) in &mut self.roles {
            if !parent.is_some_and(|parent| parent.0 == root) {
                continue;
            }
            role.group = group.and_then(|g| self.groups.get(g.0).ok()).map(|g| g.authored);
            let at = Transform::from_translation(crate::game::world::navigation::feet(
                pose.translation(),
                walker,
            ))
            .with_rotation(pose.rotation());
            retire(commands, entity, at);
        }
    }
}
/// End an NPC body's embodiment, leaving it standing dormant at `at`: it
/// keeps its monster state but leaves its level, its group, physics and
/// navigation.
fn retire(commands: &mut Commands, entity: Entity, at: Transform) {
    use avian3d::prelude::{Collider, RigidBody};
    commands
        .entity(entity)
        .remove::<LevelOwned>()
        .remove::<crate::game::world::identify::GroupMember>()
        .remove_with_requires::<(Collider, RigidBody)>()
        .remove_with_requires::<bevy_landmass::prelude::Agent3dBundle>()
        .remove_with_requires::<crate::game::world::navigation::links::Crossing>()
        .remove::<(Walker, bevy_landmass::prelude::ReachedAnimationLink3d)>()
        .despawn_related::<Children>()
        .insert((at, Disabled));
}
pub struct NpcPlugin;
impl Plugin for NpcPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<DialogueScript>()
            .init_resource::<NpcAssets>()
            .init_resource::<conversation::StoryFacts>()
            .init_resource::<conversation::NoAnswerTurn>()
            .add_message::<conversation::ConversationRequest>()
            .add_message::<conversation::PartyRemark>()
            .add_message::<conversation::NpcSpeech>()
            .add_observer(conversation::end_conversations)
            .add_systems(Update, conversation::update_conversation.after(GameplaySet::Combat))
            .add_systems(Update, recruit::settle_npc_members.before(GameplaySet::LevelActivation))
            .add_systems(
                FixedUpdate,
                recruit::settle_npc_members
                    .after(GameplaySet::Combat)
                    .run_if(crate::game::world::level::level_active),
            );
    }
}
