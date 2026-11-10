//! How far a fight reaches: the ranges groups engage and reinforce within,
//! and whether a neutral group stands with a fight.

use bevy::ecs::system::SystemParam;

use super::*;
use crate::game::actors::{Attacks, Attributes, Eyes, Hidden, Laid, Level, Place, Places};
use crate::game::world::hostility::{Loyalty, Side};

/// Who in and about a fight sees whom, and how they stand: asked of the
/// actors as they are now.
#[derive(SystemParam)]
pub(super) struct Sight<'w, 's> {
    actors: Query<
        'w,
        's,
        (
            &'static Vitals,
            &'static Attributes,
            &'static Attacks,
            &'static Level,
            &'static Side,
            &'static Loyalty,
        ),
    >,
    pub places: Places<'w, 's>,
    laid: Laid<'w, 's>,
    party: Single<'w, 's, (Entity, &'static PartyRoster)>,
    level: PlayedLevel<'w, 's>,
    pub obstacles: WorldObstacles<'w, 's>,
}

impl Sight<'_, '_> {
    /// The far clip, past which nobody sees anybody.
    fn far(&self) -> f32 {
        self.level.view_distance().unwrap_or(0.0)
    }

    pub fn party(&self) -> Place {
        self.places.of(self.party.0)
    }

    /// How far an actor is from the party.
    pub fn party_gap(&self, actor: Entity) -> f32 {
        self.places.gap(actor, self.party.0)
    }

    pub fn alive(&self, actor: Entity) -> bool {
        self.actors.get(actor).is_ok_and(|(vitals, ..)| !vitals.is_dead())
    }

    /// The side an actor's group or kind has it on, its own turning aside.
    pub fn side(&self, actor: Entity) -> Option<Stance> {
        self.actors.get(actor).ok().map(|(.., side, _)| side.own())
    }

    fn eyes(&self, actor: Entity) -> Option<Eyes> {
        let (vitals, attributes, attacks, ..) = self.actors.get(actor).ok()?;
        Some(Eyes::of(vitals, attributes, attacks))
    }

    /// Whether the party has a clear line to an actor within the far clip.
    pub fn in_party_view(&self, actor: Entity) -> bool {
        self.party().in_view(&self.places.of(actor), self.far(), &self.obstacles)
    }

    /// Whether an actor's eyes make out the party, which hides by its
    /// lowest able level and by Chameleon.
    pub fn sees_party(&self, actor: Entity) -> bool {
        let (party, roster) = *self.party;
        let members = self
            .actors
            .iter_many(roster.iter())
            .matched()
            .map(|(vitals, _, _, level, ..)| (level, vitals));
        let hidden = Hidden::party(members, self.laid.bonus(party).concealment);
        self.eyes(actor).is_some_and(|eyes| {
            eyes.see(&self.places.of(actor), (&self.party(), hidden), self.far(), &self.obstacles)
        })
    }

    /// Whether an actor's eyes make out a monster, which hides by its level.
    pub fn sees(&self, actor: Entity, monster: Entity) -> bool {
        let Ok((.., level, _, _)) = self.actors.get(monster) else { return false };
        let seen = (&self.places.of(monster), Hidden::alone(*level));
        self.eyes(actor)
            .is_some_and(|eyes| eyes.see(&self.places.of(actor), seen, self.far(), &self.obstacles))
    }

    /// Whether an actor sees a living foe of its own among `others`.
    pub fn sees_foe(&self, actor: Entity, others: &[Entity]) -> bool {
        let Ok((vitals, .., side, loyalty)) = self.actors.get(actor) else { return false };
        let mine = (side.now(vitals), loyalty.now(vitals));
        others.iter().any(|&other| {
            self.actors.get(other).is_ok_and(|(vitals, .., side, loyalty)| {
                let theirs = (side.now(vitals), loyalty.now(vitals));
                !vitals.is_dead()
                    && hostility::between(mine, theirs) == Stance::Hostile
                    && self.sees(actor, other)
            })
        })
    }

    /// Whether a neutral group stands with a fight (retail
    /// `MonsterGroupHasReinforcement`): one of its `members` in the party's
    /// view, within the engagement range, stands roughly between the party
    /// and one of the fight's `hostiles` in the party's view.
    pub fn reinforces(&self, members: &[Entity], hostiles: &[Entity]) -> bool {
        let place = |&actor: &Entity| (actor, self.places.of(actor));
        let members: Vec<_> =
            members.iter().filter(|&&m| self.in_party_view(m)).map(place).collect();
        let hostiles: Vec<_> = hostiles.iter().map(place).collect();
        reinforces(&self.party(), &members, &hostiles)
    }
}

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
fn reinforces(party: &Place, members: &[(Entity, Place)], hostiles: &[(Entity, Place)]) -> bool {
    members.iter().filter(|(_, member)| party.gap(member) <= ENGAGEMENT_RANGE).any(
        |&(member, at)| {
            let from_party = party.gap(&at);
            hostiles.iter().filter(|&&(other, _)| other != member).any(|(_, other)| {
                let theirs = party.gap(other);
                let between = at.feet.distance(other.feet);
                between + from_party < theirs * REINFORCEMENT_SLACK
                    && (from_party < theirs || between < theirs)
            })
        },
    )
}
