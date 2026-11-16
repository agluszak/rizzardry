//! Turns in a round. Accepting a round gives every actor its [`Turn`]: its
//! initiative, and the attacks it has with each weapon slot; a member's
//! order becomes what they are [`Ordered`] to do. When an actor's moment
//! comes ([`Moment`]) it takes its turn ([`take_turn`]): revalidated, one
//! under orders follows them, one without decides for itself
//! ([`super::monster`]), and its conditions may take the action off it. What it
//! then does is an event at it with one observer that does all of it: a
//! weapon attack [`super::swings::Strike`], a cast
//! [`crate::game::magic::casting::Cast`], and here [`Breathe`], [`Guard`],
//! [`Retreat`] and [`Hold`].

use crate::game::magic::workings::Response;
use bevy::prelude::*;
use enum_map::EnumMap;

use super::attack::phase;
use super::effort::{Effort, Exerted};
use super::monster::ailments::Plans;
use super::monster::conduct::Conduct;
use super::monster::sight::Sight;
use super::monster::{self, Interrupt, ailments, close};
use super::swings::Strike;
use super::{ACTION_SECONDS, Battle, CombatNews, Combatant, PartyMove, Pause, Round, summon};
use crate::content::characters::Trait;
use crate::content::species::SpecialAttackKind;
use crate::content::spells::SpellId;
use crate::game::actors::Initiative;
use crate::game::characters::{Condition, Vitals};
use crate::game::data::Catalogs;
use crate::game::hits::definitions::Rows;
use crate::game::hits::effects;
use crate::game::hits::{Deed, Landed};
use crate::game::inventory::WeaponSlot;
use crate::game::items::Items;
use crate::game::magic::Strength;
use crate::game::magic::casting::{Cast, MagicChoice, Refused, reaches as spell_reaches};
use crate::game::magic::may::Refusal;
use crate::game::magic::reach::in_cone;
use crate::game::party::Order;
use crate::game::party::formation::Formation;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::navigation::Walker;
use crate::game::world::obstacles::WorldObstacles;

/// An actor's accepted work and history this round: its next moment,
/// the attacks each weapon slot has left (retail `hand_attack_values`; a
/// monster's all in its primary slot), the slot due to attack next (retail
/// `current_hand`), the weapons the attacks were accepted with, and what
/// the round has done to it.
#[derive(Component, Clone, Debug)]
pub struct Turn {
    /// The moment it acts next, 1 to 100; nothing when done for the round.
    pub when: Option<u32>,
    pub attacks: EnumMap<WeaponSlot, u32>,
    pub due: WeaponSlot,
    pub weapons: EnumMap<WeaponSlot, Option<Entity>>,
    /// A Samurai's Lightning Strike, readied for the primary slot's next
    /// attack (retail `extra_swings`), by their Samurai level.
    pub lightning: Option<u8>,
    /// It has made its move this round.
    pub moved: bool,
    /// It made its special attack this round.
    pub special: bool,
    /// Insanity turned it on its friends this round.
    pub berserk: bool,
    /// It holds back on guard this round: a monster that waited, a member
    /// ordered to defend. It rests at the round's end and is better guarded.
    pub waited: bool,
}

impl Turn {
    /// Done for the round: nothing left to act with.
    pub fn end(&mut self) {
        self.when = None;
        self.attacks = EnumMap::default();
    }

    /// Nothing is left to strike with in a slot (a launcher out of shots):
    /// it has no attacks left this round.
    pub fn run_out(&mut self, slot: WeaponSlot) {
        self.attacks[slot] = 0;
    }
}

/// What a member was ordered to do this round, accepted as the round
/// began: later edits to their order wait for the next round.
#[derive(Component, Clone, Copy, Debug)]
pub struct Ordered(pub Action);

/// A member's accepted action.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    /// A Berserk order is an attack too.
    Attack {
        berserk: bool,
    },
    Defend,
    /// A spell or item use, revalidated and paid when its initiative comes.
    Magic(MagicChoice),
    Breathe(Entity),
}

impl Ordered {
    /// The accepted action stands until the next round: spending or losing
    /// a turn does not end defending; a refused spell starts it.
    pub fn defends(&self) -> bool {
        self.0 == Action::Defend
    }
}

/// An actor's moment has come: it acts now.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Moment {
    #[event_target]
    pub actor: Entity,
}

