//! The party's fights, first slice: the party
//! against the hostile monsters that see it, weapon attacks only, everyone
//! fighting on their own (retail's continuous mode with every member set to
//! attack). Who reaches whom in melee follows the party's formation.

pub mod approach;
pub mod attack;
mod blows;
mod conduct;
pub use conduct::Interrupt;
mod monster_spells;
pub mod summon;

use crate::game::world::level::LevelOwned;
use crate::game::world::level::PlayedLevel;
use crate::game::world::navigation::{Walker, feet};
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::{ActorPose, ActorPoseItem};
use avian3d::prelude::Position;
use bevy::prelude::*;

use crate::content::items::AttackMode;
use crate::content::species::AttackWord;

use crate::content::items::{Dice, ItemCatalog};
use crate::content::species::{MonsterCatalog, Reach, Species};
use crate::game::characters::{Character, Condition, Vitals};
use crate::game::inventory::{EquipSlot, StackId};
use crate::game::magic::casting::{self, MagicChoice, MagicNews};
use crate::game::magic::{Effect, LearnedSpells, MagicContext, SpellPoints, Victim};
use crate::game::party::Members;
use crate::game::party::Order;
use crate::game::party::body::PartyBody;
use crate::game::party::formation::{self, Formation};
use crate::game::party::recovery;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::time::GameClock;
use crate::game::world::damage::{MonsterDamage, MonsterQuery};
use crate::game::world::hostility;
use crate::game::world::identify::{GroupMember, GroupMembers, GroupRecognition, MonsterGroup};
use crate::game::world::population::Monster;
use attack::{Armour, phase};
use bevy::ecs::relationship::RelationshipTarget;

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<CombatNews>()
            .add_message::<hostility::Provocation>()
            .add_message::<CyclePartyMove>()
            .add_message::<StartRound>()
            .add_message::<summon::Summon>()
            .add_message::<CombatControl>()
            .add_message::<GiveOrder>()
            .init_resource::<CombatSettings>()
            .init_resource::<casting::FlightOrder>()
            .add_observer(approach::end_move)
            .add_observer(approach::stop_walking)
            .add_systems(
                Update,
                (
                    give_orders,
                    // Defeat is settled before a fight starts or a round runs,
                    settle_defeat,
                    hostility::provoke,
                    start_battle,
                    control_combat,
                    begin_round,
                    run_battle,
                    summon::summon,
                    // and again after the round's damage.
                    settle_defeat,
                    approach::close_in,
                )
                    .chain()
                    .in_set(crate::game::GameplaySet::Combat)
                    .run_if(crate::game::world::level::level_active),
            )
            // The party walks on the fixed step; its move is measured on
            // every step, once physics has moved it, so how frames batch
            // those steps changes neither the ground counted nor where the
            // budget runs out.
            .add_systems(
                FixedPostUpdate,
                track_party_move
                    .after(avian3d::prelude::PhysicsSystems::Writeback)
                    .run_if(crate::game::world::level::level_active),
            );
    }
}

/// The party's reach point is this far under its eye (retail
/// `g_default_world_height`, 1000 world units).
const CAMERA_ABOVE_PARTY_POINT: f32 = 2.0;
/// The party navigator's alternate radius (retail's default, 500 units).
const PARTY_ALTERNATE_RADIUS: f32 = 1.0;
/// A monster facing within this of the party sees it as far as anyone can
/// (retail `g_monster_facing_tolerance`, 45 degrees).
const FACING_TOLERANCE: f32 = std::f32::consts::FRAC_PI_4;

/// Seconds between actors (retail `combat_delay_ms`, 1000 by default).
const ACTION_SECONDS: f32 = 1.0;
/// Real seconds between rounds, so that a round in which nobody can strike
/// still lets monsters close in.
const ROUND_SECONDS: f32 = 1.0;
/// Game time one round takes.
const ROUND_MS: u64 = 120_000;
/// Rounds in which nobody could attack before the fight peters out.
const PASSIVE_ROUNDS: u32 = 2;

/// Retail's combat settings (`ResetGameplaySettings`): phased by default,
/// each round started by the player.
#[derive(Resource, Debug, Default)]
pub struct CombatSettings {
    pub continuous: bool,
}

/// How the party moves in a fight (retail Walk and Run party actions).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartyPace {
    Walk,
    Run,
}

/// Give a member their order for the coming rounds.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct GiveOrder {
    pub member: Entity,
    pub order: crate::game::party::Order,
}

/// Orders are given to the party's members during a fight.
fn give_orders(
    mut requests: MessageReader<GiveOrder>,
    battle: Option<Single<&Battle>>,
    roster: Single<&crate::game::party::roster::PartyRoster>,
    mut orders: Query<(&mut crate::game::party::Order, &Character)>,
) {
    use crate::content::characters::Trait;
    for &GiveOrder { member, order } in requests.read() {
        if battle.is_some()
            && roster.contains(&member)
            && let Ok((mut current, character)) = orders.get_mut(member)
            // Only those with the trait go berserk.
            && (order != Order::Berserk || character.has_trait(Trait::Berserk))
        {
            current.set_if_neq(order);
        }
    }
}

/// Choose, between rounds, whether the party moves in the next ones.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct CyclePartyMove;

/// The party's move this round.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum PartyMove {
    #[default]
    None,
    /// Waiting for its phase; members do not act before it.
    Pending { pace: PartyPace, phase: u8 },
    /// Under way: metres allowed and covered (retail `GetPartyMovementSpeed`:
    /// 10000 units walking, 15000 running).
    Moving { pace: PartyPace, budget: f32, moved: f32, last: Option<Vec3> },
}

/// Start the next round (Enter, or the round button).
#[derive(Message, Debug, Clone, Copy)]
pub struct StartRound;

/// Player intent; gameplay interprets the round button using the current battle.
#[derive(Message, Clone, Copy, Debug)]
pub enum CombatControl {
    RoundButton,
    ToggleContinuous,
}

fn control_combat(
    mut requests: MessageReader<CombatControl>,
    battle: Option<Single<&Battle>>,
    mut settings: ResMut<CombatSettings>,
    mut rounds: MessageWriter<StartRound>,
) {
    for request in requests.read() {
        match request {
            CombatControl::ToggleContinuous => settings.continuous = !settings.continuous,
            CombatControl::RoundButton => {
                if let Some(battle) = &battle {
                    if battle.executing() {
                        settings.continuous = !settings.continuous;
                    } else {
                        rounds.write(StartRound);
                    }
                }
            }
        }
    }
}

/// When an actor acts this round, and how many attacks it has left.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Turn {
    /// 1 (first) to 100; 0 when done for the round.
    phase: u8,
    attacks_left: u32,
}

/// Effective round work identifies the actor and weapon, never a roster slot.
/// The action starts from the accepted order; execution may replace a refused
/// spell with Defend. Weapon identity, initiative and attack count stay locked.
/// Execution revalidates membership, ability, weapon presence and current reach;
/// damage profiles and fatigue use the actor's live stats and accepted weapon.
#[derive(Clone, Copy, Debug)]
enum RoundAction {
    /// The weapons in both hands; a Berserk order is an attack too.
    Attack {
        weapons: [Option<StackId>; 2],
        berserk: bool,
    },
    Defend,
    /// A spell or item use, revalidated and paid when its initiative comes.
    Magic(MagicChoice),
}

#[derive(Clone, Copy, Debug)]
struct PartyTurn {
    actor: Entity,
    action: RoundAction,
    /// When the member acts next, 1 to 100; 0 when done for the round.
    phase: u8,
    /// What an attack has left; other actions act once.
    hands: HandTurns,
}

impl PartyTurn {
    /// Done for the round: nothing left to act with.
    fn end(&mut self) {
        self.phase = 0;
        self.hands.left = [0, 0];
    }
}

/// A member's attacks left in each hand this round and the hand due next
/// (retail `hand_attack_values`, `current_hand`): the one authority on
/// what an attack has left.
#[derive(Clone, Copy, Debug, Default)]
struct HandTurns {
    left: [u32; 2],
    current: usize,
    /// A Samurai's Lightning Strike, readied for the main hand's next
    /// attack (retail `extra_swings`).
    lightning: bool,
}

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

/// One round: its committed work, fixed when the round is accepted (who
/// acts, with what, when and how often), and what has happened in it so
/// far. Execution revalidates each actor (still a member or combatant, able,
/// the same weapon) but never redirects work to whoever holds its slot now.
/// Each round starts a new one.
#[derive(Debug, Default)]
struct Round {
    party: Vec<PartyTurn>,
    monsters: Vec<(Entity, Turn)>,
    /// Someone has attacked.
    attacked: bool,
    /// Monsters that have moved: one move each.
    moved: Vec<Entity>,
    /// Monsters that made their special attack.
    specials: Vec<Entity>,
    /// Monsters insanity turned on their friends this round.
    berserk: Vec<Entity>,
    /// Monsters guarding a fellow this round.
    guards: Vec<Guard>,
    /// Monsters that waited this round, which rests them.
    waited: Vec<Entity>,
    /// How the party moves this round.
    pace: Option<PartyPace>,
    /// How often each member has tried to spot an attacker this round.
    spotted: bevy::ecs::entity::EntityHashMap<u32>,
}

/// A monster guarding a fellow this round, and how many blows it has
/// taken in its place (retail `ACTION_PROTECT`, `interception_count`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Guard {
    guard: Entity,
    ward: Entity,
    interceptions: u32,
}

/// The party's fight, on an entity owned by its level. Exists only while
/// there is one.
#[derive(Component, Debug, Default)]
pub struct Battle {
    pub round: u32,
    phase: RoundPhase,
    this_round: Round,
    /// Experience from kills, shared at the end.
    experience: u32,
    kills: u32,
    passive_rounds: u32,
    /// The party's standing choice to walk or run in the next rounds.
    pub party_pace: Option<PartyPace>,
    party_move: PartyMove,
    /// Monsters running for their lives (retail `pCombat->advancing`).
    fleeing: Vec<Entity>,
    /// Rounds each monster still waits before its next special attack
    /// (retail `pCombat->special_cooldown`).
    cooldowns: Vec<(Entity, u8)>,
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
    Executing(u8),
}

impl Battle {
    /// A round is accepted or running, including its pacing delay.
    pub fn executing(&self) -> bool {
        self.phase != RoundPhase::Waiting
    }

    /// The effective action retained until the next round or battle removal.
    /// Spending or losing a turn does not end defense; a refused spell starts it.
    pub fn defends(&self, actor: Entity) -> bool {
        self.this_round
            .party
            .iter()
            .any(|work| work.actor == actor && matches!(work.action, RoundAction::Defend))
    }

    /// Whether the party may walk: in a fight, only during its own move.
    pub fn party_may_move(&self) -> bool {
        matches!(self.party_move, PartyMove::Moving { .. })
    }

    /// Whether the party's move is a run.
    pub fn party_running(&self) -> bool {
        matches!(self.party_move, PartyMove::Moving { pace: PartyPace::Run, .. })
    }

    /// A monster died to the party: its experience is shared at the end and
    /// it acts no more this round.
    pub fn credit_kill(
        &mut self,
        monster: Entity,
        experience: u32,
        news: &mut MessageWriter<CombatNews>,
    ) {
        self.experience += experience;
        self.kills += 1;
        if let Some((_, turn)) =
            self.this_round.monsters.iter_mut().find(|(actor, _)| *actor == monster)
        {
            *turn = Turn::default();
        }
        news.write(CombatNews::Killed { actor: monster });
    }

    /// Credit and announce the kills a spell's effects made, whether it
    /// landed at once or as a missile: the one place spell kills are counted.
    /// A member it killed is announced as one a monster's blow kills is.
    pub fn credit_kills(&mut self, effects: &[Effect], news: &mut MessageWriter<CombatNews>) {
        for effect in effects {
            let Effect::Damaged { victim, outcome, .. } = *effect else { continue };
            if !outcome.died {
                continue;
            }
            match victim {
                Victim::Monster(target) => self.credit_kill(target, outcome.experience, news),
                Victim::Member(member) => {
                    news.write(CombatNews::Killed { actor: member });
                }
            }
        }
    }

    /// How far through the round its play is, 0 to 1, while a round is
    /// being played.
    pub fn progress(&self) -> Option<f32> {
        match self.phase {
            RoundPhase::Waiting => None,
            RoundPhase::Accepted => Some(0.0),
            RoundPhase::Executing(step) => Some(f32::from(step.min(100)) / 100.0),
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

/// What the fight tells the player.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum CombatNews {
    Began,
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
        victim: Victim,
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

/// How far a monster is from the party (retail `GetPointDistanceToPlayer`):
/// from its feet to the point 1000 world units (2 m) under the party's eye,
/// less both bodies' alternate radii (its fight radius, the party's 1 m),
/// never below 0. Reach and sight compare against this.
fn distance_to_party(party_eye: Vec3, monster: &Monster, feet: Vec3) -> f32 {
    let party_point = party_eye - Vec3::Y * CAMERA_ABOVE_PARTY_POINT;
    ((feet - party_point).length() - monster.body.radius - PARTY_ALTERNATE_RADIUS).max(0.0)
}

/// Retail `ComputeSightThreshold`: how far an observer facing away by
/// `angle` (radians) sees, given its senses, a ranged bonus percent and the
/// penalty source (the party's lowest level, doubled); within 45 degrees, or
/// at no distance, the whole `far` clip.
fn sight_threshold(
    angle: f32,
    senses: i32,
    bonus: u32,
    penalty: i32,
    far: f32,
    distance: f32,
) -> f32 {
    let mut percent = if angle < FACING_TOLERANCE || distance == 0.0 {
        100
    } else {
        ((senses - penalty * 2) as f32 * ((far - distance) / distance)) as i32
    };
    if bonus != 0 {
        percent = (bonus as i32 + 100) * percent / 100;
    }
    percent.clamp(2, 100) as f32 * far * 0.01
}

/// What monsters see of the party (after retail `UpdateMonsterSight`).
struct Sight<'a, 'w, 's> {
    eye: Vec3,
    far: f32,
    /// The party's lowest level among those who can act, which weakens
    /// sight from behind.
    lowest_level: i32,
    obstacles: &'a WorldObstacles<'w, 's>,
}

impl Sight<'_, '_, '_> {
    /// Whether a monster that can still fight sees the party: within the
    /// far clip, a clear line from its eyes past the level's static
    /// geometry, and its sight threshold covering the distance.
    fn sees(
        &self,
        monster: &Monster,
        at: &ActorPoseItem,
        walker: Option<&Walker>,
        kind: &Species,
    ) -> bool {
        let distance = distance_to_party(self.eye, monster, feet(at.translation(), walker));
        self.sees_point((monster, at, walker, kind), self.eye, distance, self.lowest_level)
    }

    /// Whether the party has a clear line to a monster (retail
    /// `GetPlayerToMonsterSightFlags`): within the far clip, nothing solid
    /// between the party's eye and the monster's. This is the party's line
    /// of sight, whichever way the monster faces and however keen it is.
    fn in_view(&self, monster: &Monster, at: &ActorPoseItem, walker: Option<&Walker>) -> bool {
        let feet = feet(at.translation(), walker);
        let eye = feet + Vec3::Y * monster.body.eye_height;
        distance_to_party(self.eye, monster, feet) <= self.far
            && Dir3::new(eye - self.eye).map_or(true, |direction| {
                self.obstacles.first_hit(self.eye, direction, self.eye.distance(eye)).is_none()
            })
    }

    /// Whether a monster sees another (retail `CanMonsterSeeMonster`):
    /// the same rule, weakened from behind by the other's level.
    fn sees_monster(
        &self,
        observer: (&Monster, &ActorPoseItem, Option<&Walker>, &Species),
        target: (&Monster, &ActorPoseItem, Option<&Walker>, &Species),
    ) -> bool {
        let (body, at, walker, kind) = target;
        let (from, ..) = observer;
        let target_feet = feet(at.translation(), walker);
        let observer_feet = feet(observer.1.translation(), observer.2);
        let distance =
            (observer_feet.distance(target_feet) - from.body.radius - body.body.radius).max(0.0);
        let eye = target_feet + Vec3::Y * body.body.eye_height;
        self.sees_point(observer, eye, distance, i32::from(kind.level))
    }

    fn sees_point(
        &self,
        (monster, at, walker, kind): (&Monster, &ActorPoseItem, Option<&Walker>, &Species),
        target: Vec3,
        distance: f32,
        penalty: i32,
    ) -> bool {
        let eye = feet(at.translation(), walker) + Vec3::Y * monster.body.eye_height;
        if !monster.can_fight() || distance > self.far {
            return false;
        }
        let clear = Dir3::new(target - eye).map_or(true, |direction| {
            self.obstacles.first_hit(eye, direction, eye.distance(target)).is_none()
        });
        let forward = at.forward().with_y(0.0);
        let angle = forward.angle_between((target - eye).with_y(0.0));
        let senses = i32::from(kind.attributes.senses);
        let bonus = if kind.level_sight { (u32::from(kind.level) * 5).min(125) } else { 0 };
        clear && sight_threshold(angle, senses, bonus, penalty, self.far, distance) >= distance
    }
}

/// How far from the party a neutral member may stand and still reinforce a
/// fight (retail `GetMonsterEngagementRange`, 25000 world units).
const ENGAGEMENT_RANGE: f32 = 50.0;
/// What a group's leader reaches at least, beyond the engagement range, to
/// join a running fight (retail `g_monster_engagement_range_floor`).
const ENGAGEMENT_FLOOR: f32 = 15.0;
/// How far out of its way a neutral member may stand from a hostile's line
/// to the party and still be between them (retail
/// `g_reinforcement_distance_slack`).
const REINFORCEMENT_SLACK: f32 = 1.15;

/// A monster as the reinforcement and leaving rules read it: where its
/// feet are, how far it is from the party, whether it sees the party
/// ([`Sight::sees`]) and whether the party has a line to it
/// ([`Sight::in_view`]).
#[derive(Clone, Copy, Debug)]
struct Seen {
    entity: Entity,
    at: Vec3,
    from_party: f32,
    sees: bool,
    in_view: bool,
}

/// Whether a neutral group stands with a fight (retail
/// `MonsterGroupHasReinforcement`): one of its members in the party's view,
/// within the engagement range, stands roughly between the party and a
/// hostile fighter also in the party's view ([`Seen::in_view`], retail
/// `party_threat.los_flags[1]`). A camouflaged member does not count.
fn reinforces(members: &[(Seen, bool)], hostiles: &[Seen]) -> bool {
    members
        .iter()
        .filter(|(member, camouflaged)| {
            !camouflaged && member.in_view && member.from_party <= ENGAGEMENT_RANGE
        })
        .any(|(member, _)| {
            hostiles.iter().filter(|other| other.entity != member.entity && other.in_view).any(
                |other| {
                    let between = member.at.distance(other.at);
                    between + member.from_party < other.from_party * REINFORCEMENT_SLACK
                        && (member.from_party < other.from_party || between < other.from_party)
                },
            )
        })
}

/// The party's lowest level among those who can act.
fn lowest_level<'a>(party: impl Iterator<Item = (&'a Character, &'a Vitals)>) -> i32 {
    party.filter(|(_, v)| v.can_fight()).map(|(c, _)| i32::from(c.level)).min().unwrap_or(0)
}

/// Where the party stands and faces, and its formation, as reach needs them.
struct PartyView {
    eye: Vec3,
    yaw: f32,
    /// The level's far clip, which bounds monsters' sight.
    far: f32,
    /// The game time the round's actions happen at.
    now: u64,
}

impl PartyView {
    /// How monsters see the party now.
    fn sight<'a, 'w, 's>(
        &self,
        members: &Members,
        roster: &PartyRoster,
        obstacles: &'a WorldObstacles<'w, 's>,
    ) -> Sight<'a, 'w, 's> {
        Sight {
            eye: self.eye,
            far: self.far,
            lowest_level: lowest_level(
                members.iter_many(roster.iter()).matched().map(|(c, v, ..)| (c, v)),
            ),
            obstacles,
        }
    }
}

impl PartyView {
    fn reaches(
        &self,
        formation: &Formation,
        member: Entity,
        at: Vec3,
        reach: Reach,
        distance: f32,
    ) -> bool {
        let side = formation::quadrant(self.eye, self.yaw, at);
        formation.reach(member, side, reach).is_some_and(|r| distance <= r.metres())
    }
}

