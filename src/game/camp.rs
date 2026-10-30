//! Camping: the party rests eight game hours with time racing, or is forced
//! to rest while nobody can act until someone can. Retail's sequence
//! (`BeginPartyCamping`, `UpdateSurpriseMode`, `EndSurprise`) fades into the
//! camping screen and back; the fades are the presentation's, and the rest
//! runs from when it is accepted until it ends. Ambushes and the waking
//! Senses roll wait for combat.

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;

use crate::game::characters::Vitals;
use crate::game::combat::Battle;
use crate::game::party::body::PartyBody;
use crate::game::party::roster::PartyRoster;
use crate::game::time::GameClock;
use crate::game::world::level::level_active;

/// How long a requested rest lasts, in game milliseconds.
const REST_MS: u64 = 8 * crate::game::time::HOUR_MS;
/// Faster than this, in metres a second, the party may not camp.
const STILL_BELOW: f32 = 0.05;

/// The party is camping, with game time racing. Exists only while it does.
#[derive(Resource, Clone, Copy, Debug)]
pub enum Camp {
    /// A requested rest, until this game time.
    Until(u64),
    /// A forced rest, until someone can act.
    UntilAble,
}

impl Camp {
    /// How the rest ends, once this step's time and recovery have passed.
    pub(crate) fn outcome(self, now: u64, anyone_able: bool) -> Option<CampNews> {
        match self {
            Self::Until(until) if now >= until => Some(CampNews::Done),
            Self::UntilAble if anyone_able => Some(CampNews::Recovered),
            _ => None,
        }
    }
}

/// What camping tells the player.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CampNews {
    Started {
        forced: bool,
    },
    /// "You may not camp while moving!"
    Moving,
    /// "Not while in combat!"
    InCombat,
    Done,
    Interrupted,
    Recovered,
}

pub struct CampPlugin;

impl Plugin for CampPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<CampNews>()
            .add_message::<CampRequest>()
            .add_message::<LeaveCamp>()
            .add_systems(
                Update,
                (request_camp, force_camp, leave_camp)
                    .chain()
                    .after(crate::game::GameplaySet::Combat)
                    .run_if(level_active),
            )
            .add_systems(
                FixedPostUpdate,
                advance_camp.after(crate::game::time::pass_time).run_if(level_active),
            );
    }
}

/// End the rest once this step's time and recovery have passed.
pub(crate) fn advance_camp(
    mut commands: Commands,
    clock: Res<GameClock>,
    camp: Option<Res<Camp>>,
    battle: Option<Single<&Battle>>,
    roster: Single<&PartyRoster>,
    members: Query<&Vitals>,
    mut news: MessageWriter<CampNews>,
) {
    let Some(camp) = camp else { return };
    if battle.is_some() {
        return;
    }
    let anyone_able = members.iter_many(roster.iter()).matched().any(Vitals::can_act);
    if let Some(outcome) = camp.outcome(clock.ms, anyone_able) {
        news.write(outcome);
        commands.remove_resource::<Camp>();
    }
}

/// The Camp button (retail `RequestCamp`, as the key does).
#[derive(Message, Debug, Clone, Copy)]
pub struct CampRequest;

pub fn request_camp(
    mut commands: Commands,
    clock: Res<GameClock>,
    freedom: crate::game::freedom::Freedom,
    mut button: MessageReader<CampRequest>,
    battle: Option<Single<&Battle>>,
    party: Query<&LinearVelocity, With<PartyBody>>,
    mut news: MessageWriter<CampNews>,
) {
    let asked = button.read().count() > 0;
    if !asked || !freedom.free() {
        return;
    }
    if battle.is_some() {
        news.write(CampNews::InCombat);
        return;
    }
    if !party.single().is_ok_and(|v| v.length() < STILL_BELOW) {
        news.write(CampNews::Moving);
        return;
    }
    commands.insert_resource(Camp::Until(clock.ms + REST_MS));
    news.write(CampNews::Started { forced: false });
}

/// When the living can no longer act, the party must camp.
pub fn force_camp(
    mut commands: Commands,
    camp: Option<Res<Camp>>,
    battle: Option<Single<&Battle>>,
    roster: Single<&PartyRoster>,
    members: Query<&Vitals>,
    mut news: MessageWriter<CampNews>,
) {
    let anyone_alive =
        crate::game::characters::anyone_alive(members.iter_many(roster.iter()).matched());
    let anyone_able = members.iter_many(roster.iter()).matched().any(Vitals::can_act);
    if camp.is_some() || battle.is_some() || !anyone_alive || anyone_able {
        return;
    }
    commands.insert_resource(Camp::UntilAble);
    news.write(CampNews::Started { forced: true });
}

/// The player asks to stop camping (Escape on the camping screen).
#[derive(Message, Debug, Clone, Copy)]
pub struct LeaveCamp;

/// A forced camp cannot be left; a rest is cut short.
pub fn leave_camp(
    mut commands: Commands,
    mut requests: MessageReader<LeaveCamp>,
    camp: Option<Res<Camp>>,
    mut news: MessageWriter<CampNews>,
) {
    let (Some(camp), Some(LeaveCamp)) = (camp, requests.read().last()) else { return };
    match *camp {
        Camp::UntilAble => {
            news.write(CampNews::Started { forced: true });
        }
        Camp::Until(_) => {
            news.write(CampNews::Interrupted);
            commands.remove_resource::<Camp>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rest_ends_at_its_deadline_and_a_forced_one_on_recovery() {
        assert_eq!(Camp::Until(1000).outcome(999, true), None);
        assert_eq!(Camp::Until(1000).outcome(1000, false), Some(CampNews::Done));
        assert_eq!(Camp::UntilAble.outcome(u64::MAX, false), None);
        assert_eq!(Camp::UntilAble.outcome(0, true), Some(CampNews::Recovered));
    }

    #[test]
    fn leaving_cuts_a_rest_short_at_once_but_not_a_forced_one() {
        let mut app = App::new();
        app.add_message::<LeaveCamp>()
            .add_message::<CampNews>()
            .add_systems(Update, leave_camp)
            .insert_resource(Camp::UntilAble);
        app.world_mut().write_message(LeaveCamp);
        app.update();
        assert!(app.world().contains_resource::<Camp>());
        app.insert_resource(Camp::Until(1000));
        app.world_mut().write_message(LeaveCamp);
        app.update();
        assert!(!app.world().contains_resource::<Camp>(), "no fade holds the rest");
        let news: Vec<_> = app.world_mut().resource_mut::<Messages<CampNews>>().drain().collect();
        assert_eq!(news, [CampNews::Started { forced: true }, CampNews::Interrupted]);
    }
}
