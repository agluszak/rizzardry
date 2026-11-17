//! What an actor does with its moment (`docs/rules/monsters.md` section 8
//! and "How this is built"). Whoever decides makes an action entity
//! ([`ActionOf`] the actor); what it is is which component it has
//! ([`Strikes`], [`Retreats`], [`Holds`], [`Protects`], [`Breathes`],
//! [`Casts`]), and each kind's doer is set on it as it is made. Before it is
//! done its ailments weigh it, in retail's order (`GetConditionInterrupt`,
//! `Combat.cpp:2940`), one event and one observer each; one may take it, or
//! it goes on, [`Unhindered`], to be done.

use bevy::ecs::lifecycle::HookContext;
use bevy::ecs::world::DeferredWorld;
use bevy::prelude::*;

use super::effort::{Effort, Exerted};
use super::monster::close;
use super::monster::conduct::Conduct;
use super::swings::Strike;
use super::turn::{
    Berserk, Done, Fleeing, Guarding, Interceptions, MadeSpecial, Moved, Ordered, Shown,
    SpecialCooldown, Waited,
};
use super::{Battle, CombatNews, Combatants, Round, summon};
use crate::content::characters::Attribute;
use crate::content::species::SpecialAttackKind;
use crate::content::spells::SpellId;
use crate::game::actors::arm::Tempo;
use crate::game::actors::{ActsThoughBlind, Attributes, Nature, Places};
use crate::game::characters::{Condition, Conditions, Vitals};
use crate::game::data::Catalogs;
use crate::game::hits::definitions::Rows;
use crate::game::hits::{Deed, Landed, effects};
use crate::game::inventory::WeaponSlot;
use crate::game::items::Items;
use crate::game::magic::Strength;
use crate::game::magic::casting::{Cast, MagicChoice, Refused, reaches as spell_reaches};
use crate::game::magic::may::Refusal;
use crate::game::magic::pointcasts::{Lure, LureInfluence};
use crate::game::magic::reach::in_cone;
use crate::game::magic::workings::Response;
use crate::game::party::roster::{Party, PartyMember};
use crate::game::rng::GameRng;
use crate::game::world::hostility::Stance;
use crate::game::world::navigation::Walker;
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::Temper;

/// An action, of the actor taking it. It lasts no longer than the moment
/// it is made in.
#[derive(Component, Clone, Copy, Debug)]
pub struct ActionOf(pub Entity);

/// An action is made: its actor's ailments weigh it, then it is done. Its
/// maker triggers it once the action is spawned.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Acts {
    #[event_target]
    pub action: Entity,
}

/// It is aimed at the actor's foes: a strike, a breath, a cast of a spell
/// against enemies.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct AtFoes;

/// Strike someone, or close in to; `waits` when it may hold back instead;
/// `berserk` for a Berserk order's fury.
#[derive(Clone, Copy, Debug)]
pub struct Fights {
    pub waits: bool,
    pub berserk: bool,
}

/// It strikes.
#[derive(Component, Clone, Copy, Debug)]
#[require(AtFoes)]
#[component(on_add = strikes)]
pub struct Strikes(pub Fights);

/// It runs for its life or withdraws; `quiet` for a flight fear forced,
/// which retail does not announce.
#[derive(Component, Clone, Copy, Debug)]
#[component(on_add = retreats)]
pub struct Retreats {
    pub conduct: Conduct,
    pub quiet: bool,
}

/// It holds back and rests.
#[derive(Component, Clone, Copy, Debug)]
#[component(on_add = holds)]
pub struct Holds;

/// It guards a fellow this round.
#[derive(Component, Clone, Copy, Debug)]
#[component(on_add = protects)]
pub struct Protects(pub Entity);

/// It makes its special attack, or a breath when it is of no kind: a deed
/// at this strength toward `toward`, catching whoever stands in its cone,
/// or a summoning of a species; it costs stamina, and a monster's cools
/// down.
#[derive(Component, Clone, Copy, Debug)]
#[require(AtFoes)]
#[component(on_add = breathes)]
pub struct Breathes {
    pub special: Option<SpecialAttackKind>,
    pub summons: Option<u32>,
    pub strength: Strength,
    pub toward: Vec3,
    pub cost: u32,
    pub cooldown: u8,
}

