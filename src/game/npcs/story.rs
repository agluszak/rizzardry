//! Campaign facts and their journal, independent of any active speaker.
use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

use bevy::ecs::query::QueryData;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use serde::Deserialize;

use super::conversation::{Begin, Conversation};
use super::trade::Delivery;
use super::{ActorId, Npc, NpcBodies};
use crate::content::characters::{Attribute, Race, Skill};
use crate::content::factions::Faction;
use crate::content::items::ItemId;
use crate::content::level::LevelDefinition;
use crate::content::npcs::*;
use crate::game::actors::Places;
use crate::game::actors::Refit;
use crate::game::characters::{Character, Condition, Lasting, Vitals};
use crate::game::data::Catalogs;
use crate::game::factions::{Band, FactionNews, Factions};
use crate::game::items::{Items, Place, Put, make, roll_amount};
use crate::game::party::body::PartyBody;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::time::GameClock;
use crate::game::world::bidden::{Script, ScriptAction, ScriptShow};
use crate::game::world::facing;
use crate::game::world::hostility::Provocation;
use crate::game::world::hostility::{Stance, Stand};
use crate::game::world::identify::GroupMember;
use crate::game::world::level::Level;
use crate::game::world::population::Monster;
use crate::game::world::triggers::{self, Bidding};
use avian3d::prelude::{Position, Rotation};
#[derive(Resource, Debug, Default)]
pub struct StoryFacts {
    values: BTreeMap<FactId, u8>,
    pub topics: Vec<(TopicCategory, String)>,
    pub journal: Vec<JournalEntry>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEntry {
    pub fact: FactId,
    pub was_true: bool,
    pub level: String,
}
impl JournalEntry {
    pub fn description<'a>(&self, catalog: &'a FactCatalog) -> &'a str {
        let d = &catalog[self.fact];
        if self.was_true { &d.alternate_description } else { &d.description }
    }
}
impl StoryFacts {
    pub fn remember_topic(&mut self, category: TopicCategory, text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() || self.topics.iter().any(|(_, s)| s.eq_ignore_ascii_case(text)) {
            return false;
        }
        self.topics.push((category, text.to_owned()));
        true
    }
    pub fn forget_topic(&mut self, text: &str) -> bool {
        let before = self.topics.len();
        self.topics.retain(|(_, s)| !s.eq_ignore_ascii_case(text.trim()));
        self.topics.len() != before
    }
    /// What a fact was last set to; one never set is as the game begins.
    pub fn stored(&self, fact: &FactDefinition) -> u8 {
        let begins = || u8::from(FACTS.begins.contains(&fact.symbol));
        self.values.get(&fact.id).copied().unwrap_or_else(begins)
    }
    /// Whether everything setting a fact sets off can be carried out,
    /// through whatever other facts it sets.
    pub fn writable(id: FactId, catalog: &FactCatalog) -> bool {
        fn ported(rows: &[Consequence], catalog: &FactCatalog, depth: u32) -> bool {
            rows.iter().all(|row| match row {
                Consequence::Fact(name, _) => {
                    depth < 8
                        && catalog.0.iter().find(|d| &d.symbol == name).is_some_and(|other| {
                            follows(other).all(|rows| ported(rows, catalog, depth + 1))
                        })
                }
                Consequence::If(_, rows) => ported(rows, catalog, depth),
                // What this game cannot carry out yet.
                Consequence::Raise(..)
                | Consequence::Returns
                | Consequence::Unstock(..)
                | Consequence::Mark(_)
                | Consequence::Stands(..)
                | Consequence::Reload(_)
                | Consequence::SpawnMonster(..)
                | Consequence::Scene(_) => false,
                _ => true,
            })
        }
        catalog.get(id).is_some_and(|fact| follows(fact).all(|rows| ported(rows, catalog, 0)))
    }
    pub fn set(
        &mut self,
        id: FactId,
        value: u8,
        catalog: &FactCatalog,
        level: &str,
    ) -> Result<(), String> {
        if !Self::writable(id, catalog) {
            return Err(format!("unimplemented fact consequences: {id:?}"));
        }
        let previous = self.stored(&catalog[id]);
        self.values.insert(id, value);
        if previous != value {
            self.journal.push(JournalEntry { fact: id, was_true: value != 0, level: level.into() });
        }
        Ok(())
    }
}
/// What a named fact was last set to.
pub fn fact_named(story: &StoryFacts, catalog: &FactCatalog, name: &str) -> Result<u8, String> {
    let fact = catalog.0.iter().find(|d| d.symbol == name);
    fact.map(|fact| story.stored(fact)).ok_or_else(|| format!("missing fact {name}"))
}

