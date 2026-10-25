//! The party's fights (`docs/research/combat.md`), first slice: the party
//! against the hostile monsters that see it, weapon attacks only, everyone
//! fighting on their own (retail's continuous mode with every member set to
//! attack). Who reaches whom in melee follows the party's formation.

pub mod approach;
pub mod attack;

use avian3d::prelude::{Position, RigidBody, SpatialQuery, SpatialQueryFilter};
use bevy::prelude::*;

use crate::content::items::{Dice, ItemCatalog};
use crate::content::monster::MonsterModel;
use crate::content::species::{MonsterCatalog, Reach};
use crate::game::characters::{Character, Condition};
use crate::game::data::GameData;
use crate::game::inventory::{Carried, PartyInventory};
use crate::game::party::body::{EYE_ABOVE_CENTRE, PartyBody, PartyCamera};
use crate::game::party::formation::{self, Formation, Row};
use crate::game::party::recovery;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::time::GameClock;
use crate::game::world::identify::{DetectGroups, MonsterGroups};
use crate::game::world::population::Monster;
use crate::presentation::monsters::MonsterAppearance;
use attack::{
    Swing, armour, character_initiative, character_swing, monster_swing, phase, primary_hand,
};

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<CombatNews>()
            .add_message::<StartRound>()
            .init_resource::<PartyOrders>()
            .init_resource::<CombatSettings>()
            .add_systems(
                Update,
                (start_battle, run_battle, approach::embody_fighters, approach::close_in)
                    .chain()
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

/// What a member does in a round (retail's slot action; `ResetPartySlotRow`
/// starts everyone on Attack).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    #[default]
    Attack,
    /// No attack; +2 armour, and stamina back at the round's end.
    Defend,
}

/// Each member's order by party slot; a slot without one attacks. Orders
/// outlast the fight, as retail remembers them.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct PartyOrders(pub Vec<Order>);

impl PartyOrders {
    pub fn of(&self, slot: usize) -> Order {
        self.0.get(slot).copied().unwrap_or_default()
    }

    pub fn set(&mut self, slot: usize, order: Order) {
        if self.0.len() <= slot {
            self.0.resize(slot + 1, Order::Attack);
        }
        self.0[slot] = order;
    }
}

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

/// The party's fight. Exists only while there is one.
#[derive(Resource, Debug)]
pub struct Battle {
    pub round: u32,
    /// A round is being played out; between rounds a phased fight waits for
    /// the player (retail `execution_active`).
    pub executing: bool,
    /// The phase being played, 1..=100; 0 between rounds.
    step: u8,
    party: Vec<Turn>,
    /// The monsters in the fight, in a stable order (by location id).
    monsters: Vec<(Entity, Turn)>,
    /// Experience from kills, shared at the end.
    experience: u32,
    kills: u32,
    passive_rounds: u32,
    attacked_this_round: bool,
    /// Monsters are walking up to the party: the round is not passive.
    approaching: bool,
    /// Seconds until the next action.
    wait: f32,
}

impl Battle {
    /// Whether a monster is in the fight.
    pub fn fights(&self, monster: Entity) -> bool {
        self.monsters.iter().any(|&(m, _)| m == monster)
    }

    /// How far through the round its play is, 0 to 1, while a round is
    /// being played.
    pub fn progress(&self) -> Option<f32> {
        self.executing.then_some(f32::from(self.step.min(100)) / 100.0)
    }
}

/// How an attack is named: by its mode's verb ("Swings", "Punches"), or for
/// a monster attack by its own word, with retail's weapon-name flag
/// deciding whether "at" follows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Announcement {
    Mode(attack::Mode),
    Monster { weapon_type: u8, mode: attack::Mode, named: bool },
}