/// What an actor decided to do with its moment, before its conditions have
/// their say ([`take_turn`]); each is then done by the event for it.
/// Accepted player orders remain separate ([`Ordered`]) because later
/// edits cannot change them.
#[derive(Clone, Copy, Debug)]
pub enum Decision {
    Fight(Fights),
    Retreat(Retreats),
    Protect(Entity),
    Special(Breathes),
    Magic(MagicChoice),
}

/// Strike someone, or close in to; `waits` when it may hold back instead;
/// `berserk` for a Berserk order's fury.
#[derive(Clone, Copy, Debug)]
pub struct Fights {
    pub waits: bool,
    pub berserk: bool,
}

/// Run, withdraw or hold back; `quiet` for a flight fear forced, which
/// retail does not announce.
#[derive(Clone, Copy, Debug)]
pub struct Retreats {
    pub conduct: Conduct,
    pub quiet: bool,
}

/// Make a special attack, or a breath when it is of no kind: a deed at this
/// strength toward `toward`, catching whoever stands in its cone, or a
/// summoning of a species; it costs stamina, and a monster's cools down.
#[derive(Clone, Copy, Debug)]
pub struct Breathes {
    pub special: Option<SpecialAttackKind>,
    pub summons: Option<u32>,
    pub strength: Strength,
    pub toward: Vec3,
    pub cost: u32,
    pub cooldown: u8,
}

/// An actor runs, withdraws or holds back ([`retreat`]).
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Retreat {
    #[event_target]
    pub actor: Entity,
    pub retreats: Retreats,
}

/// An actor holds back and rests, taking no pause ([`hold`]).
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Hold {
    #[event_target]
    pub actor: Entity,
}

/// An actor makes its special attack or breath ([`breathe`]).
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Breathe {
    #[event_target]
    pub actor: Entity,
    pub breathes: Breathes,
}

/// An actor guards a fellow this round ([`guard`]).
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Guard {
    #[event_target]
    pub actor: Entity,
    pub ward: Entity,
}

/// A monster guarding a fellow this round (retail `ACTION_PROTECT`).
#[derive(Component, Debug)]
#[relationship(relationship_target = Guards)]
pub struct Guarding(pub Entity);

/// How many blows a guard has taken in its ward's place this round (retail
/// `interception_count`), and how many it may.
#[derive(Component, Debug, Default)]
pub struct Interceptions {
    pub taken: u32,
    pub allowed: u32,
}

impl Interceptions {
    /// A blow taken in its ward's place.
    pub fn take(&mut self) {
        self.taken += 1;
    }
}

/// A monster's guards this round, in the order they took it up, which also
/// fixes the order they try to intercept.
#[derive(Component, Debug)]
#[relationship_target(relationship = Guarding)]
pub struct Guards(Vec<Entity>);

impl Guards {
    pub fn entities(&self) -> &[Entity] {
        &self.0
    }
}

/// How often a member has tried to spot an attacker this round.
#[derive(Component, Debug, Default)]
pub struct Spotted(pub u32);

/// A monster running for its life (retail `pCombat->advancing`), for the
/// rest of the fight.
#[derive(Component, Debug)]
pub struct Fleeing;

/// Rounds a monster still waits before its next special attack (retail
/// `pCombat->special_cooldown`).
#[derive(Component, Debug)]
pub struct SpecialCooldown(pub u8);

/// The fight is over: what its rounds left on the members and monsters goes
/// with it.
#[allow(clippy::type_complexity)]
pub fn clear_fight(left: On<Remove<Combatant>>, mut commands: Commands) {
    commands.entity(left.entity).try_remove::<(
        Turn,
        Ordered,
        Guarding,
        Guards,
        Interceptions,
        Spotted,
        Fleeing,
        SpecialCooldown,
        super::approach::CombatMove,
    )>();
}

/// The fight accepts a round (retail `BeginCombatExecution`): it counts it,
/// says it began, settles the party's pace (none when surprised), has each
/// combatant take up its turn in order, and places the party's move.
/// Pacing delays only its execution.
#[derive(Event, Clone, Copy, Debug)]
pub struct Accepts;

/// A combatant takes up its turn for the round.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct TakesUp {
    #[event_target]
    actor: Entity,
    surprised: bool,
}

