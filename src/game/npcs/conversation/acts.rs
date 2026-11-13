//! What the party does in a conversation (`docs/rules/conversation.md`
//! sections 4 and 5): each an event at the conversation, carried out by one
//! observer that checks it is possible now, changes what the rule says, and
//! queues what the person answers.

use avian3d::prelude::{Position, Rotation};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::say::{Continue, fall_silent};
use super::{
    Approach, Asked, Closing, Conversation, Dialogue, DialogueItem, DialogueReadOnly, Outcome,
    Parting, TradeQuote, TranscriptLine,
};
use crate::content::characters::Skill;
use crate::content::items::{AMMUNITION_BUNDLE, EquipClass, ItemId, QuantityKind};
use crate::content::npcs::{DialogueOp, NpcDefinition, TopicCategory};
use crate::game::characters::Practise;
use crate::game::combat::Battle;
use crate::game::factions::{Band, Factions};
use crate::game::freedom::Freedom;
use crate::game::inventory::PartyInventory;
use crate::game::items::{Items, Lose};
use crate::game::npcs::Npc;
use crate::game::npcs::access::{NpcActors, Scripts};
use crate::game::npcs::recruit::{JoinRefusal, Recruiter};
use crate::game::npcs::story::StoryFacts;
use crate::game::npcs::trade::{self, TradeError, TradeSource};
use crate::game::party::body::PartyBody;
use crate::game::party::roster::PartyRoster;
use crate::game::time::GameClock;
use crate::game::world::facing;

/// The game time that must pass before someone can be charmed, or chatted
/// with, again.
const APPROACH_AGAIN_MS: u64 = 43_200_000;

/// The party walks up to someone and talks, or they to it. One who has
/// only a voice, or is bound to a monster, begins with `quote` if given.
/// `with_held`: what the party's hand holds is offered as the talk begins,
/// in place of the greeting (retail `BeginNpcDialogue` with an item).
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Begin {
    #[event_target]
    pub actor: Entity,
    pub quote: Option<u16>,
    pub with_held: bool,
}

/// Someone says one of their lines as they stand, heard and not answered.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Overheard {
    #[event_target]
    pub actor: Entity,
    pub quote: u16,
}

/// The party asks about a topic, or where something is.
#[derive(EntityEvent, Clone, Debug)]
pub struct Ask {
    #[event_target]
    pub conversation: Entity,
    pub text: String,
    pub where_is: bool,
}

/// What the party answers a question with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Word(String),
    /// Yes or no, to a price or to giving everything.
    Accept(bool),
}

/// The party answers the question that stands.
#[derive(EntityEvent, Clone, Debug)]
pub struct Answer {
    #[event_target]
    pub conversation: Entity,
    pub reply: Reply,
}

/// The party offers an item.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Offer {
    #[event_target]
    pub conversation: Entity,
    pub item: Entity,
}

/// The party offers gold; `sure`, even if it is all it has.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct OfferGold {
    #[event_target]
    pub conversation: Entity,
    pub amount: u32,
    pub sure: bool,
}

/// The party tries to win the person over.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Sway {
    #[event_target]
    pub conversation: Entity,
    pub approach: Approach,
}

/// The party asks the person to join it.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Invite {
    #[event_target]
    pub conversation: Entity,
}

/// The party leaves.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Leave {
    #[event_target]
    pub conversation: Entity,
}

/// The party means to turn on the person.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Attack {
    #[event_target]
    pub conversation: Entity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotebookEdit {
    Remember,
    Forget,
}

/// The player notes a topic down, or strikes it out.
#[derive(Event, Clone, Debug, PartialEq, Eq)]
pub struct EditNotebook {
    pub edit: NotebookEdit,
    pub text: String,
}

/// Those of the party who speak for it.
#[derive(SystemParam)]
pub(super) struct Voices<'w, 's> {
    roster: Single<'w, 's, &'static PartyRoster>,
    members: Query<
        'w,
        's,
        (
            &'static crate::game::actors::Level,
            &'static crate::game::actors::Skills,
            Has<crate::game::actors::CanAct>,
            Has<crate::game::actors::CanFight>,
        ),
    >,
}

