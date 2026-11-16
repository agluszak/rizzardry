//! Turns in a round (`docs/rules/monsters.md`, "How this is built"). Taking
//! up a turn writes its facts onto the actor: its next moment, the attacks
//! each hand has left, the weapons it accepted them with; a member's order
//! is locked. When its moment comes ([`Moment`]) it is revalidated and then
//! decides by its kind: a member follows their order, a monster deliberates
//! ([`super::monster`]). What it decided is an action entity
//! ([`super::action`]). A moment ends by [`Done`] and [`Shown`].

use bevy::prelude::*;
use enum_map::EnumMap;

use super::action::{ActionOf, Acts, Breathes, Casts, Fights, Strikes};
use super::attack::phase;
use super::{ACTION_SECONDS, Battle, Combatant, PartyMove, Pause, Round};
use crate::content::characters::Trait;
use crate::content::spells::SpellId;
use crate::game::actors::Initiative;
use crate::game::actors::arm::{Arms, AttacksARound, Hand, Reaches, Sword, Tempo, Wielding};
use crate::game::characters::{Calling, Condition, Conditions, Vitals};
use crate::game::data::Catalogs;
use crate::game::inventory::WeaponSlot;
use crate::game::magic::casting::{MagicChoice, reaches as spell_reaches};
use crate::game::party::Order;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::Temper;

/// The moment an actor acts next this round, 1 to 100. Gone when it is done
/// for the round.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct NextMoment(pub u32);

/// The attacks each hand has left this round (retail `hand_attack_values`; a
/// monster's all in its main hand), and the hand due to attack next (retail
/// `current_hand`).
#[derive(Component, Clone, Copy, Debug)]
pub struct AttacksLeft {
    pub left: EnumMap<WeaponSlot, u32>,
    pub due: WeaponSlot,
}

impl AttacksLeft {
    /// Nothing is left to strike with in a hand (a launcher out of shots).
    pub fn run_out(&mut self, slot: WeaponSlot) {
        self.left[slot] = 0;
    }
}

/// The weapons each hand accepted its attacks with this round.
#[derive(Component, Clone, Copy, Debug)]
pub struct AcceptedWith(pub EnumMap<WeaponSlot, Option<Entity>>);

/// A Samurai's Lightning Strike, readied for the main hand's next attack
/// (retail `extra_swings`), by their Samurai level.
#[derive(Component, Clone, Copy, Debug)]
pub struct Lightning(pub u8);

/// It has made its move this round.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Moved;

/// It made its special attack this round.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct MadeSpecial;

/// It holds back on guard this round: a monster that waited, a member
/// ordered to defend. It rests at the round's end and is better guarded.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Waited;

/// Insanity turned it on its friends this round (retail `berserk`): whom its
/// blows seek is its friends, and an action its ailments take later this
/// round is a strike at them instead.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Berserk;

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

/// What a round leaves on an actor, gone as the next is taken up.
type RoundsFacts = (
    NextMoment,
    AttacksLeft,
    AcceptedWith,
    Lightning,
    Moved,
    MadeSpecial,
    Waited,
    Berserk,
    Guarding,
    Interceptions,
);

/// The fight is over: what its rounds left on the members and monsters goes
/// with it.
pub fn clear_fight(left: On<Remove<Combatant>>, mut commands: Commands) {
    commands.entity(left.entity).try_remove::<RoundsFacts>().try_remove::<(
        super::other_set::OtherSet,
        super::other_set::WithOtherSet,
        Ordered,
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

/// A combatant takes up its turn for the round, by its kind: a member by
/// their order, a monster to fight.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct TakesUp {
    #[event_target]
    actor: Entity,
    surprised: bool,
}

/// A combatant takes up its attacks for the round.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub(super) struct TakesUpAttacks {
    #[event_target]
    pub(super) actor: Entity,
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
        commands.entity(actor).remove::<RoundsFacts>();
        commands.trigger(TakesUp { actor, surprised });
    }
    commands.trigger(PlacesPartyMove);
}

/// A monster takes up its turn to fight.
fn monster_takes_up(takes: On<TakesUp>, mut commands: Commands) {
    commands.trigger(TakesUpAttacks { actor: takes.actor });
}

