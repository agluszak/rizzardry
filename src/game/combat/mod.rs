//! Combat requests, encounter lifetime and round execution. Accepted actions
//! and round history live on the actors ([`turn`]); attacks and magic settle
//! their consequences before the scheduler advances to the next actor.

pub mod action;
pub mod approach;
pub mod attack;
pub mod breath;
pub mod effort;
mod engage;
pub mod monster;
mod orders;
mod round_end;
pub mod summon;
mod swings;
pub mod turn;

#[cfg(test)]
mod tests;

use crate::game::GameplaySet;
pub use action::Interrupt;
pub use orders::*;
pub use round_end::{RoundEnds, end_round};
pub use turn::{Action, Ordered};

use crate::game::world::level::LevelOwned;
use crate::game::world::level::PlayedLevel;
use bevy::prelude::*;

use crate::content::items::AttackMode;
use crate::content::species::AttackWord;

use crate::game::characters::{Dead, Vitals};
use crate::game::hits::Effect;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::hostility::{self, Side, Stance};
use crate::game::world::identify::{GroupMember, GroupMembers, GroupRecognition, MonsterGroup};
use crate::game::world::population::Monster;
use bevy::ecs::relationship::RelationshipTarget;

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            hostility::plugin,
            swings::plugin,
            turn::plugin,
            action::plugin,
            monster::plugin,
            engage::plugin,
        ))
        .add_observer(credit_death)
        .add_observer(count_kill)
        .add_observer(settle_defeat.run_if(resource_exists::<Battle>))
        .add_observer(round_end::end_the_round)
        .add_observer(orders::end_party_move)
        .add_observer(round_end::leave_fight)
        .add_observer(round_end::rest_the_idle)
        .add_observer(round_end::cool_specials)
        .add_observer(round_end::pulse_clouds)
        .add_observer(count_cast)
        .add_observer(round_end::judge_victory)
        .add_observer(round_end::look_again)
        .add_observer(round_end::end_round)
        .add_observer(join_party)
        .add_observer(approach::end_move)
        .add_observer(approach::stop_walking)
        .add_observer(effort::tire)
        .add_observer(effort::tires)
        .add_observer(summon::summon)
        .add_message::<CombatNews>()
        .add_message::<CyclePartyMove>()
        .add_message::<StartRound>()
        .add_message::<CombatControl>()
        .add_message::<crate::game::camp::CampNews>()
        .add_message::<GiveOrder>()
        .add_message::<engage::Engaged>()
        .init_resource::<CombatSettings>()
        .add_systems(
            Update,
            (
                give_orders.run_if(resource_exists::<Battle>),
                // alerts and turning, then who comes in, then the opening,
                engage::alert_factions.run_if(resource_exists::<Battle>),
                engage::turn_by_radius,
                engage::come_in_provoked,
                engage::look_to_open.run_if(not(resource_exists::<Battle>)),
                engage::join.run_if(resource_exists::<Battle>),
                engage::tell,
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
        // that same step. Spell missiles fly on it too: one lands at
        // the step its flight ends, its deaths and a defeat it causes
        // settled before anyone else acts.
        .add_systems(
            FixedUpdate,
            (
                // What was in the air has landed before anyone acts.
                crate::game::hits::fly,
                // The earliest moment comes: its actor decides and acts,
                // and what the act did lands, its deaths settled, before
                // the next moment. When none is left the round ends.
                (stop_party_move, begin_round, next_moment)
                    .chain()
                    .run_if(resource_exists::<Battle>),
            )
                .chain()
                .in_set(GameplaySet::Combat)
                .run_if(crate::game::world::level::level_active)
                .after(crate::game::party::body::turn_party)
                .before(bevy_tnua::TnuaUserControlsSystems),
        )
        // What a moment or a round's end did to bodies is settled, and
        // only then is a closing round judged.
        .add_systems(
            FixedUpdate,
            round_end::close_round
                .after(crate::game::actors::settle)
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
        // What monsters make out follows where everyone stands: before the
        // frame's engagement and the fixed step's fighting and charging.
        .add_systems(
            Update,
            (monster::sight::hiding, monster::sight::look, monster::sight::reinforce)
                .chain()
                .in_set(GameplaySet::Bodies)
                .run_if(crate::game::world::level::level_active),
        )
        .add_systems(
            FixedPreUpdate,
            (monster::sight::hiding, monster::sight::look, monster::sight::reinforce)
                .chain()
                .run_if(crate::game::world::level::level_active),
        )
        .add_systems(
            FixedPreUpdate,
            approach::steer_moves
                .before(bevy_landmass::LandmassSystems::SyncExistence)
                .run_if(crate::game::world::level::level_active),
        );
    }
}

/// Seconds of simulation between actors (retail `combat_delay_ms`, 1000 by
/// default).
const ACTION_SECONDS: f32 = 1.0;
/// The last moment of a round's timeline.
const LAST_MOMENT: u32 = 100;
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

/// The party's fight: a resource, so there is never more than one, on the
/// entity of the fight, owned by its level, which combatants are related to.
/// Exists only while there is one. A fight ends by removing the resource,
/// and its entity goes with it, and with that every combatant's place in it.
#[derive(Resource, Debug, Default)]
#[require(Transform, Round, Spoils, PartyMove, Pause)]
#[component(on_remove = over)]
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
    /// A round is accepted or being played.
    playing: bool,
    /// The moment the round has come to, 1 to 100: 0 while it is accepted
    /// and not begun, past 100 once no moment is left.
    moment: u32,
    /// How the party moves this round.
    pub(crate) pace: Option<PartyPace>,
    /// Someone has attacked this round.
    attacked: bool,
    /// Those who leave the fight as the round ends.
    leaving: Vec<Entity>,
    /// The fight is over as the round ends.
    over: bool,
}

