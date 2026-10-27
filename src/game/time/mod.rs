//! Simulation time.

use crate::game::camp::{self, Camp, CampNews};
use crate::game::characters::{Character, Vitals};
use crate::game::party::{
    body::{PartyBody, WALK_SPEED},
    recovery::{self, Pace},
    roster::PartyRoster,
};
use crate::game::rng::GameRng;
use crate::game::world::{identify::GroupRecognition, level::level_active};
use avian3d::prelude::LinearVelocity;
use avian3d::prelude::Position;
use bevy::prelude::*;

const RESTING_BELOW: f32 = 0.05;
const RUNNING_ABOVE: f32 = WALK_SPEED * 1.1;

const DAY_MS: u64 = 86_400_000;

/// The game clock. Retail starts a new game at noon and runs the clock at 12
/// times real time while exploring (`g_view_distance`, despite its name), so
/// a day lasts two real hours. Camping runs it at 2880 times, so eight hours
/// pass in ten real seconds.
///
/// Time passes only through [`GameClock::advance`], whose caller applies
/// what that time means for the party at once (`recovery::elapse`).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameClock {
    /// Milliseconds since midnight of the first day.
    pub ms: u64,
    /// Game nanoseconds short of the next millisecond, so that fixed steps
    /// do not lose time to rounding.
    nanos: u64,
}

impl Default for GameClock {
    fn default() -> Self {
        GameClock::at(DAY_MS / 2)
    }
}

/// Game milliseconds per real millisecond while exploring.
pub const EXPLORATION_RATE: u64 = 12;
/// Game milliseconds per real millisecond while camping.
pub const CAMPING_RATE: u64 = 2880;

/// Retail ticks stamina every 1.4 game seconds and ages everything (health,
/// conditions) every 120.
const STAMINA_TICK_MS: u64 = 1400;
pub const AGING_TICK_MS: u64 = 120_000;

/// The periodic ticks an advance of the clock crossed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ticks {
    pub stamina: u32,
    pub aging: u32,
}

impl GameClock {
    pub fn at(ms: u64) -> Self {
        GameClock { ms, nanos: 0 }
    }

    /// The time of day as 0..=255: 0 is midnight, 128 noon (retail
    /// `((ms / 1000) << 8) / 86400`).
    pub fn phase(&self) -> u8 {
        (((self.ms % DAY_MS) / 1000) * 256 / 86_400) as u8
    }

    /// Pass `real` time at `rate` game milliseconds per real one, keeping
    /// the remainder; returns the ticks crossed.
    pub fn advance_real(&mut self, real: std::time::Duration, rate: u64) -> Ticks {
        let nanos = self.nanos + real.as_nanos() as u64 * rate;
        self.nanos = nanos % 1_000_000;
        self.advance(nanos / 1_000_000)
    }

    /// Pass `ms` of game time; returns the ticks crossed.
    pub fn advance(&mut self, ms: u64) -> Ticks {
        let before = self.ms;
        self.ms += ms;
        let crossed = |period: u64| (self.ms / period - before / period) as u32;
        Ticks { stamina: crossed(STAMINA_TICK_MS), aging: crossed(AGING_TICK_MS) }
    }
}

pub struct TimePlugin;

impl Plugin for TimePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameClock>()
            .add_message::<CampNews>()
            .add_message::<camp::CampRequest>()
            .add_message::<camp::LeaveCamp>()
            .add_systems(
                Update,
                (camp::request_camp, camp::force_camp, camp::leave_camp)
                    .chain()
                    .after(crate::game::GameplaySet::Combat)
                    .run_if(level_active),
            )
            .add_systems(
                FixedPostUpdate,
                pass_time
                    .after(crate::game::party::walk_in::move_party)
                    .after(avian3d::prelude::PhysicsSystems::Writeback)
                    .run_if(level_active),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_steps_lose_no_time_and_count_ticks() {
        let mut clock = GameClock::at(0);
        // 1/64 s steps at 12x: 187.5 game ms each.
        let step = std::time::Duration::from_secs_f64(1.0 / 64.0);
        let mut ticks = Ticks::default();
        for _ in 0..64 {
            let crossed = clock.advance_real(step, EXPLORATION_RATE);
            ticks.stamina += crossed.stamina;
        }
        assert_eq!(clock.ms, 12_000);
        assert_eq!(ticks.stamina, 8);
        assert_eq!(clock.advance(AGING_TICK_MS).aging, 1);
    }
}

