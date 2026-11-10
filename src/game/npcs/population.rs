use super::*;
use crate::{
    content::{level::MonsterPlacement, npcs::*},
    game::{
        characters::{Character, Vitals},
        data::Catalogs,
        party::roster::{PartyMember, PartyRoster},
        rng::GameRng,
        time::GameClock,
        world::{
            ActorPose,
            identify::{GroupMember, MonsterGroup},
            level::{Destination, Entry, KeptMonster, LevelOwned},
            navigation::{self, Walker},
            population::{Monster, ScaledBody, SpellBudget},
        },
    },
};
use bevy::ecs::system::SystemParam;
use std::collections::HashSet;

pub(super) fn initialize(
    content: If<Res<Catalogs>>,
    mut registry: ResMut<Registry>,
    mut commands: Commands,
    mut rng: ResMut<GameRng>,
    clock: Res<GameClock>,
    party: Query<&PartyRoster>,
    characters: Query<&Character>,
    destinations: Query<&Destination>,
) {
    if registry.initialized {
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
        let actor =
            spawn_record(&mut commands, NpcId(index as u16), &content, average, clock.ms, &mut rng);
        registry.named.insert(NpcId(index as u16), actor);
    }
    registry.initialized = true;
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
        record.insert(recruit::character(template, content, rng));
    }
    if def.owns_stock {
        record.insert(trade::MerchantStock::new(def, &content.items, average, now, rng));
    }
    record.id()
}
#[derive(SystemParam)]
pub struct NpcPopulation<'w, 's> {
    registry: ResMut<'w, Registry>,
    assets: ResMut<'w, NpcAssets>,
    scripts: Res<'w, Assets<DialogueScript>>,
    states: Query<'w, 's, (&'static Npc, Has<PartyMember>, Option<&'static mut NpcLocation>)>,
    bodies: Query<
        'w,
        's,
        (
            &'static NpcBody,
            &'static Monster,
            &'static Vitals,
            &'static SpellBudget,
            ActorPose,
            Option<&'static Walker>,
            &'static LevelOwned,
            Option<&'static GroupMember>,
        ),
        Without<crate::game::party::body::PartyBody>,
    >,
    groups: Query<'w, 's, &'static MonsterGroup>,
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
        if !self.registry.initialized {
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
            let handle = self.assets.scripts.entry(id).or_insert_with(|| {
                server.load(crate::content::paths::retail(&content.npcs[id].script_path()))
            });
            assert!(
                !matches!(server.get_load_state(&*handle), Some(bevy::asset::LoadState::Failed(_))),
                "NPC script failed to load"
            );
            ready &= self.scripts.contains(&*handle);
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
        roll: impl FnOnce(&mut GameRng) -> KeptMonster,
    ) -> Option<(Entity, Vec3, f32, ScaledBody, Option<i32>)> {
        let actor = if content.npcs[p.id].monster_bound {
            let key = (p.map.to_owned(), p.placed.location);
            if let Some(&actor) = self.registry.placed.get(&key) {
                actor
            } else {
                let actor = spawn_record(commands, p.id, content, p.average, self.clock.ms, rng);
                self.registry.placed.insert(key, actor);
                actor
            }
        } else {
            *self.registry.named.get(&p.id)?
        };
        if !placed.insert(actor) {
            return None;
        }
        let remembered = if let Ok((npc, member, location)) = self.states.get(actor) {
            if npc.dead || member {
                return None;
            }
            if location.is_some_and(|l| l.map != p.map) {
                return None;
            }
            location.and_then(|l| l.dormant.clone())
        } else {
            None
        };
        let kept = remembered.unwrap_or_else(|| roll(rng));
        if kept.vitals.is_dead() {
            return None;
        }
        commands.entity(actor).insert(NpcLocation {
            map: p.map.into(),
            species: kept.monster.species,
            dormant: None,
        });
        Some(bind(commands, actor, kept))
    }
    pub fn visitors(
        &mut self,
        commands: &mut Commands,
        map: &str,
        placed: &mut HashSet<Entity>,
    ) -> Vec<(Entity, Vec3, f32, ScaledBody, Option<i32>, u32)> {
        let mut found = vec![];
        for &actor in self.registry.named.values() {
            let Ok((npc, member, Some(location))) = self.states.get(actor) else { continue };
            if npc.dead || member || location.map != map || !placed.insert(actor) {
                continue;
            }
            let Some(kept) = location.dormant.clone() else { continue };
            let species = location.species;
            commands.entity(actor).insert(NpcLocation { map: map.into(), species, dormant: None });
            let (e, at, yaw, body, group) = bind(commands, actor, kept);
            found.push((e, at, yaw, body, group, species));
        }
        found
    }
    pub fn depart(&mut self, _commands: &mut Commands, root: Entity) {
        for (binding, monster, vitals, budget, pose, walker, owner, group) in &self.bodies {
            if owner.0 != root {
                continue;
            }
            let Ok((_, _, Some(mut location))) = self.states.get_mut(binding.0) else { continue };
            let body = KeptMonster {
                monster: monster.clone(),
                vitals: vitals.clone(),
                budget: *budget,
                feet: navigation::feet(pose.translation(), walker),
                yaw: pose.rotation().to_euler(EulerRot::YXZ).0,
                group: group.and_then(|g| self.groups.get(g.0).ok()).map(|g| g.authored),
            };
            location.dormant = Some(body);
        }
        // LevelOwned despawns the physical bindings with the departing level.
    }
}
fn bind(
    commands: &mut Commands,
    actor: Entity,
    kept: KeptMonster,
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
    registry: Res<Registry>,
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
        if let Some(&actor) = registry.named.get(&NpcId(id as u16)) {
            commands.entity(actor).insert(NpcTrigger(owner.0));
            commands.entity(entity).insert(NpcSpeaker(actor));
        }
    }
}