/// A member takes up their turn: their order is locked. One turned on the
/// party attacks whatever their order; a Berserk order needs the trait, an
/// arm, and a main hand for close quarters (retail `CanCharacterBerserk`). A
/// Samurai attacking with a sword in the main hand readies Lightning Strike,
/// by their Samurai level, at `12` scaled by it in a hundred, unless the
/// party was surprised. One that attacks takes up their attacks; one that
/// casts or breathes acts by their own initiative; one that defends takes no
/// moment and holds back.
#[allow(clippy::type_complexity)]
fn member_takes_up(
    takes: On<TakesUp>,
    mut commands: Commands,
    members: Query<(
        (&Order, &crate::game::actors::Traits, &Calling, &Conditions),
        (Has<crate::game::actors::CanFight>, &Initiative, &Tempo, &Arms),
    )>,
    arms: Query<(&Hand, &Reaches, &AttacksARound, Has<Sword>)>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let TakesUp { actor, surprised } = *takes;
    let ((order, traits, calling, conditions), (can_fight, own, tempo, held)) =
        members.get(actor)?;
    let mut mains = Vec::new();
    for &arm in held.entities() {
        let (hand, reach, attacks, sword) = arms.get(arm)?;
        if hand.0 == WeaponSlot::Primary && attacks.0 > 0 {
            mains.push((reach.0, sword));
        }
    }
    let striking = tempo.attacks.values().any(|&attacks| attacks > 0);
    let action = match *order {
        _ if conditions.has(Condition::Turncoat) => Action::Attack { berserk: false },
        Order::Attack => Action::Attack { berserk: false },
        Order::Berserk => Action::Attack {
            berserk: traits.has(Trait::Berserk)
                && striking
                && mains.iter().all(|(reach, _)| *reach <= crate::content::species::Reach::Short),
        },
        Order::Defend => Action::Defend,
        Order::Magic(choice) => Action::Magic(choice),
        Order::Breathe(target) => Action::Breathe(target),
    };
    let mut member = commands.entity(actor);
    member.insert((Ordered(action), Spotted(0)));
    if !surprised
        && *order == Order::Attack
        && traits.has(Trait::LightningStrike)
        && mains.iter().any(|(_, sword)| *sword)
        && rng.below(100) < u32::from(calling.scale(12.0) as u8)
    {
        member.insert(Lightning(calling.mastery()));
    }
    match action {
        Action::Attack { .. } => {
            commands.trigger(TakesUpAttacks { actor });
        }
        Action::Magic(_) | Action::Breathe(_) if can_fight => {
            member.insert(NextMoment(phase(&mut rng, own.0)));
        }
        Action::Magic(_) | Action::Breathe(_) => {}
        Action::Defend => {
            member.insert(Waited);
        }
    }
    Ok(())
}

/// One that can fight takes up its attacks: it opens with the hand of most
/// attacks, its initiative counting (retail `AssignCombatPhases`), with the
/// weapons it wields now.
fn take_up_attacks(
    takes: On<TakesUpAttacks>,
    mut commands: Commands,
    actors: Query<(Has<crate::game::actors::CanFight>, &Initiative, &Tempo, &Wielding)>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let actor = takes.actor;
    let (can_fight, own, tempo, wielding) = actors.get(actor)?;
    let Some(slot) = tempo.opening().filter(|_| can_fight) else { return Ok(()) };
    commands.entity(actor).insert((
        AttacksLeft { left: tempo.attacks, due: slot },
        AcceptedWith(wielding.0),
        NextMoment(phase(&mut rng, own.0 + tempo.initiative[slot])),
    ));
    Ok(())
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
) -> Result {
    let (round, mut party_move) = fight.into_inner();
    *party_move = match round.pace {
        Some(pace) => {
            let mut latest = None;
            for member in roster.iter() {
                let (vitals, own) = members.get(member)?;
                if !vitals.is_dead() {
                    latest = latest.max(Some(phase(&mut rng, own.0)));
                }
            }
            let latest = latest.unwrap_or(90);
            let phase = (latest + 5) - (latest + 5) % 10;
            PartyMove::Pending { pace, phase: phase.clamp(10, 100) }
        }
        None => PartyMove::None,
    };
    Ok(())
}

/// An actor's moment has come: it acts now.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Moment {
    #[event_target]
    pub actor: Entity,
}

/// A revalidated actor decides what to do with its moment, by its kind.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Decides {
    #[event_target]
    pub actor: Entity,
}

/// Its moment is revalidated (retail `ExecuteCharacterAction`,
/// `StartMonsterCombatAction`): one no longer able to fight, or whose
/// weapons changed since it accepted its attacks, is done for the round.
/// One that took up no attacks (a member casting or breathing) has no
/// weapons to keep.
fn revalidate(
    moment: On<Moment>,
    mut commands: Commands,
    actors: Query<(Has<crate::game::actors::CanFight>, &Wielding, Option<&AcceptedWith>)>,
) -> Result {
    let actor = moment.actor;
    let (can_fight, wielding, accepted) = actors.get(actor)?;
    // The weapons accepted, or nothing where one was used up.
    let weapons_hold = match accepted {
        Some(accepted) => {
            wielding.0.iter().all(|(slot, &now)| now.is_none() || now == accepted.0[slot])
        }
        None => true,
    };
    if can_fight && weapons_hold {
        commands.trigger(Decides { actor });
    } else {
        commands.trigger(Done { actor });
    }
    Ok(())
}

