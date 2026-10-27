//! Native saves: the campaign as ordinary records, written as RON.
//!
//! A save is taken only when play is settled (exploring, nothing pending),
//! and holds what play cannot rebuild: the party (members in order, their
//! characters, vitals, belongings and knowledge, formation and pose), what
//! it shares, the level being played and every level it has left (as the
//! records travel keeps), the clock with its remainder, item identities and
//! the gameplay RNG's state, so play continues as it would have. Physics,
//! navigation and presentation are rebuilt from these.
//!
//! Loading is a level entry ([`Entry::Restore`]): it prepares the saved
//! level like any other and, at the commit, replaces the campaign with the
//! save's. It never grants starting equipment, rolls a population or opens
//! anything again.

use std::path::Path;

use avian3d::prelude::Position;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::game::characters::{Character, MonsterKnowledge, Vitals};
use crate::game::combat::Battle;
use crate::game::difficulty::Difficulty;
use crate::game::factions::Factions;
use crate::game::inventory::{Carried, ItemIds, ItemStack, PartyInventory};
use crate::game::magic::casting::{Aim, MagicChoice, Source, SpellInFlight};
use crate::game::magic::{LearnedSpells, SpellPoints};
use crate::game::party::Order;
use crate::game::party::body::{EYE_ABOVE_CENTRE, PartyBody, PartyStart};
use crate::game::party::formation::{Formation, Row};
use crate::game::party::roster::{NeedsEquipment, Party, PartyMember, PartyRoster};
use crate::game::rng::GameRng;
use crate::game::time::GameClock;
use crate::game::world::level::LevelOwned;
use crate::game::world::level::{
    Destination, Entry, Level, LevelContents, LevelEntities, LevelMemory, LevelRecord,
};
use crate::game::world::population::Monster;
use crate::game::world::triggers::{LevelState, PendingPartyMove};

/// Bumped whenever a save's layout changes; older saves are refused.
const VERSION: u32 = 2;

/// A whole campaign at a settled moment.
#[derive(Serialize, Deserialize, Debug)]
pub struct SaveRecord {
    version: u32,
    /// The level being played, by path.
    pub level: String,
    played: LevelRecord,
    /// The levels the party has left, by path.
    dormant: Vec<(String, LevelRecord)>,
    party: SavedParty,
    clock: GameClock,
    item_ids: ItemIds,
    rng: GameRng,
    factions: Factions,
    difficulty: Difficulty,
}

/// The party, its members in roster order. Members are referred to by
/// their place in that order, never by runtime entity.
#[derive(Serialize, Deserialize, Debug)]
struct SavedParty {
    members: Vec<SavedMember>,
    /// Each seated member's row and seat.
    seats: Vec<(usize, Row, usize)>,
    pool: Vec<ItemStack>,
    gold: u32,
    /// Where its body's centre is, and where it looks.
    position: Vec3,
    yaw: f32,
    pitch: f32,
}

#[derive(Serialize, Deserialize, Debug)]
struct SavedMember {
    character: Character,
    vitals: Vitals,
    /// Absent until a new member's starting equipment is handed out.
    carried: Option<Carried>,
    order: SavedOrder,
    learned: LearnedSpells,
    spell_points: Option<SpellPoints>,
    knowledge: Option<MonsterKnowledge>,
}

/// Saved choices name actors by campaign identity, never runtime entity IDs.
#[derive(Serialize, Deserialize, Debug)]
enum SavedAim {
    Member(usize),
    Monster(i32),
    Unavailable,
}
#[derive(Serialize, Deserialize, Debug)]
enum SavedOrder {
    Attack,
    Defend,
    Magic { source: Source, aim: SavedAim },
}
impl SavedOrder {
    fn capture(order: Order, world: &World, members: &[Entity], level: Entity) -> Self {
        match order {
            Order::Attack => Self::Attack,
            Order::Defend => Self::Defend,
            Order::Magic(choice) => {
                let aim = match choice.aim {
                    Aim::Member(actor) => members
                        .iter()
                        .position(|&m| m == actor)
                        .map_or(SavedAim::Unavailable, SavedAim::Member),
                    Aim::Monster(actor) => world
                        .get::<Monster>(actor)
                        .filter(|monster| !monster.is_dead())
                        .filter(|_| {
                            world.get::<LevelOwned>(actor).is_some_and(|owner| owner.0 == level)
                        })
                        .map_or(SavedAim::Unavailable, |monster| {
                            SavedAim::Monster(monster.location)
                        }),
                    Aim::Unavailable => SavedAim::Unavailable,
                };
                Self::Magic { source: choice.source, aim }
            }
        }
    }
    fn restore(
        self,
        members: &[Entity],
        monsters: &std::collections::HashMap<i32, Entity>,
    ) -> Order {
        match self {
            Self::Attack => Order::Attack,
            Self::Defend => Order::Defend,
            Self::Magic { source, aim } => {
                let aim = match aim {
                    SavedAim::Member(index) => members.get(index).copied().map(Aim::Member),
                    SavedAim::Monster(location) => {
                        monsters.get(&location).copied().map(Aim::Monster)
                    }
                    SavedAim::Unavailable => None,
                }
                .unwrap_or(Aim::Unavailable);
                Order::Magic(MagicChoice { source, aim })
            }
        }
    }
}

