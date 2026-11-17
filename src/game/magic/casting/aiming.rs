//! The steps of a cast that end in a deed (`docs/rules/casting.md` sections
//! 4 to 7): whom it is aimed at, the area its power spreads over, the roll,
//! a blind caster's aim gone astray, a backfire turned on the caster's side,
//! and its release. A utility spell's steps end in its work ([`Worked`]).
//! Each aim is an entity of [`crate::game::magic::aim`]; what follows its
//! taking is the observer set on it here.

use bevy::ecs::lifecycle::HookContext;
use bevy::ecs::world::DeferredWorld;
use bevy::prelude::*;

use super::{
    Again, Aimed, Allowed, Came, CastBy, CastSpell, Chosen, Failure, LandsBy, MagicNews, Mark, Pay,
    Power, Release, Report, Roll, Rolled, SeesBlind, Spread, Widens, Worked, refuse_cast,
};
use crate::content::characters::Realm;
use crate::content::spells::{SpellDefinition, SpellId};
use crate::game::actors::{CasterLevels, Places};
use crate::game::characters::{Condition, Conditions, Vitals};
use crate::game::combat::{Battle, Combatants};
use crate::game::data::Catalogs;
use crate::game::hits::Deed;
use crate::game::hits::definitions::Rows;
use crate::game::hits::effects::Strike;
use crate::game::items::Items;
use crate::game::magic::aim::{
    Against, AimOf, At, Origin, Radius, Reach, Spot, Take, Takes, Took, Victims, Whose,
};
use crate::game::magic::landed::{Leech, Spell};
use crate::game::magic::may::Refusal;
use crate::game::magic::reach::{Aim, blast_radius, flight};
use crate::game::magic::{Outcome, Strength, power, roll};
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::hostility::{Provocation, Standing};
use crate::game::world::identify::{GroupMember, GroupMembers, Leads};
use crate::game::world::level::InPlay;
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::Monster;
use crate::game::world::triggers::LevelState;

/// Its kind of target, a component of the cast inserted as it is made (a
/// potion or food taken by its user in a fight is at the caster). Each one's
/// hook sets the observers of the steps where kinds differ: whom its first
/// aim takes ([`Allowed`]), what its power spreads it over ([`Spread`]),
/// where it goes when it strays ([`Strays`]), what it falls on when it
/// backfires ([`Backfired`]), and whom it comes from and where it lands once
/// let go ([`Lets`]). What is the same for all is one observer.
#[derive(Component, Clone, Copy, Debug, Default)]
#[component(on_add = at_caster)]
pub struct AtCaster;

/// At a friend: one body, the caster or one friendly to them.
#[derive(Component, Clone, Copy, Debug, Default)]
#[component(on_add = at_friend)]
pub struct AtFriend;

/// At the party.
#[derive(Component, Clone, Copy, Debug, Default)]
#[component(on_add = at_party)]
pub struct AtParty;

/// At one foe.
#[derive(Component, Clone, Copy, Debug, Default)]
#[component(on_add = at_foe)]
pub struct AtFoe;

/// At a foe and the rest of its group.
#[derive(Component, Clone, Copy, Debug, Default)]
#[component(on_add = at_foe_group)]
pub struct AtFoeGroup;

/// In a cone toward a foe.
#[derive(Component, Clone, Copy, Debug, Default)]
#[component(on_add = at_cone)]
pub struct AtCone;

/// A blast at a foe's feet.
#[derive(Component, Clone, Copy, Debug, Default)]
#[component(on_add = at_blast)]
pub struct AtBlast;

/// At every foe reached.
#[derive(Component, Clone, Copy, Debug, Default)]
#[component(on_add = at_all_foes)]
pub struct AtAllFoes;

fn at_caster(mut world: DeferredWorld, context: HookContext) {
    let mut commands = world.commands();
    commands
        .entity(context.entity)
        .observe(aim_at_caster)
        .observe(spread_nothing)
        .observe(stray_nowhere)
        .observe(fall_on_caster)
        .observe(from_caster);
}

fn at_friend(mut world: DeferredWorld, context: HookContext) {
    let mut commands = world.commands();
    commands
        .entity(context.entity)
        .observe(aim_at_friend)
        .observe(spread_nothing)
        .observe(stray_at_one)
        .observe(fall_on_caster)
        .observe(from_aimed);
}

fn at_party(mut world: DeferredWorld, context: HookContext) {
    let mut commands = world.commands();
    commands
        .entity(context.entity)
        .observe(aim_at_party)
        .observe(widen)
        .observe(stray_nowhere)
        .observe(fall_on_fellows)
        .observe(from_caster);
}

