//! Searching: hidden items come to light when a member spots them (retail
//! `Search.cpp`). Every half second the items in view are matched with the
//! party's best searcher. Rangers always search; everyone else only in
//! search mode, which a fight suspends.

use crate::game::world::obstacles::WorldObstacles;
use bevy::prelude::*;

use crate::content::characters::{Attribute, Skill};
use crate::game::characters::{Character, Profession, Vitals};
use crate::game::combat::Battle;
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

/// The party's search mode (retail `search_mode`): whether the player asked
/// to search. A fight holds it in abeyance without forgetting it.
#[derive(Resource, Debug)]
pub struct Search {
    requested: bool,
    /// The sweeps, every half second.
    pulse: Timer,
}

impl Search {
    /// Searching is in effect: asked for, and no fight going on.
    pub fn on(&self, fighting: bool) -> bool {
        self.requested && !fighting
    }

    fn restart_pulse(&mut self) {
        self.pulse.reset();
    }
}

impl Default for Search {
    fn default() -> Self {
        Search { requested: false, pulse: Timer::from_seconds(PULSE_SECONDS, TimerMode::Repeating) }
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

/// A fight switches searching off (retail `InitializeCombat`)...
pub fn suspend_search(
    _: On<Add<Battle>>,
    mut search: ResMut<Search>,
    mut news: MessageWriter<SearchNews>,
) {
    if search.requested {
        search.restart_pulse();
        news.write(SearchNews::Off);
    }
}

/// ...and its end back on, if it was asked for (retail `EndCombat`).
pub fn resume_search(
    _: On<Remove<Battle>>,
    mut search: ResMut<Search>,
    mut news: MessageWriter<SearchNews>,
) {
    if search.requested {
        search.restart_pulse();
        news.write(SearchNews::On);
    }
}

pub fn toggle_search(
    mut requests: MessageReader<ToggleSearch>,
    freedom: crate::game::freedom::Freedom,
    battle: Option<Single<&Battle>>,
    mut search: ResMut<Search>,
    mut news: MessageWriter<SearchNews>,
) {
    for ToggleSearch in requests.read() {
        if !freedom.free() {
            continue;
        }
        if battle.is_some() {
            news.write(SearchNews::NotInCombat);
        } else if search.requested {
            search.requested = false;
            news.write(SearchNews::Off);
        } else {
            search.requested = true;
            search.restart_pulse();
            news.write(SearchNews::On);
        }
    }
}

/// How well a member searches (retail `PickBestSearcher`): half their
/// Scouting (a quarter while running) plus a bonus rising with Senses, at
/// most 100. Without search mode only rangers search; while running, those
/// without Scouting score nothing.
pub fn search_score(
    member: &crate::game::characters::CharacterStats<'_>,
    search_mode: bool,
    running: bool,
) -> u32 {
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
        81..=90 => base + senses - 60,
        91..=99 => base + senses * 2 - 150,
        100 => base + 50,
        _ => base + 50 + (senses * 10 - 1000) / 25,
    };
    score.min(100)
}

/// The half-second sweep (retail `RunSearchPulse`), on the fixed gameplay
/// step: each hidden item within the search radius, in front of the eye and
/// in its sight, is spotted by the best searcher if it lies within their
/// range.
pub fn search_pulse(
    time: Res<Time>,
    mut search: ResMut<Search>,
    enchantments: Single<&crate::game::magic::enchantments::PartyEffects>,
    mut rng: ResMut<crate::game::rng::GameRng>,
    battle: Option<Single<&Battle>>,
    movement: Single<&PartyMovement>,
    party: Query<(&Position, &PartyBody)>,
    roster: Single<&PartyRoster>,
    members: Query<(&Character, &Vitals)>,
    mut items: Query<(Entity, &mut WorldItem, &Transform)>,
    obstacles: WorldObstacles,
    content: If<Res<crate::game::data::Catalogs>>,
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
    let mut best: Option<(u32, &Character)> = None;
    for (member, vitals) in members.iter_many(roster.iter()).matched().filter(|(_, v)| v.can_act())
    {
        let stats = member.stats(&vitals.enchantments);
        let score = search_score(&stats, search.on(battle.is_some()), running);
        if score > best.map_or(0, |b| b.0) {
            best = Some((score, member));
        }
        if enchantments.0.has(crate::content::spells::SpellId::DETECT_SECRETS)
            && !(best.is_some() && rng.below(2) == 0)
        {
            best = Some((100, member));
        }
    }
    let Some((score, searcher)) = best else { return };
    let catalog = &content.items;
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
        if !obstacles.clear(from, to) {
            continue;
        }
        item.hidden = false;
        news.write(SearchNews::Spotted {
            target,
            member: searcher.name.clone(),
            item: item.display_name(&catalog[item.appearance()], false),
        });
    }
}
