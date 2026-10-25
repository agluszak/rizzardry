//! Searching: hidden items come to light when a member spots them (retail
//! `Search.cpp`). Every half second the items in view are matched with the
//! party's best searcher. Rangers always search; everyone else only in
//! search mode, which a fight suspends.

use avian3d::prelude::{RigidBody, SpatialQuery, SpatialQueryFilter};
use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;

use crate::content::characters::{Attribute, Skill};
use crate::content::items::ItemCatalog;
use crate::game::camp::can_act;
use crate::game::characters::{Character, Profession};
use crate::game::combat::Battle;
use crate::game::data::GameData;
use avian3d::prelude::Position;

use crate::game::party::body::{PartyBody, Run};
use crate::game::party::roster::PartyRoster;
use crate::game::world::items::WorldItem;

/// Searchables farther than this from the eye are not considered (10000
/// units).
const SEARCH_RADIUS: f32 = 20.0;
/// A searcher with score `s` spots things within `s` × this (retail
/// `score × 10000 × 0.01` units).
const RANGE_PER_POINT: f32 = 0.2;
const PULSE_SECONDS: f32 = 0.5;

/// The party's search mode (retail `search_mode`).
#[derive(Resource, Debug, Default)]
pub struct Search {
    pub on: bool,
    /// On before the fight that suspended it (`search_mode_saved`).
    suspended: bool,
    /// Seconds until the next sweep.
    pulse: f32,
}

/// Retail's S key (`mgskeyboard.ini` command 312).
#[derive(InputAction)]
#[action_output(bool)]
pub struct SearchKey;

pub fn search_key(
    key: Single<&ActionEvents, With<Action<SearchKey>>>,
    mut toggle: MessageWriter<ToggleSearch>,
) {
    if key.contains(ActionEvents::START) {
        toggle.write(ToggleSearch);
    }
}

/// The search button or key (retail `ToggleSearchMode`).
#[derive(Message, Debug, Clone, Copy)]
pub struct ToggleSearch;

#[derive(Message, Debug, Clone, PartialEq)]
pub enum SearchNews {
    On,
    Off,
    NotInCombat,
    /// "Ragnar spots a hidden Dagger!"
    Spotted {
        member: String,
        item: String,
    },
}

pub fn toggle_search(
    mut requests: MessageReader<ToggleSearch>,
    battle: Option<Res<Battle>>,
    mut search: ResMut<Search>,
    mut news: MessageWriter<SearchNews>,
) {
    // A fight switches search mode off, and the end of the fight back on
    // (retail `InitializeCombat`, `EndCombat`).
    match (battle.is_some(), search.on, search.suspended) {
        (true, true, _) => {
            *search = Search { on: false, suspended: true, pulse: 0.0 };
            news.write(SearchNews::Off);
        }
        (false, false, true) => {
            *search = Search { on: true, suspended: false, pulse: PULSE_SECONDS };
            news.write(SearchNews::On);
        }
        _ => {}
    }
    for ToggleSearch in requests.read() {
        if search.on {
            search.on = false;
            news.write(SearchNews::Off);
        } else if battle.is_some() {
            news.write(SearchNews::NotInCombat);
        } else {
            *search = Search { on: true, suspended: false, pulse: PULSE_SECONDS };
            news.write(SearchNews::On);
        }
    }
}

/// How well a member searches (retail `PickBestSearcher`): half their
/// Scouting (a quarter while running) plus a bonus rising with Senses, at
/// most 100. Without search mode only rangers search; while running, those
/// without Scouting score nothing.
pub fn search_score(member: &Character, search_mode: bool, running: bool) -> u32 {
    if member.profession != Profession::Ranger && !search_mode {
        return 0;
    }
    let level = member.skill(Skill::Scouting);
    let base = if running { level / 4 } else { level / 2 };
    if base == 0 && !(search_mode && !running) {
        return 0;
    }
    let senses = member.attribute(Attribute::Senses);
    let score = match senses {
        0..=50 => base + senses / 5,
        51..=80 => base + 10 + (senses - 50) / 3,
        81..=90 => (base + senses).saturating_sub(60),
        91..=99 => (base + senses * 2).saturating_sub(150),
        100 => base + 50,
        _ => base + 50 + (senses * 10 - 1000) / 25,
    };
    score.min(100)
}

