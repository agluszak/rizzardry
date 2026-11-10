//! Groups coming into a fight and leaving it (`docs/rules/monsters.md`
//! section 3; retail `MonsterGroupCanEngage`, `ShouldMonsterGroupEnterCombat`,
//! `AlertSameFactionGroups`, `CheckMonsterGroupsLeaveCombat`).

use bevy::prelude::{Entity, Vec3};

use super::close::{Striking, reaches_party};
use crate::content::species::Reach;
use crate::game::world::hostility::Stance;

/// How far from the party a neutral member may stand and still reinforce a
/// fight (retail `GetMonsterEngagementRange`, 25000 world units).
pub const ENGAGEMENT_RANGE: f32 = 50.0;

/// What a group's leader reaches at least, beyond the engagement range, to
/// join a running fight (retail `g_monster_engagement_range_floor`).
pub const ENGAGEMENT_FLOOR: f32 = 15.0;

/// How far out of its way a neutral member may stand from a hostile's line to
/// the party and still be between them (retail `g_reinforcement_distance_slack`).
pub const REINFORCEMENT_SLACK: f32 = 1.15;

/// A fighter as the reinforcement rule weighs it.
#[derive(Clone, Copy, Debug)]
pub struct Standing {
    pub body: Entity,
    pub feet: Vec3,
    /// The gap to the party.
    pub party_gap: f32,
    /// The party has a clear line to it within the far clip.
    pub in_view: bool,
    /// Its kind keeps its place hidden.
    pub camouflaged: bool,
}

/// Whether a neutral group stands with a fight (retail
/// `MonsterGroupHasReinforcement`): one of its `members` the party has in view,
/// not camouflaged, within 50 m of the party, stands roughly between the party
/// and one of the fight's `hostiles` in the party's view. That is, its gap to
/// that hostile plus its gap to the party is under 1.15 times the hostile's gap
/// to the party, and it is nearer the party than the hostile or nearer the
/// hostile than the hostile is to the party.
pub fn stands_with(members: &[Standing], hostiles: &[Standing]) -> bool {
    members.iter().filter(|m| m.in_view && !m.camouflaged && m.party_gap <= ENGAGEMENT_RANGE).any(
        |member| {
            hostiles.iter().filter(|h| h.in_view && h.body != member.body).any(|hostile| {
                let theirs = hostile.party_gap;
                let between = member.feet.distance(hostile.feet);
                between + member.party_gap < theirs * REINFORCEMENT_SLACK
                    && (member.party_gap < theirs || between < theirs)
            })
        },
    )
}

/// What a member of a group outside the fight can do about the party.
#[derive(Clone, Copy, Debug)]
pub struct Acting<'a> {
    /// It can take a combat action.
    pub can_fight: bool,
    /// What its arms reach of the party.
    pub striking: Striking<'a>,
    pub walks: bool,
    /// One of its spells aimed at enemies is castable with a target.
    pub casts: bool,
    /// Its special attack is ready with an aim.
    pub breathes: bool,
}

/// Whether a member can act on the party: it can take a combat action, and one
/// of its attacks reaches the party now, or its longest attack is touch or short
/// and it can walk, or it can cast at a foe, or breathe.
pub fn acts_on_party(facts: &Acting) -> bool {
    let close = facts.striking.arms.iter().max().is_some_and(|&longest| longest <= Reach::Short);
    facts.can_fight
        && (reaches_party(&facts.striking)
            || (close && facts.walks)
            || facts.casts
            || facts.breathes)
}

/// A member of a group outside the fight as the group's coming in weighs it.
#[derive(Clone, Copy, Debug)]
pub struct Looking {
    /// The gap to the party.
    pub party_gap: f32,
    /// It makes out the party.
    pub sees_party: bool,
    /// It can act on the party ([`acts_on_party`]).
    pub acts: bool,
}

/// A group's leader.
#[derive(Clone, Copy, Debug)]
pub struct Leader {
    pub party_gap: f32,
    /// The reach of its longest attack, if it has one.
    pub reach: Option<Reach>,
    /// How far it moves in a combat move.
    pub stride: f32,
}