fn at_foe(mut world: DeferredWorld, context: HookContext) {
    let mut commands = world.commands();
    commands
        .entity(context.entity)
        .observe(aim_at_foe)
        .observe(spread_nothing)
        .observe(stray_at_one)
        .observe(fall_on_caster)
        .observe(from_aimed);
}

fn at_foe_group(mut world: DeferredWorld, context: HookContext) {
    let mut commands = world.commands();
    commands
        .entity(context.entity)
        .observe(aim_at_foe)
        .observe(spread_over_group)
        .observe(stray_at_group)
        .observe(fall_on_group)
        .observe(from_leader);
}

fn at_cone(mut world: DeferredWorld, context: HookContext) {
    let mut commands = world.commands();
    commands
        .entity(context.entity)
        .observe(aim_at_foe)
        .observe(spread_over_cone)
        .observe(stray_cone)
        .observe(fall_on_fellows)
        .observe(from_spot);
}

fn at_blast(mut world: DeferredWorld, context: HookContext) {
    let mut commands = world.commands();
    commands
        .entity(context.entity)
        .observe(aim_at_foe)
        .observe(spread_over_blast)
        .observe(stray_blast)
        .observe(fall_blast)
        .observe(from_spot_to_spot);
}

fn at_all_foes(mut world: DeferredWorld, context: HookContext) {
    let mut commands = world.commands();
    commands
        .entity(context.entity)
        .observe(aim_at_all_foes)
        .observe(spread_nothing)
        .observe(stray_nowhere)
        .observe(fall_on_fellows)
        .observe(from_caster);
}

/// A spell at a spot, an item or a lock with no working of its own is one
/// the game does not resolve.
pub(super) fn unaimable(
    allowed: On<Allowed>,
    mut commands: Commands,
    casts: Query<&CastBy>,
) -> Result {
    let &CastBy(caster) = casts.get(allowed.cast)?;
    refuse_cast(&mut commands, (allowed.cast, caster), Refusal::Unsupported);
    Ok(())
}

/// Its first aim (section 5): whom `takes` takes of those the caster stands
/// against, within the spell's reach.
fn first_aim(
    commands: &mut Commands,
    cast: Entity,
    casts: &Query<(&CastBy, &CastSpell, &Chosen)>,
    takes: Takes,
) -> Result {
    let (&CastBy(caster), CastSpell(spell), &Chosen(choice)) = casts.get(cast)?;
    let made = (AimOf(cast), Whose(caster), At(choice.aim), Reach(spell.reach.metres()));
    let aim = commands.spawn((made, Against::FoesOf, takes)).observe(aimed).id();
    commands.trigger(Take { aim });
    Ok(())
}

/// At the caster.
fn aim_at_caster(
    allowed: On<Allowed>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, &Chosen)>,
) -> Result {
    first_aim(&mut commands, allowed.cast, &casts, Takes::Caster)
}

/// At the friend aimed at.
fn aim_at_friend(
    allowed: On<Allowed>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, &Chosen)>,
) -> Result {
    first_aim(&mut commands, allowed.cast, &casts, Takes::Friend)
}

/// At the party.
fn aim_at_party(
    allowed: On<Allowed>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, &Chosen)>,
) -> Result {
    first_aim(&mut commands, allowed.cast, &casts, Takes::Party)
}

/// At the foe that an enemy, group, cone or blast spell is aimed at, whose
/// area is spread once its power is known.
fn aim_at_foe(
    allowed: On<Allowed>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, &Chosen)>,
) -> Result {
    first_aim(&mut commands, allowed.cast, &casts, Takes::Foe)
}

/// At every foe reached.
fn aim_at_all_foes(
    allowed: On<Allowed>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, &Chosen)>,
) -> Result {
    first_aim(&mut commands, allowed.cast, &casts, Takes::Reached)
}

/// Whom it is aimed at is settled: none is no target, but for the caster or
/// the party.
fn aimed(
    took: On<Took>,
    mut commands: Commands,
    aims: Query<(&AimOf, &Victims, &Takes, Option<&Spot>)>,
    casts: Query<&CastBy>,
) -> Result {
    let (&AimOf(cast), Victims(victims), takes, spot) = aims.get(took.aim)?;
    let &CastBy(caster) = casts.get(cast)?;
    if victims.is_empty() && !matches!(takes, Takes::Caster | Takes::Party) {
        refuse_cast(&mut commands, (cast, caster), Refusal::NoTarget);
        return Ok(());
    }
    commands.entity(cast).insert(LandsBy(took.aim));
    if let Some(&spot) = spot {
        commands.entity(cast).insert(spot);
    }
    commands.trigger(Aimed { cast });
    Ok(())
}

