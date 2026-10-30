//! The text box's messages (decomp `MGSTextBox.cpp`): trigger messages,
//! "Nothing happens.", camping and the fight's course, in retail's three
//! channels (General, Combat, Dialog) of up to 350 lines, each message
//! wrapped to the box's width less 10 (`ShowNotice`). A message goes to the
//! Combat channel during a fight, and the box shows Combat while fighting
//! and General otherwise (`Combat.cpp` `SelectTextBox`).

use bevy::prelude::*;

use crate::presentation::screen_art::GameStrings;

use crate::content::items::AttackMode;
use crate::content::species::AttackWord;
use crate::content::ui::{SpriteSheet, TextTable};
use crate::game::camp::CampNews;
use crate::game::combat::{Announcement, CombatNews};
use crate::game::factions::FactionNews;
use crate::game::inventory::ItemNews;
use crate::game::world::identify::Identified;
use crate::game::world::interaction::LockNews;
use crate::game::world::search::SearchNews;
use crate::game::world::triggers::TriggerEffect;

/// Lock and trap notices in the game's string table: a picked lock, a
/// lock forced or not (after the member's name), the trap types' names, and
/// what became of a trap (after its name).
const LOCK_PICKED: usize = 0x7ac;
const LOCK_NOT_FORCED: usize = 0x7ad;
const LOCK_FORCED: usize = 0x7ae;
const TRAP_DISARMED: usize = 0x7b2;
/// Retail's "Nothing happens." in the game's string table.
const NOTHING_HAPPENS: usize = 0x964;
/// Lines a channel keeps.
const MAX_LINES: usize = 350;
/// Lines the box shows.
pub const VISIBLE_LINES: usize = 7;
/// How many lines retail counts as visible when scrolling
/// (`GetTextBoxVisibleLineCount`): one outside dialogue, spells, items and
/// camp, none of which exist yet, so the box scrolls on until only its last
/// line is left at the top. Flagged as uncertain in the research.
const SCROLL_WINDOW: usize = 1;
/// The body is 166..456 wide; messages wrap 10 pixels short of it.
const WRAP_WIDTH: u32 = 456 - 166 - 10;

#[derive(Resource)]
pub struct NoticeArt {
    font: Handle<SpriteSheet>,
}

pub fn load_notice_art(mut commands: Commands, assets: Res<AssetServer>) {
    commands
        .insert_resource(NoticeArt { font: assets.load("retail://data/fonts/wiz_text_font.sti") });
}

/// The text box's channels, the first line each shows, and which one it
/// shows (retail `g_message_storage`, `text_lines`, `text_line_cursor`).
#[derive(Resource, Debug, Default)]
pub struct TextLog {
    channels: [Vec<String>; 3],
    scroll: [usize; 3],
    selected: usize,
}

/// The Dialog channel.
const DIALOG: usize = 2;