impl Leader {
    /// Whether it is in reach of the party: its longest attack's reach plus one
    /// combat move, but never under 65 m (the engagement range 50 m plus a
    /// floor of 15 m).
    fn in_reach_of_party(&self) -> bool {
        let reach = (self.reach.map_or(0.0, Reach::metres) + self.stride)
            .max(ENGAGEMENT_RANGE + ENGAGEMENT_FLOOR);
        self.party_gap <= reach
    }
}

/// What a group's coming in weighs.
#[derive(Clone, Copy, Debug)]
pub struct Entering<'a> {
    /// The side its group stands on.
    pub stance: Stance,
    /// A fight is running already.
    pub fight_running: bool,
    /// The party has just acted against it.
    pub provoked: bool,
    /// The hostility radius its kind turns hostile within, if it has one.
    pub radius: Option<f32>,
    /// Its living members that fight and are not in the fight already.
    pub members: &'a [Looking],
    pub leader: Leader,
    /// The same members as the reinforcement rule weighs them, and the fight's
    /// hostile fighters.
    pub standing: &'a [Standing],
    pub hostiles: &'a [Standing],
}

/// What came of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entered {
    pub enters: bool,
    /// The group is to be made hostile.
    pub turns_hostile: bool,
}

/// Whether a group comes into a fight, and whether it turns hostile doing so.
///
/// A group of a kind with a hostility radius that is not hostile turns hostile
/// when its nearest member is within the radius and a member makes out the party.
/// Then a hostile group comes in when the party has just acted against it, or,
/// if no fight runs, a member makes out the party and a member can act on it, or,
/// if one runs, a member makes out the party and its leader is in reach of the
/// party. A neutral group joins a running fight it stands with, and turns
/// hostile as it does if its kind has a radius. A friendly group never comes in.
pub fn comes_in(facts: &Entering) -> Entered {
    let nearest = facts.members.iter().map(|m| m.party_gap).fold(f32::INFINITY, f32::min);
    let sees = facts.members.iter().any(|m| m.sees_party);
    let acts = facts.members.iter().any(|m| m.acts);
    let turns = facts.stance != Stance::Hostile
        && sees
        && facts.radius.is_some_and(|radius| nearest <= radius);
    let enters = match if turns { Stance::Hostile } else { facts.stance } {
        Stance::Hostile if facts.fight_running => {
            facts.provoked || (sees && facts.leader.in_reach_of_party())
        }
        Stance::Hostile => facts.provoked || (sees && acts),
        Stance::Neutral if facts.fight_running => stands_with(facts.standing, facts.hostiles),
        _ => false,
    };
    let radius_as_it_enters = enters && facts.stance == Stance::Neutral && facts.radius.is_some();
    Entered { enters, turns_hostile: turns || radius_as_it_enters }
}

/// Another group's leader as a group's leader weighs it for an alert.
#[derive(Clone, Copy, Debug)]
pub struct Alert {
    /// It is not the group's own.
    pub other: bool,
    /// Its group is of the same faction.
    pub same_faction: bool,
    /// It is fighting and hostile.
    pub fighting_hostile: bool,
    /// The group's leader sees it. Only looked for when the rest holds.
    pub seen: bool,
}

/// What a group's being alerted weighs.
#[derive(Clone, Copy, Debug)]
pub struct Alerted<'a> {
    pub npc: bool,
    pub unaligned: bool,
    /// The group is hostile already.
    pub hostile: bool,
    pub leaders: &'a [Alert],
}

/// Whether a group turns hostile because a fight is running (retail
/// `AlertSameFactionGroups`): a group of a faction, not an NPC's and not
/// hostile already, turns hostile if its leader sees the leader of another,
/// fighting hostile group of the same faction.
pub fn is_alerted(facts: &Alerted) -> bool {
    !facts.npc
        && !facts.unaligned
        && !facts.hostile
        && facts.leaders.iter().any(|l| l.other && l.same_faction && l.fighting_hostile && l.seen)
}