/// One that spreads over nothing, a utility spell's too, goes to its roll.
pub(super) fn spread_nothing(spread: On<Spread>, mut commands: Commands) {
    commands.trigger(Roll { cast: spread.cast });
}

/// Its area (section 5) is taken in, and it lands by it.
fn spread_over(commands: &mut Commands, cast: Entity, area: impl Bundle) {
    let aim = commands.spawn((area, AimOf(cast))).observe(spread_over_taken).id();
    commands.trigger(Take { aim });
}

/// A group spell spreads over the rest of the foe's group.
fn spread_over_group(
    spread: On<Spread>,
    mut commands: Commands,
    casts: Query<&LandsBy>,
    aims: Query<&At>,
) -> Result {
    let &LandsBy(first) = casts.get(spread.cast)?;
    spread_over(&mut commands, spread.cast, (Takes::Group, *aims.get(first)?));
    Ok(())
}

/// A cone spreads over those in it, from the caster's eye toward the foe.
fn spread_over_cone(
    spread: On<Spread>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, Option<&Spot>)>,
    places: Places,
) -> Result {
    let (&CastBy(caster), CastSpell(spell), spot) = casts.get(spread.cast)?;
    let at = places.of(caster);
    let foe = spot.map_or(at.feet, |spot| spot.0);
    let reach = Reach(spell.reach.metres());
    let cone = (Takes::Cone, Whose(caster), Origin(at.eye), Spot(foe), reach, Against::Everyone);
    spread_over(&mut commands, spread.cast, cone);
    Ok(())
}

/// A blast spreads over those of the caster's foes' side within its radius
/// of the foe's feet.
fn spread_over_blast(
    spread: On<Spread>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, &Power, Option<&Spot>)>,
    standings: Query<&Standing>,
    places: Places,
) -> Result {
    let (&CastBy(caster), CastSpell(spell), &Power(power), spot) = casts.get(spread.cast)?;
    let side = standings.get(caster)?.stance;
    let foe = spot.map_or(places.of(caster).feet, |spot| spot.0);
    let radius = Radius(blast_radius(spell, power));
    let blast = (Takes::Blast, Spot(foe), Origin(foe), radius, Against::Opponents(side));
    spread_over(&mut commands, spread.cast, blast);
    Ok(())
}

/// A member's spell at the party spreads over the monsters on the party's
/// side within its radius and the party's own of the party; any other goes
/// to its roll.
fn widen(
    spread: On<Spread>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, &Power, Has<Widens>)>,
    standings: Query<&Standing>,
    places: Places,
) -> Result {
    let cast = spread.cast;
    let (&CastBy(caster), CastSpell(spell), &Power(power), widens) = casts.get(cast)?;
    let radius = blast_radius(spell, power);
    if !widens || radius <= 0.0 {
        commands.trigger(Roll { cast });
        return Ok(());
    }
    let side = standings.get(caster)?.stance;
    let at = places.of(caster);
    let widened = (
        Spot(at.centre),
        Origin(at.eye),
        Radius(radius + crate::game::party::body::RADIUS),
        Against::Opponents(side).reversed(),
    );
    let aim = commands.spawn((AimOf(cast), Takes::Blast, widened)).observe(widened_to_allies).id();
    commands.trigger(Take { aim });
    Ok(())
}

/// It lands by the area it spread over.
fn spread_over_taken(took: On<Took>, mut commands: Commands, aims: Query<&AimOf>) -> Result {
    let &AimOf(cast) = aims.get(took.aim)?;
    commands.entity(cast).insert(LandsBy(took.aim));
    commands.trigger(Roll { cast });
    Ok(())
}

/// The monsters on the party's side about it join the party it lands on.
fn widened_to_allies(
    took: On<Took>,
    mut commands: Commands,
    aims: Query<(&AimOf, &Victims)>,
    casts: Query<&LandsBy>,
    party: Query<&Victims>,
    roster: Single<&PartyRoster>,
) -> Result {
    let (&AimOf(cast), Victims(allies)) = aims.get(took.aim)?;
    let &LandsBy(first) = casts.get(cast)?;
    let mut victims = party.get(first)?.0.clone();
    victims.extend(allies.iter().filter(|ally| !roster.contains(*ally)));
    commands.entity(first).insert(Victims(victims));
    commands.trigger(Roll { cast });
    Ok(())
}