/// It casts a spell, or uses an item's; its maker marks it [`AtFoes`] when
/// the spell is against enemies.
#[derive(Component, Clone, Copy, Debug)]
#[component(on_add = casts)]
pub struct Casts(pub MagicChoice);

fn strikes(mut world: DeferredWorld, context: HookContext) {
    world.commands().entity(context.entity).observe(strike);
}

fn retreats(mut world: DeferredWorld, context: HookContext) {
    world.commands().entity(context.entity).observe(retreat_as_decided);
}

fn holds(mut world: DeferredWorld, context: HookContext) {
    world.commands().entity(context.entity).observe(hold);
}

fn protects(mut world: DeferredWorld, context: HookContext) {
    world.commands().entity(context.entity).observe(guard);
}

fn breathes(mut world: DeferredWorld, context: HookContext) {
    world.commands().entity(context.entity).observe(breathe);
}

fn casts(mut world: DeferredWorld, context: HookContext) {
    world.commands().entity(context.entity).observe(cast);
}

/// What an actor's conditions did to its action as it began; its number is
/// retail's, which numbers its notice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interrupt {
    /// A web held it.
    Webbed,
    /// Fear froze it.
    Afraid,
    Blinded,
    Nauseated,
    /// Insanity took it, in one of three fits.
    Insane {
        fit: u8,
    },
    /// It broke out of its web.
    Freed,
    /// Insanity turned it on its friends; it attacks them.
    Berserk,
    /// Turncoat holds it; its action against its new foes stands.
    Turncoat,
    /// Turncoat holds a member who can strike nobody: the action is lost.
    CannotStrike,
    /// A lure holds it: it walks toward the lure.
    Lured,
}

impl Interrupt {
    /// Retail's interrupt number, which numbers its notice.
    pub fn number(self) -> u8 {
        match self {
            Interrupt::Webbed => 0,
            Interrupt::Afraid => 1,
            Interrupt::Blinded => 2,
            Interrupt::Nauseated => 3,
            Interrupt::Insane { fit } => 4 + fit,
            Interrupt::Freed => 7,
            Interrupt::Berserk => 8,
            Interrupt::Turncoat => 9,
            Interrupt::CannotStrike => 10,
            Interrupt::Lured => 11,
        }
    }
}

// The steps of weighing an action against its actor's ailments, in
// retail's order, and what comes of them.

/// A web weighs it.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct WeighsWeb {
    #[event_target]
    pub action: Entity,
}

/// Blindness weighs it.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct WeighsBlindness {
    #[event_target]
    pub action: Entity,
}

/// Insanity weighs it.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct WeighsInsanity {
    #[event_target]
    pub action: Entity,
}

/// Fear weighs it.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct WeighsFear {
    #[event_target]
    pub action: Entity,
}

/// Panic weighs it.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct WeighsPanic {
    #[event_target]
    pub action: Entity,
}

/// A lure weighs it.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct WeighsLure {
    #[event_target]
    pub action: Entity,
}

/// Turncoat weighs it.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct WeighsTurncoat {
    #[event_target]
    pub action: Entity,
}

/// No ailment took it: it is done.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Unhindered {
    #[event_target]
    pub action: Entity,
}

/// An ailment took it and it is lost.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Loses {
    #[event_target]
    pub action: Entity,
}

/// It broke out of a web.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct BrokeFree {
    #[event_target]
    pub action: Entity,
}

/// Fear sent its monster running.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Panics {
    #[event_target]
    pub action: Entity,
}

/// A lure draws its actor.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Lured {
    #[event_target]
    pub action: Entity,
}

/// Insanity turned its actor on its friends.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct TurnsOnFriends {
    #[event_target]
    pub action: Entity,
}

/// What took an action is told (retail `g_condition_interrupt_notices`).
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Interrupted {
    #[event_target]
    pub action: Entity,
    pub interrupt: Interrupt,
}