/// How a fact is read off the world rather than kept (retail
/// `EvaluateFact`): a row of `assets/tables/facts.ron`.
#[derive(Clone, Debug, Deserialize)]
pub enum Reading {
    /// As another fact is.
    Is(String),
    /// The party holds an item.
    Has(u16),
    /// How many of these items the party holds, each counted once.
    Held {
        of: Vec<u16>,
        at_least: usize,
        at_most: usize,
    },
    /// Every member holds an item.
    EveryoneHas(u16),
    Friendly(Faction),
    /// A person is in the party, and awake.
    Joined(u16),
    /// So many people have joined the party.
    Companions(usize),
    /// A member of a race is awake.
    FitMember(Race),
    Not(Box<Reading>),
    All(Vec<Reading>),
    Any(Vec<Reading>),
    /// What was last set, but only while this holds.
    SetAnd(Box<Reading>),
    /// Read off something this game does not have yet.
    Unported,
}

/// Something that setting a fact sets off (retail `HandleFactChange`): a
/// row of `assets/tables/facts.ron`, where each kind is described.
#[derive(Clone, Debug, Deserialize)]
pub enum Consequence {
    Fact(String, u8),
    If(Reading, Vec<Consequence>),
    Faction(Faction, Stance),
    FactionMet(Faction),
    Group(u32, Stance),
    SkillFloor(Skill, u32),
    Raise(Attribute, u32, usize),
    Returns,
    Trigger(String),
    TriggerOnce(String),
    TriggerOff(String),
    LevelVar(String, i32),
    LevelVarOr(String, i32),
    Stock(u16, u16, u32, u32),
    Unstock(u16, u16),
    Mark(u16),
    Gone(u16),
    GoneUnmet(u16),
    Stands(u16, u8, String),
    Script(u16, String),
    Reload(u16),
    SpawnItem(u16, String),
    SpawnMonster(u32, String),
    LookAt(f32, f32, f32),
    EndTalk,
    GroupScript(u32, String),
    GroupGone(u32),
    SelfGone,
    Moved(u16, String),
    GroupMoved(u32, String),
    Says(u16, u16),
    Unlock(String),
    TriggerOn(String),
    Shows(u16, String),
    GroupShows(u32, String),
    LookAtSpot(String),
    BodyGone,
    BodyScript(String),
    BodyAsleep(u32),
    BodyFacesParty,
    Engages(u32),
    Scene(String),
}

/// When a fact's consequences follow: as it is set, as it is cleared, or
/// either way.
#[derive(Clone, Debug, Deserialize)]
enum Follows {
    Set(Vec<Consequence>),
    Cleared(Vec<Consequence>),
    Always(Vec<Consequence>),
}

/// The story's facts that are not simply what was last set.
#[derive(Debug, Deserialize)]
struct FactTable {
    /// The facts a new game begins with set.
    begins: Vec<String>,
    derived: HashMap<String, Reading>,
    follows: HashMap<String, Vec<Follows>>,
    /// What each named happening of a monster's script does.
    happenings: HashMap<String, Vec<Consequence>>,
}

/// Everything setting a fact may set off, whatever it is set to.
fn follows(fact: &FactDefinition) -> impl Iterator<Item = &'static [Consequence]> {
    FACTS.follows.get(&fact.symbol).into_iter().flatten().map(|follows| match follows {
        Follows::Set(rows) | Follows::Cleared(rows) | Follows::Always(rows) => rows.as_slice(),
    })
}

/// A fact has been set, to whatever it already was or not.
#[derive(Event, Clone, Copy, Debug)]
pub struct FactSet {
    pub fact: FactId,
    pub value: u8,
}

/// One thing a fact set off, to be carried out by whoever it concerns.
#[derive(Event, Clone, Debug)]
pub(super) struct Befalls(Consequence);

/// The party is to look somewhere, for the screen to turn it.
#[derive(Message, Clone, Copy, Debug)]
pub struct StoryLook {
    pub at: Vec3,
}