/// The party's move is placed for the round.
#[derive(Event, Clone, Copy, Debug)]
struct PlacesPartyMove;

pub(super) fn accept(
    _: On<Accepts>,
    fight: Single<(&mut Battle, &mut Round, Option<&super::Combatants>)>,
    mut commands: Commands,
) {
    let (mut battle, mut round, combatants) = fight.into_inner();
    battle.round += 1;
    commands.trigger(super::RoundBegan);
    let surprised = battle.opening.take().is_some();
    round.pace = if surprised { None } else { battle.party_pace };
    round.attacked = false;
    round.playing = true;
    round.moment = 0;
    for &actor in combatants.map_or(&[][..], super::Combatants::entities) {
        commands.trigger(TakesUp { actor, surprised });
    }
    commands.trigger(PlacesPartyMove);
}

/// A combatant takes up its turn: a member's order is locked, and what it
/// wields with it. A member turned on the party attacks whatever its order;
/// a Berserk order needs the trait, an arm, and a main hand for close
/// quarters (retail `CanCharacterBerserk`). A Samurai attacking with a
/// sword in the main hand readies Lightning Strike, by their Samurai level,
/// at `12` scaled by it in a hundred, unless the party was surprised. One
/// that attacks opens with the hand of most attacks, its initiative
/// counting (retail `AssignCombatPhases`); one that casts or breathes acts
/// by its own initiative; one that defends takes no moment.
#[allow(clippy::type_complexity)]
fn take_up(
    takes: On<TakesUp>,
    mut commands: Commands,
    actors: Query<(
        (&crate::game::characters::Conditions, Has<crate::game::actors::CanFight>),
        (&Initiative, &crate::game::actors::arm::Tempo, &crate::game::actors::arm::Wielding),
        Option<(&Order, &crate::game::actors::Traits, &crate::game::characters::Calling)>,
        &crate::game::actors::arm::Arms,
    )>,
    arms: Query<(
        &crate::game::actors::arm::Hand,
        &crate::game::actors::arm::Reaches,
        &crate::game::actors::arm::AttacksARound,
        Has<crate::game::actors::arm::Sword>,
    )>,
    mut rng: ResMut<GameRng>,
) {
    let TakesUp { actor, surprised } = *takes;
    let Ok(((conditions, can_fight), (own, tempo, wielding), ordered, held)) = actors.get(actor)
    else {
        return;
    };
    let mains: Vec<_> = held
        .entities()
        .iter()
        .filter_map(|&arm| arms.get(arm).ok())
        .filter(|(hand, _, attacks, _)| hand.0 == WeaponSlot::Primary && attacks.0 > 0)
        .collect();
    let striking = tempo.attacks.values().any(|&attacks| attacks > 0);
    let action = ordered.map(|(order, traits, _)| match *order {
        _ if conditions.has(Condition::Turncoat) => Action::Attack { berserk: false },
        Order::Attack => Action::Attack { berserk: false },
        Order::Berserk => Action::Attack {
            berserk: traits.has(Trait::Berserk)
                && striking
                && mains
                    .iter()
                    .all(|(_, reach, ..)| reach.0 <= crate::content::species::Reach::Short),
        },
        Order::Defend => Action::Defend,
        Order::Magic(choice) => Action::Magic(choice),
        Order::Breathe(target) => Action::Breathe(target),
    });
    let lightning = ordered.and_then(|(order, traits, calling)| {
        (!surprised
            && *order == Order::Attack
            && traits.has(Trait::LightningStrike)
            && mains.iter().any(|(.., sword)| *sword)
            && rng.below(100) < u32::from(calling.scale(12.0) as u8))
        .then(|| calling.mastery())
    });
    let mut turn = Turn {
        when: None,
        attacks: EnumMap::default(),
        due: WeaponSlot::Primary,
        weapons: wielding.0,
        lightning,
        moved: false,
        special: false,
        berserk: false,
        waited: action == Some(Action::Defend),
    };
    if can_fight {
        match action {
            None | Some(Action::Attack { .. }) => {
                if let Some(slot) = tempo.opening() {
                    turn.due = slot;
                    turn.attacks = tempo.attacks;
                    turn.when = Some(phase(&mut rng, own.0 + tempo.initiative[slot]));
                }
            }
            Some(Action::Magic(_) | Action::Breathe(_)) => {
                turn.when = Some(phase(&mut rng, own.0));
            }
            Some(Action::Defend) => {}
        }
    }
    let mut entity = commands.entity(actor);
    entity.insert(turn).remove::<(Guarding, Interceptions)>();
    match action {
        Some(action) => {
            entity.insert((Ordered(action), Spotted(0)));
        }
        None => {
            entity.remove::<Ordered>();
        }
    }
}

