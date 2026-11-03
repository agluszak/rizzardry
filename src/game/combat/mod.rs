//! Combat requests, encounter lifetime and round execution. Accepted actions
//! and round history live on the actors ([`turn`]); attacks and magic settle
//! their consequences before the scheduler advances to the next actor.

pub mod ai;
pub mod approach;
pub mod attack;
pub mod breath;
mod conduct;
mod engage;
mod fighters;
mod monster_spells;
mod orders;
mod round_end;
mod sight;
pub mod summon;
mod swings;
pub mod turn;

#[cfg(test)]
mod tests;

use crate::game::GameplaySet;
use crate::game::data::Catalogs;
pub use conduct::Interrupt;
pub use orders::*;
pub use round_end::{RoundEnding, end_round, judge_round};
use sight::*;
pub use turn::{Action, Ordered, Turn};

use crate::game::world::level::LevelOwned;
use crate::game::world::level::PlayedLevel;
use crate::game::world::obstacles::WorldObstacles;
use bevy::prelude::*;

use crate::content::items::AttackMode;
use crate::content::species::AttackWord;

use crate::content::species::{Reach, Species};
use crate::game::characters::{Character, Dead, Vitals};
use crate::game::hits::Effect;
use crate::game::magic::casting::{self};
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::time::GameClock;
use crate::game::world::hostility::{self, Side, Stance};
use crate::game::world::identify::{GroupMember, GroupMembers, GroupRecognition, MonsterGroup};
use crate::game::world::population::Monster;
use bevy::ecs::relationship::RelationshipTarget;

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        swings::stages(app);
        app.add_systems(
            Settlement,
            (
                crate::game::hits::run_hits,
                crate::game::characters::cheat_death,
                crate::game::characters::settle_deaths,
                credit_deaths,
                settle_defeat,
                hostility::judge_sides,
            )
                .chain(),
        );
        app.add_observer(fighters::leave).add_observer(join_party);
        app.add_observer(turn::clear_fight)
            .add_message::<CombatNews>()
            .add_message::<hostility::Provocation>()
            .add_message::<CyclePartyMove>()
            .add_message::<StartRound>()
            .add_message::<summon::Summon>()
            .add_message::<CombatControl>()
            .add_message::<crate::game::camp::CampNews>()
            .add_message::<GiveOrder>()
            .add_message::<engage::Engaged>()
            .init_resource::<CombatSettings>()
            // Every body's side is judged at the start of the frame, before
            // magic is accepted and cast, and again on each fight step.
            .add_systems(
                Update,
                hostility::judge_sides
                    .after(GameplaySet::Burden)
                    .before(crate::game::magic::accept_magic),
            )
            // Where everyone stands and how they see is placed before
            // gameplay reads it, each frame and each fixed step.
            .add_systems(
                Update,
                crate::game::world::footing::place_bodies.before(GameplaySet::LevelActivation),
            )
            .add_observer(approach::end_move)
            .add_observer(approach::stop_walking)
            .add_systems(
                Update,
                (
                    give_orders,
                    // Defeat is settled before a fight starts,
                    settle_defeat,
                    // provocations and alerts, then engagement, then the opening,
                    hostility::take_offence,
                    engage::alert_factions.run_if(any_with_component::<Battle>),
                    engage::engage,
                    engage::open_battle,
                    control_combat,
                )
                    .chain()
                    .in_set(GameplaySet::Combat)
                    .run_if(crate::game::world::level::level_active),
            )
            // Rounds run on the fixed step: when each actor acts, and so
            // every roll, follows simulation time, not how often frames
            // come. Requests made in Update wait for the next step, and a
            // move the round starts or ends governs the party's input on
            // that same step. Spell missiles fly on it too: an impact lands
            // at the step its travel ends, and a defeat it causes is settled
            // before anyone else acts.
            .add_systems(
                FixedUpdate,
                (
                    // Place bodies before arrivals can reflect or burst from them.
                    (
                        crate::game::world::footing::place_bodies,
                        crate::game::hits::effects::fly,
                        settle,
                    )
                        .chain(),
                    // Current capabilities are ready before initiative is accepted.
                    (crate::game::hits::defences::judge_defences, fighters::judge_fighters)
                        .run_if(any_with_component::<Battle>),
                    (stop_party_move, begin_round, call_actor).chain(),
                    // The actor whose initiative came follows its orders or
                    // decides, its conditions may interrupt it, and it acts;
                    (
                        turn::begin_turns,
                        turn::follow_orders,
                        ai::decide_conduct,
                        ai::decide_special,
                        ai::decide_spell,
                        ai::withdraw_if_helpless,
                        ai::decide_guard,
                        ai::default_fight,
                        turn::interrupt,
                        turn::retreat,
                        turn::protect,
                        turn::start_specials,
                        turn::start_casts,
                        swings::open_attacks,
                    )
                        .chain(),
                    // its blows and casts settle within the step,
                    (
                        swings::run_swings,
                        casting::run_casts,
                        turn::defend_instead,
                        summon::summon,
                        settle,
                    )
                        .chain(),
                    // and once everyone has acted the round ends.
                    (
                        judge_round,
                        round_end::leave_fight,
                        round_end::rest_the_idle,
                        round_end::cool_specials,
                        round_end::age_conditions,
                        round_end::recover_pools,
                        // Aging damage is settled before selecting cloud victims.
                        settle.run_if(any_with_component::<RoundEnding>),
                        round_end::pulse_clouds,
                        // The last pulse still sees the round's protections.
                        settle.run_if(any_with_component::<RoundEnding>),
                        round_end::age_enchantments,
                        hostility::judge_sides,
                        round_end::judge_victory,
                        round_end::look_again,
                        end_round,
                    )
                        .chain(),
                )
                    .chain()
                    .in_set(GameplaySet::Combat)
                    .after(crate::game::party::body::turn_party)
                    .before(bevy_tnua::TnuaUserControlsSystems)
                    .run_if(crate::game::world::level::level_active),
            )
            // The party walks on the fixed step; its move is measured on
            // every step, once physics has moved it, so how frames batch
            // those steps changes neither the ground counted nor where the
            // budget runs out.
            .add_systems(
                FixedPostUpdate,
                (track_party_move, approach::spend_moves)
                    .after(avian3d::prelude::PhysicsSystems::Writeback)
                    .run_if(crate::game::world::level::level_active),
            )
            // Monsters' moves likewise: headed before navigation plans the
            // step, charged once physics has walked it.
            .add_systems(
                FixedPreUpdate,
                approach::steer_moves
                    .before(bevy_landmass::LandmassSystems::SyncExistence)
                    .run_if(crate::game::world::level::level_active),
            );
    }
}