/// Nauseated: 25 percent, it loses the action.
fn nausea(
    weighs: On<Acts>,
    mut commands: Commands,
    actions: Query<&ActionOf>,
    actors: Query<&Conditions>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let action = weighs.action;
    let &ActionOf(actor) = actions.get(action)?;
    if actors.get(actor)?.has(Condition::Nauseated) && rng.below(100) < 25 {
        commands.trigger(Interrupted { action, interrupt: Interrupt::Nauseated });
        commands.trigger(Loses { action });
    } else {
        commands.trigger(WeighsWeb { action });
    }
    Ok(())
}

/// Webbed: it breaks free with chance `Strength / 3` percent, else the web
/// holds it; either way the action is lost.
fn web(
    weighs: On<WeighsWeb>,
    mut commands: Commands,
    actions: Query<&ActionOf>,
    actors: Query<(&Conditions, &Attributes)>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let action = weighs.action;
    let &ActionOf(actor) = actions.get(action)?;
    let (conditions, attributes) = actors.get(actor)?;
    if !conditions.has(Condition::Webbed) {
        commands.trigger(WeighsBlindness { action });
        return Ok(());
    }
    if rng.below(100) < attributes[Attribute::Strength] / 3 {
        commands.trigger(Interrupted { action, interrupt: Interrupt::Freed });
        commands.trigger(BrokeFree { action });
    } else {
        commands.trigger(Interrupted { action, interrupt: Interrupt::Webbed });
    }
    commands.trigger(Loses { action });
    Ok(())
}

/// Blind, and not unhindered by it: 20 percent, it loses the action
/// (retail `Combat.cpp:3002`).
fn blindness(
    weighs: On<WeighsBlindness>,
    mut commands: Commands,
    actions: Query<&ActionOf>,
    actors: Query<(&Conditions, Has<ActsThoughBlind>)>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let action = weighs.action;
    let &ActionOf(actor) = actions.get(action)?;
    let (conditions, fights_blind) = actors.get(actor)?;
    if conditions.has(Condition::Blind) && !fights_blind && rng.below(100) < 20 {
        commands.trigger(Interrupted { action, interrupt: Interrupt::Blinded });
        commands.trigger(Loses { action });
    } else {
        commands.trigger(WeighsInsanity { action });
    }
    Ok(())
}

/// Insane: 80 percent. If it is not berserk yet, 40 percent it turns on its
/// friends if it can strike one ([`turn_on_friends`]); otherwise it loses
/// the action in one of three fits.
fn insanity(
    weighs: On<WeighsInsanity>,
    mut commands: Commands,
    actions: Query<&ActionOf>,
    actors: Query<(&Conditions, Has<Berserk>)>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let action = weighs.action;
    let &ActionOf(actor) = actions.get(action)?;
    let (conditions, berserk) = actors.get(actor)?;
    if !conditions.has(Condition::Insane) || rng.below(100) >= 80 {
        commands.trigger(WeighsFear { action });
        return Ok(());
    }
    if !berserk && rng.below(100) < 40 {
        commands.trigger(TurnsOnFriends { action });
    } else {
        let fit = rng.below(3) as u8;
        commands.trigger(Interrupted { action, interrupt: Interrupt::Insane { fit } });
        commands.trigger(Loses { action });
    }
    Ok(())
}

/// Afraid: 25 percent, unless the action is a run for its life, it loses
/// the action.
fn fear(
    weighs: On<WeighsFear>,
    mut commands: Commands,
    actions: Query<(&ActionOf, Option<&Retreats>)>,
    actors: Query<&Conditions>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let action = weighs.action;
    let (&ActionOf(actor), retreats) = actions.get(action)?;
    let running = retreats.is_some_and(|retreats| retreats.conduct == Conduct::Flee);
    if actors.get(actor)?.has(Condition::Afraid) && !running && rng.below(100) < 25 {
        commands.trigger(Interrupted { action, interrupt: Interrupt::Afraid });
        commands.trigger(Loses { action });
    } else {
        commands.trigger(WeighsPanic { action });
    }
    Ok(())
}