/// The roll to cast (section 4): against its failure chance, but Fire under
/// water is quenched. What follows the roll whatever it was hears
/// [`Rolled`]; a deed's own steps hear how it came out.
pub(super) fn roll(
    roll: On<Roll>,
    mut commands: Commands,
    casts: Query<(&CastSpell, &Failure)>,
    level: Option<Single<&LevelState, With<InPlay>>>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let cast = roll.cast;
    let (CastSpell(spell), &Failure { chance, quiet }) = casts.get(cast)?;
    let underwater = level.is_some_and(|state| state.camera_sway);
    let quenched = spell.realm == Some(Realm::Fire) && underwater;
    let came = roll::outcome(&mut rng, chance, quiet, quenched, spell.can_backfire());
    commands.entity(cast).insert(Came(came));
    commands.trigger(Rolled { cast });
    match came {
        Outcome::Cast => commands.trigger(CameOff { cast }),
        Outcome::Backfired => commands.trigger(Backfired { cast }),
        Outcome::Fizzled => commands.trigger(Fizzled { cast }),
    }
    Ok(())
}

/// A cast that came off.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct CameOff {
    #[event_target]
    pub cast: Entity,
}

/// A cast that backfired.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Backfired {
    #[event_target]
    pub cast: Entity,
}

/// A cast that fizzled.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Fizzled {
    #[event_target]
    pub cast: Entity,
}

/// A utility spell is paid for whatever the roll.
pub(super) fn rolled_work(rolled: On<Rolled>, mut commands: Commands) {
    commands.trigger(Pay { cast: rolled.cast });
}

/// A member's spell against enemies provokes those it reaches, whatever the
/// roll (retail `MakeTargetGroupHostile` as the action starts): set on the
/// cast by [`Provokes`].
pub(super) fn provoke(
    rolled: On<Rolled>,
    mut commands: Commands,
    casts: Query<&LandsBy>,
    victims: Query<&Victims>,
) -> Result {
    let &LandsBy(lands) = casts.get(rolled.cast)?;
    for &monster in &victims.get(lands)?.0 {
        commands.trigger(Provocation { monster });
    }
    Ok(())
}

/// A deed that fizzled is paid for all the same.
pub(super) fn fizzled(fizzled: On<Fizzled>, mut commands: Commands) {
    commands.trigger(Pay { cast: fizzled.cast });
}

/// A blind caster's spell that came off may go astray (retail
/// `CheckSpellBackfire` and its caller), unless it is aimed at the caster, of
/// the Mental realm, Rocket Blast or an automatic recast: one time in five,
/// but one who sees blind escapes it first. Where it goes is its kind's
/// ([`Strays`]); one that does not stray is paid for.
pub(super) fn go_astray(
    came_off: On<CameOff>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, &Chosen, &SeesBlind, Has<Again>)>,
    casters: Query<&Conditions>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let cast = came_off.cast;
    let (&CastBy(caster), CastSpell(spell), &Chosen(choice), &SeesBlind(sees), again) =
        casts.get(cast)?;
    let guarded = again
        || choice.aim.body() == Some(caster)
        || spell.realm == Some(Realm::Mental)
        || spell.id == SpellId::ROCKET_BLAST;
    let strays = !guarded
        && casters.get(caster)?.has(Condition::Blind)
        && rng.below(100) >= sees
        && rng.below(100) < 20;
    match strays {
        true => commands.trigger(Strays { cast }),
        false => commands.trigger(Pay { cast }),
    }
    Ok(())
}

/// A blind caster's spell goes astray: where to is its kind's.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Strays {
    #[event_target]
    pub cast: Entity,
}

/// One aimed at the caster, the party or every foe goes nowhere else: it is
/// paid for.
fn stray_nowhere(strays: On<Strays>, mut commands: Commands) {
    commands.trigger(Pay { cast: strays.cast });
}

/// It goes astray by this aim: the caster is told, and it lands where it
/// went.
fn astray(
    commands: &mut Commands,
    cast: Entity,
    caster: Entity,
    aim: impl Bundle,
    news: &mut MessageWriter<MagicNews>,
) {
    let aim = commands.spawn((aim, AimOf(cast))).observe(went_astray).id();
    news.write(MagicNews::Astray { caster });
    commands.trigger(Take { aim });
}

