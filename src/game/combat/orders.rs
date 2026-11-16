//! The player's say in a fight: members' orders, the party's pace and
//! move between rounds, and starting rounds.

use super::*;
use crate::game::characters::Condition;

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
pub(super) fn give_orders(
    mut requests: MessageReader<GiveOrder>,
    roster: Single<&crate::game::party::roster::PartyRoster>,
    mut orders: Query<
        (&mut crate::game::party::Order, &crate::game::actors::Traits, Has<breath::CanBreathe>),
        With<crate::game::actors::CanFight>,
    >,
) {
    use crate::content::characters::Trait;
    use crate::game::party::Order;
    for &GiveOrder { member, order } in requests.read() {
        if roster.contains(&member)
            && let Ok((mut current, character, can_breathe)) = orders.get_mut(member)
            // Only those with the trait go berserk.
            && (order != Order::Berserk || character.has(Trait::Berserk))
            && (!matches!(order, Order::Breathe(_)) || can_breathe)
        {
            current.set_if_neq(order);
        }
    }
}

/// Choose, between rounds, whether the party moves in the next ones.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct CyclePartyMove;

/// The party's move this round, on the fight.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub enum PartyMove {
    #[default]
    None,
    /// Waiting for its phase; members do not act before it.
    Pending { pace: PartyPace, phase: u32 },
    /// Under way: metres allowed and covered (retail `GetPartyMovementSpeed`:
    /// 10000 units walking, 15000 running).
    Moving { pace: PartyPace, budget: f32, moved: f32, last: Vec3 },
}

impl PartyMove {
    /// Whether the party may walk: in a fight, only during its own move.
    pub fn under_way(&self) -> bool {
        matches!(self, PartyMove::Moving { .. })
    }

    /// Whether the party's move is a run.
    pub fn running(&self) -> bool {
        matches!(self, PartyMove::Moving { pace: PartyPace::Run, .. })
    }
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

pub(super) fn control_combat(
    mut requests: MessageReader<CombatControl>,
    round: Option<Single<&Round, With<Battle>>>,
    mut settings: ResMut<CombatSettings>,
    mut rounds: MessageWriter<StartRound>,
) {
    for request in requests.read() {
        match request {
            CombatControl::ToggleContinuous => settings.continuous = !settings.continuous,
            CombatControl::RoundButton => {
                if let Some(round) = &round {
                    if round.executing() {
                        settings.continuous = !settings.continuous;
                    } else {
                        rounds.write(StartRound);
                    }
                }
            }
        }
    }
}

/// Count the ground the party covers while it moves in a fight, and end the
/// move when it is spent.
pub fn track_party_move(
    mut commands: Commands,
    mut party_move: Single<&mut PartyMove, With<Battle>>,
    position: Single<&avian3d::prelude::Position, With<crate::game::party::body::PartyBody>>,
    mut news: MessageWriter<CombatNews>,
) {
    let PartyMove::Moving { budget, moved, last, .. } = &mut **party_move else {
        return;
    };
    *moved += last.with_y(0.0).distance(position.0.with_y(0.0));
    *last = position.0;
    if *moved >= *budget {
        commands.trigger(PartyMoveEnds);
        news.write(CombatNews::PartyStops);
    }
}

impl PartyPace {
    /// The metres a move covers in a fight (retail `GetPartyMovementSpeed`:
    /// 10000 units walking, 15000 running).
    pub(super) fn combat_budget(self) -> f32 {
        match self {
            PartyPace::Walk => 20.0,
            PartyPace::Run => 30.0,
        }
    }

    /// Stamina a metre costs an unencumbered member in a fight: retail
    /// charges `distance × load × 2 (combat)` units at one stamina per 2500
    /// (`MGSPartyMovement.cpp:302-358`).
    pub(super) fn stamina_per_metre(self) -> f32 {
        match self {
            PartyPace::Walk => 0.2,
            PartyPace::Run => 0.4,
        }
    }
}

/// The party's move in a fight is over.
#[derive(Event, Clone, Copy, Debug)]
pub struct PartyMoveEnds;

/// The move is over (retail `CompletePartyMovementTurns`, `HandlePartyMovement`):
/// each member keeps their action with the chance the unspent share gives
/// (spent counting half again when running), and pays stamina for the
/// ground covered, by load, half again when diseased.
#[allow(clippy::type_complexity)]
pub(super) fn end_party_move(
    _: On<PartyMoveEnds>,
    mut commands: Commands,
    fight: Single<(&Round, &mut PartyMove), With<Battle>>,
    roster: Single<&PartyRoster>,
    mut members: Query<(
        Option<&mut super::turn::NextMoment>,
        (&mut Vitals, &crate::game::characters::Conditions, &crate::game::items::Burden),
    )>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let (round, mut party_move) = fight.into_inner();
    let PartyMove::Moving { pace, budget, moved, .. } = *party_move else { return Ok(()) };
    *party_move = PartyMove::None;
    let mut spent = (moved / budget.max(0.01) * 100.0).min(100.0);
    if pace == PartyPace::Run {
        spent *= 1.5;
    }
    let keep = 100 - (spent as u32).min(100);
    for member in roster.iter() {
        let (next, (mut vitals, conditions, burden)) = members.get_mut(member)?;
        // One with a moment still to come this round keeps it or loses it.
        if let Some(mut next) = next {
            if rng.below(100) < keep {
                next.0 = next.0.max(round.moment);
            } else {
                commands.trigger(super::turn::Done { actor: member });
            }
        }
        if vitals.is_dead() {
            continue;
        }
        let disease = if conditions.has(Condition::Diseased) { 1.5 } else { 1.0 };
        let cost = moved * pace.stamina_per_metre() * burden.load.movement_factor() * disease;
        vitals.tire(cost.round() as u32);
    }
    Ok(())
}
