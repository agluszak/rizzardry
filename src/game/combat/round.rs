//! Acceptance fixes actor identity, weapons, initiative and attack counts.
//! Execution spends that work while retaining history until round recovery.

use bevy::prelude::*;

use super::attack::phase;
use super::{Battle, PartyMove, RoundPhase};
use super::attack::Grip;
use enum_map::EnumMap;
use crate::content::items::ItemCatalog;
use crate::content::species::MonsterCatalog;
use crate::game::characters::Condition;
use crate::game::inventory::{EquipSlot, StackId, WeaponSlot};
use crate::game::magic::casting::MagicChoice;
use crate::game::party::roster::PartyRoster;
use crate::game::party::{Members, Order};
use crate::game::rng::GameRng;
use crate::game::world::damage::MonsterQuery;

/// A monster's accepted work and history this round. Spending its attacks
/// never discards the facts needed by later attacks or end-of-round
/// recovery.
#[derive(Component, Clone, Copy, Debug)]
pub(super) struct MonsterTurn {
    pub(super) phase: u32,
    pub(super) attacks_left: u32,
    pub(super) moved: bool,
    pub(super) special: bool,
    pub(super) berserk: bool,
    pub(super) waited: bool,
}
impl MonsterTurn {
    fn new(phase: u32, attacks_left: u32) -> Self {
        Self { phase, attacks_left, moved: false, special: false, berserk: false, waited: false }
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

/// A member's accepted work this round.
#[derive(Component, Clone, Copy, Debug)]
pub struct PartyTurn {
    pub(super) action: RoundAction,
    /// When the member acts next, 1 to 100; 0 when done for the round.
    pub(super) phase: u32,
    /// The attacks each weapon slot has left this round (retail
    /// `hand_attack_values`); other actions act once.
    pub(super) attacks: EnumMap<WeaponSlot, u32>,
    /// The slot due to attack next (retail `current_hand`).
    pub(super) due: WeaponSlot,
    /// A Samurai's Lightning Strike, readied for the primary slot's next
    /// attack (retail `extra_swings`).
    pub(super) lightning: bool,
}

impl PartyTurn {
    /// The effective action stands until the next round: spending or losing
    /// a turn does not end defending; a refused spell starts it.
    pub fn defends(&self) -> bool {
        matches!(self.action, RoundAction::Defend)
    }

    /// Done for the round: nothing left to act with.
    pub(super) fn end(&mut self) {
        self.phase = 0;
        self.attacks = EnumMap::default();
    }
}

/// The actor whose initiative has come this step, to act now.
#[derive(Component, Debug)]
pub(super) struct Acting;

/// A monster guarding a fellow this round (retail `ACTION_PROTECT`).
#[derive(Component, Debug)]
#[relationship(relationship_target = Guards)]
#[require(Interceptions)]
pub(super) struct Guarding(pub(super) Entity);

/// How many blows a guard has taken in its ward's place this round (retail
/// `interception_count`).
#[derive(Component, Debug, Default)]
pub(super) struct Interceptions(pub(super) u32);

/// A monster's guards this round, in the order they took it up, which also
/// fixes the order they try to intercept.
#[derive(Component, Debug)]
#[relationship_target(relationship = Guarding)]
pub(super) struct Guards(Vec<Entity>);

impl Guards {
    pub(super) fn entities(&self) -> &[Entity] {
        &self.0
    }
}

/// How often a member has tried to spot an attacker this round.
#[derive(Component, Debug, Default)]
pub(super) struct Spotted(pub(super) u32);

/// A monster running for its life (retail `pCombat->advancing`), for the
/// rest of the fight.
#[derive(Component, Debug)]
pub(super) struct Fleeing;

/// Rounds a monster still waits before its next special attack (retail
/// `pCombat->special_cooldown`).
#[derive(Component, Debug)]
pub(super) struct SpecialCooldown(pub(super) u8);

/// The fight is over: what its rounds left on the members and monsters goes
/// with it.
pub(super) fn clear_fight(
    _: On<Remove<Battle>>,
    mut commands: Commands,
    actors: Query<
        Entity,
        Or<(
            With<PartyTurn>,
            With<MonsterTurn>,
            With<Acting>,
            With<Guarding>,
            With<Interceptions>,
            With<Spotted>,
            With<Fleeing>,
            With<SpecialCooldown>,
        )>,
    >,
) {
    for actor in &actors {
        commands.entity(actor).try_remove::<(
            PartyTurn,
            MonsterTurn,
            Acting,
            Guarding,
            Interceptions,
            Spotted,
            Fleeing,
            SpecialCooldown,
        )>();
    }
}

/// Accept a round (retail `BeginCombatExecution`): lock each member's order
/// and weapon, and everyone's initiative and attacks, the party first, then
/// the combatants in order. Pacing delays only its execution.
pub(super) fn accept_round(
    commands: &mut Commands,
    battle: &mut Battle,
    combatants: &[Entity],
    roster: &PartyRoster,
    members: &Members,
    orders: &Query<&Order>,
    monsters: &MonsterQuery,
    items: &ItemCatalog,
    species: &MonsterCatalog,
    rng: &mut GameRng,
) {
    use crate::content::characters::{Skill, Trait};
    battle.round += 1;
    let surprised = battle.opening.take().is_some();
    let pace = if surprised { None } else { battle.party_pace };
    battle.pace = pace;
    battle.attacked = false;
    // Samurai ready their Lightning Strike with a sword before anyone's
    // initiative is rolled (retail `BeginCombatExecution`).
    let prepared: Vec<_> = roster
        .iter()
        .filter(|_| !surprised)
        .map(|member| {
            let (character, _, carried, _) = members.get(member).unwrap();
            let grip = Grip::of(carried, items);
            let lightning = *orders.get(member).unwrap() == Order::Attack
                && character.has_trait(Trait::LightningStrike)
                && grip.weapon_skill(WeaponSlot::Primary) == Skill::Sword
                && rng.below(100) < u32::from(character.scale_by_profession(12.0) as u8);
            (member, lightning)
        })
        .collect();
    for (member, lightning) in prepared {
        let (character, vitals, carried, burden) = members.get(member).unwrap();
        let stats = character.stats(&vitals.enchantments);
        let grip = Grip::of(carried, items);
        let weapons =
            [EquipSlot::Primary, EquipSlot::Secondary].map(|slot| carried.worn(slot).map(|i| i.id));
        let order = if vitals.conditions.has(Condition::Turncoat) {
            Order::Attack
        } else {
            *orders.get(member).unwrap()
        };
        let action = match order {
            Order::Attack => RoundAction::Attack { weapons, berserk: false },
            // One who can no longer berserk attacks as usual.
            Order::Berserk => RoundAction::Attack {
                weapons,
                berserk: grip.can_berserk(&stats),
            },
            Order::Defend => RoundAction::Defend,
            Order::Magic(choice) => RoundAction::Magic(choice),
            Order::Breathe(target) => RoundAction::Breathe(target),
        };
        let mut turn = PartyTurn {
            action,
            phase: 0,
            attacks: EnumMap::default(),
            due: WeaponSlot::Primary,
            lightning,
        };
        let load = burden.load;
        turn.phase = match action {
            // The slot with the most attacks opens, its weapon's initiative
            // counting (retail `AssignCombatPhases`).
            RoundAction::Attack { .. } if vitals.can_fight() => match grip.opening(&stats, load) {
                Some(slot) => {
                    turn.due = slot;
                    turn.attacks = EnumMap::from_fn(|slot| grip.attacks(&stats, load, slot));
                    phase(rng, stats.initiative(load) + grip.initiative(&stats, slot))
                }
                None => 0,
            },
            RoundAction::Magic(_) | RoundAction::Breathe(_) if vitals.can_fight() => {
                phase(rng, stats.initiative(load))
            }
            _ => 0,
        };
        commands.entity(member).insert((turn, Spotted(0)));
    }
    for &entity in combatants {
        let (monster, .., vitals) = monsters.get(entity).unwrap();
        let kind = &species[monster.species];
        let (phase, attacks) = if vitals.is_dead() {
            (0, 0)
        } else {
            (phase(rng, i32::from(kind.initiative)), u32::from(kind.attacks_per_round))
        };
        commands
            .entity(entity)
            .insert(MonsterTurn::new(phase, attacks))
            .remove::<(Guarding, Interceptions)>();
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
    battle.phase = RoundPhase::Accepted;
}