/// A member follows their order. A breath order is checked by its own
/// observer.
fn follow_order(
    decides: On<Decides>,
    mut commands: Commands,
    members: Query<&Ordered>,
    held: crate::game::items::Items,
    content: Res<Catalogs>,
) -> Result {
    let actor = decides.actor;
    match members.get(actor)?.0 {
        Action::Attack { berserk } => {
            let action =
                commands.spawn((ActionOf(actor), Strikes(Fights { waits: false, berserk }))).id();
            commands.trigger(Acts { action });
        }
        Action::Magic(choice) => {
            let at_foes = choice
                .source
                .spell_id(&held)
                .is_some_and(|spell| content.spells[spell].aimed_at_enemies());
            let mut action = commands.spawn((ActionOf(actor), Casts(choice)));
            if at_foes {
                action.insert(super::action::AtFoes);
            }
            let action = action.id();
            commands.trigger(Acts { action });
        }
        Action::Breathe(target) => commands.trigger(BreathOrdered { actor, target }),
        Action::Defend => unreachable!("one defending takes no moment"),
    }
    Ok(())
}

/// A member's breath order is followed.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct BreathOrdered {
    #[event_target]
    actor: Entity,
    target: Entity,
}

/// A breath at the foe chosen costs a fifth of their stamina; one who has no
/// breath to take now, or whose foe is no longer fighting, alive and in the
/// cone's reach, defends instead and is done.
#[allow(clippy::type_complexity)]
fn breathe_as_ordered(
    ordered: On<BreathOrdered>,
    mut commands: Commands,
    mut members: Query<(
        &mut Ordered,
        Has<super::breath::CanBreathe>,
        &Vitals,
        &crate::game::actors::Level,
    )>,
    living: Query<&Vitals>,
    combatants: Single<Option<&super::Combatants>, With<Battle>>,
    places: crate::game::actors::Places,
    obstacles: WorldObstacles,
    content: Res<Catalogs>,
) -> Result {
    let BreathOrdered { actor, target } = *ordered;
    let combatants = combatants.map_or(&[][..], super::Combatants::entities);
    let (mut order, can_breathe, vitals, level) = members.get_mut(actor)?;
    let cone = content.spells[SpellId::SPECIAL_ATTACK_CONE].reach.metres();
    let aimed = combatants.contains(&target)
        && !living.get(target)?.is_dead()
        && spell_reaches(&places.of(actor), &places.of(target), cone, &obstacles);
    if !(can_breathe && aimed) {
        order.0 = Action::Defend;
        commands.trigger(Done { actor });
        return Ok(());
    }
    let action = commands
        .spawn((
            ActionOf(actor),
            Breathes {
                special: None,
                summons: None,
                strength: crate::game::hits::specials::strength(level.0 as u8, vitals.stamina),
                toward: places.of(target).feet,
                cost: vitals.stamina.max / 5,
                cooldown: 0,
            },
        ))
        .id();
    commands.trigger(Acts { action });
    Ok(())
}

/// A monster decides for itself: one going on with its turn after moving,
/// or with attacks already spent, fights on; otherwise it deliberates.
fn decide_for_itself(
    decides: On<Decides>,
    mut commands: Commands,
    monsters: Query<(&Tempo, Option<&AttacksLeft>, Has<Moved>)>,
) -> Result {
    let monster = decides.actor;
    let (tempo, left, moved) = monsters.get(monster)?;
    let spent = left
        .is_some_and(|left| left.left[WeaponSlot::Primary] != tempo.attacks[WeaponSlot::Primary]);
    if spent || moved {
        let action = commands
            .spawn((ActionOf(monster), Strikes(Fights { waits: false, berserk: false })))
            .id();
        commands.trigger(Acts { action });
    } else {
        commands.trigger(super::monster::Deliberates { monster });
    }
    Ok(())
}

/// Its moment is over and nothing is left to it this round.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Done {
    #[event_target]
    pub actor: Entity,
}

/// It did something the player watches: the fight holds for the action's
/// pause.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Shown {
    #[event_target]
    pub actor: Entity,
}

/// A dead actor's body may be gone by the time its moment is over.
fn done(done: On<Done>, mut commands: Commands) {
    commands.entity(done.actor).try_remove::<(NextMoment, AttacksLeft)>();
}

fn shown(_: On<Shown>, mut pause: Single<&mut Pause, With<Battle>>) {
    pause.0 = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
}

/// How each kind of actor takes up its turn and decides at its moment is
/// set where the kind is made: a member by their order, a monster by its
/// temper.
fn by_order(added: On<Add<Order>>, mut commands: Commands) {
    commands
        .entity(added.entity)
        .observe(member_takes_up)
        .observe(follow_order)
        .observe(super::other_set::member_strikes);
}

fn by_temper(added: On<Add<Temper>>, mut commands: Commands) {
    commands
        .entity(added.entity)
        .observe(monster_takes_up)
        .observe(decide_for_itself)
        .observe(super::swings::monster_strikes);
}

pub(super) fn plugin(app: &mut App) {
    app.add_observer(accept)
        .add_observer(take_up_attacks)
        .add_observer(place_party_move)
        .add_observer(revalidate)
        .add_observer(breathe_as_ordered)
        .add_observer(done)
        .add_observer(shown)
        .add_observer(by_order)
        .add_observer(by_temper)
        .add_observer(clear_fight);
}