/// The party's own move takes the phase its slowest living member allows,
/// by the initiative its actions use, on a step of ten (retail
/// `InitializePartyMovementPhase`).
fn place_party_move(
    _: On<PlacesPartyMove>,
    fight: Single<(&Round, &mut PartyMove)>,
    roster: Single<&PartyRoster>,
    members: Query<(&Vitals, &Initiative)>,
    mut rng: ResMut<GameRng>,
) {
    let (round, mut party_move) = fight.into_inner();
    *party_move = match round.pace {
        Some(pace) => {
            let latest = roster
                .iter()
                .filter_map(|member| members.get(member).ok())
                .filter(|(vitals, _)| !vitals.is_dead())
                .map(|(_, own)| phase(&mut rng, own.0))
                .max()
                .unwrap_or(90);
            let phase = (latest + 5) - (latest + 5) % 10;
            PartyMove::Pending { pace, phase: phase.clamp(10, 100) }
        }
        None => PartyMove::None,
    };
}

pub(super) fn plugin(app: &mut App) {
    app.add_observer(accept).add_observer(take_up).add_observer(place_party_move);
}

/// An actor's moment: it takes its turn (retail `ExecuteCharacterAction`,
/// `StartMonsterCombatAction`). It is revalidated: one no longer in the
/// fight or able, or whose weapons changed since its attacks were accepted,
/// loses its turn; defending takes no turn. One going on with its turn
/// after moving, or with attacks already spent, fights on. Otherwise a
/// member follows its orders and a monster decides for itself
/// ([`monster::Deliberates`]); either way what it decided is [`Decided`].
#[allow(clippy::type_complexity)]
pub(super) fn take_turn(
    moment: On<Moment>,
    mut commands: Commands,
    mut turns: Query<(&mut Turn, Option<&mut Ordered>), With<Combatant>>,
    sight: Sight,
    laid: crate::game::actors::Laid,
    held: Query<(&crate::game::actors::arm::Wielding, &crate::game::actors::arm::Tempo)>,
    actors: Query<(
        Has<crate::game::actors::CanFight>,
        Has<crate::game::characters::Calling>,
        (&crate::game::actors::Traits, &Vitals, &crate::game::actors::Level),
    )>,
    living: Query<&Vitals>,
    combatants: Single<Option<&super::Combatants>, With<Battle>>,
    content: Res<Catalogs>,
) {
    let actor = moment.actor;
    let combatants = *combatants;
    let combatants = combatants.map_or(&[][..], super::Combatants::entities);
    let (places, obstacles) = (&sight.places, &sight.obstacles);
    let Ok((mut turn, ordered)) = turns.get_mut(actor) else { return };
    let (Ok((can_fight, trained, (traits, vitals, level))), Ok((wielding, tempo))) =
        (actors.get(actor), held.get(actor))
    else {
        return;
    };
    let order = ordered.as_ref().map(|ordered| ordered.0);
    // The weapons accepted, or nothing where one was used up.
    let weapons_hold =
        wielding.0.iter().all(|(slot, &now)| now.is_none() || now == turn.weapons[slot]);
    let able = can_fight
        && match order {
            Some(Action::Attack { .. }) => weapons_hold,
            Some(Action::Defend) => false,
            _ => true,
        };
    if !able {
        turn.end();
        return;
    }
    let fights = Fights { waits: false, berserk: false };
    let fights_on = Decision::Fight(fights);
    if order.is_none()
        && (turn.attacks[WeaponSlot::Primary] != tempo.attacks[WeaponSlot::Primary] || turn.moved)
    {
        commands.trigger(Strike { actor, fights });
        return;
    }
    let action = match order {
        Some(Action::Attack { berserk }) => Decision::Fight(Fights { waits: false, berserk }),
        Some(Action::Magic(choice)) => Decision::Magic(choice),
        // A breath at the foe chosen costs a fifth of their stamina;
        // one who cannot breathe, or whose foe is out of the cone,
        // defends instead.
        Some(Action::Breathe(target)) => {
            let cone = &content.spells[SpellId::SPECIAL_ATTACK_CONE];
            let aimed = living.get(target).is_ok_and(|their| {
                combatants.contains(&target)
                    && !their.is_dead()
                    && spell_reaches(
                        &places.of(actor),
                        &places.of(target),
                        cone.reach.metres(),
                        obstacles,
                    )
            });
            if !(trained && aimed && super::breath::available(actor, traits, vitals, &laid)) {
                ordered.unwrap().0 = Action::Defend;
                turn.end();
                return;
            }
            Decision::Special(Breathes {
                special: None,
                summons: None,
                strength: crate::game::hits::specials::strength(level.0 as u8, vitals.stamina),
                toward: places.of(target).feet,
                cost: vitals.stamina.max / 5,
                cooldown: 0,
            })
        }
        Some(Action::Defend) => unreachable!("defending takes no turn"),
        None if sight.tempers.contains(actor) => {
            commands.trigger(monster::Deliberates { monster: actor });
            return;
        }
        None => fights_on,
    };
    commands.trigger(Decided { actor, decision: action, own_will: order.is_none() });
}

