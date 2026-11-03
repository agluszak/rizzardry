//! Conversations with NPCs: retail's dialogue scripts run by gameplay, with
//! their waits, prices, bribes, charms, trades and recruitment. Authored
//! continuation and waits belong to gameplay, not the dialogue view.
use super::{
    Npc,
    trade::{self, MerchantStock, TradeError, TradeQuote, TradeSource},
};
use crate::content::factions::Faction;
use crate::content::{items::ItemId, npcs::*};
use crate::game::data::Catalogs;
use crate::game::factions::{Band, FactionNews, Factions};
use crate::game::freedom::Freedom;
use crate::game::time::GameClock;
use crate::game::{
    characters::{Character, Condition, Vitals},
    combat::{Battle, Combatant},
    inventory::{Carried, ItemIds, ItemOwner, ItemRef, ItemStack, PartyInventory, make_item},
    party::{body::PartyBody, formation::Formation, roster::PartyRoster},
    rng::GameRng,
    world::{
        ActorPose, facing,
        hostility::{self, Stance},
        identify::{GroupMember, MonsterGroup},
        level::{Destination, Level, LevelOwned},
        navigation::Walker,
        population::Monster,
    },
};
use avian3d::prelude::{Position, Rotation};
use bevy::{
    ecs::{entity::EntityHashSet, query::QueryData, system::SystemParam},
    prelude::*,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Engine semantics refer to the database's symbolic keys; script operands and
/// campaign storage keep the actual decoded identifiers. No Burz record or
/// fact number is embedded in the interpreter.
#[derive(Resource, Debug, Default)]
pub struct StoryFacts {
    values: BTreeMap<FactId, u8>,
    pub topics: Vec<(TopicCategory, String)>,
    pub journal: Vec<JournalEntry>,
}
/// A fact transition, retaining the level it happened on and whether the
/// fact became true, which picks its description (retail `W8JournalEntry`'s
/// `alternate_text`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalEntry {
    pub fact: FactId,
    pub was_true: bool,
    pub level: String,
}
impl JournalEntry {
    pub fn description<'a>(&self, catalog: &'a FactCatalog) -> &'a str {
        let fact = &catalog[self.fact];
        if self.was_true { &fact.alternate_description } else { &fact.description }
    }
}
impl StoryFacts {
    /// The keyword notebook outlives a conversation. Learning/discussing a
    /// keyword does not consume it; only the explicit minus button removes it.
    pub fn remember_topic(&mut self, category: TopicCategory, text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() || self.topics.iter().any(|(_, known)| known.eq_ignore_ascii_case(text))
        {
            return false;
        }
        self.topics.push((category, text.to_owned()));
        true
    }

    /// Forget a notebook entry without changing discovered facts or journal history.
    pub fn forget_topic(&mut self, text: &str) -> bool {
        let text = text.trim();
        let before = self.topics.len();
        self.topics.retain(|(_, known)| !known.eq_ignore_ascii_case(text));
        self.topics.len() != before
    }

    /// A fact's value before anything sets it: a new game starts its
    /// virgin facts true.
    fn initial(fact: &crate::content::npcs::FactDefinition) -> u8 {
        u8::from(matches!(
            fact.symbol.as_str(),
            "FACT_VIRGIN" | "FACT_QUEST_VIRGIN_1" | "FACT_QUEST_VIRGIN_2" | "FACT_QUEST_VIRGIN_3"
        ))
    }
    pub fn get(&self, id: FactId, catalog: &FactCatalog) -> Result<u8, String> {
        let fact = catalog.get(id).ok_or_else(|| format!("missing fact definition {id:?}"))?;
        match fact.symbol.as_str() {
            // Plain stored facts (retail `EvaluateFact` reads them unchanged).
            "FACT_VIRGIN"
            | "FACT_IMPORT"
            | "FACT_BURZ_SAID130"
            | "FACT_TEMPLAR"
            | "FACT_UMISSION_SCUBA_DONE"
            | "FACT_SEXUS_PAID"
            | "FACT_TRYNNIE_MADRAS_WILL_JOIN"
            | "FACT_PEACE_ACHIEVED" => {
                Ok(self.values.get(&id).copied().unwrap_or(Self::initial(fact)))
            }
            name => Err(format!("unported fact predicate {name}")),
        }
    }
    fn can_set(id: FactId, catalog: &FactCatalog) -> bool {
        catalog
            .get(id)
            .is_some_and(|f| matches!(f.symbol.as_str(), "FACT_BURZ_SAID130" | "FACT_TEMPLAR"))
    }
    fn set(
        &mut self,
        id: FactId,
        value: u8,
        catalog: &FactCatalog,
        level: String,
    ) -> Result<(), String> {
        if !Self::can_set(id, catalog) {
            return Err(format!("unported fact assignment {id:?}"));
        }
        let initial = catalog.get(id).map_or(0, Self::initial);
        let previous = self.values.insert(id, value).unwrap_or(initial);
        if previous != value {
            self.journal.push(JournalEntry { fact: id, was_true: value != 0, level });
        }
        Ok(())
    }
}
/// A fact by its symbol, for rules that name one.
pub fn fact_named(story: &StoryFacts, catalog: &FactCatalog, symbol: &str) -> Result<u8, String> {
    let id = catalog
        .0
        .iter()
        .find(|fact| fact.symbol == symbol)
        .ok_or_else(|| format!("missing fact {symbol}"))?
        .id;
    story.get(id, catalog)
}
#[derive(Debug)]
enum Step {
    Quote {
        quote: u16,
        /// Which of the quote's lines is next.
        line: usize,
        paid: bool,
        plan: Option<QuotePlan>,
    },
    /// The party is asked for an answer.
    Wait(ConversationWait),
    Join,
    End,
}
impl Step {
    /// A quote from its first line, asking its price first if it has one.
    fn quote(quote: u16) -> Self {
        Step::Quote { quote, line: 0, paid: false, plan: None }
    }
    /// A quote from its first line, said without asking a price.
    fn said(quote: u16) -> Self {
        Step::Quote { quote, line: 0, paid: true, plan: None }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum ConversationWait {
    Ready,
    Line {
        speech: Entity,
    },
    /// The party answers with one of `words`, choosing it, or typing it
    /// when `typed`.
    Options {
        quote: u16,
        words: Vec<String>,
        typed: bool,
    },
    Price {
        quote: u16,
        gold: u32,
        always: bool,
    },
    /// The party is asked whether to give the NPC all its gold (retail
    /// string 0x7d6), answered as a price is.
    GiveAll {
        gold: u32,
    },
    Blocked {
        quote: u16,
        entry: Option<usize>,
        reason: String,
    },
}
/// Accepted dialogue history, including the party's input. Presentation chooses
/// the retail palette without guessing speakers from the text.
#[derive(Clone, Debug, PartialEq)]
pub enum TranscriptLine {
    Npc(String),
    Question(String),
    Reply(String),
    Reward(QuestReward),
    /// The NPC has nothing to say to the question: retail string 1867, or
    /// 1868 for the `second`, which every NPC takes in turn.
    NoAnswer {
        npc: String,
        second: bool,
    },
}
/// Committed outcomes, not commands. Presentation supplies retail wording.
#[derive(Clone, Debug, PartialEq)]
pub enum QuestReward {
    Gold {
        giver: String,
        amount: u32,
    },
    Experience {
        amount_each: u32,
    },
    /// The NPC handed the party this item.
    Item {
        name: String,
    },
    /// One of an item left this member's pack, or the party's items.
    Taken {
        member: Option<String>,
    },
}
#[derive(Component, Debug)]
pub struct Conversation {
    pub actor: Entity,
    /// Temporary effects belong to the affected speakers. Closing the
    /// conversation clears all of them; handing it over transfers none.
    charms: BTreeMap<Entity, u32>,
    pub current: Option<(u16, usize)>,
    pub wait: ConversationWait,
    pub transcript: Vec<TranscriptLine>,
    /// Keywords discovered during this conversation are green. Reopening
    /// imports the persistent notebook without the transient discovery marks.
    pub learned_topics: BTreeSet<String>,
    /// The NPC is not friendly yet: the party sees retail's topic menu
    /// (charm, small talk, bribe) rather than the dialogue's topics.
    pub menu: bool,
    /// What the party's last request came to; presentation owns its
    /// wording.
    result: Option<Outcome>,
    queue: VecDeque<Step>,
}
/// What the party's last request came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    NoTopic,
    Traded,
    TradeRefused(TradeError),
    GiftRefused(TradeError),
}
pub(crate) enum MagicalReply {
    Unnoticed,
    Charmed(u32),
    Quote(u16),
}
impl Conversation {
    pub fn ready(&self) -> bool {
        self.wait == ConversationWait::Ready && self.queue.is_empty()
    }

