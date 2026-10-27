//! The party's fights (`docs/research/combat.md`), first slice: the party
//! against the hostile monsters that see it, weapon attacks only, everyone
//! fighting on their own (retail's continuous mode with every member set to
//! attack). Who reaches whom in melee follows the party's formation.

pub mod approach;
pub mod attack;

use crate::game::world::ActorPose;
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
use crate::game::characters::{Character, Condition, Vitals};
use crate::game::data::GameData;
use crate::game::inventory::{Carried, EquipSlot, StackId};
use crate::game::party::body::PartyBody;
use crate::game::party::formation::{self, Formation, Row};
use crate::game::party::recovery;
use crate::game::party::roster::PartyRoster;
use crate::game::party::{DamageApplied, Order};
use crate::game::rng::GameRng;
use crate::game::time::GameClock;
use crate::game::world::damage::{DamageOutcome, MonsterDamage};
use crate::game::world::identify::GroupRecognition;
use crate::game::world::population::Monster;
use attack::{
    Swing, armour, character_initiative, character_swing, monster_swing, phase, primary_hand,
};

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<CombatNews>()
            .add_message::<StartRound>()
            .init_resource::<CombatSettings>()
            .add_systems(
                Update,
                (settle_defeat, start_battle, run_battle, settle_defeat, approach::close_in)
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
    Attack { weapon: Option<StackId> },
    Defend,
}

#[derive(Clone, Copy, Debug)]
struct PartyTurn {
    actor: Entity,
    action: AcceptedAction,
    turn: Turn,
}