/// A member of a group in the fight as its leaving weighs it.
#[derive(Clone, Copy, Debug)]
pub struct Remaining {
    /// It makes out the party.
    pub sees_party: bool,
    /// It sees a living foe of its own.
    pub sees_foe: bool,
}

/// What a group in the fight weighs in leaving it.
#[derive(Clone, Copy, Debug)]
pub struct Leaving<'a> {
    pub stance: Stance,
    /// Its living members in the fight.
    pub members: &'a [Remaining],
    pub standing: &'a [Standing],
    pub hostiles: &'a [Standing],
}

/// Which groups leave the fight, and whether everyone does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leaves {
    /// For each group, in order, whether it leaves.
    pub groups: Vec<bool>,
    /// No hostile group is left engaged: everyone leaves at once.
    pub everyone: bool,
}

/// Who leaves the fight at a round's end (retail `CheckMonsterGroupsLeaveCombat`,
/// `MonsterAI.cpp:2065-2152`): a neutral group unless it still stands with the
/// fight; a friendly group unless one of its members sees a foe of its own; a
/// hostile group unless a living member makes out the party or sees a foe of its
/// own, and it is engaged while a member makes out the party. A hostile fighter
/// outside any group keeps the others engaged. When no hostile group is left
/// engaged, everyone leaves.
pub fn leaves(groups: &[Leaving], ungrouped_hostile: bool) -> Leaves {
    let mut engaged = ungrouped_hostile;
    let leaving = groups
        .iter()
        .map(|group| {
            let any = |seen: fn(&Remaining) -> bool| group.members.iter().any(seen);
            let stays = match group.stance {
                Stance::Neutral => stands_with(group.standing, group.hostiles),
                Stance::Hostile => {
                    engaged |= any(|m| m.sees_party);
                    any(|m| m.sees_party || m.sees_foe)
                }
                Stance::Friendly => any(|m| m.sees_foe),
            };
            !stays
        })
        .collect();
    Leaves { groups: leaving, everyone: !engaged }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(index: u32) -> Entity {
        Entity::from_raw_u32(index).unwrap()
    }

    /// A fighter on the party's line along +x at `feet`, `party_gap` from it.
    fn at(index: u32, x: f32, party_gap: f32) -> Standing {
        Standing {
            body: entity(index),
            feet: Vec3::new(x, 0.0, 0.0),
            party_gap,
            in_view: true,
            camouflaged: false,
        }
    }

    #[test]
    fn the_slack_is_fifteen_percent_of_the_hostiles_gap() {
        // Hostile 30 off: the limit is 34.5. Member 10 off with the hostile 24 from
        // it: 34 < 34.5 stands with; 25 from it: 35 > 34.5 does not.
        let hostile = |between: f32| Standing {
            feet: Vec3::new(10.0 + between, 0.0, 0.0),
            ..at(2, 0.0, 30.0)
        };
        assert!(stands_with(&[at(1, 10.0, 10.0)], &[hostile(24.0)]));
        assert!(!stands_with(&[at(1, 10.0, 10.0)], &[hostile(25.0)]));
    }

    #[test]
    fn a_member_further_than_fifty_metres_from_the_party_does_not_stand_with_it() {
        // 50 m counts, 50.1 m does not, with a hostile right beside it.
        assert!(stands_with(&[at(1, 50.0, 50.0)], &[at(2, 50.0, 60.0)]));
        assert!(!stands_with(&[at(1, 50.1, 50.1)], &[at(2, 50.1, 60.0)]));
    }

    fn looks(party_gap: f32, sees_party: bool, acts: bool) -> Looking {
        Looking { party_gap, sees_party, acts }
    }

    fn leader(party_gap: f32) -> Leader {
        Leader { party_gap, reach: Some(Reach::Touch), stride: 20.0 }
    }

    /// A hostile group of two, the party just off, no fight running.
    fn entering<'a>(members: &'a [Looking]) -> Entering<'a> {
        Entering {
            stance: Stance::Hostile,
            fight_running: false,
            provoked: false,
            radius: None,
            members,
            leader: leader(10.0),
            standing: &[],
            hostiles: &[],
        }
    }

    #[test]
    fn a_hostile_group_opens_a_fight_when_a_member_sees_the_party_and_a_member_can_act() {
        let both = [looks(30.0, true, false), looks(40.0, false, true)];
        assert!(comes_in(&entering(&both)).enters, "one sees, another can act");
        let blind = [looks(30.0, false, true)];
        assert!(!comes_in(&entering(&blind)).enters, "none sees");
        let helpless = [looks(30.0, true, false)];
        assert!(!comes_in(&entering(&helpless)).enters, "none can act");
        assert!(!comes_in(&entering(&[])).enters);
    }

    #[test]
    fn a_hostile_group_joins_a_running_fight_when_it_sees_the_party_and_its_leader_is_in_reach() {
        let sees = [looks(30.0, true, false)];
        let joins = |party_gap, reach, stride, sees: &[Looking]| {
            let facts = Entering {
                fight_running: true,
                leader: Leader { party_gap, reach: Some(reach), stride },
                ..entering(sees)
            };
            comes_in(&facts).enters
        };
        // Touch 2 + 20 = 22 is under the floor 50 + 15 = 65: 65 m counts, 65.1 not.
        assert!(joins(65.0, Reach::Touch, 20.0, &sees));
        assert!(!joins(65.1, Reach::Touch, 20.0, &sees));
        // Extreme 50 + a stride of 40 = 90 is over the floor: 90 counts, 91 not.
        assert!(joins(90.0, Reach::Extreme, 40.0, &sees));
        assert!(!joins(90.5, Reach::Extreme, 40.0, &sees));
        // Short 4 + 0 is under the floor, as is long 25 + 30 = 55.
        assert!(joins(65.0, Reach::Long, 30.0, &sees));
        // It must see the party all the same.
        assert!(!joins(10.0, Reach::Touch, 20.0, &[looks(30.0, false, true)]));
        // And acting on it is not asked of it in a running fight.
        assert!(joins(10.0, Reach::Touch, 20.0, &[looks(30.0, true, false)]));
    }

    #[test]
    fn a_neutral_group_joins_only_a_running_fight_it_stands_with() {
        let (member, hostile) = (at(1, 10.0, 10.0), at(2, 30.0, 30.0));
        let members = [looks(10.0, true, true)];
        let neutral = Entering {
            stance: Stance::Neutral,
            standing: &[member],
            hostiles: &[hostile],
            ..entering(&members)
        };
        assert!(!comes_in(&neutral).enters, "no fight is running");
        let running = Entering { fight_running: true, ..neutral };
        assert!(comes_in(&running).enters);
        assert!(!comes_in(&Entering { hostiles: &[], ..running }).enters);
        // Standing with a fight does not turn a kind with no radius hostile.
        assert!(!comes_in(&running).turns_hostile);
        // A kind with a hostility radius turns hostile as it enters, even from afar.
        let radius = Entering { radius: Some(5.0), ..running };
        assert_eq!(comes_in(&radius), Entered { enters: true, turns_hostile: true });
    }

    #[test]
    fn a_neutral_group_with_a_radius_turns_hostile_when_its_nearest_member_is_within_it_and_sees() {
        let near = [looks(10.0, true, true), looks(30.0, false, false)];
        let neutral = Entering { stance: Stance::Neutral, radius: Some(10.0), ..entering(&near) };
        // Within 10 m and seen: turned hostile, and as a hostile group sees and acts.
        assert_eq!(comes_in(&neutral), Entered { enters: true, turns_hostile: true });
        // 10.5 m is outside.
        let outside = [looks(10.5, true, true)];
        let neutral = Entering { members: &outside, ..neutral };
        assert_eq!(comes_in(&neutral), Entered { enters: false, turns_hostile: false });
        // In the radius but nobody makes out the party.
        let unseen = [looks(5.0, false, true)];
        let neutral = Entering { members: &unseen, ..neutral };
        assert_eq!(comes_in(&neutral), Entered { enters: false, turns_hostile: false });
        // A kind with no radius is never turned by distance.
        let near = [looks(1.0, true, true)];
        let plain = Entering { stance: Stance::Neutral, radius: None, ..entering(&near) };
        assert_eq!(comes_in(&plain), Entered { enters: false, turns_hostile: false });
    }

    fn alerted(leaders: &[Alert]) -> Alerted<'_> {
        Alerted { npc: false, unaligned: false, hostile: false, leaders }
    }

    #[test]
    fn a_group_of_a_faction_turns_hostile_when_its_leader_sees_a_fighting_hostile_leader_of_it() {
        let source = Alert { other: true, same_faction: true, fighting_hostile: true, seen: true };
        assert!(is_alerted(&alerted(&[source])));
        // Each term: not seen, another faction, not fighting hostile, its own.
        for broken in [
            Alert { seen: false, ..source },
            Alert { same_faction: false, ..source },
            Alert { fighting_hostile: false, ..source },
            Alert { other: false, ..source },
        ] {
            assert!(!is_alerted(&alerted(&[broken])), "{broken:?}");
        }
        assert!(!is_alerted(&alerted(&[])));
        // Any one of several will do.
        assert!(is_alerted(&alerted(&[Alert { seen: false, ..source }, source])));
        // An NPC's group, an unaligned one and a hostile one are not alerted.
        let leaders = [source];
        for broken in [
            Alerted { npc: true, ..alerted(&leaders) },
            Alerted { unaligned: true, ..alerted(&leaders) },
            Alerted { hostile: true, ..alerted(&leaders) },
        ] {
            assert!(!is_alerted(&broken), "{broken:?}");
        }
    }

    fn sees(party: bool, foe: bool) -> Remaining {
        Remaining { sees_party: party, sees_foe: foe }
    }

    fn leaving<'a>(stance: Stance, members: &'a [Remaining]) -> Leaving<'a> {
        Leaving { stance, members, standing: &[], hostiles: &[] }
    }

    #[test]
    fn a_hostile_group_stays_while_a_member_makes_out_the_party_or_sees_a_foe_of_its_own() {
        let party = [sees(true, false), sees(false, false)];
        assert_eq!(leaves(&[leaving(Stance::Hostile, &party)], false).groups, [false]);
        // Another monster that is its foe keeps it though the party is out of sight.
        let foe = [sees(false, true)];
        assert_eq!(leaves(&[leaving(Stance::Hostile, &foe)], false).groups, [false]);
        let nothing = [sees(false, false)];
        assert_eq!(leaves(&[leaving(Stance::Hostile, &nothing)], false).groups, [true]);
        assert_eq!(leaves(&[leaving(Stance::Hostile, &[])], false).groups, [true]);
    }

    #[test]
    fn everyone_leaves_when_no_hostile_group_is_left_engaged() {
        // A hostile group that only sees a foe monster keeps staying but is not engaged.
        let foe = [sees(false, true)];
        let only_foe = leaves(&[leaving(Stance::Hostile, &foe)], false);
        assert_eq!(only_foe, Leaves { groups: vec![false], everyone: true });
        // Seeing the party engages it.
        let party = [sees(true, false)];
        let engaged = leaves(&[leaving(Stance::Hostile, &party)], false);
        assert_eq!(engaged, Leaves { groups: vec![false], everyone: false });
        // A friendly or neutral group does not engage.
        let friendly = leaves(&[leaving(Stance::Friendly, &foe)], false);
        assert!(friendly.everyone);
        // A hostile fighter outside any group keeps the fight engaged.
        let kept = leaves(&[leaving(Stance::Friendly, &foe)], true);
        assert!(!kept.everyone);
        assert!(leaves(&[], false).everyone);
        assert!(!leaves(&[], true).everyone);
    }
}