/// One at a body or a group goes at another in the fight at random, members
/// and living monsters alike, never the caster (`PickBackfireTarget`): the
/// members by seat, then the monsters as they were placed. With nobody else
/// it is paid for where it was.
#[allow(clippy::type_complexity)]
fn stray_at_another(
    commands: &mut Commands,
    (cast, caster): (Entity, Entity),
    takes: Takes,
    (roster, combatants, living): (
        &PartyRoster,
        Option<&Combatants>,
        &Query<(&Vitals, Option<&Monster>)>,
    ),
    places: &Places,
    rng: &mut GameRng,
    news: &mut MessageWriter<MagicNews>,
) {
    let members = roster.iter().filter(|&member| member != caster);
    let mut monsters: Vec<(i64, Entity)> = combatants
        .iter()
        .flat_map(|fight| fight.entities().iter().copied())
        .filter(|&body| body != caster && !roster.contains(&body))
        .filter_map(|body| {
            let (vitals, monster) = living.get(body).ok()?;
            (!vitals.is_dead()).then(|| (monster.map_or(i64::MAX, |m| i64::from(m.location)), body))
        })
        .collect();
    monsters.sort_by_key(|&(location, _)| location);
    let others: Vec<Entity> = members.chain(monsters.into_iter().map(|(_, body)| body)).collect();
    if others.is_empty() {
        commands.trigger(Pay { cast });
        return;
    }
    let body = others[rng.below(others.len() as u32) as usize];
    astray(commands, cast, caster, (takes, At(Aim::Body(body)), Spot(places.of(body).feet)), news);
}

/// One at a body goes at another.
#[allow(clippy::type_complexity)]
fn stray_at_one(
    strays: On<Strays>,
    mut commands: Commands,
    casts: Query<&CastBy>,
    (roster, combatants, living): (
        Single<&PartyRoster>,
        Option<Single<&Combatants, With<Battle>>>,
        Query<(&Vitals, Option<&Monster>)>,
    ),
    places: Places,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<MagicNews>,
) -> Result {
    let &CastBy(caster) = casts.get(strays.cast)?;
    let combatants = combatants.as_deref().copied();
    let fighting = (*roster, combatants, &living);
    stray_at_another(
        &mut commands,
        (strays.cast, caster),
        Takes::One,
        fighting,
        &places,
        &mut rng,
        &mut news,
    );
    Ok(())
}

/// One at a group goes at another's group.
#[allow(clippy::type_complexity)]
fn stray_at_group(
    strays: On<Strays>,
    mut commands: Commands,
    casts: Query<&CastBy>,
    (roster, combatants, living): (
        Single<&PartyRoster>,
        Option<Single<&Combatants, With<Battle>>>,
        Query<(&Vitals, Option<&Monster>)>,
    ),
    places: Places,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<MagicNews>,
) -> Result {
    let &CastBy(caster) = casts.get(strays.cast)?;
    let combatants = combatants.as_deref().copied();
    let fighting = (*roster, combatants, &living);
    stray_at_another(
        &mut commands,
        (strays.cast, caster),
        Takes::Group,
        fighting,
        &places,
        &mut rng,
        &mut news,
    );
    Ok(())
}

/// A cone goes off toward a spot scattered within its reach.
fn stray_cone(
    strays: On<Strays>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell)>,
    (places, obstacles): (Places, WorldObstacles),
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<MagicNews>,
) -> Result {
    let (&CastBy(caster), CastSpell(spell)) = casts.get(strays.cast)?;
    let at = places.of(caster);
    let reach = Reach(spell.reach.metres());
    let spot = scatter(&mut rng, at.feet, reach.0, &obstacles);
    let cone = (Takes::Cone, Whose(caster), Origin(at.eye), Spot(spot), reach, Against::Everyone);
    astray(&mut commands, strays.cast, caster, cone, &mut news);
    Ok(())
}

/// A blast goes off at a spot scattered within its reach.
fn stray_blast(
    strays: On<Strays>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, &Power)>,
    standings: Query<&Standing>,
    (places, obstacles): (Places, WorldObstacles),
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<MagicNews>,
) -> Result {
    let (&CastBy(caster), CastSpell(spell), &Power(power)) = casts.get(strays.cast)?;
    let side = standings.get(caster)?.stance;
    let spot = scatter(&mut rng, places.of(caster).feet, spell.reach.metres(), &obstacles);
    let radius = Radius(blast_radius(spell, power));
    let blast = (Takes::Blast, Spot(spot), Origin(spot), radius, Against::Opponents(side));
    astray(&mut commands, strays.cast, caster, blast, &mut news);
    Ok(())
}

/// It lands where it went astray.
fn went_astray(
    took: On<Took>,
    mut commands: Commands,
    aims: Query<(&AimOf, Option<&Spot>)>,
) -> Result {
    let (&AimOf(cast), spot) = aims.get(took.aim)?;
    commands.entity(cast).insert(LandsBy(took.aim));
    if let Some(&spot) = spot {
        commands.entity(cast).insert(spot);
    }
    commands.trigger(Pay { cast });
    Ok(())
}