impl Voices<'_, '_> {
    /// The party's best Communication.
    pub fn skill(&self) -> u32 {
        trade::communication(
            self.roster
                .iter()
                .filter_map(|m| self.members.get(m).ok())
                .filter(|(_, _, can_act, _)| *can_act)
                .map(|(_, skills, ..)| skills),
        )
    }

    /// The best speaker able to act.
    fn best(&self) -> Option<Entity> {
        self.roster
            .iter()
            .filter_map(|m| {
                self.members
                    .get(m)
                    .ok()
                    .filter(|(_, _, can_act, _)| *can_act)
                    .map(|(_, skills, ..)| (m, skills[Skill::Communication]))
            })
            .max_by_key(|(_, skill)| *skill)
            .map(|(m, _)| m)
    }

    /// Those fit to fight.
    fn fit(&self) -> impl Iterator<Item = (Entity, u8)> + '_ {
        self.roster.iter().filter_map(|m| {
            self.members
                .get(m)
                .ok()
                .filter(|(.., can_fight)| *can_fight)
                .map(|(level, ..)| (m, level.0 as u8))
        })
    }

    /// Their average level.
    fn level(&self) -> u32 {
        crate::game::party::average_level(self.fit().map(|(_, level)| level))
    }
}

/// Whether someone last charmed or chatted with at `at` can be again.
pub fn approach_ready(at: Option<u64>, now: u64) -> bool {
    at.is_none_or(|at| now - at > APPROACH_AGAIN_MS)
}

/// How a person stands toward the party (retail `GetNpcDisposition`,
/// `GetNpcDispositionBand`): their own regard and the charm on them, 100 if
/// turned; the faction's standing for one bound to a monster or of a
/// hostile faction; hostile up to 32, neutral up to 65.
pub(super) fn band(
    own: u8,
    charm: u32,
    turned: bool,
    d: &NpcDefinition,
    factions: &Factions,
) -> Band {
    let score = if turned { 100 } else { (u32::from(own) + charm).min(100) as i32 };
    let score = if d.faction != 0
        && (d.monster_bound || factions.band(i32::from(d.faction)) == Band::Hostile)
    {
        factions.score(i32::from(d.faction)).map_or(score, i32::from)
    } else {
        score
    };
    match score {
        ..=32 => Band::Hostile,
        33..=65 => Band::Neutral,
        _ => Band::Friendly,
    }
}

/// The quote a friend greets the party with: the first greeting, or a later
/// one.
pub(super) fn greeting(npc: &Npc, d: &NpcDefinition) -> u16 {
    if npc.greeting_pending || d.merchant { 0 } else { 1 }
}

/// A gift counts against what the person still wants (retail
/// `DebitNpcTradePool`): its worth, and a fifth of it more for each hundred
/// of the party's best Communication. Whether that used the pool up.
fn debit(npc: &mut Npc, worth: u32, skill: u32) -> bool {
    let counted = (worth + skill * ((worth & 0xffff) / 5) / 100) & 0xffff;
    let pool = u32::from(npc.trade_pool);
    npc.trade_pool = (pool.max(counted) - counted) as u16;
    npc.trade_pool == 0
}

/// A person at one moment of a conversation: how they stand, and what a
/// gift does to that.
pub(super) struct Regard<'a> {
    d: &'a NpcDefinition,
    charm: u32,
    turned: bool,
    factions: &'a Factions,
    skill: u32,
}

impl<'a> Regard<'a> {
    /// How the one spoken with regards the party now.
    pub(super) fn of(
        talk: &DialogueItem,
        actors: &'a NpcActors,
        voices: &Voices,
        factions: &'a Factions,
    ) -> Option<Self> {
        let actor = talk.conversation.actor;
        Some(Self {
            d: actors.definition(actor)?,
            charm: talk.conversation.charm(),
            turned: actors.turned(actor),
            factions,
            skill: voices.skill(),
        })
    }

