//! Groups coming into a fight and leaving it (`docs/rules/monsters.md`
//! section 3; retail `MonsterGroupCanEngage`, `ShouldMonsterGroupEnterCombat`,
//! `CheckMonsterGroupsLeaveCombat`).

use bevy::prelude::Entity;

use super::close::reaches_party;
use super::sight::{Fray, Sight};
use super::{special, spell};
use crate::content::species::Reach;
use crate::content::spells::{SpellCatalog, SpellId};
use crate::game::hits::definitions::Definitions;
use crate::game::world::hostility::Stance;

/// How far from the party a group still takes part (retail's 50 m).
pub const ENGAGEMENT_RANGE: f32 = 50.0;

/// What is added to it for a leader's reach (retail's 15 m).
pub const ENGAGEMENT_FLOOR: f32 = 15.0;

/// How much longer the way round by a member may be than the straight line
/// from the party to a hostile.
pub const REINFORCEMENT_SLACK: f32 = 1.15;

/// Whether a group stands with the fight: one of its `members` in the
/// party's view, not of a kind that keeps hidden and within 50 m of the
/// party, stands between the party and one of the fight's `hostiles` in its
/// view: the way from the party by the member to the hostile is under 1.15
/// times the hostile's own gap, and the member is nearer the party, or
/// nearer the hostile, than the hostile is to the party.
pub fn stands_with(sight: &Sight, members: &[Entity], hostiles: &[Entity]) -> bool {
    let hidden = |m| sight.minds.get(m).is_ok_and(|mind| mind.temper.0.camouflaged);
    members
        .iter()
        .filter(|&&m| sight.in_party_view(m) && !hidden(m))
        .filter(|&&m| sight.party_gap(m) <= ENGAGEMENT_RANGE)
        .any(|&member| {
            let (gap, feet) = (sight.party_gap(member), sight.places.of(member).feet);
            hostiles.iter().filter(|&&h| h != member && sight.in_party_view(h)).any(|&hostile| {
                let theirs = sight.party_gap(hostile);
                let between = feet.distance(sight.places.of(hostile).feet);
                between + gap < theirs * REINFORCEMENT_SLACK && (gap < theirs || between < theirs)
            })
        })
}

/// Whether a monster can act on the party with no fight on yet: it can
/// fight, and an arm reaches the party, or it walks and its arms are for
/// close quarters, or it could cast at its foes, or its special attack is
/// ready.
pub fn acts_on_party(
    sight: &Sight,
    fighter: Entity,
    (spells, definitions): (&SpellCatalog, &Definitions),
) -> bool {
    let Some((own, mind)) = sight.monster(fighter) else { return false };
    if !own.vitals.can_fight() {
        return false;
    }
    let gap = sight.party_gap(fighter);
    let close =
        own.attacks.arms.iter().map(|arm| arm.reach).max().is_some_and(|r| r <= Reach::Short);
    // What it sees of the fight to be: itself and the party it makes out.
    let bodies: Vec<Entity> =
        std::iter::once(fighter).chain(sight.roster().iter().copied()).collect();
    let fray = Fray { bodies: &bodies, fighting: false, sees_party: sight.sees_party(fighter) };
    let breath = || {
        spells
            .get(SpellId::SPECIAL_ATTACK_CONE)
            .map_or(Vec::new(), |cone| spell::aims(sight, &own, cone, definitions, fray))
    };
    reaches_party(own.attacks, gap)
        || (close && mind.monster.body.speed > 0.0)
        || spell::casts_at_foes(sight, &own, &mind, (spells, definitions), fray)
        || special::ready(&own, &mind, gap, false, &breath())
}

/// What became of a group outside the fight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entered {
    pub enters: bool,
    /// Its kind's hostility radius turned it on the party.
    pub turns_hostile: bool,
}

/// Whether a group outside the fight comes in: its `fighters` (the first its
/// leader), on the side `stance`, of a kind with a hostility `radius` or
/// none.
///
/// A group not hostile whose kind has a radius turns hostile when a member
/// sees the party and its nearest is within the radius. A hostile group
/// comes in at once when the party has just acted against it. Otherwise,
/// with no fight on, it opens one when a member sees the party and a member
/// can act on it; into a running fight it comes when a member sees the party
/// and its leader is in reach of it: within its longest reach and a stride,
/// 65 m at the least. A neutral group joins only a running fight it stands
/// with, turning hostile as it does if its kind has a radius. A friendly
/// group never comes in.
pub fn comes_in(
    sight: &Sight,
    fighters: &[Entity],
    (stance, radius): (Stance, Option<f32>),
    (fight_running, provoked): (bool, bool),
    hostiles: &[Entity],
    records: (&SpellCatalog, &Definitions),
) -> Entered {
    let nearest = fighters.iter().map(|&m| sight.party_gap(m)).fold(f32::INFINITY, f32::min);
    let sees = || fighters.iter().any(|&m| sight.sees_party(m));
    let turns = stance != Stance::Hostile && radius.is_some_and(|r| nearest <= r) && sees();
    let leader_in_reach = || {
        fighters.first().and_then(|&leader| sight.bodies.get(leader).ok()).is_some_and(|leader| {
            let reach =
                leader.attacks.arms.iter().map(|arm| arm.reach.metres()).fold(0.0, f32::max)
                    + leader.attacks.stride.unwrap_or(0.0);
            sight.party_gap(leader.entity) <= reach.max(ENGAGEMENT_RANGE + ENGAGEMENT_FLOOR)
        })
    };
    let enters = match if turns { Stance::Hostile } else { stance } {
        Stance::Hostile if provoked => true,
        Stance::Hostile if fight_running => sees() && leader_in_reach(),
        Stance::Hostile => sees() && fighters.iter().any(|&m| acts_on_party(sight, m, records)),
        Stance::Neutral if fight_running => stands_with(sight, fighters, hostiles),
        _ => false,
    };
    let radius_as_it_enters = enters && stance == Stance::Neutral && radius.is_some();
    Entered { enters, turns_hostile: turns || radius_as_it_enters }
}

/// Whether a group in the fight stays in it as a round ends, by its
/// `members` still fighting, and whether it keeps the fight engaged. A
/// hostile group stays while a member makes out the party or sees a foe of
/// its own, and is engaged only by the party; a friendly one stays while a
/// member sees a foe; a neutral one while it stands with the fight.
pub fn stays(
    sight: &Sight,
    stance: Stance,
    members: &[Entity],
    hostiles: &[Entity],
    combatants: &[Entity],
) -> (bool, bool) {
    let sees_foe = || members.iter().any(|&m| sight.sees_foe(m, combatants));
    match stance {
        Stance::Neutral => (stands_with(sight, members, hostiles), false),
        Stance::Hostile => {
            let engaged = members.iter().any(|&m| sight.sees_party(m));
            (engaged || sees_foe(), engaged)
        }
        Stance::Friendly => (sees_foe(), false),
    }
}
