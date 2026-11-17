//! Casting a learned spell, using an item's spell, or a monster's own spell
//! (`docs/rules/casting.md`, "How this is built"). An accepted cast
//! ([`Cast`]) becomes an entity (`CastBy` its caster) carrying the spell and
//! what it is cast with; it goes through the page's steps as events, each
//! heard by the observer of what that step reads: whether it may be cast
//! ([`means`]), whom it reaches ([`aiming`]), its power, the roll, what it
//! costs, and its release as a deed that lands ([`crate::game::hits`]) or as
//! a utility spell's own work ([`Worked`]). It is gone once released or
//! refused.
//!
//! Choosing never spends anything. A refused cast spends nothing; one that is
//! rolled pays even when it fizzles or backfires. Its deed keeps its own copy
//! of the spell and victims, so using the last potion still heals, and a
//! missile in flight still lands after its caster falls.

mod aiming;
mod means;
use bevy::prelude::*;

pub use self::means::{goes_again, resolvable, usable_item, usable_items};
pub use super::aim::reaches;
use super::may::Refusal;
use super::reach::{Aim, Marked};
use super::{Outcome, Strength};
use crate::content::spells::{SpellDefinition, SpellId, Working};
use crate::game::hits::Effect;

/// An accepted magic action: what it is cast with (a castable: a learned
/// spell, an item, a monster's slot), the power asked for, and the aim,
/// nothing resolved yet. A learned spell's power is asked for, or
/// [`power::AUTO`], and clamped to what the caster can afford when it runs;
/// an item casts at its own; a monster's slot at the power it chose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MagicChoice {
    pub castable: Entity,
    pub power: u32,
    pub aim: Aim,
}

/// What became of a magic action, for whoever tells the player.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum MagicNews {
    Done(Report),
    /// A blind caster's spell went astray.
    Astray {
        caster: Entity,
    },
    /// What a deed of magic did, once all of it has landed.
    Landed {
        effects: Vec<Effect>,
    },
    Refused {
        caster: Entity,
        refusal: Refusal,
    },
}

/// A cast was refused, by whoever checked it: the caster is told
/// ([`MagicNews::Refused`]), and a member who was ordered to cast defends
/// instead.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Refused {
    #[event_target]
    pub caster: Entity,
    pub refusal: Refusal,
}

/// The caster is told their cast was refused.
pub fn refuse(refused: On<Refused>, mut news: MessageWriter<MagicNews>) {
    news.write(MagicNews::Refused { caster: refused.caster, refusal: refused.refusal });
}

/// What a cast did.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub caster: Entity,
    pub spell: SpellId,
    pub power: u32,
    /// The item it came from, already spent.
    pub item: Option<String>,
    pub outcome: Outcome,
    /// What a utility spell did; what a deed does is told as it lands
    /// ([`MagicNews::Landed`]).
    pub effects: Vec<Effect>,
}

/// A cast's practice, owed once its deed has landed: its caster practises
/// these skills if the cast affected anyone (retail `SpellAffectedTarget`),
/// or whatever came of it once a missile is on its way.
#[derive(Component, Clone, Debug)]
pub struct Practice {
    pub caster: Entity,
    pub skills: Vec<(crate::content::characters::Skill, u32)>,
    pub sure: bool,
}

/// An auto recast (retail's "repeat" power): once it has landed, its caster
/// casts again while its victims are not whole and stamina and points last.
#[derive(Component, Clone, Debug)]
pub struct Recast {
    pub caster: Entity,
    pub choice: MagicChoice,
    pub victims: Vec<Entity>,
    /// It goes on while a victim is short of stamina rather than of health.
    pub watches_stamina: bool,
}

/// An accepted cast: the spell, and what the caster chose. [`cast`] carries it
/// out or refuses it.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Cast {
    #[event_target]
    pub caster: Entity,
    pub choice: MagicChoice,
    /// An auto cast that fizzled, going again.
    pub again: bool,
}

/// A caster pays for a cast: spell points from a realm, and stamina.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Pays {
    #[event_target]
    pub caster: Entity,
    pub points: Option<(crate::content::characters::Realm, u32)>,
    pub stamina: u32,
}

pub fn pay(
    pays: On<Pays>,
    mut casters: Query<(
        &mut crate::game::characters::Vitals,
        Option<&mut crate::game::magic::SpellPoints>,
    )>,
) -> Result {
    let (mut vitals, pool) = casters.get_mut(pays.caster)?;
    if let (Some((realm, amount)), Some(mut pool)) = (pays.points, pool) {
        pool.spend(realm, amount);
    }
    vitals.tire(pays.stamina);
    Ok(())
}