    pub(super) fn band(&self, npc: &Npc) -> Band {
        band(npc.disposition, self.charm, self.turned, self.d, self.factions)
    }

    /// A gift of some worth: when it uses up what they still want, one who
    /// is not a friend becomes one, and wants as much again.
    fn bribe(&self, npc: &mut Npc, worth: u32) {
        if debit(npc, worth, self.skill) {
            if self.band(npc) != Band::Friendly {
                npc.disposition = 75;
            }
            npc.trade_pool = self.d.trade_pool;
        }
    }
}

/// Begin talking (retail `BeginNpcDialogueInternal`, `OpenNpcDialoguePanel`):
/// with a free hand, not fighting, within reach of one who is there and has
/// something to say. The person turns to the party. A hostile one says so
/// and that is all; a neutral one greets coldly and keeps the party to its
/// menu; a friend greets, and a first greeting is practice.
pub(super) fn begin(
    begin: On<Begin>,
    freedom: Freedom,
    (conversations, battles): (Query<(), With<Conversation>>, Query<(), With<Battle>>),
    (actors, scripts, voices): (NpcActors, Scripts, Voices),
    npcs: Query<&Npc>,
    factions: Res<Factions>,
    party: Single<(Entity, &Position)>,
    held: Items,
    mut commands: Commands,
) {
    let actor = begin.actor;
    if !conversations.is_empty()
        || !freedom.free()
        || !battles.is_empty()
        || (held.held(party.0).is_some() && !begin.with_held)
        || !actors.available(actor)
        || scripts.get(actor).is_none()
    {
        return;
    }
    let centre = PartyBody::centre(PartyBody::eye(party.1));
    if let Some(body) = actors.active_body(actor) {
        let Some(at) = actors.position(body) else { return };
        if at.distance(centre) > crate::game::world::items::REACH {
            return;
        }
        if let Some(rotation) = facing(at, centre) {
            commands.entity(body).insert(Rotation(rotation));
        }
    }
    let (Some(d), Ok(npc)) = (actors.definition(actor), npcs.get(actor)) else { return };
    let plain = !d.monster_bound && !d.voice_script && !d.merchant;
    let standing = if plain {
        band(npc.disposition, 0, actors.turned(actor), d, &factions)
    } else {
        Band::Friendly
    };
    let quote = match (standing, begin.quote) {
        (Band::Hostile, _) => 0x18,
        (Band::Neutral, _) => 2,
        (Band::Friendly, Some(quote)) if d.voice_script || d.monster_bound => quote,
        (Band::Friendly, _) => greeting(npc, d),
    };
    let offering = begin.with_held.then(|| held.held(party.0)).flatten();
    let offers = offering.filter(|_| standing != Band::Hostile);
    let mut script = super::Script::default();
    if offers.is_none() {
        script.say(quote);
    }
    let mut talk = commands.spawn((Conversation::new(actor, standing == Band::Neutral), script));
    if standing == Band::Hostile {
        talk.insert(Closing);
    }
    let conversation = talk.id();
    if quote == 0 && plain {
        for (member, _) in voices.fit() {
            commands.trigger(Practise { member, skill: Skill::Communication, usage: 15 });
        }
    }
    if standing != Band::Hostile {
        commands.trigger(trade::Visit { merchant: actor });
    }
    commands.trigger(Continue { conversation });
    if let Some(item) = offers {
        commands.trigger(Offer { conversation, item });
    }
}

/// A line said as someone stands (retail `SpeakNpcSubquote` outside a
/// dialogue): a conversation the party is not in, over when the line is.
pub(super) fn overhear(
    heard: On<Overheard>,
    conversations: Query<(), With<Conversation>>,
    (actors, scripts): (NpcActors, Scripts),
    battles: Query<(), With<Battle>>,
    mut commands: Commands,
) {
    if !conversations.is_empty()
        || !battles.is_empty()
        || !actors.available(heard.actor)
        || scripts.get(heard.actor).is_none()
    {
        return;
    }
    let mut script = super::Script::default();
    script.say_free(heard.quote);
    let conversation =
        commands.spawn((Conversation::new(heard.actor, false), script, Closing)).id();
    commands.trigger(Continue { conversation });
}