    pub(crate) fn charm(&self) -> u32 {
        self.charms.get(&self.actor).copied().unwrap_or(0)
    }

    pub(crate) fn magical_reply(&mut self, reply: MagicalReply) {
        self.result = None;
        match reply {
            MagicalReply::Unnoticed => {}
            MagicalReply::Charmed(strength) if strength > self.charm() => {
                self.charms.insert(self.actor, strength);
                self.transcript.push(TranscriptLine::Npc("Charmed.".into()));
            }
            MagicalReply::Charmed(_) => {}
            MagicalReply::Quote(quote) => self.queue.push_back(Step::quote(quote)),
        }
    }

    /// What the last request came to, until the party asks something else.
    pub fn outcome(&self) -> Option<Outcome> {
        self.result
    }
}
/// A line said: the words and the voice stem presentation may play. How
/// long it shows is presentation's to decide.
#[derive(Component, Debug)]
pub struct ConversationSpeech {
    /// The line's recording, if it has one to wait on.
    pub voice: Option<String>,
    pub text: String,
    /// An alert line: the alert chime and the text, without a recording.
    pub alert: bool,
}
/// A party member remarks on the conversation with this character event
/// (retail `QueueCharacterEvent` for `PickRandomPartySpeaker`); presentation
/// picks who and plays it.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartyRemark {
    pub event: u16,
}
/// An explicit edit of the persistent keyword notebook, not a spoken question.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotebookEdit {
    Remember,
    Forget,
}

/// Start talking to an NPC.
#[derive(Message, Clone, Copy, Debug)]
pub struct BeginConversation {
    pub actor: Entity,
}

/// What the party does in a conversation under way.
#[derive(Message, Clone, Debug)]
pub struct ConversationRequest {
    pub conversation: Entity,
    pub talk: Talk,
}

impl ConversationRequest {
    pub fn new(conversation: Entity, talk: Talk) -> Self {
        Self { conversation, talk }
    }

    /// Settle a quoted trade in the conversation it was quoted in.
    pub fn trade(quote: TradeQuote) -> Self {
        Self::new(quote.conversation, Talk::Trade(quote))
    }
}

#[derive(Clone, Debug)]
pub enum Talk {
    Topic {
        text: String,
    },
    Reply {
        text: String,
    },
    PriceAnswer {
        accept: bool,
    },
    Close,
    Recruit,
    Trade(TradeQuote),
    /// Hand the NPC an item from a member's pack or the party's items.
    Give {
        item: ItemRef,
    },
    /// Hand the NPC some of the party's gold.
    GiveGold {
        amount: u32,
    },
    /// Try to win over an NPC that is not yet friendly.
    Approach {
        approach: Approach,
    },
}
/// How the party tries to win an NPC over from retail's topic menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Approach {
    Charm,
    SmallTalk,
}
/// Script quotes with a fixed meaning (retail `W8NpcDialogue`): a neutral
/// NPC's greeting and its answers to charm, small talk and bribes that leave
/// it unmoved; a hostile one's dismissal; a charmed one's reply; the price
/// question and the answer to a party too poor; answers to joining; answers
/// to gifts; the answers for having nothing to say, not knowing and not
/// telling.
const NEUTRAL_GREETING: u16 = 2;
const SMALL_TALK_QUOTE: u16 = 3;
const CHARMED_QUOTE: u16 = 4;
const UNCHARMED_QUOTE: u16 = 6;
const UNBRIBED_QUOTE: u16 = 7;
const DECLINES_TO_JOIN: u16 = 0xb;
const NO_ROOM_TO_JOIN: u16 = 0xc;
const JOINS: u16 = 0xd;
const ACCEPTED_GIFT_QUOTE: u16 = 0x10;
const REFUSED_GIFT_QUOTE: u16 = 0x11;
const PRICE_QUOTE: u16 = 0x12;
const TOO_POOR_QUOTE: u16 = 0x14;
const HOSTILE_QUOTE: u16 = 0x18;
const UNKNOWN_QUOTES: [u16; 2] = [0x1c, 0x1d];
const CLASSIFIED_QUOTE: u16 = 0x1e;
const NO_ANSWER_QUOTES: std::ops::RangeInclusive<u16> = 0x67..=0x69;
/// How long the party waits to charm or make small talk with an NPC again
/// (retail `AdvanceNpcTimers`: 0xa8c0 seconds).
const APPROACH_COOLDOWN_MS: u64 = 0xa8c0 * 1000;
/// Retail `WillNpcTradeForItem`: one kind of NPC takes no bribes in kind;
/// the rest take one item always, and others worth 300 gold.
const NO_TRADE_KIND: i32 = 20;
const ALWAYS_TAKEN: ItemId = ItemId(0x29f);
const LEAST_BRIBE: u32 = 300;
/// RFS-81 answers to another name once the party has fixed it (retail
/// `GetNpcDisplayName`).
const RFS81: NpcId = NpcId(0x20);
const RFS81_FIXED_NAME: &str = "RFS81B";

/// Which of retail's two answers for having nothing to say comes next; retail
/// alternates them across every NPC (`g_empty_quote_text_index`).
#[derive(Resource, Debug, Default)]
pub struct NoAnswerTurn(bool);

/// Whether an approach made at `at` may be tried again at `now`.
pub fn approach_ready(at: Option<u64>, now: u64) -> bool {
    at.is_none_or(|at| now - at > APPROACH_COOLDOWN_MS)
}

/// How an NPC stands toward the party (retail `GetNpcDisposition`): its own
/// disposition `own` raised by the charm on it, fully friendly while its body
/// is turned. A factioned NPC answers with its faction's score when bound to
/// its monster or when the faction is hostile to the party.
fn standing(own: u8, charm: u32, turned: bool, def: &NpcDefinition, factions: &Factions) -> i32 {
    let own = if turned { 100 } else { (u32::from(own) + charm).min(100) as i32 };
    let faction = i32::from(def.faction);
    if def.faction != 0 && (def.monster_bound || factions.band(faction) == Band::Hostile) {
        return factions.score(faction).map_or(own, i32::from);
    }
    own
}

/// Retail `GetNpcDispositionBand`: hostile under 0x21, friendly from 0x42.
fn band_of(disposition: i32) -> Band {
    match disposition {
        d if d < 0x21 => Band::Hostile,
        d if d < 0x42 => Band::Neutral,
        _ => Band::Friendly,
    }
}

/// Retail `AdjustNpcDisposition`, clamped to 0..99.
fn shift_disposition(npc: &mut Npc, delta: i8) {
    npc.disposition = (i16::from(npc.disposition) + i16::from(delta)).clamp(0, 99) as u8;
}

/// Retail `GetItemStackValue`: ammunition by the bundle of 25, a stack of
/// things by each, anything else as one.
fn stack_value(item: &crate::content::items::ItemDefinition, count: u16) -> u32 {
    use crate::content::items::{AMMUNITION_BUNDLE, EquipClass, QuantityKind};
    let count = u32::from(count);
    if item.class == EquipClass::Ammunition {
        (item.value * count).div_ceil(AMMUNITION_BUNDLE)
    } else if item.quantity == QuantityKind::Stack && count > 1 {
        item.value * count
    } else {
        item.value
    }
}