/// One settlement system instance owns death credit across all boundaries.
/// Calling it repeatedly must not credit Added<Dead> once per call site.
#[derive(bevy::ecs::schedule::ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
struct Settlement;

fn settle(world: &mut World) {
    world.run_schedule(Settlement);
}

/// Seconds of simulation between actors (retail `combat_delay_ms`, 1000 by
/// default).
const ACTION_SECONDS: f32 = 1.0;
/// Seconds of simulation between rounds, so that a round in which nobody
/// can strike still lets monsters close in.
const ROUND_SECONDS: f32 = 1.0;
/// Rounds in which nobody could attack before the fight peters out.
const PASSIVE_ROUNDS: u32 = 2;

/// An actor participating in the current fight. Removing this relationship
/// removes its fight-specific state, never the actor or its party membership.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
#[relationship(relationship_target = Combatants)]
pub struct Combatant(pub Entity);

/// Participants in action order: party seats first, then joining monsters,
/// maintained by Bevy from [`Combatant`].
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = Combatant)]
pub struct Combatants(Vec<Entity>);

impl Combatants {
    pub fn entities(&self) -> &[Entity] {
        &self.0
    }
}

/// The party's fight, on an entity owned by its level. Exists only while
/// there is one.
#[derive(Component, Debug, Default)]
#[require(Transform, Round, Spoils, PartyMove, Pause)]
pub struct Battle {
    pub round: u32,
    /// Accepted encounter history, consumed when the opening round is initialized.
    opening: Option<SurpriseCause>,
    passive_rounds: u32,
    /// The party's standing choice to walk or run in the next rounds.
    pub party_pace: Option<PartyPace>,
}

/// The round being played, on the fight.
#[derive(Component, Debug, Default)]
pub struct Round {
    phase: RoundPhase,
    /// How the party moves this round.
    pub(crate) pace: Option<PartyPace>,
    /// Someone has attacked this round.
    attacked: bool,
}

/// Accepted work survives pacing/content waits without pretending the round
/// has already initialized its actors. Waiting cannot retain an executing phase.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum RoundPhase {
    #[default]
    Waiting,
    Accepted,
    Executing(u32),
}

impl Round {
    /// A round is accepted or running, including its pacing delay.
    pub fn executing(&self) -> bool {
        self.phase != RoundPhase::Waiting
    }

    /// Something in this round counts as an attack, keeping the fight going.
    pub fn attacked(&mut self, attacked: bool) {
        self.attacked |= attacked;
    }