/// What an actor decided to do with its moment, before its conditions have
/// their say; `own_will` when it decided for itself rather than by orders.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Decided {
    #[event_target]
    pub actor: Entity,
    pub decision: Decision,
    pub own_will: bool,
}

/// What an actor decided, its conditions may take off it (retail
/// `GetConditionInterrupt`): insanity may turn it on its friends this round,
/// fear send a monster running unannounced, a lure draw it. What is left it
/// does, as one event at it; a cast is cast, or for a point cast the utility
/// magic it is.
#[allow(clippy::type_complexity)]
pub(super) fn act(
    decided: On<Decided>,
    mut commands: Commands,
    mut turns: Query<&mut Turn>,
    sight: Sight,
    fight: Single<(&mut Pause, Option<&super::Combatants>), With<Battle>>,
    formation: Single<&Formation>,
    held: Items,
    content: Res<Catalogs>,
    reaches: Query<&crate::game::actors::arm::Furthest>,
    ailing: Query<(
        &crate::game::characters::Conditions,
        &crate::game::actors::Attributes,
        Has<crate::game::actors::BlindFighting>,
        &crate::game::actors::Nature,
    )>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
    (lured, lures): (
        Query<&crate::game::magic::pointcasts::LureInfluence>,
        Query<&Transform, With<crate::game::magic::pointcasts::Lure>>,
    ),
) {
    let Decided { actor, decision: action, own_will } = *decided;
    let (mut pause, combatants) = fight.into_inner();
    let combatants = combatants.map_or(&[][..], super::Combatants::entities);
    let (Ok(mut turn), Ok((conditions, a, fights_blind, nature))) =
        (turns.get_mut(actor), ailing.get(actor))
    else {
        return;
    };
    let fights_on = Decision::Fight(Fights { waits: false, berserk: false });
    let against_foes = match action {
        Decision::Magic(choice) => choice
            .source
            .spell_id(&held)
            .is_some_and(|spell| content.spells[spell].aimed_at_enemies()),
        _ => matches!(action, Decision::Special(_) | Decision::Fight(_)),
    };
    let running = matches!(action, Decision::Retreat(r) if r.conduct == Conduct::Flee);
    let lure = lured.get(actor).ok().filter(|hold| hold.response == Response::Controlled);
    let plans = Plans {
        runs: own_will,
        running,
        turned_on_friends: turn.berserk,
        against_foes,
        lured: lure.is_some(),
    };
    let reach = reaches.get(actor).copied().unwrap_or_default();
    let friend = ailments::friend_in_reach(&sight, actor, reach, combatants, &formation);
    let has = |condition| conditions.has(condition);
    let strength = a[crate::content::characters::Attribute::Strength];
    let interrupted = ailments::interrupt(&mut rng, (has, strength, fights_blind), plans, friend);
    let action = match interrupted {
        None | Some(Interrupt::Turncoat) => action,
        // Insanity turns it on its friends this round.
        Some(Interrupt::Berserk) => {
            turn.berserk = true;
            fights_on
        }
        // Fear sends it running, without its flight's notice.
        Some(Interrupt::Panics) => {
            commands.entity(actor).insert(Fleeing);
            Decision::Retreat(Retreats { conduct: Conduct::Flee, quiet: true })
        }
        // A lure draws it, at half its stride whatever hastes or slows it
        // (retail's controlled move, `SetUpMonsterTurn`).
        Some(Interrupt::Lured) => {
            let toward = lure.and_then(|hold| lures.get(hold.lure).ok()).map(|at| at.translation);
            if let Some(toward) = toward
                && !turn.moved
            {
                turn.moved = true;
                commands.entity(actor).insert(super::approach::CombatMove {
                    budget: nature.stride.unwrap_or(0.0) * 0.5,
                    moved: 0.0,
                    goal: super::approach::MoveGoal::Retreat(toward),
                });
            }
            news.write(CombatNews::Interrupted { actor, interrupt: Interrupt::Lured });
            commands.trigger(Exerted { actor, effort: Effort::Wait });
            turn.end();
            pause.0 = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
            return;
        }
        Some(interrupt) => {
            if interrupt == Interrupt::Freed {
                commands.entity(actor).entry::<crate::game::characters::Conditions>().and_modify(
                    |mut conditions| {
                        conditions.lift(Condition::Webbed);
                    },
                );
            }
            news.write(CombatNews::Interrupted { actor, interrupt });
            turn.end();
            pause.0 = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
            return;
        }
    };
    match action {
        Decision::Fight(fights) => commands.trigger(Strike { actor, fights }),
        Decision::Retreat(retreats) if retreats.conduct == Conduct::Hold => {
            commands.trigger(Hold { actor });
        }
        Decision::Retreat(retreats) => commands.trigger(Retreat { actor, retreats }),
        Decision::Protect(ward) => commands.trigger(Guard { actor, ward }),
        Decision::Special(breathes) => commands.trigger(Breathe { actor, breathes }),
        Decision::Magic(choice) => {
            turn.end();
            pause.0 = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
            let Some(spell) = choice.source.spell_id(&held) else {
                commands.trigger(Refused { caster: actor, refusal: Refusal::NoItem });
                return;
            };
            commands.trigger(Cast { caster: actor, spell, choice, again: false });
        }
    }
}

