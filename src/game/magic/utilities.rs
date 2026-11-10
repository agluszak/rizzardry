//! Identify Item (`docs/rules/casting.md` section 8): once the cast comes off
//! it reveals what the party has aimed it at. The cast is paid for and rolled
//! as any other ([`super::casting`]); this is what it then does.

use super::casting::{MagicNews, Worked};
use super::{Outcome, power, reach::Marked};
use crate::content::spells::Working;
use crate::game::data::Catalogs;
use crate::game::items::{Items, Known};
use bevy::prelude::*;

/// Identify Item on a stack the party holds, wherever it has moved among the
/// members and the pool since it was chosen. Success reveals its identity,
/// spell and binding; there is no item roll.
pub(crate) fn identify(
    worked: On<Worked>,
    held: Items,
    content: Res<Catalogs>,
    mut news: MessageWriter<MagicNews>,
    mut commands: Commands,
) {
    let Worked { working, report, strength, marked, .. } = worked.event().clone();
    if working != Working::Identify {
        return;
    }
    let Marked::Item(target) = marked else { return };
    let hard = held.id(target).map(|id| content.items[id].identify_difficulty);
    if report.outcome == Outcome::Cast
        && hard.is_some_and(|hard| power::reveals(strength.power, strength.boost, hard))
    {
        commands.entity(target).insert(Known { identified: true, spell: true, curse: true });
    }
    news.write(MagicNews::Done(report));
}