/// The party's fight. Exists only while there is one.
#[derive(Resource, Debug, Default)]
pub struct Battle {
    pub round: u32,
    phase: RoundPhase,
    party: Vec<PartyTurn>,
    /// The monsters in the fight, in a stable order (by location id).
    monsters: Vec<(Entity, Turn)>,
    /// Experience from kills, shared at the end.
    experience: u32,
    kills: u32,
    passive_rounds: u32,
    attacked_this_round: bool,
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
        self.party.iter().any(|p| p.actor == actor && matches!(p.action, AcceptedAction::Defend))
    }

    /// Whether a monster is in the fight.
    pub fn fights(&self, monster: Entity) -> bool {
        self.monsters.iter().any(|&(m, _)| m == monster)
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
    battle: Option<Res<Battle>>,
    camp: Option<Res<crate::game::camp::Camp>>,
    mut party: Query<(&Position, &mut PartyBody)>,
    level: PlayedLevel,
    members: Query<(&Character, &Vitals)>,
    monsters: Query<(Entity, &Monster, ActorPose, Option<&Walker>)>,
    obstacles: WorldObstacles,
    data: Option<Res<GameData>>,
    species: Res<Assets<MonsterCatalog>>,
    roster: Res<PartyRoster>,
    mut news: MessageWriter<CombatNews>,
    mut recognition: GroupRecognition,
    mut rng: ResMut<GameRng>,
) {
    if battle.is_some()
        || camp.is_some()
        || !members.iter_many(&roster.0).matched().any(|(_, v)| !v.is_dead())
    {
        return;
    }
    let (Ok((party_at, mut body)), Some(catalog)) =
        (party.single_mut(), data.and_then(|d| species.get(&d.species)))
    else {
        return;
    };
    let Some(far) = level.view_distance() else { return };
    let party_eye = PartyBody::eye(party_at);
    // The party's lowest level among those who can act weakens sight
    // from behind.
    let lowest_level = members
        .iter_many(&roster.0)
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
        .filter(|(_, monster, at, walker)| {
            let Some(kind) = catalog.0.get(monster.species as usize) else { return false };
            if kind.npc || !monster_can_attack(monster) {
                return false;
            }
            let feet = feet(at.translation(), *walker);
            let distance = distance_to_party(party_eye, monster, feet);
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
        .map(|(_, _, at, _)| at.translation())
        .min_by(|a, b| a.distance(party_eye).total_cmp(&b.distance(party_eye)));
    if let Some(nearest) = nearest {
        body.face(nearest - party_eye);
    }
    let battle = Battle {
        monsters: joined.into_iter().map(|(_, e)| (e, Turn::default())).collect(),
        ..default()
    };
    recognition.detect(party_eye, body.forward(), Some(&battle), &mut rng);
    commands.insert_resource(battle);
    news.write(CombatNews::Began);
}

/// Terminal state is settled independently of whose action dealt the damage.
/// Used before/after combat execution and at the end of environmental damage.
pub fn settle_defeat(
    mut commands: Commands,
    battle: Option<Res<Battle>>,
    roster: Res<PartyRoster>,
    members: Query<&Vitals>,
    mut news: MessageWriter<CombatNews>,
) {
    if battle.is_some() && !members.iter_many(&roster.0).matched().any(|v| !v.is_dead()) {
        news.write(CombatNews::Lost);
        commands.remove_resource::<Battle>();
    }
}

/// Advance the fight by one action when its pause is over.
pub fn run_battle(
    mut commands: Commands,
    time: Res<Time>,
    battle: Option<ResMut<Battle>>,
    mut rng: ResMut<GameRng>,
    mut clock: ResMut<GameClock>,
    roster: Res<PartyRoster>,
    mut actors: ParamSet<(
        (Query<(&mut Character, &mut Vitals, &Carried)>, MonsterDamage),
        GroupRecognition,
    )>,
    party_body: Query<(&Position, &PartyBody)>,
    mut formation: ResMut<Formation>,
    data: Option<Res<GameData>>,
    (items, species): (Res<Assets<ItemCatalog>>, Res<Assets<MonsterCatalog>>),
    (mut news, mut party_damage): (MessageWriter<CombatNews>, MessageWriter<DamageApplied>),
    (mut starts, settings, orders): (MessageReader<StartRound>, Res<CombatSettings>, Query<&Order>),
) {
    let Some(mut battle) = battle else {
        starts.clear();
        return;
    };
    // Accept input before any pacing/content early return. The existing round
    // state owns the request; it must not wait in the transient message buffer.
    let start = starts.read().count() > 0;
    if start && !battle.executing() {
        battle.phase = RoundPhase::Accepted;
    }
    let (mut members, mut damage) = actors.p0();
    if !damage.ready() {
        return;
    }
    let (Some(items), Some(species)) = (
        data.as_ref().and_then(|d| items.get(&d.items)),
        data.as_ref().and_then(|d| species.get(&d.species)),
    ) else {
        return;
    };
    let Ok((position, body)) = party_body.single() else { return };
    let party = PartyView { eye: PartyBody::eye(position), yaw: body.yaw };
    // A finished pause is not ticked again: a fight waiting for the player
    // stays unchanged.
    if !battle.wait.is_finished() && !battle.wait.tick(time.delta()).is_finished() {
        return;
    }

    // Between rounds a phased fight waits for the player; a continuous one
    // goes on (retail `UpdateCombat`, `BeginCombatExecution`).
    if !battle.executing() {
        if !settings.continuous {
            return;
        }
        battle.phase = RoundPhase::Accepted;
    }
    if battle.phase == RoundPhase::Accepted {
        // A new round: the party's phases, then the monsters'.
        battle.round += 1;
        battle.attacked_this_round = false;
        battle.party.clear();
        for &member in &roster.0 {
            let Ok((character, vitals, carried)) = members.get(member) else { continue };
            let action = match orders.get(member).copied().unwrap_or_default() {
                Order::Attack => AcceptedAction::Attack {
                    weapon: carried.worn(EquipSlot::Primary).map(|i| i.id),
                },
                Order::Defend => AcceptedAction::Defend,
            };
            let turn = if matches!(action, AcceptedAction::Attack { .. }) && able(vitals) {
                let hand = primary_hand(character, carried, items);
                Turn {
                    phase: phase(&mut rng, character_initiative(character, &hand)),
                    attacks_left: hand.attacks,
                }
            } else {
                Turn::default()
            };
            battle.party.push(PartyTurn { actor: member, action, turn });
        }
        for index in 0..battle.monsters.len() {
            let entity = battle.monsters[index].0;
            let Ok((monster, ..)) = damage.monsters.get(entity) else { continue };
            let Some(kind) = species.0.get(monster.species as usize) else { continue };
            battle.monsters[index].1 = if monster.is_dead() {
                Turn::default()
            } else {
                Turn {
                    phase: phase(&mut rng, i32::from(kind.initiative)),
                    attacks_left: u32::from(kind.attacks_per_round),
                }
            };
        }
        battle.phase = RoundPhase::Executing(1);
    }

    // The next actor due: accepted party order first, then monsters in order.
    while let RoundPhase::Executing(step) = battle.phase {
        if step > 100 {
            end_round(
                &mut commands,
                &mut battle,
                &mut clock,
                &mut rng,
                &roster,
                &mut members,
                &damage.monsters,
                party.eye,
                species,
                &mut news,
            );
            // Recognition is a required part of this round, before the next
            // accepted action or a removed battle can change its meaning.
            actors.p1().detect(party.eye, body.forward(), Some(&battle), &mut rng);
            return;
        }
        if let Some(index) = battle.party.iter().position(|p| p.turn.phase == step) {
            let work = battle.party[index];
            let member = work.actor;
            let able_now = roster.0.contains(&member) && members.get(member).is_ok_and(|(_, vitals, carried)| {
                able(vitals) && matches!(work.action, AcceptedAction::Attack { weapon } if weapon == carried.worn(EquipSlot::Primary).map(|i| i.id))
            });
            if !able_now {
                battle.party[index].turn = Turn::default();
                continue;
            }
            // Retail `GetAveragePartyLevel`: what loot tables scale by.
            let levels_sum: u32 =
                members.iter_many(&roster.0).matched().map(|(c, _, _)| u32::from(c.level)).sum();
            let party_level = levels_sum / roster.0.len().max(1) as u32;
            let casualty = member_acts(
                &mut battle,
                index,
                member,
                &mut rng,
                &mut members,
                &mut damage,
                &party,
                &formation,
                items,
                species,
                party_level,
                &mut news,
            );
            if let Some((target, outcome)) = casualty {
                battle.experience += outcome.experience;
                battle.kills += 1;
                if let Some((_, turn)) =
                    battle.monsters.iter_mut().find(|(actor, _)| *actor == target)
                {
                    *turn = Turn::default();
                }
            }
            battle.wait = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
            return;
        }
        if let Some(index) = battle.monsters.iter().position(|(_, t)| t.phase == step) {
            let entity = battle.monsters[index].0;
            if !damage.monsters.get(entity).is_ok_and(|(m, ..)| monster_can_attack(m)) {
                battle.monsters[index].1 = Turn::default();
                continue;
            }
            monster_acts(
                &mut battle,
                index,
                &mut rng,
                &roster,
                &mut members,
                &damage.monsters,
                &party,
                &mut formation,
                items,
                species,
                &mut news,
                &mut party_damage,
            );
            battle.wait = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
            return;
        }
        battle.phase = RoundPhase::Executing(step + 1);
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

fn member_acts(
    battle: &mut Battle,
    index: usize,
    member: Entity,
    rng: &mut GameRng,
    members: &mut Query<(&mut Character, &mut Vitals, &Carried)>,
    damage: &mut MonsterDamage,
    party: &PartyView,
    formation: &Formation,
    items: &ItemCatalog,
    species: &MonsterCatalog,
    party_level: u32,
    news: &mut MessageWriter<CombatNews>,
) -> Option<(Entity, DamageOutcome)> {
    let Ok((character, mut vitals, carried)) = members.get_mut(member) else { return None };
    let hand = primary_hand(&character, carried, items);
    // Retail `ChooseMonsterTarget`: among the monsters this hand reaches,
    // the least afflicted, then those "in reach" of the party, then the
    // nearest; ties keep the fight's order.
    let target = battle
        .monsters
        .iter()
        .filter_map(|&(entity, _)| {
            let (monster, at, walker, _) = damage.monsters.get(entity).ok()?;
            let distance = distance_to_party(party.eye, monster, feet(at.translation(), walker));
            if !species.0.get(monster.species as usize).is_some_and(|kind| !kind.npc)
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
    let turn = &mut battle.party[index].turn;
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
    members: &mut Query<(&mut Character, &mut Vitals, &Carried)>,
    monsters: &Query<(&mut Monster, ActorPose, Option<&Walker>, &ChildOf)>,
    party: &PartyView,
    formation: &mut ResMut<Formation>,
    items: &ItemCatalog,
    species: &MonsterCatalog,
    news: &mut MessageWriter<CombatNews>,
    damage: &mut MessageWriter<DamageApplied>,
) {
    let (entity, turn) = battle.monsters[index];
    let Ok((monster, at, walker, _)) = monsters.get(entity) else { return };
    let Some(kind) = species.0.get(monster.species as usize) else { return };
    let distance = distance_to_party(party.eye, monster, feet(at.translation(), walker));
    // Every (attack, mode, member) that reaches is equally likely.
    let mut candidates = vec![];
    for (a, attack) in kind.attacks.iter().enumerate() {
        for &member in &roster.0 {
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
    battle.monsters[index].1 = Turn { phase: 0, attacks_left: 0 };
    if candidates.is_empty() {
        return;
    }
    battle.attacked_this_round = true;
    let (a, mode, member) = candidates[rng.below(candidates.len() as u32) as usize];
    let attack = &kind.attacks[a];
    // Retail draws for the mode again, from just the one chosen.
    let mode = attack::choose_mode(rng, EnumSet::only(mode));
    let Ok((character, mut vitals, carried)) = members.get_mut(member) else { return };
    let mut armour = armour(&character, carried, items);
    // Defending is worth 2 (`characters.md` §4.8, component 10).
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
            }
            Swing::NoPenetration => hits += 1,
            Swing::Missed => {}
        };
    }
    news.write(CombatNews::Result { attacker: entity, target: member, hits, damage: dealt });
    if vitals.is_dead() {
        news.write(CombatNews::Killed { actor: member });
    }
    battle.monsters[index].1 =
        Turn { phase: next_phase(turn.phase, turn_left), attacks_left: turn_left };
}

/// The round is over: a round's game time passes, and the fight ends when
/// no hostile is left or nobody could attack for two rounds.
fn end_round(
    commands: &mut Commands,
    battle: &mut Battle,
    clock: &mut GameClock,
    rng: &mut GameRng,
    roster: &PartyRoster,
    members: &mut Query<(&mut Character, &mut Vitals, &Carried)>,
    monsters: &Query<(&mut Monster, ActorPose, Option<&Walker>, &ChildOf)>,
    party_eye: Vec3,
    species: &MonsterCatalog,
    news: &mut MessageWriter<CombatNews>,
) {
    // The round's time passes now, with its meaning: conditions age and
    // nobody recovers (retail `AdvanceCombatRound`).
    let ticks = clock.advance(ROUND_MS);
    for (profile, mut vitals, _) in members.iter_many_unique_mut(&roster.0).matched() {
        if recovery::has_work(&profile, &vitals, ticks, recovery::Pace::Fighting) {
            recovery::elapse(&profile, &mut vitals, ticks, recovery::Pace::Fighting);
        }
    }
    battle.phase = RoundPhase::Waiting;
    // Recover after aging conditions, in accepted actor order.
    for work in &battle.party {
        let member = work.actor;
        if !roster.0.contains(&member) {
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
    let approaching = battle.monsters.iter().any(|&(entity, _)| {
        monsters.get(entity).is_ok_and(|(monster, at, walker, _)| {
            approach::needs_approach(monster, at.translation(), walker, party_eye, species)
        })
    });
    let active = battle.attacked_this_round || approaching;
    battle.passive_rounds = if active { 0 } else { battle.passive_rounds + 1 };
    let hostiles_left =
        battle.monsters.iter().any(|&(e, _)| monsters.get(e).is_ok_and(|(m, ..)| !m.is_dead()));
    if hostiles_left && battle.passive_rounds < PASSIVE_ROUNDS {
        return;
    }
    // Experience is shared among the living, the unconscious included.
    let living =
        members.iter_many(&roster.0).matched().filter(|(_, v, _)| !v.is_dead()).count() as u32;
    let each = battle.experience / living.max(1);
    if each > 0 {
        for (mut character, vitals, _) in members.iter_many_unique_mut(&roster.0).matched() {
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
    commands.remove_resource::<Battle>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::items::{ItemId, LootTables};
    use crate::game::inventory::{ItemIds, PartyInventory, make_item};

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
            .init_resource::<Assets<LootTables>>()
            .init_resource::<avian3d::collider_tree::ColliderTrees>()
            .init_resource::<PartyRoster>()
            .init_resource::<Formation>()
            .init_resource::<PartyInventory>()
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
            app.world_mut().resource_mut::<PartyRoster>().0.insert(entity);
            app.world_mut().resource_mut::<Formation>().seat(entity);
            members.push(entity);
        }
        let items = app.world_mut().resource_mut::<Assets<ItemCatalog>>().add(ItemCatalog(weapons));
        let species =
            app.world_mut().resource_mut::<Assets<MonsterCatalog>>().add(MonsterCatalog(vec![
                Species { name: "Target".into(), generic_name: "Target".into(), ..default() },
            ]));
        let loot = app.world_mut().resource_mut::<Assets<LootTables>>().add(LootTables::default());
        app.insert_resource(GameData { items, species, loot });
        app.world_mut().spawn((PartyBody::default(), Position::default()));
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
                ChildOf(root),
            ))
            .id();
        app.insert_resource(Battle { monsters: vec![(monster, Turn::default())], ..default() });
        app.finish();
        app.cleanup();
        app.world_mut().write_message(StartRound);
        (app, [members[0], members[1]])
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
        let actor = app.world().resource::<Battle>().monsters[0].0;
        app.world_mut().entity_mut(actor).insert((
            Walker { speed: 2.0, float_height: 1.0 },
            Transform::from_translation(Vec3::NEG_Z * 20.0),
        ));
        {
            let mut battle = app.world_mut().resource_mut::<Battle>();
            battle.phase = RoundPhase::Executing(101);
            battle.passive_rounds = 1;
        }
        app.update();
        assert!(app.world().contains_resource::<Battle>(), "a living walker is still approaching");
        assert_eq!(app.world().resource::<Battle>().passive_rounds, 0);
        app.world_mut().get_mut::<Transform>(actor).unwrap().translation = Vec3::NEG_Z;
        {
            let mut battle = app.world_mut().resource_mut::<Battle>();
            battle.phase = RoundPhase::Executing(101);
            battle.passive_rounds = 1;
            battle.wait = default();
        }
        app.update();
        assert!(!app.world().contains_resource::<Battle>(), "the actor is now in reach and idle");
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
        let group =
            app.world_mut().spawn((MonsterGroup { authored: 1, ..default() }, ChildOf(level))).id();
        app.world_mut().entity_mut(monster).insert(GroupMember(group));
        let outsider_group =
            app.world_mut().spawn((MonsterGroup { authored: 2, ..default() }, ChildOf(level))).id();
        let outsider = app.world().get::<Monster>(monster).unwrap().clone();
        app.world_mut().spawn((
            outsider,
            Transform::from_translation(Vec3::NEG_Z),
            GroupMember(outsider_group),
            ChildOf(level),
        ));
        app.world_mut().resource_mut::<CombatSettings>().continuous = true;
        for _ in 0..12 {
            app.update();
            if !app.world().contains_resource::<Battle>() {
                break;
            }
        }
        assert!(!app.world().contains_resource::<Battle>(), "a passive fight ends");
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
                if !app.world().resource::<Battle>().executing() {
                    break;
                }
            }
            assert!(!app.world().resource::<Battle>().executing());
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
        app.world_mut().resource_mut::<PartyRoster>().0 = [b, a].into_iter().collect();
        app.world_mut().entity_mut(b).insert(Order::Defend);
        app.update();
        assert_eq!(
            attacks(&mut app),
            ["B"],
            "editing the next order cannot retarget or replace accepted work"
        );
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
        let mut params =
            bevy::ecs::system::SystemState::<(ResMut<PartyRoster>, ResMut<Formation>)>::new(
                app.world_mut(),
            );
        let (mut roster, mut formation) = params.get_mut(app.world_mut()).unwrap();
        assert!(roster.remove(b, &mut formation));
        roster.0.insert(replacement);
        formation.seat(replacement);
        app.update();
        assert!(attacks(&mut app).is_empty());
        assert!(
            app.world().get::<Character>(b).is_some(),
            "a retained profile is no longer a member"
        );
        assert!(app.world().resource::<Formation>().place_of(b).is_none());
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
            if !app.world().resource::<Battle>().executing() {
                break;
            }
            app.update();
        }
        assert!(attacks(&mut app).is_empty());
        assert!(app.world().get::<Vitals>(b).unwrap().stamina > before);
        assert!(!app.world().resource::<Battle>().executing());
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
        app.world_mut().resource_mut::<Battle>().wait = Timer::from_seconds(1.0, TimerMode::Once);
        app.update();
        assert!(app.world().resource::<Battle>().executing());
        let mut announced = Vec::new();
        for _ in 0..12 {
            app.update();
            announced.extend(attacks(&mut app));
        }
        assert_eq!(app.world().resource::<Battle>().round, 1);
        assert_eq!(announced, ["A"]);
    }

    #[test]
    fn defeat_settles_without_round_input_content_or_an_expired_timer() {
        let (mut app, members) = round_app(false);
        app.world_mut().resource_mut::<Battle>().wait = Timer::from_seconds(60.0, TimerMode::Once);
        app.world_mut().resource_mut::<Messages<StartRound>>().clear();
        let data = app.world_mut().remove_resource::<GameData>().unwrap();
        for member in members {
            app.world_mut().get_mut::<Vitals>(member).unwrap().hp = 0;
        }
        app.update();
        assert!(!app.world().contains_resource::<Battle>());
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
        assert!(!app.world().contains_resource::<Battle>());
        assert!(app.world().resource::<Messages<CombatNews>>().is_empty());
        app.world_mut().get_mut::<Vitals>(members[0]).unwrap().hp = 1;
        app.update();
        assert!(app.world().contains_resource::<Battle>(), "the hostile can engage a live party");
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
            if !app.world().contains_resource::<Battle>() {
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
            app.world_mut().resource_mut::<PartyRoster>().0.insert(c);
            app.world_mut().resource_mut::<Formation>().seat(c);
            app.world_mut().get_mut::<Vitals>(b).unwrap().stamina = 1;
            app.update();
            if reorder {
                app.world_mut().resource_mut::<PartyRoster>().0 = [c, a, b].into_iter().collect();
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