/// The NPCs the party may talk to: their actors and definitions, their
/// bodies, their scripts, the catalogs and the level being played. Reads
/// only; what a conversation changes about an NPC, [`Dialogue`] holds.
#[derive(SystemParam)]
pub struct Speakers<'w, 's> {
    actors: Query<'w, 's, (Entity, &'static super::ActorId, &'static super::NpcBodies)>,
    bodies: Query<'w, 's, NpcBodyData>,
    groups: Query<'w, 's, &'static MonsterGroup>,
    levels: Query<'w, 's, (Entity, &'static Level)>,
    scripts: Res<'w, Assets<DialogueScript>>,
    assets: Res<'w, super::NpcAssets>,
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

impl Speakers<'_, '_> {
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

    /// The NPC's dialogue script, loaded before the level that placed it
    /// was entered.
    pub fn script(&self, actor: Entity) -> &DialogueScript {
        self.scripts.get(&self.assets.scripts[&self.id(actor)]).expect("a loaded NPC script")
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
    fn species_level(&self, actor: Entity) -> u32 {
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
    fn turned(&self, actor: Entity) -> bool {
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
    fn actor_shown_as(&self, name: &str, story: &StoryFacts) -> Option<Entity> {
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

/// The party as one entity: its roster, purse and pool, formation, and
/// where it stands once it has a body.
#[derive(QueryData)]
#[query_data(mutable)]
pub(super) struct PartyData {
    pub(super) entity: Entity,
    pub(super) roster: &'static PartyRoster,
    pub(super) inventory: &'static mut PartyInventory,
    pub(super) formation: &'static mut Formation,
    position: Option<&'static Position>,
}

/// The news a dialogue makes for others.
#[derive(SystemParam)]
struct News<'w> {
    remarks: MessageWriter<'w, PartyRemark>,
    speech: MessageWriter<'w, NpcSpeech>,
    factions: MessageWriter<'w, FactionNews>,
    items: MessageWriter<'w, crate::game::inventory::ItemNews>,
}

/// What dialogues read and write besides the conversation itself: the
/// speakers, the party and its members, the NPCs' retained opinions and
/// stock, the factions, the story's facts, and the news they make.
#[derive(SystemParam)]
pub struct Dialogue<'w, 's> {
    pub(super) speakers: Speakers<'w, 's>,
    pub(super) npcs: Query<'w, 's, &'static mut Npc>,
    stocks: Query<'w, 's, &'static mut MerchantStock>,
    pub(super) factions: ResMut<'w, Factions>,
    pub(super) party: Single<'w, 's, PartyData>,
    /// The party's members' characters and packs, which are never monster bodies.
    pub(super) characters: Query<
        'w,
        's,
        (&'static mut Character, &'static mut Vitals, Option<&'static mut Carried>),
        Without<Monster>,
    >,
    pub(super) ids: ResMut<'w, ItemIds>,
    pub(super) story: ResMut<'w, StoryFacts>,
    turn: ResMut<'w, NoAnswerTurn>,
    pub(super) rng: ResMut<'w, GameRng>,
    pub(super) commands: Commands<'w, 's>,
    news: News<'w>,
}

impl Dialogue<'_, '_> {
    /// The party's eye, while it has a body in the world.
    fn eye(&self) -> Option<Vec3> {
        self.party.position.map(PartyBody::eye)
    }

    /// The party's feet, while it has a body in the world.
    fn feet(&self) -> Option<Vec3> {
        self.party.position.map(PartyBody::feet)
    }

    /// The members in roster order.
    pub fn members(&self) -> impl Iterator<Item = (Entity, &Character, &Vitals)> {
        self.party
            .roster
            .iter()
            .filter_map(|member| self.characters.get(member).ok().map(|(c, v, _)| (member, c, v)))
    }

    /// The members awake and well enough to fight: the population retail's
    /// NPC rules work with (`hp_current != 0 && highest_condition < ASLEEP`,
    /// as in `CanNpcJoinParty`, the charm and dialogue practice).
    pub fn able(&self) -> impl Iterator<Item = (Entity, &Character)> {
        self.members().filter(|(_, _, v)| v.can_fight()).map(|(m, c, _)| (m, c))
    }

    /// The party's best Communication, which prices trades and bribes.
    fn communication(&self) -> u32 {
        trade::communication(self.members().map(|(_, c, v)| (c, v)))
    }

    /// The member with the best Communication among those who can act, the
    /// first on a tie, and that level (retail `GetBestPartySkillLevel`:
    /// `hp_current != 0 && highest_condition < TURNCOAT`).
    fn best_communicator(&self) -> Option<(Entity, u32)> {
        let mut best: Option<(Entity, u32)> = None;
        for (member, character, vitals) in self.members() {
            let level = character.skill(crate::content::characters::Skill::Communication);
            if vitals.can_act() && best.is_none_or(|(_, best)| level > best) {
                best = Some((member, level));
            }
        }
        best
    }

    /// Practise `members`' Communication `usage` times.
    fn practise(&mut self, members: &[Entity], usage: u32) {
        for &member in members {
            if let Ok((mut character, ..)) = self.characters.get_mut(member) {
                character.practise(
                    crate::content::characters::Skill::Communication,
                    usage,
                    &mut self.rng,
                );
            }
        }
    }

    /// The party's stack an offer or sale names, in the pool or a member's
    /// backpack.
    fn stack(&self, source: ItemRef) -> Result<&ItemStack, TradeError> {
        let carried = match source.owner {
            ItemOwner::Pool => None,
            ItemOwner::Member(member) => {
                self.characters.get(member).map_err(|_| TradeError::StaleSelection)?.2
            }
        };
        trade::sale_item(source, &self.party.inventory, carried)
    }

    /// Take `units` of a stack whose ownership this operation resolved.
    fn consume(&mut self, source: ItemRef, units: u16) {
        match source.owner {
            ItemOwner::Pool => self.party.inventory.pool.consume_units(source.id, units),
            ItemOwner::Member(member) => {
                if let Ok((_, _, Some(mut carried))) = self.characters.get_mut(member) {
                    carried.consume_backpack_units(source.id, units);
                }
            }
        }
    }

    /// Where the party holds one of `item`: a member's backpack first, then
    /// the pool.
    fn holding(&self, item: ItemId) -> Option<ItemRef> {
        let packed = self.party.roster.iter().find_map(|member| {
            let (_, _, pack) = self.characters.get(member).ok()?;
            let stack = pack?.backpack.iter().flatten().find(|s| s.item == item)?;
            Some(ItemRef { owner: ItemOwner::Member(member), id: stack.id })
        });
        packed.or_else(|| {
            let stack = self.party.inventory.pool.stacks.iter().find(|s| s.item == item)?;
            Some(ItemRef { owner: ItemOwner::Pool, id: stack.id })
        })
    }

    fn available(&self, actor: Entity) -> bool {
        self.speakers.available(actor, &self.factions)
    }

    /// How the NPC would stand toward the party in this conversation were
    /// its own disposition `own`. Charm is scoped to this speaker in the
    /// open conversation; it never overwrites retained opinion.
    fn band_at(&self, c: &Conversation, actor: Entity, own: u8) -> Band {
        let charm = c.charms.get(&actor).copied().unwrap_or(0);
        let def = self.speakers.definition(actor);
        band_of(standing(own, charm, self.speakers.turned(actor), def, &self.factions))
    }

    /// How the NPC stands toward the party in this conversation.
    fn band(&self, c: &Conversation, actor: Entity) -> Band {
        self.band_at(c, actor, self.npcs.get(actor).expect("an NPC actor").disposition)
    }

    /// Retail `SetNpcDispositionBand` to hostile, and `ResumeNpc` as a
    /// dialogue closes on the party: an NPC that is not already hostile
    /// becomes so. Retail's NPC then fights with its group; NPCs do not fight
    /// here yet, so it only turns the party away from now on.
    fn turn_hostile(&mut self, c: &Conversation, actor: Entity) {
        if self.band(c, actor) != Band::Hostile {
            self.npcs.get_mut(actor).unwrap().disposition = 0x19;
        }
    }

    /// What the NPC says to open its dialogue; meeting it for the first time
    /// teaches the party's able members Communication (retail
    /// `HandleNpcDialogueDeparture`).
    fn greet(&mut self, actor: Entity) -> u16 {
        let def = self.speakers.definition(actor);
        let first = def.merchant || self.npcs.get(actor).expect("an NPC actor").greeting_pending;
        let teaches = first && !def.monster_bound && !def.voice_script && !def.merchant;
        if teaches {
            let able: Vec<_> = self.able().map(|(m, _)| m).collect();
            self.practise(&able, 0xf);
        }
        if first { 0 } else { 1 }
    }

    /// Retail `DebitNpcTradePool`: a bribe, Communication-weighted, draws on
    /// what the NPC will take. Once that runs out an NPC not yet friendly
    /// warms to the party, and the pool fills again. The pool and own
    /// disposition the bribe leaves, without debiting it.
    fn debited(&self, c: &Conversation, actor: Entity, price: u32) -> (u16, u8) {
        let level = self.communication();
        let adjusted = price as u64 + price as u64 * level as u64 / 500;
        let refill = self.speakers.definition(actor).trade_pool;
        let cold = self.band(c, actor) != Band::Friendly;
        let npc = self.npcs.get(actor).unwrap();
        match (npc.trade_pool as u64 - adjusted.min(npc.trade_pool as u64)) as u16 {
            0 if cold => (refill, 0x4b),
            0 => (refill, npc.disposition),
            pool => (pool, npc.disposition),
        }
    }
}

/// Begin a conversation with the NPC `actor`, or refuse: the party must
/// hold nothing and be within reach of its body, which must not be
/// hostile. A merchant's stock is maintained as the party comes to talk.
/// The body turns to face the party and the conversation exists once the
/// commands apply.
fn begin(d: &mut Dialogue, actor: Entity) -> Result<Entity, TradeError> {
    if d.party.inventory.hand.held.is_some() {
        return Err(TradeError::Busy);
    }
    let body = d.speakers.active_body(actor).ok_or(TradeError::Unavailable)?;
    // Reach is judged from authoritative actor/party poses, not a UI claim.
    let at = d.speakers.position(body).ok_or(TradeError::Unavailable)?;
    let party_at = PartyBody::centre(d.eye().ok_or(TradeError::Unavailable)?);
    if party_at.distance(at) > crate::game::world::items::REACH || !d.available(actor) {
        return Err(TradeError::Unavailable);
    }
    let mut c = Conversation {
        actor,
        charms: default(),
        current: None,
        wait: ConversationWait::Ready,
        transcript: vec![],
        learned_topics: default(),
        menu: false,
        result: None,
        queue: default(),
    };
    // Retail `BeginNpcDialogueInternal` and `OpenNpcDialoguePanel`: an NPC
    // that speaks through its own script greets as it is; otherwise one
    // hostile to the party only turns it away, a neutral one offers the
    // topic menu, and a merchant always does business.
    let def = d.speakers.definition(actor);
    let as_scripted = def.monster_bound || def.voice_script || def.merchant;
    let band = if as_scripted { Band::Friendly } else { d.band(&c, actor) };
    if band == Band::Hostile {
        c.queue.push_back(Step::quote(HOSTILE_QUOTE));
        return Ok(d.commands.spawn((c, Closing)).id());
    }
    c.menu = band == Band::Neutral;
    let greeting = if c.menu { NEUTRAL_GREETING } else { d.greet(actor) };
    c.queue.push_back(Step::quote(greeting));
    maintain_stock(d, actor);
    // OpenNpcDialoguePanel / MonsterForwardReferencePosition: the accepted
    // speaker faces the party. Its physics rotation and its transform turn
    // together, so physics' transform synchronization cannot restore the
    // old facing.
    if let Some(facing) = facing(at, party_at) {
        d.commands.entity(body).entry::<Rotation>().and_modify(move |mut r| r.0 = facing);
        d.commands.entity(body).entry::<Transform>().and_modify(move |mut t| t.rotation = facing);
    }
    Ok(d.commands.spawn(c).id())
}

/// A merchant's stock is maintained as the party comes to talk.
fn maintain_stock(d: &mut Dialogue, actor: Entity) {
    let average = crate::game::party::average_level(d.members().map(|(_, c, _)| c.level()));
    let Ok(mut stock) = d.stocks.get_mut(actor) else { return };
    let content = &d.speakers.content;
    let def = &content.npcs[d.speakers.id(actor)];
    let now = d.speakers.now();
    let gated = stock.restock_at.is_some()
        && now - stock.maintenance_at >= crate::game::time::DAY_MS
        && def.stock.iter().any(|r| !r.persistent && r.item == trade::TEMPLAR_GATED);
    let templar = gated
        && fact_named(&d.story, &content.facts, "FACT_TEMPLAR").expect("the Templar fact") != 0;
    stock.maintain(def, &content.items, average, now, templar, &mut d.rng);
}

/// Travel, battle or the speaker's absence ends a conversation at once.
/// Retail's message queue would still play the answers already under way,
/// a paid answer or an accepted hand-in among them, so their effects commit
/// as it ends; only their lines are lost.
fn close(d: &mut Dialogue, entity: Entity, c: &mut Conversation) {
    for step in std::mem::take(&mut c.queue) {
        if let Step::Quote { plan: Some(plan), .. } = step {
            for effect in plan.effects {
                apply(d, c, effect);
            }
        }
    }
    d.commands.entity(entity).despawn();
}

/// The party leaves the dialogue: at once, or once its queued lines have
/// played out.
fn leave(commands: &mut Commands, entity: Entity, c: &Conversation) {
    let pending = matches!(c.wait, ConversationWait::Line { .. })
        || c.queue.iter().any(|step| matches!(step, Step::Quote { .. }));
    if pending {
        commands.entity(entity).insert(Closing);
    } else {
        commands.entity(entity).despawn();
    }
}

/// A quote's supported effects and selected control transfer. Planning reads
/// through preceding assignments, but never changes campaign state or draws RNG.
#[derive(Debug)]
struct QuotePlan {
    effects: Vec<QuoteEffect>,
    following: Option<Continuation>,
}
#[derive(Debug)]
enum QuoteEffect {
    SetFact(FactId, u8),
    Mention(TopicCategory, String),
    Gold(u32),
    Experience { members: Vec<Entity>, amount: u32 },
    Faction { faction: Faction, delta: i8 },
    GiveItem(ItemId),
    TakeItem(ItemId),
    Disposition { npc: Entity, delta: i8 },
    PartyRemark(u16),
    // The speaker turns on the party as the dialogue closes.
    Resume,
}
#[derive(Debug)]
enum Continuation {
    Quote(Response),
    Options { words: Vec<String>, typed: bool },
    End,
}

fn plan(d: &Dialogue, q: &Quote) -> Result<QuotePlan, (usize, String)> {
    let catalog = &d.speakers.content.facts;
    let mut writes = BTreeMap::new();
    let mut plan = QuotePlan { effects: Vec::new(), following: None };
    for (i, op) in q.operations.iter().enumerate() {
        let effect = match op {
            DialogueOp::Unsupported { opcode, .. } => {
                return Err((i, format!("unported mandatory opcode {opcode}")));
            }
            DialogueOp::SetFact(id, value) => {
                if !StoryFacts::can_set(*id, catalog) {
                    return Err((i, format!("unported fact assignment {id:?}")));
                }
                writes.insert(*id, *value);
                QuoteEffect::SetFact(*id, *value)
            }
            DialogueOp::GiveGold(amount) => QuoteEffect::Gold(*amount),
            // CanPartySlotParticipate includes the unconscious. Quest XP is
            // the authored amount for EACH eligible member.
            DialogueOp::GiveExperience(amount) => QuoteEffect::Experience {
                members: d.members().filter(|(_, _, v)| !v.is_dead()).map(|m| m.0).collect(),
                amount: *amount,
            },
            DialogueOp::AdjustFaction { faction: Some(faction), delta } => {
                QuoteEffect::Faction { faction: *faction, delta: *delta }
            }
            DialogueOp::GiveItem(item) => QuoteEffect::GiveItem(*item),
            DialogueOp::TakeItem(item) => QuoteEffect::TakeItem(*item),
            DialogueOp::Conditional { facts, response } => {
                let mut matched = true;
                for (id, expected) in facts {
                    let actual = match writes.get(id) {
                        Some(value) => *value,
                        None => d.story.get(*id, catalog).map_err(|e| (i, e))?,
                    };
                    if actual != *expected {
                        matched = false;
                        break;
                    }
                }
                if !matched {
                    continue;
                }
                plan.following = Some(Continuation::Quote(response.clone()));
                break;
            }
            DialogueOp::Next(response) => {
                plan.following = Some(Continuation::Quote(response.clone()));
                break;
            }
            DialogueOp::Options(words) => {
                plan.following = Some(Continuation::Options { words: words.clone(), typed: false });
                break;
            }
            DialogueOp::KeywordInput(words) => {
                plan.following = Some(Continuation::Options { words: words.clone(), typed: true });
                break;
            }
            // Retail closes without reading the quote's later entries.
            DialogueOp::CloseAndResume => {
                plan.effects.push(QuoteEffect::Resume);
                plan.following = Some(Continuation::End);
                break;
            }
            DialogueOp::Close => {
                plan.following = Some(Continuation::End);
                continue;
            }
            // An NPC no one answers to is left alone, as in retail.
            DialogueOp::AdjustDisposition { npc, delta } => {
                match d.speakers.actor_shown_as(npc, &d.story) {
                    Some(npc) => QuoteEffect::Disposition { npc, delta: *delta },
                    None => continue,
                }
            }
            DialogueOp::PartyRemark(event) => QuoteEffect::PartyRemark(*event),
            // Retail `W8Monster::SetScriptLabel` moves the NPC's monster
            // script, whose interpreter is not ported.
            DialogueOp::MonsterScriptLabel(Some(label)) => {
                return Err((i, format!("unported monster script label {label}")));
            }
            DialogueOp::Mention { category, text } => QuoteEffect::Mention(*category, text.clone()),
            // Retail ignores a faction name it does not know. Retail
            // `DismissNpcFromParty` acts only on an NPC in the party, and
            // none can join it yet (#117); for any other NPC the action only
            // locks and releases the world cursor.
            _ => continue,
        };
        plan.effects.push(effect);
    }
    Ok(plan)
}

/// Commit a played quote: its effects, then what follows it.
fn commit(d: &mut Dialogue, c: &mut Conversation, quote: u16, plan: QuotePlan) {
    for effect in plan.effects {
        apply(d, c, effect);
    }
    let next = match plan.following {
        None => return,
        // Another NPC takes over the conversation and says `quote` of its
        // own script (retail `ProcessNpcQuoteEntry` for a response naming a
        // speaker). Retail finds the NPC by display name among those it
        // knows; when none answers to it, the conversation simply waits.
        Some(Continuation::Quote(Response::Speaker { npc, quote })) => {
            if let Some(speaker) = d.speakers.actor_shown_as(&npc, &d.story) {
                c.actor = speaker;
                c.queue.push_front(Step::quote(quote));
            }
            return;
        }
        Some(Continuation::Quote(Response::Fixed(quote))) => Step::quote(quote),
        Some(Continuation::Quote(Response::Random { first, last })) => {
            Step::quote(first + d.rng.below(u32::from(last - first) + 1) as u16)
        }
        Some(Continuation::Options { words, typed }) => {
            Step::Wait(ConversationWait::Options { quote, words, typed })
        }
        Some(Continuation::End) => Step::End,
    };
    c.queue.push_front(next);
}

fn apply(d: &mut Dialogue, c: &mut Conversation, effect: QuoteEffect) {
    let reward = match effect {
        QuoteEffect::SetFact(id, value) => {
            let level = d.speakers.level().map_or_else(String::new, |(_, l)| l.path.clone());
            d.story
                .set(id, value, &d.speakers.content.facts, level)
                .expect("planned fact assignment");
            return;
        }
        QuoteEffect::Mention(category, text) => return learn(c, &mut d.story, category, &text),
        QuoteEffect::Gold(amount) => {
            d.party.inventory.gold += amount;
            let giver = d.speakers.definition(c.actor).source_name.clone();
            QuestReward::Gold { giver, amount }
        }
        QuoteEffect::Experience { members, amount } => {
            for member in members {
                if let Ok((mut character, ..)) = d.characters.get_mut(member) {
                    character.award_experience(amount);
                }
            }
            QuestReward::Experience { amount_each: amount }
        }
        QuoteEffect::GiveItem(item) => {
            let catalog = &d.speakers.content.items;
            let definition = &catalog[item];
            let stack = make_item(definition, &mut d.ids, &mut d.rng, true, true);
            let (name, crucial) = (definition.name.clone(), definition.crucial);
            if let Some(rest) = d.party.inventory.pool.receive(stack, catalog) {
                drop_gift(d, rest, crucial);
            }
            QuestReward::Item { name }
        }
        QuoteEffect::TakeItem(item) => {
            let Some(source) = d.holding(item) else { return };
            d.consume(source, 1);
            let member = match source.owner {
                ItemOwner::Member(member) => {
                    d.characters.get(member).ok().map(|(c, ..)| c.name.clone())
                }
                ItemOwner::Pool => None,
            };
            QuestReward::Taken { member }
        }
        QuoteEffect::Disposition { npc, delta } => {
            if let Ok(mut npc) = d.npcs.get_mut(npc) {
                shift_disposition(&mut npc, delta);
            }
            return;
        }
        QuoteEffect::Resume => return d.turn_hostile(c, c.actor),
        QuoteEffect::PartyRemark(event) => {
            d.news.remarks.write(PartyRemark { event });
            return;
        }
        // Retail announces it in the text box, not the dialogue.
        QuoteEffect::Faction { faction, delta } => {
            let now = d.speakers.now();
            if let Some(news) = d.factions.adjust(faction, delta, now) {
                d.news.factions.write(news);
            }
            return;
        }
    };
    c.transcript.push(TranscriptLine::Reward(reward));
}

/// Retail `AddItemToPartyOrDrop`: a gift the party's items have no room
/// for lands at the party's feet, unless the party may not part with it;
/// that one goes to the hand with retail's notice
/// (`DropUnstoredCharacterItem`). Retail then puts back whatever the hand
/// held, so it keeps only the first such gift.
fn drop_gift(d: &mut Dialogue, gift: ItemStack, crucial: bool) {
    use crate::game::world::items;
    match (d.speakers.level(), d.feet()) {
        (Some((level, _)), Some(feet)) if !crucial => {
            items::lay(&mut d.commands, level, feet, items::WorldItem::loose(gift, false));
        }
        _ => {
            let hand = &mut d.party.inventory.hand.held;
            if hand.is_none() {
                *hand = Some(gift);
            }
            d.news.items.write(crate::game::inventory::ItemNews::CannotDrop);
        }
    }
}

/// The party learns a keyword in this conversation: it goes in the
/// notebook, and this conversation marks it as new.
fn learn(c: &mut Conversation, story: &mut StoryFacts, category: TopicCategory, text: &str) {
    if story.remember_topic(category, text) {
        c.learned_topics.insert(text.trim().to_ascii_lowercase());
    }
}

/// What a line of a quote stands for (retail `SpeakNpcSubquote`): an alert
/// line is shown with the alert chime and no recording; otherwise a few
/// words stand for something other than a line.
enum Line {
    Alert,
    Nothing,
    /// The answer to a question it has nothing to say to.
    NoAnswer,
    Unknown,
    Classified,
    /// Its recording plays without showing a line.
    Sound,
    Spoken,
}

fn line_kind(text: &str, alert: bool) -> Line {
    let is = |word: &str| text.eq_ignore_ascii_case(word);
    if alert {
        Line::Alert
    } else if text.is_empty() {
        Line::Nothing
    } else if is("EMPTY") || is("BLANK") {
        Line::NoAnswer
    } else if is("UNKNOWN") {
        Line::Unknown
    } else if is("CLASSIFIED") {
        Line::Classified
    } else if is("SOUND") {
        Line::Sound
    } else {
        Line::Spoken
    }
}

/// Run the conversation's script as far as it goes: until it waits for a
/// line to be spoken or the party to answer, or ends.
fn run(d: &mut Dialogue, entity: Entity, c: &mut Conversation, closing: bool) {
    let actor = c.actor;
    // Bound immediate authored transitions, including malicious empty-quote loops.
    for _ in 0..256 {
        // A new speaker's script is fetched on the next run.
        if c.wait != ConversationWait::Ready || c.actor != actor {
            return;
        }
        let Some(step) = c.queue.pop_front() else { return };
        let (quote, line, paid, planned) = match step {
            Step::End => {
                c.queue.push_front(Step::End);
                return;
            }
            // The NPC joins the party once its acceptance is said.
            Step::Join => {
                if let Err(refusal) = super::recruit::join(d, actor) {
                    warn!("NPC did not join after all: {refusal:?}");
                }
                continue;
            }
            Step::Wait(wait) => {
                c.wait = wait;
                return;
            }
            Step::Quote { quote, line, paid, plan } => (quote, line, paid, plan),
        };
        let q = &d.speakers.script(actor).quotes[usize::from(quote)];
        let price = q.operations.iter().find_map(|op| match op {
            DialogueOp::Price { gold, always } => Some((*gold, *always)),
            _ => None,
        });
        let (text, alert, last) =
            (q.text.get(line).cloned().unwrap_or_default(), q.alert, line + 1 >= q.text.len());
        // A priced quote asks its price before its first line, unless the
        // party has paid for it before and it is not always paid.
        if let Some((gold, always)) = price
            && !paid
            && line == 0
            && (always || !d.npcs.get(actor).unwrap().paid_topics.contains(&quote))
        {
            let gold = gold - gold * c.charm().min(100) / 100;
            let gold = (gold - gold * d.communication().min(500) / 500).max(1);
            c.queue.push_front(Step::Wait(ConversationWait::Price { quote, gold, always }));
            c.queue.push_front(Step::said(PRICE_QUOTE));
            continue;
        }
        let planned = match planned.map_or_else(|| plan(d, q), Ok) {
            Ok(plan) => plan,
            Err((entry, reason)) => {
                c.queue.push_front(Step::Quote { quote, line, paid, plan: None });
                c.wait = ConversationWait::Blocked { quote, entry: Some(entry), reason };
                return;
            }
        };
        // The first greeting teaches the party the NPC's name.
        if quote == 0 && std::mem::take(&mut d.npcs.get_mut(actor).unwrap().greeting_pending) {
            let name = d.speakers.definition(actor).source_name.clone();
            learn(c, &mut d.story, TopicCategory::Person, &name);
        }
        c.current = Some((quote, line));
        if last {
            commit(d, c, quote, planned);
        } else {
            c.queue.push_front(Step::Quote {
                quote,
                line: line + 1,
                paid: true,
                plan: Some(planned),
            });
        }
        let def = d.speakers.definition(actor);
        let (speaker, stem) = (def.source_name.clone(), def.voice_stem());
        let voice = match line_kind(&text, alert) {
            Line::Alert => None,
            Line::Nothing => continue,
            Line::NoAnswer => {
                if NO_ANSWER_QUOTES.contains(&quote) {
                    let second = d.turn.0;
                    d.turn.0 = !second;
                    c.transcript.push(TranscriptLine::NoAnswer { npc: speaker, second });
                }
                continue;
            }
            Line::Unknown => {
                c.queue.push_back(Step::quote(UNKNOWN_QUOTES[usize::from(d.rng.below(100) >= 50)]));
                continue;
            }
            Line::Classified => {
                c.queue.push_back(Step::quote(CLASSIFIED_QUOTE));
                continue;
            }
            Line::Sound | Line::Spoken => {
                let suffix = match line {
                    0 => String::new(),
                    n => char::from(b'a' + (n - 1) as u8).to_string(),
                };
                Some(format!("{stem}_{quote:03}{suffix}"))
            }
        };
        let text =
            if matches!(line_kind(&text, alert), Line::Sound) { String::new() } else { text };
        let speech = d
            .commands
            .spawn((ConversationSpeech { voice, text: text.clone(), alert }, ChildOf(entity)))
            .id();
        if closing && !text.is_empty() {
            d.news.speech.write(NpcSpeech { speaker, text: text.clone() });
        }
        // The line being spoken is always the transcript's last; one with no
        // text is not shown.
        c.transcript.push(TranscriptLine::Npc(text));
        c.wait = ConversationWait::Line { speech };
        return;
    }
    let quote = c.current.map_or(0, |v| v.0);
    c.wait = ConversationWait::Blocked {
        quote,
        entry: None,
        reason: "script made no bounded progress".into(),
    };
}

/// The party asks about `text`: the script's quote for it, by the whole,
/// by a pair of its words, or by one.
fn topic(c: &mut Conversation, script: &DialogueScript, story: &mut StoryFacts, text: &str) {
    if !c.ready() || c.menu {
        return;
    }
    let cleaned: String = text
        .chars()
        .filter(|ch| ch.is_alphanumeric() || ch.is_whitespace() || *ch == '\'')
        .collect();
    let words: Vec<_> = cleaned.split_whitespace().collect();
    let matched = script
        .topic(&cleaned)
        .or_else(|| words.windows(2).find_map(|pair| script.topic(&pair.join(" "))))
        .or_else(|| words.iter().find_map(|word| script.topic(word)));
    let Some(quote) = matched else {
        c.result = Some(Outcome::NoTopic);
        return;
    };
    // The accepted question belongs in the notebook even when its answer
    // first requires payment or another continuation.
    story.remember_topic(TopicCategory::Misc, &cleaned);
    c.transcript.push(TranscriptLine::Question(text.to_owned()));
    c.queue.push_back(Step::quote(quote));
}

/// The party answers a question with `text`: a reply naming the word takes
/// it; otherwise the reply for a known word, or for an unknown one.
fn reply(c: &mut Conversation, script: &DialogueScript, text: &str) {
    let ConversationWait::Options { quote, words, .. } = &c.wait else { return };
    let matched = words.iter().any(|w| w.eq_ignore_ascii_case(text));
    let Some(entries) = script.quotes.get(usize::from(*quote)).map(|q| &q.operations) else {
        return;
    };
    let role = if matched { 1 } else { 2 };
    let response = entries
        .iter()
        .find_map(|op| match op {
            DialogueOp::Reply { response, role: 3, words }
                if matched && words.iter().any(|w| w.eq_ignore_ascii_case(text)) =>
            {
                Some(*response)
            }
            _ => None,
        })
        .or_else(|| {
            entries.iter().find_map(|op| match op {
                DialogueOp::Reply { response, role: r, .. } if *r == role => Some(*response),
                _ => None,
            })
        });
    if let Some(quote) = response {
        c.transcript.push(TranscriptLine::Reply(text.to_owned()));
        c.wait = ConversationWait::Ready;
        c.queue.push_back(Step::quote(quote));
    }
}

/// The party asks the NPC to join. It answers from its script, and joining,
/// leaves the conversation.
fn ask_to_join(d: &Dialogue, c: &mut Conversation) {
    if !c.ready() {
        return;
    }
    let answer = super::recruit::joinable(d, c.actor);
    c.queue.push_back(Step::quote(match answer {
        Ok(()) => JOINS,
        Err(super::recruit::JoinRefusal::NoRoom) => NO_ROOM_TO_JOIN,
        Err(super::recruit::JoinRefusal::Declines) => DECLINES_TO_JOIN,
    }));
    if answer.is_ok() {
        c.queue.extend([Step::Join, Step::End]);
    }
}

/// The party charms the NPC or makes small talk with it from the topic
/// menu (retail `SelectNpcDialogueService`, `SelectNpcDialogueTalk` and
/// `ApplyNpcInteraction` 1 and 0). Its disposition moves by its record's
/// scale, more with the party's best Communication, which practises; a
/// charm that warms it moves it further the higher the party's level is
/// over its own. The band it ends in answers: won over, it turns to its
/// dialogue; turned hostile, it sends the party away.
fn approach(d: &mut Dialogue, c: &mut Conversation, how: Approach) {
    let actor = c.actor;
    let now = d.speakers.now();
    let def = d.speakers.definition(actor);
    let npc = d.npcs.get(actor).unwrap();
    let (at, scale, usage) = match how {
        Approach::Charm => (npc.charmed_at, def.charm_scale, 5),
        Approach::SmallTalk => (npc.small_talk_at, def.talk_scale, 8),
    };
    if !c.ready() || !c.menu || !approach_ready(at, now) {
        return;
    }
    let Some((speaker, level)) = d.best_communicator() else { return };
    let scale = i32::from(scale);
    let mut delta = scale + level as i32 * (scale.abs() / 5) / 100;
    if how == Approach::Charm && delta > 0 {
        let average = crate::game::party::average_level(d.able().map(|(_, c)| c.level()));
        delta = (average * delta as u32 / d.speakers.species_level(actor)) as i32;
    }
    let mut npc = d.npcs.get_mut(actor).unwrap();
    // Retail narrows the change to a signed byte.
    shift_disposition(&mut npc, delta as i8);
    match how {
        Approach::Charm => npc.charmed_at = Some(now),
        Approach::SmallTalk => npc.small_talk_at = Some(now),
    }
    d.practise(&[speaker], usage);
    let answer = match (d.band(c, actor), how) {
        (Band::Friendly, Approach::Charm) => CHARMED_QUOTE,
        (Band::Friendly, Approach::SmallTalk) => d.greet(actor),
        (Band::Neutral, Approach::Charm) => UNCHARMED_QUOTE,
        (Band::Neutral, Approach::SmallTalk) => SMALL_TALK_QUOTE,
        (Band::Hostile, _) => {
            d.turn_hostile(c, actor);
            HOSTILE_QUOTE
        }
    };
    c.queue.push_back(Step::quote(answer));
    c.menu = d.band(c, actor) != Band::Friendly;
    if c.menu && answer == HOSTILE_QUOTE {
        c.queue.push_back(Step::End);
    }
}

/// A bribe that wins the NPC over from the topic menu turns it to its
/// dialogue; it greets the party unless it took all the party's gold
/// (retail `HandleNpcDialogueItemChoice`, `OnNpcTradeDialogClosed`). A
/// charm spell wins it over the same way as its script next runs.
fn won_over(d: &mut Dialogue, c: &mut Conversation, greeting: bool) {
    if !c.menu || d.band(c, c.actor) != Band::Friendly {
        return;
    }
    c.menu = false;
    if greeting {
        let greeting = d.greet(c.actor);
        c.queue.push_back(Step::quote(greeting));
    }
}

/// The party offers the NPC an item from a member's pack or the party's
/// items (retail `HandleNpcDialogueItem`). The quote answering it runs;
/// the NPC keeps what it asked for, and any other item it is given unless
/// the party may not part with it. A voice-scripted NPC ignores an item
/// it has nothing to say about. The item is taken only when its answer's
/// effects are valid; a content gap keeps it.
fn give(d: &mut Dialogue, c: &mut Conversation, item: ItemRef) -> Result<(), TradeError> {
    if !c.ready() {
        return Err(TradeError::Busy);
    }
    let actor = c.actor;
    let stack = d.stack(item)?;
    let (offered, count) = (stack.item, stack.count());
    let definition = &d.speakers.content.items[offered];
    let (crucial, value) = (definition.crucial, stack_value(definition, count));
    let def = d.speakers.definition(actor);
    let (voiced, kind) = (def.voice_script, def.kind);
    let answer = d.speakers.script(actor).item_quote(offered);
    // Retail `HandleNpcDialogueItem`: an NPC not yet friendly takes an item
    // as a bribe, valued by the whole stack offered, and keeps it whatever
    // it says. A bribe is debited only with the item it buys.
    let bribe = !voiced && d.band(c, actor) != Band::Friendly;
    let mut debit = None;
    let (quote, keeps) = match answer {
        _ if bribe && crucial => (REFUSED_GIFT_QUOTE, false),
        _ if bribe
            && (kind == NO_TRADE_KIND || (offered != ALWAYS_TAKEN && value < LEAST_BRIBE)) =>
        {
            (UNBRIBED_QUOTE, false)
        }
        Some((quote, _)) if bribe => (quote, true),
        None if bribe => {
            let (pool, disposition) = d.debited(c, actor, value);
            debit = Some((pool, disposition));
            let warmed = d.band_at(c, actor, disposition) == Band::Friendly;
            (if warmed { ACCEPTED_GIFT_QUOTE } else { UNBRIBED_QUOTE }, true)
        }
        Some(answer) => answer,
        None if voiced => return Ok(()),
        None if crucial => (REFUSED_GIFT_QUOTE, false),
        None => (ACCEPTED_GIFT_QUOTE, true),
    };
    let planned =
        d.speakers.script(actor).quotes.get(usize::from(quote)).and_then(|q| plan(d, q).ok());
    // Retail takes the item at once; the answer's effects follow at its last
    // line, which plays out even if the party leaves the dialogue. A content
    // gap keeps the item and queues the answer to show it.
    if keeps && planned.is_some() {
        d.consume(item, 1);
        if let Some((pool, disposition)) = debit {
            let mut npc = d.npcs.get_mut(actor).unwrap();
            (npc.trade_pool, npc.disposition) = (pool, disposition);
        }
    }
    c.queue.push_back(Step::Quote { quote, line: 0, paid: false, plan: planned });
    won_over(d, c, true);
    Ok(())
}

/// The party gives the NPC gold (retail `HandleNpcDialogueItem` with
/// trade gold): all of it only once the party confirms, as retail asks.
fn give_gold(d: &mut Dialogue, c: &mut Conversation, amount: u32) -> Result<(), TradeError> {
    if !c.ready() {
        return Err(TradeError::Busy);
    }
    let gold = d.party.inventory.gold;
    if amount == 0 {
        return Err(TradeError::InvalidQuantity);
    }
    if amount > gold {
        return Err(TradeError::Funds);
    }
    if amount == gold {
        c.wait = ConversationWait::GiveAll { gold };
        return Ok(());
    }
    spend_gift(d, c, amount, false)
}

/// The NPC takes the gold (retail `ConfirmNpcTradePurchase`): a friendly
/// one thanks the party for it; one not yet friendly takes it as a bribe,
/// unless it takes no bribes, and says whether that won it over.
fn spend_gift(
    d: &mut Dialogue,
    c: &mut Conversation,
    amount: u32,
    all: bool,
) -> Result<(), TradeError> {
    let actor = c.actor;
    let bribe = d.band(c, actor) != Band::Friendly;
    if bribe && d.speakers.definition(actor).trade_pool == 0 {
        c.queue.push_back(Step::said(UNBRIBED_QUOTE));
        return Ok(());
    }
    if amount > d.party.inventory.gold {
        return Err(TradeError::Funds);
    }
    d.party.inventory.gold -= amount;
    if bribe {
        let (pool, disposition) = d.debited(c, actor, amount);
        let mut npc = d.npcs.get_mut(actor).unwrap();
        (npc.trade_pool, npc.disposition) = (pool, disposition);
    }
    let thanks = d.band(c, actor) == Band::Friendly;
    c.queue.push_back(Step::said(if thanks { ACCEPTED_GIFT_QUOTE } else { UNBRIBED_QUOTE }));
    won_over(d, c, !all);
    Ok(())
}

/// The party answers the price of a topic, or the question of giving all
/// its gold.
fn answer_price(d: &mut Dialogue, c: &mut Conversation, accept: bool) {
    match c.wait.clone() {
        ConversationWait::GiveAll { gold } => {
            c.wait = ConversationWait::Ready;
            if accept && let Err(refused) = spend_gift(d, c, gold, true) {
                c.result = Some(Outcome::GiftRefused(refused));
            }
        }
        ConversationWait::Price { quote, gold, always } => {
            let actor = c.actor;
            let Some(line) = d.speakers.script(actor).quotes.get(usize::from(quote)) else {
                return;
            };
            if !accept {
                for op in &line.operations {
                    if let DialogueOp::Decline(quote) = op {
                        c.queue.push_back(Step::quote(*quote));
                    }
                }
                c.wait = ConversationWait::Ready;
                return;
            }
            let step = if d.party.inventory.gold < gold {
                Step::said(TOO_POOR_QUOTE)
            } else {
                let planned = match plan(d, line) {
                    Ok(planned) => planned,
                    Err((entry, reason)) => {
                        c.wait = ConversationWait::Blocked { quote, entry: Some(entry), reason };
                        return;
                    }
                };
                d.party.inventory.gold -= gold;
                if !always {
                    d.npcs.get_mut(actor).unwrap().paid_topics.push(quote);
                }
                Step::Quote { quote, line: 0, paid: true, plan: Some(planned) }
            };
            c.queue.push_back(step);
            c.wait = ConversationWait::Ready;
        }
        _ => {}
    }
}

/// Settle a quoted trade: the merchant's next trade, at the quoted price
/// still, the goods and the gold moved at once; or nothing.
fn trade(d: &mut Dialogue, c: &Conversation, quote: TradeQuote) -> Result<(), TradeError> {
    if !c.ready() {
        return Err(TradeError::StaleSelection);
    }
    if !d.available(c.actor) {
        return Err(TradeError::Unavailable);
    }
    let stock = d.stocks.get(c.actor).map_err(|_| TradeError::Unavailable)?;
    if stock.trades != quote.trade {
        return Err(TradeError::StaleSelection);
    }
    let communication = d.communication();
    let content = &d.speakers.content;
    let def = &content.npcs[d.speakers.id(c.actor)];
    let catalog = &content.items;
    match quote.source {
        TradeSource::Stock(lot) => trade::buy(
            &mut d.stocks.get_mut(c.actor).unwrap(),
            &mut d.party.inventory,
            &mut d.ids,
            def,
            catalog,
            lot,
            quote.quantity,
            quote.gold,
            communication,
        )?,
        TradeSource::Party(source) => {
            let item = d.stack(source)?;
            if trade::validate_sale(def, item, quote.quantity, catalog, communication)?
                != quote.gold
            {
                return Err(TradeError::PriceChanged);
            }
            let (item, units) = (item.item, quote.quantity as u16);
            d.stocks.get_mut(c.actor).unwrap().add(item, quote.quantity, catalog, false);
            d.consume(source, units);
            d.party.inventory.gold += quote.gold;
        }
    }
    d.stocks.get_mut(c.actor).unwrap().trades += 1;
    Ok(())
}

/// The party has left the dialogue while its lines were still queued.
/// Retail keeps its message queue when the dialogue closes, so they play out
/// in the world and commit their effects; the conversation ends with them,
/// or at the first question the party is no longer there to answer.
#[derive(Component, Debug)]
pub struct Closing;
/// What the speaker of a dialogue the party has left says, for the text box
/// (retail `SpeakNpcSubquote` outside dialogue mode: "Name: text").
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct NpcSpeech {
    pub speaker: String,
    pub text: String,
}
/// The systems that carry conversations: lines spoken, conversations
/// begun, the party's requests answered, then scripts run.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct Conversing;

/// The NPC has finished saying a line.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineSpoken {
    pub conversation: Entity,
    pub speech: Entity,
}

/// The party remembers or forgets a topic in its notebook.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct EditNotebook {
    pub edit: NotebookEdit,
    pub text: String,
}

/// A spoken line gives way to what follows it.
pub fn finish_lines(
    mut spoken: MessageReader<LineSpoken>,
    mut conversations: Query<&mut Conversation>,
    mut commands: Commands,
) {
    for &LineSpoken { conversation, speech } in spoken.read() {
        let Ok(mut c) = conversations.get_mut(conversation) else { continue };
        if matches!(c.wait, ConversationWait::Line { speech: s, .. } if s == speech) {
            commands.entity(speech).despawn();
            c.wait = ConversationWait::Ready;
        }
    }
}

pub fn edit_notebook(mut edits: MessageReader<EditNotebook>, mut facts: ResMut<StoryFacts>) {
    for EditNotebook { edit, text } in edits.read() {
        match edit {
            NotebookEdit::Remember => facts.remember_topic(TopicCategory::Misc, text),
            NotebookEdit::Forget => facts.forget_topic(text),
        };
    }
}

/// What ends every conversation at once: an accepted level entry, or a
/// fight. No queued request may alter the departing campaign.
#[derive(SystemParam)]
pub struct Interruptions<'w, 's> {
    entering: Query<'w, 's, (), With<Destination>>,
    fighting: Query<'w, 's, (), With<Battle>>,
}

/// Talking to an NPC begins a conversation while the party is free.
pub fn begin_conversations(
    mut begins: MessageReader<BeginConversation>,
    interruptions: Interruptions,
    freedom: Freedom,
    mut dialogue: Dialogue,
) {
    if !interruptions.entering.is_empty() {
        begins.clear();
        return;
    }
    for &BeginConversation { actor } in begins.read() {
        let begun = if freedom.free() && interruptions.fighting.is_empty() {
            begin(&mut dialogue, actor)
        } else {
            Err(TradeError::Busy)
        };
        if let Err(e) = begun {
            warn!("conversation refused: {e:?}");
        }
    }
}

/// Answer the party's requests in a conversation. A dialogue the party has
/// left takes none; anything but leaving ends a dialogue whose NPC, a
/// fight or a camp no longer allows it.
pub fn answer_requests(
    mut requests: MessageReader<ConversationRequest>,
    interruptions: Interruptions,
    camp: Option<Res<crate::game::camp::Camp>>,
    mut conversations: Query<(&mut Conversation, Has<Closing>)>,
    mut d: Dialogue,
) {
    if !interruptions.entering.is_empty() {
        requests.clear();
        return;
    }
    // A conversation closed by an earlier request this frame is gone once
    // the commands apply; later requests to it are already stale.
    let mut closed = EntityHashSet::default();
    for ConversationRequest { conversation: entity, talk } in requests.read().cloned() {
        let Ok((mut c, closing)) = conversations.get_mut(entity) else { continue };
        if closing || closed.contains(&entity) {
            continue;
        }
        if !matches!(talk, Talk::Close) {
            let allowed =
                interruptions.fighting.is_empty() && camp.is_none() && d.available(c.actor);
            if !allowed {
                close(&mut d, entity, &mut c);
                closed.insert(entity);
                continue;
            }
            c.result = None;
        }
        let script = d.speakers.script(c.actor);
        let outcome = match talk {
            Talk::Close => {
                leave(&mut d.commands, entity, &c);
                closed.insert(entity);
                None
            }
            Talk::Topic { text } => {
                topic(&mut c, script, &mut d.story, &text);
                None
            }
            Talk::Reply { text } => {
                reply(&mut c, script, &text);
                None
            }
            Talk::PriceAnswer { accept } => {
                answer_price(&mut d, &mut c, accept);
                None
            }
            Talk::Recruit => {
                ask_to_join(&d, &mut c);
                None
            }
            Talk::Give { item } => give(&mut d, &mut c, item).err().map(Outcome::GiftRefused),
            Talk::GiveGold { amount } => {
                give_gold(&mut d, &mut c, amount).err().map(Outcome::GiftRefused)
            }
            Talk::Approach { approach: how } => {
                approach(&mut d, &mut c, how);
                None
            }
            Talk::Trade(quote) => Some(match trade(&mut d, &c, quote) {
                Ok(()) => Outcome::Traded,
                Err(e) => Outcome::TradeRefused(e),
            }),
        };
        if let Some(outcome) = outcome {
            c.result = Some(outcome);
        }
    }
}

/// Run each conversation's script as far as it goes, and close those that
/// are done: ended, left with nothing more to say, or no longer possible.
/// A conversation whose NPC a spell has just won over leaves the menu here.
pub fn run_conversations(
    interruptions: Interruptions,
    mut conversations: Query<(Entity, &mut Conversation, Has<Closing>)>,
    mut d: Dialogue,
) {
    let interrupted = !interruptions.entering.is_empty() || !interruptions.fighting.is_empty();
    for (entity, mut c, closing) in &mut conversations {
        if interrupted || !d.available(c.actor) {
            close(&mut d, entity, &mut c);
            continue;
        }
        won_over(&mut d, &mut c, true);
        run(&mut d, entity, &mut c, closing);
        let left = closing
            && match c.wait {
                ConversationWait::Ready => c.queue.is_empty(),
                ConversationWait::Line { .. } => false,
                _ => true,
            };
        let ended = c.wait == ConversationWait::Ready && matches!(c.queue.front(), Some(Step::End));
        if left || ended {
            close(&mut d, entity, &mut c);
        }
    }
}