/// What a backfire falls on (retail `PrepareSpellTarget`,
/// `RedirectBackfiredSpellTarget`), by this aim: it lands on whom it turned
/// on.
fn falls_on(commands: &mut Commands, cast: Entity, aim: impl Bundle) {
    let aim = commands.spawn((aim, AimOf(cast))).observe(lands_by).id();
    commands.trigger(Take { aim });
}

/// A spell at one falls on the caster.
fn fall_on_caster(
    backfired: On<Backfired>,
    mut commands: Commands,
    casts: Query<&CastBy>,
) -> Result {
    let &CastBy(caster) = casts.get(backfired.cast)?;
    falls_on(&mut commands, backfired.cast, (Takes::Caster, Whose(caster)));
    Ok(())
}

/// A group spell falls on the caster's whole party or group, however far.
fn fall_on_group(
    backfired: On<Backfired>,
    mut commands: Commands,
    casts: Query<&CastBy>,
) -> Result {
    let &CastBy(caster) = casts.get(backfired.cast)?;
    falls_on(&mut commands, backfired.cast, (Takes::Group, At(Aim::Body(caster))));
    Ok(())
}

/// A blast falls on the caster's side within its radius of the caster.
fn fall_blast(
    backfired: On<Backfired>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, &Power)>,
    standings: Query<&Standing>,
    places: Places,
) -> Result {
    let (&CastBy(caster), CastSpell(spell), &Power(power)) = casts.get(backfired.cast)?;
    let feet = places.of(caster).feet;
    let side = Against::Opponents(standings.get(caster)?.stance).reversed();
    let blast = (Takes::Blast, Spot(feet), Origin(feet), Radius(blast_radius(spell, power)), side);
    falls_on(&mut commands, backfired.cast, blast);
    Ok(())
}

/// The rest fall on the caster's side the caster reaches, itself among them.
fn fall_on_fellows(
    backfired: On<Backfired>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell)>,
) -> Result {
    let (&CastBy(caster), CastSpell(spell)) = casts.get(backfired.cast)?;
    let fellows = (Takes::Reached, Whose(caster), Reach(spell.reach.metres()), Against::FellowsOf);
    falls_on(&mut commands, backfired.cast, fellows);
    Ok(())
}

/// It lands on whom it turned on.
fn lands_by(took: On<Took>, mut commands: Commands, aims: Query<&AimOf>) -> Result {
    let &AimOf(cast) = aims.get(took.aim)?;
    commands.entity(cast).insert(LandsBy(took.aim));
    commands.trigger(Pay { cast });
    Ok(())
}

/// A spot at random within `reach` of `origin` (retail
/// `ScatterSpellPointTarget`): up to five tries for one that stands on
/// ground, each stopped short by what is solid in the way; one too far is
/// drawn back to the reach.
fn scatter(rng: &mut GameRng, origin: Vec3, reach: f32, obstacles: &WorldObstacles) -> Vec3 {
    let mut spot = origin;
    for _ in 0..5 {
        let across = |rng: &mut GameRng| (rng.below(2001) as f32 - 1000.0) / 1000.0 * reach;
        let (x, z) = (across(rng), across(rng));
        let up = rng.below(1001) as f32 / 1000.0 * reach;
        let mut point = origin + Vec3::new(x, up, z);
        if let Ok(way) = Dir3::new(point - origin)
            && let Some(hit) = obstacles.first_hit(origin, way, origin.distance(point))
        {
            point = origin + *way * hit.distance;
        }
        spot = point;
        if let Some(ground) = obstacles.ground_below(point, up + reach) {
            spot = ground;
            break;
        }
    }
    if spot.distance(origin) > reach {
        spot = origin + (spot - origin).normalize_or_zero() * reach;
    }
    spot
}

/// What a cast did, for the player: the item it came from by name.
fn report(
    (caster, spell, choice): (Entity, &SpellDefinition, super::MagicChoice),
    (power, came): (u32, Outcome),
    held: &Items,
    content: &Catalogs,
) -> Report {
    let item = held.id(choice.castable).map(|id| content.items[id].name.clone());
    Report { caster, spell: spell.id, power, item, outcome: came, effects: Vec::new() }
}

/// A deed of a cast has been made, not yet let go: what its means puts
/// beside it (practice, an auto recast) is put there now.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Released {
    #[event_target]
    pub cast: Entity,
    pub deed: Entity,
    /// Its missiles are on their way, which counts for practice whatever
    /// they do.
    pub flies: bool,
}

