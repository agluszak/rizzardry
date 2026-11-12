//! Campaign facts and their journal, independent of any active speaker.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::LazyLock;

use bevy::ecs::query::QueryData;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use serde::Deserialize;

use super::conversation::{Begin, Continue, Conversation, PartyRemark, Script as TalkScript};
use super::population::{GreetsOnArrival, Post};
use super::trade::Delivery;
use super::{ActorId, Alias, Gone, Killed, Npc, NpcAssets, NpcBodies};
use crate::content::characters::{Attribute, Profession, Race, Skill};
use crate::content::factions::Faction;
use crate::content::items::ItemId;
use crate::content::level::LevelDefinition;
use crate::content::npcs::*;
use crate::game::actors::Places;
use crate::game::characters::{Character, Condition, Lasting, Vitals};
use crate::game::data::Catalogs;
use crate::game::factions::{Band, FactionNews, Factions};
use crate::game::items::{Items, Place, Put, make, roll_amount};
use crate::game::party::body::{LookAt, PartyBody};
use crate::game::party::roster::{PartyMember, PartyRoster};
use crate::game::rng::GameRng;
use crate::game::time::GameClock;
use crate::game::world::bidden::{Cycle, Played, Script, ScriptAction};
use crate::game::world::facing;
use crate::game::world::hostility::Provocation;
use crate::game::world::hostility::{Stance, Stand};
use crate::game::world::identify::GroupMember;
use crate::game::world::level::{Level, PlayedLevel, SetAside};
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::particles::Emitter;
use crate::game::world::perception::in_view;
use crate::game::world::population::{Coming, Monster};
use crate::game::world::triggers::{self, Bidding};
use crate::game::world::triggers::{LevelState, PartyMove, PendingPartyMove, TriggerEffect};
use avian3d::prelude::{Position, Rotation};
#[derive(Resource, Debug, Default)]
pub struct StoryFacts {
    values: BTreeMap<FactId, u8>,
    /// The facts that, once seen to be so, are from then on what was last
    /// set.
    seen: BTreeSet<FactId>,
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
    pub fn set(&mut self, id: FactId, value: u8, catalog: &FactCatalog, level: &str) {
        let previous = catalog.get(id).map_or(0, |fact| self.stored(fact));
        self.values.insert(id, value);
        if previous != value && catalog.get(id).is_some() {
            self.journal.push(JournalEntry { fact: id, was_true: value != 0, level: level.into() });
        }
    }
}
/// What a named fact was last set to.
pub fn fact_named(story: &StoryFacts, catalog: &FactCatalog, name: &str) -> Result<u8, String> {
    let fact = catalog.0.iter().find(|d| d.symbol == name);
    fact.map(|fact| story.stored(fact)).ok_or_else(|| format!("missing fact {name}"))
}

/// How a fact is read off the world rather than kept (retail
/// `EvaluateFact`): a row of `assets/tables/facts.ron`.
#[derive(Clone, Debug, Default, Deserialize)]
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
    /// A person was killed.
    Killed(u16),
    /// So many of the party's own, companions aside, are awake.
    OwnAwake(usize),
    /// The one Al-Sedexus chose is of the party.
    Chosen,
    /// Whatever the world is.
    #[default]
    Always,
    /// A faction is neither friend nor foe.
    Neutral(Faction),
    /// At least so many of these hold.
    AtLeast(usize, Vec<Reading>),
    /// A variable of the level was never made.
    Unset(String),
    /// The party stands on a named device of the level: within its
    /// bounds, seen from above.
    On(String),
    /// A named door of the level stands open.
    Open(String),
    /// The party's hand holds this item; it holds nothing.
    Holding(u16),
    HandEmpty,
    /// One of a species is in the level.
    Here(u32),
    /// A person stands in the level.
    Stands(u16),
    /// The party holds so many of an item, counting each of a stack.
    Units(u16, u32),
    /// As this reads until it is first seen to hold, and from then on
    /// whatever was last set.
    Once(Box<Reading>),
    /// The party is in a level, by its number.
    Level(u8),
    /// A variable of the level the party is in holds a number.
    LevelVar(String, i32),
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
    TriggerEnabled(String),
    /// The person speaks to the party, handed what its hand holds.
    Offers(u16),
    /// While two named devices close on each other, what stands between
    /// them is pushed along or crushed; crushing one of a species is worth
    /// experience.
    Crushes {
        low: String,
        high: String,
        spared: u32,
        prize: (u32, u32),
    },
    Shoots {
        from: Vec<String>,
        dice: (i32, i32, i32),
    },
    FactionGone(Faction),
    Breeds {
        from: String,
        to: String,
    },
    GroupDrops(u32, u16),
    AfterTalk(Vec<Consequence>),
    WhenStill(String, Vec<Consequence>),
    LevelVarStep(String, i32),
    Travel(String),
    OneOf(Vec<Vec<Consequence>>),
    Every(f32, Reading, Vec<Consequence>),
    SpawnItemAt(u16, (f32, f32, f32)),
    FactNumbered(u16, u8),
    Births(u32),
    Tells(usize),
    Message(i32),
    Experience(u32),
    Generator(String, bool),
    Hurt(i32, i32, i32),
    CastHere(u8, u32),
    LoseHeld,
    HandItem(u16),
    ParticleOff(String),
    /// One of the party is taken: not the first so many of its order, and
    /// of the rest one of those the takers want least, by profession (any
    /// not named counts 1).
    Takes {
        spares: usize,
        wanted: Vec<(Profession, u32)>,
    },
    AfflictOne(Condition, u32, u32),
    RestoreMagic,
    Heal(u32),
    Rest(u32),
    PickMember,
    OthersFaint,
    OthersWake,
    ChosenRemarks(u16),
    AfterBattle(Reading, Vec<Consequence>),
    Ends,
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
    SpawnItem(u16, String),
    SpawnMonster {
        species: u32,
        at: String,
        stance: Stance,
        #[serde(default = "one")]
        count: u32,
        #[serde(default)]
        faces: Option<String>,
        #[serde(default)]
        script: Option<String>,
        /// How far from the spot, in the original's units.
        #[serde(default)]
        shift: (f32, f32, f32),
    },
    LeavesParty(u16),
    /// The person of an id stands at a spot of a level, by its path.
    StandsHere(u16, String, String),
    Greets(u16),
    LookAt(f32, f32, f32),
    EndTalk,
    GroupScript(u32, String),
    GroupGone(u32),
    SelfGone,
    FirstGone(Vec<u16>),
    Moved(u16, String),
    GroupMoved(u32, String),
    Says(u16, u16),
    Unlock(String),
    TriggerOn(String),
    Shows(u16, String),
    GroupShows(u32, String),
    ShowsThen(u16, String, Vec<Consequence>),
    GroupShowsThen(u32, String, Vec<Consequence>),
    GroupDies(u32),
    GreetsOf(u32),
    Later(f32, Vec<Consequence>),
    Sound(String),
    Shake(f32, f32),
    Particle(String),
    CameraPath(String),
    Fade(bool, f32, Vec<Consequence>),
    LookAtSpot(String),
    BodyGone,
    BodyScript(String),
    BodyAsleep(u32),
    BodyFacesParty,
    Engages(u32),
    Stays,
    Drops(u16, u16, String),
    Renamed(u16),
    Restarts(u16),
    MemberRemarks(u16, u16),
    RaceRemarks(Race, u16),
    /// A faction's standing moves by so much.
    FactionShift(Faction, i8),
    /// The chosen one is smitten no longer.
    ChosenFreed,
    /// Every member fit to speaks.
    AllRemark(u16),
    /// When more than two are fit to, one of them other than a person
    /// speaks.
    AnotherRemarks {
        but: u16,
        event: u16,
    },
    /// Every hostile monster of the level dies.
    HostilesDie,
    /// These befall after so many hours of the game's time.
    InHours(u64, Vec<Consequence>),
}

