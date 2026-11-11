//! Asking about a topic (`docs/rules/conversation.md` section 5): how what
//! the party types is matched against what the person has to say, and what
//! they answer when asked where someone or somewhere is.

use std::sync::LazyLock;

use bevy::prelude::*;
use serde::Deserialize;

use super::acts::{Ask, Invite};
use super::say::Continue;
use super::{Dialogue, TranscriptLine};
use crate::content::levels::LevelInfo;
use crate::content::npcs::{DialogueScript, NpcDefinition, TopicCategory};
use crate::game::npcs::access::{NpcActors, Scripts};
use crate::game::npcs::story::StoryFacts;
use crate::game::rng::GameRng;

/// The quote of one who does not know where something is.
const NOWHERE: u16 = 0x76;
/// The quote of one asked where they are themselves.
const HERE_I_AM: u16 = 0x75;

/// What can be asked after: a row of `assets/tables/where_is.ron`.
#[derive(Debug, Deserialize)]
struct WhereIs {
    joins: Vec<String>,
    asks_where: Vec<String>,
    talks_about: String,
    /// A name, the bit of a speaker's record that says they know of it,
    /// and the quote they answer with.
    people: Vec<(String, u32, u16)>,
    /// A region's number and the names it goes by.
    regions: Vec<(usize, Vec<String>)>,
    /// The quote for each region asked after, from each region the party
    /// may be in.
    directions: Vec<Vec<Option<u16>>>,
}

static WHERE_IS: LazyLock<WhereIs> = LazyLock::new(|| {
    ron::from_str(include_str!("../../../../assets/tables/where_is.ron"))
        .expect("assets/tables/where_is.ron is a table of what can be asked after")
});

fn begins(text: &str, with: &str) -> bool {
    text.len() >= with.len() && text[..with.len()].eq_ignore_ascii_case(with)
}

/// What the speaker answers when asked where a name is (retail
/// `FindNpcNameOrPlaceQuote`): themselves; someone they know of; a region,
/// by where the party is now. None for a name they have nothing on.
fn whereabouts(name: &str, speaker: &NpcDefinition, region: usize) -> Option<u16> {
    let is = |other: &String| other.eq_ignore_ascii_case(name);
    if is(&speaker.source_name) {
        return Some(HERE_I_AM);
    }
    if let Some((_, knows, quote)) = WHERE_IS.people.iter().find(|(person, ..)| is(person)) {
        return (speaker.knows & knows != 0).then_some(*quote);
    }
    let (asked, _) = WHERE_IS.regions.iter().find(|(_, names)| names.iter().any(is))?;
    *WHERE_IS.directions.get(asked - 1)?.get(region)?
}

/// The quotes that answer a topic (retail `HandleNpcDialogueInput`): the
/// whole text, then each pair of adjacent words, then each word, matched
/// against the person's topics. The first match; with two different single
/// words, both; with more than two, 0x20; with none, nothing.
fn matched(script: &DialogueScript, text: &str) -> (Option<u16>, Option<u16>) {
    let words: Vec<_> = text.split_whitespace().collect();
    let singles: Vec<u16> = words.iter().filter_map(|word| script.topic(word)).collect();
    match script
        .topic(text)
        .or_else(|| words.windows(2).find_map(|pair| script.topic(&pair.join(" "))))
    {
        Some(quote) => (Some(quote), None),
        None if singles.len() > 2 => (Some(0x20), None),
        None => (singles.first().copied(), singles.get(1).copied().filter(|q| *q != singles[0])),
    }
}

/// Ask about a topic. `join` asks the person to join. Asked where
/// something is (by the button, or by beginning with `where is`), the
/// person answers from their own topics if they can, else with what they
/// know of the name, else that they do not know. Any other topic is matched
/// by word; two different single words are both answered, with 0x21
/// between; none is answered with 0x23 or 0x24. Quote 0x59 is said alone.
pub(super) fn ask(
    ask: On<Ask>,
    mut talks: Query<Dialogue>,
    (scripts, actors): (Scripts, NpcActors),
    mut story: ResMut<StoryFacts>,
    mut rng: ResMut<GameRng>,
    mut commands: Commands,
) {
    let conversation = ask.conversation;
    let Ok(mut talk) = talks.get_mut(conversation) else { return };
    if !talk.idle() || talk.conversation.menu {
        return;
    }
    let actor = talk.conversation.actor;
    let (Some(script), Some(speaker)) = (scripts.get(actor), actors.definition(actor)) else {
        return;
    };
    let cleaned: String = ask
        .text
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '\'')
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return;
    }
    if WHERE_IS.joins.iter().any(|word| begins(cleaned, word)) {
        commands.trigger(Invite { conversation });
        return;
    }
    let region = actors
        .level()
        .and_then(|(_, level)| LevelInfo::for_path(&level.path))
        .map_or(0, |level| usize::from(level.band));
    let wheres = WHERE_IS.asks_where.iter().any(|way| begins(cleaned, way));
    let own = match (ask.where_is, wheres) {
        (true, false) => script.topic(&format!("Where is {cleaned}")),
        _ => script.topic(cleaned),
    };
    let (first, second) = match own {
        Some(quote) => (Some(quote), None),
        None if begins(cleaned, &WHERE_IS.talks_about) => {
            matched(script, &cleaned[WHERE_IS.talks_about.len()..])
        }
        // The original drops nine letters whichever way it was asked, so
        // only `where is` leaves the name whole.
        None if wheres => {
            let name: String = cleaned.chars().skip(9).collect();
            (Some(whereabouts(&name, speaker, region).unwrap_or(NOWHERE)), None)
        }
        None if ask.where_is => {
            (Some(whereabouts(cleaned, speaker, region).unwrap_or(NOWHERE)), None)
        }
        None => matched(script, cleaned),
    };
    talk.conversation.transcript.push(TranscriptLine::Question(ask.text.clone()));
    match (first, second) {
        (None, _) => talk.script.say(0x23 + rng.below(2) as u16),
        (Some(0x59), _) | (_, Some(0x59)) => talk.script.say(0x59),
        (Some(first), second) => {
            story.remember_topic(TopicCategory::Misc, cleaned);
            talk.script.say(first);
            if let Some(second) = second {
                talk.script.say(0x21);
                talk.script.say(second);
            }
        }
    }
    commands.trigger(Continue { conversation });
}