/// What setting a fact sets off begins (retail `HandleFactChange`): each
/// consequence that follows now, its conditions asked of the world as it
/// is, befalls in the order the table gives.
pub(super) fn follow(
    set: On<FactSet>,
    readings: Readings,
    (story, factions): (Res<StoryFacts>, Res<Factions>),
    content: Res<Catalogs>,
    mut commands: Commands,
) {
    let Some(fact) = content.facts.get(set.fact) else { return };
    let mut pending: Vec<&Consequence> = FACTS
        .follows
        .get(&fact.symbol)
        .into_iter()
        .flatten()
        .filter_map(|follows| match follows {
            Follows::Set(rows) if set.value != 0 => Some(rows),
            Follows::Cleared(rows) if set.value == 0 => Some(rows),
            Follows::Always(rows) => Some(rows),
            _ => None,
        })
        .flatten()
        .rev()
        .collect();
    while let Some(row) = pending.pop() {
        match row {
            Consequence::If(reading, rows) => {
                if readings.answer(reading, fact, &story, &factions).is_ok_and(|so| so != 0) {
                    pending.extend(rows.iter().rev());
                }
            }
            row => commands.trigger(Befalls(row.clone())),
        }
    }
}

/// A monster's script comes to one of the story's named happenings
/// (retail `ProcessScript`, `DOACTION`): what befalls the monster itself is
/// done here, and the rest befalls as anything a fact sets off does.
pub(super) fn happens(
    action: On<ScriptAction>,
    mut bodies: Query<&mut Vitals, With<Monster>>,
    (places, party): (Places, Single<Entity, With<PartyBody>>),
    monsters: Query<(Entity, &Monster)>,
    mut commands: Commands,
) {
    let body = action.body;
    for row in FACTS.happenings.get(&action.action).into_iter().flatten() {
        match row {
            Consequence::BodyGone => commands.entity(body).despawn(),
            Consequence::BodyScript(script) => {
                commands.entity(body).insert(Script(script.clone()));
            }
            Consequence::BodyAsleep(rounds) => {
                if let Ok(mut vitals) = bodies.get_mut(body) {
                    vitals.afflict(Condition::Asleep, Lasting::Rounds(*rounds));
                }
            }
            Consequence::BodyFacesParty => {
                let turned = facing(places.of(body).feet, places.of(*party).feet);
                if let Some(turned) = turned {
                    commands.entity(body).insert(Rotation(turned));
                }
            }
            Consequence::Engages(species) => {
                let first = monsters.iter().find(|(_, monster)| monster.species == *species);
                if let Some((monster, _)) = first {
                    commands.trigger(Provocation { monster });
                }
            }
            row => commands.trigger(Befalls(row.clone())),
        }
    }
}

/// What befalls the story itself and the party: another fact set, a
/// faction put in a band or marked as met, a skill raised to a floor.
pub(super) fn befalls_the_story(
    befalls: On<Befalls>,
    (mut story, mut factions): (ResMut<StoryFacts>, ResMut<Factions>),
    mut members: Query<(&mut Character, &Vitals)>,
    (roster, levels): (Single<&PartyRoster>, Query<&Level>),
    (content, clock): (Res<Catalogs>, Res<GameClock>),
    mut news: MessageWriter<FactionNews>,
    mut commands: Commands,
) {
    match &befalls.0 {
        Consequence::Fact(name, value) => {
            let Some(other) = content.facts.0.iter().find(|d| &d.symbol == name) else { return };
            let level = levels.iter().next().map_or("", |level| level.path.as_str());
            if story.set(other.id, *value, &content.facts, level).is_ok() {
                commands.trigger(FactSet { fact: other.id, value: *value });
            }
        }
        Consequence::Faction(faction, stance) => {
            let band = match stance {
                Stance::Hostile => Band::Hostile,
                Stance::Neutral => Band::Neutral,
                Stance::Friendly => Band::Friendly,
            };
            news.write_batch(factions.set_band(*faction, band, clock.ms));
        }
        Consequence::FactionMet(faction) => factions.encounter(*faction as i32),
        Consequence::SkillFloor(skill, points) => {
            for member in roster.iter() {
                if let Ok((mut character, vitals)) = members.get_mut(member)
                    && !vitals.is_dead()
                    && character.skill_available(*skill)
                    && character.skill_points[*skill] < *points
                {
                    character.skill_points[*skill] = *points;
                    commands.trigger(Refit { entity: member });
                }
            }
        }
        _ => {}
    }
}