/// Answer the question that stands (retail `HandleNpcDialogueReply`,
/// `FindNpcReplyQuote`). A word that is one of those listed takes the
/// quote's reply for that word, or its general reply; any other its reply
/// for a wrong answer; with none to take the question stands. A price
/// refused says the quote's decline quotes; accepted without the gold,
/// 0x14; accepted, the gold is paid and the quote said. Giving everything,
/// confirmed, is offered for sure.
pub(super) fn answer(
    answer: On<Answer>,
    mut talks: Query<Dialogue>,
    (scripts, actors): (Scripts, NpcActors),
    mut npcs: Query<&mut Npc>,
    factions: Res<Factions>,
    mut inventory: Single<&mut PartyInventory>,
    mut commands: Commands,
) {
    let conversation = answer.conversation;
    let Ok(mut talk) = talks.get_mut(conversation) else { return };
    let actor = talk.conversation.actor;
    if talk.closing {
        return;
    }
    match (talk.asked.cloned(), &answer.reply) {
        (Some(Asked::Options { quote, words, .. }), Reply::Word(text)) => {
            let Some(q) = scripts.get(actor).and_then(|s| s.quotes.get(usize::from(quote))) else {
                return;
            };
            let listed = words.iter().any(|word| word.eq_ignore_ascii_case(text));
            let replies = || {
                q.operations.iter().filter_map(|op| match op {
                    DialogueOp::Reply { response, role, words } => Some((*response, *role, words)),
                    _ => None,
                })
            };
            let own = replies()
                .find(|(_, role, words)| {
                    *role == 3 && listed && words.iter().any(|w| w.eq_ignore_ascii_case(text))
                })
                .or_else(|| replies().find(|(_, role, _)| *role == if listed { 1 } else { 2 }));
            let Some((next, ..)) = own else { return };
            talk.conversation.transcript.push(TranscriptLine::Reply(text.clone()));
            talk.script.say(next);
        }
        (Some(Asked::Price { quote, gold, always }), &Reply::Accept(accept)) => {
            let Some(q) = scripts.get(actor).and_then(|s| s.quotes.get(usize::from(quote))) else {
                return;
            };
            if !accept {
                for op in &q.operations {
                    if let DialogueOp::Decline(declined) = op {
                        talk.script.say(*declined);
                    }
                }
            } else if inventory.gold < gold {
                talk.script.say_free(0x14);
            } else if let Ok(mut npc) = npcs.get_mut(actor) {
                inventory.gold -= gold;
                if !always {
                    npc.paid_topics.push(quote);
                }
                talk.script.say_free(quote);
            }
        }
        // The person is cool toward the party at best, says so, and goes
        // back to what they were doing against it (retail
        // `OnNpcDispositionPromptClosed`).
        (Some(Asked::Attack), &Reply::Accept(accept)) => {
            if accept {
                if let (Some(d), Ok(mut npc)) = (actors.definition(actor), npcs.get_mut(actor)) {
                    let charm = talk.conversation.charm();
                    let turned = actors.turned(actor);
                    if band(npc.disposition, charm, turned, d, &factions) != Band::Hostile {
                        npc.disposition = 25;
                    }
                }
                talk.script.say(0x18);
                talk.script.resume();
            }
        }
        (Some(Asked::GiveAll { gold }), &Reply::Accept(accept)) => {
            if accept {
                commands.entity(conversation).remove::<Asked>();
                commands.trigger(OfferGold { conversation, amount: gold, sure: true });
                return;
            }
        }
        _ => return,
    }
    commands.entity(conversation).remove::<Asked>();
    commands.trigger(Continue { conversation });
}