/// A utility spell has been paid for and rolled: it now does what it is for
/// (`docs/rules/casting.md` section 8). Its working's own observer, set on
/// the cast as it is made, answers.
#[derive(EntityEvent, Clone, Debug)]
pub struct Worked {
    #[event_target]
    pub cast: Entity,
    pub caster: Entity,
    pub report: Report,
    pub strength: Strength,
    pub marked: Marked,
}

/// A utility spell's working has found what it is cast at, or found nothing
/// it may be: the cast goes on to its power, or is refused.
pub fn marked(
    commands: &mut Commands,
    (cast, caster): (Entity, Entity),
    mark: Result<Marked, Refusal>,
) {
    match mark {
        Ok(marked) => {
            commands.entity(cast).insert(Mark(marked));
            commands.trigger(Aimed { cast });
        }
        Err(refusal) => refuse_cast(commands, (cast, caster), refusal),
    }
}

/// A working's observers: what it may be cast at, and what it does.
fn works(made: &mut EntityCommands, working: Working) {
    use super::{dialogue, pointcasts, portals, utilities};
    use crate::game::world::interaction;
    match working {
        Working::Identify => made.observe(utilities::mark).observe(utilities::identify),
        Working::SetPortal => made.observe(portals::mark_here).observe(portals::set),
        Working::ReturnToPortal => made.observe(portals::mark_saved).observe(portals::go_back),
        Working::Charm | Working::Mindread => made.observe(dialogue::mark).observe(dialogue::cast),
        Working::Lure => made.observe(pointcasts::mark_lure).observe(pointcasts::lure),
        Working::Summon => made.observe(pointcasts::mark_summon).observe(pointcasts::summon),
        Working::Knock | Working::DivineTrap => {
            made.observe(interaction::mark).observe(interaction::cast_at)
        }
    };
}

/// The caster of a cast under way.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = Casting)]
pub struct CastBy(pub Entity);

/// The casts an actor has under way; they go with it.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = CastBy, linked_spawn)]
pub struct Casting(Vec<Entity>);

/// The spell cast, as content defines it; its hook sets the steps that
/// follow from what the spell is.
#[derive(Component, Clone, Debug)]
#[component(on_add = spelled)]
pub struct CastSpell(pub SpellDefinition);

/// In a fight, a potion or food is taken by its user: it is at them.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct TakenByUser;

/// What the caster chose: what it is cast with and what it is aimed at.
#[derive(Component, Clone, Copy, Debug)]
pub struct Chosen(pub MagicChoice);

/// An auto cast that fizzled, going again: it provokes nobody anew and does
/// not go astray.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Again;

/// The power it goes off at.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Power(pub u32);

/// How likely the roll to cast fails, in percent, and whether a failure
/// passes without notice.
#[derive(Component, Clone, Copy, Debug)]
pub struct Failure {
    pub chance: u32,
    pub quiet: bool,
}

/// How likely a blind caster's aim holds anyway, in percent.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct SeesBlind(pub u32);

/// The aim the cast lands by.
#[derive(Component, Clone, Copy, Debug)]
pub struct LandsBy(pub Entity);

/// How the roll to cast came out.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Came(pub Outcome);

/// What a utility spell is aimed at.
#[derive(Component, Clone, Copy, Debug)]
pub struct Mark(pub Marked);

/// A member's spell at the party also reaches the monsters on the party's
/// side about it.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Widens;

/// A member's spell against enemies provokes whom it reaches
/// ([`aiming::provoke`], set by its hook).
#[derive(Component, Clone, Copy, Debug, Default)]
#[component(on_add = provokes)]
pub struct Provokes;

fn provokes(
    mut world: bevy::ecs::world::DeferredWorld,
    context: bevy::ecs::lifecycle::HookContext,
) {
    world.commands().entity(context.entity).observe(aiming::provoke);
}

/// Ask whether it may be cast.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Ask {
    #[event_target]
    pub cast: Entity,
}

/// It may be cast: aim it.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Allowed {
    #[event_target]
    pub cast: Entity,
}

/// It has its aim: settle its power.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Aimed {
    #[event_target]
    pub cast: Entity,
}

/// It has its power: spread it over whom an area takes.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Spread {
    #[event_target]
    pub cast: Entity,
}

/// Roll it.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Roll {
    #[event_target]
    pub cast: Entity,
}

/// It has been rolled: what follows the roll.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Rolled {
    #[event_target]
    pub cast: Entity,
}

/// Whom it falls on is settled: pay for it.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Pay {
    #[event_target]
    pub cast: Entity,
}

/// It is paid for: let it go.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Release {
    #[event_target]
    pub cast: Entity,
}

/// A cast is refused: the caster is told, and it is gone.
fn refuse_cast(commands: &mut Commands, (cast, caster): (Entity, Entity), refusal: Refusal) {
    commands.trigger(Refused { caster, refusal });
    commands.entity(cast).despawn();
}