/// A cast that fizzled has been paid for and told: it is spent.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Spent {
    #[event_target]
    pub cast: Entity,
}

/// A utility spell does its own thing, whatever the roll.
pub(super) fn release_work(
    release: On<Release>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, &Chosen, &Power, &Came, &Strength, &Mark)>,
    held: Items,
    content: Res<Catalogs>,
) -> Result {
    let cast = release.cast;
    let (
        &CastBy(caster),
        CastSpell(spell),
        &Chosen(choice),
        &Power(power),
        &Came(came),
        &strength,
        &Mark(marked),
    ) = casts.get(cast)?;
    let report = report((caster, spell, choice), (power, came), &held, &content);
    commands.trigger(Worked { cast, caster, report, strength, marked });
    commands.entity(cast).despawn();
    Ok(())
}

/// A cast is let go (sections 5 to 7), and told. A fizzle leaves only its
/// report. One that came off or backfired goes on to be a deed: whom it
/// comes from and where it lands are its kind's ([`Lets`]).
pub(super) fn release(
    release: On<Release>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, &Chosen, &Power, &Came)>,
    held: Items,
    content: Res<Catalogs>,
    mut news: MessageWriter<MagicNews>,
) -> Result {
    let cast = release.cast;
    let (&CastBy(caster), CastSpell(spell), &Chosen(choice), &Power(power), &Came(came)) =
        casts.get(cast)?;
    news.write(MagicNews::Done(report((caster, spell, choice), (power, came), &held, &content)));
    if came == Outcome::Fizzled {
        commands.trigger(Spent { cast });
        commands.entity(cast).despawn();
        return Ok(());
    }
    commands.trigger(Lets { cast });
    Ok(())
}

/// A cast that came off or backfired is let go: its kind says whom it comes
/// from and where it lands ([`LetGo`]).
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Lets {
    #[event_target]
    pub cast: Entity,
}

/// Whom a cast let go comes from (whose side and level it carries, which a
/// backfire is as strong as) and from where; what it was aimed at; and the
/// spot its missiles fly to, if not to each victim's feet.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct LetGo {
    #[event_target]
    pub cast: Entity,
    pub owner: Entity,
    pub from: Vec3,
    pub aim: Aim,
    pub to: Option<Vec3>,
}

/// It comes from the caster, at what was aimed at.
fn from_caster(
    lets: On<Lets>,
    mut commands: Commands,
    casts: Query<(&CastBy, &Chosen)>,
    places: Places,
) -> Result {
    let (&CastBy(caster), &Chosen(choice)) = casts.get(lets.cast)?;
    let from = places.of(caster).eye;
    commands.trigger(LetGo { cast: lets.cast, owner: caster, from, aim: choice.aim, to: None });
    Ok(())
}

/// A backfire of a spell at a body comes from this one of them, from its
/// eye; anything else from the caster.
fn from_body(
    commands: &mut Commands,
    (cast, caster): (Entity, Entity),
    (aim, came): (Aim, Outcome),
    owner: impl Fn(Entity) -> Entity,
    places: &Places,
) {
    let owner = match (came, aim) {
        (Outcome::Backfired, Aim::Body(aimed)) => owner(aimed),
        _ => caster,
    };
    commands.trigger(LetGo { cast, owner, from: places.of(owner).eye, aim, to: None });
}

/// A backfire of a spell at one comes from the body aimed at.
fn from_aimed(
    lets: On<Lets>,
    mut commands: Commands,
    casts: Query<(&CastBy, &Chosen, &Came)>,
    places: Places,
) -> Result {
    let (&CastBy(caster), &Chosen(choice), &Came(came)) = casts.get(lets.cast)?;
    from_body(&mut commands, (lets.cast, caster), (choice.aim, came), |aimed| aimed, &places);
    Ok(())
}

/// A backfire of a group spell comes from the group's leader (`Leads`).
#[allow(clippy::type_complexity)]
fn from_leader(
    lets: On<Lets>,
    mut commands: Commands,
    casts: Query<(&CastBy, &Chosen, &Came)>,
    grouping: (Query<&GroupMember>, Query<&GroupMembers>, Query<(), With<Leads>>),
    places: Places,
) -> Result {
    let (&CastBy(caster), &Chosen(choice), &Came(came)) = casts.get(lets.cast)?;
    let leader = |body: Entity| {
        grouping
            .0
            .get(body)
            .and_then(|member| grouping.1.get(member.0))
            .ok()
            .and_then(|members| members.iter().find(|&mate| grouping.2.contains(mate)))
            .unwrap_or(body)
    };
    from_body(&mut commands, (lets.cast, caster), (choice.aim, came), leader, &places);
    Ok(())
}

