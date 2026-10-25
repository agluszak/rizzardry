//! The party's fights (`docs/research/combat.md`), first slice: the party
//! against the hostile monsters near it, weapon attacks only, everyone
//! fighting on their own (retail's continuous mode with every member set to
//! attack). Monsters do not move, so a fight starts when the party walks up
//! to one: a stand-in for retail's sight and pursuit. Slots 0-2 are the
//! front row and 3-5 the rear until formations exist.

pub mod attack;

use avian3d::prelude::Position;
use bevy::prelude::*;

use crate::content::items::ItemCatalog;
use crate::content::species::{MonsterCatalog, Reach};
use crate::game::characters::{Character, Condition};
use crate::game::data::GameData;
use crate::game::inventory::{Carried, PartyInventory};
use crate::game::party::body::PartyBody;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::time::GameClock;
use crate::game::world::population::Monster;
use attack::{
    Swing, armour, character_initiative, character_swing, monster_swing, phase, primary_hand,
};

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<CombatNews>().add_systems(Update, (start_battle, run_battle).chain());
    }
}

/// A hostile monster this close starts a fight, in metres.
const ENGAGE_WITHIN: f32 = 3.0;
/// Hostile monsters this close join it.
const JOIN_WITHIN: f32 = 12.0;
/// Seconds between actions, so they can be followed.
const ACTION_SECONDS: f32 = 0.5;
/// Game time one round takes.
const ROUND_MS: u64 = 120_000;
/// Members in the front row; the rest cannot strike or be struck in melee.
const FRONT_ROW: usize = 3;
/// Rounds in which nobody could attack before the fight peters out.
const PASSIVE_ROUNDS: u32 = 2;

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
    /// Seconds until the next action.
    wait: f32,
}