/// Run, withdraw or hold back, the action holding the fight for its pause:
/// a monster that cannot walk, or has moved this round, waits instead
/// (retail falls back on `WAIT` when the move fails).
pub(super) fn retreat(
    retreat: On<Retreat>,
    mut commands: Commands,
    mut actors: Query<(
        &mut Turn,
        Option<&Walker>,
        (&crate::game::actors::Nature, &crate::game::actors::Modifiers),
        &crate::game::characters::Conditions,
    )>,
    mut pause: Single<&mut Pause, With<Battle>>,
    places: crate::game::actors::Places,
    party: Single<Entity, With<PartyRoster>>,
    mut news: MessageWriter<CombatNews>,
) {
    let Retreat { actor, retreats } = *retreat;
    let Ok((mut turn, walker, (nature, modifiers), conditions)) = actors.get_mut(actor) else {
        return;
    };
    turn.end();
    pause.0 = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
    if !walker.is_some_and(|w| w.speed > 0.0) || turn.moved {
        turn.waited = true;
        commands.trigger(Exerted { actor, effort: Effort::Wait });
        return;
    }
    turn.moved = true;
    let effort = if retreats.conduct == Conduct::Flee { Effort::Run } else { Effort::Return };
    commands.trigger(Exerted { actor, effort });
    let slowed = conditions.has(Condition::Slowed);
    let metres = close::metres((nature.stride, modifiers.laid.stride), slowed, false);
    let feet = places.of(actor).feet;
    let away = close::withdraws_to(feet, places.of(*party).eye, metres);
    commands.entity(actor).insert(super::approach::CombatMove {
        budget: metres,
        moved: 0.0,
        goal: super::approach::MoveGoal::Retreat(away),
    });
    if !retreats.quiet {
        news.write(match retreats.conduct {
            Conduct::Flee => CombatNews::Flees { monster: actor },
            _ => CombatNews::Withdraws { monster: actor },
        });
    }
}

