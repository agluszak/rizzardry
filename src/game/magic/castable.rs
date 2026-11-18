//! What one may cast with (`docs/rules/casting.md`, "The rebuild under
//! way"): a castable is an entity of its caster's, carrying the spell's
//! record, read from content as it is made, and figures written in
//! [`Settle`] as their sources change: whether it may be cast now, the most
//! power it can be cast at, how likely it fails at each power, the figure
//! its strength grows from, its boost, and how likely a blind caster's aim
//! holds anyway. The magic menu and the cast read them; neither works them
//! out again.
//!
//! A member's learned spell is a castable of theirs ([`Learned`]).

use bevy::prelude::*;
use enum_map::EnumMap;

use crate::content::characters::{Realm, Skill, Trait};
use crate::content::spells::SpellDefinition;
use crate::game::actors::{
    CasterLevels, KeepsAlchemy, Settle, Skills, Traits, UsableSkills, Written,
};
use crate::game::characters::{Calling, Condition, Conditions, Training};
use crate::game::combat::monster::slot::{SlotOf, Slots};
use crate::game::magic::may;
use crate::game::magic::{SpellPoints, power, roll};
use crate::game::world::population::SpellBudget;

/// Whose castable it is.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = Castables)]
pub struct CastableOf(pub Entity);

/// What an actor may cast with; it goes with them.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = CastableOf, linked_spawn)]
pub struct Castables(Vec<Entity>);

/// The spell a castable casts, as content defines it.
#[derive(Component, Clone, Debug)]
pub struct CastableSpell(pub SpellDefinition);

/// A spell its caster has learned: cast from their realm's points.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Learned;

/// A spell learned, as a castable of whoever learns it.
pub fn learned(spell: &SpellDefinition) -> impl Bundle {
    (Learned, CastableSpell(spell.clone()))
}

/// An item that casts is a castable of whoever uses it: its spell, read as
/// its use is made.
fn item_casts(
    made: On<Insert<crate::game::items::Casts>>,
    items: Query<&crate::game::items::Casts>,
    content: If<Res<crate::game::data::Catalogs>>,
    mut commands: Commands,
) -> Result {
    let casts = items.get(made.entity)?;
    let spell = content.spells.get(casts.spell).ok_or("an item casts a spell of the catalog")?;
    commands.entity(made.entity).insert(CastableSpell(spell.clone()));
    Ok(())
}

/// A silence stops it now: its caster is silenced and it is not an Alchemy
/// spell in the hands of one who keeps their Alchemy (retail
/// `IsSpellBlockedForCharacter`, `IsSpellBlockedForMonster`).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Silenced;

/// The most power it may be cast at now; none while nothing pays for it.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MostPower(pub u32);

/// How likely it fails at each power, 1 to 7, in percent, before the
/// round's pace and what a spell adds cast on oneself.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FailsAt(pub [u32; 7]);

impl FailsAt {
    pub fn at(&self, power: u32) -> u32 {
        self.0[(power.clamp(1, 7) - 1) as usize]
    }
}

/// The figure its strength grows from: its caster's level for it.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StrengthFigure(pub i32);

/// What Power Cast raises its strength by, in percent.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Boost(pub u32);

/// How likely a caster's aim holds anyway while they are blind, in percent:
/// half, scaled by their profession level, with the Effective While Blind
/// trait. A figure of the caster, whatever they cast with.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SeesBlind(pub u32);

/// How many spells an actor knows in each realm, and the dearest of them at
/// power 1: what the skills it may use and its spell points follow.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Repertoire(pub EnumMap<Realm, Known>);

/// The spells known in a realm.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Known {
    pub count: u32,
    pub dearest: u32,
}

/// What an actor knows follows the spells it has learned; one whose last is
/// gone knows none.
pub fn repertoire(
    mut actors: Query<(Option<&Castables>, &mut Repertoire)>,
    changed: Query<Entity, Changed<Castables>>,
    mut emptied: RemovedComponents<Castables>,
    spells: Query<&CastableSpell, With<Learned>>,
) {
    let touched: Vec<Entity> = changed.iter().chain(emptied.read()).collect();
    for actor in touched {
        let Ok((castables, mut written)) = actors.get_mut(actor) else { continue };
        let castables = castables.into_iter().flat_map(|owned| owned.iter());
        let mut known = Repertoire::default();
        for CastableSpell(spell) in spells.iter_many(castables).flatten() {
            if let Some(realm) = spell.realm {
                let realm = &mut known.0[realm];
                realm.count += 1;
                realm.dearest = realm.dearest.max(spell.cost);
            }
        }
        written.set_if_neq(known);
    }
}

