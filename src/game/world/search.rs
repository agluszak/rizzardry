//! Searching: hidden items come to light when a member spots them (retail
//! `Search.cpp`). Every half second the items in view are matched with the
//! party's best searcher. Rangers always search; everyone else only in
//! search mode, which a fight suspends.

use crate::game::world::obstacles::WorldObstacles;
use bevy::prelude::*;

use crate::content::characters::{Attribute, Skill};
use crate::content::items::ItemCatalog;
use crate::game::camp::can_act;
use crate::game::characters::{Character, Profession, Vitals};
use crate::game::combat::Battle;
use crate::game::data::GameData;
use avian3d::prelude::Position;

use crate::game::party::body::{PartyBody, PartyMovement};
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
#[derive(Resource, Debug)]
pub struct Search {
    mode: SearchMode,
    /// The sweeps, every half second.
    pulse: Timer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchMode {
    Off,
    On,
    /// Resume after the fight; cannot also be on.
    Suspended,
}

impl Search {
    fn new(mode: SearchMode) -> Self {
        Search { mode, pulse: Timer::from_seconds(PULSE_SECONDS, TimerMode::Repeating) }
    }

    pub fn on(&self) -> bool {
        self.mode == SearchMode::On
    }
}

impl Default for Search {
    fn default() -> Self {
        Search::new(SearchMode::Off)
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
        target: Entity,
        member: String,
        item: String,
    },
}

pub fn toggle_search(
    mut requests: MessageReader<ToggleSearch>,
    freedom: crate::game::screen::Freedom,
    battle: Option<Res<Battle>>,
    mut search: ResMut<Search>,
    mut news: MessageWriter<SearchNews>,
) {
    // A fight switches search mode off, and the end of the fight back on
    // (retail `InitializeCombat`, `EndCombat`).
    match (battle.is_some(), search.mode) {
        (true, SearchMode::On) => {
            *search = Search::new(SearchMode::Suspended);
            news.write(SearchNews::Off);
        }
        (false, SearchMode::Suspended) => {
            *search = Search::new(SearchMode::On);
            news.write(SearchNews::On);
        }
        _ => {}
    }
    for ToggleSearch in requests.read() {
        if !freedom.free() {
            continue;
        }
        if search.on() {
            search.mode = SearchMode::Off;
            news.write(SearchNews::Off);
        } else if battle.is_some() {
            news.write(SearchNews::NotInCombat);
        } else {
            *search = Search::new(SearchMode::On);
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
    movement: Res<PartyMovement>,
    party: Query<(&Position, &PartyBody)>,
    roster: Res<PartyRoster>,
    members: Query<(&Character, &Vitals)>,
    mut items: Query<(Entity, &mut WorldItem, &Transform)>,
    obstacles: WorldObstacles,
    (data, catalogs): (Option<Res<GameData>>, Res<Assets<ItemCatalog>>),
    mut news: MessageWriter<SearchNews>,
) {
    if !search.pulse.tick(time.delta()).just_finished() {
        return;
    }
    if battle.is_some() {
        return;
    }
    let Ok((position, body)) = party.single() else { return };
    let (from, forward) = (PartyBody::eye(position), body.forward());
    let running = movement.running;
    let best = members
        .iter_many(&roster.0)
        .matched()
        .filter(|(_, vitals)| can_act(vitals))
        .map(|(m, _)| (search_score(m, search.on(), running), m))
        .filter(|&(score, _)| score > 0)
        // The first of equals, as retail keeps the earlier slot.
        .reduce(|a, b| if b.0 > a.0 { b } else { a });
    let Some((score, searcher)) = best else { return };
    let catalog = data.and_then(|d| catalogs.get(&d.items));
    for (target, mut item, at) in &mut items {
        if !item.hidden {
            continue;
        }
        let to = at.translation;
        let distance = from.distance(to);
        let ahead = (to - from).with_y(0.0).dot(forward) > 0.0;
        if distance >= SEARCH_RADIUS || distance >= score as f32 * RANGE_PER_POINT || !ahead {
            continue;
        }
        let Ok(direction) = Dir3::new(to - from) else { continue };
        if obstacles.first_hit(from, direction, distance).is_some() {
            continue;
        }
        item.hidden = false;
        let name =
            catalog.and_then(|c| c.get(item.item.item)).map(|d| item.item.display_name(d, false));
        news.write(SearchNews::Spotted {
            target,
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
    fn search_requests_work_without_input_and_resume_after_combat() {
        use crate::game::screen::Screen;
        use crate::game::world::level::Level;
        let mut app = App::new();
        app.insert_resource(State::new(Screen::World))
            .init_resource::<NextState<Screen>>()
            .init_resource::<Search>()
            .add_message::<ToggleSearch>()
            .add_message::<SearchNews>()
            .add_systems(Update, toggle_search);
        app.world_mut().spawn(Level {
            path: "test".into(),
            geometry: default(),
            navmeshes: vec![],
            script: default(),
            messages: None,
        });
        app.world_mut().write_message(ToggleSearch);
        app.update();
        assert!(app.world().resource::<Search>().on());
        app.insert_resource(Battle::default());
        app.update();
        assert!(!app.world().resource::<Search>().on());
        app.world_mut().write_message(ToggleSearch);
        app.update();
        assert!(!app.world().resource::<Search>().on());
        app.world_mut().remove_resource::<Battle>();
        app.update();
        assert!(app.world().resource::<Search>().on());
        app.world_mut().resource_mut::<NextState<Screen>>().set(Screen::Inventory);
        app.world_mut().write_message(ToggleSearch);
        app.update();
        assert!(app.world().resource::<Search>().on(), "a held party cannot toggle search");
        *app.world_mut().resource_mut::<NextState<Screen>>() = NextState::Unchanged;
        app.world_mut().write_message(ToggleSearch);
        app.update();
        assert!(!app.world().resource::<Search>().on());
    }

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