/// What befalls the level the party is in: a device run, switched or
/// unlocked, a variable set, and what happens to the first group of a
/// species there: it turns, follows a script, or its leader is moved,
/// leaves or plays an animation; an item appearing at a spot; the party
/// made to look somewhere.
pub(super) fn befalls_the_level(
    befalls: On<Befalls>,
    monsters: Query<(Entity, &Monster, &GroupMember)>,
    spots: Spots,
    (content, mut rng): (Res<Catalogs>, ResMut<GameRng>),
    mut shows: MessageWriter<ScriptShow>,
    mut looks: MessageWriter<StoryLook>,
    mut commands: Commands,
) {
    let first = |species: &u32| monsters.iter().find(|(_, monster, _)| monster.species == *species);
    let mut bid = |bidding: Bidding| {
        commands.queue(move |world: &mut World| triggers::bid(world, bidding));
    };
    match &befalls.0 {
        // A level's variables are set or not; the original's numbers only
        // ever ask whether one is.
        Consequence::Trigger(name) => bid(Bidding::Run(name.clone())),
        Consequence::TriggerOnce(name) => bid(Bidding::RunOnce(name.clone())),
        Consequence::TriggerOff(name) => bid(Bidding::SwitchOff(name.clone())),
        Consequence::TriggerOn(name) => bid(Bidding::SwitchOn(name.clone())),
        Consequence::Unlock(name) => bid(Bidding::Unlock(name.clone())),
        Consequence::LevelVar(name, value) => {
            bid(Bidding::Set(name.as_str().into(), *value != 0));
        }
        Consequence::LevelVarOr(name, _) => bid(Bidding::Set(name.as_str().into(), true)),
        Consequence::Group(species, stance) => {
            if let Some((_, _, member)) = first(species) {
                commands.trigger(Stand { group: member.0, stance: Some(*stance) });
            }
        }
        Consequence::GroupScript(species, script) => {
            if let Some((leader, ..)) = first(species) {
                commands.entity(leader).insert(Script(script.clone()));
            }
        }
        Consequence::GroupGone(species) => {
            if let Some((leader, ..)) = first(species) {
                commands.entity(leader).despawn();
            }
        }
        Consequence::GroupMoved(species, spot) => {
            if let (Some((leader, ..)), Some(at)) = (first(species), spots.named(spot)) {
                commands.entity(leader).insert(Position(at));
            }
        }
        Consequence::GroupShows(species, cycle) => {
            if let Some((body, ..)) = first(species) {
                shows.write(ScriptShow { body, command: format!("CYCLE {cycle}") });
            }
        }
        Consequence::LookAtSpot(spot) => {
            looks.write_batch(spots.named(spot).map(|at| StoryLook { at }));
        }
        // The point is given in the original's units.
        Consequence::LookAt(x, y, z) => {
            let at = Vec3::new(*x, *y, -*z) / crate::content::UNITS_PER_METRE;
            looks.write(StoryLook { at });
        }
        Consequence::SpawnItem(item, spot) => {
            if let (Some(at), Some(level)) = (spots.named(spot), spots.level()) {
                let definition = &content.items[ItemId(*item)];
                let amount = roll_amount(definition, &mut rng, true);
                let item = make(&mut commands, definition, amount, true);
                commands.trigger(Put { item, at: Place::Ground { level, at } });
            }
        }
        _ => {}
    }
}