/// A learned spell may be cast up to the power its realm's points pay for.
fn most_power(
    casters: Query<(&Castables, &SpellPoints), Or<(Changed<Castables>, Changed<SpellPoints>)>>,
    mut castables: Query<(&CastableSpell, &mut MostPower), With<Learned>>,
) {
    for (owned, points) in &casters {
        let mut each = castables.iter_many_mut(owned.iter());
        while let Some(next) = each.fetch_next() {
            let Ok((CastableSpell(spell), mut most)) = next else { continue };
            let affordable = spell
                .realm
                .map_or(0, |realm| power::affordable(points.available(realm), spell.cost));
            most.set_if_neq(MostPower(affordable));
        }
    }
}

/// A learned spell fails by its caster's skill and caster level.
#[allow(clippy::type_complexity)]
fn fails_at(
    casters: Query<
        (&Castables, &Calling, &Training, &Skills, &CasterLevels),
        Or<(Changed<Castables>, Changed<Calling>, Changed<Training>, Changed<Skills>)>,
    >,
    mut castables: Query<(&CastableSpell, &mut FailsAt), With<Learned>>,
) {
    for (owned, calling, training, skills, levels) in &casters {
        let mut each = castables.iter_many_mut(owned.iter());
        while let Some(next) = each.fetch_next() {
            let Ok((CastableSpell(spell), mut fails)) = next else { continue };
            let level = levels.caster_level(spell);
            // The book skill counts once, the realm skill four times.
            let book = skills[crate::game::magic::best_book((calling, training), spell)];
            let realm = spell.realm.map_or(0, |realm| skills[realm.skill()]);
            let at = std::array::from_fn(|power| {
                roll::learned((book, realm), level, spell, power as u32 + 1)
            });
            fails.set_if_neq(FailsAt(at));
        }
    }
}

/// A learned spell is as strong as its caster's level for it, boosted by
/// their Power Cast.
#[allow(clippy::type_complexity)]
fn strength_figure(
    casters: Query<
        (&Castables, &CasterLevels, &Skills, &UsableSkills),
        Or<(Changed<Castables>, Changed<CasterLevels>, Changed<Skills>, Changed<UsableSkills>)>,
    >,
    mut castables: Query<(&CastableSpell, &mut StrengthFigure, &mut Boost), With<Learned>>,
) {
    for (owned, levels, skills, usable) in &casters {
        let mut each = castables.iter_many_mut(owned.iter());
        while let Some(next) = each.fetch_next() {
            let Ok((CastableSpell(spell), mut figure, mut boost)) = next else { continue };
            figure.set_if_neq(StrengthFigure(levels.caster_level(spell)));
            let power_cast = usable.0.contains(Skill::PowerCast);
            boost.set_if_neq(Boost(power::boost(skills[Skill::PowerCast], power_cast)));
        }
    }
}

/// A member effective while blind may aim true blind.
fn sees_blind(
    mut casters: Query<
        (&Traits, &Calling, &mut SeesBlind),
        Or<(Changed<Traits>, Changed<Calling>)>,
    >,
) {
    for (traits, calling, mut sees) in &mut casters {
        let now = match traits.0.contains(Trait::EffectiveWhileBlind) {
            true => SeesBlind(calling.scale(50.0) as u32),
            false => SeesBlind(0),
        };
        sees.set_if_neq(now);
    }
}

