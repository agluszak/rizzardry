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
    freedom: crate::game::screen::Freedom,
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
    content: crate::game::data::Content,
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
    for (member, _) in members.iter_many(roster.iter()).matched().filter(|(_, v)| v.can_act()) {
        let score = search_score(member, search.on(battle.is_some()), running);
        if score > best.map_or(0, |b| b.0) {
            best = Some((score, member));
        }
        if enchantments.0.get(crate::content::spells::SpellId::DETECT_SECRETS).is_some()
            && !(best.is_some() && rng.below(2) == 0)
        {
            best = Some((100, member));
        }
    }
    let Some((score, searcher)) = best else { return };
    let catalog = content.items();
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
            catalog.and_then(|c| c.get(item.appearance())).map(|d| item.display_name(d, false));
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
    fn search_intent_survives_a_fight_which_holds_it_off() {
        use crate::game::screen::Screen;
        use crate::game::world::level::Level;
        let mut app = App::new();
        app.insert_resource(State::new(Screen::World))
            .init_resource::<NextState<Screen>>()
            .init_resource::<Search>()
            .add_message::<ToggleSearch>()
            .add_message::<SearchNews>()
            .add_observer(suspend_search)
            .add_observer(resume_search)
            .add_systems(Update, toggle_search);
        app.world_mut().spawn(Level::test(default()));
        let news = |app: &mut App| {
            app.world_mut().resource_mut::<Messages<SearchNews>>().drain().collect::<Vec<_>>()
        };
        let on = |app: &App, fighting| app.world().resource::<Search>().on(fighting);
        app.world_mut().write_message(ToggleSearch);
        app.update();
        assert!(on(&app, false));
        assert_eq!(news(&mut app), [SearchNews::On]);
        // A fight holds searching off, and remembers it was asked for.
        let battle = app.world_mut().spawn(Battle::default()).id();
        app.update();
        assert!(!on(&app, true));
        assert_eq!(news(&mut app), [SearchNews::Off]);
        app.world_mut().write_message(ToggleSearch);
        app.update();
        assert_eq!(news(&mut app), [SearchNews::NotInCombat], "no toggling in a fight");
        app.world_mut().despawn(battle);
        app.update();
        assert!(on(&app, false));
        assert_eq!(news(&mut app), [SearchNews::On]);
        app.world_mut().resource_mut::<NextState<Screen>>().set(Screen::Inventory);
        app.world_mut().write_message(ToggleSearch);
        app.update();
        assert!(on(&app, false), "a held party cannot toggle search");
        *app.world_mut().resource_mut::<NextState<Screen>>() = NextState::Unchanged;
        app.world_mut().write_message(ToggleSearch);
        app.update();
        assert!(!on(&app, false));
        assert_eq!(news(&mut app), [SearchNews::Off]);
        // Off, a fight neither announces nor resumes anything.
        let battle = app.world_mut().spawn(Battle::default()).id();
        app.update();
        app.world_mut().despawn(battle);
        app.update();
        assert!(!on(&app, false));
        assert!(news(&mut app).is_empty());
    }

    /// The same game time sweeps as often, and draws the same chances,
    /// whether a frame runs one fixed step or many.
    #[test]
    fn sweeps_follow_game_time_not_frames() {
        use crate::content::items::{ItemDefinition, ItemId};
        use crate::game::inventory::{ItemIds, ItemStack};
        use crate::game::magic::enchantments::{Enchantment, PartyEffects};
        use crate::game::rng::GameRng;
        use std::time::Duration;
        let run = |frame: Duration| {
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(frame))
                .insert_resource(Time::<Fixed>::from_duration(Duration::from_millis(125)))
                .init_resource::<avian3d::collider_tree::ColliderTrees>()
                .insert_resource(Search { requested: true, ..default() })
                .insert_resource(GameRng::seeded(5))
                .add_message::<SearchNews>()
                .add_systems(FixedUpdate, search_pulse);
            app.world_mut().resource_mut::<Time<Virtual>>().set_max_delta(Duration::from_secs(10));
            let world = app.world_mut();
            let party = crate::game::party::roster::test_party(world, &[]);
            world.entity_mut(party).insert((PartyBody::default(), Position(Vec3::ZERO)));
            world.get_mut::<PartyEffects>(party).unwrap().0.apply(Enchantment {
                spell: crate::content::spells::SpellId::DETECT_SECRETS,
                power: 5,
                boost: 0,
                turns: 100,
                absorption: 0,
            });
            let profile = Character::new("Scout", Race::Human, Gender::Male, Profession::Fighter);
            let member = world.spawn((Vitals::new(&profile), profile)).id();
            crate::game::party::roster::test_join(world, member);
            let definition = ItemDefinition { id: ItemId(1), ..default() };
            let stack = ItemStack::new(&definition, &mut ItemIds::default(), 1, true);
            let ahead =
                PartyBody::eye(&Position(Vec3::ZERO)) + PartyBody::default().forward() * 5.0;
            let item = world
                .spawn((WorldItem::loose(stack, true), Transform::from_translation(ahead)))
                .id();
            while app.world().resource::<Time<Virtual>>().elapsed() < Duration::from_secs(2) {
                app.update();
            }
            let hidden = app.world().get::<WorldItem>(item).unwrap().hidden;
            (app.world().resource::<GameRng>().clone(), hidden)
        };
        // Two seconds are four sweeps, each drawing the spell's chance.
        let mut expected = GameRng::seeded(5);
        for _ in 0..4 {
            expected.below(2);
        }
        for frame in [Duration::from_millis(125), Duration::from_secs(2)] {
            assert_eq!(run(frame), (expected.clone(), false), "frames of {frame:?}");
        }
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