/// What befalls a person: they leave where they stood for good, are moved,
/// follow a script, speak up or play an animation; a merchant is to get
/// something; the conversation under way ends.
pub(super) fn befalls_someone(
    befalls: On<Befalls>,
    mut people: Query<Person>,
    conversations: Query<(Entity, &Conversation)>,
    spots: Spots,
    (content, clock): (Res<Catalogs>, Res<GameClock>),
    mut shows: MessageWriter<ScriptShow>,
    mut commands: Commands,
) {
    let of_kind = |kind: &u16, people: &Query<Person>| one_of_kind(*kind, people, &content);
    match &befalls.0 {
        Consequence::EndTalk => {
            for (conversation, _) in &conversations {
                commands.entity(conversation).despawn();
            }
        }
        Consequence::Gone(kind) => {
            if let Some((actor, _)) = of_kind(kind, &people) {
                leave(actor, &mut people, &mut commands);
            }
        }
        Consequence::GoneUnmet(kind) => {
            if let Some((actor, _)) = of_kind(kind, &people)
                && people.get(actor).is_ok_and(|person| person.npc.greeting_pending)
            {
                leave(actor, &mut people, &mut commands);
            }
        }
        Consequence::SelfGone => {
            if let Some((_, talk)) = conversations.iter().next() {
                leave(talk.actor, &mut people, &mut commands);
            }
        }
        Consequence::Moved(kind, spot) => {
            if let (Some((_, Some(body))), Some(at)) = (of_kind(kind, &people), spots.named(spot)) {
                commands.entity(body).insert(Position(at));
            }
        }
        Consequence::Script(kind, script) => {
            if let Some((_, Some(body))) = of_kind(kind, &people) {
                commands.entity(body).insert(Script(script.clone()));
            }
        }
        Consequence::Says(kind, quote) => {
            if let Some((actor, _)) = of_kind(kind, &people) {
                commands.trigger(Begin { actor, quote: Some(*quote) });
            }
        }
        Consequence::Shows(kind, cycle) => {
            if let Some((_, Some(body))) = of_kind(kind, &people) {
                shows.write(ScriptShow { body, command: format!("CYCLE {cycle}") });
            }
        }
        // The original counts the wait in seconds.
        Consequence::Stock(kind, item, units, after) => {
            if let Some((merchant, _)) = of_kind(kind, &people) {
                let due = clock.ms + u64::from(*after) * 1000;
                commands.spawn(Delivery { merchant, item: ItemId(*item), units: *units, due });
            }
        }
        _ => {}
    }
}

/// A person as the story finds them.
#[derive(QueryData)]
#[query_data(mutable)]
pub(super) struct Person {
    entity: Entity,
    id: &'static ActorId,
    npc: &'static mut Npc,
    bodies: Option<&'static NpcBodies>,
}

/// The one of a kind, by the kind their record gives, and the body they
/// stand in.
fn one_of_kind(
    kind: u16,
    people: &Query<Person>,
    content: &Catalogs,
) -> Option<(Entity, Option<Entity>)> {
    let found = people.iter().find(|person| content.npcs[person.id.0].kind == i32::from(kind));
    found.map(|person| (person.entity, person.bodies.and_then(|bodies| bodies.first().copied())))
}

/// Someone leaves where they stood, for good.
fn leave(actor: Entity, people: &mut Query<Person>, commands: &mut Commands) {
    if let Ok(mut person) = people.get_mut(actor) {
        person.npc.gone = true;
        for body in person.bodies.into_iter().flat_map(|bodies| bodies.iter()) {
            commands.entity(body).despawn();
        }
    }
}

/// The named spots of the level the party is in.
#[derive(SystemParam)]
pub(super) struct Spots<'w, 's> {
    levels: Query<'w, 's, (Entity, &'static Level)>,
    definitions: Res<'w, Assets<LevelDefinition>>,
}

impl Spots<'_, '_> {
    fn level(&self) -> Option<Entity> {
        self.levels.iter().next().map(|(level, _)| level)
    }