/// Exploring and camping: the clock runs on the fixed step and the party
/// recovers as it passes. In a fight only its rounds pass time.
pub(crate) fn pass_time(
    mut commands: Commands,
    time: Res<Time>,
    mut camp: Option<ResMut<Camp>>,
    mut news: MessageWriter<CampNews>,
    battle: Option<Single<&crate::game::combat::Battle>>,
    party: Query<(&Position, &PartyBody, &LinearVelocity)>,
    roster: Single<&PartyRoster>,
    mut affected: ParamSet<(
        Query<(&Character, &mut Vitals)>,
        GroupRecognition,
        Query<&mut crate::game::world::population::Monster>,
    )>,
    mut clock: ResMut<GameClock>,
    mut rng: ResMut<GameRng>,
    (data, spells, mut points): (
        Option<Res<crate::game::data::GameData>>,
        Res<Assets<crate::content::spells::SpellCatalog>>,
        Query<(&crate::game::magic::LearnedSpells, &mut crate::game::magic::SpellPoints)>,
    ),
) {
    let spells = data.as_ref().and_then(|d| spells.get(&d.spells));
    if battle.is_some() {
        return;
    }
    let racing = camp.as_ref().is_some_and(|c| c.racing());
    let ticks =
        clock.advance_real(time.delta(), if racing { CAMPING_RATE } else { EXPLORATION_RATE });
    // Retail tells running (fast movement) from walking; here the speed
    // does, horizontally so that falling is not running.
    let speed = party.single().map_or(0.0, |(_, _, v)| Vec2::new(v.x, v.z).length());
    let pace = if camp.is_some() {
        Pace::Camping
    } else if speed < RESTING_BELOW {
        Pace::Still
    } else if speed <= RUNNING_ABOVE {
        Pace::Walking
    } else {
        Pace::Running
    };
    // Each aging pass settles conditions before its recognition rolls. Do not
    // collapse passes across fixed steps or inspect the final conditions for all
    // of them: a blindness duration may expire partway through this interval.
    for pass in 0..=ticks.aging {
        let ticks = if pass == 0 {
            Ticks { stamina: ticks.stamina, aging: 0 }
        } else {
            Ticks { stamina: 0, aging: 1 }
        };
        let mut members = affected.p0();
        for (profile, mut vitals) in members.iter_many_unique_mut(roster.iter()).matched() {
            if recovery::has_work(profile, &vitals, ticks, pace) {
                recovery::elapse(profile, &mut vitals, ticks, pace);
            }
        }
        if pass > 0 {
            // Spell points come back at least at half rate (retail `spellScale`).
            if let Some(spells) = spells {
                let scale = crate::game::magic::recovery_scale(pace.health_scale());
                let members = affected.p0();
                for member in roster.iter() {
                    let Ok((profile, _)) = members.get(member) else { continue };
                    let Ok((learned, mut pool)) = points.get_mut(member) else { continue };
                    let maximum = crate::game::magic::max_points(profile, learned, spells);
                    if maximum.iter().any(|(realm, &max)| pool.current[realm] < max) {
                        pool.recover(&maximum, 1, scale);
                    }
                }
            }
            // Monsters' conditions run out in the world too.
            for mut monster in &mut affected.p2() {
                if crate::game::characters::any_timed(&monster.conditions) {
                    crate::game::characters::age(&mut monster.conditions, 1);
                }
            }
        }
        if pass > 0
            && let Ok((position, body, _)) = party.single()
        {
            affected.p1().detect(PartyBody::eye(position), body.forward(), None, &mut rng);
        }
    }
    let anyone_able =
        affected.p0().iter_many(roster.iter()).matched().any(|(_, vitals)| camp::can_act(vitals));
    if let Some(camp) = camp.as_mut()
        && let Some(outcome) = camp.advance(time.delta(), clock.ms, anyone_able)
    {
        news.write(outcome);
        commands.remove_resource::<Camp>();
    }
}