/// A cone or a blast is aimed at the spot it went off toward, and a backfire
/// of it comes from there: from the caster.
fn from_area(
    commands: &mut Commands,
    (cast, caster): (Entity, Entity),
    (aim, came): (Aim, Outcome),
    spot: Option<Vec3>,
    eye: Vec3,
    lands_at_spot: bool,
) {
    let from = match came {
        Outcome::Backfired => spot.unwrap_or(eye),
        _ => eye,
    };
    let aim = spot.map_or(aim, Aim::Point);
    let to = spot.filter(|_| lands_at_spot);
    commands.trigger(LetGo { cast, owner: caster, from, aim, to });
}

/// A cone.
fn from_spot(
    lets: On<Lets>,
    mut commands: Commands,
    casts: Query<(&CastBy, &Chosen, &Came, Option<&Spot>)>,
    places: Places,
) -> Result {
    let (&CastBy(caster), &Chosen(choice), &Came(came), spot) = casts.get(lets.cast)?;
    let (spot, eye) = (spot.map(|spot| spot.0), places.of(caster).eye);
    from_area(&mut commands, (lets.cast, caster), (choice.aim, came), spot, eye, false);
    Ok(())
}

/// A blast, whose missiles fly to the spot it goes off at.
fn from_spot_to_spot(
    lets: On<Lets>,
    mut commands: Commands,
    casts: Query<(&CastBy, &Chosen, &Came, Option<&Spot>)>,
    places: Places,
) -> Result {
    let (&CastBy(caster), &Chosen(choice), &Came(came), spot) = casts.get(lets.cast)?;
    let (spot, eye) = (spot.map(|spot| spot.0), places.of(caster).eye);
    from_area(&mut commands, (lets.cast, caster), (choice.aim, came), spot, eye, true);
    Ok(())
}

/// A cast let go is a deed at those it lands on, flying to each if the spell
/// is a missile and did not backfire, with what follows it beside it: a
/// drain's share, and what its means puts there ([`Released`]). A backfire
/// is as strong as the one it comes from.
#[allow(clippy::type_complexity)]
fn let_go(
    let_go: On<LetGo>,
    mut commands: Commands,
    casts: Query<(&CastBy, &CastSpell, &Power, &Came, &Strength, &LandsBy)>,
    victims: Query<&Victims>,
    owners: Query<&CasterLevels>,
    places: Places,
    content: Res<Catalogs>,
) -> Result {
    let LetGo { cast, owner, from, aim, to } = *let_go;
    let (&CastBy(caster), CastSpell(spell), &Power(power), &Came(came), &strength, &LandsBy(lands)) =
        casts.get(cast)?;
    let backfired = came == Outcome::Backfired;
    let victims = victims.get(lands)?.0.clone();
    // A backfire is as strong as the one it comes from.
    let strength = match backfired {
        true => {
            let figure = owners.get(owner).map_or(0, |owned| owned.caster_level(spell));
            power::strength(figure, power, strength.boost, spell)
        }
        false => strength,
    };
    let row = crate::game::hits::definitions::row(spell).ok_or("a released spell has a row")?;
    // Turned on its caster's side it gives nothing back when it follows rows
    // of its own.
    let (rows, leeches) = match backfired {
        true if row.backfires_apart(spell) => (Rows::Backfire(spell.id), None),
        true => (Rows::Backfire(spell.id), row.leeches),
        false => (Rows::Spell(spell.id), row.leeches),
    };
    let flies = spell.missile.is_some() && !backfired && !victims.is_empty();
    let start = places.of(caster).eye;
    let arrivals: Vec<(Entity, Option<f32>)> = victims
        .iter()
        .map(|&victim| {
            let to = to.unwrap_or_else(|| places.of(victim).feet);
            (victim, flies.then(|| flight(start, to)))
        })
        .collect();
    let strike = Strike::new(rows, strength, Some(caster), &content);
    let made = Spell {
        id: spell.id,
        caster,
        backfired,
        reflected: false,
        owner: Some(owner),
        from,
        aim,
        victims,
    };
    let deed = commands.spawn(made).id();
    if let Some(gives) = leeches {
        commands.entity(deed).insert(Leech { caster: owner, gives });
    }
    // What follows it is put beside it before it is let go: it lands as it
    // is made.
    commands.trigger(Released { cast, deed, flies });
    commands.entity(deed).insert(Deed::flying(strike, arrivals));
    commands.entity(cast).despawn();
    Ok(())
}

pub(super) fn plugin(app: &mut App) {
    app.add_observer(roll).add_observer(let_go);
}