    /// How far through the round its play is, 0 to 1, while a round is
    /// being played.
    pub fn progress(&self) -> Option<f32> {
        match self.phase {
            RoundPhase::Waiting => None,
            RoundPhase::Accepted => Some(0.0),
            RoundPhase::Executing(step) => Some(step.min(100) as f32 / 100.0),
        }
    }
}

/// What the fight's kills are worth, shared among the living at its end.
#[derive(Component, Debug, Default)]
pub struct Spoils {
    experience: u32,
    kills: u32,
}

/// The pause until the fight's next action.
#[derive(Component, Debug)]
pub struct Pause(pub Timer);

impl Default for Pause {
    fn default() -> Self {
        let mut timer = Timer::from_seconds(0.0, TimerMode::Once);
        timer.tick(std::time::Duration::from_secs(1));
        Self(timer)
    }
}

/// How an attack is named: by its mode's verb ("Swings", "Punches"), or for
/// a monster attack by its own word, with retail's weapon-name flag
/// deciding whether "at" follows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Announcement {
    Mode(AttackMode),
    Monster { word: AttackWord, mode: AttackMode, named: bool },
}

/// Why the party loses its opening turn (retail `RollCombatSurprise`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurpriseCause {
    Ambush,
    Running,
    Searching,
}

/// Automatic encounters already require a hostile that sees the party.
/// Test party awareness before combat turns the camera toward that attacker.
fn opening_surprise<'a>(
    seen: bool,
    shadow_hound: bool,
    camping: bool,
    running: bool,
    searching: bool,
    mut members: impl Iterator<Item = (&'a Character, &'a Vitals)>,
    rng: &mut GameRng,
) -> Option<SurpriseCause> {
    if !shadow_hound {
        if !seen {
            return Some(SurpriseCause::Ambush);
        }
        if camping {
            let guard = members.find(|(_, v)| v.lucid());
            let Some((profile, vitals)) = guard else { return Some(SurpriseCause::Ambush) };
            let senses = profile
                .stats(&vitals.enchantments)
                .attribute(crate::content::characters::Attribute::Senses);
            rng.below(1);
            if senses / 2 <= rng.below(100) {
                return Some(SurpriseCause::Ambush);
            }
        }
    }
    if (running || searching) && rng.below(100) < 20 {
        return Some(if running { SurpriseCause::Running } else { SurpriseCause::Searching });
    }
    None
}

/// What the fight tells the player.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum CombatNews {
    Began {
        nearest: Option<Entity>,
    },
    Surprised(SurpriseCause),
    Breathed {
        actor: Entity,
    },
    /// An attack begins (retail `StartCharacterAttack`,
    /// `AnnounceMonsterAttack`).
    Attack {
        attacker: Entity,
        target: Entity,
        how: Announcement,
        swings: u32,
        surprise: attack::Surprise,
    },
    /// A swing goes astray onto someone else (retail
    /// `AnnounceAccidentalStrike`).
    Fumble {
        attacker: Entity,
        victim: Entity,
    },
    /// What blows did besides damage.
    Effects(Vec<Effect>),
    /// A member's training paid off.
    Improved {
        member: Entity,
        skill: crate::content::characters::Skill,
    },
    /// What it did: hits (that landed, damage or not) and damage dealt.
    Result {
        attacker: Entity,
        target: Entity,
        hits: u32,
        damage: u32,
    },
    Killed {
        actor: Entity,
    },
    Won {
        kills: u32,
        experience_each: u32,
    },
    Lost,
    /// Nobody could reach anybody for a while.
    Over,
    /// A monster advances on the party, or charges it.
    Moves {
        monster: Entity,
        charge: bool,
    },
    /// A monster makes its special attack (retail `MonsterFleeAction`'s
    /// notice); what it did follows as [`CombatNews::Effects`].
    Special {
        monster: Entity,
        kind: crate::content::species::SpecialAttackKind,
    },
    /// A guard takes a blow meant for its ward (retail string 0x21a).
    Intercepts {
        guard: Entity,
        ward: Entity,
    },
    /// An actor's condition took its action (retail
    /// `GetConditionInterrupt`, by its interrupt number).
    Interrupted {
        actor: Entity,
        interrupt: Interrupt,
    },
    /// A monster runs for its life.
    Flees {
        monster: Entity,
    },
    /// A monster moves off from the party.
    Withdraws {
        monster: Entity,
    },
    /// A summoning found nowhere for its group to stand.
    SummonFails,
    /// The party's move begins; it ends when spent or stopped.
    PartyMoves,
    PartyStops,
}