/// When a fact's consequences follow: as it is set, as it is cleared, or
/// either way.
#[derive(Clone, Debug, Deserialize)]
enum Follows {
    Set(Vec<Consequence>),
    Cleared(Vec<Consequence>),
    Always(Vec<Consequence>),
}

fn one() -> u32 {
    1
}

/// The story's facts that are not simply what was last set.
#[derive(Debug, Deserialize)]
pub(super) struct FactTable {
    /// The one Al-Sedexus is offered.
    pub(super) chosen: super::scenes::ChosenTable,
    /// The game's endings: the first whose reading holds is how it ends.
    pub(super) endings: Vec<(Reading, super::scenes::Ending)>,
    /// The facts a new game begins with set.
    begins: Vec<String>,
    derived: HashMap<String, Reading>,
    follows: HashMap<String, Vec<Follows>>,
    /// What each named happening of a monster's script does.
    happenings: HashMap<String, Vec<Consequence>>,
    /// What goes with a marked person's leaving, by the person.
    marked: HashMap<u16, Vec<Consequence>>,
    /// The other name the story may give a person.
    renames: HashMap<u16, String>,
    /// What the death of one of a species sets off.
    slain: HashMap<u32, Vec<Consequence>>,
    /// What the party's taking an item sets off.
    taken: HashMap<u16, Vec<Consequence>>,
}

/// The other name the story may give the person of a kind.
pub fn alias_of(kind: i32) -> Option<&'static str> {
    FACTS.renames.get(&u16::try_from(kind).ok()?).map(String::as_str)
}

/// The story has marked someone: they leave where they stand, for good,
/// once the party has left the level.
#[derive(Component, Clone, Copy, Debug)]
pub struct Marked;

/// A fact has been set, to whatever it already was or not.
#[derive(Event, Clone, Copy, Debug)]
pub struct FactSet {
    pub fact: FactId,
    pub value: u8,
}

/// One thing a fact set off, to be carried out by whoever it concerns.
#[derive(Event, Clone, Debug)]
pub(super) struct Befalls(pub(super) Consequence);

/// The party is told something, by the number of one of the game's
/// strings.
#[derive(Message, Clone, Copy, Debug)]
pub struct StoryTold {
    pub string: usize,
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
    let rows =
        FACTS.follows.get(&fact.symbol).into_iter().flatten().filter_map(|follows| match follows {
            Follows::Set(rows) if set.value != 0 => Some(rows),
            Follows::Cleared(rows) if set.value == 0 => Some(rows),
            Follows::Always(rows) => Some(rows),
            _ => None,
        });
    for row in unfold(rows.flatten(), Some(fact), &readings, &story, &factions) {
        commands.trigger(Befalls(row.clone()));
    }
}

/// The consequences that follow now: each `If` opened or dropped by what
/// the world is as it is asked.
fn unfold<'a>(
    rows: impl Iterator<Item = &'a Consequence>,
    fact: Option<&FactDefinition>,
    readings: &Readings,
    story: &StoryFacts,
    factions: &Factions,
) -> Vec<&'a Consequence> {
    let mut now = Vec::new();
    let mut pending: Vec<&Consequence> = rows.collect();
    pending.reverse();
    while let Some(row) = pending.pop() {
        match row {
            Consequence::If(reading, rows) => {
                if readings.answer(reading, fact, story, factions) != 0 {
                    pending.extend(rows.iter().rev());
                }
            }
            row => now.push(row),
        }
    }
    now
}

