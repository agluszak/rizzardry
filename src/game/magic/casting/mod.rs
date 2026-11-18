//! Casting a learned spell, using an item's spell, or a monster's slot
//! (`docs/rules/casting.md`, "How this is built"). A spell that ends in a
//! deed is cast in one moment by one observer ([`deed::cast`]). A utility
//! spell is an entity of its caster's (`CastBy`) while its working finds what
//! it is cast at ([`Allowed`], [`marked`]); then [`work`] settles its
//! power, rolls and pays for it, and its working does what it is for
//! ([`Worked`]).
//!
//! Choosing never spends anything. A refused cast spends nothing; one that is
//! rolled pays even when it fizzles or backfires. Its deed keeps its own copy
//! of the spell and victims, so using the last potion still heals, and a
//! missile in flight still lands after its caster falls.

mod deed;
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

/// A caster pays for a cast of a castable at a power: the stamina it tires
/// them by ([`tire`]), and a learned spell's points ([`spend_points`]).
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Pays {
    #[event_target]
    pub caster: Entity,
    pub castable: Entity,
    pub power: u32,
    pub stamina: u32,
}

/// A cast tires its caster.
fn tire(pays: On<Pays>, mut casters: Query<&mut crate::game::characters::Vitals>) -> Result {
    casters.get_mut(pays.caster)?.tire(pays.stamina);
    Ok(())
}

/// A learned spell costs its realm's points, its cost at each power.
fn spend_points(
    pays: On<Pays>,
    learned: Query<&super::castable::CastableSpell, With<super::castable::Learned>>,
    mut casters: Query<&mut crate::game::magic::SpellPoints>,
) {
    let Ok(super::castable::CastableSpell(spell)) = learned.get(pays.castable) else { return };
    let (Some(realm), Ok(mut pool)) = (spell.realm, casters.get_mut(pays.caster)) else { return };
    pool.spend(realm, spell.cost * pays.power);
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

/// The caster of a utility spell under way.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = Casting)]
pub struct CastBy(pub Entity);

/// The utility spells an actor has under way; they go with it.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = CastBy, linked_spawn)]
pub struct Casting(Vec<Entity>);

/// The spell cast, as content defines it.
#[derive(Component, Clone, Debug)]
pub struct CastSpell(pub SpellDefinition);

/// What the caster chose: what it is cast with and what it is aimed at.
#[derive(Component, Clone, Copy, Debug)]
pub struct Chosen(pub MagicChoice);

/// What a utility spell is aimed at.
#[derive(Component, Clone, Copy, Debug)]
pub struct Mark(pub Marked);

/// A utility spell may be cast: its working finds what it is cast at.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Allowed {
    #[event_target]
    pub cast: Entity,
}

/// A utility spell has what it is cast at: it goes off ([`work`]).
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Aimed {
    #[event_target]
    pub cast: Entity,
}

/// A cast is refused: the caster is told, and it is gone.
fn refuse_cast(commands: &mut Commands, (cast, caster): (Entity, Entity), refusal: Refusal) {
    commands.trigger(Refused { caster, refusal });
    commands.entity(cast).despawn();
}

/// A utility spell asked for, if it may be cast (section 2), becomes an
/// entity of its caster's, with the spell and what the caster chose, and
/// its working's observers; its working finds what it is cast at.
fn begin(
    cast: On<Cast>,
    spells: Query<&super::castable::CastableSpell>,
    means: means::Means,
    mut commands: Commands,
) {
    let Cast { caster, choice, .. } = *cast;
    let Ok(super::castable::CastableSpell(spell)) = spells.get(choice.castable) else { return };
    let Some(working) = spell.facts.working else { return };
    if let Some(refusal) = means.refusal(caster, choice, spell) {
        commands.trigger(Refused { caster, refusal });
        return;
    }
    let mut made = commands.spawn((CastBy(caster), Chosen(choice), CastSpell(spell.clone())));
    works(&mut made, working);
    let cast = made.id();
    commands.trigger(Allowed { cast });
}

/// A utility spell that has what it is cast at goes off (sections 3, 4 and
/// 7): its power, the roll, whatever came of it paid for and practised;
/// then its working does what it is for, whatever the roll.
fn work(
    aimed: On<Aimed>,
    casts: Query<(&CastBy, &CastSpell, &Chosen, &Mark)>,
    means: means::Means,
    held: crate::game::items::Items,
    content: Res<crate::game::data::Catalogs>,
    underwater: Option<
        Single<&crate::game::world::triggers::LevelState, With<crate::game::world::level::InPlay>>,
    >,
    mut rng: ResMut<crate::game::rng::GameRng>,
    mut commands: Commands,
) -> Result {
    let cast = aimed.cast;
    let (&CastBy(caster), CastSpell(spell), &Chosen(choice), &Mark(marked)) = casts.get(cast)?;
    let powered = match means.power(caster, choice, spell, &mut rng) {
        Ok(powered) => powered,
        Err(means::PlayedBadly { stamina, trains }) => {
            commands.trigger(Pays { caster, castable: choice.castable, power: 0, stamina });
            if let Some(skill) = trains {
                commands.trigger(crate::game::characters::Practise {
                    member: caster,
                    skill,
                    usage: 1,
                });
            }
            refuse_cast(&mut commands, (cast, caster), Refusal::PlayedBadly);
            return Ok(());
        }
    };
    let power = powered.power;
    // Fire under water is quenched.
    let quenched = spell.realm == Some(crate::content::characters::Realm::Fire)
        && underwater.is_some_and(|state| state.camera_sway);
    let came = super::roll::outcome(
        &mut rng,
        powered.failure,
        powered.quiet,
        quenched,
        spell.can_backfire(),
    );
    let stamina = means.toll(caster, choice, spell, power, &mut rng);
    commands.trigger(Pays { caster, castable: choice.castable, power, stamina });
    for (skill, usage) in means.practice(caster, choice, spell, power) {
        commands.trigger(crate::game::characters::Practise { member: caster, skill, usage });
    }
    let item = means.item(choice.castable).map(|_| choice.castable);
    let name = item.and_then(|item| held.id(item)).map(|id| content.items[id].name.clone());
    let report =
        Report { caster, spell: spell.id, power, item: name, outcome: came, effects: vec![] };
    commands.trigger(Worked { cast, caster, report, strength: powered.strength, marked });
    if let Some(item) = item {
        commands.trigger(crate::game::items::Spend { item, amount: 1 });
    }
    commands.entity(cast).despawn();
    Ok(())
}

pub fn plugin(app: &mut App) {
    app.add_observer(begin)
        .add_observer(deed::cast)
        .add_observer(work)
        .add_observer(super::landed::practise)
        .add_observer(super::landed::follow_spell)
        .add_observer(super::landed::recast)
        .add_observer(tire)
        .add_observer(spend_points)
        .add_observer(refuse);
}