/// An accepted cast becomes an entity of its caster, with the spell read
/// from content and what it is cast with, and is asked whether it may be
/// made. An auto cast that fizzled goes again only while the caster's
/// stamina and points last. In a fight a potion or food is taken by its
/// user. What the spell is then decides the rest of its steps
/// ([`CastSpell`]'s hook).
#[allow(clippy::type_complexity)]
fn begin(
    cast: On<Cast>,
    pools: Query<(&crate::game::characters::Vitals, &super::SpellPoints)>,
    castables: Query<(
        &super::castable::CastableSpell,
        Has<super::castable::Learned>,
        Option<&crate::game::combat::monster::slot::SlotOf>,
    )>,
    monsters: Query<(&crate::game::world::population::SpellBudget, &crate::game::actors::Level)>,
    held: crate::game::items::Items,
    context: Single<&super::may::UseContext, With<crate::game::party::roster::PartyRoster>>,
    mut commands: Commands,
) {
    let Cast { caster, choice, again } = *cast;
    let Ok((super::castable::CastableSpell(spell), learned, slot)) = castables.get(choice.castable)
    else {
        commands.trigger(Refused { caster, refusal: Refusal::NoItem });
        return;
    };
    if again
        && !pools
            .get(caster)
            .is_ok_and(|(vitals, pool)| goes_again((vitals.stamina.current, pool), spell))
    {
        return;
    }
    let mut made = commands.spawn((CastBy(caster), Chosen(choice)));
    // Only members cast learned spells and use items: theirs widen at the
    // party and provoke whom they are against.
    let by_member = slot.is_none();
    match (learned, slot) {
        (true, _) => {
            made.insert((
                means::LearnedSpell { castable: choice.castable, asked: choice.power },
                Widens,
            ));
        }
        (false, Some(_)) => {
            let (budget, level) = monsters.get(caster).map_or((0, 0), |(b, l)| (b.0, l.0 as u8));
            made.insert(means::Innate { power: choice.power, budget, level });
        }
        (false, None) => {
            made.insert((means::FromItem(choice.castable), Widens));
        }
    }
    if by_member && spell.aimed_at_enemies() && !again {
        made.insert(Provokes);
    }
    if again {
        made.insert(Again);
    }
    use crate::content::items::UseType;
    let by_user = **context == super::may::UseContext::Combat
        && held
            .casts(choice.castable)
            .is_some_and(|casts| matches!(casts.by, UseType::Potion | UseType::Food));
    if by_user {
        made.insert(TakenByUser);
    }
    // Last, so that its hook sees what it is cast with and by whom.
    made.insert(CastSpell(spell.clone()));
    let cast = made.id();
    commands.trigger(Ask { cast });
}

/// What a cast's spell is sets its remaining steps: a utility spell's
/// working, its mark and its work; a deed's kind of target (at the caster
/// when taken by its user), whose hook sets its aim, spread, stray, fall
/// and release, and the steps every deed has.
fn spelled(mut world: bevy::ecs::world::DeferredWorld, context: bevy::ecs::lifecycle::HookContext) {
    let cast = context.entity;
    let Some(CastSpell(spell)) = world.get::<CastSpell>(cast) else { return };
    let (working, targeting) = (spell.facts.working, spell.targeting);
    let by_user = world.get::<TakenByUser>(cast).is_some();
    let mut commands = world.commands();
    let mut made = commands.entity(cast);
    if let Some(working) = working {
        works(&mut made, working);
        made.observe(aiming::spread_nothing)
            .observe(aiming::rolled_work)
            .observe(aiming::release_work);
        return;
    }
    use crate::content::spells::Targeting;
    match targeting {
        _ if by_user => made.insert(aiming::AtCaster),
        Targeting::Caster => made.insert(aiming::AtCaster),
        Targeting::Ally => made.insert(aiming::AtFriend),
        Targeting::Party => made.insert(aiming::AtParty),
        Targeting::Enemy => made.insert(aiming::AtFoe),
        Targeting::EnemyGroup => made.insert(aiming::AtFoeGroup),
        Targeting::Cone => made.insert(aiming::AtCone),
        Targeting::Radius => made.insert(aiming::AtBlast),
        Targeting::AllEnemies => made.insert(aiming::AtAllFoes),
        Targeting::Point | Targeting::Item | Targeting::LockOrTrap => {
            made.observe(aiming::unaimable)
        }
    };
    made.observe(aiming::go_astray).observe(aiming::fizzled).observe(aiming::release);
}

pub fn plugin(app: &mut App) {
    app.add_observer(begin).add_observer(pay).add_observer(refuse);
    aiming::plugin(app);
}