/// What befalls one body: it leaves, follows a script, falls asleep,
/// turns to the party or dies; a group joins battle. Whatever else is in
/// the rows befalls as anything a fact sets off does.
#[derive(SystemParam)]
pub(super) struct Bodies<'w, 's> {
    conditions: Query<'w, 's, &'static mut crate::game::characters::Conditions, With<Monster>>,
    places: Places<'w, 's>,
    party: Single<'w, 's, Entity, With<PartyBody>>,
    monsters: Query<'w, 's, (Entity, &'static Monster)>,
    commands: Commands<'w, 's>,
}

impl Bodies<'_, '_> {
    fn befall(&mut self, body: Entity, rows: &[Consequence]) {
        for row in rows {
            match row {
                Consequence::BodyGone => self.commands.entity(body).despawn(),
                Consequence::BodyScript(script) => {
                    self.commands.entity(body).insert(Script(script.clone()));
                }
                Consequence::BodyAsleep(rounds) => {
                    if let Ok(mut conditions) = self.conditions.get_mut(body) {
                        conditions.afflict(Condition::Asleep, Lasting::Rounds(*rounds));
                    }
                }
                Consequence::BodyFacesParty => {
                    let from = self.places.of(body).feet;
                    if let Some(turned) = facing(from, self.places.of(*self.party).feet) {
                        self.commands.entity(body).insert(Rotation(turned));
                    }
                }
                Consequence::Engages(species) => {
                    let first = self.monsters.iter().find(|(_, m)| m.species == *species);
                    if let Some((monster, _)) = first {
                        self.commands.trigger(Provocation { monster });
                    }
                }
                row => self.commands.trigger(Befalls(row.clone())),
            }
        }
    }
}

/// A monster's script comes to one of the story's named happenings
/// (retail `ProcessScript`, `DOACTION`).
pub(super) fn happens(action: On<ScriptAction>, mut bodies: Bodies) {
    if let Some(rows) = FACTS.happenings.get(&action.action) {
        bodies.befall(action.body, rows);
    }
}

/// The screen dims to black, or comes back from it, over so many seconds;
/// the screen says when it has ([`Faded`]), and what was to follow does.
#[derive(Component, Clone, Debug)]
pub struct Fading {
    pub dark: bool,
    pub seconds: f32,
    then: Vec<Consequence>,
}

/// The screen has dimmed, or come back.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Faded {
    #[event_target]
    pub fade: Entity,
}

pub(super) fn faded(faded: On<Faded>, fades: Query<&Fading>, mut commands: Commands) {
    let Ok(fading) = fades.get(faded.fade) else { return };
    for row in &fading.then {
        commands.trigger(Befalls(row.clone()));
    }
    commands.entity(faded.fade).despawn();
}

/// Something that befalls only if a reading holds, wherever it was put off
/// to: asked of the world as it is now.
pub(super) fn befalls_if(
    befalls: On<Befalls>,
    readings: Readings,
    (story, factions): (Res<StoryFacts>, Res<Factions>),
    mut commands: Commands,
) {
    if let Consequence::If(..) = &befalls.0 {
        for row in unfold(std::iter::once(&befalls.0), None, &readings, &story, &factions) {
            commands.trigger(Befalls(row.clone()));
        }
    }
}

/// What is to follow an animation a body plays, once it has been played
/// out.
#[derive(Component, Clone, Debug)]
pub struct After(Vec<Consequence>);

/// An animation has been played out: what was to follow it does.
pub(super) fn follows_cycle(played: On<Played>, after: Query<&After>, mut bodies: Bodies) {
    if let Ok(After(rows)) = after.get(played.body) {
        bodies.commands.entity(played.body).try_remove::<After>();
        bodies.befall(played.body, rows);
    }
}

/// The story has something to say of a death (retail `MonsterKilled`,
/// `HandleScriptedNpcDeath`): what the table gives for the species befalls.
pub(super) fn slain(
    died: On<Add<crate::game::characters::Dead>>,
    monsters: Query<&Monster>,
    readings: Readings,
    (story, factions): (Res<StoryFacts>, Res<Factions>),
    mut commands: Commands,
) {
    let Ok(monster) = monsters.get(died.entity) else { return };
    let rows = FACTS.slain.get(&monster.species).into_iter().flatten();
    for row in unfold(rows, None, &readings, &story, &factions) {
        commands.trigger(Befalls(row.clone()));
    }
}

/// The story has something to say of an item the party takes up (retail
/// `UpdateFactsAfterAcquiringItem`): what the table gives for it befalls.
pub(super) fn taken_up(
    taken: On<Add<crate::game::items::Owned>>,
    items: Query<&crate::game::items::Item>,
    readings: Readings,
    (story, factions): (Res<StoryFacts>, Res<Factions>),
    mut commands: Commands,
) {
    let Ok(item) = items.get(taken.entity) else { return };
    let rows = FACTS.taken.get(&item.0.0).into_iter().flatten();
    for row in unfold(rows, None, &readings, &story, &factions) {
        commands.trigger(Befalls(row.clone()));
    }
}

/// Every hostile monster of the level dies, as anything killed does.
pub(super) fn hostiles_die(
    befalls: On<Befalls>,
    mut monsters: Query<
        (&mut Vitals, &crate::game::world::hostility::Side),
        (With<Monster>, Without<crate::game::characters::Dead>),
    >,
) {
    if !matches!(befalls.0, Consequence::HostilesDie) {
        return;
    }
    for (mut vitals, side) in &mut monsters {
        if side.own() == Stance::Hostile {
            vitals.slay(None);
        }
    }
}

/// Something the story has put off: what befalls once its time has come.
#[derive(Component, Debug)]
pub struct DueAt {
    /// The game clock's time it befalls at.
    ms: u64,
    rows: Vec<Consequence>,
}

/// Something the story has put off: what befalls once its time has come.
#[derive(Component, Debug)]
pub struct Due {
    timer: Timer,
    rows: Vec<Consequence>,
}

impl Due {
    /// What befalls as soon as whatever is under way is done.
    pub(super) fn now(rows: Vec<Consequence>) -> Self {
        Due { timer: Timer::from_seconds(0.0, TimerMode::Once), rows }
    }
}