/// What the fight tells the player.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum CombatNews {
    Began,
    /// An attack begins (retail `StartCharacterAttack`,
    /// `AnnounceMonsterAttack`).
    Attack {
        attacker: String,
        target: String,
        how: Announcement,
        swings: u32,
    },
    /// What it did: hits (that landed, damage or not) and damage dealt.
    Result {
        attacker: String,
        hits: u32,
        damage: u32,
    },
    Killed {
        name: String,
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
fn able(member: &Character) -> bool {
    !member.is_dead() && member.worst_condition().is_none_or(|c| c < Condition::Asleep)
}

/// How far a monster is from the party (retail `GetPointDistanceToPlayer`):
/// from its feet to the point 1000 world units (2 m) under the party's eye,
/// less both bodies' alternate radii (its fight radius, the party's 1 m),
/// never below 0. Reach and sight compare against this.
fn distance_to_party(
    party_eye: Vec3,
    monster: &Monster,
    at: &GlobalTransform,
    look: &MonsterAppearance,
) -> f32 {
    let feet = at.translation() - Vec3::Y * look.float_height;
    let party_point = party_eye - Vec3::Y * CAMERA_ABOVE_PARTY_POINT;
    ((feet - party_point).length() - monster.radius - PARTY_ALTERNATE_RADIUS).max(0.0)
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
struct PartyView<'a> {
    eye: Vec3,
    yaw: f32,
    formation: &'a Formation,
    orders: &'a PartyOrders,
}

impl PartyView<'_> {
    /// Whether the member in `slot` reaches, or is reached by, a monster at
    /// `at` with `reach` across `distance`: the formation rows between them
    /// shorten close reach (retail `CloseFormationGap`).
    fn reaches(&self, slot: usize, at: Vec3, reach: Reach, distance: f32) -> bool {
        let side = formation::quadrant(self.eye, self.yaw, at);
        self.formation.reach(slot, side, reach).is_some_and(|r| distance <= r.metres())
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
    camera: Query<&Projection, With<PartyCamera>>,
    members: Query<&Character>,
    monsters: Query<(Entity, &Monster, &GlobalTransform, &MonsterAppearance)>,
    models: Res<Assets<MonsterModel>>,
    spatial: SpatialQuery,
    bodies: Query<&RigidBody>,
    data: Option<Res<GameData>>,
    species: Res<Assets<MonsterCatalog>>,
    roster: Res<PartyRoster>,
    mut news: MessageWriter<CombatNews>,
    mut detect: MessageWriter<crate::game::world::identify::DetectGroups>,
) {
    if battle.is_some() || camp.is_some() {
        return;
    }
    let (Ok((party_at, mut body)), Some(catalog)) =
        (party.single_mut(), data.and_then(|d| species.get(&d.species)))
    else {
        return;
    };
    let far = match camera.single() {
        Ok(Projection::Perspective(perspective)) => perspective.far,
        _ => return,
    };
    let party_eye = party_at.0 + Vec3::Y * EYE_ABOVE_CENTRE;
    // The party's lowest level among those who can act weakens sight
    // from behind.
    let lowest_level = roster
        .0
        .iter()
        .filter_map(|&m| members.get(m).ok())
        .filter(|c| !c.is_dead() && c.worst_condition().is_none_or(|w| w < Condition::Asleep))
        .map(|c| i32::from(c.level))
        .min()
        .unwrap_or(0);
    // Only the level's static geometry blocks sight.
    let level = |entity| bodies.get(entity).is_ok_and(RigidBody::is_static);
    let clear = |from: Vec3, to: Vec3| {
        let Ok(direction) = Dir3::new(to - from) else { return true };
        let filter = SpatialQueryFilter::default();
        spatial
            .cast_ray_predicate(from, direction, from.distance(to), true, &filter, &level)
            .is_none()
    };
    let mut joined: Vec<(i32, Entity)> = monsters
        .iter()
        .filter(|(_, monster, at, look)| {
            let Some(kind) = catalog.0.get(monster.species as usize) else { return false };
            if kind.npc || monster.is_dead() || monster.radius == 0.0 {
                return false;
            }
            if monster.conditions.iter().any(|&(c, _)| c > Condition::Webbed) {
                return false;
            }
            let distance = distance_to_party(party_eye, monster, at, look);
            let eye = at.translation() - Vec3::Y * look.float_height + Vec3::Y * monster.eye_height;
            if distance > far || !clear(eye, party_eye) {
                return false;
            }
            let forward = at.forward().with_y(0.0);
            let to_party = (party_eye - eye).with_y(0.0);
            let angle = forward.angle_between(to_party);
            let senses = i32::from(kind.attributes[4]);
            let bonus = if kind.kind == 4 { (u32::from(kind.level) * 5).min(125) } else { 0 };
            if sight_threshold(angle, senses, bonus, lowest_level, far, distance) < distance {
                return false;
            }
            let reaches = kind.attacks.iter().any(|a| distance <= a.reach.metres());
            let close =
                kind.attacks.iter().map(|a| a.reach).max().is_some_and(|r| r <= Reach::Short);
            let walks = models.get(&look.model).is_some_and(|m| m.body.movement_rate > 0.0);
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
    let nearest = joined
        .iter()
        .filter_map(|&(_, e)| monsters.get(e).ok())
        .map(|(_, _, at, _)| at.translation())
        .min_by(|a, b| a.distance(party_eye).total_cmp(&b.distance(party_eye)));
    if let Some(nearest) = nearest {
        let to = nearest - party_eye;
        body.yaw = (-to.x).atan2(-to.z);
    }
    commands.insert_resource(Battle {
        round: 0,
        executing: false,
        step: 0,
        party: vec![Turn::default(); roster.0.len()],
        monsters: joined.into_iter().map(|(_, e)| (e, Turn::default())).collect(),
        experience: 0,
        kills: 0,
        passive_rounds: 0,
        attacked_this_round: false,
        approaching: false,
        wait: 0.0,
    });
    news.write(CombatNews::Began);
    detect.write(crate::game::world::identify::DetectGroups);
}

/// Advance the fight by one action when its pause is over.
#[allow(clippy::too_many_arguments)]
pub fn run_battle(
    mut commands: Commands,
    time: Res<Time>,
    battle: Option<ResMut<Battle>>,
    mut rng: ResMut<GameRng>,
    mut clock: ResMut<GameClock>,
    roster: Res<PartyRoster>,
    mut members: Query<(&mut Character, &Carried)>,
    mut monsters: Query<(&mut Monster, &GlobalTransform, &MonsterAppearance)>,
    party_body: Query<(&Position, &PartyBody)>,
    formation: Res<Formation>,
    mut purse: ResMut<PartyInventory>,
    data: Option<Res<GameData>>,
    items: Res<Assets<ItemCatalog>>,
    species: Res<Assets<MonsterCatalog>>,
    mut news: MessageWriter<CombatNews>,
    (mut starts, settings, orders, groups, mut detect): (
        MessageReader<StartRound>,
        Res<CombatSettings>,
        Res<PartyOrders>,
        Query<&MonsterGroups>,
        MessageWriter<DetectGroups>,
    ),
) {
    let Some(mut battle) = battle else { return };
    let unknown = MonsterGroups::default();
    let groups = groups.single().unwrap_or(&unknown);
    let (Some(items), Some(species)) = (
        data.as_ref().and_then(|d| items.get(&d.items)),
        data.as_ref().and_then(|d| species.get(&d.species)),
    ) else {
        return;
    };
    let Ok((position, body)) = party_body.single() else { return };
    let party = PartyView {
        eye: position.0 + Vec3::Y * EYE_ABOVE_CENTRE,
        yaw: body.yaw,
        formation: &formation,
        orders: &orders,
    };
    battle.wait -= time.delta_secs();
    if battle.wait > 0.0 {
        return;
    }

    // Between rounds a phased fight waits for the player; a continuous one
    // goes on (retail `UpdateCombat`, `BeginCombatExecution`).
    let start = starts.read().count() > 0;
    if !battle.executing {
        if !(settings.continuous || start) {
            return;
        }
        battle.executing = true;
    }
    if battle.step == 0 {
        // A new round: the party's phases, then the monsters'.
        battle.round += 1;
        battle.attacked_this_round = false;
        for (slot, &member) in roster.0.iter().enumerate() {
            let Ok((character, carried)) = members.get(member) else { continue };
            battle.party[slot] = if orders.of(slot) == Order::Attack && able(character) {
                let hand = primary_hand(character, carried, items);
                Turn {
                    phase: phase(&mut rng, character_initiative(character, &hand)),
                    attacks_left: hand.attacks,
                }
            } else {
                Turn::default()
            };
        }
        for index in 0..battle.monsters.len() {
            let entity = battle.monsters[index].0;
            let Ok((monster, ..)) = monsters.get(entity) else { continue };
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
        battle.step = 1;
    }

    // The next actor due: party slots first, then monsters in order.
    loop {
        let step = battle.step;
        if step > 100 {
            end_round(
                &mut commands,
                &mut battle,
                &mut clock,
                &mut rng,
                &orders,
                &roster,
                &mut members,
                &monsters,
                &mut news,
                &mut detect,
            );
            return;
        }
        if let Some(slot) = battle.party.iter().position(|t| t.phase == step) {
            let member = roster.0[slot];
            let able_now = members.get(member).is_ok_and(|(c, _)| able(c));
            if !able_now {
                battle.party[slot] = Turn::default();
                continue;
            }
            member_acts(
                &mut battle,
                slot,
                member,
                &mut rng,
                &mut members,
                &mut monsters,
                &party,
                items,
                species,
                groups,
                &mut purse,
                &mut news,
            );
            battle.wait = ACTION_SECONDS;
            return;
        }
        if let Some(index) = battle.monsters.iter().position(|(_, t)| t.phase == step) {
            let entity = battle.monsters[index].0;
            if monsters.get(entity).is_ok_and(|(m, ..)| m.is_dead()) {
                battle.monsters[index].1 = Turn::default();
                continue;
            }
            monster_acts(
                &mut battle,
                index,
                &mut rng,
                &roster,
                &mut members,
                &monsters,
                &party,
                items,
                species,
                groups,
                &mut news,
            );
            battle.wait = ACTION_SECONDS;
            if !roster.0.iter().any(|&m| members.get(m).is_ok_and(|(c, _)| !c.is_dead())) {
                news.write(CombatNews::Lost);
                commands.remove_resource::<Battle>();
            }
            return;
        }
        battle.step += 1;
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

#[allow(clippy::too_many_arguments)]
fn member_acts(
    battle: &mut Battle,
    slot: usize,
    member: Entity,
    rng: &mut GameRng,
    members: &mut Query<(&mut Character, &Carried)>,
    monsters: &mut Query<(&mut Monster, &GlobalTransform, &MonsterAppearance)>,
    party: &PartyView,
    items: &ItemCatalog,
    species: &MonsterCatalog,
    groups: &MonsterGroups,
    purse: &mut PartyInventory,
    news: &mut MessageWriter<CombatNews>,
) {
    let Ok((mut character, carried)) = members.get_mut(member) else { return };
    let hand = primary_hand(&character, carried, items);
    // Retail `ChooseMonsterTarget`: among the monsters this hand reaches,
    // the least afflicted, then those "in reach" of the party, then the
    // nearest; ties keep the fight's order.
    let target = battle
        .monsters
        .iter()
        .filter_map(|&(entity, _)| {
            let (monster, at, look) = monsters.get(entity).ok()?;
            let distance = distance_to_party(party.eye, monster, at, look);
            if monster.is_dead() || !party.reaches(slot, at.translation(), hand.reach, distance) {
                return None;
            }
            let worst = monster.conditions.iter().map(|&(c, _)| c).max();
            let has_attack =
                species.0.get(monster.species as usize).is_some_and(|s| !s.attacks.is_empty());
            let side = formation::quadrant(party.eye, party.yaw, at.translation());
            let in_reach = has_attack
                && (side == Row::Rear
                    || (!party.formation.occupied(side) && party.formation.occupied(Row::Center)));
            Some(((worst, !in_reach, distance), entity))
        })
        .min_by(|a, b| a.0.0.cmp(&b.0.0).then(a.0.1.cmp(&b.0.1)).then(a.0.2.total_cmp(&b.0.2)))
        .map(|(_, entity)| entity);
    let turn = &mut battle.party[slot];
    let Some(target) = target else {
        // Nobody in reach: defend for the rest of the round.
        *turn = Turn::default();
        return;
    };
    turn.attacks_left -= 1;
    turn.phase = next_phase(turn.phase, turn.attacks_left);
    battle.attacked_this_round = true;
    let Ok((mut monster, ..)) = monsters.get_mut(target) else { return };
    let Some(kind) = species.0.get(monster.species as usize) else { return };
    let name = kind.shown_name(groups.identified(monster.group), 1).to_owned();
    // Retail draws the attack's mode, then its swings.
    let mode = attack::Mode::choose(rng, hand.modes);
    let swings = rng.below(hand.swings) + 1;
    news.write(CombatNews::Attack {
        attacker: character.name.clone(),
        target: name.clone(),
        how: Announcement::Mode(mode),
        swings,
    });
    let (mut hits, mut dealt, mut killed) = (0, 0, false);
    for _ in 0..swings {
        match character_swing(rng, &character, &hand, mode, kind) {
            Swing::Hit { damage, .. } => {
                hits += 1;
                dealt += damage.min(monster.hp);
                monster.hp = monster.hp.saturating_sub(damage);
                if monster.is_dead() {
                    // Death bookkeeping at once: experience, gold, and out
                    // of the turn order. Retail drops gold with the dying
                    // animation; here it comes at the blow.
                    battle.experience += kind.experience;
                    battle.kills += 1;
                    purse.gold += rng.roll(kind.gold);
                    if let Some(entry) = battle.monsters.iter_mut().find(|(e, _)| *e == target) {
                        entry.1 = Turn::default();
                    }
                    killed = true;
                    break;
                }
            }
            Swing::NoPenetration => hits += 1,
            Swing::Missed => {}
        };
    }
    // The summary, then the casualty (retail `ReportCharacterAttackResult`).
    news.write(CombatNews::Result { attacker: character.name.clone(), hits, damage: dealt });
    if killed {
        news.write(CombatNews::Killed { name });
    }
    // Swinging tires: weight/40 + 1 = b, then b + 1 + below(b).
    let weight = carried
        .worn(crate::game::inventory::EquipSlot::Primary)
        .and_then(|i| items.get(i.item))
        .map_or(0, |d| u32::from(d.weight));
    let b = weight / 40 + 1;
    let cost = rng.below(b) + 1 + b;
    character.spend_stamina(cost);
}

#[allow(clippy::too_many_arguments)]
fn monster_acts(
    battle: &mut Battle,
    index: usize,
    rng: &mut GameRng,
    roster: &PartyRoster,
    members: &mut Query<(&mut Character, &Carried)>,
    monsters: &Query<(&mut Monster, &GlobalTransform, &MonsterAppearance)>,
    party: &PartyView,
    items: &ItemCatalog,
    species: &MonsterCatalog,
    groups: &MonsterGroups,
    news: &mut MessageWriter<CombatNews>,
) {
    let (entity, turn) = battle.monsters[index];
    let Ok((monster, at, look)) = monsters.get(entity) else { return };
    let Some(kind) = species.0.get(monster.species as usize) else { return };
    let name = kind.shown_name(groups.identified(monster.group), 1).to_owned();
    let distance = distance_to_party(party.eye, monster, at, look);
    // Every (attack, mode, member) that reaches is equally likely.
    let mut candidates = vec![];
    for (a, attack) in kind.attacks.iter().enumerate() {
        if attack.modes & 0x1ff == 0 {
            continue;
        }
        for (slot, &member) in roster.0.iter().enumerate() {
            let Ok((character, _)) = members.get(member) else { continue };
            if able(character) && party.reaches(slot, at.translation(), attack.reach, distance) {
                for bit in (0..9).filter(|b| attack.modes & (1 << b) != 0) {
                    candidates.push((a, bit, slot, member));
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
    let (a, bit, slot, member) = candidates[rng.below(candidates.len() as u32) as usize];
    let attack = &kind.attacks[a];
    let mode = attack::Mode::choose(rng, 1 << bit);
    let Ok((mut character, carried)) = members.get_mut(member) else { return };
    let mut armour = armour(&character, carried, items);
    // Defending is worth 2 (`characters.md` §4.8, component 10).
    if party.orders.of(slot) == Order::Defend {
        armour.total += 2;
        armour.by_location.iter_mut().for_each(|a| *a += 2);
    }
    let swings = rng.below(u32::from(kind.swings_per_round.max(1))) + 1;
    news.write(CombatNews::Attack {
        attacker: name.clone(),
        target: character.name.clone(),
        how: Announcement::Monster {
            weapon_type: attack.weapon_type,
            mode,
            named: attack.weapon_name != 0,
        },
        swings,
    });
    let (mut hits, mut dealt) = (0, 0);
    for _ in 0..swings {
        match monster_swing(rng, kind, attack, mode, &character, &armour) {
            Swing::Hit { damage, .. } => {
                hits += 1;
                dealt += character.damage(damage);
                if character.is_dead() {
                    break;
                }
            }
            Swing::NoPenetration => hits += 1,
            Swing::Missed => {}
        };
    }
    news.write(CombatNews::Result { attacker: name, hits, damage: dealt });
    if character.is_dead() {
        news.write(CombatNews::Killed { name: character.name.clone() });
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
    orders: &PartyOrders,
    roster: &PartyRoster,
    members: &mut Query<(&mut Character, &Carried)>,
    monsters: &Query<(&mut Monster, &GlobalTransform, &MonsterAppearance)>,
    news: &mut MessageWriter<CombatNews>,
    detect: &mut MessageWriter<DetectGroups>,
) {
    // The round's time passes now, with its meaning: conditions age and
    // nobody recovers (retail `AdvanceCombatRound`).
    let ticks = clock.advance(ROUND_MS);
    for &member in &roster.0 {
        if let Ok((mut character, _)) = members.get_mut(member) {
            recovery::elapse(&mut character, ticks, recovery::Pace::Fighting);
        }
    }
    if ticks.aging > 0 {
        detect.write(DetectGroups);
    }
    battle.step = 0;
    battle.executing = false;
    // Defenders get their breath back: 2d(max(1, max stamina / 20 - 1))
    // (retail `AdvanceCombatRound`); the Fighter trait's scaling is not
    // modelled.
    for (slot, &member) in roster.0.iter().enumerate() {
        if orders.of(slot) == Order::Defend
            && let Ok((mut character, _)) = members.get_mut(member)
            && !character.is_dead()
        {
            let sides = (character.max_stamina() / 20).saturating_sub(1).max(1);
            let gained = rng.roll(Dice { count: 2, sides: sides.min(255) as u8, base: 0 });
            character.restore_stamina(gained);
        }
    }
    battle.wait = ROUND_SECONDS;
    let active = battle.attacked_this_round || battle.approaching;
    battle.passive_rounds = if active { 0 } else { battle.passive_rounds + 1 };
    let hostiles_left =
        battle.monsters.iter().any(|&(e, _)| monsters.get(e).is_ok_and(|(m, ..)| !m.is_dead()));
    if hostiles_left && battle.passive_rounds < PASSIVE_ROUNDS {
        return;
    }
    // Experience is shared among the living, the unconscious included.
    let living =
        roster.0.iter().filter(|&&m| members.get(m).is_ok_and(|(c, _)| !c.is_dead())).count()
            as u32;
    let each = battle.experience / living.max(1);
    for &member in &roster.0 {
        if let Ok((mut character, _)) = members.get_mut(member)
            && !character.is_dead()
        {
            character.experience += each;
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
}