/// Start a fight when a hostile group can act on the party, and bring
/// groups into one already running (after retail `UpdateMonsterGroups`,
/// `MonsterGroupCanEngage`, `CheckMonsterGroupsEnterCombat`,
/// `ShouldMonsterGroupEnterCombat`, `AlertSameFactionGroups`,
/// `MakeTargetGroupHostile`).
///
/// Each group outside the fight, in authored order, takes its stance
/// ([`hostility::stance`]), turning hostile when the party comes within its
/// hostility radius in its sight. A hostile group enters with all its
/// living members when the party has just acted against it. Otherwise a
/// fight starts when a member sees the party ([`Sight`]) and a member can
/// act on it: one of its attacks reaches, or its best attack is close
/// (touch or short) and it can walk up. A fight under way takes in a
/// hostile group that sees the party with its leader in reach, and a
/// neutral group standing with it ([`reinforces`]); and while it runs, a
/// hostile group fighting turns hostile the other groups of its faction
/// whose leader sees its leader. NPC groups keep to their roles. Retail's
/// path-length check is taken as the straight distance; its rendered-member
/// clause and the party's own surprise rolls are not modelled yet.
pub fn start_battle(
    mut commands: Commands,
    battles: Query<Entity, With<Battle>>,
    camp: Option<Res<crate::game::camp::Camp>>,
    mut party: Query<(&Position, &mut PartyBody)>,
    level: PlayedLevel,
    members: Query<(&Character, &Vitals)>,
    monsters: Query<(
        Entity,
        &Monster,
        ActorPose,
        Option<&Walker>,
        Option<&GroupMember>,
        Has<Combatant>,
    )>,
    mut provocations: MessageReader<hostility::Provocation>,
    mut stage: ParamSet<(
        (
            Query<(Entity, &mut MonsterGroup, &GroupMembers, &LevelOwned)>,
            Res<crate::game::factions::Factions>,
        ),
        GroupRecognition,
    )>,
    clock: Res<GameClock>,
    obstacles: WorldObstacles,
    content: crate::game::data::Content,
    roster: Single<&PartyRoster>,
    mut news: MessageWriter<CombatNews>,
    mut rng: ResMut<GameRng>,
) {
    let provoked: Vec<Entity> = provocations
        .read()
        .filter_map(|&hostility::Provocation(m)| Some(monsters.get(m).ok()?.4?.0))
        .collect();
    let running = battles.single().ok();
    if running.is_none()
        && (camp.is_some()
            || !members.iter_many(roster.iter()).matched().any(|(_, v)| !v.is_dead()))
    {
        return;
    }
    let Some(catalog) = content.species() else { return };
    let Ok((party_at, mut body)) = party.single_mut() else { return };
    let (Some(far), Some(root)) = (level.view_distance(), level.entity()) else { return };
    let sight = Sight {
        eye: PartyBody::eye(party_at),
        far,
        lowest_level: lowest_level(members.iter_many(roster.iter()).matched()),
        obstacles: &obstacles,
    };
    // Those of a group that fight: alive, and not an NPC.
    let fighters = |group: &GroupMembers| -> Vec<Entity> {
        group
            .iter()
            .filter(|&member| {
                monsters.get(member).is_ok_and(|(_, monster, ..)| {
                    !monster.is_dead()
                        && catalog.kind(monster.species).is_some_and(|k| k.npc.is_none())
                })
            })
            .collect()
    };
    let spells = content.spells();
    // How far a fighter is, whether it sees the party, and whether it can
    // act on it (retail `MonsterGroupCanEngage`): one of its attacks
    // reaches, or its best attack is close and it can walk up, or it can
    // cast or breathe at the party.
    let look = |fighter: Entity| -> Option<(f32, bool, bool)> {
        let (_, monster, at, walker, ..) = monsters.get(fighter).ok()?;
        let kind = catalog.kind(monster.species)?;
        let at_feet = feet(at.translation(), walker);
        let distance = distance_to_party(sight.eye, monster, at_feet);
        let sees = sight.sees(monster, &at, walker, kind);
        let reaches = kind.attacks.iter().any(|a| distance <= a.reach.metres());
        let close = kind.attacks.iter().map(|a| a.reach).max().is_some_and(|r| r <= Reach::Short);
        let party_at = sight.eye - Vec3::Y * CAMERA_ABOVE_PARTY_POINT;
        let toward = |victim: crate::game::magic::effects::Victim| {
            let turncoat =
                members.get(victim.entity()).is_ok_and(|(_, v)| v.has(Condition::Turncoat));
            hostility::toward_member(hostility::Stance::Hostile, turncoat)
        };
        let scene = monster_spells::Scene {
            caster: fighter,
            from: at_feet,
            party_at,
            members: roster
                .iter()
                .filter_map(|m| members.get(m).ok().map(|(c, v)| (m, c, v)))
                .filter(|(_, _, v)| !v.is_dead())
                .map(|(m, c, v)| member_in_scene((m, c, v), (party_at, distance), &toward))
                .collect(),
            monsters: Vec::new(),
            party: distance,
            sees_party: sees,
        };
        let casts = spells
            .is_some_and(|spells| monster_spells::casts_at_foes(monster, kind, spells, &scene));
        let breathes = kind.special.chance != 0
            && special_ready(monster, kind, false)
            && spells
                .and_then(|spells| spells.get(crate::content::spells::SpellId::SPECIAL_ATTACK_CONE))
                .is_some_and(|cone| !monster_spells::aims(cone, &scene).is_empty());
        let acts = monster.can_fight()
            && (reaches || (close && monster.body.speed > 0.0) || casts || breathes);
        Some((distance, sees, acts))
    };
    let seen = |member: Entity| -> Option<Seen> {
        let (_, monster, at, walker, ..) = monsters.get(member).ok()?;
        let kind = catalog.kind(monster.species)?;
        let at_feet = feet(at.translation(), walker);
        Some(Seen {
            entity: member,
            at: at_feet,
            from_party: distance_to_party(sight.eye, monster, at_feet),
            sees: sight.sees(monster, &at, walker, kind),
            in_view: sight.in_view(monster, &at, walker),
        })
    };
    let now = clock.ms;
    // How far a monster is from the party.
    let nearest_of = |member: Entity| {
        monsters.get(member).map_or(f32::INFINITY, |(_, monster, at, walker, ..)| {
            distance_to_party(sight.eye, monster, feet(at.translation(), walker))
        })
    };
    // Whether a group's leader (its first fighter) is in reach of the
    // party: its best attack's
    // reach and one combat move, never under the engagement range and its
    // floor (retail `ShouldMonsterGroupEnterCombat`; retail's path length
    // check is taken as the straight distance).
    let leader_reaches = |kind: &Species, distance: f32| {
        let best = kind.attacks.iter().map(|a| a.reach.metres()).fold(0.0, f32::max);
        let reach = (best + kind.combat_move_range * approach::METRES_PER_MOVE_RANGE)
            .max(ENGAGEMENT_RANGE + ENGAGEMENT_FLOOR);
        distance <= reach
    };
    // The fight's hostile fighters in the party's sight, as reinforcement
    // reads them, and its same-faction groups alerted (retail
    // `CheckMonsterGroupsEnterCombat`, `AlertSameFactionGroups`).
    let mut hostiles = Vec::new();
    if running.is_some() {
        let (mut groups, factions) = stage.p0();
        let stands = |monster: Entity, group: Option<&MonsterGroup>| -> Option<hostility::Stance> {
            let (_, body, ..) = monsters.get(monster).ok()?;
            let kind = catalog.kind(body.species)?;
            Some(hostility::stance(group.and_then(|g| g.set), kind, &factions, now, true))
        };
        let living = |membership: &GroupMembers| -> Option<Entity> {
            membership
                .iter()
                .find(|&m| monsters.get(m).is_ok_and(|(_, monster, ..)| !monster.is_dead()))
        };
        for (entity, monster, .., group, fighting) in &monsters {
            let membership = group.and_then(|g| groups.get(g.0).ok());
            if fighting
                && !monster.is_dead()
                && stands(entity, membership.map(|(_, g, ..)| g))
                    == Some(hostility::Stance::Hostile)
                && let Some(seen) = seen(entity).filter(|s| s.in_view)
            {
                hostiles.push(seen);
            }
        }
        // Fighting hostile groups by their leaders, and their factions.
        let alerting: Vec<(Entity, i32, Entity)> = groups
            .iter()
            .filter_map(|(entity, group, membership, owner)| {
                let leader = living(membership).filter(|_| owner.0 == root)?;
                let (_, body, .., fighting) = monsters.get(leader).ok()?;
                let kind = catalog.kind(body.species)?;
                (fighting
                    && kind.faction != crate::game::factions::UNALIGNED
                    && stands(leader, Some(group)) == Some(hostility::Stance::Hostile))
                .then_some((entity, kind.faction, leader))
            })
            .collect();
        let view = |monster: Entity| {
            monsters.get(monster).ok().and_then(|(_, body, at, walker, ..)| {
                Some((body, at, walker, catalog.kind(body.species)?))
            })
        };
        for (entity, mut group, membership, owner) in &mut groups {
            let Some(leader) = living(membership).filter(|_| owner.0 == root) else { continue };
            let Some(observer) = view(leader) else { continue };
            // NPCs keep to their own roles here.
            if observer.3.npc.is_some() {
                continue;
            }
            let alerted = alerting.iter().any(|&(source, faction, target)| {
                source != entity
                    && faction == observer.3.faction
                    && view(target).is_some_and(|target| {
                        sight.sees_monster(
                            (observer.0, &observer.1, observer.2, observer.3),
                            (target.0, &target.1, target.2, target.3),
                        )
                    })
            });
            let fighting = monsters.get(leader).is_ok_and(|m| m.5);
            if alerted
                && hostility::stance(group.set, observer.3, &factions, now, fighting)
                    != hostility::Stance::Hostile
            {
                group.set =
                    Some(hostility::SetStance { stance: hostility::Stance::Hostile, at: now });
            }
        }
    }
    // Each group outside the fight in authored order, then any monster
    // without one.
    let mut units: Vec<(i32, Option<Entity>, Vec<Entity>)> = {
        let (groups, _) = stage.p0();
        groups
            .iter()
            .filter(|(.., owner)| owner.0 == root)
            .map(|(entity, group, membership, _)| {
                (group.authored, Some(entity), fighters(membership))
            })
            .collect()
    };
    units.sort_unstable_by_key(|&(authored, ..)| authored);
    units.extend(
        monsters
            .iter()
            .filter(|(_, monster, .., group, _)| {
                group.is_none()
                    && !monster.is_dead()
                    && catalog.kind(monster.species).is_some_and(|k| k.npc.is_none())
            })
            .map(|(entity, ..)| (i32::MAX, None, vec![entity])),
    );
    let mut joined: Vec<(i32, Entity)> = Vec::new();
    for (_, group, fighters) in units {
        // Those already fighting stay; the rest of their group may join them.
        let fighters: Vec<Entity> =
            fighters.into_iter().filter(|&m| monsters.get(m).is_ok_and(|m| !m.5)).collect();
        let Some(&first) = fighters.first() else { continue };
        let kind = catalog.kind(monsters.get(first).unwrap().1.species).unwrap();
        let looks: Vec<_> = fighters.iter().filter_map(|&m| look(m)).collect();
        let nearest = looks.iter().map(|l| l.0).fold(f32::INFINITY, f32::min);
        let sees = looks.iter().any(|l| l.1);
        let acts = looks.iter().any(|l| l.2);
        let (mut groups, factions) = stage.p0();
        let mut stance = match group.and_then(|g| groups.get(g).ok()) {
            Some((_, group, ..)) => hostility::stance(group.set, kind, &factions, now, false),
            None => hostility::default_stance(kind, &factions),
        };
        // Coming within a hostility radius in the group's sight turns it.
        if stance != hostility::Stance::Hostile && sees && hostility::within_radius(kind, nearest) {
            stance = hostility::Stance::Hostile;
            if let Some(Ok((_, mut group, ..))) = group.map(|g| groups.get_mut(g)) {
                group.set = Some(hostility::SetStance { stance, at: now });
            }
        }
        let provoked = group.is_some_and(|g| provoked.contains(&g));
        let enters = match stance {
            // A fight starts when a hostile group can act on the party
            // (retail `MonsterGroupCanEngage`); one under way takes in a
            // hostile group that sees the party with its leader in reach
            // (`ShouldMonsterGroupEnterCombat`).
            hostility::Stance::Hostile if running.is_some() => {
                provoked || (sees && leader_reaches(kind, nearest_of(first)))
            }
            hostility::Stance::Hostile => provoked || (sees && acts),
            // A neutral group joins a running fight it stands with.
            hostility::Stance::Neutral if running.is_some() => {
                let members: Vec<(Seen, bool)> = fighters
                    .iter()
                    .filter_map(|&m| seen(m))
                    .map(|s| (s, kind.temper.camouflaged))
                    .collect();
                reinforces(&members, &hostiles)
            }
            _ => false,
        };
        if !enters {
            continue;
        }
        // An unaligned species with a hostility radius turns hostile as it
        // enters.
        if stance == hostility::Stance::Neutral
            && kind.npc.is_none()
            && kind.faction == crate::game::factions::UNALIGNED
            && matches!(kind.disposition, crate::content::species::Disposition::HostileWithin(_))
            && let Some(Ok((_, mut group, ..))) = group.map(|g| groups.get_mut(g))
        {
            group.set = Some(hostility::SetStance { stance: hostility::Stance::Hostile, at: now });
        }
        joined.extend(fighters.iter().map(|&m| (monsters.get(m).unwrap().1.location, m)));
    }
    if joined.is_empty() {
        return;
    }
    joined.sort();
    let joined: Vec<Entity> = joined.into_iter().map(|(_, e)| e).collect();
    if let Some(battle) = running {
        for &monster in &joined {
            commands.entity(monster).insert(Combatant(battle));
        }
        return;
    }
    info!("combat: {} monsters", joined.len());
    // The view turns to the nearest of them (retail `beginCombat`).
    let nearest = monsters
        .iter_many(&joined)
        .matched()
        .map(|(_, _, at, ..)| at.translation())
        .min_by(|a, b| a.distance(sight.eye).total_cmp(&b.distance(sight.eye)));
    if let Some(nearest) = nearest {
        body.face(nearest - sight.eye);
    }
    let battle = commands.spawn((Name::new("Battle"), Battle::default(), LevelOwned(root))).id();
    for &monster in &joined {
        commands.entity(monster).insert(Combatant(battle));
    }
    stage.p1().detect(sight.eye, body.forward(), Some(&joined), &mut rng);
    news.write(CombatNews::Began);
}

/// Whether the party is fighting, for operations that hold the whole world.
pub fn in_battle(world: &mut World) -> bool {
    world.query_filtered::<(), With<Battle>>().iter(world).next().is_some()
}

/// Terminal state is settled independently of whose action dealt the damage.
/// Used before/after combat execution and at the end of environmental damage.
pub fn settle_defeat(
    mut commands: Commands,
    battle: Option<Single<Entity, With<Battle>>>,
    roster: Single<&PartyRoster>,
    members: Query<&Vitals>,
    mut news: MessageWriter<CombatNews>,
    mut effects: Single<&mut crate::game::magic::enchantments::PartyEffects>,
    mut monsters: Query<&mut Monster>,
) {
    let Some(battle) = battle.map(Single::into_inner) else { return };
    if !members.iter_many(roster.iter()).matched().any(|v| !v.is_dead()) {
        effects.0.end_combat();
        for mut monster in &mut monsters {
            monster.enchantments.end_combat();
        }
        news.write(CombatNews::Lost);
        // Its combatants leave the fight and stop closing in; they remain.
        commands.entity(battle).despawn();
    }
}

/// How each monster in the fight stands, through its group (one outside
/// any group by its species), as this update of the fight begins.
fn fighting_stances(
    combatants: &[Entity],
    stances: &hostility::Stances,
    now: u64,
    species: &MonsterCatalog,
) -> Vec<(Entity, hostility::Stance)> {
    combatants
        .iter()
        .filter_map(|&monster| Some((monster, stances.group(monster, species, now)?)))
        .collect()
}

/// Groups with nothing left to fight leave it as the round ends (retail
/// `CheckMonsterGroupsLeaveCombat`): a neutral group once it no longer
/// stands with the fight ([`reinforces`]); any other unless it is hostile
/// and one of its living members still sees the party. When no hostile
/// group is left engaged, everyone leaves. Returns the combatants that
/// stay; one outside any group stays while hostile groups fight on.
fn leave_fight(
    commands: &mut Commands,
    combatants: &[Entity],
    monsters: &MonsterQuery,
    (membership, stances): (&Query<&GroupMember>, &[(Entity, hostility::Stance)]),
    sight: &Sight,
    species: &MonsterCatalog,
) -> Vec<Entity> {
    use hostility::Stance;
    let group_of = |monster: Entity| membership.get(monster).ok().map(|m| m.0);
    let stance_of =
        |monster: Entity| stances.iter().find(|&&(m, _)| m == monster).map(|&(_, stance)| stance);
    let seen = |monster: Entity| -> Option<(Seen, bool)> {
        let (body, at, walker, _) = monsters.get(monster).ok()?;
        let kind = species.kind(body.species)?;
        if body.is_dead() {
            return None;
        }
        let at_feet = feet(at.translation(), walker);
        let seen = Seen {
            entity: monster,
            at: at_feet,
            from_party: distance_to_party(sight.eye, body, at_feet),
            sees: sight.sees(body, &at, walker, kind),
            in_view: sight.in_view(body, &at, walker),
        };
        Some((seen, kind.temper.camouflaged))
    };
    let hostiles: Vec<Seen> = combatants
        .iter()
        .filter(|&&c| stance_of(c) == Some(Stance::Hostile))
        .filter_map(|&c| seen(c).map(|(s, _)| s))
        .filter(|s| s.in_view)
        .collect();
    let (mut checked, mut leaving, mut engaged) = (Vec::new(), Vec::new(), false);
    for &combatant in combatants {
        let Some(group) = group_of(combatant) else {
            engaged |= stance_of(combatant) == Some(Stance::Hostile);
            continue;
        };
        if checked.contains(&group) {
            continue;
        }
        checked.push(group);
        let fellows: Vec<(Seen, bool)> = combatants
            .iter()
            .filter(|&&c| group_of(c) == Some(group))
            .filter_map(|&c| seen(c))
            .collect();
        let stays = match stance_of(combatant) {
            Some(Stance::Neutral) => reinforces(&fellows, &hostiles),
            Some(Stance::Hostile) => {
                let sees = fellows.iter().any(|(s, _)| s.sees);
                engaged |= sees;
                sees
            }
            _ => false,
        };
        if !stays {
            leaving.push(group);
        }
    }
    combatants
        .iter()
        .copied()
        .filter(|&combatant| {
            let leaves = !engaged || group_of(combatant).is_some_and(|g| leaving.contains(&g));
            if leaves {
                commands.entity(combatant).remove::<Combatant>();
            }
            !leaves
        })
        .collect()
}

/// Accept a round (retail `BeginCombatExecution`): lock each member's order
/// and weapon, and everyone's initiative and attacks, the party first, then
/// the combatants in order. Pacing delays only its execution.
fn accept_round(
    battle: &mut Battle,
    combatants: &[Entity],
    roster: &PartyRoster,
    members: &Members,
    orders: &Query<&Order>,
    monsters: &MonsterQuery,
    (items, species): (&ItemCatalog, &MonsterCatalog),
    rng: &mut GameRng,
) {
    use crate::content::characters::{Skill, Trait};
    battle.round += 1;
    let mut round = Round { pace: battle.party_pace, ..default() };
    // Samurai ready their Lightning Strike with a sword before anyone's
    // initiative is rolled (retail `BeginCombatExecution`).
    let lightning: Vec<bool> = roster
        .iter()
        .map(|member| {
            let Ok((character, vitals, carried, burden)) = members.get(member) else {
                return false;
            };
            let stats = character.stats(&vitals.enchantments);
            orders.get(member).copied().unwrap_or_default() == Order::Attack
                && character.has_trait(Trait::LightningStrike)
                && attack::hands(&stats, carried, items, burden.load).hands[0].weapon_skill
                    == Skill::Sword
                && rng.below(100) < u32::from(character.scale_by_profession(12.0) as u8)
        })
        .collect();
    for (member, lightning) in roster.iter().zip(lightning) {
        let Ok((character, vitals, carried, burden)) = members.get(member) else { continue };
        let stats = character.stats(&vitals.enchantments);
        let hands = attack::hands(&stats, carried, items, burden.load);
        let weapons =
            [EquipSlot::Primary, EquipSlot::Secondary].map(|slot| carried.worn(slot).map(|i| i.id));
        let action = match orders.get(member).copied().unwrap_or_default() {
            Order::Attack => RoundAction::Attack { weapons, berserk: false },
            // One who can no longer berserk attacks as usual.
            Order::Berserk => {
                RoundAction::Attack { weapons, berserk: blows::can_berserk(character, &hands) }
            }
            Order::Defend => RoundAction::Defend,
            Order::Magic(choice) => RoundAction::Magic(choice),
        };
        let mut turns = HandTurns { lightning, ..default() };
        let phase = match action {
            // The hand with the most attacks opens, its weapon's initiative
            // counting (retail `AssignCombatPhases`).
            RoundAction::Attack { .. } if vitals.can_fight() => match hands.choose() {
                Some(hand) => {
                    turns.current = hand;
                    turns.left =
                        hands.hands.each_ref().map(|h| if h.in_play { h.attacks } else { 0 });
                    let initiative = stats.initiative(burden.load) + hands.hands[hand].initiative;
                    phase(rng, initiative)
                }
                None => 0,
            },
            RoundAction::Magic(_) if vitals.can_fight() => {
                phase(rng, stats.initiative(burden.load))
            }
            _ => 0,
        };
        round.party.push(PartyTurn { actor: member, action, phase, hands: turns });
    }
    for &entity in combatants {
        let Ok((monster, ..)) = monsters.get(entity) else { continue };
        let Some(kind) = species.kind(monster.species) else { continue };
        let turn = if monster.is_dead() {
            Turn::default()
        } else {
            Turn {
                phase: phase(rng, i32::from(kind.initiative)),
                attacks_left: u32::from(kind.attacks_per_round),
            }
        };
        round.monsters.push((entity, turn));
    }
    // The party's own move takes the phase its slowest member allows,
    // on a step of ten (retail `InitializePartyMovementPhase`).
    battle.party_move = match battle.party_pace {
        Some(pace) => {
            let latest = roster
                .iter()
                .filter_map(|m| members.get(m).ok())
                .filter(|(_, vitals, ..)| vitals.can_fight())
                .map(|(c, _, _, burden)| phase(rng, c.initiative(burden.load)))
                .max()
                .unwrap_or(90);
            let phase = (latest + 5) - (latest + 5) % 10;
            PartyMove::Pending { pace, phase: phase.clamp(10, 100) }
        }
        None => PartyMove::None,
    };
    battle.this_round = round;
    battle.phase = RoundPhase::Accepted;
}