/// What the story put off befalls as its time comes.
pub(super) fn come_due(
    mut due: Query<(Entity, &mut Due)>,
    mut due_at: Query<(Entity, &mut DueAt)>,
    (time, clock): (Res<Time>, Res<GameClock>),
    mut commands: Commands,
) {
    for (entity, mut due) in &mut due_at {
        if clock.ms >= due.ms {
            for row in due.rows.drain(..) {
                commands.trigger(Befalls(row));
            }
            commands.entity(entity).despawn();
        }
    }
    for (entity, mut due) in &mut due {
        if due.timer.tick(time.delta()).is_finished() {
            for row in due.rows.drain(..) {
                commands.trigger(Befalls(row));
            }
            commands.entity(entity).despawn();
        }
    }
}

/// What befalls the story itself and the party: another fact set, a
/// faction put in a band or marked as met, a skill raised to a floor, an
/// attribute raised.
pub(super) fn befalls_the_story(
    befalls: On<Befalls>,
    (mut story, mut factions): (ResMut<StoryFacts>, ResMut<Factions>),
    mut members: Query<
        (&mut Character, Has<crate::game::actors::CanFight>),
        Without<crate::game::characters::Dead>,
    >,
    (roster, levels): (Single<&PartyRoster>, Query<&Level>),
    (content, clock): (Res<Catalogs>, Res<GameClock>),
    (mut news, mut told, mut remarks): (
        MessageWriter<FactionNews>,
        MessageWriter<StoryTold>,
        MessageWriter<PartyRemark>,
    ),
    mut commands: Commands,
) {
    match &befalls.0 {
        // One the game's records do not name, by its number.
        Consequence::FactNumbered(number, value) => {
            let level = levels.iter().next().map_or("", |level| level.path.as_str());
            story.set(FactId(*number), *value, &content.facts, level);
            commands.trigger(FactSet { fact: FactId(*number), value: *value });
        }
        Consequence::Fact(name, value) => {
            let Some(other) = content.facts.0.iter().find(|d| &d.symbol == name) else { return };
            let level = levels.iter().next().map_or("", |level| level.path.as_str());
            story.set(other.id, *value, &content.facts, level);
            commands.trigger(FactSet { fact: other.id, value: *value });
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
        Consequence::FactionShift(faction, by) => {
            news.write_batch(factions.adjust(*faction, *by, clock.ms));
        }
        Consequence::InHours(hours, rows) => {
            let ms = clock.ms + hours * crate::game::time::HOUR_MS;
            commands.spawn(DueAt { ms, rows: rows.clone() });
        }
        // Each member awake gains up to so much, a hundred at most.
        Consequence::Raise(attribute, by, tells) => {
            for member in roster.iter() {
                if let Ok((mut character, true)) = members.get_mut(member) {
                    let base = character.attributes[*attribute];
                    character.attributes[*attribute] = base + (*by).min(100.max(base) - base);
                }
            }
            told.write(StoryTold { string: *tells });
        }
        // Each member of a race who is awake says something.
        Consequence::RaceRemarks(race, event) => {
            for member in roster.iter() {
                if members
                    .get(member)
                    .is_ok_and(|(character, can_fight)| character.race == *race && can_fight)
                {
                    remarks.write(PartyRemark { event: *event, after: 0.0, speaker: Some(member) });
                }
            }
        }
        Consequence::SkillFloor(skill, points) => {
            for member in roster.iter() {
                if let Ok((mut character, _)) = members.get_mut(member)
                    && character.skill_available(*skill)
                    && character.skill_points[*skill] < *points
                {
                    character.skill_points[*skill] = *points;
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
    leading: Query<(), With<crate::game::world::identify::Leads>>,
    positions: Query<&Position>,
    spots: Spots,
    (content, mut rng): (Res<Catalogs>, ResMut<GameRng>),
    mut emitters: Query<&mut Emitter>,
    mut effects: MessageWriter<TriggerEffect>,
    mut commands: Commands,
) {
    // The leader of the first group of a species.
    let first = |species: &u32| {
        let of_kind = || monsters.iter().filter(|(_, monster, _)| monster.species == *species);
        of_kind().find(|&(body, ..)| leading.contains(body)).or_else(|| of_kind().next())
    };
    let mut bid = |bidding: Bidding| {
        commands.queue(move |world: &mut World| triggers::bid(world, bidding));
    };
    match &befalls.0 {
        Consequence::Trigger(name) => bid(Bidding::Run(name.clone())),
        Consequence::TriggerOnce(name) => bid(Bidding::RunOnce(name.clone())),
        Consequence::TriggerOff(name) => bid(Bidding::SwitchOff(name.clone())),
        Consequence::TriggerOn(name) => bid(Bidding::SwitchOn(name.clone())),
        Consequence::TriggerEnabled(name) => bid(Bidding::Enable(name.clone())),
        Consequence::LevelVarStep(name, count) => {
            bid(Bidding::Step(name.as_str().into(), *count));
        }
        Consequence::Unlock(name) => bid(Bidding::Unlock(name.clone())),
        Consequence::LevelVar(name, value) => bid(Bidding::Set(name.as_str().into(), *value)),
        Consequence::LevelVarOr(name, bits) => {
            bid(Bidding::SetBits(name.as_str().into(), *bits));
        }
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
        // What it leaves lies where it stands, not yet known for what it
        // is.
        Consequence::GroupDrops(species, item) => {
            let at = first(species).and_then(|(leader, ..)| positions.get(leader).ok());
            if let (Some(at), Some(level)) = (at, spots.level()) {
                let definition = &content.items[ItemId(*item)];
                let amount = roll_amount(definition, &mut rng, true);
                let item = make(&mut commands, definition, amount, false);
                commands.trigger(Put { item, at: Place::Ground { level, at: at.0 } });
            }
        }
        // Every group of a faction's kind leaves the level.
        Consequence::FactionGone(faction) => {
            for (body, monster, member) in &monsters {
                if content.species[monster.species].faction == *faction as i32 {
                    commands.entity(body).try_despawn();
                    commands.entity(member.0).try_despawn();
                }
            }
        }
        Consequence::GroupMoved(species, spot) => {
            if let (Some((leader, ..)), Some(at)) = (first(species), spots.named(spot)) {
                commands.entity(leader).insert(Position(at));
            }
        }
        Consequence::GroupShows(species, cycle) => {
            if let Some((body, ..)) = first(species) {
                commands.entity(body).insert(Cycle(cycle.clone()));
            }
        }
        Consequence::GroupShowsThen(species, cycle, rows) => {
            if let Some((body, ..)) = first(species) {
                commands.entity(body).insert((Cycle(cycle.clone()), After(rows.clone())));
            }
        }
        // Its leader dies, as anything killed does.
        Consequence::GroupDies(species) => {
            if let Some((leader, ..)) = first(species) {
                commands.entity(leader).entry::<Vitals>().and_modify(|mut vitals| {
                    vitals.slay(None);
                });
            }
        }
        Consequence::Sound(path) => {
            let path = format!("data/sound/{}", path.to_lowercase());
            effects.write(TriggerEffect::Sound { path, volume: 1.0, at: None });
        }
        Consequence::Shake(seconds, intensity) => {
            effects.write(TriggerEffect::Shake {
                seconds: *seconds,
                intensity: *intensity,
                fade_out: false,
            });
        }
        // A named particle system of the level starts over.
        Consequence::ParticleOff(name) => {
            let named = spots.definition().and_then(|definition| {
                definition.particles.iter().position(|p| p.name.eq_ignore_ascii_case(name))
            });
            for mut emitter in &mut emitters {
                if Some(emitter.definition()) == named {
                    emitter.set_active(false);
                }
            }
        }
        Consequence::Particle(name) => {
            let named = spots.definition().and_then(|definition| {
                definition.particles.iter().position(|p| p.name.eq_ignore_ascii_case(name))
            });
            for mut emitter in &mut emitters {
                if Some(emitter.definition()) == named {
                    emitter.set_active(true);
                }
            }
        }
        // The party is flown along a named camera path of the level.
        Consequence::CameraPath(name) => {
            let path = spots.definition().and_then(|definition| {
                definition.camera_paths.iter().position(|p| p.name.eq_ignore_ascii_case(name))
            });
            if let (Some(path), Some(level)) = (path, spots.level()) {
                commands.entity(level).insert(PendingPartyMove(PartyMove::Fly(path)));
            }
        }
        Consequence::Fade(dark, seconds, rows) => {
            commands.spawn(Fading { dark: *dark, seconds: *seconds, then: rows.clone() });
        }
        // The party goes in by an entrance, of this level or another.
        Consequence::Travel(entrance) => {
            commands.queue({
                let entrance = entrance.clone();
                move |world: &mut World| {
                    world.write_message(crate::game::world::level::Travel { entrance });
                }
            });
        }
        // One of these, each as likely as another.
        Consequence::OneOf(choices) => {
            if !choices.is_empty() {
                let chosen = &choices[rng.below(choices.len() as u32) as usize];
                for row in chosen {
                    commands.trigger(Befalls(row.clone()));
                }
            }
        }
        Consequence::Later(seconds, rows) => {
            let timer = Timer::from_seconds(*seconds, TimerMode::Once);
            commands.spawn(Due { timer, rows: rows.clone() });
        }
        Consequence::LookAtSpot(spot) => {
            if let Some(at) = spots.named(spot) {
                commands.trigger(LookAt { at });
            }
        }
        // The point is given in the original's units.
        Consequence::LookAt(x, y, z) => {
            let at = Vec3::new(*x, *y, -*z) / crate::content::UNITS_PER_METRE;
            commands.trigger(LookAt { at });
        }
        // One of a species that is a person is that person, stood there.
        Consequence::SpawnMonster { species, at: spot, stance, count, faces, script, shift } => {
            let shift = Vec3::new(shift.0, shift.1, -shift.2) / crate::content::UNITS_PER_METRE;
            let person = content.species.kind(*species).and_then(|kind| kind.npc);
            match (person, spots.path()) {
                (Some(id), Some(map)) => {
                    commands.trigger(Befalls(Consequence::StandsHere(id.0, map, spot.clone())));
                }
                _ => {
                    commands.spawn(Coming {
                        species: *species,
                        spot: spot.clone(),
                        stance: *stance,
                        count: *count,
                        faces: faces.clone(),
                        script: script.clone(),
                        shift,
                    });
                }
            }
        }
        // At a point given in the original's units.
        Consequence::SpawnItemAt(item, (x, y, z)) => {
            if let Some(level) = spots.level() {
                let at = Vec3::new(*x, *y, -*z) / crate::content::UNITS_PER_METRE;
                let definition = &content.items[ItemId(*item)];
                let amount = roll_amount(definition, &mut rng, true);
                let item = make(&mut commands, definition, amount, true);
                commands.trigger(Put { item, at: Place::Ground { level, at } });
            }
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
    people: Query<Person>,
    conversations: Query<(Entity, &Conversation)>,
    (spots, places): (Spots, Places),
    (content, clock, mut rng): (Res<Catalogs>, Res<GameClock>, ResMut<GameRng>),
    (mut assets, deliveries): (ResMut<NpcAssets>, Query<(Entity, &Delivery)>),
    mut remarks: MessageWriter<PartyRemark>,
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
                leave(actor, &people, &mut commands);
            }
        }
        Consequence::GoneUnmet(kind) => {
            if let Some((actor, _)) = of_kind(kind, &people)
                && people.get(actor).is_ok_and(|person| person.npc.greeting_pending)
            {
                leave(actor, &people, &mut commands);
            }
        }
        Consequence::FirstGone(kinds) => {
            let standing = kinds.iter().find_map(|kind| match of_kind(kind, &people) {
                Some((actor, Some(_))) => Some(actor),
                _ => None,
            });
            if let Some(actor) = standing {
                leave(actor, &people, &mut commands);
            }
        }
        // Something of theirs is left where they stand, or at a spot if
        // they stand nowhere.
        Consequence::Drops(kind, item, spot) => {
            let body = of_kind(kind, &people).and_then(|(_, body)| body);
            let at = body.map(|body| places.of(body).feet).or_else(|| spots.named(spot));
            if let (Some(at), Some(level)) = (at, spots.level()) {
                let definition = &content.items[ItemId(*item)];
                let amount = roll_amount(definition, &mut rng, true);
                let item = make(&mut commands, definition, amount, true);
                commands.trigger(Put { item, at: Place::Ground { level, at } });
            }
        }
        Consequence::SelfGone => {
            if let Some((_, talk)) = conversations.iter().next() {
                leave(talk.actor, &people, &mut commands);
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
                commands.trigger(Begin { actor, quote: Some(*quote), with_held: false });
            }
        }
        // One who is not there yet greets the party as they arrive.
        Consequence::Greets(kind) => match of_kind(kind, &people) {
            Some((actor, Some(_))) => {
                commands.trigger(Begin { actor, quote: None, with_held: false })
            }
            Some((actor, None)) => {
                commands.entity(actor).insert(GreetsOnArrival);
            }
            None => {}
        },
        Consequence::Offers(kind) => {
            if let Some((actor, _)) = of_kind(kind, &people) {
                commands.trigger(Begin { actor, quote: None, with_held: true });
            }
        }
        // They are of the party no longer, and stand nowhere.
        Consequence::LeavesParty(kind) => {
            if let Some((actor, _)) = of_kind(kind, &people) {
                commands.queue(move |world: &mut World| {
                    let _ = world.run_system_cached_with(super::recruit::dismiss, (actor, false));
                });
            }
        }
        Consequence::Shows(kind, cycle) => {
            if let Some((_, Some(body))) = of_kind(kind, &people) {
                commands.entity(body).insert(Cycle(cycle.clone()));
            }
        }
        Consequence::ShowsThen(kind, cycle, rows) => {
            if let Some((_, Some(body))) = of_kind(kind, &people) {
                commands.entity(body).insert((Cycle(cycle.clone()), After(rows.clone())));
            }
        }
        // The person whose species this is greets the party, as they
        // arrive if they are not there yet.
        Consequence::GreetsOf(species) => {
            let id = content.species.kind(*species).and_then(|kind| kind.npc);
            let found = people.iter().find(|person| Some(person.id.0) == id);
            match found.map(|person| (person.entity, person.bodies.is_some_and(|b| !b.is_empty())))
            {
                Some((actor, true)) => {
                    commands.trigger(Begin { actor, quote: None, with_held: false })
                }
                Some((actor, false)) => {
                    commands.entity(actor).insert(GreetsOnArrival);
                }
                None => {}
            }
        }
        Consequence::Mark(kind) => {
            if let Some((actor, _)) = of_kind(kind, &people) {
                commands.entity(actor).insert(Marked);
            }
        }
        // They go by their other name from now on: its lines and voice are
        // theirs (retail `ReloadNpcScriptResources`).
        Consequence::Renamed(kind) => {
            let renamed = of_kind(kind, &people).and_then(|(actor, _)| {
                let id = people.get(actor).ok()?.id.0;
                Some((actor, id, alias_of(i32::from(*kind))?, assets.renamed.get(&id)?.clone()))
            });
            if let Some((actor, id, name, lines)) = renamed {
                assets.scripts.insert(id, lines);
                commands.entity(actor).insert(Alias(name.to_owned()));
            }
        }
        // Whatever the one spoken with was saying, they say this instead.
        Consequence::Restarts(quote) => {
            for (conversation, _) in &conversations {
                let quote = *quote;
                commands.entity(conversation).entry::<TalkScript>().and_modify(
                    move |mut script| {
                        script.restart(quote);
                    },
                );
                commands.trigger(Continue { conversation });
            }
        }
        Consequence::MemberRemarks(kind, event) => {
            if let Some((actor, _)) = of_kind(kind, &people) {
                remarks.write(PartyRemark { event: *event, after: 0.0, speaker: Some(actor) });
            }
        }
        Consequence::StandsHere(id, map, spot) => {
            let found = people.iter().find(|person| person.id.0.0 == *id).map(|p| p.entity);
            if let Some(actor) = found {
                leave(actor, &people, &mut commands);
                commands
                    .entity(actor)
                    .remove::<Gone>()
                    .insert(Post { map: map.clone(), spot: spot.clone() });
            }
        }
        // They leave wherever they stood and stand there instead, when
        // the party is next in that level.
        Consequence::Stands(kind, level, spot) => {
            let map = crate::content::levels::LEVELS.get(usize::from(*level));
            if let (Some((actor, _)), Some(map)) = (of_kind(kind, &people), map) {
                leave(actor, &people, &mut commands);
                commands
                    .entity(actor)
                    .remove::<Gone>()
                    .insert(Post { map: map.path(), spot: spot.clone() });
            }
        }
        // The original counts the wait in seconds.
        Consequence::Stock(kind, item, units, after) => {
            if let Some((merchant, _)) = of_kind(kind, &people) {
                let due = clock.ms + u64::from(*after) * 1000;
                commands.spawn(Delivery { merchant, item: ItemId(*item), units: *units, due });
            }
        }
        // What they were yet to get, they do not.
        Consequence::Unstock(kind, item) => {
            if let Some((merchant, _)) = of_kind(kind, &people) {
                let due = deliveries.iter().filter(|(_, delivery)| {
                    delivery.merchant == merchant && delivery.item == ItemId(*item)
                });
                for (delivery, _) in due {
                    commands.entity(delivery).despawn();
                }
            }
        }
        _ => {}
    }
}

/// A marked person's turn comes (retail `HandleMarkedNpcEvent`): what goes
/// with their leaving befalls, and unless that keeps them they leave where
/// they stood, for good.
fn depart(
    actor: Entity,
    people: &Query<Person>,
    (readings, story, factions): (&Readings, &StoryFacts, &Factions),
    content: &Catalogs,
    commands: &mut Commands,
) {
    let Ok(kind) = people.get(actor).map(|person| content.npcs[person.id.0].kind) else { return };
    let rows = u16::try_from(kind).ok().and_then(|kind| FACTS.marked.get(&kind));
    let rows = unfold(rows.into_iter().flatten(), None, readings, story, factions);
    let stays = rows.iter().any(|row| matches!(row, Consequence::Stays));
    for row in rows {
        commands.trigger(Befalls(row.clone()));
    }
    if !stays {
        leave(actor, people, commands);
    }
    commands.entity(actor).remove::<Marked>();
}

/// The party leaves a level (retail `ReleaseMarkedNpcBindings`): each
/// marked person's turn comes.
pub(super) fn marked_leave(
    _: On<Add<SetAside>>,
    marked: Query<Entity, With<Marked>>,
    people: Query<Person>,
    readings: Readings,
    (story, factions, content): (Res<StoryFacts>, Res<Factions>, Res<Catalogs>),
    mut commands: Commands,
) {
    for actor in &marked {
        depart(actor, &people, (&readings, &story, &factions), &content, &mut commands);
    }
}

/// A marked person's turn comes as soon as the party has no clear line to
/// them (retail `Sight.cpp`), while nobody is being spoken with.
pub(super) fn marked_slip_away(
    marked: Query<Entity, With<Marked>>,
    people: Query<Person>,
    bodies: Query<&Monster>,
    (places, obstacles, level): (Places, WorldObstacles, PlayedLevel),
    party: Single<Entity, With<PartyBody>>,
    conversations: Query<(), With<Conversation>>,
    readings: Readings,
    (story, factions, content): (Res<StoryFacts>, Res<Factions>, Res<Catalogs>),
    mut commands: Commands,
) {
    let Some(far) = level.view_distance() else { return };
    if !conversations.is_empty() {
        return;
    }
    let eye = places.of(*party).eye;
    for actor in &marked {
        let body = people.get(actor).ok().and_then(|person| person.bodies?.first().copied());
        let seen = body.and_then(|body| Some((body, bodies.get(body).ok()?))).is_some_and(
            |(body, monster)| in_view((eye, far), monster, places.of(body).feet, &obstacles),
        );
        if body.is_some() && !seen {
            depart(actor, &people, (&readings, &story, &factions), &content, &mut commands);
        }
    }
}

/// A person as the story finds them.
#[derive(QueryData)]
pub(super) struct Person {
    entity: Entity,
    id: &'static ActorId,
    npc: &'static Npc,
    bodies: Option<&'static NpcBodies>,
}

/// The one of a kind, by the kind their record gives, and the body they
/// stand in.
pub(super) fn one_of_kind(
    kind: u16,
    people: &Query<Person>,
    content: &Catalogs,
) -> Option<(Entity, Option<Entity>)> {
    let found = people.iter().find(|person| content.npcs[person.id.0].kind == i32::from(kind));
    found.map(|person| (person.entity, person.bodies.and_then(|bodies| bodies.first().copied())))
}

/// Someone leaves where they stood, for good.
fn leave(actor: Entity, people: &Query<Person>, commands: &mut Commands) {
    if let Ok(person) = people.get(actor) {
        commands.entity(actor).insert(Gone);
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

    fn definition(&self) -> Option<&LevelDefinition> {
        let (_, level) = self.levels.iter().next()?;
        self.definitions.get(&level.definition)
    }

    fn path(&self) -> Option<String> {
        self.levels.iter().next().map(|(_, level)| level.path.clone())
    }

    fn named(&self, name: &str) -> Option<Vec3> {
        let (_, level) = self.levels.iter().next()?;
        self.definitions.get(&level.definition)?.spot(name)
    }
}

pub(super) static FACTS: LazyLock<FactTable> = LazyLock::new(|| {
    ron::from_str(include_str!("../../../assets/tables/facts.ron"))
        .expect("assets/tables/facts.ron is a table of facts")
});

#[derive(SystemParam)]
pub struct Readings<'w, 's> {
    party: Single<'w, 's, (Entity, &'static PartyRoster)>,
    members: Query<
        'w,
        's,
        (&'static Character, Has<crate::game::actors::CanFight>, Option<&'static ActorId>),
    >,
    held: Items<'w, 's>,
    slain: Query<'w, 's, &'static ActorId, With<Killed>>,
    chosen: Query<'w, 's, (), (With<super::scenes::Chosen>, With<PartyMember>)>,
    standing: Query<'w, 's, (&'static ActorId, &'static NpcBodies)>,
    names: Query<'w, 's, &'static triggers::DeviceNames>,
    doors: Query<'w, 's, &'static triggers::DoorState>,
    parts: Query<'w, 's, &'static Children>,
    bounds: Query<'w, 's, &'static avian3d::prelude::ColliderAabb>,
    footing: Query<'w, 's, &'static Position, With<PartyBody>>,
    bodies: Query<'w, 's, &'static Monster>,
    levels: Query<'w, 's, (&'static Level, &'static LevelState)>,
    content: Res<'w, Catalogs>,
}

impl Readings<'_, '_> {
    /// Whether a fact is so: what it was last set to, or what the world
    /// says if it is one read off the world.
    /// One the game's records do not name was never set, and reads as
    /// unset.
    pub fn read(&self, id: FactId, story: &StoryFacts, factions: &Factions) -> u8 {
        let facts = &self.content.facts;
        let Some(fact) = facts.get(id) else { return 0 };
        match FACTS.derived.get(&fact.symbol) {
            Some(reading) => self.answer(reading, Some(fact), story, factions),
            None => story.stored(fact),
        }
    }

    /// Read a fact, noting one that stays once seen (retail's latch on the
    /// covert training).
    pub fn read_noting(&self, id: FactId, story: &mut StoryFacts, factions: &Factions) -> u8 {
        let value = self.read(id, story, factions);
        let once =
            self.content.facts.get(id).is_some_and(|fact| {
                matches!(FACTS.derived.get(&fact.symbol), Some(Reading::Once(_)))
            });
        if once && value != 0 {
            story.seen.insert(id);
        }
        value
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

    pub(super) fn answer(
        &self,
        reading: &Reading,
        fact: Option<&FactDefinition>,
        story: &StoryFacts,
        factions: &Factions,
    ) -> u8 {
        let members = || self.party.1.iter().filter_map(|member| self.members.get(member).ok());
        let so = |reading: &Reading| self.answer(reading, fact, story, factions) != 0;
        u8::from(match reading {
            Reading::Is(name) => {
                let other = self.content.facts.0.iter().find(|d| &d.symbol == name);
                return other.map_or(0, |other| self.read(other.id, story, factions));
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
            Reading::Joined(npc) => {
                members().any(|(_, can_fight, id)| id.is_some_and(|id| id.0.0 == *npc) && can_fight)
            }
            Reading::Companions(count) => {
                members().filter(|(_, _, id)| id.is_some()).count() == *count
            }
            Reading::FitMember(race) => {
                members().any(|(character, can_fight, _)| character.race == *race && can_fight)
            }
            Reading::Not(reading) => !so(reading),
            Reading::All(readings) => readings.iter().all(so),
            Reading::Any(readings) => readings.iter().any(so),
            Reading::SetAnd(reading) => {
                let stored = fact.map_or(0, |fact| story.stored(fact));
                return if so(reading) { stored } else { 0 };
            }
            Reading::Chosen => !self.chosen.is_empty(),
            Reading::Always => true,
            Reading::Open(name) => {
                let device = self.names.iter().find_map(|names| names.named(name));
                device.and_then(|device| self.doors.get(device).ok()).is_some_and(|door| door.open)
            }
            Reading::On(name) => {
                let device = self.names.iter().find_map(|names| names.named(name));
                let parts = device.and_then(|device| self.parts.get(device).ok());
                let at = self.footing.iter().next();
                parts.zip(at).is_some_and(|(parts, at)| {
                    parts.iter().filter_map(|part| self.bounds.get(part).ok()).any(|bounds| {
                        (bounds.min.x..=bounds.max.x).contains(&at.x)
                            && (bounds.min.z..=bounds.max.z).contains(&at.z)
                    })
                })
            }
            Reading::Neutral(faction) => factions.band(*faction as i32) == Band::Neutral,
            Reading::AtLeast(count, readings) => {
                readings.iter().filter(|reading| so(reading)).count() >= *count
            }
            Reading::Unset(name) => {
                let name = name.as_str().into();
                !self.levels.iter().any(|(_, state)| state.made(&name))
            }
            Reading::Holding(item) => {
                let held = self.held.held(self.party.0);
                held.and_then(|held| self.held.id(held)) == Some(ItemId(*item))
            }
            Reading::HandEmpty => self.held.held(self.party.0).is_none(),
            Reading::Here(species) => self.bodies.iter().any(|body| body.species == *species),
            Reading::Stands(kind) => self.standing.iter().any(|(id, bodies)| {
                self.content.npcs[id.0].kind == i32::from(*kind) && !bodies.is_empty()
            }),
            Reading::Units(item, count) => {
                let (party, roster) = *self.party;
                let held = roster
                    .iter()
                    .flat_map(|member| self.held.carried(member))
                    .chain(self.held.pool(party).iter().copied())
                    .chain(self.held.held(party))
                    .filter(|&held| self.held.id(held) == Some(ItemId(*item)));
                held.map(|held| self.held.units(held)).sum::<u32>() >= *count
            }
            Reading::Once(reading) => match fact {
                Some(fact) if story.seen.contains(&fact.id) => return story.stored(fact),
                _ => so(reading),
            },
            Reading::OwnAwake(count) => {
                let own = members().filter(|(_, can_fight, id)| id.is_none() && *can_fight);
                own.count() >= *count
            }
            Reading::Level(number) => {
                let asked = crate::content::levels::LEVELS.get(usize::from(*number));
                let asked = asked.map(|level| level.path());
                self.levels.iter().any(|(level, _)| Some(&level.path) == asked.as_ref())
            }
            Reading::LevelVar(name, value) => {
                let name = name.as_str().into();
                self.levels.iter().any(|(_, state)| state.value(&name) == *value)
            }
            Reading::Killed(kind) => {
                self.slain.iter().any(|id| self.content.npcs[id.0].kind == i32::from(*kind))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(reading: &Reading, names: &mut Vec<String>) {
        match reading {
            Reading::Is(name) => names.push(name.clone()),
            Reading::Not(reading) | Reading::SetAnd(reading) | Reading::Once(reading) => {
                named(reading, names);
            }
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
                    Consequence::InHours(_, rows) => set(rows, names),
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
        for rows in FACTS.slain.values().chain(FACTS.taken.values()) {
            set(rows, &mut names);
        }
        for name in names {
            assert!(catalog.0.iter().any(|fact| fact.symbol == name), "{name}");
        }
    }
}