/// Whether the party is fighting, for operations that hold the whole world.
pub fn in_battle(world: &mut World) -> bool {
    world.query_filtered::<(), With<Battle>>().iter(world).next().is_some()
}

/// Terminal state is settled independently of whose action dealt the damage.
/// Used before/after combat execution and at the end of environmental damage.
pub fn settle_defeat(
    mut commands: Commands,
    battle: Option<Single<(Entity, Option<&Combatants>), With<Battle>>>,
    roster: Single<&PartyRoster>,
    mut news: MessageWriter<CombatNews>,
    mut effects: Single<&mut crate::game::magic::enchantments::PartyEffects>,
    mut bodies: Query<&mut Vitals>,
) {
    let Some((battle, combatants)) = battle.map(Single::into_inner) else { return };
    if roster.iter().all(|m| bodies.get(m).unwrap().is_dead()) {
        // What lasts a fight ends on the party and on those in it, as when
        // the party wins.
        effects.0.end_combat();
        let combatants = combatants.map_or(&[][..], Combatants::entities);
        for &monster in combatants {
            if let Ok(mut monster) = bodies.get_mut(monster) {
                monster.enchantments.end_combat();
            }
        }
        news.write(CombatNews::Lost);
        // Its combatants leave the fight and stop closing in; they remain.
        commands.entity(battle).despawn();
    }
}

/// A death in the fight: a hostile body's with a worth is a kill worth it
/// (nothing for one summoned), shared at the fight's end (retail
/// `MonsterStartsDying`); everyone's is announced.
pub fn credit_deaths(
    deaths: Query<
        (
            Entity,
            &Side,
            Option<&crate::game::world::population::Worth>,
            Has<crate::game::magic::pointcasts::SummonedBy>,
        ),
        Added<Dead>,
    >,
    fight: Option<Single<(&mut Spoils, Option<&Combatants>), With<Battle>>>,
    mut news: MessageWriter<CombatNews>,
) {
    let Some((mut spoils, combatants)) = fight.map(Single::into_inner) else { return };
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    for (actor, side, worth, summoned) in &deaths {
        if !combatants.contains(&actor) {
            continue;
        }
        if let Some(worth) = worth
            && side.0 == Stance::Hostile
        {
            spoils.kills += 1;
            if !summoned {
                spoils.experience += worth.0;
            }
        }
        news.write(CombatNews::Killed { actor });
    }
}

/// Starting the round while the party moves ends its move (retail
/// `MGSKeyboard.cpp:568-573`).
pub fn stop_party_move(
    fight: Option<Single<(&Round, &mut PartyMove), With<Battle>>>,
    mut rng: ResMut<GameRng>,
    roster: Single<&PartyRoster>,
    mut movers: Query<(&mut Vitals, &crate::game::inventory::Burden), With<Turn>>,
    mut turns: Query<&mut Turn>,
    mut news: MessageWriter<CombatNews>,
    mut starts: MessageReader<StartRound>,
) {
    let Some((round, mut party_move)) = fight.map(Single::into_inner) else {
        starts.clear();
        return;
    };
    if starts.read().count() > 0 && matches!(*party_move, PartyMove::Moving { .. }) {
        finish_party_move((round, &mut party_move), &mut rng, &roster, &mut movers, &mut turns);
        news.write(CombatNews::PartyStops);
    }
}

/// Start the round the player asks for, or in a continuous fight the next
/// one once the last action's pause is over and nothing is in flight
/// (retail `UpdateCombat`, `BeginCombatExecution`). Choosing the party's
/// pace between rounds is taken here too.
#[allow(clippy::type_complexity)]
pub fn begin_round(
    mut commands: Commands,
    time: Res<Time>,
    fight: Option<
        Single<(&mut Battle, &mut Round, &mut PartyMove, &mut Pause, Option<&Combatants>)>,
    >,
    mut rng: ResMut<GameRng>,
    roster: Single<&PartyRoster>,
    bodies: Query<(
        &Vitals,
        &fighters::Arms,
        &fighters::Initiative,
        Option<(&Character, &crate::game::party::Order)>,
    )>,
    settings: Res<CombatSettings>,
    in_flight: Query<(), With<crate::game::hits::effects::InFlight>>,
    moving: Query<(), With<approach::CombatMove>>,
    mut starts: MessageReader<StartRound>,
    mut moves: MessageReader<CyclePartyMove>,
) {
    let Some((mut battle, mut round, mut party_move, mut pause, combatants)) =
        fight.map(Single::into_inner)
    else {
        starts.clear();
        moves.clear();
        return;
    };
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    let start = starts.read().count() > 0;
    for _ in moves.read() {
        if !round.executing() {
            battle.party_pace = match battle.party_pace {
                None => Some(PartyPace::Walk),
                Some(PartyPace::Walk) => Some(PartyPace::Run),
                Some(PartyPace::Run) => None,
            };
        }
    }
    // Between rounds a phased fight waits for the player; a continuous one
    // goes on.
    if (start || battle.opening.is_some()) && !round.executing() {
        turn::accept_round(
            &mut commands,
            (&mut battle, &mut round, &mut party_move),
            combatants,
            &roster,
            &bodies,
            &mut rng,
        );
    }
    // A finished pause is not ticked again: a fight waiting for the player
    // stays unchanged.
    if !pause.0.is_finished() && !pause.0.tick(time.delta()).is_finished() {
        return;
    }
    // The next actor waits until the last one's missiles have landed
    // (retail `UpdateCombat`).
    if !in_flight.is_empty() || !moving.is_empty() {
        return;
    }
    if !round.executing() && settings.continuous {
        turn::accept_round(
            &mut commands,
            (&mut battle, &mut round, &mut party_move),
            combatants,
            &roster,
            &bodies,
            &mut rng,
        );
    }
}