/// Afraid, a monster (one that decides for itself): 40 percent it runs for
/// its life. Retail's notice for it is past the end of its table: it is
/// told by nothing.
fn panic(
    weighs: On<WeighsPanic>,
    mut commands: Commands,
    actions: Query<&ActionOf>,
    actors: Query<(&Conditions, Has<Temper>)>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let action = weighs.action;
    let &ActionOf(actor) = actions.get(action)?;
    let (conditions, own_will) = actors.get(actor)?;
    if own_will && conditions.has(Condition::Afraid) && rng.below(100) < 40 {
        commands.trigger(Panics { action });
    } else {
        commands.trigger(WeighsLure { action });
    }
    Ok(())
}

/// A lure that controls it draws it instead.
fn lure(
    weighs: On<WeighsLure>,
    mut commands: Commands,
    actions: Query<&ActionOf>,
    actors: Query<Option<&LureInfluence>>,
) -> Result {
    let action = weighs.action;
    let &ActionOf(actor) = actions.get(action)?;
    if actors.get(actor)?.is_some_and(|hold| hold.response == Response::Controlled) {
        commands.trigger(Interrupted { action, interrupt: Interrupt::Lured });
        commands.trigger(Lured { action });
    } else {
        commands.trigger(WeighsTurncoat { action });
    }
    Ok(())
}

/// Turncoat: a monster's action against its new foes stands, and is told.
/// A member's action, whatever it was, becomes a strike at their fellow
/// members (retail `Combat.cpp:2207`), told by what it finds
/// ([`super::other_set`]).
fn turncoat(
    weighs: On<WeighsTurncoat>,
    mut commands: Commands,
    actions: Query<(&ActionOf, Has<AtFoes>)>,
    actors: Query<(&Conditions, Has<PartyMember>)>,
) -> Result {
    let action = weighs.action;
    let (&ActionOf(actor), at_foes) = actions.get(action)?;
    let (conditions, member) = actors.get(actor)?;
    if !conditions.has(Condition::Turncoat) {
        commands.trigger(Unhindered { action });
    } else if member {
        commands.trigger(Strike { actor, fights: Fights { waits: false, berserk: false } });
        commands.entity(action).despawn();
    } else {
        if at_foes {
            commands.trigger(Interrupted { action, interrupt: Interrupt::Turncoat });
        }
        commands.trigger(Unhindered { action });
    }
    Ok(())
}

/// What took an action is told.
fn tell(
    interrupted: On<Interrupted>,
    actions: Query<&ActionOf>,
    mut news: MessageWriter<CombatNews>,
) -> Result {
    let &ActionOf(actor) = actions.get(interrupted.action)?;
    news.write(CombatNews::Interrupted { actor, interrupt: interrupted.interrupt });
    Ok(())
}

/// A lost action ends the moment, and the pause after it; a monster
/// berserk this round strikes at its friends instead (retail
/// `Combat.cpp:2461`, in the monster's turn only).
fn lose(
    loses: On<Loses>,
    mut commands: Commands,
    actions: Query<&ActionOf>,
    actors: Query<(Has<Berserk>, Has<Temper>)>,
) -> Result {
    let action = loses.action;
    let &ActionOf(actor) = actions.get(action)?;
    // Only a monster's (retail's member has no such turn).
    let (berserk, monster) = actors.get(actor)?;
    if berserk && monster {
        commands.trigger(Strike { actor, fights: Fights { waits: false, berserk: false } });
    } else {
        commands.trigger(Done { actor });
        commands.trigger(Shown { actor });
    }
    commands.entity(action).despawn();
    Ok(())
}

/// Breaking out lifts the web.
fn break_free(
    broke: On<BrokeFree>,
    actions: Query<&ActionOf>,
    mut actors: Query<&mut Conditions>,
) -> Result {
    let &ActionOf(actor) = actions.get(broke.action)?;
    actors.get_mut(actor)?.lift(Condition::Webbed);
    Ok(())
}

