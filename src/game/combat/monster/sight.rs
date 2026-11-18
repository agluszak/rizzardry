//! What a monster makes out (`docs/rules/monsters.md` sections 2 and 4):
//! who sees whom ([`Eyesight`]), and what each monster makes out, kept on
//! it as markers.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::game::actors::{CanFight, Laid, Level};
use crate::game::actors::{Eyes, Hidden, Places};
use crate::game::characters::Vitals;
use crate::game::party::roster::{Party, PartyRoster};
use crate::game::world::hostility::{Side, Stance};
use crate::game::world::level::PlayedLevel;
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::{Monster, Temper};

/// How far from the party a group still takes part (retail's 50 m).
pub const ENGAGEMENT_RANGE: f32 = 50.0;

/// How much longer the way round by a member may be than the straight line
/// from the party to a hostile.
const REINFORCEMENT_SLACK: f32 = 1.15;

/// How hard the party is to make out: by the lowest level among its members
/// who can fight, and by what Chameleon adds.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PartyHides(pub Hidden);

/// How hard the party is to make out follows its members and Chameleon.
pub fn hiding(
    party: Single<(Entity, &PartyRoster, &PartyHides), With<Party>>,
    members: Query<(&Level, Has<CanFight>)>,
    laid: Laid,
    mut commands: Commands,
) -> Result {
    let (party, roster, &was) = *party;
    let mut able = Vec::new();
    for member in roster.iter() {
        let (level, fit) = members.get(member)?;
        if fit {
            able.push(level);
        }
    }
    let now = PartyHides(Hidden::party(able, laid.bonus(party).concealment));
    if was != now {
        commands.entity(party).insert(now);
    }
    Ok(())
}

/// Who sees whom in the played level: a watcher's eyes, where both stand,
/// how hard the watched is to make out, the far clip and what stands
/// between.
#[derive(SystemParam)]
pub struct Eyesight<'w, 's> {
    eyes: Query<'w, 's, &'static Eyes>,
    levels: Query<'w, 's, &'static Level>,
    party: Single<'w, 's, (Entity, &'static PartyHides), With<Party>>,
    places: Places<'w, 's>,
    obstacles: WorldObstacles<'w, 's>,
    level: PlayedLevel<'w, 's>,
    members: Query<'w, 's, (), With<crate::game::party::roster::PartyMember>>,
    making_out: Query<'w, 's, (), With<MakesOutParty>>,
}

impl Eyesight<'_, '_> {
    /// Whether a monster makes out a body (`docs/rules/monsters.md` section
    /// 5): a member if its eyes make out the party (`MakesOutParty`), a
    /// monster if its eyes make it out, itself always.
    pub fn makes_out(&self, monster: Entity, body: Entity) -> bool {
        match self.members.contains(body) {
            true => self.making_out.contains(monster),
            false => body == monster || self.sees(monster, body),
        }
    }

    /// How far the level lets one see: its far clip.
    pub fn far(&self) -> f32 {
        self.level.view_distance().unwrap_or(0.0)
    }

    /// Whether the party has a clear line to a body within the far clip.
    pub fn in_party_view(&self, body: Entity) -> bool {
        let party = self.places.of(self.party.0);
        party.in_view(&self.places.of(body), self.far(), &self.obstacles)
    }

    /// Whether a body's eyes make out the party.
    pub fn sees_party(&self, watcher: Entity) -> bool {
        let eyes = self.eyes.get(watcher).expect("a body has eyes");
        let (party, hides) = *self.party;
        let party = (&self.places.of(party), hides.0);
        eyes.see(&self.places.of(watcher), party, self.far(), &self.obstacles)
    }

    /// Whether a body's eyes make out a monster, which hides by its level.
    pub fn sees(&self, watcher: Entity, watched: Entity) -> bool {
        let eyes = self.eyes.get(watcher).expect("a body has eyes");
        let level = self.levels.get(watched).expect("a body has a level");
        let watched = (&self.places.of(watched), Hidden::alone(*level));
        eyes.see(&self.places.of(watcher), watched, self.far(), &self.obstacles)
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
pub fn look(
    monsters: Query<(Entity, &Vitals, Has<MakesOutParty>, Has<InPartyView>), With<Monster>>,
    eyesight: Eyesight,
    mut commands: Commands,
) {
    for (monster, vitals, made_out, in_view) in &monsters {
        let alive = !vitals.is_dead();
        let now = alive && eyesight.sees_party(monster);
        mark::<MakesOutParty>(&mut commands, monster, (now, made_out));
        let now = alive && eyesight.in_party_view(monster);
        mark::<InPartyView>(&mut commands, monster, (now, in_view));
    }
}

/// Whether each monster stands with the fight follows where it and the
/// hostiles fighting stand.
#[allow(clippy::type_complexity)]
pub fn reinforce(
    monsters: Query<(Entity, &Temper, Has<InPartyView>, Has<Reinforces>), With<Monster>>,
    fighting: Query<
        (Entity, &Vitals, &Side),
        (With<InPartyView>, With<crate::game::combat::Combatant>),
    >,
    party: Single<Entity, With<Party>>,
    places: Places,
    mut commands: Commands,
) {
    let hostiles: Vec<Entity> = fighting
        .iter()
        .filter(|(_, vitals, side)| !vitals.is_dead() && side.own() == Stance::Hostile)
        .map(|(hostile, ..)| hostile)
        .collect();
    let party_gap = |body| places.gap(body, *party);
    for (monster, temper, in_view, was) in &monsters {
        let gap = party_gap(monster);
        let feet = places.of(monster).feet;
        let now = in_view
            && !temper.0.camouflaged
            && gap <= ENGAGEMENT_RANGE
            && hostiles.iter().filter(|&&h| h != monster).any(|&hostile| {
                let theirs = party_gap(hostile);
                let between = feet.distance(places.of(hostile).feet);
                between + gap < theirs * REINFORCEMENT_SLACK && (gap < theirs || between < theirs)
            });
        mark::<Reinforces>(&mut commands, monster, (now, was));
    }
}