impl TextLog {
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Show a channel; the Dialog channel shows its last lines (retail
    /// `TextBoxChannelTabRegionEvent`).
    pub fn select(&mut self, channel: usize) {
        self.selected = channel;
        if channel == DIALOG {
            self.scroll_to(self.channels[channel].len().saturating_sub(VISIBLE_LINES));
        }
    }

    /// The shown channel's lines from its first shown one.
    pub fn visible(&self) -> &[String] {
        let lines = &self.channels[self.selected];
        let first = self.scroll[self.selected].min(lines.len());
        &lines[first..(first + VISIBLE_LINES).min(lines.len())]
    }

    /// Whether the shown channel can scroll up, and down.
    pub fn can_scroll(&self) -> (bool, bool) {
        let count = self.channels[self.selected].len();
        let first = self.scroll[self.selected];
        (first > 0, first + SCROLL_WINDOW < count)
    }

    /// Where the scroll thumb sits, 0 at the top to 1 at the end, when the
    /// text overflows (retail: `scroll × 40 / (lines − visible)`).
    pub fn thumb(&self) -> Option<f32> {
        let count = self.channels[self.selected].len();
        (count > SCROLL_WINDOW)
            .then(|| self.scroll[self.selected] as f32 / (count - SCROLL_WINDOW) as f32)
    }

    /// Map the pointer's retail track position to the selected channel's line.
    pub fn scroll_thumb(&mut self, position: u32) {
        let range = self.channels[self.selected].len().saturating_sub(SCROLL_WINDOW);
        self.scroll_to(range * position.min(46) as usize / 47);
    }

    /// Retail `ScrollTextBoxUp`.
    pub fn scroll_up(&mut self, lines: usize) {
        let first = &mut self.scroll[self.selected];
        *first = first.saturating_sub(lines);
    }

    /// Retail `ScrollTextBoxDown`: no further than the scroll window from
    /// the end.
    pub fn scroll_down(&mut self, lines: usize) {
        let count = self.channels[self.selected].len();
        let first = &mut self.scroll[self.selected];
        if *first + SCROLL_WINDOW < count {
            *first = (*first + lines).min(count - SCROLL_WINDOW);
        }
    }

    /// Retail `ScrollTextBoxTo`, which leaves a box that fits alone.
    pub fn scroll_to(&mut self, line: usize) {
        let count = self.channels[self.selected].len();
        if count > SCROLL_WINDOW {
            self.scroll[self.selected] = line.min(count - SCROLL_WINDOW);
        }
    }

    /// End: the last line, less the scroll window (retail
    /// `ScrollDialogueTextBoxToLine`).
    pub fn scroll_to_end(&mut self) {
        self.scroll_to(self.channels[self.selected].len().saturating_sub(SCROLL_WINDOW));
    }

    /// Del empties the shown channel (retail `ResetEditorStatusLine`).
    pub fn clear(&mut self) {
        self.channels[self.selected].clear();
        self.scroll[self.selected] = 0;
    }

    fn push(&mut self, channel: usize, text: &str, font: Option<&SpriteSheet>) {
        info!("notice: {text}");
        let wrapped = match font {
            Some(font) => wrap(text, WRAP_WIDTH, |t| font.text_width(t)),
            None => vec![text.to_owned()],
        };
        for line in wrapped {
            let lines = &mut self.channels[channel];
            let first = &mut self.scroll[channel];
            if lines.len() == MAX_LINES {
                lines.remove(0);
                *first = first.saturating_sub(1);
            }
            lines.push(line);
            // Each new line brings the last seven into view, unless the box
            // is scrolled further down (retail `AdvanceNoticeLine`).
            let shown = lines.len();
            if shown > VISIBLE_LINES && shown - VISIBLE_LINES > *first {
                *first = shown - VISIBLE_LINES;
            }
        }
    }
}

/// Break a message into lines no wider than `width`, at spaces, keeping at
/// least one word on each line (retail `ShowNotice`).
pub(super) fn wrap(text: &str, width: u32, measure: impl Fn(&str) -> u32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split(' ').filter(|w| !w.is_empty()) {
            let candidate =
                if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
            if measure(&candidate) >= width && !line.is_empty() {
                lines.push(std::mem::replace(&mut line, word.to_owned()));
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    lines
}

/// "Party takes %s", and the two halves of the gold notice.
const PARTY_TAKES: usize = 0x1e9;
const TAKES: usize = 0x1e8;
const NO_ROOM: usize = 0x90c;
/// What retail says when an item that may not be discarded stays in hand.
const CANNOT_DROP: usize = 0x4ef;
/// Merging: nothing is made from these, "%s lacks the skill", and both must
/// be identified first (retail `MergeItems`).
const CANNOT_MERGE: usize = 0x161;
const UNSKILLED_MERGE: usize = 0x162;
const MERGE_UNIDENTIFIED: usize = 0x163;
/// The omnigun given, then each tier raised to (retail
/// `UpgradeProfessionClassItem`), naming its owner by pronoun
/// (`g_gender_name_message_rows[gender][3]` for the gift, `[2]` after).
const OMNIGUN_TIERS: usize = 0x167;
const GIFT_PRONOUNS: usize = 0x2da;
const POSSESSIVES: usize = 0x2d7;
const SEARCH_ON: usize = 0x78e;
const SEARCH_OFF: usize = 0x78f;
const SPOTS_HIDDEN: usize = 0x772;
const IDENTIFIES: usize = 0x1ca;
const ITS_A: usize = 0x1cb;
const ITS_AN: usize = 0x1cc;
const THEYRE: usize = 0x1cd;
const GOLD_BEFORE: usize = 0x15f;
const GOLD_AFTER: usize = 0x160;

/// Camping's notices.
const CAMPING_FOR: usize = 0x790;
const CAMP_DONE: usize = 0x791;
const CAMP_INTERRUPTED: usize = 0x792;
const FORCED_TO_CAMP: usize = 0x794;
const RECOVERED: usize = 0x795;
const MOVING: usize = 0x796;
const NOT_IN_COMBAT: usize = 0x774;

const COMBAT: usize = 0x224;
const LOST: usize = 0x230;
const SURVIVORS_EARNED: usize = 0x231;
const WON: usize = 0x22c;
const NOTHING_TO_FIGHT: usize = 0x22e;
const MISSED: usize = 0x209;
const NO_DAMAGE: usize = 0x20e;
const HITS: usize = 0x265;
const HITS_TIMES: usize = 0x266;
/// The Dead condition's singular notice ("DIES!!").
const DIES: usize = 0x391;
/// "at " after Swing, Thrust and Throw; "upon " after Berserk.
const AT: usize = 0x217;
const UPON: usize = 0x218;
const SNEAKS: usize = 0x204;
const UNAWARE: usize = 0x207;

/// An attack's opening line in retail's terse form (`StartCharacterAttack`,
/// `AnnounceMonsterAttack` without verbose messages): the attacker, the
/// verb, "at" or "upon" where the mode takes it, the target, and the swings
/// when more than one ("Ragnar Swings at Crab", "Crab Claws Pippa 2x").
fn announce(
    string: &impl Fn(usize) -> String,
    attacker: &str,
    target: &str,
    how: Announcement,
    (swings, surprise): (u32, crate::game::combat::attack::Surprise),
) -> String {
    // Retail `g_attack_flag_name_ids`: each mode's noun, then its verb.
    let verb = |mode: AttackMode| string(0x51f + 2 * mode as usize + 1);
    let preposition = |mode: AttackMode| match mode {
        AttackMode::Swing | AttackMode::Thrust | AttackMode::Throw => string(AT),
        AttackMode::Berserk => string(UPON),
        _ => String::new(),
    };
    let (word, preposition) = match how {
        Announcement::Mode(mode) => (verb(mode), preposition(mode)),
        Announcement::Monster { word, mode, named } => (
            match word {
                AttackWord::Verb => verb(mode),
                word => string(word.text_id()),
            },
            if named { preposition(mode) } else { String::new() },
        ),
    };
    let swings = if swings > 1 { format!(" {swings}x") } else { String::new() };
    // A backstab or an ambush says so instead of the verb; a blow from
    // behind says so after the target (retail strings 0x204 and 0x207).
    if surprise.natural {
        return format!("{attacker} {} {target}{swings}", string(SNEAKS));
    }
    let unaware = if surprise.unaware { format!(" {}", string(UNAWARE)) } else { String::new() };
    // `string` trims, so the preposition's own trailing space is restored.
    let preposition = if preposition.is_empty() { preposition } else { format!("{preposition} ") };
    format!("{attacker} {word} {preposition}{target}{unaware}{swings}")
}

/// A condition as notices name it.
fn condition_word(condition: crate::game::characters::Condition) -> String {
    format!("{condition:?}").to_lowercase()
}

/// A member's skill improving.
fn improves(
    names: &super::names::ActorNames,
    string: &impl Fn(usize) -> String,
    member: Entity,
    skill: crate::content::characters::Skill,
) -> String {
    format!(
        "{}'s {} improves.",
        names.name(member).unwrap_or_default(),
        string(crate::content::ui::SKILL_NAMES + skill as usize)
    )
}

/// What a spell or a blow did to someone. Retail's own strings for these
/// are not mapped yet; the wording is plain.
fn effect_line(names: &super::names::ActorNames, effect: crate::game::magic::Effect) -> String {
    use crate::game::magic::{Effect, Victim};
    let victim = |victim: Victim| names.name(victim.entity()).unwrap_or_default();
    match effect {
        Effect::Purified { victim: v } => format!("The air around {} clears.", victim(v)),
        Effect::Enchanted { victim: v, .. } => format!("{} is enchanted.", victim(v)),
        Effect::Healed { victim: v, amount } => {
            format!("{} heals {amount}.", victim(v))
        }
        Effect::Damaged { victim: v, outcome } => {
            format!("{} takes {} damage.", victim(v), outcome.damage)
        }
        Effect::Inflicted { victim: v, condition } => {
            format!("{} is {}.", victim(v), condition_word(condition))
        }
        Effect::Refreshed { victim: v, amount } => {
            format!("{} regains {amount} stamina.", victim(v))
        }
        Effect::Replenished { victim: v, amount } => {
            format!("{} regains {amount} spell points.", victim(v))
        }
        Effect::Cured { victim: v, condition } => {
            format!("{} is no longer {}.", victim(v), condition_word(condition))
        }
        Effect::GoldLost { amount } => format!("The party loses {amount} gold."),
        Effect::Destroyed { class, amount } => format!("{amount} {class:?} items are destroyed."),
        Effect::Drained { victim: v, amount } => {
            format!("{} loses {amount} spell points.", victim(v))
        }
        Effect::Tired { victim: v, amount } => format!("{} loses {amount} stamina.", victim(v)),
        Effect::Raised { victim: v } => format!("{} lives again!", victim(v)),
        Effect::Resisted { victim: v } => format!("{} resists.", victim(v)),
        Effect::Reflected { victim: v } => format!("{} reflects the spell.", victim(v)),
        Effect::Nothing { victim: v } => format!("{} is unaffected.", victim(v)),
    }
}

/// What a cast or an item use tells the player. Retail's own strings for
/// these are not mapped yet; the wording is plain.
fn magic_lines(
    news: &crate::game::magic::casting::MagicNews,
    names: &super::names::ActorNames,
    spells: &crate::content::spells::SpellCatalog,
    string: &impl Fn(usize) -> String,
) -> Vec<String> {
    use crate::game::magic::Outcome;
    use crate::game::magic::casting::{MagicNews, Refusal};
    let name = |entity| names.name(entity).unwrap_or_default();
    let improves = |member, skill| improves(names, string, member, skill);
    let effect_line = |effect| effect_line(names, effect);
    match news {
        MagicNews::Refused { caster, refusal } => {
            let why = match refusal {
                Refusal::CannotAct => "cannot act",
                Refusal::Silenced => "is silenced",
                Refusal::NotLearned => "does not know that spell",
                Refusal::NoSpellPoints => "has too few spell points",
                Refusal::NoItem => "no longer has that item",
                Refusal::NoCharges => "finds it has no charges left",
                Refusal::CannotUse => "cannot use that",
                Refusal::Unsupported => "cannot do that yet",
                Refusal::WrongTime => "cannot do that now",
                Refusal::NoTarget => "has no target in reach",
                Refusal::PlayedBadly { .. } => "fails to play it",
            };
            let mut lines = vec![format!("{} {why}.", name(*caster))];
            if let Refusal::PlayedBadly { improved: Some(skill) } = refusal {
                lines.push(format!("{}'s {skill:?} improves.", name(*caster)));
            }
            lines
        }
        MagicNews::Triggered { effects, improved, .. }
        | MagicNews::Landed { effects, improved, .. } => {
            let mut lines: Vec<_> = effects.iter().map(|&e| effect_line(e)).collect();
            lines.extend(improved.iter().map(|&(member, skill)| improves(member, skill)));
            lines
        }
        MagicNews::Done(report) => {
            let spell = spells.get(report.spell).map_or("", |s| s.name.as_str());
            let mut lines = vec![match &report.item {
                Some(item) => format!("{} uses {item}.", name(report.caster)),
                None => format!("{} casts {spell} ({}).", name(report.caster), report.power),
            }];
            match report.outcome {
                Outcome::Fizzled => lines.push("The spell fizzles.".into()),
                Outcome::Backfired => lines.push("The spell backfires!".into()),
                Outcome::Cast => {}
            }
            lines.extend(report.effects.iter().map(|&e| effect_line(e)));
            for &(member, skill) in &report.improved {
                lines.push(improves(member, skill));
            }
            lines
        }
    }
}

/// Turn trigger effects, camping and combat news into notice texts.
/// Retail `AdjustFactionDisposition`'s notice: "%s standing %s." with the
/// faction's name (`g_faction_name_ids`, whose two spare factions reuse the
/// first name) and whether it worsened or improved.
fn faction_line(news: &FactionNews, string: &impl Fn(usize) -> String) -> String {
    const NAMES: usize = 0x5a1;
    const WORSENED: usize = 0x245;
    const IMPROVED: usize = 0x246;
    const CHANGED: usize = 0x247;
    let name = match usize::try_from(news.faction) {
        Ok(faction @ 0..=18) => NAMES + faction,
        _ => NAMES,
    };
    string(CHANGED).replacen("%s", &string(name), 1).replacen(
        "%s",
        &string(if news.improved { IMPROVED } else { WORSENED }),
        1,
    )
}

/// Where a notice goes: the text log's general channel, or its combat
/// channel during a fight.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Notices<'w, 's> {
    log: ResMut<'w, TextLog>,
    art: Res<'w, NoticeArt>,
    sheets: Res<'w, Assets<SpriteSheet>>,
    battles: Query<'w, 's, (), With<crate::game::combat::Battle>>,
}

impl Notices<'_, '_> {
    fn post(&mut self, text: &str) {
        self.post_to(usize::from(!self.battles.is_empty()), text);
    }

    fn post_to(&mut self, channel: usize, text: &str) {
        self.log.push(channel, text, self.sheets.get(&self.art.font));
    }
}

/// Every kind of news posts its notices, in this order within a frame.
pub fn queue_notices() -> bevy::ecs::schedule::ScheduleConfigs<bevy::ecs::system::ScheduleSystem> {
    (
        post_remarks,
        post_magic,
        post_speech,
        post_factions,
        post_trigger_effects,
        post_identified,
        post_search,
        post_items,
        post_locks,
        post_camp,
        post_combat,
    )
        .chain()
        .in_set(QueueNotices)
}

/// The systems [`queue_notices`] schedules.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct QueueNotices;

fn post_remarks(
    mut remarks: MessageReader<super::party_voice::SpokenRemark>,
    names: super::names::ActorNames,
    mut notices: Notices,
) {
    for remark in remarks.read() {
        if let Some(name) = names.name(remark.actor) {
            notices.post(&format!("{name}: {}", remark.text));
        }
    }
}

fn post_magic(
    mut magic: MessageReader<crate::game::magic::casting::MagicNews>,
    names: super::names::ActorNames,
    content: If<Res<crate::game::data::Catalogs>>,
    strings: GameStrings,
    mut notices: Notices,
) {
    let spells = &content.spells;
    let string = |index| strings.get(index).to_owned();
    for news in magic.read() {
        for line in magic_lines(news, &names, spells, &string) {
            notices.post(&line);
        }
    }
}

fn post_speech(
    mut speech: MessageReader<crate::game::npcs::conversation::NpcSpeech>,
    mut notices: Notices,
) {
    // Retail `SpeakNpcSubquote` outside dialogue mode.
    for line in speech.read() {
        notices.post(&format!("{}: \"{}\"", line.speaker, line.text));
    }
}

fn post_factions(
    mut factions: MessageReader<FactionNews>,
    strings: GameStrings,
    mut notices: Notices,
) {
    let string = |index| strings.get(index).to_owned();
    for news in factions.read() {
        notices.post(&faction_line(news, &string));
    }
}

fn post_trigger_effects(
    mut effects: MessageReader<TriggerEffect>,
    tables: Res<Assets<TextTable>>,
    strings: GameStrings,
    mut notices: Notices,
) {
    let string = |index| strings.get(index).to_owned();
    for effect in effects.read() {
        let text = match effect {
            TriggerEffect::Message { source, id } => source
                .as_ref()
                .and_then(|source| tables.get(source))
                .and_then(|messages| messages.get(usize::try_from(*id).ok()?))
                .map(str::to_owned),
            TriggerEffect::Unsupported { device, code } => {
                Some(format!("{device}: action {code:#x} is not implemented."))
            }
            TriggerEffect::NothingHappened => Some(string(NOTHING_HAPPENS)),
            // Whizzardry's notice for a locked door without its key.
            TriggerEffect::Locked => Some("Your party doesn't have required key.".to_owned()),
            TriggerEffect::Gold(gold) => {
                Some(format!("{} {gold} {}.", string(GOLD_BEFORE), string(GOLD_AFTER)))
            }
            TriggerEffect::Sound { .. }
            | TriggerEffect::FountainUsed { .. }
            | TriggerEffect::BellRang(_)
            | TriggerEffect::Shake { .. } => None,
        };
        if let Some(text) = text.filter(|t| !t.trim().is_empty()) {
            notices.post(text.trim());
        }
    }
}

fn post_identified(
    mut identified: MessageReader<Identified>,
    strings: GameStrings,
    mut notices: Notices,
) {
    let string = |index| strings.get(index).to_owned();
    // Retail `DetectMonsterGroups`: "Ragnar identifies the Crocodiles:",
    // then "They're Ironskin Crocodiles!" ("It's a" / "It's an" for one).
    for news in identified.read() {
        let line =
            format!("{} {}", news.member, string(IDENTIFIES).replace("%s", &news.looks_like));
        notices.post(&line);
        let vowel =
            news.is.chars().next().is_some_and(|c| "AEIOU".contains(c.to_ascii_uppercase()));
        let article = match (news.count, vowel) {
            (1, false) => string(ITS_A),
            (1, true) => string(ITS_AN),
            _ => string(THEYRE),
        };
        notices.post(&format!("{article} {}!", news.is));
    }
}

fn post_search(mut search: MessageReader<SearchNews>, strings: GameStrings, mut notices: Notices) {
    let string = |index| strings.get(index).to_owned();
    for news in search.read() {
        let text = match news {
            SearchNews::On => string(SEARCH_ON),
            SearchNews::Off => string(SEARCH_OFF),
            SearchNews::NotInCombat => string(NOT_IN_COMBAT),
            SearchNews::Spotted { member, item, .. } => {
                format!("{member} {}", string(SPOTS_HIDDEN).replace("%s", item))
            }
        };
        notices.post(&text);
    }
}

fn post_items(mut items: MessageReader<ItemNews>, strings: GameStrings, mut notices: Notices) {
    let string = |index| strings.get(index).to_owned();
    for news in items.read() {
        let text = match news {
            ItemNews::Takes { member: Some(member), item } => {
                format!("{member} {}", string(TAKES).replace("%s", item))
            }
            ItemNews::Takes { member: None, item } => string(PARTY_TAKES).replace("%s", item),
            ItemNews::NoRoom => string(NO_ROOM),
            ItemNews::CannotDrop => string(CANNOT_DROP),
            ItemNews::CannotMerge => string(CANNOT_MERGE),
            ItemNews::Unskilled { member } => string(UNSKILLED_MERGE).replace("%s", member),
            ItemNews::Unidentified => string(MERGE_UNIDENTIFIED),
            ItemNews::Omnigun { member, gender, tier } => {
                let pronouns = if *tier == 0 { GIFT_PRONOUNS } else { POSSESSIVES };
                let pronoun = string(pronouns + *gender as usize);
                let text = string(OMNIGUN_TIERS + tier).replace("%s", &pronoun);
                // Retail runs a notice starting with an apostrophe or colon
                // straight on from the name (`PostCharacterNoticeText`).
                let separator = if text.starts_with(['\'', ':']) { "" } else { " " };
                format!("{member}{separator}{text}")
            }
        };
        notices.post(&text);
    }
}

fn post_locks(
    mut locks: MessageReader<LockNews>,
    members: Query<&crate::game::characters::Character>,
    strings: GameStrings,
    mut notices: Notices,
) {
    let string = |index| strings.get(index).to_owned();
    // Retail's lock and trap notices (`AttemptForce`, `ResolvePick`,
    // `CompleteTrapDisarm`, `ResolveSprungTrap`); the rest are voices.
    let member = |entity: &Entity| members.get(*entity).map(|c| c.name.clone()).unwrap_or_default();
    let trap = |trap: &u8| string(crate::content::ui::TRAP_NAMES + usize::from(*trap));
    for news in locks.read() {
        let text = match news {
            LockNews::Picked => string(LOCK_PICKED),
            LockNews::Sprung { trap: kind, destroyed } => {
                format!("{} {}", trap(kind), string(if *destroyed { 0x7b3 } else { 0x7b4 }))
            }
            LockNews::SkillImproved { member: who } => {
                format!("{}'s Locks & Traps improves.", member(who))
            }
            LockNews::Forced { member: who } => format!("{} {}", member(who), string(LOCK_FORCED)),
            LockNews::NotForced { member: who } => {
                format!("{} {}", member(who), string(LOCK_NOT_FORCED))
            }
            LockNews::Disarmed { trap: kind } => {
                format!("{} {}", trap(kind), string(TRAP_DISARMED))
            }
            LockNews::UnsupportedTrap => {
                "This trap effect is not implemented yet; the device remains guarded.".to_owned()
            }
            LockNews::Locked
            | LockNews::Trapped { .. }
            | LockNews::NotInCombat
            | LockNews::Tumbler { .. }
            | LockNews::Inspected { .. }
            | LockNews::ComponentDisarmed { .. } => continue,
        };
        notices.post(&text);
    }
}

fn post_camp(mut camp: MessageReader<CampNews>, strings: GameStrings, mut notices: Notices) {
    let string = |index| strings.get(index).to_owned();
    for news in camp.read() {
        let text = match news {
            CampNews::Started { forced: false } => string(CAMPING_FOR).replace("%d", "8"),
            CampNews::Started { forced: true } => string(FORCED_TO_CAMP),
            CampNews::Moving => string(MOVING),
            CampNews::InCombat => string(NOT_IN_COMBAT),
            CampNews::Done => string(CAMP_DONE),
            CampNews::Interrupted => string(CAMP_INTERRUPTED),
            CampNews::Recovered => string(RECOVERED),
        };
        notices.post(&text);
    }
}

fn post_combat(
    mut combat: MessageReader<CombatNews>,
    names: super::names::ActorNames,
    strings: GameStrings,
    mut notices: Notices,
) {
    let string = |index| strings.get(index).to_owned();
    // Blow by blow goes to the log; the text box carries the fight's
    // course (retail's own blow lines are not ported yet).
    for news in combat.read() {
        let texts = match news {
            CombatNews::Began { .. } => {
                notices.log.selected = 1;
                vec![string(COMBAT)]
            }
            CombatNews::Breathed { actor } => {
                names.name(*actor).map(|name| vec![format!("{name} breathes!")]).unwrap_or_default()
            }
            CombatNews::Surprised(cause) => vec![string(match cause {
                crate::game::combat::SurpriseCause::Ambush => 0x240,
                crate::game::combat::SurpriseCause::Running => 0x23e,
                crate::game::combat::SurpriseCause::Searching => 0x23f,
            })],
            // Retail notices 0x21b and 0x21c: the party moves, and stops.
            CombatNews::PartyMoves => vec![string(0x21b)],
            CombatNews::PartyStops => vec![string(0x21c)],
            // Retail strings 0x236 ("advances") and 0x237 ("charges").
            CombatNews::Moves { monster, charge } => names
                .name(*monster)
                .map(|name| vec![format!("{name} {}", string(if *charge { 0x237 } else { 0x236 }))])
                .unwrap_or_default(),
            // Retail `StartMonsterCombatAction`'s notices: 0x23b takes the
            // monster's name and possessive, 0x23a its name.
            CombatNews::Flees { monster } => names
                .name(*monster)
                .map(|name| {
                    let pronouns = usize::from(names.pronouns(*monster).unwrap_or(2));
                    let possessive =
                        string(if pronouns < 3 { POSSESSIVES + pronouns } else { 0x2df });
                    vec![string(0x23b).replacen("%s", &name, 1).replacen("%s", &possessive, 1)]
                })
                .unwrap_or_default(),
            // Retail `MonsterFleeAction`: the name and the attack's own
            // (`g_monster_special_attack_name_ids`, known for kinds 1 to 11).
            CombatNews::Special { monster, kind } => names
                .name(*monster)
                .map(|name| match kind.row() {
                    row @ 1..=11 => vec![format!("{name} {}", string(1597 + usize::from(row)))],
                    _ => vec![],
                })
                .unwrap_or_default(),
            // Retail `g_condition_interrupt_notices` (0x3a0 on, twelve of
            // them); fear's flight has none.
            CombatNews::Interrupted { monster, interrupt } => names
                .name(*monster)
                .filter(|_| interrupt.number() < 12)
                .map(|name| {
                    vec![format!("{name} {}", string(0x3a0 + usize::from(interrupt.number())))]
                })
                .unwrap_or_default(),
            // Retail string 0x21a after the guard's name, naming its ward.
            CombatNews::Intercepts { guard, ward } => {
                match (names.name(*guard), names.name(*ward)) {
                    (Some(guard), Some(ward)) => {
                        vec![format!("{guard} {}", string(0x21a).replacen("%s", &ward, 1))]
                    }
                    _ => vec![],
                }
            }
            CombatNews::Withdraws { monster } => names
                .name(*monster)
                .map(|name| vec![string(0x23a).replacen("%s", &name, 1)])
                .unwrap_or_default(),
            CombatNews::SummonFails => vec![string(0x26a)],
            CombatNews::Attack { attacker, target, how, swings, surprise } => {
                match (names.name(*attacker), names.name(*target)) {
                    (Some(attacker), Some(target)) => {
                        vec![announce(&string, &attacker, &target, *how, (*swings, *surprise))]
                    }
                    _ => vec![],
                }
            }
            // Retail `AnnounceAccidentalStrike`'s own words.
            CombatNews::Fumble { attacker, victim } => {
                match (names.name(*attacker), names.name(victim.entity())) {
                    (Some(attacker), Some(victim)) => {
                        vec![format!("{attacker} accidentally strikes {victim}")]
                    }
                    _ => vec![],
                }
            }
            CombatNews::Effects(effects) => {
                effects.iter().map(|&effect| effect_line(&names, effect)).collect()
            }
            CombatNews::Improved { member, skill } => {
                vec![improves(&names, &string, *member, *skill)]
            }
            // Retail's summary (`ReportCharacterAttackResult`).
            CombatNews::Result { attacker, hits, damage, .. } => {
                let Some(attacker) = names.name(*attacker) else { continue };
                vec![match (hits, damage) {
                    (0, _) => string(MISSED),
                    (_, 0) => string(NO_DAMAGE),
                    (1, damage) => {
                        format!("{attacker} {}", string(HITS).replace("%ld", &damage.to_string()))
                    }
                    (hits, damage) => format!(
                        "{attacker} {}",
                        string(HITS_TIMES).replacen("%ld", &hits.to_string(), 1).replacen(
                            "%ld",
                            &damage.to_string(),
                            1
                        )
                    ),
                }]
            }
            CombatNews::Killed { actor } => names
                .name(*actor)
                .map(|name| vec![format!("{name} {}", string(DIES))])
                .unwrap_or_default(),
            CombatNews::Won { experience_each, .. } => vec![
                string(WON),
                string(SURVIVORS_EARNED).replace("%d", &experience_each.to_string()),
            ],
            CombatNews::Lost => vec![string(LOST)],
            CombatNews::Over => vec![string(NOTHING_TO_FIGHT)],
        };
        let ended = matches!(news, CombatNews::Won { .. } | CombatNews::Lost | CombatNews::Over);
        // Retail CheckCombatEnd explicitly sends outcomes to General before
        // EndCombat selects it; only the preceding blows belong to Combat.
        for text in texts {
            notices.post_to(usize::from(!ended), &text);
        }
        if ended {
            notices.log.selected = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A faction's notice names it and says which way its standing went;
    /// retail's spare factions borrow the first name.
    #[test]
    fn faction_notices_name_the_faction_and_the_direction() {
        let string = |index: usize| match index {
            0x245 => "worsened".to_owned(),
            0x246 => "improved".to_owned(),
            0x247 => "%s standing %s.".to_owned(),
            0x5a1 => "Unaligned".to_owned(),
            0x5a5 => "Umpani".to_owned(),
            _ => String::new(),
        };
        let news = |faction, improved| crate::game::factions::FactionNews { faction, improved };
        assert_eq!(faction_line(&news(4, false), &string), "Umpani standing worsened.");
        assert_eq!(faction_line(&news(20, true), &string), "Unaligned standing improved.");
    }

    #[test]
    fn combat_outcomes_are_visible_after_returning_to_general() {
        let mut app = App::new();
        app.init_resource::<crate::game::data::Catalogs>();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<SpriteSheet>>()
            .init_resource::<Assets<TextTable>>()
            .insert_resource(NoticeArt { font: default() })
            .init_resource::<TextLog>()
            .add_message::<TriggerEffect>()
            .add_message::<CampNews>()
            .add_message::<CombatNews>()
            .add_message::<ItemNews>()
            .add_message::<SearchNews>()
            .add_message::<Identified>()
            .add_message::<LockNews>()
            .add_message::<super::super::party_voice::SpokenRemark>()
            .add_message::<crate::game::magic::casting::MagicNews>()
            .add_message::<crate::game::factions::FactionNews>()
            .add_message::<crate::game::npcs::conversation::NpcSpeech>()
            .add_systems(Update, queue_notices());
        let mut strings = vec![None; SURVIVORS_EARNED + 1];
        for (id, text) in [
            (COMBAT, "Combat"),
            (WON, "Won"),
            (LOST, "Lost"),
            (NOTHING_TO_FIGHT, "Over"),
            (SURVIVORS_EARNED, "%d XP each"),
        ] {
            strings[id] = Some(text.into());
        }
        let table = app.world_mut().resource_mut::<Assets<TextTable>>().add(TextTable(strings));
        app.insert_resource(super::super::screen_art::StringTable(table));
        for (outcome, expected) in [
            (CombatNews::Won { kills: 3, experience_each: 111 }, vec!["Won", "111 XP each"]),
            (CombatNews::Lost, vec!["Lost"]),
            (CombatNews::Over, vec!["Over"]),
        ] {
            *app.world_mut().resource_mut::<TextLog>() = TextLog::default();
            app.world_mut().write_message(CombatNews::Began { nearest: None });
            app.update();
            assert_eq!(app.world().resource::<TextLog>().selected(), 1);
            app.world_mut().write_message(outcome);
            app.update();
            let log = app.world().resource::<TextLog>();
            assert_eq!(log.selected(), 0);
            assert_eq!(log.visible(), expected);
            assert_eq!(log.channels[1], ["Combat"]);
        }
    }

    #[test]
    fn the_box_follows_new_lines_and_scrolls_to_its_last() {
        let mut log = TextLog::default();
        for n in 0..10 {
            log.push(0, &n.to_string(), None);
        }
        // The last seven show.
        assert_eq!(log.visible().first().map(String::as_str), Some("3"));
        assert_eq!(log.can_scroll(), (true, true));
        log.scroll_up(7);
        assert_eq!(log.visible()[0], "0");
        assert_eq!(log.can_scroll(), (false, true));
        // A new line brings the newest back into view.
        log.push(0, "10", None);
        assert_eq!(log.visible()[0], "4");
        log.scroll_thumb(0);
        assert_eq!(log.visible()[0], "0");
        log.scroll_thumb(23);
        assert_eq!(log.visible()[0], "4");
        log.scroll_thumb(46);
        assert_eq!(log.visible()[0], "9");
        // Down stops with the last line alone at the top.
        log.scroll_down(100);
        assert_eq!(log.visible(), ["10"]);
        assert_eq!(log.can_scroll(), (true, false));
        log.clear();
        assert!(log.visible().is_empty());
    }

    #[test]
    fn messages_wrap_at_spaces_and_keep_a_word_per_line() {
        let measure = |t: &str| t.len() as u32;
        assert_eq!(wrap("one two three", 8, measure), vec!["one two", "three"]);
        assert_eq!(wrap("unbreakable", 3, measure), vec!["unbreakable"]);
    }
}