/// Insanity would turn it on its friends: it looks for a friend to strike,
/// its fellow members only for a member (retail's check in its friendly
/// band, `Combat Range.cpp:77`, `:533`). Finding one, it is berserk for the
/// rest of the round, told, and strikes; finding nobody it loses the action
/// in a fit, or, its eye missing one it could have struck, berserk all the
/// same with nobody to strike.
#[allow(clippy::type_complexity)]
fn turn_on_friends(
    turns: On<TurnsOnFriends>,
    mut commands: Commands,
    actions: Query<&ActionOf>,
    actors: Query<(Has<PartyMember>, Has<super::turn::AttacksLeft>)>,
) -> Result {
    let action = turns.action;
    let &ActionOf(actor) = actions.get(action)?;
    let (member, has_attacks) = actors.get(actor)?;
    // One that took up no attacks takes them up now (retail
    // `PrepareCharacterAttacks`, `Combat.cpp:2224`).
    if !has_attacks {
        commands.trigger(super::turn::TakesUpAttacks { actor });
    }
    let fights = Fights { waits: false, berserk: false };
    let attack = super::swings::begin(&mut commands, actor, fights, Stance::Friendly);
    let mut made = commands.entity(attack);
    if member {
        made.insert(super::swings::AmongFellows);
    }
    made.observe(turned_berserk).observe(fit_instead);
    commands.trigger(super::swings::Seeks { attack });
    commands.entity(action).despawn();
    Ok(())
}

/// It found a friend to strike: berserk for the rest of the round, and told.
fn turned_berserk(
    found: On<super::swings::Found>,
    mut commands: Commands,
    attacks: Query<&super::swings::AttackBy>,
    mut news: MessageWriter<CombatNews>,
) -> Result {
    let &super::swings::AttackBy(actor) = attacks.get(found.attack)?;
    commands.entity(actor).insert(Berserk);
    news.write(CombatNews::Interrupted { actor, interrupt: Interrupt::Berserk });
    Ok(())
}

/// It found no friend to strike: a fit takes the action, or berserk with
/// nobody to strike when its eye missed one.
fn fit_instead(
    nobody: On<super::swings::Nobody>,
    mut commands: Commands,
    attacks: Query<&super::swings::AttackBy>,
    mut news: MessageWriter<CombatNews>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let attack = nobody.attack;
    let &super::swings::AttackBy(actor) = attacks.get(attack)?;
    commands.entity(attack).despawn();
    let interrupt = if nobody.evaded {
        commands.entity(actor).insert(Berserk);
        Interrupt::Berserk
    } else {
        Interrupt::Insane { fit: rng.below(3) as u8 }
    };
    news.write(CombatNews::Interrupted { actor, interrupt });
    commands.trigger(Done { actor });
    Ok(())
}

/// Fear sends a monster running, without its flight's notice; it is
/// fleeing from now on.
fn run_in_panic(panics: On<Panics>, mut commands: Commands, actions: Query<&ActionOf>) -> Result {
    let action = panics.action;
    let &ActionOf(actor) = actions.get(action)?;
    commands.entity(actor).insert(Fleeing);
    commands.trigger(Retreat { actor, retreats: Retreats { conduct: Conduct::Flee, quiet: true } });
    commands.entity(action).despawn();
    Ok(())
}

/// A lure draws it, at half its stride whatever hastes or slows it (retail's
/// controlled move, `SetUpMonsterTurn`), unless it has moved this round;
/// its moment ends.
fn follow_lure(
    lured: On<Lured>,
    mut commands: Commands,
    actions: Query<&ActionOf>,
    actors: Query<(&LureInfluence, &Nature, Has<Moved>)>,
    lures: Query<&Transform, With<Lure>>,
) -> Result {
    let action = lured.action;
    let &ActionOf(actor) = actions.get(action)?;
    let (hold, nature, moved) = actors.get(actor)?;
    if let Ok(lure) = lures.get(hold.lure)
        && !moved
    {
        commands.entity(actor).insert((
            Moved,
            super::approach::CombatMove {
                budget: nature.stride.unwrap_or(0.0) * 0.5,
                moved: 0.0,
                goal: super::approach::MoveGoal::Retreat(lure.translation),
            },
        ));
    }
    commands.trigger(Exerted { actor, effort: Effort::Wait });
    commands.trigger(Done { actor });
    commands.trigger(Shown { actor });
    commands.entity(action).despawn();
    Ok(())
}

/// An action done is gone.
fn spend(unhindered: On<Unhindered>, mut commands: Commands) {
    commands.entity(unhindered.action).despawn();
}

