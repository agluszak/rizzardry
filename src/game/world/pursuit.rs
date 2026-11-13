//! A hostile group that has seen the party makes for it while it is not
//! fighting (retail `DoMonsterRTAI`, the charge): seen now, or within the
//! last four minutes. One badly hurt shadows the party from afar instead.
//! What brings it into a fight is the fight's own rule.
use bevy::prelude::*;
use bevy_landmass::prelude::AgentTarget3d;

use super::hostility::Stance;
use super::identify::{GroupMembers, MonsterGroup};
use super::population::Monster;
use crate::game::combat::Combatant;
use crate::game::combat::monster::sight::Sight;
use crate::game::npcs::conversation::Conversation;
use crate::game::time::GameClock;

/// On a group: when one of its own last made the party out, in game
/// milliseconds.
#[derive(Component, Clone, Copy, Debug)]
pub struct SawParty(pub u64);

/// A sighting is recent for this long (retail's 240 seconds).
const RECENT_MS: u64 = 240_000;

/// One at a fifth of its health or less does not charge.
const TOO_HURT: u32 = 20;

/// One too hurt to charge keeps within this many far clips of the party
/// (retail `MonsterLinkToStartupNavigator`, twice the far clip).
const SHADOWS_WITHIN: f32 = 2.0;

/// On a monster making for the party it has seen, or shadowing it.
#[derive(Component, Clone, Copy, Debug)]
pub struct Charging;

/// Each group notes when it sees the party, and its hostile members charge
/// while the sighting is recent; the rest stop.
pub(super) fn charge(
    groups: Query<(Entity, &GroupMembers, Option<&SawParty>), With<MonsterGroup>>,
    mut walkers: Query<(&mut AgentTarget3d, Has<Charging>), (With<Monster>, Without<Combatant>)>,
    sight: Sight,
    clock: Res<GameClock>,
    conversations: Query<(), With<Conversation>>,
    mut commands: Commands,
) {
    if !conversations.is_empty() {
        return;
    }
    let party = sight.party();
    for (group, members, saw) in &groups {
        // Those whose eyes count: alive and not blinded or worse.
        let watching = |member: &Entity| {
            sight.monster(*member).is_some_and(|(body, _)| {
                !body.vitals.is_dead()
                    && body.conditions.below(crate::game::characters::Condition::Blind)
            })
        };
        let sees = members.iter().filter(watching).any(|member| sight.sees_party(member));
        if sees {
            commands.entity(group).insert(SawParty(clock.ms));
        }
        let recent = sees || saw.is_some_and(|saw| clock.ms - saw.0.min(clock.ms) <= RECENT_MS);
        for member in members.iter() {
            let Ok((mut target, charging)) = walkers.get_mut(member) else { continue };
            let pursues = recent
                && sight.side(member) == Some(Stance::Hostile)
                && sight.monster(member).is_some_and(|(body, _)| body.can_fight);
            let hurt = sight.monster(member).is_some_and(|(body, _)| {
                body.vitals.health.current * 100 <= body.vitals.health.max * TOO_HURT
            });
            if pursues {
                // The hurt keep their distance, closing only when it grows
                // past theirs.
                let closes = !hurt || sight.party_gap(member) > sight.far() * SHADOWS_WITHIN;
                *target = if closes { AgentTarget3d::Entity(party) } else { AgentTarget3d::None };
                if !charging {
                    commands.entity(member).insert(Charging);
                }
            } else if charging {
                *target = AgentTarget3d::None;
                commands.entity(member).remove::<Charging>();
            }
        }
    }
}