/// Hold back and rest: done for the round, without a pause.
pub(super) fn hold(hold: On<Hold>, mut turns: Query<&mut Turn>, mut commands: Commands) {
    if let Ok(mut turn) = turns.get_mut(hold.actor) {
        turn.end();
        commands.trigger(Exerted { actor: hold.actor, effort: Effort::Wait });
    }
}

/// Guard the fellow chosen this round, taking blows in its ward's place up
/// to its attacks a round.
pub(super) fn guard(
    guard: On<Guard>,
    mut commands: Commands,
    mut actors: Query<(&mut Turn, &crate::game::actors::arm::Tempo)>,
    mut pause: Single<&mut Pause, With<Battle>>,
) {
    let Guard { actor, ward } = *guard;
    let Ok((mut turn, tempo)) = actors.get_mut(actor) else { return };
    turn.end();
    pause.0 = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
    commands.entity(actor).insert((
        Guarding(ward),
        Interceptions { taken: 0, allowed: tempo.attacks[WeaponSlot::Primary] },
    ));
}

/// Make the special attack or breath (retail `ExecuteMonsterSpecialAttack`,
/// special-attack entry 30): a deed reaching whoever stands in its cone,
/// whatever their side, or a summoning calling its group up; it costs its
/// stamina, and a monster's cooldown starts.
#[allow(clippy::type_complexity)]
pub(super) fn breathe(
    breathe: On<Breathe>,
    mut commands: Commands,
    mut turns: Query<&mut Turn>,
    mut vitals: Query<&mut Vitals>,
    fight: Single<(&mut Round, &mut Pause, Option<&super::Combatants>), With<Battle>>,
    places: crate::game::actors::Places,
    obstacles: WorldObstacles,
    content: Res<Catalogs>,
    mut news: MessageWriter<CombatNews>,
) {
    let Breathe { actor, breathes } = *breathe;
    let (mut round, mut pause, combatants) = fight.into_inner();
    let combatants = combatants.map_or(&[][..], super::Combatants::entities);
    let Ok(mut turn) = turns.get_mut(actor) else { return };
    let Ok(mut own) = vitals.get_mut(actor) else { return };
    turn.end();
    pause.0 = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
    turn.special = true;
    round.attacked(true);
    news.write(match breathes.special {
        Some(kind) => CombatNews::Special { monster: actor, kind },
        None => CombatNews::Breathed { actor },
    });
    own.tire(breathes.cost);
    if breathes.special.is_some() {
        commands.entity(actor).insert(SpecialCooldown(breathes.cooldown));
    }
    let from = places.of(actor);
    if let Some(species) = breathes.summons {
        commands.trigger(summon::Summon { summoner: actor, species });
    } else {
        let rows = Rows::Special(breathes.special.unwrap_or(crate::game::hits::specials::BREATH));
        let cone = content.spells[SpellId::SPECIAL_ATTACK_CONE].reach.metres();
        let toward = breathes.toward - from.eye;
        let victims = combatants
            .iter()
            .copied()
            .filter(|&other| other != actor)
            .filter(|&other| {
                vitals.get(other).is_ok_and(|their| {
                    !their.is_dead()
                        && spell_reaches(&from, &places.of(other), cone, &obstacles)
                        && in_cone(toward, places.of(other).feet - from.eye)
                })
            })
            .collect::<Vec<Entity>>();
        let strike = effects::Strike::new(rows, breathes.strength, None, Some(actor), &content);
        commands.spawn((Deed::at(strike, victims), Breath));
    }
}

/// A deed that is a breath or another special attack.
#[derive(Component)]
pub(super) struct Breath;

/// What a breath did is told as the fight's news.
pub(super) fn tell_breath(
    landed: On<Landed>,
    breaths: Query<(), With<Breath>>,
    mut news: MessageWriter<CombatNews>,
) {
    if breaths.contains(landed.deed) && !landed.effects.is_empty() {
        news.write(CombatNews::Effects(landed.effects.clone()));
    }
}

/// A member whose cast was refused, whoever refused it, defends instead
/// (retail `ExecuteCharacterAction`), the turn spent.
pub(super) fn defend_instead(refused: On<Refused>, mut orders: Query<&mut Ordered>) {
    if let Ok(mut ordered) = orders.get_mut(refused.caster)
        && matches!(ordered.0, Action::Magic(_))
    {
        ordered.0 = Action::Defend;
    }
}