/// Why the campaign cannot be saved now.
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("there is nothing to save")]
    NoCampaign,
    #[error("the party is busy")]
    Unsettled,
    #[error("put down the item in hand first")]
    ItemInHand,
    #[error("cannot write the save: {0}")]
    Io(#[from] std::io::Error),
    #[error("save format: {0}")]
    Format(String),
}

/// The campaign as it is now, if play is settled: a level is played and
/// the party is free (no screen, camp, question, flight or entry), not
/// fighting, with nothing in hand and no move pending.
pub fn capture(world: &mut World) -> Result<SaveRecord, SaveError> {
    let free = world.run_system_cached(crate::game::screen::party_free).unwrap_or(false);
    let fighting = world.query::<&Battle>().iter(world).next().is_some();
    let Ok((level, path, pending)) = world
        .query_filtered::<(Entity, &Level, Has<PendingPartyMove>), Without<Destination>>()
        .single(world)
        .map(|(entity, level, pending)| (entity, level.path.clone(), pending))
    else {
        return Err(SaveError::NoCampaign);
    };
    if !free || fighting || pending || world.query::<&SpellInFlight>().iter(world).next().is_some()
    {
        return Err(SaveError::Unsettled);
    }
    let party = crate::game::party::roster::the_party(world).ok_or(SaveError::NoCampaign)?;
    if world.get::<PartyInventory>(party).is_some_and(|i| i.hand.held.is_some()) {
        return Err(SaveError::ItemInHand);
    }

    let mut contents = SystemState::<LevelContents>::new(world);
    let played = {
        let state = world.get::<LevelState>(level).unwrap();
        let owned = world.get::<LevelEntities>(level).map_or(&[][..], LevelEntities::entities);
        contents.get(world).unwrap().remember(level, state, owned)
    };
    let members: Vec<Entity> =
        world.get::<PartyRoster>(party).map_or_else(Vec::new, |roster| roster.iter().collect());
    let formation = world.get::<Formation>(party).unwrap();
    let seats = members
        .iter()
        .enumerate()
        .filter_map(|(index, &member)| {
            formation.place_of(member).map(|(row, seat)| (index, row, seat))
        })
        .collect();
    let saved_members = members
        .iter()
        .map(|&member| {
            let actor = world.entity(member);
            SavedMember {
                character: actor.get::<Character>().unwrap().clone(),
                vitals: actor.get::<Vitals>().unwrap().clone(),
                carried: actor.get::<Carried>().cloned(),
                order: SavedOrder::capture(
                    actor.get::<Order>().copied().unwrap_or_default(),
                    world,
                    &members,
                    level,
                ),
                learned: actor.get::<LearnedSpells>().cloned().unwrap_or_default(),
                spell_points: actor.get::<SpellPoints>().cloned(),
                knowledge: actor.get::<MonsterKnowledge>().cloned(),
            }
        })
        .collect();
    let inventory = world.get::<PartyInventory>(party).unwrap();
    let body = world.get::<PartyBody>(party).ok_or(SaveError::NoCampaign)?;
    let position = world.get::<Position>(party).ok_or(SaveError::NoCampaign)?.0;
    let party = SavedParty {
        members: saved_members,
        seats,
        pool: inventory.pool.stacks.clone(),
        gold: inventory.gold,
        position,
        yaw: body.yaw,
        pitch: body.pitch,
    };
    let dormant = world.resource::<LevelMemory>().records();
    Ok(SaveRecord {
        version: VERSION,
        level: path,
        played,
        dormant,
        party,
        clock: *world.resource::<GameClock>(),
        item_ids: world.resource::<ItemIds>().clone(),
        rng: world.resource::<GameRng>().clone(),
        factions: world.resource::<Factions>().clone(),
        difficulty: *world.resource::<Difficulty>(),
    })
}

/// Write a save as RON, whole.
pub fn write(path: &Path, record: &SaveRecord) -> Result<(), SaveError> {
    let text = ron::ser::to_string_pretty(record, ron::ser::PrettyConfig::default())
        .map_err(|error| SaveError::Format(error.to_string()))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    std::fs::write(&temp, text)?;
    Ok(std::fs::rename(&temp, path)?)
}

/// Read a save of this version.
pub fn read(path: &Path) -> Result<SaveRecord, SaveError> {
    let text = std::fs::read_to_string(path)?;
    let record: SaveRecord =
        ron::from_str(&text).map_err(|error| SaveError::Format(error.to_string()))?;
    if record.version != VERSION {
        return Err(SaveError::Format(format!("save version {} is not {VERSION}", record.version)));
    }
    Ok(record)
}

impl SaveRecord {
    /// Where the party stands in the saved level.
    pub fn start(&self) -> PartyStart {
        PartyStart {
            entrance: None,
            eye: Some(self.party.position + Vec3::Y * EYE_ABOVE_CENTRE),
            yaw: Some(self.party.yaw),
            pitch: self.party.pitch,
        }
    }