impl Round {
    /// A round is accepted or running, including its pacing delay.
    pub fn executing(&self) -> bool {
        self.playing
    }

    /// The moment the round has come to, 1 to 100; 0 before it begins.
    pub fn moment(&self) -> u32 {
        self.moment.min(LAST_MOMENT)
    }

    /// Something in this round counts as an attack, keeping the fight going.
    pub fn attacked(&mut self, attacked: bool) {
        self.attacked |= attacked;
    }

    /// How far through the round its play is, 0 to 1, while a round is
    /// being played.
    pub fn progress(&self) -> Option<f32> {
        self.playing.then(|| self.moment.min(100) as f32 / 100.0)
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
    mut lucid: impl Iterator<Item = &'a crate::game::actors::Attributes>,
    rng: &mut GameRng,
) -> Option<SurpriseCause> {
    if !shadow_hound {
        if !seen {
            return Some(SurpriseCause::Ambush);
        }
        if camping {
            let Some(attributes) = lucid.next() else { return Some(SurpriseCause::Ambush) };
            let senses = attributes[crate::content::characters::Attribute::Senses];
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

/// A round of the fight has begun.
#[derive(Event, Clone, Copy, Debug)]
pub struct RoundBegan;

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
    /// A member swings a weapon of this sound class.
    Swung {
        sound_class: Option<u8>,
    },
    /// A blow gets through to a body, at a place of it (for a member, head,
    /// body, legs, hands or feet in that order), by an arm of this sound
    /// class.
    Struck {
        victim: Entity,
        place: usize,
        sound_class: Option<u8>,
    },
    /// A blow that barely missed is blocked, by an arm of this sound class
    /// (retail `BlockedForSpecialReason`).
    Blocked {
        victim: Entity,
        sound_class: Option<u8>,
    },
    /// A swing goes astray onto someone else (retail
    /// `AnnounceAccidentalStrike`).
    Fumble {
        attacker: Entity,
        victim: Entity,
    },
    /// What blows did besides damage.
    Effects(Vec<Effect>),
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
    /// A member made a kill worth speaking of.
    Feat {
        member: Entity,
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

/// A fight that is over goes, as the resource it was: its round and spoils
/// with it, and its combatants are in it no longer.
fn over(mut world: bevy::ecs::world::DeferredWorld, context: bevy::ecs::lifecycle::HookContext) {
    world
        .commands()
        .entity(context.entity)
        .try_remove::<bevy::ecs::resource::IsResource>()
        .try_despawn();
}

/// A party that has fallen has lost the fight it was in: its combatants
/// leave the fight and stop closing in; they remain.
pub fn settle_defeat(
    _: On<crate::game::party::PartyFell>,
    mut commands: Commands,
    mut news: MessageWriter<CombatNews>,
) {
    news.write(CombatNews::Lost);
    commands.remove_resource::<Battle>();
}

/// A death in the fight: a hostile body's with a worth is a kill worth it
/// (nothing for one summoned), shared at the fight's end (retail
/// `RecordMonsterKill`); everyone's is announced.
fn credit_death(
    died: On<Add<Dead>>,
    bodies: Query<(
        &Side,
        Option<&crate::game::world::population::Worth>,
        Has<crate::game::magic::pointcasts::SummonedBy>,
        Has<Combatant>,
    )>,
    mut spoils: Single<&mut Spoils, With<Battle>>,
    mut news: MessageWriter<CombatNews>,
) -> Result {
    let actor = died.entity;
    let (side, worth, summoned, fighting) = bodies.get(actor)?;
    if !fighting {
        return Ok(());
    }
    if let Some(worth) = worth
        && side.own() == Stance::Hostile
    {
        spoils.kills += 1;
        if !summoned {
            spoils.experience += worth.0;
        }
    }
    news.write(CombatNews::Killed { actor });
    Ok(())
}

/// A member who killed a hostile in the fight has it on their tally; if it
/// was a kill to speak of, of a level two or more above the party's
/// average, they speak (retail `RecordMonsterKill`).
#[allow(clippy::type_complexity)]
fn count_kill(
    died: On<Add<Dead>>,
    killed: Query<(
        (&Dead, &Side, &crate::game::actors::Level),
        (Has<crate::game::world::population::Notable>, Has<Combatant>),
        Has<crate::game::world::population::Worth>,
    )>,
    mut members: Query<
        (&mut crate::game::characters::Tally, &crate::game::actors::Level),
        With<crate::game::party::roster::PartyMember>,
    >,
    tallied: Query<Has<crate::game::party::roster::PartyMember>>,
    mut news: MessageWriter<CombatNews>,
) -> Result {
    let ((&Dead { by }, side, level), (notable, fighting), worth) = killed.get(died.entity)?;
    let Some(killer) = by.filter(|&killer| tallied.get(killer).is_ok_and(|member| member)) else {
        return Ok(());
    };
    if !fighting || !worth || side.own() != Stance::Hostile {
        return Ok(());
    }
    let (sum, count) =
        members.iter().fold((0, 0), |(sum, count), (_, level)| (sum + level.0, count + 1));
    let (mut tally, _) = members.get_mut(killer)?;
    tally.kills += 1;
    if notable && sum / count + 2 <= level.0 {
        news.write(CombatNews::Feat { member: killer });
    }
    Ok(())
}

/// Starting the round while the party moves ends its move (retail
/// `MGSKeyboard.cpp:568-573`).
pub fn stop_party_move(
    mut commands: Commands,
    fight: Option<Single<&PartyMove, With<Battle>>>,
    mut news: MessageWriter<CombatNews>,
    mut starts: MessageReader<StartRound>,
) {
    let Some(party_move) = fight else {
        starts.clear();
        return;
    };
    if starts.read().count() > 0 && matches!(**party_move, PartyMove::Moving { .. }) {
        commands.trigger(PartyMoveEnds);
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
    fight: Option<Single<(&mut Battle, &Round, &mut Pause)>>,
    settings: Res<CombatSettings>,
    in_flight: Query<(), With<crate::game::hits::Deed>>,
    moving: Query<(), With<approach::CombatMove>>,
    mut starts: MessageReader<StartRound>,
    mut moves: MessageReader<CyclePartyMove>,
) {
    let Some((mut battle, round, mut pause)) = fight.map(Single::into_inner) else {
        starts.clear();
        moves.clear();
        return;
    };
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
    let accepted = (start || battle.opening.is_some()) && !round.executing();
    if accepted {
        commands.trigger(turn::Accepts);
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
    if !accepted && !round.executing() && settings.continuous {
        commands.trigger(turn::Accepts);
    }
}

/// Play the earliest moment still to come, once the pace allows it (the
/// last act's pause over, nothing in flight, nobody walking): the party's
/// move when its moment comes, else the actor whose moment is earliest,
/// who is given it. Two at the same moment go in the fight's order, the
/// party's move first; members do not act before the party's move. When
/// no moment is left the timeline is over and the round ends.
#[allow(clippy::type_complexity)]
fn next_moment(
    mut commands: Commands,
    fight: Option<
        Single<(Entity, &mut Round, &mut PartyMove, &Pause, Option<&Combatants>), With<Battle>>,
    >,
    roster: Single<&PartyRoster>,
    places: crate::game::actors::Places,
    laid: crate::game::actors::Laid,
    turns: Query<&turn::NextMoment>,
    in_flight: Query<(), With<crate::game::hits::Deed>>,
    moving: Query<(), With<approach::CombatMove>>,
    mut news: MessageWriter<CombatNews>,
) {
    let Some((fight, mut round, mut party_move, pause, combatants)) = fight.map(Single::into_inner)
    else {
        return;
    };
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    if !round.executing()
        || !pause.0.is_finished()
        || !in_flight.is_empty()
        || !moving.is_empty()
        || matches!(*party_move, PartyMove::Moving { .. })
    {
        return;
    }
    // Nobody in the party acts while its move is still to come.
    let party_waits = matches!(*party_move, PartyMove::Pending { .. });
    let next = combatants
        .iter()
        .copied()
        .filter(|actor| !party_waits || !roster.contains(actor))
        .filter_map(|actor| Some((actor, turns.get(actor).ok()?.0)))
        .min_by_key(|&(_, when)| when);
    if let PartyMove::Pending { pace, phase } = *party_move
        && next.is_none_or(|(_, when)| phase <= when)
    {
        round.moment = round.moment.max(phase);
        // The party moves further only when every member is hasted: by
        // a fifth of the Speed Haste adds them, averaged.
        let haste = roster
            .iter()
            .try_fold(0, |sum, member| {
                let effect = laid.get(member, crate::content::spells::SpellId::HASTE)?;
                Some(sum + effect.scaled(effect.power * 10))
            })
            .map_or(0.0, |sum| (sum / roster.len() as u32) as f32 * 0.2);
        let last = roster.first().map_or(Vec3::ZERO, |&member| places.of(member).centre);
        *party_move =
            PartyMove::Moving { pace, budget: pace.combat_budget() + haste, moved: 0.0, last };
        news.write(CombatNews::PartyMoves);
        return;
    }
    match next {
        Some((actor, when)) => {
            round.moment = round.moment.max(when);
            commands.trigger(turn::Moment { actor });
        }
        None => {
            round.moment = LAST_MOMENT + 1;
            commands.trigger(RoundEnds { fight });
        }
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

/// A spell that goes off in a fight counts as an attack, keeping the fight
/// going.
fn count_cast(
    _: On<Add<crate::game::magic::landed::Spell>>,
    round: Option<Single<&mut Round, With<Battle>>>,
) {
    if let Some(mut round) = round {
        round.attacked(true);
    }
}