#[cfg(test)]
mod pass_time_tests {
    use super::*;
    use crate::game::camp::RestGoal;
    use crate::game::characters::{Condition, Gender, Lasting, Profession, Race};
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;

    /// Each update advances `step` of real time in 1/64 s fixed steps.
    fn fixed_app(step: Duration) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(step))
            .insert_resource(Time::<Fixed>::from_hz(64.0))
            .insert_resource(GameClock::at(0))
            .init_resource::<GameRng>()
            .init_resource::<Assets<crate::content::species::MonsterCatalog>>()
            .init_resource::<Assets<crate::content::spells::SpellCatalog>>()
            .init_resource::<Assets<crate::content::level::LevelDefinition>>()
            .add_message::<crate::game::world::identify::Identified>()
            .add_message::<CampNews>()
            .add_systems(FixedUpdate, pass_time);
        app.world_mut().resource_mut::<Time<Virtual>>().set_max_delta(step);
        app.update();
        app
    }

    #[test]
    fn passing_time_heals_the_party_not_every_character() {
        // A long real step: at 12x, many aging ticks of standing still.
        let step = Duration::from_millis(AGING_TICK_MS * 30 / EXPLORATION_RATE);
        let mut app = fixed_app(step);
        // One fixed step per update; 1/64 s steps would take tens of thousands.
        app.insert_resource(Time::<Fixed>::from_duration(step));
        let hurt = || {
            let c = Character::new("Hurt", Race::Human, Gender::Male, Profession::Fighter);
            let mut vitals = Vitals::new(&c);
            vitals.hp = 1;
            (c, vitals)
        };
        let member = app.world_mut().spawn(hurt()).id();
        // An adventurer the party has not recruited.
        let stranger = app.world_mut().spawn(hurt()).id();
        crate::game::party::roster::test_party(app.world_mut(), &[member]);
        for _ in 0..3 {
            app.update();
        }
        assert!(app.world().get::<Vitals>(member).unwrap().hp > 1);
        assert_eq!(app.world().get::<Vitals>(stranger).unwrap().hp, 1);
    }

    #[test]
    fn a_batch_of_fixed_steps_stops_accelerating_at_the_rest_deadline() {
        let mut app = fixed_app(Duration::from_millis(250));
        let profile = Character::new("Rester", Race::Human, Gender::Male, Profession::Fighter);
        let member = app.world_mut().spawn((Vitals::new(&profile), profile)).id();
        crate::game::party::roster::test_party(app.world_mut(), &[member]);
        app.insert_resource(Camp::Resting(RestGoal::Until(180_000)));
        app.update();
        // Four accelerated steps, then twelve exploration-rate fade steps.
        assert_eq!(app.world().resource::<GameClock>().ms, 180_000 + 2250);
        assert!(!app.world().resource::<Camp>().racing());
        for _ in 0..4 {
            app.update();
        }
        assert!(!app.world().contains_resource::<Camp>());
    }

    #[test]
    fn recovery_ends_forced_rest_in_the_same_fixed_step() {
        let mut app = fixed_app(Duration::from_millis(250));
        let profile = Character::new("Sleeper", Race::Human, Gender::Male, Profession::Fighter);
        let mut vitals = Vitals::new(&profile);
        vitals.inflict(Condition::Asleep, Lasting::Rounds(1));
        let member = app.world_mut().spawn((profile, vitals)).id();
        crate::game::party::roster::test_party(app.world_mut(), &[member]);
        app.insert_resource(Camp::Resting(RestGoal::UntilAble));
        app.update();
        // The third accelerated step crosses the aging tick and clears sleep;
        // the other thirteen steps already use the ordinary rate.
        assert_eq!(app.world().resource::<GameClock>().ms, 135_000 + 2437);
        assert!(!app.world().get::<Vitals>(member).unwrap().has(Condition::Asleep));
        assert!(matches!(
            app.world().resource::<Camp>(),
            Camp::FadingOut { outcome: CampNews::Recovered, .. }
        ));
    }
}
