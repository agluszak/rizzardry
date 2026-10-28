//! The party's fights, first slice: the party
//! against the hostile monsters that see it, weapon attacks only, everyone
//! fighting on their own (retail's continuous mode with every member set to
//! attack). Who reaches whom in melee follows the party's formation.

pub mod approach;
pub mod attack;

use crate::game::world::ActorPose;
use crate::game::world::level::LevelOwned;
use crate::game::world::level::PlayedLevel;
use crate::game::world::navigation::{Walker, feet};
use crate::game::world::obstacles::WorldObstacles;
use avian3d::prelude::Position;
use bevy::prelude::*;
use enumset::EnumSet;

use crate::content::items::AttackMode;
use crate::content::species::AttackWord;

use crate::content::items::{Dice, ItemCatalog};
use crate::content::species::{MonsterCatalog, Reach, Species};
use crate::game::characters::{Character, Condition, Vitals};
use crate::game::inventory::PartyInventory;
use crate::game::inventory::{EquipSlot, StackId};
use crate::game::magic::casting::{self, MagicChoice, MagicNews};
use crate::game::magic::{Effect, LearnedSpells, MagicContext, SpellPoints, Victim};
use crate::game::party::Members;
use crate::game::party::body::PartyBody;
use crate::game::party::formation::{self, Formation, Row};
use crate::game::party::recovery;
use crate::game::party::roster::PartyRoster;
use crate::game::party::{DamageApplied, Order};
use crate::game::rng::GameRng;
use crate::game::time::GameClock;
use crate::game::world::damage::{MonsterDamage, MonsterQuery};
use crate::game::world::identify::{GroupMember, GroupMembers, GroupRecognition};
use crate::game::world::population::Monster;
use attack::{Hand, Swing, character_initiative, character_swing, monster_swing, phase};
use bevy::ecs::relationship::RelationshipTarget;

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<CombatNews>()
            .add_message::<CyclePartyMove>()
            .add_message::<StartRound>()
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
                    start_battle,
                    begin_round,
                    run_battle,
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
    mut orders: Query<&mut crate::game::party::Order>,
) {
    for &GiveOrder { member, order } in requests.read() {
        if battle.is_some()
            && roster.contains(&member)
            && let Ok(mut current) = orders.get_mut(member)
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
    Attack {
        weapon: Option<StackId>,
    },
    Defend,
    /// A spell or item use, revalidated and paid when its initiative comes.
    Magic(MagicChoice),
}

#[derive(Clone, Copy, Debug)]
struct PartyTurn {
    actor: Entity,
    action: RoundAction,
    turn: Turn,
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

/// Where the party stands and faces, and its formation, as reach needs them.
struct PartyView {
    eye: Vec3,
    yaw: f32,
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

/// Start a fight when hostile monsters that see the party can act on it
/// (after retail `UpdateMonsterSight` and `MonsterGroupCanEngage`): the
/// monster is within the far clip, has a line of sight from its eyes to the
/// party's, and its sight threshold covers the distance; and one of its
/// attacks reaches, or its best attack is close (touch or short) and it can
/// walk up. Retail's combat move range and path checks, groups, factions and
/// the party's own surprise rolls are not modelled yet.
pub fn start_battle(
    mut commands: Commands,
    battles: Query<(), With<Battle>>,
    camp: Option<Res<crate::game::camp::Camp>>,
    mut party: Query<(&Position, &mut PartyBody)>,
    level: PlayedLevel,
    members: Query<(&Character, &Vitals)>,
    monsters: Query<(
        Entity,
        &Monster,
        ActorPose,
        Option<&Walker>,
        Has<crate::game::world::population::Provoked>,
    )>,
    obstacles: WorldObstacles,
    content: crate::game::data::Content,
    roster: Single<&PartyRoster>,
    mut news: MessageWriter<CombatNews>,
    mut recognition: GroupRecognition,
    mut rng: ResMut<GameRng>,
    factions: Res<crate::game::factions::Factions>,
) {
    if !battles.is_empty()
        || camp.is_some()
        || !members.iter_many(roster.iter()).matched().any(|(_, v)| !v.is_dead())
    {
        return;
    }
    let (Ok((party_at, mut body)), Some(catalog)) = (party.single_mut(), content.species()) else {
        return;
    };
    let (Some(far), Some(root)) = (level.view_distance(), level.entity()) else { return };
    let party_eye = PartyBody::eye(party_at);
    // The party's lowest level among those who can act weakens sight
    // from behind.
    let lowest_level = members
        .iter_many(roster.iter())
        .matched()
        .filter(|(_, v)| v.can_fight())
        .map(|(c, _)| i32::from(c.level))
        .min()
        .unwrap_or(0);
    // Only the level's static geometry blocks sight.
    let clear = |from: Vec3, to: Vec3| {
        let Ok(direction) = Dir3::new(to - from) else { return true };
        obstacles.first_hit(from, direction, from.distance(to)).is_none()
    };
    let mut joined: Vec<(i32, Entity)> = monsters
        .iter()
        .filter(|(_, monster, at, walker, provoked)| {
            let Some(kind) = catalog.kind(monster.species) else { return false };
            if kind.npc.is_some() || !monster.can_fight() {
                return false;
            }
            let feet = feet(at.translation(), *walker);
            let distance = distance_to_party(party_eye, monster, feet);
            // Authored disposition: some keep to themselves until the party
            // comes close or hurts them.
            if !provoked && !factions.hostile_on_sight(kind, distance) {
                return false;
            }
            let eye = feet + Vec3::Y * monster.body.eye_height;
            if distance > far || !clear(eye, party_eye) {
                return false;
            }
            let forward = at.forward().with_y(0.0);
            let to_party = (party_eye - eye).with_y(0.0);
            let angle = forward.angle_between(to_party);
            let senses = i32::from(kind.attributes.senses);
            let bonus = if kind.level_sight { (u32::from(kind.level) * 5).min(125) } else { 0 };
            if sight_threshold(angle, senses, bonus, lowest_level, far, distance) < distance {
                return false;
            }
            let reaches = kind.attacks.iter().any(|a| distance <= a.reach.metres());
            let close =
                kind.attacks.iter().map(|a| a.reach).max().is_some_and(|r| r <= Reach::Short);
            let walks = monster.body.speed > 0.0;
            reaches || (close && walks)
        })
        .map(|(entity, m, ..)| (m.location, entity))
        .collect();
    if joined.is_empty() {
        return;
    }
    joined.sort();
    info!("combat: {} monsters", joined.len());
    // The view turns to the nearest of them (retail `beginCombat`).
    let nearest = monsters
        .iter_many(joined.iter().map(|&(_, e)| e))
        .matched()
        .map(|(_, _, at, ..)| at.translation())
        .min_by(|a, b| a.distance(party_eye).total_cmp(&b.distance(party_eye)));
    if let Some(nearest) = nearest {
        body.face(nearest - party_eye);
    }
    let joined: Vec<Entity> = joined.into_iter().map(|(_, e)| e).collect();
    let battle = commands.spawn((Name::new("Battle"), Battle::default(), LevelOwned(root))).id();
    for &monster in &joined {
        commands.entity(monster).insert(Combatant(battle));
    }
    recognition.detect(party_eye, body.forward(), Some(&joined), &mut rng);
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
) {
    let Some(battle) = battle.map(Single::into_inner) else { return };
    if !members.iter_many(roster.iter()).matched().any(|v| !v.is_dead()) {
        news.write(CombatNews::Lost);
        // Its combatants leave the fight and stop closing in; they remain.
        commands.entity(battle).despawn();
    }
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
    (hands, species): (&Query<&Hand>, &MonsterCatalog),
    rng: &mut GameRng,
) {
    battle.round += 1;
    let mut round = Round::default();
    for member in roster.iter() {
        let (Ok((character, vitals, carried, burden)), Ok(hand)) =
            (members.get(member), hands.get(member))
        else {
            continue;
        };
        let action = match orders.get(member).copied().unwrap_or_default() {
            Order::Attack => {
                RoundAction::Attack { weapon: carried.worn(EquipSlot::Primary).map(|i| i.id) }
            }
            Order::Defend => RoundAction::Defend,
            Order::Magic(choice) => RoundAction::Magic(choice),
        };
        let turn = match action {
            RoundAction::Attack { .. } if vitals.can_fight() => Turn {
                phase: phase(rng, character_initiative(character, hand, burden.load)),
                attacks_left: hand.attacks,
            },
            RoundAction::Magic(_) if vitals.can_fight() => {
                Turn { phase: phase(rng, character.initiative(burden.load)), attacks_left: 1 }
            }
            _ => Turn::default(),
        };
        round.party.push(PartyTurn { actor: member, action, turn });
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
    (mut members, damage, hands): (Members, MonsterDamage, Query<&Hand>),
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
    let Some(species) = content.species() else { return };
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
            (&hands, species),
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
    mut actors: ParamSet<((Members, MonsterDamage), GroupRecognition)>,
    party_body: Query<(&Position, &PartyBody)>,
    (roster, content): (Single<&PartyRoster>, crate::game::data::Content),
    hands: Query<&Hand>,
    mut magic: BattleMagic,
    mut news: MessageWriter<CombatNews>,
) {
    let Some((entity, mut battle, combatants)) = battle.map(Single::into_inner) else { return };
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    let (mut members, mut damage) = actors.p0();
    // Content and the party's body exist whenever a level is played.
    let (Some(items), Some(species)) = (content.items(), content.species()) else { return };
    let Ok((position, body)) = party_body.single() else { return };
    if !damage.ready() || !battle.executing() || !battle.wait.is_finished() {
        return;
    }
    if !magic.in_flight.is_empty() || !magic.moving.is_empty() {
        return;
    }
    let catalogs = (items, species);
    let roster = &**roster;
    let party = PartyView { eye: PartyBody::eye(position), yaw: body.yaw };
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
            // Counted from where the party stands as its move begins.
            battle.party_move = PartyMove::Moving {
                pace,
                budget: pace.combat_budget(),
                moved: 0.0,
                last: Some(position.0),
            };
            news.write(CombatNews::PartyMoves);
            return;
        }
        if step > 100 {
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
            );
            // Recognition is a required part of this round, before the next
            // accepted action or the battle's end can change its meaning.
            actors.p1().detect(party.eye, body.forward(), Some(combatants), &mut rng);
            if over {
                commands.entity(entity).despawn();
            }
            return;
        }
        // Nobody in the party acts while its move is still to come.
        let party_waits = matches!(battle.party_move, PartyMove::Pending { .. });
        if let Some(index) = battle
            .this_round
            .party
            .iter()
            .position(|p| p.turn.phase == step)
            .filter(|_| !party_waits)
        {
            let acted = party_turn(
                &mut commands,
                &mut battle,
                index,
                combatants,
                &mut rng,
                (&mut members, &hands),
                &mut damage,
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
                index,
                step,
                combatants,
                &mut rng,
                roster,
                &mut members,
                &damage,
                &party,
                &mut magic.context,
                catalogs,
                &mut news,
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

/// A member's accepted turn comes: revalidated, then their spell or attack.
/// Returns whether they acted; an actor who can no longer act loses the turn.
fn party_turn(
    commands: &mut Commands,
    battle: &mut Battle,
    index: usize,
    combatants: &[Entity],
    rng: &mut GameRng,
    (members, hands): (&mut Members, &Query<&Hand>),
    damage: &mut MonsterDamage,
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
                    RoundAction::Attack { weapon } => {
                        weapon == carried.worn(EquipSlot::Primary).map(|i| i.id)
                    }
                    RoundAction::Magic(_) => true,
                    RoundAction::Defend => false,
                }
        });
    if !able_now {
        battle.this_round.party[index].turn = Turn::default();
        return false;
    }
    if let RoundAction::Magic(choice) = work.action {
        battle.this_round.party[index].turn = Turn::default();
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
    let (Some(items), Some(species), Ok(hand)) =
        (magic.context.content.items(), magic.context.content.species(), hands.get(member))
    else {
        return false;
    };
    let party_level = crate::game::party::average_level(
        members.iter_many(magic.context.roster.iter()).matched().map(|(c, ..)| c.level),
    );
    let purse = &mut *magic.context.inventory;
    member_acts(
        battle,
        index,
        combatants,
        rng,
        members,
        damage,
        purse,
        party,
        &magic.context.formation,
        (hand, items, species),
        party_level,
        news,
    );
    true
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
        if work.turn.phase == 0 {
            continue;
        }
        if rng.below(100) < keep {
            work.turn.phase = work.turn.phase.max(step);
        } else {
            work.turn = Turn::default();
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
        .resolve(members, damage, |field, spells| {
            casting::perform(member, choice, &situation, spells, points, rng, field)
        })
        .and_then(|performed| performed)
}

/// Retail `ChooseMonsterTarget`: among the monsters a hand reaches, the
/// least afflicted, then those "in reach" of the party, then the nearest;
/// ties keep the fight's order.
fn melee_target<'a>(
    member: Entity,
    hand: &Hand,
    combatants: &[Entity],
    damage: &MonsterDamage,
    party: &PartyView,
    formation: &Formation,
    species: &'a MonsterCatalog,
) -> Option<(Entity, &'a Species)> {
    combatants
        .iter()
        .filter_map(|&entity| {
            let (monster, at, walker, _) = damage.monsters().get(entity).ok()?;
            let kind = species.kind(monster.species).filter(|kind| kind.npc.is_none())?;
            let distance = distance_to_party(party.eye, monster, feet(at.translation(), walker));
            if !damage.active(entity)
                || monster.is_dead()
                || !party.reaches(formation, member, at.translation(), hand.reach, distance)
            {
                return None;
            }
            let worst = monster.conditions.iter().map(|&(c, _)| c).max();
            let side = formation::quadrant(party.eye, party.yaw, at.translation());
            let in_reach = !kind.attacks.is_empty()
                && (side == Row::Rear
                    || (!formation.occupied(side) && formation.occupied(Row::Center)));
            Some(((worst, !in_reach, distance), (entity, kind)))
        })
        .min_by(|a, b| a.0.0.cmp(&b.0.0).then(a.0.1.cmp(&b.0.1)).then(a.0.2.total_cmp(&b.0.2)))
        .map(|(_, target)| target)
}

/// A member's attack: one of their turn's attacks, its swings at the best
/// target their hand reaches.
fn member_acts(
    battle: &mut Battle,
    index: usize,
    combatants: &[Entity],
    rng: &mut GameRng,
    members: &mut Members,
    damage: &mut MonsterDamage,
    purse: &mut PartyInventory,
    party: &PartyView,
    formation: &Formation,
    (hand, items, species): (&Hand, &ItemCatalog, &MonsterCatalog),
    party_level: u32,
    news: &mut MessageWriter<CombatNews>,
) {
    let member = battle.this_round.party[index].actor;
    let Ok((character, mut vitals, carried, burden)) = members.get_mut(member) else { return };
    let target = melee_target(member, hand, combatants, damage, party, formation, species);
    let turn = &mut battle.this_round.party[index].turn;
    let Some((target, kind)) = target else {
        // No target in reach: discard the remaining attacks this round.
        *turn = Turn::default();
        return;
    };
    turn.attacks_left -= 1;
    turn.phase = next_phase(turn.phase, turn.attacks_left);
    battle.this_round.attacked = true;
    // Retail draws the attack's mode, then its swings.
    let mode = attack::choose_mode(rng, hand.modes);
    let swings = rng.below(hand.swings) + 1;
    news.write(CombatNews::Attack {
        attacker: member,
        target,
        how: Announcement::Mode(mode),
        swings,
    });
    let (mut hits, mut dealt, mut casualty) = (0, 0, None);
    let weight = carried.worn(EquipSlot::Primary).and_then(|i| items.get(i.item)).map(|d| d.weight);
    for _ in 0..swings {
        if !vitals.can_fight() {
            break;
        }
        match character_swing(rng, &character, &vitals, hand, mode, kind) {
            Swing::Hit(damage_amount) => {
                hits += 1;
                let Ok(outcome) = damage.apply(target, damage_amount, party_level, purse, rng)
                else {
                    break;
                };
                dealt += outcome.damage;
                if outcome.died {
                    casualty = Some(outcome);
                }
            }
            Swing::NoPenetration => hits += 1,
            Swing::Missed => {}
        };
        let cost = attack::swing_stamina(rng, mode, weight, vitals.has(Condition::Diseased));
        let slowed = vitals.has(Condition::Slowed);
        vitals.spend_stamina(burden.load.scale_fatigue(cost, slowed));
        if casualty.is_some() {
            break;
        }
    }
    // The summary, then the casualty (retail `ReportCharacterAttackResult`).
    news.write(CombatNews::Result { attacker: member, target, hits, damage: dealt });
    if let Some(outcome) = casualty {
        battle.credit_kill(target, outcome.experience, news);
    }
}

/// A monster's accepted turn comes: it attacks someone it reaches, or with
/// nobody in reach advances (or charges when hurt) and acts again later in
/// the round (retail `UpdateMonsterAI`, `StepMonsterCombatAction`). Returns
/// whether it acted.
fn monster_turn(
    commands: &mut Commands,
    battle: &mut Battle,
    index: usize,
    step: u8,
    combatants: &[Entity],
    rng: &mut GameRng,
    roster: &PartyRoster,
    members: &mut Members,
    damage: &MonsterDamage,
    party: &PartyView,
    context: &mut MagicContext,
    catalogs: (&ItemCatalog, &MonsterCatalog),
    news: &mut MessageWriter<CombatNews>,
) -> bool {
    let monster = battle.this_round.monsters[index].0;
    if !combatants.contains(&monster)
        || !damage.monsters().get(monster).is_ok_and(|(m, ..)| m.can_fight())
    {
        battle.this_round.monsters[index].1 = Turn::default();
        return false;
    }
    let turn = battle.this_round.monsters[index].1;
    let MagicContext { formation, party_damage, .. } = context;
    let idle = monster_acts(
        battle,
        index,
        rng,
        roster,
        members,
        damage.monsters(),
        party,
        formation,
        catalogs,
        news,
        party_damage,
    );
    if idle
        && !battle.this_round.moved.contains(&monster)
        && let Ok((walker_monster, at, walker, _)) = damage.monsters().get(monster)
        && let Some(budget) =
            approach::move_budget(walker_monster, at.translation(), walker, party.eye, catalogs.1)
    {
        battle.this_round.moved.push(monster);
        battle.this_round.monsters[index].1 =
            Turn { phase: (u32::from(step) * 2).min(100) as u8, attacks_left: turn.attacks_left };
        let charge = budget.charge;
        commands.entity(monster).insert(approach::CombatMove { budget: budget.metres, moved: 0.0 });
        news.write(CombatNews::Moves { monster, charge });
    }
    true
}

fn monster_acts(
    battle: &mut Battle,
    index: usize,
    rng: &mut GameRng,
    roster: &PartyRoster,
    members: &mut Members,
    monsters: &MonsterQuery,
    party: &PartyView,
    formation: &mut Mut<Formation>,
    (items, species): (&ItemCatalog, &MonsterCatalog),
    news: &mut MessageWriter<CombatNews>,
    damage: &mut MessageWriter<DamageApplied>,
) -> bool {
    let (entity, turn) = battle.this_round.monsters[index];
    let Ok((monster, at, walker, _)) = monsters.get(entity) else { return false };
    let Some(kind) = species.kind(monster.species) else { return false };
    let distance = distance_to_party(party.eye, monster, feet(at.translation(), walker));
    // Every (attack, mode, member) that reaches is equally likely.
    let mut candidates = vec![];
    for (a, attack) in kind.attacks.iter().enumerate() {
        for member in roster.iter() {
            let Ok((_, vitals, ..)) = members.get(member) else { continue };
            if vitals.can_fight()
                && party.reaches(formation, member, at.translation(), attack.reach, distance)
            {
                for mode in attack.modes {
                    candidates.push((a, mode, member));
                }
            }
        }
    }
    let turn_left = turn.attacks_left.saturating_sub(1);
    battle.this_round.monsters[index].1 = Turn { phase: 0, attacks_left: 0 };
    if candidates.is_empty() {
        return true;
    }
    battle.this_round.attacked = true;
    let (a, mode, member) = candidates[rng.below(candidates.len() as u32) as usize];
    let attack = &kind.attacks[a];
    // Retail draws for the mode again, from just the one chosen.
    let mode = attack::choose_mode(rng, EnumSet::only(mode));
    let Ok((character, mut vitals, carried, burden)) = members.get_mut(member) else {
        return false;
    };
    let armour = attack::armour(&character, &carried, items, burden.load, battle.defends(member));
    let swings = rng.below(u32::from(kind.swings_per_round.max(1))) + 1;
    news.write(CombatNews::Attack {
        attacker: entity,
        target: member,
        how: Announcement::Monster { word: attack.word, mode, named: attack.named },
        swings,
    });
    let (mut hits, mut dealt) = (0, 0);
    for _ in 0..swings {
        match monster_swing(rng, kind, attack, mode, &character, &armour) {
            Swing::Hit(amount) => {
                hits += 1;
                dealt += crate::game::party::damage_member(
                    member,
                    &mut vitals,
                    formation,
                    amount,
                    damage,
                );
                if vitals.is_dead() {
                    break;
                }
                let senses = character.attribute(crate::content::characters::Attribute::Senses);
                crate::game::characters::wake_on_hit(&mut vitals.conditions, senses, rng);
            }
            Swing::NoPenetration => hits += 1,
            Swing::Missed => {}
        };
    }
    news.write(CombatNews::Result { attacker: entity, target: member, hits, damage: dealt });
    if vitals.is_dead() {
        news.write(CombatNews::Killed { actor: member });
    }
    battle.this_round.monsters[index].1 =
        Turn { phase: next_phase(turn.phase, turn_left), attacks_left: turn_left };
    false
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
) -> bool {
    // The round's time passes now, with its meaning: conditions age, spell
    // points come back at half rate and nothing else recovers (retail
    // `AdvanceCombatRound`, `AdvanceTimedEffects`).
    let ticks = clock.advance(ROUND_MS);
    for &entity in combatants {
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
        let sides = (character.max_stamina() / 20).saturating_sub(1).max(1);
        let rolled = rng.roll(Dice { count: 2, sides: sides.min(255) as u8, base: 0 });
        let gained = recovery::combat_rest_amount(&character, rolled);
        if vitals.stamina < character.max_stamina()
            || vitals
                .conditions
                .contains(&(Condition::Unconscious, crate::game::characters::Lasting::Indefinite))
        {
            vitals.restore_stamina(gained, character.max_stamina());
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
    let hostiles_left =
        combatants.iter().any(|&e| monsters.get(e).is_ok_and(|(m, ..)| !m.is_dead()));
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
    use crate::game::inventory::{Carried, ItemIds, make_item};

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
            .init_resource::<avian3d::collider_tree::ColliderTrees>()
            .init_resource::<crate::game::factions::Factions>()
            .init_resource::<crate::game::difficulty::Difficulty>()
            .add_message::<MagicNews>()
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
                    location: 0,
                    species: 0,
                    hp: 10000,
                    max_hp: 10000,
                    stamina: 0,
                    max_stamina: 0,
                    conditions: vec![],
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
            [Effect::Damaged { victim: Victim::Monster(m), amount, .. }] if *m == monster && *amount > 0
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
        assert_eq!(battle.this_round.party[0].turn, Turn::default());
        let world = app.world();
        let character = world.get::<Character>(caster).unwrap();
        let carried = world.get::<Carried>(caster).unwrap();
        let load = world.get::<Burden>(caster).unwrap().load;
        let items = GameData::items(world).unwrap();
        let baseline = attack::armour(character, carried, items, load, false);
        let defended = attack::armour(character, carried, items, load, battle.defends(caster));
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
                    id: crate::content::spells::SpellId(12),
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
                vec![missile(caster, crate::content::spells::SpellId(12), Victim::Member(member))],
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
                named: false,
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
        let pending = fight(app.world())
            .unwrap()
            .this_round
            .party
            .iter()
            .filter(|p| p.turn.phase > 0)
            .count();
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
        app.world_mut().spawn((
            outsider,
            Transform::from_translation(Vec3::NEG_Z),
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
                    named: false,
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
}