/// Offer gold: all the party has asks for confirmation first. A friend
/// takes it with thanks; one who is not, and can be bought, counts it
/// against what they still want, and thanks the party if that made them a
/// friend; one who cannot be bought takes none.
pub(super) fn offer_gold(
    offer: On<OfferGold>,
    mut talks: Query<Dialogue>,
    (actors, voices): (NpcActors, Voices),
    mut npcs: Query<&mut Npc>,
    factions: Res<Factions>,
    mut inventory: Single<&mut PartyInventory>,
    mut commands: Commands,
) {
    let OfferGold { conversation, amount, sure } = *offer;
    let Ok(mut talk) = talks.get_mut(conversation) else { return };
    if !talk.idle() {
        return;
    }
    if amount == 0 || amount > inventory.gold {
        let why = if amount == 0 { TradeError::InvalidQuantity } else { TradeError::Funds };
        talk.conversation.transcript.push(TranscriptLine::Told(Outcome::GiftRefused(why)));
        return;
    }
    if amount == inventory.gold && !sure {
        commands.entity(conversation).insert(Asked::GiveAll { gold: amount });
        return;
    }
    let (Some(regard), Ok(mut npc)) =
        (Regard::of(&talk, &actors, &voices, &factions), npcs.get_mut(talk.conversation.actor))
    else {
        return;
    };
    let cold = regard.band(&npc) != Band::Friendly;
    if cold && regard.d.trade_pool == 0 {
        talk.script.say_free(7);
    } else {
        inventory.gold -= amount;
        if cold {
            regard.bribe(&mut npc, amount);
        }
        talk.script.say_free(if regard.band(&npc) == Band::Friendly { 0x10 } else { 7 });
    }
    commands.trigger(Continue { conversation });
}

/// Offer an item (retail `HandleNpcDialogueItem`). A friend answers with
/// the quote their script has for it, keeping it if the script says so;
/// with none, one that must not be lost is refused and any other taken with
/// thanks. One who is not a friend refuses what must not be lost and what
/// they have no use for; anything else they keep, answering with their
/// script's quote for it, or counting its worth against what they still
/// want and thanking the party if that made them a friend.
pub(super) fn offer(
    offer: On<Offer>,
    mut talks: Query<Dialogue>,
    (actors, scripts, voices): (NpcActors, Scripts, Voices),
    mut npcs: Query<&mut Npc>,
    factions: Res<Factions>,
    held: Items,
    mut commands: Commands,
) {
    let Offer { conversation, item: source } = *offer;
    let Ok(mut talk) = talks.get_mut(conversation) else { return };
    let actor = talk.conversation.actor;
    if !talk.idle() {
        return;
    }
    // What may be given: what could be sold, or what the party's hand holds,
    // which is how the original gives anything.
    let in_hand = matches!(held.place(source), Some(crate::game::items::Place::Hand { .. }));
    let offerable = in_hand || trade::for_sale(source, &voices.roster, &held).is_ok();
    let offered = offerable.then(|| held.id(source)).flatten();
    let Some(id) = offered else {
        let stale = Outcome::GiftRefused(TradeError::StaleSelection);
        talk.conversation.transcript.push(TranscriptLine::Told(stale));
        return;
    };
    let (Some(regard), Some(script), Ok(mut npc)) =
        (Regard::of(&talk, &actors, &voices, &factions), scripts.get(actor), npcs.get_mut(actor))
    else {
        return;
    };
    let d = regard.d;
    let item = &actors.content.items[id];
    let count = held.units(source);
    let worth = match (item.class, item.quantity) {
        (EquipClass::Ammunition, _) => (item.value * count).div_ceil(AMMUNITION_BUNDLE),
        (_, QuantityKind::Stack) => item.value * count.max(1),
        _ => item.value,
    };
    let cold = !d.voice_script && regard.band(&npc) != Band::Friendly;
    let authored = script.item_quote(id);
    // What they answer with, whether they keep it, and whether it counts
    // against what they still want.
    let (quote, keeps, bribe) = if cold && item.crucial {
        (Some(0x11), false, false)
    } else if cold && (d.kind == 20 || (id != ItemId(0x29f) && worth < 300)) {
        (Some(7), false, false)
    } else if let Some((quote, keeps)) = authored {
        (Some(quote), keeps || cold, false)
    } else if cold {
        (None, true, true)
    } else if d.voice_script {
        return;
    } else if item.crucial {
        (Some(0x11), false, false)
    } else {
        (Some(0x10), true, false)
    };
    // Nothing is taken for an answer that cannot be given.
    for quote in quote.map_or(vec![0x10, 7], |quote| vec![quote]) {
        if script.quotes.get(usize::from(quote)).is_none() {
            fall_silent(&mut talk, &mut commands, &format!("no answer {quote} to a gift"));
            return;
        }
    }
    if keeps {
        commands.trigger(Lose { item: source, amount: 1 });
    }
    if bribe {
        regard.bribe(&mut npc, worth);
    }
    let thanks = if regard.band(&npc) == Band::Friendly { 0x10 } else { 7 };
    talk.script.say(quote.unwrap_or(thanks));
    commands.trigger(Continue { conversation });
}