/// It strikes as it decided.
fn strike(
    unhindered: On<Unhindered>,
    mut commands: Commands,
    actions: Query<(&ActionOf, &Strikes)>,
) -> Result {
    let (&ActionOf(actor), &Strikes(fights)) = actions.get(unhindered.action)?;
    commands.trigger(Strike { actor, fights });
    Ok(())
}

/// It retreats as it decided.
fn retreat_as_decided(
    unhindered: On<Unhindered>,
    mut commands: Commands,
    actions: Query<(&ActionOf, &Retreats)>,
) -> Result {
    let (&ActionOf(actor), &retreats) = actions.get(unhindered.action)?;
    commands.trigger(Retreat { actor, retreats });
    Ok(())
}

/// An actor runs or withdraws ([`retreat`]): as it decided, or as panic
/// sends it.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Retreat {
    #[event_target]
    pub actor: Entity,
    pub retreats: Retreats,
}

/// Run or withdraw, the action holding the fight for its pause: a monster
/// that cannot walk, or has moved this round, waits instead (retail falls
/// back on `WAIT` when the move fails).
pub(super) fn retreat(
    retreat: On<Retreat>,
    mut commands: Commands,
    actors: Query<(Option<&Walker>, Option<&close::Move>, Has<Moved>)>,
    places: Places,
    party: Single<Entity, With<Party>>,
    mut news: MessageWriter<CombatNews>,
) -> Result {
    let Retreat { actor, retreats } = *retreat;
    let (walker, moves, moved) = actors.get(actor)?;
    commands.trigger(Done { actor });
    commands.trigger(Shown { actor });
    if !walker.is_some_and(|w| w.speed > 0.0) || moved {
        commands.entity(actor).insert(Waited);
        commands.trigger(Exerted { actor, effort: Effort::Wait });
        return Ok(());
    }
    let effort = if retreats.conduct == Conduct::Flee { Effort::Run } else { Effort::Return };
    commands.trigger(Exerted { actor, effort });
    // A flight never charges.
    let metres = moves.map_or(0.0, |moves| moves.walking);
    let feet = places.of(actor).feet;
    let away = close::withdraws_to(feet, places.of(*party).eye, metres);
    commands.entity(actor).insert((
        Moved,
        super::approach::CombatMove {
            budget: metres,
            moved: 0.0,
            goal: super::approach::MoveGoal::Retreat(away),
        },
    ));
    if !retreats.quiet {
        news.write(match retreats.conduct {
            Conduct::Flee => CombatNews::Flees { monster: actor },
            _ => CombatNews::Withdraws { monster: actor },
        });
    }
    Ok(())
}

/// Hold back and rest: done for the round, without a pause.
fn hold(
    unhindered: On<Unhindered>,
    mut commands: Commands,
    actions: Query<&ActionOf, With<Holds>>,
) -> Result {
    let &ActionOf(actor) = actions.get(unhindered.action)?;
    commands.entity(actor).insert(Waited);
    commands.trigger(Done { actor });
    commands.trigger(Exerted { actor, effort: Effort::Wait });
    Ok(())
}

/// Guard the fellow chosen this round, taking blows in its ward's place up
/// to its attacks a round.
fn guard(
    unhindered: On<Unhindered>,
    mut commands: Commands,
    actions: Query<(&ActionOf, &Protects)>,
    actors: Query<&Tempo>,
) -> Result {
    let (&ActionOf(actor), &Protects(ward)) = actions.get(unhindered.action)?;
    let tempo = actors.get(actor)?;
    commands.trigger(Done { actor });
    commands.trigger(Shown { actor });
    commands.entity(actor).insert((
        Guarding(ward),
        Interceptions { taken: 0, allowed: tempo.attacks[WeaponSlot::Primary] },
    ));
    Ok(())
}

