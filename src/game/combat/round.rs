//! Acceptance fixes actor identity, weapons, initiative and attack counts.
//! Execution spends that work while retaining history until round recovery.

use bevy::prelude::*;

use super::attack::phase;
use super::{Battle, PartyMove, PartyPace, RoundPhase, attack, blows};
use crate::content::items::ItemCatalog;
use crate::content::species::MonsterCatalog;
use crate::game::characters::Condition;
use crate::game::inventory::{EquipSlot, StackId};
use crate::game::magic::casting::MagicChoice;
use crate::game::party::roster::PartyRoster;
use crate::game::party::{Members, Order};
use crate::game::rng::GameRng;
use crate::game::world::damage::MonsterQuery;

/// Accepted work and history for one monster. Spending its attacks never
/// discards the facts needed by later attacks or end-of-round recovery.
#[derive(Clone, Copy, Debug)]
pub(super) struct MonsterTurn {
    pub(super) actor: Entity,
    pub(super) phase: u8,
    pub(super) attacks_left: u32,
    pub(super) moved: bool,
    pub(super) special: bool,
    pub(super) berserk: bool,
    pub(super) waited: bool,
}
impl MonsterTurn {
    fn new(actor: Entity, phase: u8, attacks_left: u32) -> Self {
        Self {
            actor,
            phase,
            attacks_left,
            moved: false,
            special: false,
            berserk: false,
            waited: false,
        }
    }

    pub(super) fn end(&mut self) {
        self.phase = 0;
        self.attacks_left = 0;
    }
}

/// Effective round work identifies the actor and weapon, never a roster slot.
/// The action starts from the accepted order; execution may replace a refused
/// spell with Defend. Weapon identity, initiative and attack count stay locked.
/// Execution revalidates membership, ability, weapon presence and current reach;
/// damage profiles and fatigue use the actor's live stats and accepted weapon.
#[derive(Clone, Copy, Debug)]
pub(super) enum RoundAction {
    /// The weapons in both hands; a Berserk order is an attack too.
    Attack {
        weapons: [Option<StackId>; 2],
        berserk: bool,
    },
    Defend,
    /// A spell or item use, revalidated and paid when its initiative comes.
    Magic(MagicChoice),
    Breathe(Entity),
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PartyTurn {
    pub(super) actor: Entity,
    pub(super) action: RoundAction,
    /// When the member acts next, 1 to 100; 0 when done for the round.
    pub(super) phase: u8,
    /// What an attack has left; other actions act once.
    pub(super) hands: HandTurns,
}

impl PartyTurn {
    /// Done for the round: nothing left to act with.
    pub(super) fn end(&mut self) {
        self.phase = 0;
        self.hands.left = [0, 0];
    }
}

/// A member's attacks left in each hand this round and the hand due next
/// (retail `hand_attack_values`, `current_hand`): the one authority on
/// what an attack has left.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct HandTurns {
    pub(super) left: [u32; 2],
    pub(super) current: usize,
    /// A Samurai's Lightning Strike, readied for the main hand's next
    /// attack (retail `extra_swings`).
    pub(super) lightning: bool,
}

/// One round: its committed work, fixed when the round is accepted (who
/// acts, with what, when and how often), and what has happened in it so
/// far. Execution revalidates each actor (still a member or combatant, able,
/// the same weapon) but never redirects work to whoever holds its slot now.
/// Each round starts a new one.
#[derive(Debug, Default)]
pub(super) struct Round {
    pub(super) party: Vec<PartyTurn>,
    pub(super) monsters: Vec<MonsterTurn>,
    /// Someone has attacked.
    pub(super) attacked: bool,
    /// Guards in activation order, which also fixes interception RNG order.
    pub(super) guards: Vec<Guard>,
    /// How the party moves this round.
    pub(super) pace: Option<PartyPace>,
    /// How often each member has tried to spot an attacker this round.
    pub(super) spotted: bevy::ecs::entity::EntityHashMap<u32>,
}

/// A monster guarding a fellow this round, and how many blows it has
/// taken in its place (retail `ACTION_PROTECT`, `interception_count`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Guard {
    pub(super) guard: Entity,
    pub(super) ward: Entity,
    pub(super) interceptions: u32,
}

/// Accept a round (retail `BeginCombatExecution`): lock each member's order
/// and weapon, and everyone's initiative and attacks, the party first, then
/// the combatants in order. Pacing delays only its execution.
pub(super) fn accept_round(
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
    let surprised = battle.opening.take().is_some();
    let pace = if surprised { None } else { battle.party_pace };
    let mut round = Round { pace, ..default() };
    // Samurai ready their Lightning Strike with a sword before anyone's
    // initiative is rolled (retail `BeginCombatExecution`).
    let prepared: Vec<_> = roster
        .iter()
        .filter(|_| !surprised)
        .map(|member| {
            let (character, vitals, carried, burden) = members.get(member).unwrap();
            let stats = character.stats(&vitals.enchantments);
            let hands = attack::hands(&stats, carried, items, burden.load);
            let lightning = orders.get(member).copied().unwrap_or_default() == Order::Attack
                && character.has_trait(Trait::LightningStrike)
                && hands.hands[0].weapon_skill == Skill::Sword
                && rng.below(100) < u32::from(character.scale_by_profession(12.0) as u8);
            (member, hands, lightning)
        })
        .collect();
    for (member, hands, lightning) in prepared {
        let (character, vitals, carried, burden) = members.get(member).unwrap();
        let stats = character.stats(&vitals.enchantments);
        let weapons =
            [EquipSlot::Primary, EquipSlot::Secondary].map(|slot| carried.worn(slot).map(|i| i.id));
        let order = if vitals.conditions.has(Condition::Turncoat) {
            Order::Attack
        } else {
            orders.get(member).copied().unwrap_or_default()
        };
        let action = match order {
            Order::Attack => RoundAction::Attack { weapons, berserk: false },
            // One who can no longer berserk attacks as usual.
            Order::Berserk => {
                RoundAction::Attack { weapons, berserk: blows::can_berserk(character, &hands) }
            }
            Order::Defend => RoundAction::Defend,
            Order::Magic(choice) => RoundAction::Magic(choice),
            Order::Breathe(target) => RoundAction::Breathe(target),
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
                    let initiative =
                        attack::character_initiative(&stats, &hands.hands[hand], burden.load);
                    phase(rng, initiative)
                }
                None => 0,
            },
            RoundAction::Magic(_) | RoundAction::Breathe(_) if vitals.can_fight() => {
                phase(rng, stats.initiative(burden.load))
            }
            _ => 0,
        };
        round.party.push(PartyTurn { actor: member, action, phase, hands: turns });
    }
    for &entity in combatants {
        let (monster, ..) = monsters.get(entity).unwrap();
        let kind = &species[monster.species];
        let (phase, attacks) = if monster.is_dead() {
            (0, 0)
        } else {
            (phase(rng, i32::from(kind.initiative)), u32::from(kind.attacks_per_round))
        };
        round.monsters.push(MonsterTurn::new(entity, phase, attacks));
    }
    // The party's own move takes the phase its slowest living member
    // allows, by the initiative its actions use, on a step of ten (retail
    // `InitializePartyMovementPhase`).
    battle.party_move = match pace {
        Some(pace) => {
            let latest = roster
                .iter()
                .map(|m| members.get(m).unwrap())
                .filter(|(_, vitals, ..)| !vitals.is_dead())
                .map(|(c, vitals, _, burden)| {
                    phase(rng, c.stats(&vitals.enchantments).initiative(burden.load))
                })
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
