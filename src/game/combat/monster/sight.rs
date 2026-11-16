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
use crate::game::world::population::{Monster, Temper};

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
    fn in_party_view(&self, body: Entity) -> bool {
        let party = self.places.of(self.party());
        party.in_view(&self.places.of(body), self.far(), &self.obstacles)
    }

    /// Whether a body's eyes make out the party, which hides by its lowest
    /// able level and by Chameleon.
    fn sees_party(&self, watcher: Entity) -> bool {
        let eyes = self.eyes.get(watcher).expect("a body has eyes");
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
        let eyes = self.eyes.get(watcher).expect("a body has eyes");
        let (level, _) = self.hiding.get(watched).expect("a body has a level");
        eyes.see(
            &self.places.of(watcher),
            (&self.places.of(watched), Hidden::alone(*level)),
            self.far(),
            &self.obstacles,
        )
    }
}

/// Its eyes make out the party (section 2).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct MakesOutParty;

/// The party has a clear line to it within the far clip.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct InPartyView;

/// It stands with the fight (retail `MonsterGroupHasReinforcement`): in the
/// party's view, not of a kind that keeps hidden and within 50 m of the
/// party, it stands between the party and a hostile fighting in the party's
/// view: the way from the party by it to the hostile is under 1.15 times the
/// hostile's own gap, and it is nearer the party, or nearer the hostile,
/// than the hostile is to the party.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Reinforces;

/// Puts a marker on or takes it off, as it now holds or not.
fn mark<M: Component + Default>(commands: &mut Commands, body: Entity, (now, was): (bool, bool)) {
    match (now, was) {
        (true, false) => {
            commands.entity(body).insert(M::default());
        }
        (false, true) => {
            commands.entity(body).remove::<M>();
        }
        _ => {}
    }
}

/// What each living monster makes out of the party, and whether the party
/// has it in view, follow where they stand.
pub(crate) fn look(
    monsters: Query<(Entity, Has<MakesOutParty>, Has<InPartyView>), With<Monster>>,
    sight: Sight,
    mut commands: Commands,
) {
    for (monster, made_out, in_view) in &monsters {
        let alive = sight.alive(monster);
        let now = alive && sight.sees_party(monster);
        mark::<MakesOutParty>(&mut commands, monster, (now, made_out));
        let now = alive && sight.in_party_view(monster);
        mark::<InPartyView>(&mut commands, monster, (now, in_view));
    }
}

/// Whether each monster stands with the fight follows where it and the
/// hostiles fighting stand.
#[allow(clippy::type_complexity)]
pub(crate) fn reinforce(
    monsters: Query<(Entity, Has<InPartyView>, Has<Reinforces>), With<Monster>>,
    fighting: Query<Entity, (With<InPartyView>, With<crate::game::combat::Combatant>)>,
    sight: Sight,
    mut commands: Commands,
) {
    let hostiles: Vec<Entity> = fighting
        .iter()
        .filter(|&h| sight.alive(h) && sight.side(h) == Some(Stance::Hostile))
        .collect();
    for (monster, in_view, was) in &monsters {
        let hidden = sight.tempers.get(monster).is_ok_and(|temper| temper.0.camouflaged);
        let gap = sight.party_gap(monster);
        let feet = sight.places.of(monster).feet;
        let now = in_view
            && !hidden
            && gap <= ENGAGEMENT_RANGE
            && hostiles.iter().filter(|&&h| h != monster).any(|&hostile| {
                let theirs = sight.party_gap(hostile);
                let between = feet.distance(sight.places.of(hostile).feet);
                between + gap < theirs * REINFORCEMENT_SLACK && (gap < theirs || between < theirs)
            });
        mark::<Reinforces>(&mut commands, monster, (now, was));
    }
}