    fn named(&self, name: &str) -> Option<Vec3> {
        let (_, level) = self.levels.iter().next()?;
        self.definitions.get(&level.definition)?.spot(name)
    }
}

static FACTS: LazyLock<FactTable> = LazyLock::new(|| {
    ron::from_str(include_str!("../../../assets/tables/facts.ron"))
        .expect("assets/tables/facts.ron is a table of facts")
});

/// Why a fact cannot be read yet, if it cannot.
pub fn unreadable(id: FactId, catalog: &FactCatalog) -> Option<String> {
    let Some(fact) = catalog.get(id) else { return Some(format!("unknown fact {id:?}")) };
    matches!(FACTS.derived.get(&fact.symbol), Some(Reading::Unported))
        .then(|| format!("unimplemented fact predicate: {}", fact.symbol))
}

/// What the story's facts are read off: the party, what it holds and who
/// has joined it.
#[derive(SystemParam)]
pub struct Readings<'w, 's> {
    party: Single<'w, 's, (Entity, &'static PartyRoster)>,
    members: Query<'w, 's, (&'static Character, &'static Vitals, Option<&'static ActorId>)>,
    held: Items<'w, 's>,
    content: Res<'w, Catalogs>,
}

impl Readings<'_, '_> {
    /// Whether a fact is so: what it was last set to, or what the world
    /// says if it is one read off the world.
    pub fn read(&self, id: FactId, story: &StoryFacts, factions: &Factions) -> Result<u8, String> {
        let facts = &self.content.facts;
        let fact = facts.get(id).ok_or_else(|| format!("unknown fact {id:?}"))?;
        match FACTS.derived.get(&fact.symbol) {
            Some(reading) => self.answer(reading, fact, story, factions),
            None => Ok(story.stored(fact)),
        }
    }

    fn holds(&self, item: u16) -> bool {
        let (party, roster) = *self.party;
        roster
            .iter()
            .flat_map(|member| self.held.carried(member))
            .chain(self.held.pool(party).iter().copied())
            .chain(self.held.held(party))
            .any(|held| self.held.id(held) == Some(ItemId(item)))
    }

    fn answer(
        &self,
        reading: &Reading,
        fact: &FactDefinition,
        story: &StoryFacts,
        factions: &Factions,
    ) -> Result<u8, String> {
        let members = || self.party.1.iter().filter_map(|member| self.members.get(member).ok());
        let so = |reading: &Reading| -> Result<bool, String> {
            self.answer(reading, fact, story, factions).map(|value| value != 0)
        };
        // Every one of them holds; none of them holds.
        let every = |readings: &[Reading]| {
            readings.iter().map(so).try_fold(true, |all, one| Ok::<_, String>(all && one?))
        };
        let any = |readings: &[Reading]| {
            readings.iter().map(so).try_fold(false, |any, one| Ok::<_, String>(any || one?))
        };
        Ok(u8::from(match reading {
            Reading::Is(name) => {
                let other = self.content.facts.0.iter().find(|d| &d.symbol == name);
                let other = other.ok_or_else(|| format!("missing fact {name}"))?;
                return self.read(other.id, story, factions);
            }
            Reading::Has(item) => self.holds(*item),
            Reading::Held { of, at_least, at_most } => {
                let held = of.iter().filter(|&&item| self.holds(item)).count();
                (*at_least..=*at_most).contains(&held)
            }
            Reading::EveryoneHas(item) => self.party.1.iter().all(|member| {
                self.held.carried(member).any(|held| self.held.id(held) == Some(ItemId(*item)))
            }),
            Reading::Friendly(faction) => factions.band(*faction as i32) == Band::Friendly,
            Reading::Joined(npc) => members()
                .any(|(_, vitals, id)| id.is_some_and(|id| id.0.0 == *npc) && vitals.can_fight()),
            Reading::Companions(count) => {
                members().filter(|(_, _, id)| id.is_some()).count() == *count
            }
            Reading::FitMember(race) => members()
                .any(|(character, vitals, _)| character.race == *race && vitals.can_fight()),
            Reading::Not(reading) => !so(reading)?,
            Reading::All(readings) => every(readings)?,
            Reading::Any(readings) => any(readings)?,
            Reading::SetAnd(reading) => {
                return Ok(if so(reading)? { story.stored(fact) } else { 0 });
            }
            Reading::Unported => {
                return Err(format!("unimplemented fact predicate: {}", fact.symbol));
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(reading: &Reading, names: &mut Vec<String>) {
        match reading {
            Reading::Is(name) => names.push(name.clone()),
            Reading::Not(reading) | Reading::SetAnd(reading) => named(reading, names),
            Reading::All(readings) | Reading::Any(readings) => {
                readings.iter().for_each(|reading| named(reading, names));
            }
            _ => {}
        }
    }

    /// Every fact the table names is one of the original's.
    #[test]
    #[ignore = "requires a retail installation"]
    fn the_facts_the_table_names_are_the_originals() {
        let files = crate::retail::files::retail_files();
        let data = files.read("data/databases/fact.dbs").unwrap().unwrap();
        let catalog = crate::retail::npcs::read_facts(&data).unwrap();
        let mut names: Vec<String> = FACTS.begins.clone();
        for (name, reading) in &FACTS.derived {
            names.push(name.clone());
            named(reading, &mut names);
        }
        fn set(rows: &[Consequence], names: &mut Vec<String>) {
            for row in rows {
                match row {
                    Consequence::Fact(name, _) => names.push(name.clone()),
                    Consequence::If(reading, rows) => {
                        named(reading, names);
                        set(rows, names);
                    }
                    _ => {}
                }
            }
        }
        for (name, follows) in &FACTS.follows {
            names.push(name.clone());
            for follows in follows {
                let (Follows::Set(rows) | Follows::Cleared(rows) | Follows::Always(rows)) = follows;
                set(rows, &mut names);
            }
        }
        for name in names {
            assert!(catalog.0.iter().any(|fact| fact.symbol == name), "{name}");
        }
    }
}
