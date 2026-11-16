//! What a monster makes out (`docs/rules/monsters.md` sections 2 and 4):
//! who stands where, on which side, and who sees whom ([`Sight`]).

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::game::actors::{CanFight, Level};
use crate::game::actors::{Eyes, Hidden, Laid, Places};
use crate::game::characters::Vitals;
use crate::game::party::roster::PartyRoster;
use crate::game::world::hostility::{self, Side, Stance, Standing};
use crate::game::world::identify::GroupMember;
use crate::game::world::level::PlayedLevel;
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::Temper;

/// How far from the party a group still takes part (retail's 50 m).
pub const ENGAGEMENT_RANGE: f32 = 50.0;

/// How much longer the way round by a member may be than the straight line
/// from the party to a hostile.
const REINFORCEMENT_SLACK: f32 = 1.15;

/// Everyone in the played level, where they stand, and who makes out whom.
#[derive(SystemParam)]
pub struct Sight<'w, 's> {
    /// Who is alive, and on which side.
    standing: Query<'w, 's, (&'static Vitals, &'static Side, &'static Standing)>,
    /// How each hides: by its level, if it can fight.
    hiding: Query<'w, 's, (&'static Level, Has<CanFight>)>,
    /// The group each is of, if any.
    pub groups: Query<'w, 's, &'static GroupMember>,
    /// The temper of each that decides for itself.
    pub tempers: Query<'w, 's, &'static Temper>,
    eyes: Query<'w, 's, &'static Eyes>,
    pub places: Places<'w, 's>,
    pub laid: Laid<'w, 's>,
    pub obstacles: WorldObstacles<'w, 's>,
    party: Single<'w, 's, (Entity, &'static PartyRoster)>,
    level: PlayedLevel<'w, 's>,
}

impl Sight<'_, '_> {
    /// The party, which every member stands in.
    pub fn party(&self) -> Entity {
        self.party.0
    }

    pub fn roster(&self) -> &PartyRoster {
        self.party.1
    }

    /// The far clip, past which nobody sees anybody.
    /// How far the level lets one see: its far clip.
    pub fn far(&self) -> f32 {
        self.level.view_distance().unwrap_or(0.0)
    }

    pub fn alive(&self, body: Entity) -> bool {
        self.standing.get(body).is_ok_and(|(vitals, ..)| !vitals.is_dead())
    }

    /// The side a body's group or kind has it on, its own turning aside.
    pub fn side(&self, body: Entity) -> Option<Stance> {
        self.standing.get(body).ok().map(|(_, side, _)| side.own())
    }

    /// How one living body stands toward another, itself a friend.
    pub fn toward(&self, own: Entity, other: Entity) -> Option<Stance> {
        let (_, _, mine) = self.standing.get(own).ok()?;
        let (vitals, _, theirs) = self.standing.get(other).ok()?;
        if vitals.is_dead() {
            return None;
        }
        Some(if other == own { Stance::Friendly } else { hostility::between(*mine, *theirs) })
    }

    /// How far a body is from the party.
    pub fn party_gap(&self, body: Entity) -> f32 {
        self.places.gap(body, self.party())
    }

    /// Whether the party has a clear line to a body within the far clip.
    pub fn in_party_view(&self, body: Entity) -> bool {
        let party = self.places.of(self.party());
        party.in_view(&self.places.of(body), self.far(), &self.obstacles)
    }

    /// Whether a body's eyes make out the party, which hides by its lowest
    /// able level and by Chameleon.
    pub fn sees_party(&self, watcher: Entity) -> bool {
        let Ok(eyes) = self.eyes.get(watcher) else { return false };
        let members = self
            .roster()
            .iter()
            .filter_map(|member| self.hiding.get(member).ok())
            .filter(|(_, fit)| *fit)
            .map(|(level, _)| level);
        let hidden = Hidden::party(members, self.laid.bonus(self.party()).concealment);
        eyes.see(
            &self.places.of(watcher),
            (&self.places.of(self.party()), hidden),
            self.far(),
            &self.obstacles,
        )
    }

    /// Whether a body's eyes make out a monster, which hides by its level.
    pub fn sees(&self, watcher: Entity, watched: Entity) -> bool {
        let (Ok(eyes), Ok((level, _))) = (self.eyes.get(watcher), self.hiding.get(watched)) else {
            return false;
        };
        eyes.see(
            &self.places.of(watcher),
            (&self.places.of(watched), Hidden::alone(*level)),
            self.far(),
            &self.obstacles,
        )
    }

    /// Whether a body sees a living foe of its own among `others`: another
    /// monster it sees, or the party, if it makes it out and a member
    /// stands against it.
    pub fn sees_foe(&self, watcher: Entity, others: &[Entity]) -> bool {
        let foe = |other: Entity| self.toward(watcher, other) == Some(Stance::Hostile);
        let member = |other: &Entity| self.roster().contains(other);
        others.iter().any(|&other| !member(&other) && foe(other) && self.sees(watcher, other))
            || (others.iter().any(|&other| member(&other) && foe(other))
                && self.sees_party(watcher))
    }

    /// Whether a group stands with the fight (retail
    /// `MonsterGroupHasReinforcement`): one of its `members` in the party's
    /// view, not of a kind that keeps hidden and within 50 m of the party,
    /// stands between the party and one of the fight's `hostiles` in its
    /// view: the way from the party by the member to the hostile is under
    /// 1.15 times the hostile's own gap, and the member is nearer the party,
    /// or nearer the hostile, than the hostile is to the party.
    pub fn stands_with(&self, members: &[Entity], hostiles: &[Entity]) -> bool {
        let hidden = |m| self.tempers.get(m).is_ok_and(|temper| temper.0.camouflaged);
        members
            .iter()
            .filter(|&&m| self.in_party_view(m) && !hidden(m))
            .filter(|&&m| self.party_gap(m) <= ENGAGEMENT_RANGE)
            .any(|&member| {
                let (gap, feet) = (self.party_gap(member), self.places.of(member).feet);
                hostiles.iter().filter(|&&h| h != member && self.in_party_view(h)).any(|&hostile| {
                    let theirs = self.party_gap(hostile);
                    let between = feet.distance(self.places.of(hostile).feet);
                    between + gap < theirs * REINFORCEMENT_SLACK
                        && (gap < theirs || between < theirs)
                })
            })
    }
}
