use super::*;
use crate::{
    content::{level::MonsterPlacement, npcs::*},
    game::{
        characters::Character,
        data::Catalogs,
        party::roster::{PartyMember, PartyRoster},
        rng::GameRng,
        time::GameClock,
        world::{
            level::{Destination, Entry, LevelOwned},
            population::{Newborn, ScaledBody},
        },
    },
};
use bevy::ecs::system::SystemParam;
use std::collections::HashSet;

pub(super) fn initialize(
    content: If<Res<Catalogs>>,
    peopled: Option<Res<Peopled>>,
    mut commands: Commands,
    mut rng: ResMut<GameRng>,
    clock: Res<GameClock>,
    party: Query<&PartyRoster>,
    characters: Query<&Character>,
    destinations: Query<&Destination>,
) {
    if peopled.is_some() {
        return;
    }
    let Ok(destination) = destinations.single() else { return };
    let average = match &destination.entry {
        Entry::NewGame(profiles) => crate::game::party::average_level(
            profiles.iter().map(|(character, _)| character.level()),
        ),
        Entry::Travel => crate::game::party::average_level(
            party
                .iter()
                .flat_map(|r| r.iter())
                .filter_map(|e| characters.get(e).ok())
                .map(Character::level),
        ),
    };
    for (index, _) in content.npcs.0.iter().enumerate().filter(|(_, d)| !d.monster_bound) {
        spawn_record(&mut commands, NpcId(index as u16), &content, average, clock.ms, &mut rng);
    }
    commands.insert_resource(Peopled);
}
fn spawn_record(
    commands: &mut Commands,
    id: NpcId,
    content: &Catalogs,
    average: u32,
    now: u64,
    rng: &mut GameRng,
) -> Entity {
    let def = &content.npcs[id];
    let mut record = commands.spawn((ActorId(id), Npc::from_definition(def)));
    if let Some(template) = &def.character {
        recruit::character(&mut record, template, content, rng);
    }
    let actor = record.id();
    if def.owns_stock {
        trade::open(commands, actor, def, &content.items, average, now, rng);
    }
    actor
}
#[derive(SystemParam)]
pub struct NpcPopulation<'w, 's> {
    peopled: Option<Res<'w, Peopled>>,
    people: Query<'w, 's, (Entity, &'static ActorId, Option<&'static Placed>)>,
    dead: Query<'w, 's, (), With<crate::game::characters::Dead>>,
    assets: ResMut<'w, NpcAssets>,
    scripts: Res<'w, Assets<DialogueScript>>,
    states: Query<
        'w,
        's,
        (Has<Killed>, Has<Gone>, Has<PartyMember>, Option<&'static NpcLocation>, Has<Post>),
    >,
    roster: Query<'w, 's, &'static PartyRoster>,
    characters: Query<'w, 's, &'static Character>,
    clock: Res<'w, GameClock>,
}
pub struct Placing<'a> {
    pub placed: &'a MonsterPlacement,
    pub id: NpcId,
    pub map: &'a str,
    pub average: u32,
}
impl NpcPopulation<'_, '_> {
    pub fn average_level(&self) -> u32 {
        crate::game::party::average_level(
            self.roster
                .iter()
                .flat_map(|r| r.iter())
                .filter_map(|e| self.characters.get(e).ok())
                .map(Character::level),
        )
    }
    pub fn prepare(
        &mut self,
        content: &Catalogs,
        geometry: &crate::content::level::LevelDefinition,
        server: &AssetServer,
    ) -> bool {
        if self.peopled.is_none() {
            return false;
        }
        let mut ready = true;
        let ids = geometry.monsters.iter().filter_map(|p| content.species[p.species].npc).chain(
            content
                .npcs
                .0
                .iter()
                .enumerate()
                .filter(|(_, d)| {
                    (d.voice_script || d.merchant)
                        && geometry
                            .triggers
                            .iter()
                            .any(|t| t.name.eq_ignore_ascii_case(&format!("_{}", d.source_name)))
                })
                .map(|(i, _)| NpcId(i as u16)),
        );
        for id in ids {
            let definition = &content.npcs[id];
            let handle = self.assets.scripts.entry(id).or_insert_with(|| {
                server.load(crate::content::paths::retail(&definition.script_path(None)))
            });
            assert!(
                !matches!(server.get_load_state(&*handle), Some(bevy::asset::LoadState::Failed(_))),
                "NPC script failed to load"
            );
            ready &= self.scripts.contains(&*handle);
            // The lines of one the story may rename are ready with their
            // own.
            if let Some(name) = story::alias_of(definition.kind) {
                let path = crate::content::paths::retail(&definition.script_path(Some(name)));
                let handle = self.assets.renamed.entry(id).or_insert_with(|| server.load(path));
                ready &= self.scripts.contains(&*handle);
            }
        }
        ready
    }
    pub fn place(
        &mut self,
        commands: &mut Commands,
        p: Placing,
        content: &Catalogs,
        placed: &mut HashSet<Entity>,
        rng: &mut GameRng,
        roll: impl FnOnce(&mut GameRng) -> Newborn,
    ) -> Option<(Entity, Vec3, f32, ScaledBody, Option<i32>)> {
        // One with a name of their own is the one NPC of that id; one of a
        // level's placed monsters is made the first time its place is met.
        let here = Placed { map: p.map.to_owned(), location: p.placed.location };
        let bound = content.npcs[p.id].monster_bound;
        let known = self
            .people
            .iter()
            .find(|(_, id, at)| id.0 == p.id && (!bound || *at == Some(&here)))
            .map(|(actor, ..)| actor);
        let actor = match known {
            Some(actor) => actor,
            None if bound => {
                let actor = spawn_record(commands, p.id, content, p.average, self.clock.ms, rng);
                commands.entity(actor).insert(here);
                actor
            }
            None => return None,
        };
        if !placed.insert(actor) {
            return None;
        }
        // One the story has stood somewhere stands there, not where a
        // level placed them.
        if let Ok((killed, gone, member, location, posted)) = self.states.get(actor)
            && (killed
                || posted
                || gone
                || self.dead.contains(actor)
                || member
                || location.is_some_and(|l| l.map != p.map))
        {
            return None;
        }
        let kept = roll(rng);
        commands
            .entity(actor)
            .insert(NpcLocation { map: p.map.into(), species: kept.monster.species });
        Some(bind(commands, actor, kept))
    }
}
fn bind(
    commands: &mut Commands,
    actor: Entity,
    kept: Newborn,
) -> (Entity, Vec3, f32, ScaledBody, Option<i32>) {
    let shape = kept.monster.body;
    let e = commands.spawn((NpcBody(actor), kept.monster, kept.vitals, kept.budget)).id();
    (e, kept.feet, kept.yaw, shape, kept.group)
}