    /// The saved level's record, and those of the levels left.
    pub(crate) fn into_levels(self) -> (LevelRecord, Vec<(String, LevelRecord)>, Restored) {
        let restored = Restored {
            party: self.party,
            clock: self.clock,
            item_ids: self.item_ids,
            rng: self.rng,
            factions: self.factions,
            difficulty: self.difficulty,
        };
        (self.played, self.dormant, restored)
    }
}

/// What a restore puts back besides the levels, at the entry's commit.
pub(crate) struct Restored {
    party: SavedParty,
    clock: GameClock,
    item_ids: ItemIds,
    rng: GameRng,
    factions: Factions,
    difficulty: Difficulty,
}

impl Restored {
    /// Put the saved party and campaign state in place of whatever there
    /// was. Returns the party, to be placed in the level.
    pub(crate) fn apply(
        self,
        commands: &mut Commands,
        monsters: &std::collections::HashMap<i32, Entity>,
    ) -> Entity {
        let saved = self.party;
        let party = commands
            .spawn((
                Name::new("Party"),
                Party,
                PartyInventory {
                    pool: crate::game::inventory::ItemPool { stacks: saved.pool },
                    gold: saved.gold,
                    ..default()
                },
            ))
            .id();
        // Reserve the whole party before restoring references between members.
        let members: Vec<_> = saved.members.iter().map(|_| commands.spawn_empty().id()).collect();
        for (entity, member) in members.iter().copied().zip(saved.members) {
            let order = member.order.restore(&members, monsters);
            let mut actor = commands.entity(entity);
            actor.insert((
                Name::new(member.character.name.clone()),
                member.character,
                member.vitals,
                order,
                member.learned,
                PartyMember(party),
            ));
            match member.carried {
                Some(carried) => actor.insert(carried),
                None => actor.insert(NeedsEquipment),
            };
            if let Some(knowledge) = member.knowledge {
                actor.insert(knowledge);
            }
            if let Some(points) = member.spell_points {
                actor.insert(points);
            }
        }
        let mut formation = Formation::default();
        for (index, row, seat) in saved.seats {
            if let Some(&member) = members.get(index) {
                formation.put(member, row, seat);
            }
        }
        commands.entity(party).insert(formation);
        commands.insert_resource(self.clock);
        commands.insert_resource(self.item_ids);
        commands.insert_resource(self.rng);
        commands.insert_resource(self.factions);
        commands.insert_resource(self.difficulty);
        party
    }
}

/// Load a save: request its level as an entry that restores the campaign.
pub fn load(world: &mut World, record: SaveRecord) -> Result<(), SaveError> {
    if world.query::<&Destination>().iter(world).next().is_some() {
        return Err(SaveError::Unsettled);
    }
    let level = Level::load(&record.level, world.resource::<AssetServer>());
    let start = record.start();
    world.spawn((
        Name::new(record.level.clone()),
        Destination::new(level, start, Entry::Restore(Box::new(record))),
    ));
    Ok(())
}

/// Save the campaign to a file.
#[derive(Message, Debug, Clone)]
pub struct SaveGame(pub std::path::PathBuf);

/// Load a campaign from a file.
#[derive(Message, Debug, Clone)]
pub struct LoadGame(pub std::path::PathBuf);

/// What became of a save or load request.
#[derive(Message, Debug, Clone)]
pub enum SaveNews {
    Saved,
    Loading,
    Failed(String),
}

pub struct SavePlugin;

impl Plugin for SavePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SaveGame>()
            .add_message::<LoadGame>()
            .add_message::<SaveNews>()
            .add_systems(PreUpdate, handle_requests.after(crate::game::world::level::travel));
    }
}

/// Carry out save and load requests, in order.
fn handle_requests(world: &mut World) {
    let saves: Vec<_> = world.resource_mut::<Messages<SaveGame>>().drain().collect();
    let loads: Vec<_> = world.resource_mut::<Messages<LoadGame>>().drain().collect();
    for SaveGame(path) in saves {
        let news = match capture(world).and_then(|record| write(&path, &record)) {
            Ok(()) => {
                info!("saved {}", path.display());
                SaveNews::Saved
            }
            Err(error) => {
                warn!("save to {} refused: {error}", path.display());
                SaveNews::Failed(error.to_string())
            }
        };
        world.write_message(news);
    }
    if let Some(LoadGame(path)) = loads.into_iter().last() {
        let news = match read(&path).and_then(|record| load(world, record)) {
            Ok(()) => SaveNews::Loading,
            Err(error) => {
                warn!("cannot load {}: {error}", path.display());
                SaveNews::Failed(error.to_string())
            }
        };
        world.write_message(news);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_expired_saved_target_preserves_the_choice_without_retargeting() {
        let source =
            Source::Spell { spell: crate::content::spells::SpellId::ENERGY_BLAST, power: 3 };
        let saved = SavedOrder::Magic { source, aim: SavedAim::Monster(42) };
        assert_eq!(
            saved.restore(&[], &default()),
            Order::Magic(MagicChoice { source, aim: Aim::Unavailable }),
        );
    }
}
