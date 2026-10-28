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
use crate::content::species::{MonsterCatalog, Reach};
use crate::content::spells::SpellCatalog;
use crate::game::characters::{Character, Condition, Vitals};
use crate::game::data::GameData;
use crate::game::inventory::{EquipSlot, StackId};
use crate::game::magic::casting::{self, MagicChoice, MagicNews};
use crate::game::magic::{Effect, LearnedSpells, SpellPoints, Victim};
use crate::game::party::body::PartyBody;
use crate::game::party::formation::{self, Formation, Row};
use crate::game::party::recovery;
use crate::game::party::roster::PartyRoster;
use crate::game::party::{DamageApplied, Order};
use crate::game::rng::GameRng;
use crate::game::time::GameClock;
use crate::game::world::damage::{DamageOutcome, MonsterDamage};
use crate::game::world::identify::{GroupMember, GroupMembers, GroupRecognition};
use crate::game::world::population::Monster;
use attack::{
    Swing, armour, character_initiative, character_swing, monster_swing, phase, primary_hand,
};
use bevy::ecs::relationship::RelationshipTarget;

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<CombatNews>()
            .add_message::<MagicNews>()
            .add_message::<ChoosePartyMove>()
            .add_message::<StartRound>()
            .init_resource::<CombatSettings>()
            .init_resource::<casting::FlightOrder>()
            .add_systems(
                Update,
                (
                    settle_defeat,
                    start_battle,
                    track_party_move,
                    run_battle,
                    settle_defeat,
                    approach::close_in,
                )
                    .chain()
                    .in_set(crate::game::GameplaySet::Combat)
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

/// Choose, between rounds, whether the party moves in the next ones.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChoosePartyMove(pub Option<PartyPace>);

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

/// Accepted round work identifies the actor and weapon, never a roster slot.
/// Order, weapon identity, initiative and attack count are locked at acceptance.
/// Execution revalidates membership, ability, weapon presence and current reach;
/// damage profiles and fatigue use the actor's live stats and accepted weapon.
#[derive(Clone, Copy, Debug)]
enum AcceptedAction {
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
    action: AcceptedAction,
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

/// A round's committed work, fixed when the round is accepted: who acts,
/// with what, when and how often. Execution revalidates each actor (still a
/// member or combatant, able, the same weapon) but never redirects work to
/// whoever holds its slot now.
#[derive(Debug, Default)]
struct AcceptedRound {
    party: Vec<PartyTurn>,
    monsters: Vec<(Entity, Turn)>,
}

/// The party's fight, on an entity owned by its level. Exists only while
/// there is one.
#[derive(Component, Debug, Default)]
pub struct Battle {
    pub round: u32,
    phase: RoundPhase,
    accepted: AcceptedRound,
    /// Experience from kills, shared at the end.
    experience: u32,
    kills: u32,
    passive_rounds: u32,
    attacked_this_round: bool,
    /// Monsters that have moved this round: one move each.
    moved: Vec<Entity>,
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

    fn defends(&self, actor: Entity) -> bool {
        self.accepted
            .party
            .iter()
            .any(|p| p.actor == actor && matches!(p.action, AcceptedAction::Defend))
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
    pub fn credit_kill(&mut self, monster: Entity, experience: u32) {
        self.experience += experience;
        self.kills += 1;
        if let Some((_, turn)) =
            self.accepted.monsters.iter_mut().find(|(actor, _)| *actor == monster)
        {
            *turn = Turn::default();
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

/// Whether a member can act or be targeted: alive and nothing worse than
/// webbed.
fn able(member: &Vitals) -> bool {
    !member.is_dead() && member.worst_condition().is_none_or(|c| c < Condition::Asleep)
}

fn monster_can_attack(monster: &Monster) -> bool {
    !monster.is_dead() && !monster.conditions.iter().any(|&(c, _)| c > Condition::Webbed)
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
pub fn sight_threshold(
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
    data: Option<Res<GameData>>,
    species: Res<Assets<MonsterCatalog>>,
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
    let (Ok((party_at, mut body)), Some(catalog)) =
        (party.single_mut(), data.and_then(|d| species.get(&d.species)))
    else {
        return;
    };
    let (Some(far), Some(root)) = (level.view_distance(), level.entity()) else { return };
    let party_eye = PartyBody::eye(party_at);
    // The party's lowest level among those who can act weakens sight
    // from behind.
    let lowest_level = members
        .iter_many(roster.iter())
        .matched()
        .filter(|(_, v)| able(v))
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
            let Some(kind) = catalog.0.get(monster.species as usize) else { return false };
            if kind.npc.is_some() || !monster_can_attack(monster) {
                return false;
            }
            let feet = feet(at.translation(), *walker);
            let distance = distance_to_party(party_eye, monster, feet);
            // Authored disposition: some keep to themselves until the party
            // comes close or hurts them.
            if !provoked && !kind.hostile_at(distance, &factions) {
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

/// End the fight: its combatants stop closing in (the movement it gave
/// them) and are no longer in a fight; they themselves remain.
fn end_battle(commands: &mut Commands, battle: Entity, combatants: &[Entity]) {
    for &monster in combatants {
        commands
            .entity(monster)
            .remove::<approach::CombatMove>()
            .entry::<bevy_landmass::prelude::AgentTarget3d>()
            .and_modify(|mut target| *target = bevy_landmass::prelude::AgentTarget3d::None);
    }
    commands.entity(battle).despawn();
}

/// Terminal state is settled independently of whose action dealt the damage.
/// Used before/after combat execution and at the end of environmental damage.
pub fn settle_defeat(
    mut commands: Commands,
    battle: Option<Single<(Entity, Option<&Combatants>), With<Battle>>>,
    roster: Single<&PartyRoster>,
    members: Query<&Vitals>,
    mut news: MessageWriter<CombatNews>,
) {
    let Some((battle, combatants)) = battle.map(Single::into_inner) else { return };
    if !members.iter_many(roster.iter()).matched().any(|v| !v.is_dead()) {
        news.write(CombatNews::Lost);
        end_battle(&mut commands, battle, combatants.map_or(&[][..], Combatants::entities));
    }
}

/// Accept a round (retail `BeginCombatExecution`): lock each member's order
/// and weapon, and everyone's initiative and attacks, the party first, then
/// the combatants in order. Pacing delays only its execution.
fn accept_round(
    battle: &mut Battle,
    combatants: &[Entity],
    roster: &PartyRoster,
    members: &crate::game::magic::Members,
    orders: &Query<&Order>,
    monsters: &Query<(&mut Monster, ActorPose, Option<&Walker>, &LevelOwned)>,
    (items, species): (&ItemCatalog, &MonsterCatalog),
    rng: &mut GameRng,
) {
    battle.round += 1;
    battle.attacked_this_round = false;
    battle.moved.clear();
    let mut accepted = AcceptedRound::default();
    for member in roster.iter() {
        let Ok((character, vitals, carried)) = members.get(member) else { continue };
        let action = match orders.get(member).copied().unwrap_or_default() {
            Order::Attack => {
                AcceptedAction::Attack { weapon: carried.worn(EquipSlot::Primary).map(|i| i.id) }
            }
            Order::Defend => AcceptedAction::Defend,
            Order::Magic(choice) => AcceptedAction::Magic(choice),
        };
        let turn = if matches!(action, AcceptedAction::Attack { .. }) && able(vitals) {
            let hand = primary_hand(character, carried, items);
            Turn {
                phase: phase(rng, character_initiative(character, &hand)),
                attacks_left: hand.attacks,
            }
        } else if matches!(action, AcceptedAction::Magic(_)) && able(vitals) {
            Turn {
                phase: phase(rng, character.initiative(crate::game::characters::Load::None)),
                attacks_left: 1,
            }
        } else {
            Turn::default()
        };
        accepted.party.push(PartyTurn { actor: member, action, turn });
    }
    for &entity in combatants {
        let Ok((monster, ..)) = monsters.get(entity) else { continue };
        let Some(kind) = species.0.get(monster.species as usize) else { continue };
        let turn = if monster.is_dead() {
            Turn::default()
        } else {
            Turn {
                phase: phase(rng, i32::from(kind.initiative)),
                attacks_left: u32::from(kind.attacks_per_round),
            }
        };
        accepted.monsters.push((entity, turn));
    }
    // The party's own move takes the phase its slowest member allows,
    // on a step of ten (retail `InitializePartyMovementPhase`).
    battle.party_move = match battle.party_pace {
        Some(pace) => {
            let slowest = roster
                .iter()
                .filter_map(|m| members.get(m).ok())
                .filter(|(_, vitals, _)| able(vitals))
                .map(|(c, _, _)| {
                    c.initiative(crate::game::characters::Load::None) + rng.below(4) as i32
                })
                .min()
                .unwrap_or(0)
                .clamp(-10, 89);
            let phase = (90 - slowest) as u8;
            let phase = (phase + 5) - (phase + 5) % 10;
            PartyMove::Pending { pace, phase: phase.clamp(10, 100) }
        }
        None => PartyMove::None,
    };
    battle.accepted = accepted;
    battle.phase = RoundPhase::Accepted;
}

/// Advance the fight by one action when its pause is over.
pub fn run_battle(
    mut commands: Commands,
    time: Res<Time>,
    battle: Option<Single<(Entity, &mut Battle, Option<&Combatants>, &LevelOwned)>>,
    mut rng: ResMut<GameRng>,
    mut clock: ResMut<GameClock>,
    roster: Single<&PartyRoster>,
    mut actors: ParamSet<((crate::game::magic::Members, MonsterDamage), GroupRecognition)>,
    party_body: Query<(&Position, &PartyBody)>,
    mut formation: Single<&mut Formation>,
    data: Option<Res<GameData>>,
    (items, species): (Res<Assets<ItemCatalog>>, Res<Assets<MonsterCatalog>>),
    (mut news, mut party_damage): (MessageWriter<CombatNews>, MessageWriter<DamageApplied>),
    (mut starts, settings, orders): (MessageReader<StartRound>, Res<CombatSettings>, Query<&Order>),
    (mut magic, mut magic_news): (BattleMagic, MessageWriter<MagicNews>),
    mut moves: MessageReader<ChoosePartyMove>,
) {
    let Some((entity, mut battle, combatants, level)) = battle.map(Single::into_inner) else {
        starts.clear();
        return;
    };
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    let start = starts.read().count() > 0;
    for &ChoosePartyMove(pace) in moves.read() {
        if !battle.executing() {
            battle.party_pace = pace;
        }
    }
    // Starting the round while the party moves ends its move (retail
    // `MGSKeyboard.cpp:568-573`).
    if start && matches!(battle.party_move, PartyMove::Moving { .. }) {
        let step = match battle.phase {
            RoundPhase::Executing(step) => step,
            _ => 1,
        };
        let (mut members, _) = actors.p0();
        let items = data.as_ref().and_then(|d| items.get(&d.items));
        finish_party_move(&mut battle, step, &mut rng, &mut members, items);
        news.write(CombatNews::PartyStops);
    }
    let (mut members, mut damage) = actors.p0();
    // Content and the party's body exist whenever a level is played.
    let (Some(items), Some(species)) = (
        data.as_ref().and_then(|d| items.get(&d.items)),
        data.as_ref().and_then(|d| species.get(&d.species)),
    ) else {
        return;
    };
    let Ok((position, body)) = party_body.single() else { return };
    if !damage.ready() {
        return;
    }
    let catalogs = (items, species);
    if start && !battle.executing() {
        accept_round(
            &mut battle,
            combatants,
            &roster,
            &members,
            &orders,
            &damage.monsters,
            catalogs,
            &mut rng,
        );
    }
    let party = PartyView { eye: PartyBody::eye(position), yaw: body.yaw };
    // A finished pause is not ticked again: a fight waiting for the player
    // stays unchanged.
    if !battle.wait.is_finished() && !battle.wait.tick(time.delta()).is_finished() {
        return;
    }
    // The next actor waits until the last one's missiles have landed
    // (retail `UpdateCombat`).
    if !magic.in_flight.is_empty() || !magic.moving.is_empty() {
        return;
    }

    // Between rounds a phased fight waits for the player; a continuous one
    // goes on (retail `UpdateCombat`, `BeginCombatExecution`).
    if !battle.executing() {
        if !settings.continuous {
            return;
        }
        accept_round(
            &mut battle,
            combatants,
            &roster,
            &members,
            &orders,
            &damage.monsters,
            catalogs,
            &mut rng,
        );
    }
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
            battle.party_move = PartyMove::Moving {
                pace,
                budget: if pace == PartyPace::Run { 30.0 } else { 20.0 },
                moved: 0.0,
                last: None,
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
                &roster,
                &mut members,
                &mut damage.monsters,
                party.eye,
                species,
                &mut news,
                (&mut magic.points, data.as_ref().and_then(|d| magic.spells.get(&d.spells))),
            );
            // Recognition is a required part of this round, before the next
            // accepted action or the battle's end can change its meaning.
            actors.p1().detect(party.eye, body.forward(), Some(combatants), &mut rng);
            if over {
                end_battle(&mut commands, entity, combatants);
            }
            return;
        }
        // Nobody in the party acts while its move is still to come.
        let party_waits = matches!(battle.party_move, PartyMove::Pending { .. });
        if let Some(index) =
            battle.accepted.party.iter().position(|p| p.turn.phase == step).filter(|_| !party_waits)
        {
            let work = battle.accepted.party[index];
            let member = work.actor;
            let able_now = roster.contains(&member)
                && members.get(member).is_ok_and(|(_, vitals, carried)| {
                    able(vitals)
                        && match work.action {
                            AcceptedAction::Attack { weapon } => {
                                weapon == carried.worn(EquipSlot::Primary).map(|i| i.id)
                            }
                            AcceptedAction::Magic(_) => true,
                            AcceptedAction::Defend => false,
                        }
                });
            if !able_now {
                battle.accepted.party[index].turn = Turn::default();
                continue;
            }
            if let AcceptedAction::Magic(choice) = work.action {
                let levels_sum: u32 = members
                    .iter_many(roster.iter())
                    .matched()
                    .map(|(c, _, _)| u32::from(c.level))
                    .sum();
                let party_level = levels_sum / roster.len().max(1) as u32;
                let Some(data) = data.as_ref() else { return };
                let report = member_casts(
                    combatants,
                    member,
                    choice,
                    &roster,
                    &mut rng,
                    &mut members,
                    &mut damage,
                    &party,
                    &mut formation,
                    &mut party_damage,
                    data,
                    items,
                    species,
                    party_level,
                    &mut magic,
                );
                battle.accepted.party[index].turn = Turn::default();
                match report {
                    Ok((report, flights)) => {
                        battle.attacked_this_round = true;
                        for effect in &report.effects {
                            if let Effect::Damaged {
                                victim: Victim::Monster(target), outcome, ..
                            } = *effect
                                && outcome.died
                            {
                                battle.credit_kill(target, outcome.experience);
                                news.write(CombatNews::Killed { actor: target });
                            }
                        }
                        magic.flight_order.launch(&mut commands, flights, level.0);
                        magic_news.write(MagicNews::Done(report));
                    }
                    // A refused action falls back to defending (retail
                    // `ExecuteCharacterAction`), its turn spent.
                    Err(refusal) => {
                        info!("magic refused: {refusal:?}");
                        battle.accepted.party[index].action = AcceptedAction::Defend;
                        magic_news.write(MagicNews::Refused { caster: member, refusal });
                    }
                }
                battle.wait = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
                return;
            }
            // Retail `GetAveragePartyLevel`: what loot tables scale by.
            let levels_sum: u32 = members
                .iter_many(roster.iter())
                .matched()
                .map(|(c, _, _)| u32::from(c.level))
                .sum();
            let party_level = levels_sum / roster.len().max(1) as u32;
            let casualty = member_acts(
                &mut battle,
                index,
                member,
                combatants,
                &mut rng,
                &mut members,
                &mut damage,
                &party,
                &formation,
                catalogs,
                party_level,
                &mut news,
            );
            if let Some((target, outcome)) = casualty {
                battle.experience += outcome.experience;
                battle.kills += 1;
                if let Some((_, turn)) =
                    battle.accepted.monsters.iter_mut().find(|(actor, _)| *actor == target)
                {
                    *turn = Turn::default();
                }
            }
            battle.wait = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
            return;
        }
        if let Some(index) = battle.accepted.monsters.iter().position(|(_, t)| t.phase == step) {
            let monster = battle.accepted.monsters[index].0;
            if !combatants.contains(&monster)
                || !damage.monsters.get(monster).is_ok_and(|(m, ..)| monster_can_attack(m))
            {
                battle.accepted.monsters[index].1 = Turn::default();
                continue;
            }
            let turn = battle.accepted.monsters[index].1;
            let idle = monster_acts(
                &mut battle,
                index,
                &mut rng,
                &roster,
                &mut members,
                &damage.monsters,
                &party,
                &mut formation,
                catalogs,
                &mut news,
                &mut party_damage,
            );
            // Nothing in reach: advance, or charge when hurt, then act again
            // later in the round (retail `UpdateMonsterAI`, `StepMonsterCombatAction`).
            if idle
                && !battle.moved.contains(&monster)
                && let Ok((moving_monster, at, walker, _)) = damage.monsters.get(monster)
                && approach::needs_approach(
                    moving_monster,
                    at.translation(),
                    walker,
                    party.eye,
                    species,
                )
                && let Some(kind) = species.0.get(moving_monster.species as usize)
            {
                let charge = moving_monster.hp * 100 <= moving_monster.max_hp * 95;
                let mut budget = kind.combat_move_range * 20.0 * if charge { 1.5 } else { 1.0 };
                if moving_monster.conditions.iter().any(|&(c, _)| c == Condition::Slowed) {
                    budget *= 0.5;
                }
                battle.moved.push(monster);
                battle.accepted.monsters[index].1 = Turn {
                    phase: (u32::from(step) * 2).min(100) as u8,
                    attacks_left: turn.attacks_left,
                };
                commands.entity(monster).insert(approach::CombatMove { budget, moved: 0.0 });
                news.write(CombatNews::Moves { monster, charge });
            }
            battle.wait = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
            return;
        }
        battle.phase = RoundPhase::Executing(step + 1);
    }
}

/// Count the ground the party covers while it moves in a fight, and end the
/// move when it is spent.
pub fn track_party_move(
    battle: Option<Single<&mut Battle>>,
    party: Query<&Position, With<PartyBody>>,
    mut rng: ResMut<GameRng>,
    mut members: crate::game::magic::Members,
    data: Option<Res<GameData>>,
    items: Res<Assets<ItemCatalog>>,
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
        let step = match battle.phase {
            RoundPhase::Executing(step) => step,
            _ => 1,
        };
        let items = data.as_ref().and_then(|d| items.get(&d.items));
        finish_party_move(&mut battle, step, &mut rng, &mut members, items);
        news.write(CombatNews::PartyStops);
    }
}

/// The move is over (retail `CompletePartyMovementTurns`, `HandlePartyMovement`):
/// each member keeps their action with the chance the unspent share gives
/// (spent counting half again when running), and pays stamina for the
/// ground covered, by load.
fn finish_party_move(
    battle: &mut Battle,
    step: u8,
    rng: &mut GameRng,
    members: &mut crate::game::magic::Members,
    items: Option<&ItemCatalog>,
) {
    let PartyMove::Moving { pace, budget, moved, .. } = battle.party_move else { return };
    battle.party_move = PartyMove::None;
    let mut spent = (moved / budget.max(0.01) * 100.0).min(100.0);
    if pace == PartyPace::Run {
        spent *= 1.5;
    }
    let keep = 100u32.saturating_sub(spent as u32);
    for work in &mut battle.accepted.party {
        if work.turn.phase == 0 {
            continue;
        }
        if rng.below(100) < keep {
            work.turn.phase = work.turn.phase.max(step);
        } else {
            work.turn = Turn::default();
        }
    }
    // Retail charges `distance × load × 2 (combat) × 1.5 (diseased)` units
    // at one stamina per 2500 (`MGSPartyMovement.cpp:302-358`): 0.2 a metre
    // walking unencumbered, 0.4 running.
    let per_metre = if pace == PartyPace::Run { 0.4 } else { 0.2 };
    for work in &battle.accepted.party {
        let Ok((character, mut vitals, carried)) = members.get_mut(work.actor) else { continue };
        if vitals.is_dead() {
            continue;
        }
        let weight =
            items.map_or(0, |items| crate::game::inventory::carried_weight(&carried, items));
        let load = match character.load(weight) {
            crate::game::characters::Load::None => 1.0,
            crate::game::characters::Load::Light => 1.25,
            crate::game::characters::Load::Medium => 1.5,
            crate::game::characters::Load::Heavy => 2.0,
            crate::game::characters::Load::Extreme => 3.0,
        };
        let disease = if vitals.has(Condition::Diseased) { 1.5 } else { 1.0 };
        vitals.spend_stamina((moved * per_metre * load * disease).round() as u32);
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
    spells: Res<'w, Assets<SpellCatalog>>,
    points: Query<'w, 's, (&'static LearnedSpells, &'static mut SpellPoints)>,
    obstacles: WorldObstacles<'w, 's>,
    groups: Query<'w, 's, &'static GroupMember>,
    group_members: Query<'w, 's, &'static GroupMembers>,
    difficulty: Res<'w, crate::game::difficulty::Difficulty>,
    /// Missiles still on their way hold the fight's next action, as do
    /// monsters still advancing.
    in_flight: Query<'w, 's, (), With<casting::SpellInFlight>>,
    moving: Query<'w, 's, (), With<approach::CombatMove>>,
    flight_order: ResMut<'w, casting::FlightOrder>,
}

/// A member casts or uses an item at their initiative. The monsters it may
/// aim at are decided now: in the fight, alive, within the spell's reach of
/// the party and in its sight (retail `CharacterActionReachesTarget`).
#[allow(clippy::too_many_arguments)]
fn member_casts(
    combatants: &[Entity],
    member: Entity,
    choice: MagicChoice,
    roster: &PartyRoster,
    rng: &mut GameRng,
    members: &mut crate::game::magic::Members,
    damage: &mut MonsterDamage,
    party: &PartyView,
    formation: &mut Mut<Formation>,
    party_damage: &mut MessageWriter<DamageApplied>,
    data: &GameData,
    items: &ItemCatalog,
    species: &MonsterCatalog,
    party_level: u32,
    magic: &mut BattleMagic,
) -> Result<(casting::Report, Vec<casting::SpellInFlight>), casting::Refusal> {
    let BattleMagic { spells, points, obstacles, groups, group_members, difficulty, .. } = magic;
    let spells = spells.get(&data.spells).ok_or(casting::Refusal::MissingContent)?;
    let party_members: Vec<Entity> = roster.iter().collect();
    // Where each fighting monster in the party's sight stands, and how far.
    let in_sight: Vec<(casting::Candidate, Option<Entity>)> = combatants
        .iter()
        .filter_map(|&entity| {
            let (monster, at, walker, _) = damage.monsters.get(entity).ok()?;
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
    let mut world = crate::game::magic::Field {
        members,
        monsters: damage,
        formation,
        party_damage,
        items,
        species,
        party_level,
        difficulty: **difficulty,
        party: &party_members,
        improved: Vec::new(),
    };
    casting::perform(member, choice, &situation, spells, points, rng, &mut world)
}

fn member_acts(
    battle: &mut Battle,
    index: usize,
    member: Entity,
    combatants: &[Entity],
    rng: &mut GameRng,
    members: &mut crate::game::magic::Members,
    damage: &mut MonsterDamage,
    party: &PartyView,
    formation: &Formation,
    (items, species): (&ItemCatalog, &MonsterCatalog),
    party_level: u32,
    news: &mut MessageWriter<CombatNews>,
) -> Option<(Entity, DamageOutcome)> {
    let Ok((character, mut vitals, carried)) = members.get_mut(member) else { return None };
    let hand = primary_hand(&character, &carried, items);
    // Retail `ChooseMonsterTarget`: among the monsters this hand reaches,
    // the least afflicted, then those "in reach" of the party, then the
    // nearest; ties keep the fight's order.
    let target = combatants
        .iter()
        .filter_map(|&entity| {
            let (monster, at, walker, _) = damage.monsters.get(entity).ok()?;
            let distance = distance_to_party(party.eye, monster, feet(at.translation(), walker));
            if !species.0.get(monster.species as usize).is_some_and(|kind| kind.npc.is_none())
                || !damage.active(entity)
                || monster.is_dead()
                || !party.reaches(formation, member, at.translation(), hand.reach, distance)
            {
                return None;
            }
            let worst = monster.conditions.iter().map(|&(c, _)| c).max();
            let has_attack =
                species.0.get(monster.species as usize).is_some_and(|s| !s.attacks.is_empty());
            let side = formation::quadrant(party.eye, party.yaw, at.translation());
            let in_reach = has_attack
                && (side == Row::Rear
                    || (!formation.occupied(side) && formation.occupied(Row::Center)));
            Some(((worst, !in_reach, distance), entity))
        })
        .min_by(|a, b| a.0.0.cmp(&b.0.0).then(a.0.1.cmp(&b.0.1)).then(a.0.2.total_cmp(&b.0.2)))
        .map(|(_, entity)| entity);
    let turn = &mut battle.accepted.party[index].turn;
    let Some(target) = target else {
        // No target in reach: discard the remaining attacks this round.
        *turn = Turn::default();
        return None;
    };
    turn.attacks_left -= 1;
    turn.phase = next_phase(turn.phase, turn.attacks_left);
    battle.attacked_this_round = true;
    let kind = species.0.get(damage.monsters.get(target).ok()?.0.species as usize)?;
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
        if !able(&vitals) {
            break;
        }
        match character_swing(rng, &character, &vitals, &hand, mode, kind) {
            Swing::Hit(damage_amount) => {
                hits += 1;
                let outcome = damage
                    .apply(target, damage_amount, party_level, rng)
                    .expect("an active target and required catalogs were checked before attacking");
                dealt += outcome.damage;
                if outcome.died {
                    casualty = Some((target, outcome));
                }
            }
            Swing::NoPenetration => hits += 1,
            Swing::Missed => {}
        };
        let cost = attack::swing_stamina(rng, mode, weight, vitals.has(Condition::Diseased));
        vitals.spend_stamina(cost);
        if casualty.is_some() {
            break;
        }
    }
    // The summary, then the casualty (retail `ReportCharacterAttackResult`).
    news.write(CombatNews::Result { attacker: member, target, hits, damage: dealt });
    if casualty.is_some() {
        news.write(CombatNews::Killed { actor: target });
    }
    casualty
}

fn monster_acts(
    battle: &mut Battle,
    index: usize,
    rng: &mut GameRng,
    roster: &PartyRoster,
    members: &mut crate::game::magic::Members,
    monsters: &Query<(&mut Monster, ActorPose, Option<&Walker>, &LevelOwned)>,
    party: &PartyView,
    formation: &mut Mut<Formation>,
    (items, species): (&ItemCatalog, &MonsterCatalog),
    news: &mut MessageWriter<CombatNews>,
    damage: &mut MessageWriter<DamageApplied>,
) -> bool {
    let (entity, turn) = battle.accepted.monsters[index];
    let Ok((monster, at, walker, _)) = monsters.get(entity) else { return false };
    let Some(kind) = species.0.get(monster.species as usize) else { return false };
    let distance = distance_to_party(party.eye, monster, feet(at.translation(), walker));
    // Every (attack, mode, member) that reaches is equally likely.
    let mut candidates = vec![];
    for (a, attack) in kind.attacks.iter().enumerate() {
        for member in roster.iter() {
            let Ok((_, vitals, _)) = members.get(member) else { continue };
            if able(vitals)
                && party.reaches(formation, member, at.translation(), attack.reach, distance)
            {
                for mode in attack.modes {
                    candidates.push((a, mode, member));
                }
            }
        }
    }
    let turn_left = turn.attacks_left.saturating_sub(1);
    battle.accepted.monsters[index].1 = Turn { phase: 0, attacks_left: 0 };
    if candidates.is_empty() {
        return true;
    }
    battle.attacked_this_round = true;
    let (a, mode, member) = candidates[rng.below(candidates.len() as u32) as usize];
    let attack = &kind.attacks[a];
    // Retail draws for the mode again, from just the one chosen.
    let mode = attack::choose_mode(rng, EnumSet::only(mode));
    let Ok((character, mut vitals, carried)) = members.get_mut(member) else { return false };
    let mut armour = armour(&character, &carried, items);
    // Defending adds 2 to armour.
    if battle.defends(member) {
        armour.total += 2;
        armour.by_location.values_mut().for_each(|a| *a += 2);
    }
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
    battle.accepted.monsters[index].1 =
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
    members: &mut crate::game::magic::Members,
    monsters: &mut Query<(&mut Monster, ActorPose, Option<&Walker>, &LevelOwned)>,
    party_eye: Vec3,
    species: &MonsterCatalog,
    news: &mut MessageWriter<CombatNews>,
    (points, spells): (
        &mut Query<(&LearnedSpells, &mut SpellPoints)>,
        Option<&crate::content::spells::SpellCatalog>,
    ),
) -> bool {
    // The round's time passes now, with its meaning: conditions age, spell
    // points come back at half rate and nothing else recovers (retail
    // `AdvanceCombatRound`, `AdvanceTimedEffects`).
    let ticks = clock.advance(ROUND_MS);
    for &entity in combatants {
        if let Ok((mut monster, ..)) = monsters.get_mut(entity)
            && crate::game::characters::any_timed(&monster.conditions)
        {
            crate::game::characters::age(&mut monster.conditions, ticks.aging);
        }
    }
    if let Some(spells) = spells {
        for member in roster.iter() {
            let (Ok((character, _, _)), Ok((learned, mut pool))) =
                (members.get(member), points.get_mut(member))
            else {
                continue;
            };
            let maximum = crate::game::magic::max_points(character, learned, spells);
            if maximum.iter().any(|(realm, &max)| pool.current[realm] < max) {
                pool.recover(&maximum, ticks.aging, crate::game::magic::recovery_scale(0.0));
            }
        }
    }
    for (profile, mut vitals, _) in members.iter_many_unique_mut(roster.iter()).matched() {
        if recovery::has_work(&profile, &vitals, ticks, recovery::Pace::Fighting) {
            recovery::elapse(&profile, &mut vitals, ticks, recovery::Pace::Fighting);
        }
    }
    battle.phase = RoundPhase::Waiting;
    // Recover after aging conditions, in accepted actor order.
    for work in &battle.accepted.party {
        let member = work.actor;
        if !roster.contains(&member) {
            continue;
        }
        let Ok((character, mut vitals, _)) = members.get_mut(member) else { continue };
        let resting = matches!(work.action, AcceptedAction::Defend)
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
    let approaching = combatants.iter().any(|&entity| {
        monsters.get(entity).is_ok_and(|(monster, at, walker, _)| {
            approach::needs_approach(monster, at.translation(), walker, party_eye, species)
        })
    });
    let active = battle.attacked_this_round || approaching;
    battle.passive_rounds = if active { 0 } else { battle.passive_rounds + 1 };
    let hostiles_left =
        combatants.iter().any(|&e| monsters.get(e).is_ok_and(|(m, ..)| !m.is_dead()));
    if hostiles_left && battle.passive_rounds < PASSIVE_ROUNDS {
        return false;
    }
    // Experience is shared among the living, the unconscious included.
    let living =
        members.iter_many(roster.iter()).matched().filter(|(_, v, _)| !v.is_dead()).count() as u32;
    let each = battle.experience / living.max(1);
    if each > 0 {
        for (mut character, vitals, _) in members.iter_many_unique_mut(roster.iter()).matched() {
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
            .init_resource::<ItemIds>()
            .init_resource::<GameClock>()
            .insert_resource(GameRng::seeded(5))
            .add_message::<DamageApplied>()
            .init_resource::<Assets<crate::content::level::LevelDefinition>>()
            .add_message::<crate::game::world::identify::Identified>()
            .add_plugins(CombatPlugin);
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
        use crate::content::spells::SpellCatalog;
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
        app.world_mut().write_message(ChoosePartyMove(Some(PartyPace::Walk)));
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
        app.update();
        app.world_mut().get_mut::<Position>(body).unwrap().0 += Vec3::X * 21.0;
        app.update();
        news.extend(app.world_mut().resource_mut::<Messages<CombatNews>>().drain());
        assert!(news.contains(&CombatNews::PartyStops));
        assert!(!fight(app.world()).unwrap().party_may_move());
        let pending =
            fight(app.world()).unwrap().accepted.party.iter().filter(|p| p.turn.phase > 0).count();
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
        app.world_mut().get_mut::<Level>(level).unwrap().geometry = geometry;
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
                .geometry = geometry;
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
