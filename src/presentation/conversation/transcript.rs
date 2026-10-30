//! The session owns history; these retained rows project it and localized strings.
use super::*;

/// What the NPC says as it hands over gold or an item, and as an item leaves
/// a member's pack or the party's items (retail `ProcessNpcQuoteEntry`,
/// `RemoveNpcScriptItem`), and the experience notice (`AwardPartyExperience`).
const QUEST_GOLD: usize = 0x7e9;
const QUEST_ITEM: usize = 0x7ea;
const ITEM_TAKEN_FROM: usize = 0x7ec;
const ITEM_TAKEN: usize = 0x7ed;
const QUEST_EXPERIENCE: usize = 0x232;
/// What an NPC says, in turn, to a question it has nothing to say to.
const NO_ANSWER: [usize; 2] = [1867, 1868];

pub(super) fn sync_transcript(
    views: ViewScope,
    mut commands: Commands,
    assets: Res<AssetServer>,
    strings: crate::presentation::screen_art::GameStrings,
    sessions: Query<Ref<Conversation>>,
    lists: Query<(Entity, Ref<Summary>, Option<&Children>)>,
    mut rows: Query<(&TranscriptRow, &mut BitmapText)>,
) {
    for (list, summary, children) in &lists {
        let Some((_, session)) = views.owner(list) else { continue };
        let Ok(c) = sessions.get(session) else { continue };
        if !summary.is_added() && !c.is_changed() && !strings.is_changed() {
            continue;
        }
        // The current line is still in its speech bubble. Do not print it a
        // second time in the transcript before playback has finished.
        let count = c
            .transcript
            .len()
            .saturating_sub(usize::from(matches!(c.wait, ConversationWait::Line { .. })));
        let lines: Vec<_> = c.transcript[..count]
            .iter()
            .map(|line| match line {
                TranscriptLine::Npc(text) => (std::borrow::Cow::Borrowed(text.as_str()), None),
                // Retail alternates its two answers for having nothing to say.
                TranscriptLine::NoAnswer { npc, second } => (
                    std::borrow::Cow::Owned(
                        strings.get(NO_ANSWER[usize::from(*second)]).replacen("%s", npc, 1),
                    ),
                    None,
                ),
                TranscriptLine::Question(text) => (
                    std::borrow::Cow::Borrowed(text.as_str()),
                    Some("retail://data/fonts/palettebronze.sti"),
                ),
                TranscriptLine::Reply(text) => (
                    std::borrow::Cow::Borrowed(text.as_str()),
                    Some("retail://data/fonts/palettegreen.sti"),
                ),
                // Retail shows gifts and removals in the NPC's own quote.
                TranscriptLine::Reward(QuestReward::Gold { giver, amount }) => (
                    std::borrow::Cow::Owned(
                        strings
                            .get(QUEST_GOLD)
                            .replacen("%s", giver, 1)
                            .replace("%d", &amount.to_string()),
                    ),
                    None,
                ),
                TranscriptLine::Reward(QuestReward::Item { name }) => {
                    (std::borrow::Cow::Owned(strings.get(QUEST_ITEM).replacen("%s", name, 1)), None)
                }
                TranscriptLine::Reward(QuestReward::Taken { member: Some(member) }) => (
                    std::borrow::Cow::Owned(strings.get(ITEM_TAKEN_FROM).replacen("%s", member, 1)),
                    None,
                ),
                TranscriptLine::Reward(QuestReward::Taken { member: None }) => {
                    (std::borrow::Cow::Owned(strings.get(ITEM_TAKEN).to_owned()), None)
                }
                TranscriptLine::Reward(QuestReward::Experience { amount_each }) => (
                    std::borrow::Cow::Owned(
                        strings.get(QUEST_EXPERIENCE).replace("%d", &amount_each.to_string()),
                    ),
                    Some("retail://data/fonts/palettegreen.sti"),
                ),
            })
            // A recording without a line ("SOUND") shows nothing.
            .filter(|(text, _)| !text.is_empty())
            .collect();
        let mut trailing = Vec::new();
        if let ConversationWait::Blocked { quote, entry, reason } = &c.wait {
            trailing.push((
                format!("Content gap at quote {quote}, entry {entry:?}: {reason}"),
                Some("retail://data/fonts/palettered.sti"),
            ));
        }
        if let Some(outcome) = c.outcome() {
            let text = match outcome {
                Outcome::NoTopic => "No supported topic matches that input.".into(),
                Outcome::Traded => "Trade complete.".into(),
                Outcome::TradeRefused(error) => format!("Trade refused: {error:?}"),
                Outcome::GiftRefused(error) => format!("Gift refused: {error:?}"),
            };
            trailing.push((text, Some("retail://data/fonts/palettebronze.sti")));
        }
        let old: Vec<_> = children
            .into_iter()
            .flatten()
            .filter_map(|e| rows.get(*e).ok().map(|(row, _)| (*e, row.0)))
            .collect();
        let length = lines.len() + trailing.len();
        let mut changed = old.len() != length;
        for (entity, index) in &old {
            if *index >= length {
                commands.entity(*entity).despawn();
            }
        }
        for (index, (value, palette)) in lines
            .iter()
            .map(|(text, palette)| (text.as_ref(), *palette))
            .chain(trailing.iter().map(|(text, palette)| (text.as_str(), *palette)))
            .enumerate()
        {
            if let Some((entity, _)) = old.iter().find(|(_, i)| *i == index) {
                if let Ok((_, mut current)) = rows.get_mut(*entity) {
                    let palette_changed = current
                        .palette
                        .as_ref()
                        .and_then(|h| h.path())
                        .and_then(|p| p.path().to_str())
                        != palette.map(|path| path.strip_prefix("retail://").unwrap());
                    if current.value != value {
                        current.value = value.to_owned();
                        changed = true;
                    }
                    if palette_changed {
                        current.palette = palette.map(|path| assets.load(path));
                        changed = true;
                    }
                }
            } else {
                commands.spawn((
                    TranscriptRow(index),
                    BitmapText {
                        font: assets.load("retail://data/fonts/wiz_text_font.sti"),
                        palette: palette.map(|path| assets.load(path)),
                        value: value.to_owned(),
                        width: 284.,
                        ..default()
                    },
                    BitmapWrap { width: 284, line_spacing: 0 },
                    Node { position_type: PositionType::Relative, flex_shrink: 0., ..default() },
                    ChildOf(list),
                ));
            }
        }
        if changed {
            commands.entity(list).insert(ScrollToNewest);
        }
    }
}
pub(super) fn scroll_to_newest(
    mut commands: Commands,
    mut lists: Query<(Entity, &ComputedNode, &mut ScrollPosition), With<ScrollToNewest>>,
) {
    for (entity, node, mut scroll) in &mut lists {
        scroll.0.y = (node.content_size().y - node.size().y).max(0.) * node.inverse_scale_factor();
        commands.entity(entity).remove::<ScrollToNewest>();
    }
}