/// Charm or small talk (retail `ApplyNpcInteraction`), each once in twelve
/// hours for each person: their regard changes by their own figure for it
/// and a fifth of it more for each hundred of the party's best
/// Communication, a charm that improves scaled by the party's average level
/// over theirs; the best speaker practises; and they answer by how they now
/// stand.
pub(super) fn sway(
    sway: On<Sway>,
    mut talks: Query<Dialogue>,
    (actors, voices): (NpcActors, Voices),
    mut npcs: Query<&mut Npc>,
    factions: Res<Factions>,
    clock: Res<GameClock>,
    mut commands: Commands,
) {
    let Sway { conversation, approach } = *sway;
    let Ok(mut talk) = talks.get_mut(conversation) else { return };
    let actor = talk.conversation.actor;
    if !talk.idle() || !talk.conversation.menu {
        return;
    }
    let (Some(regard), Ok(mut npc)) =
        (Regard::of(&talk, &actors, &voices, &factions), npcs.get_mut(actor))
    else {
        return;
    };
    let d = regard.d;
    let (last, scale, usage) = match approach {
        Approach::Charm => (npc.charmed_at, d.charm_scale, 5),
        Approach::SmallTalk => (npc.small_talk_at, d.talk_scale, 8),
    };
    if !approach_ready(last, clock.ms) {
        return;
    }
    let Some(member) = voices.best() else { return };
    let scale = i32::from(scale);
    let mut delta = scale + voices.skill() as i32 * (scale.abs() / 5) / 100;
    if approach == Approach::Charm && delta > 0 {
        delta = (voices.level() * delta as u32 / actors.species_level(actor)) as i32;
    }
    npc.disposition = (i32::from(npc.disposition) + delta).clamp(0, 99) as u8;
    match approach {
        Approach::Charm => npc.charmed_at = Some(clock.ms),
        Approach::SmallTalk => npc.small_talk_at = Some(clock.ms),
    }
    let standing = regard.band(&npc);
    talk.conversation.menu = standing != Band::Friendly;
    talk.script.say(match (standing, approach) {
        (Band::Hostile, _) => 0x18,
        (Band::Neutral, Approach::Charm) => 6,
        (Band::Neutral, Approach::SmallTalk) => 3,
        (Band::Friendly, Approach::Charm) => 4,
        (Band::Friendly, Approach::SmallTalk) => greeting(&npc, d),
    });
    if standing == Band::Hostile {
        talk.script.end();
    }
    commands.trigger(Practise { member, skill: Skill::Communication, usage });
    commands.trigger(Continue { conversation });
}