/// Call the actor whose initiative has come, one per pause, once nothing
/// is in flight: the party's move when its phase comes, then the members in
/// accepted order, then the monsters in theirs. A step with nobody due
/// passes to the next.
#[allow(clippy::type_complexity)]
fn call_actor(
    mut commands: Commands,
    fight: Option<Single<(&mut Round, &mut PartyMove, &Pause, Option<&Combatants>), With<Battle>>>,
    roster: Single<&PartyRoster>,
    footings: Query<&crate::game::world::footing::Footing>,
    vitals: Query<&Vitals>,
    turns: Query<&Turn>,
    in_flight: Query<(), With<crate::game::hits::effects::InFlight>>,
    moving: Query<(), With<approach::CombatMove>>,
    acting: Query<(), With<turn::Acting>>,
    mut news: MessageWriter<CombatNews>,
) {
    let Some((mut round, mut party_move, pause, combatants)) = fight.map(Single::into_inner) else {
        return;
    };
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    if !round.executing()
        || !pause.0.is_finished()
        || !in_flight.is_empty()
        || !moving.is_empty()
        || !acting.is_empty()
    {
        return;
    }
    if round.phase == RoundPhase::Accepted {
        round.phase = RoundPhase::Executing(1);
    }
    while let RoundPhase::Executing(step) = round.phase
        && step <= 100
    {
        if matches!(*party_move, PartyMove::Moving { .. }) {
            return;
        }
        if let PartyMove::Pending { pace, phase } = *party_move
            && phase <= step
        {
            let haste = roster
                .iter()
                .try_fold(0, |sum, member| {
                    let effect = vitals
                        .get(member)
                        .unwrap()
                        .enchantments
                        .get(crate::content::spells::SpellId::HASTE)?;
                    Some(sum + effect.scaled(effect.power * 10))
                })
                .map_or(0.0, |sum| (sum / roster.len() as u32) as f32 * 0.2);
            let last = roster
                .iter()
                .find_map(|member| footings.get(member).ok())
                .map_or(Vec3::ZERO, |at| at.centre);
            *party_move =
                PartyMove::Moving { pace, budget: pace.combat_budget() + haste, moved: 0.0, last };
            news.write(CombatNews::PartyMoves);
            return;
        }
        // Nobody in the party acts while its move is still to come.
        let party_waits = matches!(*party_move, PartyMove::Pending { .. });
        let due = combatants
            .iter()
            .copied()
            .filter(|actor| !party_waits || !roster.contains(actor))
            .find(|&actor| turns.get(actor).is_ok_and(|turn| turn.phase == step));
        if let Some(actor) = due {
            commands.entity(actor).insert(turn::Acting);
            return;
        }
        round.phase = RoundPhase::Executing(step + 1);
    }
}

/// The roster controls formation and initial ordering; participation controls
/// execution and cleanup for both kinds of actor.
fn join_party(
    joined: On<Add<Battle>>,
    mut commands: Commands,
    roster: Option<Single<&PartyRoster>>,
) {
    if let Some(roster) = roster {
        for actor in roster.iter() {
            commands.entity(actor).insert(Combatant(joined.entity));
        }
    }
}

/// Targets in combat use participation order. Outside combat the party still
/// exists, and spells may target it without creating a battle.
pub(crate) fn participants<'a>(
    roster: &'a PartyRoster,
    combatants: &'a [Entity],
) -> impl Iterator<Item = Entity> + 'a {
    combatants.iter().copied().chain(roster.iter().filter(|_| combatants.is_empty()))
}