/// Make the special attack or breath (retail `ExecuteMonsterSpecialAttack`,
/// special-attack entry 30): a deed reaching whoever stands in its cone,
/// whatever their side, or a summoning calling its group up; it costs its
/// stamina, and a monster's cooldown starts.
#[allow(clippy::type_complexity)]
fn breathe(
    unhindered: On<Unhindered>,
    mut commands: Commands,
    actions: Query<(&ActionOf, &Breathes)>,
    mut vitals: Query<&mut Vitals>,
    fight: Single<(&mut Round, Option<&Combatants>), With<Battle>>,
    places: Places,
    obstacles: WorldObstacles,
    content: Res<Catalogs>,
    mut news: MessageWriter<CombatNews>,
) -> Result {
    let (&ActionOf(actor), &breathes) = actions.get(unhindered.action)?;
    let (mut round, combatants) = fight.into_inner();
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    commands.trigger(Done { actor });
    commands.trigger(Shown { actor });
    commands.entity(actor).insert(MadeSpecial);
    round.attacked(true);
    news.write(match breathes.special {
        Some(kind) => CombatNews::Special { monster: actor, kind },
        None => CombatNews::Breathed { actor },
    });
    vitals.get_mut(actor)?.tire(breathes.cost);
    if breathes.special.is_some() {
        commands.entity(actor).insert(SpecialCooldown(breathes.cooldown));
    }
    let from = places.of(actor);
    if let Some(species) = breathes.summons {
        commands.trigger(summon::Summon { summoner: actor, species });
        return Ok(());
    }
    let rows = Rows::Special(breathes.special.unwrap_or(crate::game::hits::specials::BREATH));
    let cone = content.spells[SpellId::SPECIAL_ATTACK_CONE].reach.metres();
    let toward = breathes.toward - from.eye;
    let mut victims = Vec::new();
    for &other in combatants.iter().filter(|&&other| other != actor) {
        let there = places.of(other);
        if !vitals.get(other)?.is_dead()
            && spell_reaches(&from, &there, cone, &obstacles)
            && in_cone(toward, there.feet - from.eye)
        {
            victims.push(other);
        }
    }
    let strike = effects::Strike::new(rows, breathes.strength, Some(actor), &content);
    // Watched before it lands, which it does as it is made.
    commands.spawn_empty().observe(tell_breath).insert(Deed::at(strike, victims));
    Ok(())
}

/// What a breath did is told as the fight's news: its deed's own observer.
fn tell_breath(landed: On<Landed>, mut news: MessageWriter<CombatNews>) {
    if !landed.effects.is_empty() {
        news.write(CombatNews::Effects(landed.effects.clone()));
    }
}

/// It casts, or uses an item's spell; an item no longer there refuses it.
fn cast(
    unhindered: On<Unhindered>,
    mut commands: Commands,
    actions: Query<(&ActionOf, &Casts)>,
    held: Items,
) -> Result {
    let (&ActionOf(actor), &Casts(choice)) = actions.get(unhindered.action)?;
    commands.trigger(Done { actor });
    commands.trigger(Shown { actor });
    match choice.source.spell_id(&held) {
        Some(spell) => commands.trigger(Cast { caster: actor, spell, choice, again: false }),
        None => commands.trigger(Refused { caster: actor, refusal: Refusal::NoItem }),
    }
    Ok(())
}

/// A member whose cast was refused, whoever refused it, defends instead
/// (retail `ExecuteCharacterAction`), the turn spent. A monster's cast has
/// no order to change.
pub(super) fn defend_instead(
    refused: On<Refused>,
    mut orders: Query<Option<&mut Ordered>>,
) -> Result {
    if let Some(mut ordered) = orders.get_mut(refused.caster)?
        && matches!(ordered.0, super::turn::Action::Magic(_))
    {
        ordered.0 = super::turn::Action::Defend;
    }
    Ok(())
}

pub(super) fn plugin(app: &mut App) {
    app.add_observer(nausea)
        .add_observer(web)
        .add_observer(blindness)
        .add_observer(insanity)
        .add_observer(fear)
        .add_observer(panic)
        .add_observer(lure)
        .add_observer(turncoat)
        .add_observer(tell)
        .add_observer(lose)
        .add_observer(break_free)
        .add_observer(turn_on_friends)
        .add_observer(run_in_panic)
        .add_observer(follow_lure)
        .add_observer(spend)
        .add_observer(retreat)
        .add_observer(defend_instead);
}