/// Ask the person to join (retail `RequestNpcJoinParty`): no room, 0xc;
/// will not, 0xb; joins, 0xd, and that ends the conversation.
pub(super) fn invite(
    invite: On<Invite>,
    mut talks: Query<Dialogue>,
    mut recruiter: Recruiter,
    mut commands: Commands,
) {
    let Ok(mut talk) = talks.get_mut(invite.conversation) else { return };
    if !talk.idle() {
        return;
    }
    match recruiter.admit(talk.conversation.actor) {
        Ok(()) => {
            talk.script.say(0xd);
            talk.script.end();
        }
        Err(JoinRefusal::NoRoom) => talk.script.say(0xc),
        Err(JoinRefusal::Declines) => talk.script.say(0xb),
    }
    commands.trigger(Continue { conversation: invite.conversation });
}

/// Buy or sell at a quoted price: the merchant's rules settle it, and the
/// party is told what came of it.
pub(super) fn trade(
    trade: On<TradeQuote>,
    mut talks: Query<Dialogue>,
    mut market: ParamSet<(trade::Pricing, trade::Settlement)>,
) {
    let Ok(mut talk) = talks.get_mut(trade.conversation) else { return };
    if !talk.idle() {
        return;
    }
    let outcome = if trade.offer.merchant != talk.conversation.actor {
        Outcome::TradeRefused(TradeError::StaleSelection)
    } else {
        match trade::settle(&mut market, trade.offer) {
            Ok(()) => Outcome::Traded,
            Err(error) => Outcome::TradeRefused(error),
        }
    };
    talk.conversation.transcript.push(TranscriptLine::Told(outcome));
}

/// The quote a friend says as the party takes its leave.
const FAREWELL: u16 = 0x5c;

/// Leave (retail `LeaveNpcDialogueLayout`): a friend with nothing asked of
/// the party says farewell and then it is over; with anyone else what is
/// being said is finished and nothing more is begun, a question left
/// standing unanswered. The charm on the person ends.
pub(super) fn leave(
    leave: On<Leave>,
    mut talks: Query<Dialogue>,
    scripts: Scripts,
    mut commands: Commands,
) {
    let conversation = leave.conversation;
    let Ok(mut talk) = talks.get_mut(conversation) else { return };
    if talk.closing || talk.parting {
        return;
    }
    talk.conversation.charms.clear();
    let says_farewell = scripts
        .get(talk.conversation.actor)
        .is_some_and(|script| script.quotes.len() > usize::from(FAREWELL));
    let friend = !talk.conversation.menu && talk.asked.is_none();
    if friend && says_farewell {
        talk.script.say_free(FAREWELL);
        talk.script.end();
        commands.entity(conversation).insert(Parting);
    } else {
        commands.entity(conversation).insert(Closing);
    }
    commands.trigger(Continue { conversation });
}

/// Turn on the person: the party is asked whether it means it.
pub(super) fn attack(attack: On<Attack>, talks: Query<DialogueReadOnly>, mut commands: Commands) {
    if talks.get(attack.conversation).is_ok_and(|talk| talk.idle()) {
        commands.entity(attack.conversation).insert(Asked::Attack);
    }
}

/// The player notes a topic down, or strikes it out.
pub(super) fn edit_notebook(edit: On<EditNotebook>, mut story: ResMut<StoryFacts>) {
    match edit.edit {
        NotebookEdit::Remember => story.remember_topic(TopicCategory::Misc, &edit.text),
        NotebookEdit::Forget => story.forget_topic(&edit.text),
    };
}

/// The price of something at the merchant the party is talking with, if the
/// conversation is idle.
pub fn quote_trade(
    In((conversation, source, quantity)): In<(Entity, TradeSource, u32)>,
    talks: Query<DialogueReadOnly>,
    market: trade::Pricing,
) -> Result<TradeQuote, TradeError> {
    let talk = talks.get(conversation).map_err(|_| TradeError::StaleSelection)?;
    if !talk.idle() {
        return Err(TradeError::Busy);
    }
    let offer = market.quote(talk.conversation.actor, source, quantity)?;
    Ok(TradeQuote { conversation, offer })
}