/// A monster's slot fails by the monster's budget alone, standing in for a
/// caster's skill, and is as strong as the monster's level.
#[allow(clippy::type_complexity)]
fn slot_figures(
    monsters: Query<
        (&Slots, &SpellBudget, &crate::game::actors::Level),
        Or<(Changed<Slots>, Changed<SpellBudget>, Changed<crate::game::actors::Level>)>,
    >,
    mut slots: Query<(&CastableSpell, &mut FailsAt, &mut StrengthFigure)>,
) {
    for (owned, &SpellBudget(budget), level) in &monsters {
        let mut each = slots.iter_many_mut(owned.iter());
        while let Some(next) = each.fetch_next() {
            let Ok((CastableSpell(spell), mut fails, mut figure)) = next else { continue };
            let at =
                std::array::from_fn(|power| roll::spell_failure(budget, spell, power as u32 + 1));
            fails.set_if_neq(FailsAt(at));
            figure.set_if_neq(StrengthFigure(level.0 as i32));
        }
    }
}

/// A member's learned spells are silenced as the member is, Alchemy kept
/// aside while they keep their Alchemy.
#[allow(clippy::type_complexity)]
fn silenced(
    casters: Query<
        (&Castables, &Conditions, Has<KeepsAlchemy>),
        Or<(Changed<Castables>, Changed<Conditions>, Changed<Skills>)>,
    >,
    castables: Query<(Entity, &CastableSpell, Has<Silenced>)>,
    mut commands: Commands,
) {
    for (owned, conditions, keeps) in &casters {
        for (castable, CastableSpell(spell), was) in castables.iter_many(owned.iter()).flatten() {
            silence(&mut commands, castable, was, stops(spell, conditions, keeps));
        }
    }
}

/// A monster's slots are silenced as the monster is, Alchemy kept aside by
/// one that keeps it.
fn slots_silenced(
    monsters: Query<
        (&Slots, &Conditions, Has<KeepsAlchemy>),
        Or<(Changed<Slots>, Changed<Conditions>)>,
    >,
    slots: Query<(Entity, &CastableSpell, Has<Silenced>)>,
    mut commands: Commands,
) {
    for (owned, conditions, keeps) in &monsters {
        for (slot, CastableSpell(spell), was) in slots.iter_many(owned.iter()).flatten() {
            silence(&mut commands, slot, was, stops(spell, conditions, keeps));
        }
    }
}

/// Whether a silence stops a spell now.
fn stops(spell: &SpellDefinition, conditions: &Conditions, keeps_alchemy: bool) -> bool {
    conditions.has(Condition::Silenced) && may::silence_stops(spell, keeps_alchemy)
}

/// The marker goes on or comes off as the silence holds.
fn silence(commands: &mut Commands, castable: Entity, was: bool, now: bool) {
    match (was, now) {
        (false, true) => {
            commands.entity(castable).insert(Silenced);
        }
        (true, false) => {
            commands.entity(castable).remove::<Silenced>();
        }
        _ => {}
    }
}

pub fn plugin(app: &mut App) {
    crate::game::world::aside::follows::<CastableOf>(app);
    app.add_observer(item_casts);
    app.register_required_components::<Learned, MostPower>()
        .register_required_components::<Learned, FailsAt>()
        .register_required_components::<Learned, StrengthFigure>()
        .register_required_components::<Learned, Boost>()
        .register_required_components::<SlotOf, FailsAt>()
        .register_required_components::<SlotOf, StrengthFigure>()
        .register_required_components::<SlotOf, Boost>()
        .add_systems(
            Settle,
            (
                (most_power, fails_at, strength_figure, sees_blind, silenced)
                    .chain()
                    .after(Written),
                (slot_figures, slots_silenced).after(Written),
            ),
        );
}

/// The actor learns these spells, as castables of theirs.
#[cfg(test)]
pub fn test_learn(world: &mut World, actor: Entity, spells: &[crate::content::spells::SpellId]) {
    for &id in spells {
        let spell = world.resource::<crate::game::data::Catalogs>().spells[id].clone();
        world.spawn((CastableOf(actor), learned(&spell)));
    }
}

/// The actor's castable of a learned spell.
#[cfg(test)]
pub fn test_castable(world: &World, actor: Entity, id: crate::content::spells::SpellId) -> Entity {
    let owned = world.get::<Castables>(actor).expect("the actor learned spells");
    owned
        .iter()
        .find(|&castable| world.get::<CastableSpell>(castable).is_some_and(|s| s.0.id == id))
        .expect("the actor learned the spell")
}
