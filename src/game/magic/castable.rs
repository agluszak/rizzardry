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
    CanFight, CasterLevels, KeepsAlchemy, Settle, Skills, Traits, UsableSkills, Written,
};
use crate::game::camp::Camp;
use crate::game::characters::{Calling, Condition, Conditions, Training};
use crate::game::magic::may::{self, Refusal, UseContext};
use crate::game::magic::occasion::{Occupied, SpokenTo, Unsteady};
use crate::game::magic::{SpellPoints, power, roll};
use crate::game::party::roster::{PartyMember, PartyRoster};

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

/// Why it may not be cast now, while something stops it.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Barred(pub Refusal);

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

/// How likely a blind caster's aim holds anyway, in percent: half, scaled
/// by the profession level, with the Effective While Blind trait.
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
            let at = std::array::from_fn(|power| {
                roll::learned((calling, training), skills, level, spell, power as u32 + 1)
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
    casters: Query<
        (&Castables, &Traits, &Calling),
        Or<(Changed<Castables>, Changed<Traits>, Changed<Calling>)>,
    >,
    mut castables: Query<&mut SeesBlind>,
) {
    for (owned, traits, calling) in &casters {
        let mut each = castables.iter_many_mut(owned.iter());
        while let Some(next) = each.fetch_next() {
            let Ok(mut sees) = next else { continue };
            let now = match traits.0.contains(Trait::EffectiveWhileBlind) {
                true => SeesBlind(calling.scale(50.0) as u32),
                false => SeesBlind(0),
            };
            sees.set_if_neq(now);
        }
    }
}

/// What stops a learned spell now (section 2, in its order): a caster who
/// cannot fight or is no member, a silence (Alchemy kept aside), no points
/// for power 1, a spell the game does not resolve, a moment it does not
/// suit.
#[allow(clippy::type_complexity)]
fn barred(
    castables: Query<
        (Entity, &CastableOf, &CastableSpell, &MostPower, Option<&Barred>),
        With<Learned>,
    >,
    casters: Query<(Has<CanFight>, Has<PartyMember>, &Conditions, Has<KeepsAlchemy>)>,
    party: Single<(&UseContext, Has<Occupied>, Has<SpokenTo>, Has<Unsteady>), With<PartyRoster>>,
    camp: Option<Res<Camp>>,
    mut commands: Commands,
) {
    let (&context, occupied, spoken_to, unsteady) = *party;
    for (castable, &CastableOf(caster), CastableSpell(spell), most, was) in &castables {
        let Ok((can_fight, member, conditions, keeps)) = casters.get(caster) else { continue };
        let suits =
            may::suits(spell, context, occupied, camp.is_some(), spoken_to, unsteady, false);
        let now = if !can_fight || !member {
            Some(Refusal::CannotAct)
        } else if conditions.has(Condition::Silenced) && may::silence_stops(spell, keeps) {
            Some(Refusal::Silenced)
        } else if most.0 == 0 {
            Some(Refusal::NoSpellPoints)
        } else if !super::casting::resolvable(spell) {
            Some(Refusal::Unsupported)
        } else if !suits {
            Some(Refusal::WrongTime)
        } else {
            None
        };
        match (now, was) {
            (Some(refusal), was) if was != Some(&Barred(refusal)) => {
                commands.entity(castable).insert(Barred(refusal));
            }
            (None, Some(_)) => {
                commands.entity(castable).remove::<Barred>();
            }
            _ => {}
        }
    }
}

pub fn plugin(app: &mut App) {
    crate::game::world::aside::follows::<CastableOf>(app);
    app.add_observer(item_casts);
    app.register_required_components::<Learned, MostPower>()
        .register_required_components::<Learned, FailsAt>()
        .register_required_components::<Learned, StrengthFigure>()
        .register_required_components::<Learned, Boost>()
        .register_required_components::<Learned, SeesBlind>()
        .add_systems(
            Settle,
            ((most_power, fails_at, strength_figure, sees_blind, barred.after(most_power))
                .after(Written),),
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
