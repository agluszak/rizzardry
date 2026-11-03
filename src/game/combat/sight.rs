//! How far a fight reaches: the ranges groups engage and reinforce within,
//! and whether a neutral group stands with a fight.

use super::*;
use crate::game::world::footing::Footing;

/// How far from the party a neutral member may stand and still reinforce a
/// fight (retail `GetMonsterEngagementRange`, 25000 world units).
pub(super) const ENGAGEMENT_RANGE: f32 = 50.0;

/// What a group's leader reaches at least, beyond the engagement range, to
/// join a running fight (retail `g_monster_engagement_range_floor`).
pub(super) const ENGAGEMENT_FLOOR: f32 = 15.0;

/// How far out of its way a neutral member may stand from a hostile's line
/// to the party and still be between them (retail
/// `g_reinforcement_distance_slack`).
pub(super) const REINFORCEMENT_SLACK: f32 = 1.15;

/// Whether a neutral group stands with a fight (retail
/// `MonsterGroupHasReinforcement`): one of its `members` (those the party
/// has in view, not camouflaged), within the engagement range, stands
/// roughly between the party and one of the fight's `hostiles` in the
/// party's view.
pub(super) fn reinforces(
    party: &Footing,
    members: &[(Entity, &Footing)],
    hostiles: &[(Entity, &Footing)],
) -> bool {
    members.iter().filter(|(_, member)| party.gap(member) <= ENGAGEMENT_RANGE).any(
        |&(member, at)| {
            let from_party = party.gap(at);
            hostiles.iter().filter(|&&(other, _)| other != member).any(|(_, other)| {
                let theirs = party.gap(other);
                let between = at.feet.distance(other.feet);
                between + from_party < theirs * REINFORCEMENT_SLACK
                    && (from_party < theirs || between < theirs)
            })
        },
    )
}
