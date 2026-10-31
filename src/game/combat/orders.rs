//! The player's say in a fight: members' orders, the party's pace and
//! move between rounds, and starting rounds.

use super::*;

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
    battle: Option<Single<&Battle>>,
    roster: Single<&crate::game::party::roster::PartyRoster>,
    mut orders: Query<(&mut crate::game::party::Order, &Character, &Vitals)>,
) {
    use crate::content::characters::Trait;
    for &GiveOrder { member, order } in requests.read() {
        if battle.is_some()
            && roster.contains(&member)
            && let Ok((mut current, character, vitals)) = orders.get_mut(member)
            // Only those with the trait go berserk.
            && (order != Order::Berserk || character.has_trait(Trait::Berserk))
            && (!matches!(order, Order::Breathe(_)) || breath::available(character, vitals))
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
pub(super) enum PartyMove {
    #[default]
    None,
    /// Waiting for its phase; members do not act before it.
    Pending { pace: PartyPace, phase: u32 },
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

pub(super) fn control_combat(
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

/// The move is over (retail `CompletePartyMovementTurns`, `HandlePartyMovement`):
/// each member keeps their action with the chance the unspent share gives
/// (spent counting half again when running), and pays stamina for the
/// ground covered, by load, half again when diseased.
pub(super) fn finish_party_move(battle: &mut Battle, rng: &mut GameRng, members: &mut Members) {
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
    let keep = 100 - (spent as u32).min(100);
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
        let (_, mut vitals, _, burden) = members.get_mut(work.actor).unwrap();
        if vitals.is_dead() {
            continue;
        }
        let disease = if vitals.conditions.has(Condition::Diseased) { 1.5 } else { 1.0 };
        let cost = moved * pace.stamina_per_metre() * burden.load.movement_factor() * disease;
        vitals.spend_stamina(cost.round() as u32);
    }
}