/// The half-second sweep (retail `RunSearchPulse`): each hidden item
/// within the search radius, in front of the eye and in its sight, is
/// spotted by the best searcher if it lies within their range.
pub fn search_pulse(
    time: Res<Time>,
    mut search: ResMut<Search>,
    battle: Option<Res<Battle>>,
    run: Single<&Action<Run>>,
    party: Query<(&Position, &PartyBody)>,
    roster: Res<PartyRoster>,
    members: Query<&Character>,
    mut items: Query<(&mut WorldItem, &GlobalTransform)>,
    (spatial, bodies): (SpatialQuery, Query<&RigidBody>),
    (data, catalogs): (Option<Res<GameData>>, Res<Assets<ItemCatalog>>),
    mut news: MessageWriter<SearchNews>,
) {
    search.pulse -= time.delta_secs();
    if search.pulse > 0.0 {
        return;
    }
    search.pulse = PULSE_SECONDS;
    if battle.is_some() {
        return;
    }
    let Ok((position, body)) = party.single() else { return };
    let (from, forward) = (PartyBody::eye(position), body.forward());
    let running = ***run;
    let best = roster
        .0
        .iter()
        .filter_map(|&m| members.get(m).ok())
        .filter(|m| can_act(m))
        .map(|m| (search_score(m, search.on, running), m))
        .filter(|&(score, _)| score > 0)
        // The first of equals, as retail keeps the earlier slot.
        .reduce(|a, b| if b.0 > a.0 { b } else { a });
    let Some((score, searcher)) = best else { return };
    let solid = |entity| bodies.get(entity).is_ok_and(RigidBody::is_static);
    let catalog = data.and_then(|d| catalogs.get(&d.items));
    for (mut item, at) in &mut items {
        if !item.hidden {
            continue;
        }
        let to = at.translation();
        let distance = from.distance(to);
        let ahead = (to - from).with_y(0.0).dot(forward) > 0.0;
        if distance >= SEARCH_RADIUS || distance >= score as f32 * RANGE_PER_POINT || !ahead {
            continue;
        }
        let Ok(direction) = Dir3::new(to - from) else { continue };
        let filter = SpatialQueryFilter::default();
        if spatial.cast_ray_predicate(from, direction, distance, true, &filter, &solid).is_some() {
            continue;
        }
        item.hidden = false;
        let name =
            catalog.and_then(|c| c.get(item.item.item)).map(|d| item.item.display_name(d, false));
        news.write(SearchNews::Spotted {
            member: searcher.name.clone(),
            item: name.unwrap_or_default(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::characters::{Gender, Race};

    #[test]
    fn rangers_search_on_their_own_and_others_only_in_search_mode() {
        let mut ranger = Character::new("Scout", Race::Elf, Gender::Female, Profession::Ranger);
        let fighter = Character::new("Brute", Race::Human, Gender::Male, Profession::Fighter);
        ranger.skill_points[Skill::Scouting] = 20;
        ranger.attributes[Attribute::Senses] = 60;
        // Half of Scouting 20 (plus the ranger's bonus, if Scouting is it)
        // and Senses 60's bonus of 10 + 10 / 3.
        let base = ranger.skill(Skill::Scouting) / 2;
        assert_eq!(search_score(&ranger, false, false), base + 13);
        assert_eq!(search_score(&fighter, false, false), 0);
        // In search mode anyone searches by Senses alone, but not running.
        assert!(search_score(&fighter, true, false) > 0);
        assert_eq!(search_score(&fighter, true, true), 0);
    }
}
