//! Combat requests, encounter lifetime and round execution. Accepted actions
//! and round history live in `round`; attacks and magic settle their consequences
//! before the scheduler advances to the next actor.

pub mod approach;
pub mod attack;
pub mod breath;
mod conduct;
mod engage;
mod member_turn;
mod orders;
mod round_end;
mod scene;
mod sight;
mod swings;
use crate::game::GameplaySet;
use crate::game::data::Catalogs;
pub use conduct::Interrupt;
pub use orders::*;
pub use round_end::{RoundEnding, end_round, judge_round};
use scene::*;
use sight::*;
mod monster_spells;
mod monster_turn;
mod round;
pub use round::PartyTurn;
use round::{Acting, MonsterTurn, RoundAction, accept_round};
pub mod summon;

use crate::game::world::level::LevelOwned;
use crate::game::world::level::PlayedLevel;
use crate::game::world::navigation::{Walker, feet};
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::perception::{party_distance, sight_threshold};
use crate::game::world::{ActorPose, ActorPoseItem};
use avian3d::prelude::Position;
use bevy::prelude::*;

use crate::content::items::AttackMode;
use crate::content::species::AttackWord;

use crate::content::species::{MonsterCatalog, Reach, Species};
use crate::game::characters::{Character, Condition, Vitals};
use crate::game::hits::Effect;
use crate::game::magic::casting::{self};
use crate::game::party::Members;
use crate::game::party::Order;
use crate::game::party::body::PartyBody;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::time::GameClock;
use crate::game::world::damage::MonsterDamage;
use crate::game::world::hostility;
use crate::game::world::identify::{GroupMember, GroupMembers, GroupRecognition, MonsterGroup};
use crate::game::world::population::{Monster, blinded};
use bevy::ecs::relationship::RelationshipTarget;

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        swings::stages(app);
        app.add_observer(round::clear_fight)
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
            .add_observer(approach::end_move)
            .add_observer(approach::stop_walking)
            .add_systems(
                Update,
                (
                    give_orders,
                    // Defeat is settled before a fight starts,
                    settle_defeat,
                    // alerts, then engagement, then the opening,
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
                    (
                        crate::game::characters::settle_deaths,
                        settle_defeat,
                        begin_round,
                        call_actor,
                    )
                        .chain(),
                    // The member whose initiative came acts,
                    (
                        member_turn::member_turns,
                        swings::member_attacks,
                        member_turn::start_member_cast,
                        member_turn::start_breath,
                    )
                        .chain(),
                    // or the monster decides and acts;
                    (
                        monster_turn::begin_monster_turns,
                        monster_turn::decide_conduct,
                        monster_turn::decide_special,
                        monster_turn::decide_spell,
                        monster_turn::withdraw_if_helpless,
                        monster_turn::decide_guard,
                        monster_turn::default_fight,
                        monster_turn::interrupt,
                        monster_turn::retreat,
                        monster_turn::protect,
                        monster_turn::start_special,
                        monster_turn::start_monster_cast,
                        swings::monster_attacks,
                    )
                        .chain(),
                    // its blows and casts settle within the step,
                    (swings::run_swings, casting::run_casts, member_turn::defend_instead).chain(),
                    // and once everyone has acted the round ends.
                    (
                        judge_round,
                        round_end::leave_fight,
                        round_end::rest_waiters,
                        round_end::cool_specials,
                        round_end::age_conditions,
                        round_end::recover_members,
                        round_end::pulse_clouds,
                        round_end::age_enchantments,
                        round_end::rest_defenders,
                        round_end::judge_victory,
                        round_end::look_again,
                        end_round,
                    )
                        .chain(),
                    (
                        summon::summon,
                        // and again after the round's damage.
                        crate::game::characters::settle_deaths,
                        settle_defeat,
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

/// Seconds of simulation between actors (retail `combat_delay_ms`, 1000 by
/// default).
const ACTION_SECONDS: f32 = 1.0;
/// Seconds of simulation between rounds, so that a round in which nobody
/// can strike still lets monsters close in.
const ROUND_SECONDS: f32 = 1.0;
/// Rounds in which nobody could attack before the fight peters out.
const PASSIVE_ROUNDS: u32 = 2;

/// A monster taking part in a fight. Ending the fight ends the
/// participation, never the monster.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
#[relationship(relationship_target = Combatants)]
pub struct Combatant(pub Entity);

/// The monsters in a fight, in the order they joined (by location id),
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
#[require(Transform)]
pub struct Battle {
    pub round: u32,
    /// Accepted encounter history, consumed when the opening round is initialized.
    opening: Option<SurpriseCause>,
    phase: RoundPhase,
    /// How the party moves this round.
    pace: Option<PartyPace>,
    /// Someone has attacked this round.
    attacked: bool,
    /// Experience from kills, shared at the end.
    experience: u32,
    kills: u32,
    passive_rounds: u32,
    /// The party's standing choice to walk or run in the next rounds.
    pub party_pace: Option<PartyPace>,
    party_move: PartyMove,
    /// Until the next action.
    wait: Timer,
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

impl Battle {
    /// A round is accepted or running, including its pacing delay.
    pub fn executing(&self) -> bool {
        self.phase != RoundPhase::Waiting
    }

    /// Whether the party may walk: in a fight, only during its own move.
    pub fn party_may_move(&self) -> bool {
        matches!(self.party_move, PartyMove::Moving { .. })
    }

    /// Whether the party's move is a run.
    pub fn party_running(&self) -> bool {
        matches!(self.party_move, PartyMove::Moving { pace: PartyPace::Run, .. })
    }

    /// A monster died to the party: its experience is shared at the end.
    /// Dead, it acts no more.
    pub fn credit_kill(
        &mut self,
        monster: Entity,
        experience: u32,
        news: &mut MessageWriter<CombatNews>,
    ) {
        self.experience += experience;
        self.kills += 1;
        news.write(CombatNews::Killed { actor: monster });
    }

    /// Credit and announce the kills a spell's effects made, whether it
    /// landed at once or as a missile: the one place spell kills are counted.
    /// A member it killed is announced as one a monster's blow kills is.
    pub fn credit_kills(
        &mut self,
        effects: &[Effect],
        members: &Query<(), With<crate::game::characters::Character>>,
        news: &mut MessageWriter<CombatNews>,
    ) {
        for effect in effects {
            let Effect::Damaged { victim, outcome, .. } = *effect else { continue };
            if !outcome.died {
                continue;
            }
            if members.contains(victim) {
                news.write(CombatNews::Killed { actor: victim });
            } else {
                self.credit_kill(victim, outcome.experience, news);
            }
        }
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
    /// A monster's condition took its action (retail
    /// `GetConditionInterrupt`, by its interrupt number).
    Interrupted {
        monster: Entity,
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

/// Start the round the player asks for, or in a continuous fight the next
/// one once the last action's pause is over and nothing is in flight
/// (retail `UpdateCombat`, `BeginCombatExecution`). Choosing the party's
/// pace between rounds is taken here too.
pub fn begin_round(
    mut commands: Commands,
    time: Res<Time>,
    battle: Option<Single<(&mut Battle, Option<&Combatants>)>>,
    mut rng: ResMut<GameRng>,
    (mut members, damage): (Members, MonsterDamage),
    (roster, orders, settings): (Single<&PartyRoster>, Query<&Order>, Res<CombatSettings>),
    mut turns: Query<&mut PartyTurn>,
    content: If<Res<Catalogs>>,
    (in_flight, moving): (
        Query<(), With<crate::game::hits::effects::InFlight>>,
        Query<(), With<approach::CombatMove>>,
    ),
    mut news: MessageWriter<CombatNews>,
    (mut starts, mut moves): (MessageReader<StartRound>, MessageReader<CyclePartyMove>),
) {
    let Some((mut battle, combatants)) = battle.map(Single::into_inner) else {
        starts.clear();
        moves.clear();
        return;
    };
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    let start = starts.read().count() > 0;
    for _ in moves.read() {
        if !battle.executing() {
            battle.party_pace = match battle.party_pace {
                None => Some(PartyPace::Walk),
                Some(PartyPace::Walk) => Some(PartyPace::Run),
                Some(PartyPace::Run) => None,
            };
        }
    }
    // Starting the round while the party moves ends its move (retail
    // `MGSKeyboard.cpp:568-573`).
    if start && matches!(battle.party_move, PartyMove::Moving { .. }) {
        finish_party_move(&mut battle, &mut rng, (&roster, &mut members, &mut turns));
        news.write(CombatNews::PartyStops);
    }
    let (items, species) = (&content.items, &content.species);
    let roster = &**roster;
    let mut accept = |battle: &mut Battle, rng: &mut GameRng| {
        accept_round(
            &mut commands,
            battle,
            combatants,
            roster,
            &members,
            &orders,
            damage.monsters(),
            (items, species),
            rng,
        )
    };
    // Between rounds a phased fight waits for the player; a continuous one
    // goes on.
    if (start || battle.opening.is_some()) && !battle.executing() {
        accept(&mut battle, &mut rng);
    }
    // A finished pause is not ticked again: a fight waiting for the player
    // stays unchanged.
    if !battle.wait.is_finished() && !battle.wait.tick(time.delta()).is_finished() {
        return;
    }
    // The next actor waits until the last one's missiles have landed
    // (retail `UpdateCombat`).
    if !in_flight.is_empty() || !moving.is_empty() {
        return;
    }
    if !battle.executing() && settings.continuous {
        accept(&mut battle, &mut rng);
    }
}

/// Call the actor whose initiative has come, one per pause, once nothing
/// is in flight: the party's move when its phase comes, then the members in
/// accepted order, then the monsters in theirs. A step with nobody due
/// passes to the next.
fn call_actor(
    mut commands: Commands,
    battle: Option<Single<(&mut Battle, Option<&Combatants>)>>,
    (roster, party_body): (Single<&PartyRoster>, Single<&Position, With<PartyBody>>),
    vitals: Query<&Vitals>,
    (party_turns, monster_turns): (Query<&PartyTurn>, Query<&MonsterTurn>),
    (in_flight, moving, acting): (
        Query<(), With<crate::game::hits::effects::InFlight>>,
        Query<(), With<approach::CombatMove>>,
        Query<(), With<Acting>>,
    ),
    mut news: MessageWriter<CombatNews>,
) {
    let Some((mut battle, combatants)) = battle.map(Single::into_inner) else { return };
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    if !battle.executing()
        || !battle.wait.is_finished()
        || !in_flight.is_empty()
        || !moving.is_empty()
        || !acting.is_empty()
    {
        return;
    }
    if battle.phase == RoundPhase::Accepted {
        battle.phase = RoundPhase::Executing(1);
    }
    while let RoundPhase::Executing(step) = battle.phase
        && step <= 100
    {
        if matches!(battle.party_move, PartyMove::Moving { .. }) {
            return;
        }
        if let PartyMove::Pending { pace, phase } = battle.party_move
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
            battle.party_move = PartyMove::Moving {
                pace,
                budget: pace.combat_budget() + haste,
                moved: 0.0,
                last: party_body.0,
            };
            news.write(CombatNews::PartyMoves);
            return;
        }
        // Nobody in the party acts while its move is still to come.
        let party_waits = matches!(battle.party_move, PartyMove::Pending { .. });
        let due = roster
            .iter()
            .filter(|_| !party_waits)
            .find(|&member| party_turns.get(member).is_ok_and(|turn| turn.phase == step))
            .or_else(|| {
                combatants.iter().copied().find(|&monster| {
                    monster_turns.get(monster).is_ok_and(|turn| turn.phase == step)
                })
            });
        if let Some(actor) = due {
            commands.entity(actor).insert(Acting);
            return;
        }
        battle.phase = RoundPhase::Executing(step + 1);
    }
}

/// When the remaining attacks of a turn come: spread over the rest of the
/// round, or none.
fn next_phase(step: u32, attacks_left: u32) -> u32 {
    if attacks_left == 0 {
        return 0;
    }
    let next = step + (100 - step) / (attacks_left + 1);
    if next > 100 { 0 } else { next }
}
