//! Charm and Mindread (`docs/rules/casting.md` section 8), cast at the one the
//! party is talking with while they wait for the party's word. The cast is paid
//! for and rolled as any other ([`super::casting`]); this is what it then does:
//! the speaker's answer is [`super::workings::speak`], made to happen to the
//! conversation.

use super::casting::{MagicNews, Worked};
use super::reach::Marked;
use super::workings::{self, Reply};
use crate::content::spells::Working;
use crate::game::difficulty::Difficulty;
use crate::game::hits::Effect;
use crate::game::hits::effects::BodyReadOnly;
use crate::game::npcs::Npc;
use crate::game::npcs::conversation::{Conversation, MagicalReply};
use crate::game::rng::GameRng;
use bevy::prelude::*;

/// Charm or Mindread, once cast at the one the party is talking with: a failed
/// save charms them by the cast's amount or opens their mind; a backfire may be
/// noticed.
pub(crate) fn cast(
    worked: On<Worked>,
    mut conversation: Single<&mut Conversation>,
    mut speakers: Query<&mut Npc>,
    bodies: Query<BodyReadOnly>,
    difficulty: Res<Difficulty>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<MagicNews>,
) {
    let Worked { working, mut report, strength, marked, .. } = worked.event().clone();
    if !matches!(working, Working::Charm | Working::Mindread) {
        return;
    }
    let Marked::Body(body) = marked else { return };
    let (Ok(mut npc), Ok(their)) = (speakers.get_mut(conversation.actor), bodies.get(body)) else {
        return;
    };
    let spoken = workings::speak(
        &mut rng,
        (working == Working::Mindread, report.outcome),
        strength,
        *difficulty,
        &their,
        (conversation.charm(), npc.mindread_refusals),
    );
    npc.mindread_refusals = spoken.refusals;
    if spoken.resisted {
        report.effects.push(Effect::Resisted { victim: body });
    }
    // An NPC won over leaves its menu as its script next runs.
    conversation.magical_reply(match spoken.reply {
        Reply::Unnoticed => MagicalReply::Unnoticed,
        Reply::Quote(quote) => MagicalReply::Quote(quote),
        Reply::Charmed(amount) => MagicalReply::Charmed(amount),
    });
    news.write(MagicNews::Done(report));
}
