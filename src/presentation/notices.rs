//! The text box's messages (decomp `MGSTextBox.cpp`): trigger messages,
//! "Nothing happens.", camping and the fight's course, in retail's three
//! channels (General, Combat, Dialog) of up to 350 lines, each message
//! wrapped to the box's width less 10 (`ShowNotice`). A message goes to the
//! Combat channel during a fight, and the box shows Combat while fighting
//! and General otherwise (`Combat.cpp` `SelectTextBox`).

use bevy::prelude::*;

use crate::content::ui::{SpriteSheet, TextTable};
use crate::game::camp::CampNews;
use crate::game::combat::CombatNews;
use crate::game::world::level::Level;
use crate::game::world::triggers::TriggerEffect;

/// Retail's "Nothing happens." in the game's string table.
const NOTHING_HAPPENS: usize = 0x964;
/// Lines a channel keeps.
const MAX_LINES: usize = 350;
/// Lines the box shows.
pub const VISIBLE_LINES: usize = 7;
/// The body is 166..456 wide; messages wrap 10 pixels short of it.
const WRAP_WIDTH: u32 = 456 - 166 - 10;

#[derive(Resource)]
pub struct NoticeArt {
    font: Handle<SpriteSheet>,
    strings: Handle<TextTable>,
}

pub fn load_notice_art(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(NoticeArt {
        font: assets.load("retail://data/fonts/wiz_text_font.sti"),
        strings: assets.load("retail://data/strings/stringdata.dat"),
    });
}

/// The text box's channels and which one it shows.
#[derive(Resource, Debug, Default)]
pub struct TextLog {
    channels: [Vec<String>; 3],
    pub selected: usize,
}

impl TextLog {
    /// The last lines of the shown channel.
    pub fn visible(&self) -> &[String] {
        let lines = &self.channels[self.selected];
        &lines[lines.len().saturating_sub(VISIBLE_LINES)..]
    }

    fn push(&mut self, channel: usize, text: &str, font: Option<&SpriteSheet>) {
        info!("notice: {text}");
        let lines = &mut self.channels[channel];
        match font {
            Some(font) => lines.extend(wrap(text, WRAP_WIDTH, |t| font.text_width(t))),
            None => lines.push(text.to_owned()),
        }
        let excess = lines.len().saturating_sub(MAX_LINES);
        lines.drain(..excess);
    }
}

/// Break a message into lines no wider than `width`, at spaces, keeping at
/// least one word on each line (retail `ShowNotice`).
fn wrap(text: &str, width: u32, measure: impl Fn(&str) -> u32) -> Vec<String> {
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
const GOLD_BEFORE: usize = 0x15f;
const GOLD_AFTER: usize = 0x160;

/// Camping's notices.
const CAMPING_FOR: usize = 0x790;
const CAMP_DONE: usize = 0x791;
const CAMP_INTERRUPTED: usize = 0x792;
const CAMP_CANCELLED: usize = 0x793;
const FORCED_TO_CAMP: usize = 0x794;
const RECOVERED: usize = 0x795;
const MOVING: usize = 0x796;
const NOT_IN_COMBAT: usize = 0x774;

const COMBAT: usize = 0x224;
const LOST: usize = 0x230;
const SURVIVORS_EARNED: usize = 0x231;
const WON: usize = 0x22c;
const NOTHING_TO_FIGHT: usize = 0x22e;

/// Turn trigger effects, camping and combat news into notice texts.
pub fn queue_notices(
    mut effects: MessageReader<TriggerEffect>,
    mut camp: MessageReader<CampNews>,
    mut combat: MessageReader<CombatNews>,
    art: Res<NoticeArt>,
    tables: Res<Assets<TextTable>>,
    levels: Query<&Level>,
    battle: Option<Res<crate::game::combat::Battle>>,
    sheets: Res<Assets<SpriteSheet>>,
    mut log: ResMut<TextLog>,
) {
    let font = sheets.get(&art.font);
    let channel = usize::from(battle.is_some());
    let messages = levels.iter().find_map(|level| tables.get(level.messages.as_ref()?));
    let strings = tables.get(&art.strings);
    let string = |index| strings.and_then(|s| s.get(index)).unwrap_or_default().trim().to_owned();
    for effect in effects.read() {
        let text = match effect {
            TriggerEffect::Message(id) => {
                messages.and_then(|m| m.get(usize::try_from(*id).ok()?)).map(str::to_owned)
            }
            TriggerEffect::NothingHappened => Some(string(NOTHING_HAPPENS)),
            // Whizzardry's notice for a locked door without its key.
            TriggerEffect::Locked => Some("Your party doesn't have required key.".to_owned()),
            TriggerEffect::PartyTakes(item) => Some(string(PARTY_TAKES).replace("%s", item)),
            TriggerEffect::Gold(gold) => {
                Some(format!("{} {gold} {}.", string(GOLD_BEFORE), string(GOLD_AFTER)))
            }
            TriggerEffect::Sound { .. }
            | TriggerEffect::Shake { .. }
            | TriggerEffect::Hurt { .. } => None,
        };
        if let Some(text) = text.filter(|t| !t.trim().is_empty()) {
            log.push(channel, text.trim(), font);
        }
    }
    for news in camp.read() {
        let text = match news {
            CampNews::Started { forced: false } => string(CAMPING_FOR).replace("%d", "8"),
            CampNews::Started { forced: true } => string(FORCED_TO_CAMP),
            CampNews::Moving => string(MOVING),
            CampNews::InCombat => string(NOT_IN_COMBAT),
            CampNews::Done => string(CAMP_DONE),
            CampNews::Interrupted => string(CAMP_INTERRUPTED),
            CampNews::Cancelled => string(CAMP_CANCELLED),
            CampNews::Recovered => string(RECOVERED),
        };
        log.push(channel, &text, font);
    }
    // Blow by blow goes to the log; the text box carries the fight's
    // course (retail's own blow lines are not ported yet).
    for news in combat.read() {
        let texts = match news {
            CombatNews::Began => {
                log.selected = 1;
                vec![string(COMBAT)]
            }
            CombatNews::Hit { attacker, target, damage } => {
                info!("combat: {attacker} hits {target} for {damage}");
                vec![]
            }
            CombatNews::Missed { attacker, target } => {
                info!("combat: {attacker} misses {target}");
                vec![]
            }
            CombatNews::Killed { name } => vec![format!("{name} dies!")],
            CombatNews::Won { experience_each, .. } => vec![
                string(WON),
                string(SURVIVORS_EARNED).replace("%d", &experience_each.to_string()),
            ],
            CombatNews::Lost => vec![string(LOST)],
            CombatNews::Over => vec![string(NOTHING_TO_FIGHT)],
        };
        let ended = matches!(news, CombatNews::Won { .. } | CombatNews::Lost | CombatNews::Over);
        for text in texts {
            log.push(1, &text, font);
        }
        if ended {
            log.selected = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_wrap_at_spaces_and_keep_a_word_per_line() {
        let measure = |t: &str| t.len() as u32;
        assert_eq!(wrap("one two three", 8, measure), vec!["one two", "three"]);
        assert_eq!(wrap("unbreakable", 3, measure), vec!["unbreakable"]);
    }
}