/// Trigger interaction is a binding in its own right, with no synthetic monster.
pub(super) fn bind_triggers(
    devices: Query<
        (Entity, &crate::game::world::triggers::Device, &LevelOwned),
        Added<crate::game::world::triggers::Device>,
    >,
    levels: Query<&crate::game::world::level::Level>,
    geometry: Res<Assets<crate::content::level::LevelDefinition>>,
    content: If<Res<Catalogs>>,
    people: Query<(Entity, &ActorId)>,
    mut commands: Commands,
) {
    for (entity, device, owner) in &devices {
        let Some(level) = levels.get(owner.0).ok().and_then(|l| geometry.get(&l.definition)) else {
            continue;
        };
        let Some(trigger) = level.triggers.get(device.definition) else { continue };
        let Some((id, _)) = content.npcs.0.iter().enumerate().find(|(_, d)| {
            (d.merchant || d.voice_script)
                && trigger.name.eq_ignore_ascii_case(&format!("_{}", d.source_name))
        }) else {
            continue;
        };
        let named = people.iter().find(|(_, actor)| actor.0 == NpcId(id as u16));
        if let Some((actor, _)) = named {
            commands.entity(actor).insert(NpcTrigger(owner.0));
            commands.entity(entity).insert(NpcSpeaker(actor));
        }
    }
}

/// Where the story has stood someone: a named spot of a level, in place of
/// wherever the level placed them.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct Post {
    pub map: String,
    pub spot: String,
}

/// One the story has greet the party, who was not there to do it: they do
/// as they take their place.
#[derive(Component, Clone, Copy, Debug)]
pub struct GreetsOnArrival;

/// Someone the story has stood in the level the party is in takes their
/// place there (retail `RestoreNpcMonster`): a body of the species that is
/// theirs, at the spot named, once that body is loaded.
pub(super) fn station(
    posted: Query<
        (Entity, &ActorId, &Post, Option<&NpcBodies>, Has<GreetsOnArrival>),
        Without<PartyMember>,
    >,
    level: Single<(
        Entity,
        &crate::game::world::level::Level,
        &crate::game::world::navigation::LevelNavigation,
        &mut crate::game::world::population::MonsterBodies,
    )>,
    definitions: Res<Assets<crate::content::level::LevelDefinition>>,
    (bodies, server): (Res<Assets<crate::content::monster::MonsterBody>>, Res<AssetServer>),
    content: If<Res<Catalogs>>,
    mut configs: ResMut<Assets<crate::game::world::navigation::WalkerMotionConfig>>,
    mut rng: ResMut<GameRng>,
    mut commands: Commands,
) {
    let (root, played, navigation, mut handles) = level.into_inner();
    let Some(definition) = definitions.get(&played.definition) else { return };
    for (actor, id, post, standing, greets) in &posted {
        if post.map != played.path || standing.is_some_and(|bodies| !bodies.is_empty()) {
            continue;
        }
        let species = content.species.0.iter().position(|kind| kind.npc == Some(id.0));
        let (Some(species), Some(feet)) = (species, definition.spot(&post.spot)) else { continue };
        let kind = &content.species.0[species];
        let species = species as u32;
        handles
            .0
            .entry(species)
            .or_insert_with(|| server.load(crate::content::paths::monster(&kind.model)));
        let Some(shape) = handles.get(species, &bodies) else { continue };
        let kept = Newborn::rolled((-1, species, None), (feet, 0.0), (kind, shape), &mut rng);
        let scaled = kept.monster.body;
        let mut body = commands.spawn((NpcBody(actor), kept.monster, kept.vitals, kept.budget));
        crate::game::world::population::embody(
            &mut body,
            root,
            (feet, 0.0, scaled),
            (None, kind),
            (navigation, &mut configs),
        );
        commands.entity(actor).insert(NpcLocation { map: played.path.clone(), species });
        if greets {
            commands.entity(actor).remove::<GreetsOnArrival>();
            commands.trigger(conversation::Begin { actor, quote: None, with_held: false });
        }
    }
}
