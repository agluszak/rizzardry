//! Identify Item (`docs/rules/casting.md` section 8): once the cast comes off
//! it reveals what the party has aimed it at. The cast is paid for and rolled
//! as any other ([`super::casting`]); these are what it is cast at and what it
//! then does.

use super::casting::{Allowed, CastBy, Chosen, MagicNews, Worked, marked};
use super::may::Refusal;
use super::reach::Aim;
use super::{Outcome, power, reach::Marked};
use crate::game::items::{Items, Known, Place};
use crate::game::party::roster::PartyRoster;
use bevy::prelude::*;

/// Identify Item is cast at something the party has (in hand, in the pool,
/// or with a member), not the item casting it, not wholly known.
pub fn mark(
    allowed: On<Allowed>,
    casts: Query<(&CastBy, &Chosen)>,
    held: Items,
    roster: Single<&PartyRoster>,
    mut commands: Commands,
) -> Result {
    let (&CastBy(caster), &Chosen(choice)) = casts.get(allowed.cast)?;
    let item = match choice.aim {
        Aim::Item(item) => Some(item),
        _ => None,
    };
    let fit = item.filter(|&item| {
        let ours = held.place(item).is_some_and(|place| match place {
            Place::Slot { member, .. } | Place::Pack { member, .. } => roster.contains(&member),
            _ => true,
        });
        let whole =
            held.known(item).is_some_and(|known| known.identified && known.spell && known.curse);
        ours && held.known(item).is_some() && choice.castable != item && !whole
    });
    marked(&mut commands, (allowed.cast, caster), fit.map(Marked::Item).ok_or(Refusal::NoTarget));
    Ok(())
}

/// Identify Item on a stack the party holds, wherever it has moved among the
/// members and the pool since it was chosen. Success reveals its identity,
/// spell and binding; there is no item roll.
pub fn identify(
    worked: On<Worked>,
    held: Items,
    mut news: MessageWriter<MagicNews>,
    mut commands: Commands,
) {
    let Worked { report, strength, marked, .. } = worked.event().clone();
    let Marked::Item(target) = marked else { return };
    let hard = held.obscurity(target);
    if report.outcome == Outcome::Cast
        && hard.is_some_and(|hard| power::reveals(strength.power, strength.boost, hard))
    {
        commands.entity(target).insert(Known { identified: true, spell: true, curse: true });
    }
    news.write(MagicNews::Done(report));
}