/// Start the round the player asks for, or in a continuous fight the next
/// one once the last action's pause is over and nothing is in flight
/// (retail `UpdateCombat`, `BeginCombatExecution`). Choosing the party's
/// pace between rounds is taken here too.
pub fn begin_round(
    time: Res<Time>,
    battle: Option<Single<(&mut Battle, Option<&Combatants>)>>,
    mut rng: ResMut<GameRng>,
    (mut members, damage): (Members, MonsterDamage),
    (roster, orders, settings): (Single<&PartyRoster>, Query<&Order>, Res<CombatSettings>),
    content: crate::game::data::Content,
    (in_flight, moving): (
        Query<(), With<casting::SpellInFlight>>,
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
        finish_party_move(&mut battle, &mut rng, &mut members);
        news.write(CombatNews::PartyStops);
    }
    // Content exists whenever a level is played.
    let (Some(items), Some(species)) = (content.items(), content.species()) else { return };
    if !damage.ready() {
        return;
    }
    let roster = &**roster;
    let accept = |battle: &mut Battle, rng: &mut GameRng| {
        accept_round(
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
    if start && !battle.executing() {
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

/// Carry out the round's actions, one per pause, once nothing is in flight.
pub fn run_battle(
    mut commands: Commands,
    battle: Option<Single<(Entity, &mut Battle, Option<&Combatants>)>>,
    mut rng: ResMut<GameRng>,
    mut clock: ResMut<GameClock>,
    mut actors: ParamSet<((Members, MonsterDamage), GroupRecognition, hostility::Stances)>,
    party_body: Query<(&Position, &PartyBody)>,
    (roster, content, level): (Single<&PartyRoster>, crate::game::data::Content, PlayedLevel),
    mut magic: BattleMagic,
    (mut news, mut summons): (MessageWriter<CombatNews>, MessageWriter<summon::Summon>),
) {
    let Some((entity, mut battle, combatants)) = battle.map(Single::into_inner) else { return };
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    // Content and the party's body exist whenever a level is played.
    let Some(species) = content.species() else { return };
    let stances = { fighting_stances(combatants, &actors.p2(), clock.ms, species) };
    let (mut members, mut damage) = actors.p0();
    let Ok((position, body)) = party_body.single() else { return };
    if !damage.ready() || !battle.executing() || !battle.wait.is_finished() {
        return;
    }
    if !magic.in_flight.is_empty() || !magic.moving.is_empty() {
        return;
    }
    let roster = &**roster;
    let party = PartyView {
        eye: PartyBody::eye(position),
        yaw: body.yaw,
        far: level.view_distance().unwrap_or(0.0),
        now: clock.ms,
    };
    if battle.phase == RoundPhase::Accepted {
        battle.phase = RoundPhase::Executing(1);
    }

    // The next actor due: accepted party order first, then monsters in order.
    while let RoundPhase::Executing(step) = battle.phase {
        if matches!(battle.party_move, PartyMove::Moving { .. }) {
            return;
        }
        if let PartyMove::Pending { pace, phase } = battle.party_move
            && phase <= step
        {
            let haste = roster
                .iter()
                .try_fold(0, |sum, member| {
                    let (_, vitals, ..) = members.get(member).ok()?;
                    let effect = vitals.enchantments.get(crate::content::spells::SpellId::HASTE)?;
                    Some(sum + effect.scaled(u32::from(effect.power) * 10))
                })
                .map_or(0.0, |sum| (sum / roster.len().max(1) as u32) as f32 * 0.2);
            battle.party_move = PartyMove::Moving {
                pace,
                budget: pace.combat_budget() + haste,
                moved: 0.0,
                last: Some(position.0),
            };
            news.write(CombatNews::PartyMoves);
            return;
        }
        if step > 100 {
            let sight = party.sight(&members, roster, &magic.obstacles);
            let staying = leave_fight(
                &mut commands,
                combatants,
                damage.monsters(),
                (&magic.groups, &stances),
                &sight,
                species,
            );
            let over = end_round(
                &mut battle,
                combatants,
                &mut clock,
                &mut rng,
                roster,
                &mut members,
                &mut damage,
                party.eye,
                species,
                &mut news,
                &mut magic.points,
                &mut magic.context.enchantments.0,
                &staying,
            );
            if over {
                for &actor in combatants {
                    if let Some(mut e) = damage.enchantments_mut(actor) {
                        e.end_combat();
                    }
                }
            }
            // Recognition is a required part of this round, before the next
            // accepted action or the battle's end can change its meaning.
            actors.p1().detect(party.eye, body.forward(), Some(combatants), &mut rng);
            if over {
                magic.context.enchantments.0.end_combat();
                commands.entity(entity).despawn();
            }
            return;
        }
        // Nobody in the party acts while its move is still to come.
        let party_waits = matches!(battle.party_move, PartyMove::Pending { .. });
        if let Some(index) =
            battle.this_round.party.iter().position(|p| p.phase == step).filter(|_| !party_waits)
        {
            let acted = party_turn(
                &mut commands,
                &mut battle,
                index,
                combatants,
                &mut rng,
                (&mut members, &mut damage),
                &party,
                &mut magic,
                &mut news,
            );
            if acted {
                battle.wait = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
                return;
            }
            continue;
        }
        if let Some(index) = battle.this_round.monsters.iter().position(|(_, t)| t.phase == step) {
            let acted = monster_turn(
                &mut commands,
                &mut battle,
                (index, step),
                (combatants, &stances),
                &mut rng,
                (&mut members, &mut damage),
                &party,
                &mut magic,
                species,
                (&mut news, &mut summons),
            );
            if acted {
                battle.wait = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
                return;
            }
            continue;
        }
        battle.phase = RoundPhase::Executing(step + 1);
    }
}

/// The party's armour as this round's blows meet it: the spells on each
/// member and the party's, and whether they defend (retail
/// `CalcArmorClasses`).
fn party_armours(
    battle: &Battle,
    members: &Members,
    context: &MagicContext,
) -> Vec<(Entity, Armour)> {
    let Some(items) = context.content.items() else { return Vec::new() };
    context
        .roster
        .iter()
        .filter_map(|member| {
            let (character, vitals, carried, burden) = members.get(member).ok()?;
            let armour = attack::enchanted_armour(
                &character.stats(&vitals.enchantments),
                carried,
                items,
                burden.load,
                battle.defends(member),
                (&vitals.enchantments, Some(&context.enchantments.0)),
            );
            Some((member, armour))
        })
        .collect()
}

/// A member's accepted turn comes: revalidated, then their spell or attack.
/// Returns whether they acted; an actor who can no longer act loses the turn.
fn party_turn(
    commands: &mut Commands,
    battle: &mut Battle,
    index: usize,
    combatants: &[Entity],
    rng: &mut GameRng,
    (members, damage): (&mut Members, &mut MonsterDamage),
    party: &PartyView,
    magic: &mut BattleMagic,
    news: &mut MessageWriter<CombatNews>,
) -> bool {
    let work = battle.this_round.party[index];
    let member = work.actor;
    let able_now = magic.context.roster.contains(&member)
        && members.get(member).is_ok_and(|(_, vitals, carried, _)| {
            vitals.can_fight()
                && match work.action {
                    // The weapons accepted, or nothing where one was used up.
                    RoundAction::Attack { weapons, .. } => {
                        [EquipSlot::Primary, EquipSlot::Secondary].into_iter().zip(weapons).all(
                            |(slot, accepted)| {
                                let now = carried.worn(slot).map(|i| i.id);
                                now.is_none() || now == accepted
                            },
                        )
                    }
                    RoundAction::Magic(_) => true,
                    RoundAction::Defend => false,
                }
        });
    if !able_now {
        battle.this_round.party[index].end();
        return false;
    }
    if let RoundAction::Magic(choice) = work.action {
        battle.this_round.party[index].end();
        match member_casts(combatants, member, choice, rng, members, damage, party, magic) {
            Ok((report, flights)) => {
                battle.this_round.attacked = true;
                battle.credit_kills(&report.effects, news);
                let level = magic.context.level();
                magic.context.flight_order.launch(commands, flights, level);
                magic.context.news.write(MagicNews::Done(report));
            }
            // A refused action falls back to defending (retail
            // `ExecuteCharacterAction`), its turn spent.
            Err(refusal) => {
                info!("magic refused: {refusal:?}");
                battle.this_round.party[index].action = RoundAction::Defend;
                magic.context.news.write(MagicNews::Refused { caster: member, refusal });
            }
        }
        return true;
    }
    let BattleMagic { context, points, moving, .. } = magic;
    let advancing = |monster| moving.contains(monster);
    let armours = party_armours(battle, members, context);
    let armour = |member| armours.iter().find(|&&(m, _)| m == member).map(|&(_, a)| a);
    let mut fight = blows::Fight {
        combatants,
        party,
        armour: &armour,
        advancing: &advancing,
        points,
        attacks: &|_| false,
        waits: false,
    };
    let attacked = context.with_field(party.now, members, damage, |field| {
        blows::member_attacks(battle, index, rng, field, &mut fight, news);
    });
    if attacked.is_err() {
        battle.this_round.party[index].end();
    }
    attacked.is_ok()
}

/// Count the ground the party covers while it moves in a fight, and end the
/// move when it is spent.
pub fn track_party_move(
    battle: Option<Single<&mut Battle>>,
    party: Query<&Position, With<PartyBody>>,
    mut rng: ResMut<GameRng>,
    mut members: Members,
    mut news: MessageWriter<CombatNews>,
) {
    let Some(mut battle) = battle else { return };
    let Ok(position) = party.single() else { return };
    let spent = {
        let PartyMove::Moving { budget, moved, last, .. } = &mut battle.party_move else {
            return;
        };
        if let Some(last) = *last {
            *moved += last.with_y(0.0).distance(position.0.with_y(0.0));
        }
        *last = Some(position.0);
        *moved >= *budget
    };
    if spent {
        finish_party_move(&mut battle, &mut rng, &mut members);
        news.write(CombatNews::PartyStops);
    }
}

impl PartyPace {
    /// The metres a move covers in a fight (retail `GetPartyMovementSpeed`:
    /// 10000 units walking, 15000 running).
    fn combat_budget(self) -> f32 {
        match self {
            PartyPace::Walk => 20.0,
            PartyPace::Run => 30.0,
        }
    }

    /// Stamina a metre costs an unencumbered member in a fight: retail
    /// charges `distance × load × 2 (combat)` units at one stamina per 2500
    /// (`MGSPartyMovement.cpp:302-358`).
    fn stamina_per_metre(self) -> f32 {
        match self {
            PartyPace::Walk => 0.2,
            PartyPace::Run => 0.4,
        }
    }
}

/// The move is over (retail `CompletePartyMovementTurns`, `HandlePartyMovement`):
/// each member keeps their action with the chance the unspent share gives
/// (spent counting half again when running), and pays stamina for the
/// ground covered, by load, half again when diseased.
fn finish_party_move(battle: &mut Battle, rng: &mut GameRng, members: &mut Members) {
    let PartyMove::Moving { pace, budget, moved, .. } = battle.party_move else { return };
    let step = match battle.phase {
        RoundPhase::Executing(step) => step,
        _ => 1,
    };
    battle.party_move = PartyMove::None;
    let mut spent = (moved / budget.max(0.01) * 100.0).min(100.0);
    if pace == PartyPace::Run {
        spent *= 1.5;
    }
    let keep = 100u32.saturating_sub(spent as u32);
    for work in &mut battle.this_round.party {
        if work.phase == 0 {
            continue;
        }
        if rng.below(100) < keep {
            work.phase = work.phase.max(step);
        } else {
            work.end();
        }
    }
    for work in &battle.this_round.party {
        let Ok((_, mut vitals, _, burden)) = members.get_mut(work.actor) else { continue };
        if vitals.is_dead() {
            continue;
        }
        let disease = if vitals.has(Condition::Diseased) { 1.5 } else { 1.0 };
        let cost = moved * pace.stamina_per_metre() * burden.load.movement_factor() * disease;
        vitals.spend_stamina(cost.round() as u32);
    }
}

/// When the remaining attacks of a turn come: spread over the rest of the
/// round, or none.
fn next_phase(step: u8, attacks_left: u32) -> u8 {
    if attacks_left == 0 {
        return 0;
    }
    let next = u32::from(step) + (100 - u32::from(step)) / (attacks_left + 1);
    if next > 100 { 0 } else { next as u8 }
}

/// What casting in a fight needs beside the fight itself.
#[derive(bevy::ecs::system::SystemParam)]
pub struct BattleMagic<'w, 's> {
    context: MagicContext<'w, 's>,
    points: Query<'w, 's, (&'static LearnedSpells, &'static mut SpellPoints)>,
    obstacles: WorldObstacles<'w, 's>,
    groups: Query<'w, 's, &'static GroupMember>,
    group_members: Query<'w, 's, &'static GroupMembers>,
    /// Missiles still on their way hold the fight's next action, as do
    /// monsters still advancing.
    in_flight: Query<'w, 's, (), With<casting::SpellInFlight>>,
    moving: Query<'w, 's, (), With<approach::CombatMove>>,
}

/// A member casts or uses an item at their initiative. The monsters it may
/// aim at are decided now: in the fight, alive, within the spell's reach of
/// the party and in its sight (retail `CharacterActionReachesTarget`).
fn member_casts(
    combatants: &[Entity],
    member: Entity,
    choice: MagicChoice,
    rng: &mut GameRng,
    members: &mut Members,
    damage: &mut MonsterDamage,
    party: &PartyView,
    magic: &mut BattleMagic,
) -> Result<(casting::Report, Vec<casting::SpellInFlight>), casting::Refusal> {
    let BattleMagic { context, points, obstacles, groups, group_members, .. } = magic;
    let party_members: Vec<Entity> = context.roster.iter().collect();
    // Where each fighting monster in the party's sight stands, and how far.
    let in_sight: Vec<(casting::Candidate, Option<Entity>)> = combatants
        .iter()
        .filter_map(|&entity| {
            let (monster, at, walker, _) = damage.monsters().get(entity).ok()?;
            if monster.is_dead() || !damage.active(entity) {
                return None;
            }
            let feet = feet(at.translation(), walker);
            let eye = feet + Vec3::Y * monster.body.eye_height;
            let clear = Dir3::new(eye - party.eye).map_or(true, |direction| {
                obstacles.first_hit(party.eye, direction, party.eye.distance(eye)).is_none()
            });
            clear.then(|| {
                let distance = distance_to_party(party.eye, monster, feet);
                let candidate = casting::Candidate { entity, at: feet, distance };
                (candidate, groups.get(entity).ok().map(|g| g.0))
            })
        })
        .collect();
    let candidates: Vec<_> = in_sight.iter().map(|&(c, _)| c).collect();
    let group_of = |monster: Entity| -> Vec<Entity> {
        let group = in_sight.iter().find(|(c, _)| c.entity == monster).and_then(|&(_, g)| g);
        match group.and_then(|g| group_members.get(g).ok()) {
            Some(mates) => mates.iter().collect(),
            None => vec![monster],
        }
    };
    let situation = casting::Situation {
        context: casting::UseContext::Combat,
        party: &party_members,
        party_at: party.eye,
        monsters: &candidates,
        group_of: &group_of,
    };
    context
        .resolve(party.now, members, damage, |field, spells| {
            casting::perform(member, choice, &situation, spells, points, rng, field)
        })
        .and_then(|performed| performed)
}

/// What a monster settled on doing with its action besides striking.
enum Plan<'a> {
    Spell {
        spell: crate::content::spells::SpellDefinition,
        power: u8,
        aim: monster_spells::SpellAim,
        scene: monster_spells::Scene,
        caster: (&'a Species, u32),
    },
    Special {
        kind: crate::content::species::SpecialAttackKind,
        cooldown: u8,
        breather: crate::game::magic::effects::Breather,
        victims: Vec<crate::game::magic::effects::Victim>,
    },
}

/// Whether a monster can make its special attack now (retail
/// `CanMonsterFlee`): it has one, it is not cooling down, it has a tenth of
/// its stamina, and a silence does not stop it.
fn special_ready(monster: &Monster, kind: &Species, cooling: bool) -> bool {
    let silenced = monster.conditions.iter().any(|&(c, _)| c == Condition::Silenced);
    kind.special.kind.is_some_and(|special| {
        let definition = crate::game::magic::effects::special(special);
        !cooling
            && monster.stamina >= monster.max_stamina / 10
            && !(silenced && definition.silenced)
    })
}

/// What a monster settled on as its round's action began.
enum Decision<'a> {
    /// Run, withdraw or hold back; `quiet` for a flight fear forced, which
    /// retail does not announce.
    Move {
        conduct: conduct::Conduct,
        quiet: bool,
    },
    Plan(Box<Plan<'a>>),
    /// Guard a fellow this round.
    Protect {
        ward: Entity,
    },
    /// Strike someone it may attack, or close in to; `waits` when it may
    /// hold back instead.
    Fight {
        waits: bool,
    },
}

/// A monster's accepted turn comes (retail `UpdateMonsterAI`,
/// `StartMonsterCombatAction`, `StepMonsterCombatAction`). As its round's
/// action begins it decides: to run, withdraw or hold back
/// ([`conduct::decide`]), to breathe or cast, or to fight; then its
/// conditions may interrupt that ([`conduct::interrupt`]). Fighting, it
/// strikes someone it may attack and reaches, or with nobody in reach
/// advances (or charges when hurt) and acts again later in the round.
/// Returns whether it acted.
fn monster_turn(
    commands: &mut Commands,
    battle: &mut Battle,
    (index, step): (usize, u8),
    (combatants, stances): (&[Entity], &[(Entity, hostility::Stance)]),
    rng: &mut GameRng,
    (members, damage): (&mut Members, &mut MonsterDamage),
    party: &PartyView,
    magic: &mut BattleMagic,
    species: &MonsterCatalog,
    (news, summons): (&mut MessageWriter<CombatNews>, &mut MessageWriter<summon::Summon>),
) -> bool {
    use crate::game::magic::effects::Victim;
    use hostility::Stance;
    let monster = battle.this_round.monsters[index].0;
    if !combatants.contains(&monster)
        || !damage.monsters().get(monster).is_ok_and(|(m, ..)| m.can_fight())
    {
        battle.this_round.monsters[index].1 = Turn::default();
        return false;
    }
    let turn = battle.this_round.monsters[index].1;
    // How everyone in the fight stands, each monster by its own stance.
    let allegiances: Vec<(Entity, hostility::Allegiance)> = combatants
        .iter()
        .filter_map(|&entity| {
            let (body, ..) = damage.monsters().get(entity).ok()?;
            let kind = species.kind(body.species)?;
            let group =
                stances.iter().find(|&&(e, _)| e == entity).map_or(Stance::Hostile, |&(_, s)| s);
            let turncoat = body.conditions.iter().any(|&(c, _)| c == Condition::Turncoat);
            let allegiance = hostility::Allegiance {
                species: body.species,
                stance: hostility::own_stance(group, turncoat),
                faction: kind.faction,
                turncoat,
            };
            Some((entity, allegiance))
        })
        .collect();
    let allegiance_of =
        |entity: Entity| allegiances.iter().find(|&&(e, _)| e == entity).map(|&(_, a)| a);
    let Some(mine) = allegiance_of(monster) else {
        battle.this_round.monsters[index].1 = Turn::default();
        return false;
    };
    let turncoats: Vec<Entity> = magic
        .context
        .roster
        .iter()
        .filter(|&m| members.get(m).is_ok_and(|(_, vitals, ..)| vitals.has(Condition::Turncoat)))
        .collect();
    let toward = |victim: Victim| match victim {
        Victim::Member(member) => {
            hostility::toward_member(mine.stance, turncoats.contains(&member))
        }
        Victim::Monster(other) => {
            allegiance_of(other).map_or(Stance::Neutral, |theirs| hostility::between(mine, theirs))
        }
    };
    // A monster decides as its round's action begins, not again between
    // its attacks.
    let mut decision = Decision::Fight { waits: false };
    let mut interrupted = None;
    if let Ok((body, at, walker, _)) = damage.monsters().get(monster)
        && let Some(kind) = species.kind(body.species)
        && turn.attacks_left == u32::from(kind.attacks_per_round)
        && !battle.this_round.moved.contains(&monster)
    {
        let feet = feet(at.translation(), walker);
        let nearest = distance_to_party(party.eye, body, feet);
        let mut fleeing = battle.fleeing.contains(&monster);
        let conduct = conduct::decide(rng, body, kind, mine.stance, &mut fleeing, nearest);
        battle.fleeing.retain(|&m| m != monster);
        if fleeing {
            battle.fleeing.push(monster);
        }
        // One that would close in on the party to strike does so rather
        // than breathe or cast; one that can strike does either at its
        // chance, and one that cannot always tries.
        let reaches = kind.attacks.iter().any(|a| nearest <= a.reach.metres());
        let closes_in = !kind.temper.prefers_range && !kind.attacks.is_empty() && !reaches;
        let special = kind.special;
        let cooling = battle.cooldowns.iter().any(|&(m, left)| m == monster && left > 0);
        if conduct != conduct::Conduct::Fight {
            decision = Decision::Move { conduct, quiet: false };
        } else if special.chance != 0
            && !closes_in
            && let Some(cone) =
                magic.context.content.spells().and_then(|spells| {
                    spells.get(crate::content::spells::SpellId::SPECIAL_ATTACK_CONE)
                })
            && special_ready(body, kind, cooling)
            && let scene = spell_scene(
                (monster, kind),
                (combatants, party_of(members, &magic.context.roster)),
                damage.monsters(),
                &party.sight(members, &magic.context.roster, &magic.obstacles),
                &toward,
            )
            && let aims = monster_spells::aims(cone, &scene)
            && !aims.is_empty()
            && rng.below(100) < if reaches { u32::from(special.chance) } else { 100 }
        {
            // At one of its aims at random (retail `AimMonsterAtSpellTarget`),
            // catching whoever stands in the cone.
            let aim = aims[rng.below(aims.len() as u32) as usize];
            let victims = monster_spells::cone_victims(
                &scene,
                monster_spells::aimed_at(&scene, aim),
                cone.reach.metres(),
            );
            let breather = crate::game::magic::effects::Breather {
                level: kind.level,
                stamina: body.stamina,
                max_stamina: body.max_stamina,
            };
            decision = Decision::Plan(Box::new(Plan::Special {
                kind: special.kind.expect("a ready monster has a special attack"),
                cooldown: special.cooldown,
                breather,
                victims,
            }));
        } else if kind.magic.chance != 0
            && !closes_in
            && let Some(spells) = magic.context.content.spells()
        {
            let chance = if reaches { u32::from(kind.magic.chance) } else { 100 };
            let roster = &magic.context.roster;
            let sight = party.sight(members, roster, &magic.obstacles);
            let scene = spell_scene(
                (monster, kind),
                (combatants, party_of(members, roster)),
                damage.monsters(),
                &sight,
                &toward,
            );
            if let Some(plan) = monster_spells::choose(rng, body, kind, chance, spells, &scene)
                .and_then(|(id, aim)| {
                    let spell = spells.get(id)?.clone();
                    let budget = body.spell_budget(kind);
                    let power = monster_spells::power(rng, budget, &spell);
                    Some(Plan::Spell { spell, power, aim, scene, caster: (kind, budget) })
                })
            {
                decision = Decision::Plan(Box::new(plan));
            }
        }
        // One that can strike nobody, having no attack or, blind, nobody
        // in reach, withdraws (retail `UpdateMonsterAI`'s
        // `RETURN_TO_START`); a Monk fights blind.
        let blind = body.conditions.iter().any(|&(c, _)| c == Condition::Blind)
            && kind.kind != crate::content::species::MonsterKind::Monk;
        if matches!(decision, Decision::Fight { .. })
            && (kind.attacks.is_empty() || (blind && !reaches))
        {
            decision = Decision::Move { conduct: conduct::Conduct::Withdraw, quiet: false };
        }
        // One with someone in reach builds its choices (retail
        // `BuildMonsterActionQueue`): a bodyguard, as clever as it is, may
        // guard a stronger fellow in trouble; a wary one may hold back.
        if matches!(decision, Decision::Fight { .. }) && reaches {
            let intelligence = u32::from(kind.attributes.intelligence);
            let able = kind.temper.bodyguard
                && body.conditions.iter().all(|&(c, _)| c < Condition::Blind)
                && !kind.attacks.is_empty();
            let wards: Vec<Entity> = if able && rng.below(100) < intelligence {
                let reach = kind.attacks[0].reach.metres();
                combatants
                    .iter()
                    .copied()
                    .filter(|&other| {
                        other != monster
                            && toward(Victim::Monster(other)) == Stance::Friendly
                            && damage.monsters().get(other).is_ok_and(|(o, o_at, o_walker, _)| {
                                let gap = feet.distance(crate::game::world::navigation::feet(
                                    o_at.translation(),
                                    o_walker,
                                )) - body.body.radius
                                    - o.body.radius;
                                !o.is_dead()
                                    && gap.max(0.0) <= reach
                                    && species.kind(o.species).is_some_and(|k| k.level > kind.level)
                                    && o.hp * 100 / o.max_hp.max(1) < 40
                            })
                    })
                    .collect()
            } else {
                Vec::new()
            };
            if !wards.is_empty() {
                let ward = wards[rng.below(wards.len() as u32) as usize];
                decision = Decision::Protect { ward };
            } else if kind.temper.wary && rng.below(100) < intelligence {
                decision = Decision::Fight { waits: true };
            }
        }
        // Its conditions, then, as the action begins.
        let against_foes = match &decision {
            Decision::Move { .. } | Decision::Protect { .. } => false,
            Decision::Plan(plan) => match &**plan {
                Plan::Spell { spell, .. } => spell.aimed_at_enemies(),
                Plan::Special { .. } => true,
            },
            Decision::Fight { .. } => true,
        };
        let strikes_friend = kind.attacks.iter().any(|attack| {
            let reach = attack.reach.metres();
            (nearest <= reach
                && magic
                    .context
                    .roster
                    .iter()
                    .any(|m| toward(Victim::Member(m)) == Stance::Friendly))
                || combatants.iter().any(|&other| {
                    other != monster
                        && toward(Victim::Monster(other)) == Stance::Friendly
                        && damage.monsters().get(other).is_ok_and(|(o, o_at, o_walker, _)| {
                            let gap = feet.distance(crate::game::world::navigation::feet(
                                o_at.translation(),
                                o_walker,
                            )) - body.body.radius
                                - o.body.radius;
                            gap.max(0.0) <= reach
                        })
                })
        });
        let berserk = battle.this_round.berserk.contains(&monster);
        let fleeing_now =
            matches!(decision, Decision::Move { conduct: conduct::Conduct::Flee, .. });
        interrupted = conduct::interrupt(
            rng,
            body,
            kind,
            (fleeing_now, strikes_friend, berserk, against_foes),
        );
    }
    match interrupted {
        None | Some(Interrupt::Turncoat) => {}
        // Insanity turns it on its friends this round.
        Some(Interrupt::Berserk) => {
            battle.this_round.berserk.push(monster);
            decision = Decision::Fight { waits: false };
        }
        // Fear sends it running, without its flight's notice.
        Some(Interrupt::Panics) => {
            if !battle.fleeing.contains(&monster) {
                battle.fleeing.push(monster);
            }
            decision = Decision::Move { conduct: conduct::Conduct::Flee, quiet: true };
        }
        Some(interrupt) => {
            if interrupt == Interrupt::Freed
                && let Some(mut conditions) = damage.conditions_mut(monster)
            {
                conditions.retain(|&(c, _)| c != Condition::Webbed);
            }
            news.write(CombatNews::Interrupted { monster, interrupt });
            battle.this_round.monsters[index].1 = Turn::default();
            return true;
        }
    }
    let waits = matches!(decision, Decision::Fight { waits: true });
    let plan = match decision {
        Decision::Move { conduct, quiet } => {
            battle.this_round.monsters[index].1 = Turn::default();
            let Ok((body, at, walker, _)) = damage.monsters().get(monster) else { return false };
            let Some(kind) = species.kind(body.species) else { return false };
            if conduct == conduct::Conduct::Hold {
                return false;
            }
            // A monster that cannot walk, or has moved this round, waits
            // instead (retail falls back on `WAIT` when the move fails).
            if !walker.is_some_and(|w| w.speed > 0.0) || battle.this_round.moved.contains(&monster)
            {
                battle.this_round.waited.push(monster);
                return true;
            }
            battle.this_round.moved.push(monster);
            let metres = approach::move_metres(body, kind, false);
            let away = approach::away_from(feet(at.translation(), walker), party.eye, metres);
            commands.entity(monster).insert(approach::CombatMove {
                budget: metres,
                moved: 0.0,
                away: Some(away),
            });
            if !quiet {
                news.write(match conduct {
                    conduct::Conduct::Flee => CombatNews::Flees { monster },
                    _ => CombatNews::Withdraws { monster },
                });
            }
            return true;
        }
        Decision::Protect { ward } => {
            battle.this_round.monsters[index].1 = Turn::default();
            battle.this_round.guards.push(Guard { guard: monster, ward, interceptions: 0 });
            return true;
        }
        Decision::Plan(plan) => Some(*plan),
        Decision::Fight { .. } => None,
    };
    let BattleMagic { context, points, moving, .. } = magic;
    match plan {
        Some(Plan::Spell { spell, power, aim, scene, caster }) => {
            battle.this_round.monsters[index].1 = Turn::default();
            battle.this_round.attacked = true;
            let report = context.with_field(party.now, members, damage, |field| {
                monster_spells::cast((&spell, power, aim), caster, &scene, rng, field, points)
            });
            if let Ok(report) = report {
                context.news.write(crate::game::magic::casting::MagicNews::Done(report));
            }
            return true;
        }
        // Retail `ExecuteMonsterSpecialAttack`: the attack lands, the
        // cooldown starts and it costs a tenth of the monster's stamina.
        Some(Plan::Special { kind, cooldown, breather, victims }) => {
            battle.this_round.monsters[index].1 = Turn::default();
            battle.this_round.attacked = true;
            battle.this_round.specials.push(monster);
            news.write(CombatNews::Special { monster, kind });
            // A summoning calls its group up rather than landing on anyone.
            if let Some(species) = kind.summons() {
                summons.write(summon::Summon { summoner: monster, species });
            }
            let effects = context.with_field(party.now, members, damage, |field| {
                crate::game::magic::effects::special_attack(
                    kind, breather, &victims, rng, field, points,
                )
            });
            if let Ok(effects) = effects
                && !effects.is_empty()
            {
                news.write(CombatNews::Effects(effects));
            }
            battle.cooldowns.retain(|&(m, _)| m != monster);
            battle.cooldowns.push((monster, cooldown));
            damage.fatigue(monster, breather.max_stamina / 10);
            return true;
        }
        None => {}
    }
    // A berserk monster strikes its friends; any other its foes.
    let wanted = if battle.this_round.berserk.contains(&monster) {
        Stance::Friendly
    } else {
        Stance::Hostile
    };
    let may_attack = |victim: Victim| toward(victim) == wanted;
    let advancing = |monster| moving.contains(monster);
    let armours = party_armours(battle, members, context);
    let armour = |member| armours.iter().find(|&&(m, _)| m == member).map(|&(_, a)| a);
    let mut fight = blows::Fight {
        combatants,
        party,
        armour: &armour,
        advancing: &advancing,
        points,
        attacks: &may_attack,
        waits,
    };
    let idle = context
        .with_field(party.now, members, damage, |field| {
            blows::monster_attacks(battle, index, rng, field, &mut fight, news)
        })
        .unwrap_or(false);
    if idle
        && !battle.this_round.moved.contains(&monster)
        && let Ok((walker_monster, at, walker, _)) = damage.monsters().get(monster)
        && let Some(budget) =
            approach::move_budget(walker_monster, at.translation(), walker, party.eye, species)
    {
        battle.this_round.moved.push(monster);
        battle.this_round.monsters[index].1 =
            Turn { phase: (u32::from(step) * 2).min(100) as u8, attacks_left: turn.attacks_left };
        let charge = budget.charge;
        commands.entity(monster).insert(approach::CombatMove {
            budget: budget.metres,
            moved: 0.0,
            away: None,
        });
        news.write(CombatNews::Moves { monster, charge });
    }
    true
}

/// A member as a casting monster sees them, at the party's point
/// `party` away.
fn member_in_scene(
    (member, character, vitals): (Entity, &Character, &Vitals),
    (party_at, party): (Vec3, f32),
    toward: &dyn Fn(crate::game::magic::effects::Victim) -> hostility::Stance,
) -> monster_spells::Someone {
    let stats = character.stats(&vitals.enchantments);
    monster_spells::Someone {
        entity: member,
        at: party_at,
        from_caster: party,
        standing: crate::game::magic::effects::Standing {
            hurt: vitals.hp < stats.max_hp(),
            tired: vitals.stamina < stats.max_stamina(),
            conditions: vitals.conditions.iter().map(|&(c, _)| c).collect(),
            enchanted: vitals.enchantments.0.iter().map(|e| e.spell).collect(),
        },
        stance: toward(crate::game::magic::effects::Victim::Member(member)),
    }
}

/// The party's members in roster order, as a scene reads them.
fn party_of<'a>(
    members: &'a Members,
    roster: &'a PartyRoster,
) -> impl Iterator<Item = (Entity, &'a Character, &'a Vitals)> {
    roster.iter().filter_map(|member| {
        members.get(member).ok().map(|(character, vitals, ..)| (member, character, vitals))
    })
}

/// The fight as a monster about to cast or breathe sees it: the living
/// party and the living monsters in the fight it has a clear line to, where
/// each stands, what each lacks, and how it stands toward each (`toward`).
fn spell_scene<'a>(
    (caster, kind): (Entity, &Species),
    (combatants, party_members): (
        &[Entity],
        impl Iterator<Item = (Entity, &'a Character, &'a Vitals)>,
    ),
    monsters: &MonsterQuery,
    sight: &Sight,
    toward: &dyn Fn(crate::game::magic::effects::Victim) -> hostility::Stance,
) -> monster_spells::Scene {
    use crate::game::magic::effects::Victim;
    let party_eye = sight.eye;
    let party_at = party_eye - Vec3::Y * CAMERA_ABOVE_PARTY_POINT;
    let Ok((body, at, walker, _)) = monsters.get(caster) else {
        return monster_spells::Scene {
            caster,
            from: party_at,
            party_at,
            members: vec![],
            monsters: vec![],
            party: f32::INFINITY,
            sees_party: false,
        };
    };
    let feet_at = feet(at.translation(), walker);
    let eye = feet_at + Vec3::Y * body.body.eye_height;
    let party = distance_to_party(party_eye, body, feet_at);
    // A fellow is in reach of its spell only along a clear line, as the
    // party is only where the caster sees it.
    let clear = |to: Vec3| {
        Dir3::new(to - eye).map_or(true, |direction| {
            sight.obstacles.first_hit(eye, direction, eye.distance(to)).is_none()
        })
    };
    monster_spells::Scene {
        caster,
        from: feet_at,
        party_at,
        party,
        sees_party: sight.sees(body, &at, walker, kind),
        members: party_members
            .filter(|(_, _, vitals)| !vitals.is_dead())
            .map(|(member, character, vitals)| {
                member_in_scene((member, character, vitals), (party_at, party), toward)
            })
            .collect(),
        monsters: combatants
            .iter()
            .filter_map(|&entity| {
                let (other, at, walker, _) = monsters.get(entity).ok()?;
                if other.is_dead() {
                    return None;
                }
                let at = feet(at.translation(), walker);
                if entity != caster && !clear(at + Vec3::Y * other.body.eye_height) {
                    return None;
                }
                Some(monster_spells::Someone {
                    entity,
                    at,
                    from_caster: (at.distance(feet_at) - body.body.radius - other.body.radius)
                        .max(0.0),
                    standing: monster_spells::monster_standing(other),
                    stance: if entity == caster {
                        hostility::Stance::Friendly
                    } else {
                        toward(Victim::Monster(entity))
                    },
                })
            })
            .collect(),
    }
}

/// The round is over: a round's game time passes. Returns whether the fight
/// is over: no hostile is left or nobody could attack for two rounds.
fn end_round(
    battle: &mut Battle,
    combatants: &[Entity],
    clock: &mut GameClock,
    rng: &mut GameRng,
    roster: &PartyRoster,
    members: &mut Members,
    damage: &mut MonsterDamage,
    party_eye: Vec3,
    species: &MonsterCatalog,
    news: &mut MessageWriter<CombatNews>,
    points: &mut Query<(&LearnedSpells, &mut SpellPoints)>,
    enchantments: &mut crate::game::magic::enchantments::Enchantments,
    staying: &[Entity],
) -> bool {
    // The round's time passes now, with its meaning: conditions age, spell
    // points come back at half rate and nothing else recovers (retail
    // `AdvanceCombatRound`, `AdvanceTimedEffects`).
    let ticks = clock.advance(ROUND_MS);
    enchantments.age(ticks.aging);
    // A monster that waited, or lies asleep or knocked out, gets its
    // breath back (retail `AdvanceCombatRound`).
    for &entity in combatants {
        let waited = battle.this_round.waited.contains(&entity);
        let resting = waited
            || damage.monsters().get(entity).is_ok_and(|(monster, ..)| {
                monster
                    .conditions
                    .iter()
                    .any(|&(c, _)| matches!(c, Condition::Asleep | Condition::Unconscious))
            });
        if resting {
            let maximum = damage.monsters().get(entity).map_or(0, |(m, ..)| m.max_stamina);
            let sides = (maximum / 20).saturating_sub(1).clamp(1, 255) as u8;
            let rolled = rng.roll(Dice { count: 2, sides, base: 0 });
            damage.refresh(entity, rolled);
        }
    }
    // A special attack cools down over the rounds the monster does not
    // make one (retail `AdvanceCombatRound`).
    for (monster, left) in &mut battle.cooldowns {
        if *left > 0 && !battle.this_round.specials.contains(monster) {
            *left -= 1;
        }
    }
    for &entity in combatants {
        if let Some(mut e) = damage.enchantments_mut(entity) {
            e.age(ticks.aging);
        }
        if let Some(mut conditions) = damage.conditions_mut(entity)
            && crate::game::characters::any_timed(&conditions)
        {
            crate::game::characters::age(&mut conditions, ticks.aging);
        }
    }
    for (_, mut pool) in points.iter_many_unique_mut(roster.iter()).matched() {
        if pool.short() {
            pool.recover(ticks.aging, crate::game::magic::recovery_scale(0.0));
        }
    }
    for (profile, mut vitals, ..) in members.iter_many_unique_mut(roster.iter()).matched() {
        recovery::pass(&profile, &mut vitals, ticks, recovery::Pace::Fighting);
    }
    battle.phase = RoundPhase::Waiting;
    // Recover after aging conditions, in accepted actor order.
    for work in &battle.this_round.party {
        let member = work.actor;
        if !roster.contains(&member) {
            continue;
        }
        let Ok((character, mut vitals, ..)) = members.get_mut(member) else { continue };
        let resting = battle.defends(member)
            || matches!(vitals.worst_condition(), Some(Condition::Asleep | Condition::Unconscious));
        if vitals.is_dead() || !resting {
            continue;
        }
        let sides =
            (character.stats(&vitals.enchantments).max_stamina() / 20).saturating_sub(1).max(1);
        let rolled = rng.roll(Dice { count: 2, sides: sides.min(255) as u8, base: 0 });
        let gained = recovery::combat_rest_amount(&character, rolled);
        if vitals.stamina < character.stats(&vitals.enchantments).max_stamina()
            || vitals
                .conditions
                .contains(&(Condition::Unconscious, crate::game::characters::Lasting::Indefinite))
        {
            let maximum = character.stats(&vitals.enchantments).max_stamina();
            vitals.restore_stamina(gained, maximum);
        }
    }
    battle.wait = Timer::from_seconds(ROUND_SECONDS, TimerMode::Once);
    let monsters = damage.monsters();
    let approaching = combatants.iter().any(|&entity| {
        monsters.get(entity).is_ok_and(|(monster, at, walker, _)| {
            approach::needs_approach(monster, at.translation(), walker, party_eye, species)
        })
    });
    let active = battle.this_round.attacked || approaching;
    battle.passive_rounds = if active { 0 } else { battle.passive_rounds + 1 };
    let hostiles_left = staying.iter().any(|&e| monsters.get(e).is_ok_and(|(m, ..)| !m.is_dead()));
    if hostiles_left && battle.passive_rounds < PASSIVE_ROUNDS {
        return false;
    }
    // Experience is shared among the living, the unconscious included.
    let living =
        members.iter_many(roster.iter()).matched().filter(|(_, v, ..)| !v.is_dead()).count() as u32;
    let each = battle.experience / living.max(1);
    if each > 0 {
        for (mut character, vitals, ..) in members.iter_many_unique_mut(roster.iter()).matched() {
            if !vitals.is_dead() {
                character.experience += each;
            }
        }
    }
    news.write(if battle.kills > 0 {
        CombatNews::Won { kills: battle.kills, experience_each: each }
    } else {
        CombatNews::Over
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::items::{ItemId, LootTables};
    use crate::content::spells::SpellCatalog;
    use crate::game::data::GameData;
    use crate::game::inventory::{Carried, ItemIds, ItemStack, make_item};
    use crate::game::party::DamageApplied;
    use enumset::EnumSet;

    #[test]
    fn party_armour_follows_membership_and_expiry() {
        use crate::game::magic::enchantments::{Enchantment, PartyEffects};
        use crate::game::party::roster::PartyMember;
        let (mut app, members) = round_app(false);
        app.world_mut().resource_mut::<Messages<StartRound>>().clear();
        let profile = app.world().get::<Character>(members[0]).unwrap().clone();
        let stranger =
            app.world_mut().spawn((Vitals::new(&profile), profile, Carried::default())).id();
        app.update();
        let armour = |app: &mut App, member: Entity| {
            let mut state = bevy::ecs::system::SystemState::<(
                Query<crate::presentation::character_view::CharacterView>,
                Option<Single<&PartyEffects>>,
            )>::new(app.world_mut());
            let (views, party) = state.get(app.world()).unwrap();
            let items = GameData::items(app.world()).unwrap();
            views
                .get(member)
                .unwrap()
                .armour(member, Some(items), None, party.as_deref().copied())
                .unwrap()
                .total
        };
        let before = armour(&mut app, members[0]);
        let outsider = armour(&mut app, stranger);
        let party = crate::game::party::roster::the_party(app.world_mut()).unwrap();
        app.world_mut().get_mut::<PartyEffects>(party).unwrap().0.apply(Enchantment {
            spell: crate::content::spells::SpellId::ARMORPLATE,
            power: 5,
            boost: 0,
            turns: 2,
            absorption: 0,
        });
        app.update();
        assert_eq!(armour(&mut app, members[0]), before + 3);
        assert_eq!(armour(&mut app, stranger), outsider);
        app.world_mut().entity_mut(members[0]).remove::<PartyMember>();
        app.update();
        assert_eq!(armour(&mut app, members[0]), before);
        app.world_mut().get_mut::<PartyEffects>(party).unwrap().0.age(2);
        app.update();
        assert!(app.world().get::<PartyEffects>(party).unwrap().0.0.is_empty());
    }

    #[test]
    fn round_button_is_interpreted_by_the_current_battle() {
        let mut app = App::new();
        app.init_resource::<CombatSettings>()
            .add_message::<CombatControl>()
            .add_message::<StartRound>()
            .add_systems(Update, control_combat);
        app.world_mut().write_message(CombatControl::RoundButton);
        app.update();
        assert!(app.world().resource::<Messages<StartRound>>().is_empty());
        let battle = app.world_mut().spawn(Battle::default()).id();
        app.world_mut().write_message(CombatControl::RoundButton);
        app.update();
        assert_eq!(app.world_mut().resource_mut::<Messages<StartRound>>().drain().count(), 1);
        let before = app.world().resource::<CombatSettings>().continuous;
        app.world_mut().write_message(CombatControl::RoundButton);
        app.world_mut().get_mut::<Battle>(battle).unwrap().phase = RoundPhase::Accepted;
        app.update();
        assert_eq!(app.world().resource::<CombatSettings>().continuous, !before);
        assert!(app.world().resource::<Messages<StartRound>>().is_empty());
    }

    #[test]
    fn sight_reaches_the_far_clip_ahead_and_less_from_behind() {
        // Facing the party: the whole far clip.
        assert_eq!(sight_threshold(0.3, 40, 0, 1, 60.0, 30.0), 60.0);
        // Behind it: (senses - 2 × lowest level) × (far - d) / d percent.
        let behind = sight_threshold(3.0, 40, 0, 1, 60.0, 30.0);
        assert!((behind - 0.38 * 60.0).abs() < 1e-3);
        // Never under 2%, and a ranged bonus scales the percent.
        assert!((sight_threshold(3.0, 0, 0, 10, 60.0, 50.0) - 1.2).abs() < 1e-3);
        assert!((sight_threshold(3.0, 40, 50, 1, 60.0, 30.0) - 0.57 * 60.0).abs() < 1e-3);
    }

    #[test]
    fn remaining_attacks_spread_over_the_round() {
        assert_eq!(next_phase(10, 0), 0);
        assert_eq!(next_phase(10, 1), 55);
        assert_eq!(next_phase(55, 1), 77);
        assert_eq!(next_phase(100, 1), 100, "again at the same step");
    }
    /// Real battle acceptance and round execution without movement, rendering or UI.
    fn round_app(defending: bool) -> (App, [Entity; 2]) {
        use crate::content::characters::Skill;
        use crate::content::items::{ItemDefinition, QuantityKind};
        use crate::content::species::Species;
        use crate::game::characters::{Gender, Profession, Race};
        use crate::game::world::population::ScaledBody;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_secs(1),
            ))
            .init_resource::<Assets<ItemCatalog>>()
            .init_resource::<Assets<MonsterCatalog>>()
            .init_resource::<Assets<crate::content::spells::SpellCatalog>>()
            .init_resource::<Assets<LootTables>>()
            .init_resource::<Assets<crate::content::monster::MonsterBody>>()
            .init_resource::<crate::game::world::population::MonsterBodies>()
            .init_resource::<Assets<crate::game::world::navigation::WalkerMotionConfig>>()
            .init_resource::<avian3d::collider_tree::ColliderTrees>()
            .init_resource::<crate::game::factions::Factions>()
            .init_resource::<crate::game::difficulty::Difficulty>()
            .add_message::<MagicNews>()
            .add_message::<crate::game::world::hostility::Provocation>()
            .init_resource::<ItemIds>()
            .init_resource::<GameClock>()
            .insert_resource(GameRng::seeded(5))
            .add_message::<DamageApplied>()
            .init_resource::<Assets<crate::content::level::LevelDefinition>>()
            .add_message::<crate::game::world::identify::Identified>()
            .configure_sets(
                Update,
                (crate::game::GameplaySet::Stats, crate::game::GameplaySet::Combat).chain(),
            )
            .add_plugins((CombatPlugin, crate::game::stats::plugin));
        app.world_mut()
            .resource_mut::<Time<Virtual>>()
            .set_max_delta(std::time::Duration::from_secs(2));
        let weapons: Vec<_> = [100, 0]
            .into_iter()
            .enumerate()
            .map(|(i, initiative)| ItemDefinition {
                id: ItemId(i as u16),
                name: format!("sword {i}"),
                class: crate::content::items::EquipClass::ShortWeapon,
                weapon_skill: Some(Skill::Sword),
                reach: Reach::Touch,
                damage: Dice { count: 0, sides: 0, base: 1 },
                damage_bonus: initiative,
                attack_modes: EnumSet::only(AttackMode::Swing),
                quantity: QuantityKind::None,
                ..default()
            })
            .collect();
        let mut ids = ItemIds::default();
        let mut equipment_rng = GameRng::seeded(0);
        let mut members = Vec::new();
        for (name, weapon) in ["A", "B"].into_iter().zip(&weapons) {
            let character = Character::new(name, Race::Human, Gender::Male, Profession::Fighter);
            let mut carried = Carried::default();
            carried.equipment[EquipSlot::Primary] = Some(crate::game::inventory::Equipped::new(
                make_item(weapon, &mut ids, &mut equipment_rng, false, true),
            ));
            let order = if name == "B" && defending { Order::Defend } else { Order::Attack };
            let entity =
                app.world_mut().spawn((Vitals::new(&character), character, carried, order)).id();
            members.push(entity);
        }
        let items = app.world_mut().resource_mut::<Assets<ItemCatalog>>().add(ItemCatalog(weapons));
        let species =
            app.world_mut().resource_mut::<Assets<MonsterCatalog>>().add(MonsterCatalog(vec![
                Species { name: "Target".into(), generic_name: "Target".into(), ..default() },
            ]));
        let loot = app.world_mut().resource_mut::<Assets<LootTables>>().add(LootTables::default());
        app.insert_resource(GameData { items, species, loot, spells: default() });
        let party = crate::game::party::roster::test_party(app.world_mut(), &members);
        app.world_mut().entity_mut(party).insert((PartyBody::default(), Position::default()));
        let root = app.world_mut().spawn(crate::game::world::level::Level::test(default())).id();
        let monster = app
            .world_mut()
            .spawn((
                Monster {
                    enchantments: default(),
                    location: 0,
                    species: 0,
                    hp: 10000,
                    max_hp: 10000,
                    stamina: 0,
                    max_stamina: 0,
                    conditions: vec![],
                    spell_points: 0,
                    body: ScaledBody { scale: 1.0, radius: 1.0, eye_height: 1.0, speed: 0.0 },
                },
                Transform::from_translation(Vec3::NEG_Z),
                LevelOwned(root),
            ))
            .id();
        let battle = app.world_mut().spawn((Battle::default(), LevelOwned(root))).id();
        app.world_mut().entity_mut(monster).insert(Combatant(battle));
        app.finish();
        app.cleanup();
        app.world_mut().write_message(StartRound);
        (app, [members[0], members[1]])
    }

    fn fight(world: &World) -> Option<&Battle> {
        world.try_query::<&Battle>()?.single(world).ok()
    }

    fn fight_mut(world: &mut World) -> Mut<'_, Battle> {
        world.query::<&mut Battle>().single_mut(world).unwrap()
    }

    /// Give the played level its definition, which sets how far monsters
    /// see and lets fights start.
    fn define_level(app: &mut App) {
        let world = app.world_mut();
        let geometry = world
            .resource_mut::<Assets<crate::content::level::LevelDefinition>>()
            .add(crate::content::level::LevelDefinition::default());
        world
            .query::<&mut crate::game::world::level::Level>()
            .single_mut(world)
            .unwrap()
            .definition = geometry;
    }

    /// Make `member` a trained Mage who knows `spell`, with points to cast it,
    /// and order the cast at the fight's monster.
    fn order_spell(
        app: &mut App,
        member: Entity,
        spell: crate::content::spells::SpellDefinition,
    ) -> Entity {
        use crate::content::characters::{Realm, Skill};
        use crate::game::characters::Profession;
        use crate::game::magic::casting::{Aim, MagicChoice, Source};
        let id = spell.id;
        let mut catalog: Vec<_> = (0..=id.0).map(|_| default()).collect();
        catalog[usize::from(id.0)] = spell;
        let handle =
            app.world_mut().resource_mut::<Assets<SpellCatalog>>().add(SpellCatalog(catalog));
        app.world_mut().resource_mut::<GameData>().spells = handle;
        {
            let mut character = app.world_mut().get_mut::<Character>(member).unwrap();
            character.profession = Profession::Mage;
            character.profession_levels[Profession::Mage] = 1;
            for skill in [Skill::Wizardry, Skill::FireMagic, Skill::AirMagic] {
                character.skill_points[skill] = 100;
            }
        }
        let mut points = SpellPoints::default();
        for realm in [Realm::Fire, Realm::Air] {
            points.current[realm] = 20;
        }
        let monster =
            app.world().try_query::<&Combatants>().unwrap().single(app.world()).unwrap().entities()
                [0];
        let choice = MagicChoice {
            source: Source::Spell { spell: id, power: 1 },
            aim: Aim::Monster(monster),
        };
        app.world_mut().entity_mut(member).insert((
            LearnedSpells([id].into()),
            points,
            Order::Magic(choice),
        ));
        monster
    }

    fn energy_blast() -> crate::content::spells::SpellDefinition {
        use crate::content::spells::{SpellDefinition, SpellId, Targeting, Usable};
        SpellDefinition {
            id: SpellId::ENERGY_BLAST,
            name: "Energy Blast".into(),
            realm: Some(crate::content::characters::Realm::Fire),
            level: 1,
            cost: 2,
            reach: Reach::Long,
            targeting: Targeting::Enemy,
            usable: Usable::InCombat,
            dice: Some(Dice { base: 0, count: 1, sides: 7 }),
            ..default()
        }
    }

    /// A monster with a spell it can aim at the party casts it at its turn:
    /// it lands on a member at once and spends nothing (retail
    /// `UpdateMonsterAI`, `MonsterCastsSpell`).
    #[test]
    fn a_monster_casts_its_spell_at_a_member() {
        use crate::content::spells::SpellCatalog;
        use crate::game::magic::casting::MagicNews;
        use crate::game::magic::effects::{Effect, Victim};
        let (mut app, members) = round_app(true);
        let monster =
            app.world().try_query::<&Combatants>().unwrap().single(app.world()).unwrap().entities()
                [0];
        let blast =
            crate::content::spells::SpellDefinition { monsters_cast: true, ..energy_blast() };
        let id = blast.id;
        let mut catalog: Vec<_> = (0..=id.0).map(|_| default()).collect();
        catalog[usize::from(id.0)] = blast;
        let handle =
            app.world_mut().resource_mut::<Assets<SpellCatalog>>().add(SpellCatalog(catalog));
        app.world_mut().resource_mut::<GameData>().spells = handle;
        let data = app.world().resource::<GameData>().species.clone();
        {
            let mut catalogs = app.world_mut().resource_mut::<Assets<MonsterCatalog>>();
            let mut species = catalogs.get_mut(&data).unwrap();
            species.0[0].level = 5;
            species.0[0].magic.chance = 100;
            species.0[0].magic.budget = 200;
            species.0[0].magic.spells[0] = Some(id);
        }
        let mut cast = None;
        for _ in 0..12 {
            app.update();
            let news: Vec<_> =
                app.world_mut().resource_mut::<Messages<MagicNews>>().drain().collect();
            cast = news.into_iter().find_map(|n| match n {
                MagicNews::Done(report) if report.caster == monster => Some(report),
                _ => None,
            });
            if cast.is_some() {
                break;
            }
        }
        let report = cast.expect("the monster casts");
        assert_eq!(report.spell, id);
        assert!(report.power >= 6, "a large budget casts hard: {}", report.power);
        assert!(
            report.effects.iter().any(|e| matches!(
                e,
                Effect::Damaged { victim: Victim::Member(m), .. } | Effect::Resisted { victim: Victim::Member(m) }
                    if members.contains(m)
            )),
            "{:?}",
            report.effects
        );
    }

    /// A monster with a breath and nothing to strike with breathes on the
    /// party at its turn: the notice, what it did to the members, its
    /// cooldown and a tenth of its stamina (retail `UpdateMonsterAI`,
    /// `ExecuteMonsterSpecialAttack`, `ResolveMonsterGroupAttack`).
    #[test]
    fn a_monster_breathes_on_the_party() {
        use crate::content::species::{SpecialAttack, SpecialAttackKind};
        let burn = SpecialAttackKind::WoundFire;
        use crate::content::spells::{SpellCatalog, SpellDefinition, SpellId, Targeting};
        use crate::game::magic::effects::{Effect, Victim};
        let (mut app, members) = round_app(true);
        // The party holds back, so no blow knocks the monster out first.
        for member in members {
            app.world_mut().entity_mut(member).insert(Order::Defend);
        }
        let monster =
            app.world().try_query::<&Combatants>().unwrap().single(app.world()).unwrap().entities()
                [0];
        let cone = SpellId::SPECIAL_ATTACK_CONE;
        let mut catalog: Vec<SpellDefinition> = (0..=cone.0).map(|_| default()).collect();
        catalog[usize::from(cone.0)] = SpellDefinition {
            id: cone,
            targeting: Targeting::Cone,
            reach: Reach::Long,
            ..default()
        };
        let handle =
            app.world_mut().resource_mut::<Assets<SpellCatalog>>().add(SpellCatalog(catalog));
        app.world_mut().resource_mut::<GameData>().spells = handle;
        let data = app.world().resource::<GameData>().species.clone();
        {
            let mut catalogs = app.world_mut().resource_mut::<Assets<MonsterCatalog>>();
            let mut species = catalogs.get_mut(&data).unwrap();
            species.0[0].special = SpecialAttack { kind: Some(burn), chance: 100, cooldown: 3 };
            species.0[0].level = 5;
        }
        {
            let mut body = app.world_mut().get_mut::<Monster>(monster).unwrap();
            body.stamina = 100;
            body.max_stamina = 100;
        }
        let mut news = Vec::new();
        for _ in 0..12 {
            app.update();
            news.extend(app.world_mut().resource_mut::<Messages<CombatNews>>().drain());
            if news.contains(&CombatNews::Special { monster, kind: burn }) {
                break;
            }
        }
        assert!(news.contains(&CombatNews::Special { monster, kind: burn }), "it breathes");
        let effects: Vec<Effect> = news
            .iter()
            .filter_map(|n| match n {
                CombatNews::Effects(effects) => Some(effects.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        assert!(
            effects.iter().all(|e| matches!(
                e,
                Effect::Damaged { victim: Victim::Member(m), .. }
                    | Effect::Resisted { victim: Victim::Member(m) } if members.contains(m)
            )),
            "only the party is caught: {effects:?}"
        );
        assert_eq!(app.world().get::<Monster>(monster).unwrap().stamina, 90);
        assert!(fight(app.world()).unwrap().cooldowns.contains(&(monster, 3)));
    }

    /// A monster under Turncoat strikes its fellow, not the party it now
    /// stands with (retail `MonsterHostility`, `MonsterVsCharDisposition`).
    #[test]
    fn a_turncoat_monster_strikes_its_fellow() {
        use crate::game::characters::Lasting;
        let (mut app, members) = round_app(true);
        let turned =
            app.world().try_query::<&Combatants>().unwrap().single(app.world()).unwrap().entities()
                [0];
        let data = app.world().resource::<GameData>().species.clone();
        {
            let mut catalogs = app.world_mut().resource_mut::<Assets<MonsterCatalog>>();
            let mut species = catalogs.get_mut(&data).unwrap();
            species.0[0].attacks_per_round = 1;
            species.0[0].swings_per_round = 1;
            species.0[0].attacks = vec![crate::content::species::MonsterAttack {
                word: AttackWord::Verb,
                weapon: crate::content::species::MonsterWeapon::Unarmed,
                effects: default(),
                reach: Reach::Touch,
                score: 50,
                modes: EnumSet::only(AttackMode::Bash),
                damage: Dice { base: 1, count: 1, sides: 2 },
            }];
        }
        let battle =
            app.world_mut().query_filtered::<Entity, With<Battle>>().single(app.world()).unwrap();
        let mut fellow = app.world().get::<Monster>(turned).unwrap().clone();
        fellow.location = 1;
        let level = app.world().get::<LevelOwned>(turned).unwrap().0;
        let fellow = app
            .world_mut()
            .spawn((fellow, Transform::from_translation(Vec3::NEG_Z * 2.5), LevelOwned(level)))
            .id();
        app.world_mut().entity_mut(fellow).insert(Combatant(battle));
        app.world_mut()
            .get_mut::<Monster>(turned)
            .unwrap()
            .conditions
            .push((Condition::Turncoat, Lasting::Rounds(20)));
        let mut struck = None;
        for _ in 0..16 {
            app.update();
            let news: Vec<_> =
                app.world_mut().resource_mut::<Messages<CombatNews>>().drain().collect();
            struck = news.into_iter().find_map(|n| match n {
                CombatNews::Attack { attacker, target, .. } if attacker == turned => Some(target),
                _ => None,
            });
            if struck.is_some() {
                break;
            }
        }
        let struck = struck.expect("the turncoat attacks");
        assert_eq!(struck, fellow, "its fellow, not a member");
        assert!(!members.contains(&struck));
    }

    /// A bodyguard guards a stronger fellow in trouble, and steps in front
    /// of the blows meant for it (retail `BuildMonsterActionQueue`'s
    /// protection, `ResolveGuardianInterception`).
    #[test]
    fn a_bodyguard_takes_blows_meant_for_its_ward() {
        use crate::content::species::{MonsterAttack, MonsterWeapon, Species};
        let (mut app, [a, b]) = round_app(true);
        // Only the member with the slow sword strikes, after the guard has
        // chosen to guard.
        app.world_mut().entity_mut(a).insert(Order::Defend);
        app.world_mut().entity_mut(b).insert(Order::Attack);
        let ward =
            app.world().try_query::<&Combatants>().unwrap().single(app.world()).unwrap().entities()
                [0];
        let claw = MonsterAttack {
            word: AttackWord::Verb,
            weapon: MonsterWeapon::Unarmed,
            effects: default(),
            reach: Reach::Short,
            score: 1,
            modes: EnumSet::only(AttackMode::Bash),
            damage: Dice { base: 1, count: 1, sides: 1 },
        };
        let data = app.world().resource::<GameData>().species.clone();
        {
            let mut catalogs = app.world_mut().resource_mut::<Assets<MonsterCatalog>>();
            let catalog = &mut catalogs.get_mut(&data).unwrap().0;
            catalog[0].level = 5;
            catalog[0].attacks = vec![claw.clone()];
            let mut guard = Species {
                name: "Guard".into(),
                attacks_per_round: 4,
                attacks: vec![MonsterAttack { score: 250, ..claw }],
                ..default()
            };
            guard.temper.bodyguard = true;
            // It acts first in the round, so it guards before the blows.
            guard.initiative = 89;
            guard.attributes.intelligence = 100;
            guard.attributes.speed = 200;
            catalog.push(guard);
        }
        {
            let mut body = app.world_mut().get_mut::<Monster>(ward).unwrap();
            body.max_hp = 100;
            body.hp = 30;
        }
        let battle =
            app.world_mut().query_filtered::<Entity, With<Battle>>().single(app.world()).unwrap();
        let mut guard = app.world().get::<Monster>(ward).unwrap().clone();
        guard.species = 1;
        guard.location = 1;
        guard.hp = 10000;
        guard.max_hp = 10000;
        let level = app.world().get::<LevelOwned>(ward).unwrap().0;
        let guard = app
            .world_mut()
            .spawn((guard, Transform::from_translation(Vec3::NEG_Z * 1.5), LevelOwned(level)))
            .id();
        app.world_mut().entity_mut(guard).insert(Combatant(battle));
        // The guard may not reach the ward's trouble at once, so the fight
        // runs on for a few rounds.
        let mut intercepted = false;
        for _ in 0..80 {
            app.world_mut().write_message(StartRound);
            app.update();
            let news: Vec<_> =
                app.world_mut().resource_mut::<Messages<CombatNews>>().drain().collect();
            if news.contains(&CombatNews::Intercepts { guard, ward })
                && news.iter().any(|n| {
                    matches!(n, CombatNews::Attack { attacker, target, .. }
                        if *attacker == b && *target == ward)
                })
            {
                intercepted = true;
                break;
            }
            if fight(app.world()).is_none() {
                break;
            }
        }
        assert!(intercepted, "the guard steps in");
    }

    /// A hostile monster that cannot strike or walk but has a spell to cast
    /// at the party starts a fight on sight (retail `MonsterGroupCanEngage`'s
    /// spell test).
    #[test]
    fn a_stationary_caster_starts_a_fight() {
        use crate::content::spells::SpellCatalog;
        let (mut app, _) = round_app(true);
        define_level(&mut app);
        let monster =
            app.world().try_query::<&Combatants>().unwrap().single(app.world()).unwrap().entities()
                [0];
        let battle =
            app.world_mut().query_filtered::<Entity, With<Battle>>().single(app.world()).unwrap();
        app.world_mut().entity_mut(monster).remove::<Combatant>();
        app.world_mut().despawn(battle);
        let blast =
            crate::content::spells::SpellDefinition { monsters_cast: true, ..energy_blast() };
        let id = blast.id;
        let mut catalog: Vec<_> = (0..=id.0).map(|_| default()).collect();
        catalog[usize::from(id.0)] = blast;
        let handle =
            app.world_mut().resource_mut::<Assets<SpellCatalog>>().add(SpellCatalog(catalog));
        app.world_mut().resource_mut::<GameData>().spells = handle;
        let data = app.world().resource::<GameData>().species.clone();
        {
            let mut catalogs = app.world_mut().resource_mut::<Assets<MonsterCatalog>>();
            let mut species = catalogs.get_mut(&data).unwrap();
            species.0[0].magic.chance = 50;
            species.0[0].magic.spells[0] = Some(id);
        }
        app.update();
        assert!(fight(app.world()).is_some(), "the caster starts the fight");
        assert!(app.world().get::<Combatant>(monster).is_some());
    }

    /// A cast waits for its caster's initiative, is revalidated then, spends
    /// its points and hurts the monster it was aimed at; the caster swings
    /// nothing that round.
    #[test]
    fn an_ordered_spell_is_paid_and_resolved_at_initiative() {
        use crate::content::characters::Realm;
        let (mut app, [caster, _]) = round_app(true);
        let monster = order_spell(&mut app, caster, energy_blast());
        let mut news = Vec::new();
        let mut attackers = Vec::new();
        for _ in 0..12 {
            app.update();
            news.extend(app.world_mut().resource_mut::<Messages<MagicNews>>().drain());
            attackers.extend(attacks(&mut app));
            if !news.is_empty() {
                break;
            }
        }
        let [MagicNews::Done(report)] = news.as_slice() else { panic!("{news:?}") };
        assert_eq!(report.caster, caster);
        assert!(matches!(
            report.effects.as_slice(),
            [Effect::Damaged { victim: Victim::Monster(m), outcome }] if *m == monster && outcome.damage > 0
        ));
        let monster = app.world().get::<Monster>(monster).unwrap();
        assert!(monster.hp < monster.max_hp);
        assert_eq!(app.world().get::<SpellPoints>(caster).unwrap().current[Realm::Fire], 18);
        assert!(!attackers.contains(&"A".to_owned()), "casting replaces the attack");
    }

    #[test]
    fn refused_spell_defends_for_armour_and_recovery_until_the_next_round() {
        use crate::content::characters::Realm;
        use crate::game::inventory::Burden;
        let (mut app, [caster, _]) = round_app(true);
        order_spell(&mut app, caster, energy_blast());
        app.world_mut().get_mut::<Vitals>(caster).unwrap().stamina -= 10;
        let before = app.world().get::<Vitals>(caster).unwrap().stamina;
        // Accept the spell but delay execution, then invalidate its payment.
        fight_mut(app.world_mut()).wait = Timer::from_seconds(3.0, TimerMode::Once);
        app.update();
        assert!(matches!(
            fight(app.world()).unwrap().this_round.party[0].action,
            RoundAction::Magic(_)
        ));
        assert!(!fight(app.world()).unwrap().defends(caster));
        app.world_mut().get_mut::<SpellPoints>(caster).unwrap().current[Realm::Fire] = 0;
        let mut refused = false;
        for _ in 0..12 {
            app.update();
            refused |= app.world_mut().resource_mut::<Messages<MagicNews>>().drain().any(
                |news| matches!(news, MagicNews::Refused { caster: actor, .. } if actor == caster),
            );
            if refused {
                break;
            }
        }
        assert!(refused);
        let battle = fight(app.world()).unwrap();
        assert!(battle.defends(caster));
        assert_eq!(battle.this_round.party[0].phase, 0);
        let world = app.world();
        let character = world.get::<Character>(caster).unwrap();
        let carried = world.get::<Carried>(caster).unwrap();
        let load = world.get::<Burden>(caster).unwrap().load;
        let items = GameData::items(world).unwrap();
        let stats = character.base_stats();
        let baseline = attack::armour(&stats, carried, items, load, false);
        let defended = attack::armour(&stats, carried, items, load, battle.defends(caster));
        assert_eq!(defended.total, baseline.total + 2);
        assert_eq!(defended.average(), baseline.average() + 2);
        // Draft edits and a spent turn cannot undo the effective defense.
        app.world_mut().entity_mut(caster).insert(Order::Attack);
        for _ in 0..12 {
            if !fight(app.world()).unwrap().executing() {
                break;
            }
            app.update();
        }
        assert!(!fight(app.world()).unwrap().executing());
        assert!(fight(app.world()).unwrap().defends(caster));
        assert!(app.world().get::<Vitals>(caster).unwrap().stamina > before);
        app.world_mut().write_message(StartRound);
        app.update();
        assert!(!fight(app.world()).unwrap().defends(caster));
    }

    /// A spell missile flies before it lands: the cast reports at once, the
    /// damage when it arrives, and the fight's next actor waits for it.
    #[test]
    fn a_spell_missile_lands_after_its_flight() {
        let (mut app, [caster, _]) = round_app(true);
        app.add_systems(
            Update,
            crate::game::magic::deliver_spells.before(crate::game::GameplaySet::Combat),
        );
        let monster = order_spell(
            &mut app,
            caster,
            crate::content::spells::SpellDefinition { missile: Some(2), ..energy_blast() },
        );
        let mut news = Vec::new();
        for _ in 0..14 {
            app.update();
            news.extend(app.world_mut().resource_mut::<Messages<MagicNews>>().drain());
            if news.iter().any(|n| matches!(n, MagicNews::Landed { .. })) {
                break;
            }
        }
        let cast =
            news.iter().position(|n| matches!(n, MagicNews::Done(r) if r.effects.is_empty()));
        let landed = news.iter().position(|n| {
            matches!(n, MagicNews::Landed { effects, .. }
                if matches!(effects.as_slice(), [Effect::Damaged { victim: Victim::Monster(m), .. }] if *m == monster))
        });
        assert!(cast.is_some() && landed.is_some() && cast < landed, "{news:?}");
    }

    /// Impacts due in the same update land in the order they arrive, ties
    /// in launch order, whatever the entity order: one long update and many
    /// short ones give the same impacts and leave the RNG in the same state.
    #[test]
    fn due_spell_impacts_land_in_arrival_order_however_time_is_batched() {
        use crate::game::magic::Strength;
        use crate::game::magic::casting::SpellInFlight;
        use std::time::Duration;
        let run = |steps: u32| {
            let (mut app, [caster, _]) = round_app(true);
            let monster = order_spell(&mut app, caster, energy_blast());
            let level = app.world_mut().query::<&LevelOwned>().iter(app.world()).next().unwrap().0;
            // Markers: spawned in an order unlike their arrival.
            let markers: Vec<Entity> = (0..4).map(|_| app.world_mut().spawn_empty().id()).collect();
            let flights = [(0.3, 0), (0.1, 1), (0.2, 2), (0.1, 3)]
                .map(|(seconds, marker)| SpellInFlight {
                    caster: markers[marker],
                    spell: crate::content::spells::SpellId::ENERGY_BLAST,
                    strength: Strength { power: 1, against: 1, boost: 0 },
                    victims: vec![Victim::Monster(monster)],
                    arrives: Timer::from_seconds(seconds, TimerMode::Once),
                    launched: 0,
                })
                .to_vec();
            let world = app.world_mut();
            let mut commands = world.commands();
            let mut order = casting::FlightOrder::default();
            order.launch(&mut commands, flights, level);
            world.flush();
            world.resource_mut::<Messages<MagicNews>>().clear();
            for _ in 0..steps {
                world
                    .resource_mut::<Time<Virtual>>()
                    .advance_by(Duration::from_secs_f32(0.4) / steps);
                world.run_system_cached(crate::game::magic::deliver_spells).unwrap();
                world.flush();
            }
            let landed: Vec<(usize, Vec<Effect>)> = world
                .resource_mut::<Messages<MagicNews>>()
                .drain()
                .filter_map(|news| match news {
                    MagicNews::Landed { caster, effects, .. } => {
                        Some((markers.iter().position(|&m| m == caster).unwrap(), effects))
                    }
                    _ => None,
                })
                .collect();
            (landed, world.resource::<GameRng>().clone())
        };
        let (once, rng_once) = run(1);
        let (stepped, rng_stepped) = run(40);
        assert_eq!(once.iter().map(|(m, _)| *m).collect::<Vec<_>>(), [1, 3, 2, 0]);
        assert_eq!(once, stepped);
        assert_eq!(rng_once, rng_stepped);
    }

    /// Launch `flights` in a fight's world and let them all land.
    fn land(
        app: &mut App,
        flights: Vec<crate::game::magic::casting::SpellInFlight>,
    ) -> Vec<MagicNews> {
        let level = app.world_mut().query::<&LevelOwned>().iter(app.world()).next().unwrap().0;
        let world = app.world_mut();
        let mut commands = world.commands();
        casting::FlightOrder::default().launch(&mut commands, flights, level);
        world.flush();
        world.resource_mut::<Messages<MagicNews>>().clear();
        world.resource_mut::<Time<Virtual>>().advance_by(std::time::Duration::from_secs(5));
        world.run_system_cached(crate::game::magic::deliver_spells).unwrap();
        world.flush();
        world.resource_mut::<Messages<MagicNews>>().drain().collect()
    }

    fn missile(
        caster: Entity,
        spell: crate::content::spells::SpellId,
        victim: Victim,
    ) -> crate::game::magic::casting::SpellInFlight {
        crate::game::magic::casting::SpellInFlight {
            caster,
            spell,
            strength: crate::game::magic::Strength { power: 1, against: 1, boost: 0 },
            victims: vec![victim],
            arrives: Timer::from_seconds(0.1, TimerMode::Once),
            launched: 0,
        }
    }

    /// A missile that kills is credited and announced once, as an immediate
    /// kill is; one landing on the dead afterwards adds nothing.
    #[test]
    fn a_lethal_missile_counts_its_kill_once() {
        let (mut app, [caster, _]) = round_app(true);
        let monster = order_spell(
            &mut app,
            caster,
            crate::content::spells::SpellDefinition {
                dice: Some(Dice { base: 50, count: 0, sides: 0 }),
                ..energy_blast()
            },
        );
        app.world_mut().get_mut::<Monster>(monster).unwrap().hp = 1;
        app.world_mut().resource_mut::<Messages<CombatNews>>().clear();
        let id = crate::content::spells::SpellId::ENERGY_BLAST;
        let landed = land(
            &mut app,
            vec![
                missile(caster, id, Victim::Monster(monster)),
                missile(caster, id, Victim::Monster(monster)),
            ],
        );
        assert_eq!(landed.iter().filter(|n| matches!(n, MagicNews::Landed { .. })).count(), 2);
        assert_eq!(fight(app.world()).unwrap().kills, 1);
        let killed: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<CombatNews>>()
            .drain()
            .filter(|n| matches!(n, CombatNews::Killed { .. }))
            .collect();
        assert_eq!(killed, [CombatNews::Killed { actor: monster }]);
    }

    /// A member killed by magic is announced as one killed in melee is.
    #[test]
    fn a_member_killed_by_magic_is_announced() {
        let (mut app, [caster, member]) = round_app(true);
        order_spell(
            &mut app,
            caster,
            crate::content::spells::SpellDefinition {
                dice: Some(Dice { base: 1000, count: 0, sides: 0 }),
                ..energy_blast()
            },
        );
        app.world_mut().resource_mut::<Messages<CombatNews>>().clear();
        let id = crate::content::spells::SpellId::ENERGY_BLAST;
        land(&mut app, vec![missile(caster, id, Victim::Member(member))]);
        assert!(app.world().get::<Vitals>(member).unwrap().is_dead());
        let killed: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<CombatNews>>()
            .drain()
            .filter(|n| matches!(n, CombatNews::Killed { .. }))
            .collect();
        assert_eq!(killed, [CombatNews::Killed { actor: member }]);
        assert_eq!(fight(app.world()).unwrap().kills, 0, "a member is no kill");
    }

    /// Skills an impact improves (a member resisting, practising Iron Will)
    /// are reported with the landing, exactly when they improved.
    #[test]
    fn a_landed_spell_reports_the_skills_its_impact_improved() {
        use crate::content::characters::{Attribute, Skill};
        let mut reported = 0;
        for seed in 0..40 {
            let (mut app, [caster, member]) = round_app(true);
            order_spell(
                &mut app,
                caster,
                crate::content::spells::SpellDefinition {
                    id: crate::content::spells::SpellId::SLEEP,
                    realm: Some(crate::content::characters::Realm::Mental),
                    ..energy_blast()
                },
            );
            {
                let mut character = app.world_mut().get_mut::<Character>(member).unwrap();
                character.attributes[Attribute::Piety] = 100;
                character.level = 30;
                character.skill_practice[Skill::IronWill] = 7;
            }
            app.insert_resource(GameRng::seeded(seed));
            let landed = land(
                &mut app,
                vec![missile(
                    caster,
                    crate::content::spells::SpellId::SLEEP,
                    Victim::Member(member),
                )],
            );
            let improved =
                app.world().get::<Character>(member).unwrap().skill_points[Skill::IronWill] > 0;
            let [MagicNews::Landed { improved: report, .. }] = landed.as_slice() else {
                panic!("{landed:?}")
            };
            assert_eq!(!report.is_empty(), improved, "seed {seed}");
            if improved {
                assert_eq!(report, &[(member, Skill::IronWill)]);
                reported += 1;
            }
        }
        assert!(reported > 0, "no impact improved a skill");
    }

    #[test]
    fn pace_requests_cycle_current_state_and_do_not_survive_a_battle() {
        let (mut app, _) = round_app(false);
        app.world_mut().resource_mut::<Messages<StartRound>>().clear();
        app.world_mut().write_message(CyclePartyMove);
        app.world_mut().write_message(CyclePartyMove);
        app.update();
        assert_eq!(fight(app.world()).unwrap().party_pace, Some(PartyPace::Run));
        app.world_mut().write_message(CyclePartyMove);
        app.update();
        assert_eq!(fight(app.world()).unwrap().party_pace, None);
        let battle =
            app.world_mut().query_filtered::<Entity, With<Battle>>().single(app.world()).unwrap();
        let state = app.world_mut().entity_mut(battle).take::<Battle>().unwrap();
        app.world_mut().write_message(CyclePartyMove);
        app.update();
        app.world_mut().entity_mut(battle).insert(state);
        app.update();
        assert_eq!(fight(app.world()).unwrap().party_pace, None);
    }

    /// A craven monster runs for its life at its turn, away from the party
    /// by its whole move, and runs again next round while the party stays
    /// off (retail `MonsterAdvanceChance`).
    #[test]
    fn a_craven_monster_runs_from_the_party() {
        let (mut app, _) = round_app(true);
        let monster =
            app.world().try_query::<&Combatants>().unwrap().single(app.world()).unwrap().entities()
                [0];
        app.world_mut().entity_mut(monster).insert((
            Walker { speed: 2.0, float_height: 1.0 },
            Transform::from_translation(Vec3::NEG_Z * 20.0),
        ));
        let data = app.world().resource::<GameData>().species.clone();
        {
            let mut catalogs = app.world_mut().resource_mut::<Assets<MonsterCatalog>>();
            let mut species = catalogs.get_mut(&data).unwrap();
            species.0[0].combat_move_range = 1.0;
            species.0[0].temper.craven = true;
        }
        let mut ran = false;
        for _ in 0..8 {
            app.update();
            let news: Vec<_> =
                app.world_mut().resource_mut::<Messages<CombatNews>>().drain().collect();
            if news.contains(&CombatNews::Flees { monster }) {
                ran = true;
                break;
            }
        }
        assert!(ran, "it runs");
        let order = app.world().get::<approach::CombatMove>(monster).unwrap();
        assert_eq!(order.budget, 20.0);
        let away = order.away.expect("away from the party");
        assert!(away.z < -39.0, "a whole move further off: {away}");
        assert!(fight(app.world()).unwrap().fleeing.contains(&monster));
    }

    /// A monster with nothing in reach spends its phase advancing, by its
    /// own budget, and the fight waits for the move.
    #[test]
    fn a_monster_out_of_reach_advances_as_its_action() {
        let (mut app, _) = round_app(true);
        let monster =
            app.world().try_query::<&Combatants>().unwrap().single(app.world()).unwrap().entities()
                [0];
        app.world_mut().entity_mut(monster).insert((
            Walker { speed: 2.0, float_height: 1.0 },
            Transform::from_translation(Vec3::NEG_Z * 20.0),
        ));
        let data = app.world().resource::<GameData>().species.clone();
        {
            let mut catalogs = app.world_mut().resource_mut::<Assets<MonsterCatalog>>();
            let mut species = catalogs.get_mut(&data).unwrap();
            species.0[0].combat_move_range = 1.0;
            species.0[0].attacks = vec![crate::content::species::MonsterAttack {
                word: AttackWord::Verb,
                weapon: crate::content::species::MonsterWeapon::Unarmed,
                effects: default(),
                reach: Reach::Touch,
                score: 50,
                modes: EnumSet::only(AttackMode::Bash),
                damage: Dice { base: 1, count: 1, sides: 2 },
            }];
        }
        let mut moved = None;
        for _ in 0..8 {
            app.update();
            let news: Vec<_> =
                app.world_mut().resource_mut::<Messages<CombatNews>>().drain().collect();
            if let Some(n) = news.into_iter().find(|n| matches!(n, CombatNews::Moves { .. })) {
                moved = Some(n);
                break;
            }
        }
        assert_eq!(moved, Some(CombatNews::Moves { monster, charge: false }));
        let order = app.world().get::<approach::CombatMove>(monster).unwrap();
        assert_eq!(order.budget, 20.0);
        let phase = fight(app.world()).unwrap().phase;
        app.update();
        assert_eq!(fight(app.world()).unwrap().phase, phase, "the round waits for the move");
    }

    /// A party move holds the members until its phase, lets the party walk
    /// its budget, and a move spent in full costs the members their actions
    /// (retail `CompletePartyMovementTurns`).
    #[test]
    fn a_spent_party_move_costs_the_members_their_actions() {
        let (mut app, _) = round_app(false);
        app.world_mut().write_message(CyclePartyMove);
        let mut news = Vec::new();
        for _ in 0..12 {
            app.update();
            news.extend(app.world_mut().resource_mut::<Messages<CombatNews>>().drain());
            if news.contains(&CombatNews::PartyMoves) {
                break;
            }
        }
        assert!(news.contains(&CombatNews::PartyMoves), "{news:?}");
        assert!(fight(app.world()).unwrap().party_may_move());
        let body = app
            .world_mut()
            .query_filtered::<Entity, With<PartyBody>>()
            .single(app.world())
            .unwrap();
        // Ground covered from the moment the move begins counts, however
        // the frames fall.
        app.world_mut().get_mut::<Position>(body).unwrap().0 += Vec3::X * 21.0;
        app.update();
        news.extend(app.world_mut().resource_mut::<Messages<CombatNews>>().drain());
        assert!(news.contains(&CombatNews::PartyStops));
        assert!(!fight(app.world()).unwrap().party_may_move());
        let pending =
            fight(app.world()).unwrap().this_round.party.iter().filter(|p| p.phase > 0).count();
        assert_eq!(pending, 0, "a move spent in full keeps nobody's action");
    }

    /// A monster put to sleep loses its actions until the condition runs out
    /// with the rounds.
    #[test]
    fn sleep_holds_a_monster_until_its_rounds_run_out() {
        use crate::content::spells::{SpellDefinition, SpellId, Targeting, Usable};
        let (mut app, [caster, _]) = round_app(true);
        let sleep = SpellDefinition {
            id: SpellId::SLEEP,
            name: "Sleep".into(),
            realm: Some(crate::content::characters::Realm::Air),
            level: 1,
            cost: 3,
            reach: Reach::Long,
            targeting: Targeting::EnemyGroup,
            usable: Usable::InCombat,
            duration_per_power: 1,
            ..default()
        };
        let monster = order_spell(&mut app, caster, sleep);
        let asleep = |app: &App| {
            app.world()
                .get::<Monster>(monster)
                .unwrap()
                .conditions
                .iter()
                .any(|&(c, _)| c == Condition::Asleep)
        };
        let mut slept = false;
        for _ in 0..6 {
            app.update();
            slept |= asleep(&app);
        }
        assert!(slept, "the monster failed its save against a strong caster");
        app.world_mut().entity_mut(caster).insert(Order::Defend);
        app.world_mut().resource_mut::<CombatSettings>().continuous = true;
        for _ in 0..40 {
            if !asleep(&app) {
                break;
            }
            app.update();
        }
        assert!(!asleep(&app), "sleep wears off as rounds pass");
    }

    fn attacks(app: &mut App) -> Vec<String> {
        let actors: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<CombatNews>>()
            .drain()
            .filter_map(|n| match n {
                CombatNews::Attack { attacker, .. } => Some(attacker),
                _ => None,
            })
            .collect();
        actors
            .into_iter()
            .filter_map(|a| app.world().get::<Character>(a).map(|c| c.name.clone()))
            .collect()
    }

    #[test]
    fn passive_rounds_use_current_actor_reach_without_waiting_for_navigation() {
        let (mut app, _) = round_app(true);
        let actor =
            app.world_mut().query::<&Combatants>().single(app.world()).unwrap().entities()[0];
        app.world_mut().entity_mut(actor).insert((
            Walker { speed: 2.0, float_height: 1.0 },
            Transform::from_translation(Vec3::NEG_Z * 20.0),
            bevy_landmass::prelude::AgentTarget3d::None,
        ));
        {
            let mut battle = fight_mut(app.world_mut());
            battle.phase = RoundPhase::Executing(101);
            battle.passive_rounds = 1;
        }
        app.update();
        assert!(fight(app.world()).is_some(), "a living walker is still approaching");
        assert!(
            matches!(
                app.world().get::<bevy_landmass::prelude::AgentTarget3d>(actor),
                Some(bevy_landmass::prelude::AgentTarget3d::None)
            ),
            "movement starts only when its combat action is accepted"
        );
        assert_eq!(fight(app.world()).unwrap().passive_rounds, 0);
        app.world_mut().get_mut::<Transform>(actor).unwrap().translation = Vec3::NEG_Z;
        {
            let mut battle = fight_mut(app.world_mut());
            battle.phase = RoundPhase::Executing(101);
            battle.passive_rounds = 1;
            battle.wait = default();
        }
        app.update();
        assert!(fight(app.world()).is_none(), "the actor is now in reach and idle");
        // The fight's end ends its participation and the movement it gave,
        // not the actor.
        assert!(app.world().get::<Monster>(actor).is_some());
        assert!(app.world().get::<Combatant>(actor).is_none());
        assert!(matches!(
            app.world().get::<bevy_landmass::prelude::AgentTarget3d>(actor),
            Some(bevy_landmass::prelude::AgentTarget3d::None)
        ));
    }

    #[test]
    fn a_round_recognises_only_its_participants_before_the_session_ends() {
        use crate::content::level::LevelDefinition;
        use crate::game::characters::MonsterKnowledge;
        use crate::game::world::identify::{GroupMember, MonsterGroup};
        use crate::game::world::level::Level;
        let (mut app, members) = round_app(true);
        for member in members {
            app.world_mut().entity_mut(member).insert(Order::Defend);
        }
        let level =
            app.world_mut().query_filtered::<Entity, With<Level>>().single(app.world()).unwrap();
        let geometry = app
            .world_mut()
            .resource_mut::<Assets<LevelDefinition>>()
            .add(LevelDefinition::default());
        app.world_mut().get_mut::<Level>(level).unwrap().definition = geometry;
        let monster = app.world_mut().query::<(Entity, &Monster)>().single(app.world()).unwrap().0;
        let group = app
            .world_mut()
            .spawn((MonsterGroup { authored: 1, ..default() }, LevelOwned(level)))
            .id();
        app.world_mut().entity_mut(monster).insert(GroupMember(group));
        let outsider_group = app
            .world_mut()
            .spawn((MonsterGroup { authored: 2, ..default() }, LevelOwned(level)))
            .id();
        let outsider = app.world().get::<Monster>(monster).unwrap().clone();
        // Far out of sight, so it has no part in the fight.
        app.world_mut().spawn((
            outsider,
            Transform::from_translation(Vec3::NEG_Z * 500.0),
            GroupMember(outsider_group),
            LevelOwned(level),
        ));
        app.world_mut().resource_mut::<CombatSettings>().continuous = true;
        for _ in 0..12 {
            app.update();
            if fight(app.world()).is_none() {
                break;
            }
        }
        assert!(fight(app.world()).is_none(), "a passive fight ends");
        for member in members {
            assert_eq!(
                app.world().get::<MonsterKnowledge>(member).unwrap().seen.get(&0),
                Some(&1),
                "round recognition counts battle participants once"
            );
        }
        assert!(
            !app.world().get::<MonsterGroup>(outsider_group).unwrap().counted,
            "ending the battle cannot broaden its pending recognition to outsiders"
        );
    }

    /// Attacking one member of a neutral group turns that group hostile,
    /// whatever the blows did, and brings the rest of it into the fight; a
    /// neutral group nobody acted against keeps out (retail
    /// `MakeTargetGroupHostile`).
    #[test]
    fn acting_against_a_neutral_group_brings_it_alone_into_the_fight() {
        use crate::content::species::{Disposition, Species};
        use crate::game::world::hostility::{SetStance, Stance};
        use crate::game::world::identify::{GroupMember, MonsterGroup};
        use crate::game::world::level::Level;
        let (mut app, _) = round_app(true);
        define_level(&mut app);
        let species = app.world().resource::<GameData>().species.clone();
        app.world_mut().resource_mut::<Assets<MonsterCatalog>>().get_mut(&species).unwrap().0.push(
            Species { name: "Bystander".into(), disposition: Disposition::Neutral, ..default() },
        );
        let level =
            app.world_mut().query_filtered::<Entity, With<Level>>().single(app.world()).unwrap();
        let target = app.world_mut().query::<(Entity, &Monster)>().single(app.world()).unwrap().0;
        app.world_mut().get_mut::<Monster>(target).unwrap().species = 1;
        let group = |app: &mut App, authored| {
            app.world_mut().spawn((MonsterGroup { authored, ..default() }, LevelOwned(level))).id()
        };
        let (attacked, untouched) = (group(&mut app, 1), group(&mut app, 2));
        app.world_mut().entity_mut(target).insert(GroupMember(attacked));
        let bystander = |app: &mut App, group, location| {
            let mut monster = app.world().get::<Monster>(target).unwrap().clone();
            monster.location = location;
            app.world_mut()
                .spawn((
                    monster,
                    Transform::from_translation(Vec3::NEG_Z * 30.0),
                    GroupMember(group),
                    LevelOwned(level),
                ))
                .id()
        };
        let companion = bystander(&mut app, attacked, 1);
        let stranger = bystander(&mut app, untouched, 2);
        for _ in 0..12 {
            app.update();
            if app.world().get::<Combatant>(companion).is_some() {
                break;
            }
        }
        let set = app.world().get::<MonsterGroup>(attacked).unwrap().set;
        assert!(matches!(set, Some(SetStance { stance: Stance::Hostile, .. })));
        assert!(app.world().get::<Combatant>(companion).is_some(), "the group joins the fight");
        assert_eq!(app.world().get::<MonsterGroup>(untouched).unwrap().set, None);
        assert!(app.world().get::<Combatant>(stranger).is_none());
    }

    /// A caster in the fight with a wall between it and the party has no
    /// aim at the party, however close; with the wall gone it has (retail
    /// `CollectMonsterSpellTargets`' sight gate).
    #[test]
    fn cover_keeps_a_caster_from_aiming_at_the_party() {
        use crate::content::species::{MonsterMagic, Species};
        use crate::content::spells::{SpellCatalog, SpellId, Targeting};
        use crate::game::characters::{Gender, Profession, Race};
        use crate::game::world::obstacles::scenery;
        use crate::game::world::population::ScaledBody;
        use avian3d::prelude::{Collider, PhysicsPlugins, RigidBody};
        use bevy::ecs::system::RunSystemOnce;
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            TransformPlugin,
            PhysicsPlugins::default(),
        ));
        app.finish();
        app.cleanup();
        let character = Character::new("A", Race::Human, Gender::Male, Profession::Fighter);
        let vitals = Vitals::new(&character);
        let member = app
            .world_mut()
            .spawn((
                character,
                vitals,
                Carried::default(),
                crate::game::inventory::Burden::default(),
            ))
            .id();
        crate::game::party::roster::test_party(app.world_mut(), &[member]);
        let level = app.world_mut().spawn_empty().id();
        let caster = app
            .world_mut()
            .spawn((
                Monster {
                    enchantments: default(),
                    location: 0,
                    species: 0,
                    hp: 10,
                    max_hp: 10,
                    stamina: 10,
                    max_stamina: 10,
                    conditions: vec![],
                    spell_points: 0,
                    body: ScaledBody { scale: 1.0, radius: 0.5, eye_height: 1.5, speed: 0.0 },
                },
                Transform::from_xyz(0.0, 0.0, -10.0).looking_at(Vec3::new(0.0, 0.0, 0.0), Vec3::Y),
                LevelOwned(level),
            ))
            .id();
        let wall = app
            .world_mut()
            .spawn((
                RigidBody::Static,
                Collider::cuboid(8.0, 8.0, 0.2),
                scenery(),
                Transform::from_xyz(0.0, 0.0, -5.0),
            ))
            .id();
        app.update();
        app.update();
        let mut spells = [None; 10];
        spells[0] = Some(SpellId::ENERGY_BLAST);
        let kind = Species { magic: MonsterMagic { chance: 100, spells, budget: 50 }, ..default() };
        let blast = crate::content::spells::SpellDefinition {
            monsters_cast: true,
            targeting: Targeting::Enemy,
            ..energy_blast()
        };
        let mut catalog: Vec<_> = (0..=blast.id.0).map(|_| default()).collect();
        let slot = usize::from(blast.id.0);
        catalog[slot] = blast;
        let catalog = std::sync::Arc::new(SpellCatalog(catalog));
        let aim = |app: &mut App| {
            let kind = kind.clone();
            let catalog = std::sync::Arc::clone(&catalog);
            app.world_mut()
                .run_system_once(
                    move |members: Members,
                          monsters: MonsterQuery,
                          obstacles: WorldObstacles,
                          roster: Single<&PartyRoster>| {
                        let party = PartyView { eye: Vec3::Y * 1.5, yaw: 0.0, far: 50.0, now: 0 };
                        let sight = party.sight(&members, &roster, &obstacles);
                        let scene = spell_scene(
                            (caster, &kind),
                            (&[caster], party_of(&members, &roster)),
                            &monsters,
                            &sight,
                            &|_| hostility::Stance::Hostile,
                        );
                        let (body, ..) = monsters.get(caster).unwrap();
                        let mut rng = GameRng::seeded(1);
                        (
                            scene.sees_party,
                            monster_spells::choose(&mut rng, body, &kind, 100, &catalog, &scene),
                        )
                    },
                )
                .unwrap()
        };
        assert_eq!(aim(&mut app), (false, None), "the wall hides the party");
        app.world_mut().despawn(wall);
        app.update();
        let (sees, chosen) = aim(&mut app);
        assert!(sees);
        assert_eq!(chosen, Some((SpellId::ENERGY_BLAST, monster_spells::SpellAim::Member(member))));
    }

    /// A neutral member counts only standing in the party's sight, within
    /// the engagement range, roughly between the party and a hostile
    /// fighter that sees it, and not camouflaged.
    #[test]
    fn a_neutral_group_reinforces_only_between_the_party_and_a_hostile() {
        let at = |entity: u64, z: f32, in_view: bool| Seen {
            entity: Entity::from_bits(entity),
            at: Vec3::NEG_Z * z,
            from_party: z,
            sees: false,
            in_view,
        };
        let hostile = [at(1, 10.0, true)];
        assert!(reinforces(&[(at(2, 5.0, true), false)], &hostile));
        assert!(!reinforces(&[(at(2, 5.0, true), true)], &hostile), "camouflaged");
        assert!(!reinforces(&[(at(2, 5.0, false), false)], &hostile), "out of sight");
        assert!(!reinforces(&[(at(2, 15.0, true), false)], &hostile), "behind the hostile");
        assert!(!reinforces(&[(at(2, 5.0, true), false)], &[at(1, 10.0, false)]));
        let far = [at(1, 70.0, true)];
        assert!(!reinforces(&[(at(2, 60.0, true), false)], &far), "beyond the engagement range");
        assert!(!reinforces(&[(at(2, 5.0, true), false)], &[]), "nobody hostile fights");
    }

    /// A fighting hostile group turns hostile the group of its faction
    /// whose leader sees its leader, which then joins the fight; a group of
    /// another faction is untouched (retail `AlertSameFactionGroups`).
    #[test]
    fn a_fighting_group_alerts_its_faction() {
        use crate::content::species::Species;
        use crate::game::factions::{RAPAX_TEMPLAR, RATTKIN_COMMON};
        use crate::game::world::hostility::{SetStance, Stance};
        use crate::game::world::identify::{GroupMember, MonsterGroup};
        use crate::game::world::level::Level;
        let (mut app, _) = round_app(true);
        define_level(&mut app);
        let species = app.world().resource::<GameData>().species.clone();
        {
            let mut catalogs = app.world_mut().resource_mut::<Assets<MonsterCatalog>>();
            let catalog = &mut catalogs.get_mut(&species).unwrap().0;
            catalog.push(Species { name: "Rattkin".into(), faction: RATTKIN_COMMON, ..default() });
            catalog.push(Species { name: "Templar".into(), faction: RAPAX_TEMPLAR, ..default() });
        }
        let level =
            app.world_mut().query_filtered::<Entity, With<Level>>().single(app.world()).unwrap();
        let target = app.world_mut().query::<(Entity, &Monster)>().single(app.world()).unwrap().0;
        app.world_mut().get_mut::<Monster>(target).unwrap().species = 1;
        let group = |app: &mut App, authored, set| {
            app.world_mut()
                .spawn((MonsterGroup { authored, set, ..default() }, LevelOwned(level)))
                .id()
        };
        let fighting = group(&mut app, 1, Some(SetStance { stance: Stance::Hostile, at: 0 }));
        let (kin, stranger_group) = (group(&mut app, 2, None), group(&mut app, 3, None));
        app.world_mut().entity_mut(target).insert(GroupMember(fighting));
        let watcher = |app: &mut App, group, species, location, at: Vec3| {
            let mut monster = app.world().get::<Monster>(target).unwrap().clone();
            monster.species = species;
            monster.location = location;
            app.world_mut()
                .spawn((
                    monster,
                    // Facing the fighting leader.
                    Transform::from_translation(at).looking_at(Vec3::NEG_Z, Vec3::Y),
                    GroupMember(group),
                    LevelOwned(level),
                ))
                .id()
        };
        let rattkin = watcher(&mut app, kin, 1, 1, Vec3::NEG_Z * 4.0);
        // Off to the side, so it stands between the party and no hostile.
        let templar = watcher(&mut app, stranger_group, 2, 2, Vec3::new(3.0, 0.0, -4.0));
        for _ in 0..6 {
            app.update();
            if app.world().get::<Combatant>(rattkin).is_some() {
                break;
            }
        }
        let set = app.world().get::<MonsterGroup>(kin).unwrap().set;
        assert!(matches!(set, Some(SetStance { stance: Stance::Hostile, .. })), "alerted");
        assert!(app.world().get::<Combatant>(rattkin).is_some(), "and it joins the fight");
        assert_eq!(app.world().get::<MonsterGroup>(stranger_group).unwrap().set, None);
        assert!(app.world().get::<Combatant>(templar).is_none());
    }

    /// A neutral group standing in the party's view between it and a
    /// hostile fighter joins the running fight, without turning hostile,
    /// whether or not it sees the party itself (retail
    /// `MonsterGroupHasReinforcement`'s party line of sight).
    #[test]
    fn a_neutral_group_between_the_party_and_a_foe_joins_the_fight() {
        use crate::content::species::{Disposition, Species};
        use crate::game::world::identify::{GroupMember, MonsterGroup};
        use crate::game::world::level::Level;
        let (mut app, _) = round_app(true);
        define_level(&mut app);
        let species = app.world().resource::<GameData>().species.clone();
        app.world_mut().resource_mut::<Assets<MonsterCatalog>>().get_mut(&species).unwrap().0.push(
            Species { name: "Bystander".into(), disposition: Disposition::Neutral, ..default() },
        );
        let level =
            app.world_mut().query_filtered::<Entity, With<Level>>().single(app.world()).unwrap();
        let foe = app.world_mut().query::<(Entity, &Monster)>().single(app.world()).unwrap().0;
        let facing =
            |z: f32| Transform::from_translation(Vec3::NEG_Z * z).looking_at(Vec3::ZERO, Vec3::Y);
        app.world_mut().entity_mut(foe).insert(facing(10.0));
        let group = app
            .world_mut()
            .spawn((MonsterGroup { authored: 1, ..default() }, LevelOwned(level)))
            .id();
        let mut bystander = app.world().get::<Monster>(foe).unwrap().clone();
        bystander.species = 1;
        bystander.location = 1;
        let bystander = app
            .world_mut()
            // Facing away and without Senses, it does not see the party;
            // the party's line to it is what counts.
            .spawn((
                bystander,
                Transform::from_translation(Vec3::NEG_Z * 5.0),
                GroupMember(group),
                LevelOwned(level),
            ))
            .id();
        for _ in 0..4 {
            app.update();
            if app.world().get::<Combatant>(bystander).is_some() {
                break;
            }
        }
        assert!(app.world().get::<Combatant>(bystander).is_some(), "it stands with the fight");
        assert_eq!(app.world().get::<MonsterGroup>(group).unwrap().set, None, "still neutral");
    }

    /// A spell that kills its target outright still provokes the target's
    /// group: whether it provokes is settled as the cast is accepted, and
    /// the surviving member joins the fight.
    #[test]
    fn a_lethal_opening_spell_still_provokes_the_victims_group() {
        use crate::content::species::{Disposition, Species};
        use crate::game::world::hostility::{SetStance, Stance};
        use crate::game::world::identify::{GroupMember, MonsterGroup};
        use crate::game::world::level::Level;
        let (mut app, [caster, _]) = round_app(true);
        define_level(&mut app);
        let species = app.world().resource::<GameData>().species.clone();
        app.world_mut().resource_mut::<Assets<MonsterCatalog>>().get_mut(&species).unwrap().0.push(
            Species { name: "Bystander".into(), disposition: Disposition::Neutral, ..default() },
        );
        let level =
            app.world_mut().query_filtered::<Entity, With<Level>>().single(app.world()).unwrap();
        let target = order_spell(&mut app, caster, energy_blast());
        {
            let mut monster = app.world_mut().get_mut::<Monster>(target).unwrap();
            monster.species = 1;
            monster.hp = 1;
        }
        let group = app
            .world_mut()
            .spawn((MonsterGroup { authored: 1, ..default() }, LevelOwned(level)))
            .id();
        app.world_mut().entity_mut(target).insert(GroupMember(group));
        let mut survivor = app.world().get::<Monster>(target).unwrap().clone();
        survivor.location = 1;
        survivor.hp = 10;
        let survivor = app
            .world_mut()
            .spawn((
                survivor,
                // Close and facing the party, so its group does not leave the
                // fight as the round ends; it has no attack to join by sight.
                Transform::from_translation(Vec3::NEG_Z * 3.0).looking_at(Vec3::ZERO, Vec3::Y),
                GroupMember(group),
                LevelOwned(level),
            ))
            .id();
        for _ in 0..12 {
            app.update();
            if app.world().get::<Combatant>(survivor).is_some() {
                break;
            }
        }
        assert!(app.world().get::<Monster>(target).unwrap().is_dead(), "the spell kills");
        let set = app.world().get::<MonsterGroup>(group).unwrap().set;
        assert!(matches!(set, Some(SetStance { stance: Stance::Hostile, .. })));
        assert!(app.world().get::<Combatant>(survivor).is_some(), "the survivor joins");
    }

    #[test]
    fn every_executed_swing_costs_stamina_and_exhaustion_stops_the_attack() {
        let run = |stamina| {
            let (mut app, [actor, _]) = round_app(false);
            app.world_mut().get_mut::<Vitals>(actor).unwrap().stamina = stamina;
            app.update();
            let news: Vec<_> =
                app.world_mut().resource_mut::<Messages<CombatNews>>().drain().collect();
            let announced = news
                .iter()
                .find_map(|n| match n {
                    CombatNews::Attack { attacker, swings, .. } if *attacker == actor => {
                        Some(*swings)
                    }
                    _ => None,
                })
                .unwrap();
            let target = app.world_mut().query::<&Monster>().single(app.world()).unwrap();
            (announced, app.world().get::<Vitals>(actor).unwrap().clone(), target.hp)
        };
        // The fixture's weight-zero sword costs two points per swing.
        let (swings, rested, _) = run(100);
        assert!(swings > 1, "exercise a multi-swing attack");
        assert_eq!(rested.stamina, 100 - 2 * swings);
        let (_, exhausted, hp) = run(1);
        assert_eq!(exhausted.stamina, 0);
        assert!(exhausted.has(Condition::Unconscious));
        assert!(hp >= 9999, "exhaustion allows at most one one-damage swing");
    }

    #[test]
    fn incapacitated_attackers_recover_at_round_end_without_defending() {
        use crate::game::characters::Lasting;
        for condition in [Condition::Asleep, Condition::Unconscious, Condition::Paralyzed] {
            let (mut app, [_, actor]) = round_app(false);
            let before = 1;
            {
                let mut vitals = app.world_mut().get_mut::<Vitals>(actor).unwrap();
                vitals.stamina = before;
                vitals.inflict(condition, Lasting::Indefinite);
            }
            app.update();
            for _ in 0..8 {
                app.update();
                if !fight(app.world()).unwrap().executing() {
                    break;
                }
            }
            assert!(!fight(app.world()).unwrap().executing());
            let after = app.world().get::<Vitals>(actor).unwrap();
            if condition == Condition::Paralyzed {
                assert_eq!(after.stamina, before);
            } else {
                assert!(after.stamina > before);
            }
        }
    }

    #[test]
    fn accepted_attacks_follow_actors_through_roster_reordering_and_order_edits() {
        let (mut app, [a, b]) = round_app(false);
        app.update();
        assert_eq!(attacks(&mut app), ["A"]);
        crate::game::party::roster::test_reorder(app.world_mut(), &[b, a]);
        app.world_mut().entity_mut(b).insert(Order::Defend);
        app.update();
        assert_eq!(
            attacks(&mut app),
            ["B"],
            "editing the next order cannot retarget or replace accepted work"
        );
    }

    #[test]
    fn an_order_edited_during_the_pacing_delay_waits_for_the_next_round() {
        let (mut app, [a, _]) = round_app(false);
        fight_mut(app.world_mut()).wait = Timer::from_seconds(60.0, TimerMode::Once);
        app.update();
        assert!(attacks(&mut app).is_empty(), "accepted, but the delay holds execution");
        app.world_mut().entity_mut(a).insert(Order::Defend);
        fight_mut(app.world_mut()).wait = default();
        app.update();
        assert_eq!(attacks(&mut app), ["A"], "the accepted round keeps A's attack");
    }

    #[test]
    fn a_replacement_member_cannot_inherit_departed_actors_accepted_work() {
        use crate::game::characters::{Gender, Profession, Race};
        let (mut app, [_, b]) = round_app(false);
        app.update();
        assert_eq!(attacks(&mut app), ["A"]);
        let profile = Character::new("Replacement", Race::Human, Gender::Male, Profession::Fighter);
        let replacement = app
            .world_mut()
            .spawn((Vitals::new(&profile), profile, Carried::default(), Order::Attack))
            .id();
        let mut params = bevy::ecs::system::SystemState::<(Commands, Single<&mut Formation>)>::new(
            app.world_mut(),
        );
        let (mut commands, mut formation) = params.get_mut(app.world_mut()).unwrap();
        crate::game::party::roster::leave(&mut commands, b, &mut formation);
        params.apply(app.world_mut());
        crate::game::party::roster::test_join(app.world_mut(), replacement);
        app.update();
        assert!(attacks(&mut app).is_empty());
        assert!(
            app.world().get::<Character>(b).is_some(),
            "a retained profile is no longer a member"
        );
        let formation = app.world_mut().query::<&Formation>().single(app.world()).unwrap();
        assert!(formation.place_of(b).is_none());
    }

    #[test]
    fn accepted_defense_recovers_stamina_and_an_edit_only_changes_the_next_round() {
        let (mut app, [_, b]) = round_app(true);
        app.world_mut().get_mut::<Vitals>(b).unwrap().stamina -= 10;
        let before = app.world().get::<Vitals>(b).unwrap().stamina;
        app.update();
        assert_eq!(attacks(&mut app), ["A"]);
        app.world_mut().entity_mut(b).insert(Order::Attack);
        app.update();
        assert!(attacks(&mut app).is_empty());
        for _ in 0..4 {
            if !fight(app.world()).unwrap().executing() {
                break;
            }
            app.update();
        }
        assert!(attacks(&mut app).is_empty());
        assert!(app.world().get::<Vitals>(b).unwrap().stamina > before);
        assert!(!fight(app.world()).unwrap().executing());
        app.world_mut().write_message(StartRound);
        app.update();
        assert_eq!(attacks(&mut app), ["A"]);
        app.update();
        assert_eq!(attacks(&mut app), ["B"]);
    }
    #[test]
    fn a_round_request_survives_its_pacing_delay() {
        let (mut app, _) = round_app(false);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(100),
        ));
        fight_mut(app.world_mut()).wait = Timer::from_seconds(1.0, TimerMode::Once);
        app.update();
        assert!(fight(app.world()).unwrap().executing());
        let mut announced = Vec::new();
        for _ in 0..12 {
            app.update();
            announced.extend(attacks(&mut app));
        }
        assert_eq!(fight(app.world()).unwrap().round, 1);
        assert_eq!(announced, ["A"]);
    }

    #[test]
    fn defeat_settles_without_round_input_content_or_an_expired_timer() {
        let (mut app, members) = round_app(false);
        fight_mut(app.world_mut()).wait = Timer::from_seconds(60.0, TimerMode::Once);
        app.world_mut().resource_mut::<Messages<StartRound>>().clear();
        let data = app.world_mut().remove_resource::<GameData>().unwrap();
        for member in members {
            app.world_mut().get_mut::<Vitals>(member).unwrap().hp = 0;
        }
        app.update();
        assert!(fight(app.world()).is_none());
        assert_eq!(
            app.world_mut().resource_mut::<Messages<CombatNews>>().drain().collect::<Vec<_>>(),
            [CombatNews::Lost]
        );
        app.update();
        assert!(app.world().resource::<Messages<CombatNews>>().is_empty());

        // A visible hostile must not restart combat once content is ready again.
        {
            let world = app.world_mut();
            let geometry = world
                .resource_mut::<Assets<crate::content::level::LevelDefinition>>()
                .add(crate::content::level::LevelDefinition::default());
            world
                .query::<&mut crate::game::world::level::Level>()
                .single_mut(world)
                .unwrap()
                .definition = geometry;
            world.resource_mut::<Assets<MonsterCatalog>>().get_mut(&data.species).unwrap().0[0]
                .attacks
                .push(crate::content::species::MonsterAttack {
                    word: AttackWord::Verb,
                    weapon: crate::content::species::MonsterWeapon::Unarmed,
                    effects: default(),
                    reach: Reach::Touch,
                    score: 0,
                    modes: EnumSet::only(AttackMode::Swing),
                    damage: Dice::default(),
                });
            world.insert_resource(data);
        }
        app.update();
        assert!(fight(app.world()).is_none());
        assert!(app.world().resource::<Messages<CombatNews>>().is_empty());
        app.world_mut().get_mut::<Vitals>(members[0]).unwrap().hp = 1;
        app.update();
        assert!(fight(app.world()).is_some(), "the hostile can engage a live party");
    }

    #[test]
    fn a_despawned_monster_cannot_leave_the_battle_stuck_on_its_turn() {
        let (mut app, _) = round_app(false);
        app.update();
        assert_eq!(attacks(&mut app), ["A"]);
        let entity =
            app.world_mut().query_filtered::<Entity, With<Monster>>().single(app.world()).unwrap();
        app.world_mut().despawn(entity);
        for _ in 0..4 {
            app.update();
            if fight(app.world()).is_none() {
                return;
            }
        }
        panic!("the removed actor's turn prevented battle completion");
    }

    #[test]
    fn an_actor_incapacitated_after_acceptance_cannot_execute_its_attack() {
        let (mut app, [_, b]) = round_app(false);
        app.update();
        assert_eq!(attacks(&mut app), ["A"]);
        app.world_mut()
            .get_mut::<Vitals>(b)
            .unwrap()
            .inflict(Condition::Asleep, crate::game::characters::Lasting::Rounds(2));
        app.update();
        assert!(attacks(&mut app).is_empty());
    }
    #[test]
    fn reordering_the_roster_does_not_reassign_accepted_defender_recovery() {
        use crate::game::characters::{Gender, Profession, Race};
        let run = |reorder| {
            let (mut app, [a, b]) = round_app(true);
            let character = Character::new("C", Race::Human, Gender::Male, Profession::Fighter);
            let mut vitals = Vitals::new(&character);
            vitals.stamina = 1;
            let c =
                app.world_mut().spawn((character, vitals, Carried::default(), Order::Defend)).id();
            crate::game::party::roster::test_join(app.world_mut(), c);
            app.world_mut().get_mut::<Vitals>(b).unwrap().stamina = 1;
            app.update();
            if reorder {
                crate::game::party::roster::test_reorder(app.world_mut(), &[c, a, b]);
            }
            for _ in 0..4 {
                app.update();
            }
            [b, c].map(|actor| app.world().get::<Vitals>(actor).unwrap().stamina)
        };
        let unchanged = run(false);
        assert!(unchanged.into_iter().all(|stamina| stamina > 1));
        assert_eq!(run(true), unchanged);
    }

    /// Run the accepted round to its end.
    fn finish_round(app: &mut App) -> Vec<CombatNews> {
        let mut news = Vec::new();
        for _ in 0..40 {
            app.update();
            news.extend(app.world_mut().resource_mut::<Messages<CombatNews>>().drain());
            if !fight(app.world()).unwrap().executing() {
                break;
            }
        }
        assert!(!fight(app.world()).unwrap().executing());
        news
    }

    /// A second weapon in the off hand takes its own attacks in the round,
    /// beside the main hand's (retail `CalcAttacks`, `ComputeCharacterActionPhase`).
    #[test]
    fn an_off_hand_weapon_attacks_too() {
        let (mut app, [a, _]) = round_app(true);
        let handle = app.world().resource::<GameData>().items.clone();
        let catalog = ItemCatalog(
            app.world().resource::<Assets<ItemCatalog>>().get(&handle).unwrap().0.clone(),
        );
        let mut ids = ItemIds::default();
        let off = make_item(&catalog.0[1], &mut ids, &mut GameRng::seeded(1), false, true);
        app.world_mut().get_mut::<Carried>(a).unwrap().equipment[EquipSlot::Secondary] =
            Some(crate::game::inventory::Equipped::new(off));
        let (character, carried) = (
            app.world().get::<Character>(a).unwrap().clone(),
            app.world().get::<Carried>(a).unwrap().clone(),
        );
        let hands = attack::hands(&character.base_stats(), &carried, &catalog, Default::default());
        assert!(hands.dual_wielding);
        let expected: u32 = hands.hands.iter().map(|h| h.attacks).sum();
        let news = finish_round(&mut app);
        let attacks = news
            .iter()
            .filter(|n| matches!(n, CombatNews::Attack { attacker, .. } if *attacker == a))
            .count() as u32;
        assert_eq!(attacks, expected, "both hands' attacks are made");
        assert_eq!(fight(app.world()).unwrap().this_round.party[0].hands.left, [0, 0]);
    }

    /// A Fighter's Berserk order strikes in fury at twice the stamina; nobody
    /// else may give it.
    #[test]
    fn a_berserk_order_is_a_fighters_furious_attack() {
        use crate::game::characters::Profession;
        let (mut app, [a, b]) = round_app(true);
        app.world_mut().entity_mut(a).insert(Order::Berserk);
        app.world_mut().get_mut::<Vitals>(a).unwrap().stamina = 100;
        app.update();
        let news: Vec<_> = app.world_mut().resource_mut::<Messages<CombatNews>>().drain().collect();
        let swings = news
            .iter()
            .find_map(|n| match n {
                CombatNews::Attack { attacker, how, swings, .. } if *attacker == a => {
                    assert_eq!(*how, Announcement::Mode(AttackMode::Berserk));
                    Some(*swings)
                }
                _ => None,
            })
            .unwrap();
        // The fixture's weight-zero sword costs two a swing, doubled.
        assert_eq!(app.world().get::<Vitals>(a).unwrap().stamina, 100 - 4 * swings);
        app.world_mut().get_mut::<Character>(b).unwrap().profession = Profession::Mage;
        app.world_mut().write_message(GiveOrder { member: b, order: Order::Berserk });
        app.update();
        assert_eq!(app.world().get::<Order>(b), Some(&Order::Defend));
    }

    /// A weapon's effects roll on the blows that draw blood, through the
    /// same saves and resistance as spells (retail `ApplyEffectConditions`).
    #[test]
    fn a_weapons_effects_land_on_blows_that_draw_blood() {
        use crate::content::items::AttackEffect;
        let (mut app, [a, _]) = round_app(true);
        let handle = app.world().resource::<GameData>().items.clone();
        app.world_mut().resource_mut::<Assets<ItemCatalog>>().get_mut(&handle).unwrap().0[0]
            .effects
            .chances[AttackEffect::KnockOut] = 100;
        // An easy target: no evasion, no armour where blows land.
        let species = app.world().resource::<GameData>().species.clone();
        {
            let mut catalogs = app.world_mut().resource_mut::<Assets<MonsterCatalog>>();
            let target = &mut catalogs.get_mut(&species).unwrap().0[0];
            target.evasion = 10;
            target.armour_by_location = [10; 7];
        }
        let news = finish_round(&mut app);
        let monster =
            app.world_mut().query_filtered::<Entity, With<Monster>>().single(app.world()).unwrap();
        let rolled: Vec<_> = news
            .iter()
            .filter_map(|n| match n {
                CombatNews::Effects(effects) => Some(effects.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        assert!(!rolled.is_empty(), "{a:?}'s blows rolled their effect");
        assert!(rolled.iter().all(|e| matches!(
            e,
            Effect::Inflicted { victim: Victim::Monster(m), condition: Condition::Unconscious }
                | Effect::Resisted { victim: Victim::Monster(m) } if *m == monster
        )));
        let knocked_out = rolled.iter().any(|e| matches!(e, Effect::Inflicted { .. }));
        assert_eq!(
            knocked_out,
            app.world()
                .get::<Monster>(monster)
                .unwrap()
                .conditions
                .iter()
                .any(|&(c, _)| c == Condition::Unconscious)
        );
    }

    /// A hand that runs out of shots gives up only its own attacks: the
    /// other, still loaded, makes the ones it was accepted for.
    #[test]
    fn a_hand_out_of_shots_leaves_the_other_hands_attacks() {
        use crate::content::characters::{Attribute, Skill};
        use crate::content::items::{EquipClass, ItemDefinition, QuantityKind};
        let (mut app, [a, _]) = round_app(true);
        let gun = |id: u16, initiative: i8| ItemDefinition {
            id: ItemId(id),
            name: format!("gun {id}"),
            class: EquipClass::RangedWeapon,
            weapon_skill: Some(Skill::ModernWeapon),
            reach: Reach::Touch,
            damage: Dice { count: 0, sides: 0, base: 1 },
            damage_bonus: initiative,
            attack_modes: EnumSet::only(AttackMode::Shoot),
            quantity: QuantityKind::Shots,
            ..default()
        };
        // The main gun is quicker and has more attacks, so it is due again
        // after its first; it has a single shot.
        let catalog = ItemCatalog(vec![gun(0, 10), gun(1, 0)]);
        let handle = app.world().resource::<GameData>().items.clone();
        let mut ids = ItemIds::default();
        let (main, off) = (
            ItemStack::new(&catalog.0[0], &mut ids, 1, true),
            ItemStack::new(&catalog.0[1], &mut ids, 20, true),
        );
        {
            let mut character = app.world_mut().get_mut::<Character>(a).unwrap();
            for attribute in [Attribute::Speed, Attribute::Dexterity] {
                character.attributes[attribute] = 125;
            }
            for skill in [Skill::ModernWeapon, Skill::RangedCombat, Skill::DualWeapons] {
                character.skill_points[skill] = 100;
            }
        }
        let mut carried = app.world_mut().get_mut::<Carried>(a).unwrap();
        carried.equipment[EquipSlot::Primary] = Some(crate::game::inventory::Equipped::new(main));
        carried.equipment[EquipSlot::Secondary] =
            Some(crate::game::inventory::Equipped::new(off.clone()));
        let (character, carried) = (
            app.world().get::<Character>(a).unwrap().clone(),
            app.world().get::<Carried>(a).unwrap().clone(),
        );
        let hands = attack::hands(&character.base_stats(), &carried, &catalog, Default::default());
        assert!(hands.dual_wielding);
        assert!(hands.hands[0].attacks > hands.hands[1].attacks, "{hands:?}");
        app.world_mut().resource_mut::<Assets<ItemCatalog>>().insert(&handle, catalog).unwrap();
        let news = finish_round(&mut app);
        let attacks = news
            .iter()
            .filter(|n| matches!(n, CombatNews::Attack { attacker, .. } if *attacker == a))
            .count();
        // One from the main gun before it ran dry, then the off hand's.
        assert_eq!(attacks, 1 + hands.hands[1].attacks as usize);
        let fired = app.world().get::<Carried>(a).unwrap().worn(EquipSlot::Secondary).unwrap();
        assert!(fired.remaining_uses() < off.remaining_uses(), "the loaded gun fired");
    }
    /// A monster that knocks itself out with a fumbled blow swings no more
    /// in that attack.
    #[test]
    fn a_monster_its_own_fumble_knocks_out_stops_swinging() {
        use crate::content::items::{AttackEffect, AttackEffects};
        use crate::content::species::{AttackWord, MonsterAttack, Species};
        let mut found = false;
        for seed in 0..40 {
            let (mut app, [a, _]) = round_app(true);
            app.world_mut().entity_mut(a).insert(Order::Defend);
            app.insert_resource(GameRng::seeded(seed));
            let species = app.world().resource::<GameData>().species.clone();
            {
                let mut catalogs = app.world_mut().resource_mut::<Assets<MonsterCatalog>>();
                let mut catalog = catalogs.get_mut(&species).unwrap();
                // Aim at 0 less fatigue: most swings stray onto a bystander.
                // Its own armour gives way, and every blow knocks out.
                let mut effects = AttackEffects::default();
                effects.chances[AttackEffect::KnockOut] = 100;
                let fighter = &mut catalog.0[0];
                fighter.attacks = vec![MonsterAttack {
                    word: AttackWord::Verb,
                    weapon: crate::content::species::MonsterWeapon::Unarmed,
                    reach: Reach::Short,
                    score: 0,
                    modes: EnumSet::only(AttackMode::Swing),
                    damage: Dice { count: 0, sides: 0, base: 1 },
                    effects,
                }];
                fighter.attacks_per_round = 1;
                fighter.swings_per_round = 50;
                fighter.level = 50;
                fighter.evasion = 10;
                fighter.armour_by_location = [100; 7];
                catalog.0.push(Species { name: "Bystander".into(), ..default() });
            }
            let fighter = app
                .world_mut()
                .query_filtered::<Entity, With<Monster>>()
                .single(app.world())
                .unwrap();
            let (battle, root, body) = {
                let world = app.world();
                let monster = world.get::<Monster>(fighter).unwrap();
                (
                    world.get::<Combatant>(fighter).unwrap().0,
                    world.get::<LevelOwned>(fighter).unwrap().0,
                    monster.clone(),
                )
            };
            for _ in 0..20 {
                app.world_mut().spawn((
                    Monster { species: 1, ..body.clone() },
                    Transform::from_translation(Vec3::NEG_Z),
                    LevelOwned(root),
                    Combatant(battle),
                ));
            }
            let news = finish_round(&mut app);
            let fumbles: Vec<_> = news
                .iter()
                .filter_map(|n| match n {
                    CombatNews::Fumble { attacker, victim } if *attacker == fighter => {
                        Some(*victim)
                    }
                    _ => None,
                })
                .collect();
            let knocked_out = app
                .world()
                .get::<Monster>(fighter)
                .unwrap()
                .conditions
                .iter()
                .any(|&(c, _)| c == Condition::Unconscious);
            if knocked_out && fumbles.contains(&Victim::Monster(fighter)) {
                found = true;
                // Its last fumble was the blow that knocked it out.
                assert_eq!(fumbles.last(), Some(&Victim::Monster(fighter)), "seed {seed}");
            }
        }
        assert!(found, "some seed has the monster knock itself out");
    }
}