/// What the fight tells the player.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum CombatNews {
    Began,
    Hit {
        attacker: String,
        target: String,
        damage: u32,
    },
    Missed {
        attacker: String,
        target: String,
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

fn horizontal_distance(a: Vec3, b: Vec3) -> f32 {
    Vec2::new(a.x - b.x, a.z - b.z).length()
}

/// Whether a member in `slot` reaches, or is reached by, something `reach`
/// away at `distance`.
fn reaches(slot: usize, reach: Reach, distance: f32) -> bool {
    let reach = if slot >= FRONT_ROW && reach <= Reach::Short { None } else { Some(reach) };
    reach.is_some_and(|r| distance <= r.metres())
}

/// Start a fight when a hostile monster is close enough.
pub fn start_battle(
    mut commands: Commands,
    battle: Option<Res<Battle>>,
    camp: Option<Res<crate::game::camp::Camp>>,
    party: Query<&Position, With<PartyBody>>,
    monsters: Query<(Entity, &Monster, &GlobalTransform)>,
    data: Option<Res<GameData>>,
    species: Res<Assets<MonsterCatalog>>,
    roster: Res<PartyRoster>,
    mut news: MessageWriter<CombatNews>,
) {
    if battle.is_some() || camp.is_some() {
        return;
    }
    let (Ok(party), Some(catalog)) = (party.single(), data.and_then(|d| species.get(&d.species)))
    else {
        return;
    };
    let hostile =
        |m: &Monster| !m.is_dead() && catalog.0.get(m.species as usize).is_some_and(|s| !s.npc);
    let distance = |at: &GlobalTransform| horizontal_distance(party.0, at.translation());
    if !monsters.iter().any(|(_, m, at)| hostile(m) && distance(at) <= ENGAGE_WITHIN) {
        return;
    }
    let mut joined: Vec<(i32, Entity)> = monsters
        .iter()
        .filter(|(_, m, at)| hostile(m) && distance(at) <= JOIN_WITHIN)
        .map(|(entity, m, _)| (m.location, entity))
        .collect();
    joined.sort();
    info!("combat: {} monsters", joined.len());
    commands.insert_resource(Battle {
        round: 0,
        step: 0,
        party: vec![Turn::default(); roster.0.len()],
        monsters: joined.into_iter().map(|(_, e)| (e, Turn::default())).collect(),
        experience: 0,
        kills: 0,
        passive_rounds: 0,
        attacked_this_round: false,
        wait: 0.0,
    });
    news.write(CombatNews::Began);
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
    mut monsters: Query<(&mut Monster, &GlobalTransform)>,
    party_body: Query<&Position, With<PartyBody>>,
    mut purse: ResMut<PartyInventory>,
    data: Option<Res<GameData>>,
    items: Res<Assets<ItemCatalog>>,
    species: Res<Assets<MonsterCatalog>>,
    mut news: MessageWriter<CombatNews>,
) {
    let Some(mut battle) = battle else { return };
    let (Some(items), Some(species)) = (
        data.as_ref().and_then(|d| items.get(&d.items)),
        data.as_ref().and_then(|d| species.get(&d.species)),
    ) else {
        return;
    };
    let Ok(party_at) = party_body.single().map(|p| p.0) else { return };
    battle.wait -= time.delta_secs();
    if battle.wait > 0.0 {
        return;
    }

    if battle.step == 0 {
        // A new round: the party's phases, then the monsters'.
        battle.round += 1;
        battle.attacked_this_round = false;
        for (slot, &member) in roster.0.iter().enumerate() {
            let Ok((character, carried)) = members.get(member) else { continue };
            battle.party[slot] = if able(character) {
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
                &roster,
                &mut members,
                &monsters,
                &mut news,
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
                party_at,
                items,
                species,
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
                party_at,
                items,
                species,
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
    monsters: &mut Query<(&mut Monster, &GlobalTransform)>,
    party_at: Vec3,
    items: &ItemCatalog,
    species: &MonsterCatalog,
    purse: &mut PartyInventory,
    news: &mut MessageWriter<CombatNews>,
) {
    let Ok((mut character, carried)) = members.get_mut(member) else { return };
    let hand = primary_hand(&character, carried, items);
    // The nearest monster in reach (retail prefers the least hurt first,
    // then those that can strike back; nearness is the last test).
    let target = battle
        .monsters
        .iter()
        .filter_map(|&(entity, _)| {
            let (monster, at) = monsters.get(entity).ok()?;
            let distance = horizontal_distance(party_at, at.translation());
            (!monster.is_dead() && reaches(slot, hand.reach, distance))
                .then_some((entity, distance))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(entity, _)| entity);
    let turn = &mut battle.party[slot];
    let Some(target) = target else {
        // Nobody in reach: defend for the rest of the round.
        *turn = Turn::default();
        return;
    };
    turn.attacks_left -= 1;
    turn.phase = next_phase(turn.phase, turn.attacks_left);
    battle.attacked_this_round = true;
    let Ok((mut monster, _)) = monsters.get_mut(target) else { return };
    let Some(kind) = species.0.get(monster.species as usize) else { return };
    let swings = rng.below(hand.swings) + 1;
    for _ in 0..swings {
        match character_swing(rng, &character, &hand, kind) {
            Swing::Hit { damage, .. } => {
                news.write(CombatNews::Hit {
                    attacker: character.name.clone(),
                    target: kind.name.clone(),
                    damage,
                });
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
                    news.write(CombatNews::Killed { name: kind.name.clone() });
                    break;
                }
            }
            Swing::Missed | Swing::NoPenetration => {
                news.write(CombatNews::Missed {
                    attacker: character.name.clone(),
                    target: kind.name.clone(),
                });
            }
        };
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
    monsters: &Query<(&mut Monster, &GlobalTransform)>,
    party_at: Vec3,
    items: &ItemCatalog,
    species: &MonsterCatalog,
    news: &mut MessageWriter<CombatNews>,
) {
    let (entity, turn) = battle.monsters[index];
    let Ok((monster, at)) = monsters.get(entity) else { return };
    let Some(kind) = species.0.get(monster.species as usize) else { return };
    let distance = horizontal_distance(party_at, at.translation());
    // Every (attack, mode, member) that reaches is equally likely.
    let mut candidates = vec![];
    for (a, attack) in kind.attacks.iter().enumerate() {
        if attack.modes & 0x1ff == 0 {
            continue;
        }
        for (slot, &member) in roster.0.iter().enumerate() {
            let Ok((character, _)) = members.get(member) else { continue };
            if able(character) && reaches(slot, attack.reach, distance) {
                for bit in (0..9).filter(|b| attack.modes & (1 << b) != 0) {
                    candidates.push((a, bit, member));
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
    let (a, bit, member) = candidates[rng.below(candidates.len() as u32) as usize];
    let attack = &kind.attacks[a];
    let mode = attack::Mode::choose(rng, 1 << bit);
    let Ok((mut character, carried)) = members.get_mut(member) else { return };
    let armour = armour(&character, carried, items);
    let swings = rng.below(u32::from(kind.swings_per_round.max(1))) + 1;
    for _ in 0..swings {
        match monster_swing(rng, kind, attack, mode, &character, &armour) {
            Swing::Hit { damage, .. } => {
                let damage = character.damage(damage);
                news.write(CombatNews::Hit {
                    attacker: kind.name.clone(),
                    target: character.name.clone(),
                    damage,
                });
                if character.is_dead() {
                    news.write(CombatNews::Killed { name: character.name.clone() });
                    break;
                }
            }
            Swing::Missed | Swing::NoPenetration => {
                news.write(CombatNews::Missed {
                    attacker: kind.name.clone(),
                    target: character.name.clone(),
                });
            }
        };
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
    roster: &PartyRoster,
    members: &mut Query<(&mut Character, &Carried)>,
    monsters: &Query<(&mut Monster, &GlobalTransform)>,
    news: &mut MessageWriter<CombatNews>,
) {
    clock.ms += ROUND_MS;
    battle.step = 0;
    battle.passive_rounds = if battle.attacked_this_round { 0 } else { battle.passive_rounds + 1 };
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
    fn remaining_attacks_spread_over_the_round() {
        assert_eq!(next_phase(10, 0), 0);
        assert_eq!(next_phase(10, 1), 55);
        assert_eq!(next_phase(55, 1), 77);
        assert_eq!(next_phase(100, 1), 100, "again at the same step");
    }

    #[test]
    fn the_rear_row_neither_strikes_nor_is_struck_in_melee() {
        assert!(reaches(0, Reach::Touch, 1.5));
        assert!(!reaches(0, Reach::Touch, 2.5));
        assert!(!reaches(3, Reach::Short, 1.0));
        assert!(reaches(4, Reach::Long, 20.0));
    }
}
