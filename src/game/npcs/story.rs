//! Campaign facts and their journal, independent of any active speaker.
use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use serde::Deserialize;

use super::ActorId;
use crate::content::characters::{Attribute, Race, Skill};
use crate::content::factions::Faction;
use crate::content::items::ItemId;
use crate::content::npcs::*;
use crate::game::actors::Refit;
use crate::game::characters::{Character, Vitals};
use crate::game::data::Catalogs;
use crate::game::factions::{Band, FactionNews, Factions};
use crate::game::items::Items;
use crate::game::party::roster::PartyRoster;
use crate::game::time::GameClock;
use crate::game::world::hostility::{Stance, Stand};
use crate::game::world::identify::GroupMember;
use crate::game::world::level::Level;
use crate::game::world::population::Monster;
use crate::game::world::triggers::{self, Bidding};
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
                Consequence::Faction(..)
                | Consequence::FactionMet(_)
                | Consequence::Group(..)
                | Consequence::SkillFloor(..)
                | Consequence::Trigger(_)
                | Consequence::TriggerOnce(_)
                | Consequence::TriggerOff(_)
                | Consequence::LevelVar(..)
                | Consequence::LevelVarOr(..) => true,
                _ => false,
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

/// What setting a fact sets off is carried out: another fact set, a
/// faction put in a band or marked as met, a group turned, a skill raised
/// to a floor, a device of the level run or switched off, a variable of the
/// level set.
pub(super) fn follow(
    set: On<FactSet>,
    mut world: ParamSet<(Readings, Query<(&mut Character, &Vitals)>)>,
    (mut story, mut factions): (ResMut<StoryFacts>, ResMut<Factions>),
    monsters: Query<(&Monster, &GroupMember)>,
    (roster, levels): (Single<&PartyRoster>, Query<&Level>),
    (content, clock): (Res<Catalogs>, Res<GameClock>),
    mut news: MessageWriter<FactionNews>,
    mut commands: Commands,
) {
    let Some(fact) = content.facts.get(set.fact) else { return };
    // What follows now, the conditions asked of the world as it is.
    let mut now: Vec<&Consequence> = Vec::new();
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
    let readings = world.p0();
    while let Some(row) = pending.pop() {
        match row {
            Consequence::If(reading, rows) => {
                if readings.answer(reading, fact, &story, &factions).is_ok_and(|so| so != 0) {
                    pending.extend(rows.iter().rev());
                }
            }
            row => now.push(row),
        }
    }
    let level = levels.iter().next().map_or("", |level| level.path.as_str());
    for row in now {
        match row {
            Consequence::Fact(name, value) => {
                let Some(other) = content.facts.0.iter().find(|d| &d.symbol == name) else {
                    continue;
                };
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
            Consequence::Group(species, stance) => {
                let first = monsters.iter().find(|(monster, _)| monster.species == *species);
                if let Some((_, member)) = first {
                    commands.trigger(Stand { group: member.0, stance: Some(*stance) });
                }
            }
            Consequence::SkillFloor(skill, points) => {
                let mut members = world.p1();
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
            // A level's variables are set or not; the original's numbers
            // only ever ask whether one is.
            Consequence::Trigger(name) => works(&mut commands, Bidding::Run(name.clone())),
            Consequence::TriggerOnce(name) => works(&mut commands, Bidding::RunOnce(name.clone())),
            Consequence::TriggerOff(name) => works(&mut commands, Bidding::SwitchOff(name.clone())),
            Consequence::LevelVar(name, value) => {
                works(&mut commands, Bidding::Set(name.as_str().into(), *value != 0));
            }
            Consequence::LevelVarOr(name, _) => {
                works(&mut commands, Bidding::Set(name.as_str().into(), true));
            }
            _ => {}
        }
    }
}

/// The story works a device of the level, once what set it off is done.
fn works(commands: &mut Commands, bidding: Bidding) {
    commands.queue(move |world: &mut World| triggers::bid(world, bidding));
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
