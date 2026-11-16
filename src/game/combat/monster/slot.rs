//! A monster's spell slots (`docs/rules/monsters.md` section 5): each an
//! entity made with it, carrying what its spell is to the monster, and
//! marked while it may be cast.

use bevy::ecs::relationship::RelatedSpawner;
use bevy::ecs::spawn::{SpawnRelated, SpawnWith};
use bevy::prelude::*;

use crate::content::species::Species;
use crate::content::spells::{SpellId, Spellbook, Targeting};
use crate::game::actors::KeepsAlchemy;
use crate::game::characters::{Condition, Conditions};
use crate::game::data::Catalogs;
use crate::game::hits::definitions::SpellRow;

/// How the ten slots weigh in the choice (retail `g_spell_cast_weights`).
const SLOT_WEIGHTS: [u32; 10] = [5, 5, 5, 10, 10, 10, 10, 15, 15, 15];

/// Whose slot it is.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = Slots)]
pub struct SlotOf(pub Entity);

/// A monster's slots, in their order.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = SlotOf, linked_spawn)]
pub struct Slots(Vec<Entity>);

/// The spell in a slot.
#[derive(Component, Clone, Copy, Debug)]
pub struct Casts(pub SpellId);

/// How much the slot weighs in the choice.
#[derive(Component, Clone, Copy, Debug)]
pub struct Weight(pub u32);

/// The slot is the monster's breath: its special attack's cone.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Breath;

/// How far its spell reaches, in metres.
#[derive(Component, Clone, Copy, Debug)]
pub struct Range(pub f32);

/// Whom a spell at one body is cast at.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastAt {
    Itself,
    Friends,
    Foes,
}

/// Its spell is aimed against enemies (retail `MonsterCanAimSpell`): it can
/// open a fight, and is not cast where it would take in a neutral.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct AtEnemies;

/// One aim of it takes in whoever stands in its cone or its blast.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Area {
    Cone,
    Blast,
}

/// What its spell does, by which a body would feel it or not. A breath has
/// none: every body would feel it (retail lists the cone with the damage
/// spells in `MonsterSpellTargetOK`).
#[derive(Component, Clone, Copy, Debug)]
pub struct Works(pub &'static SpellRow);

/// Its spell is Alchemy, which a silence leaves to one who keeps it.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Alchemy;

/// It may be cast now: no silence stops it.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Castable;

/// A kind's slots: each spell of its ten slots, by the slot's weight, and its
/// breath if its special attack is one.
pub fn of_kind(kind: &Species) -> impl Bundle {
    let spells = kind.magic.spells;
    let breathes = kind.special.kind.is_some() && !super::special::summons(&kind.special);
    Slots::spawn(SpawnWith(move |slots: &mut RelatedSpawner<SlotOf>| {
        for (spell, weight) in spells.into_iter().zip(SLOT_WEIGHTS) {
            if let Some(spell) = spell {
                slots.spawn((Casts(spell), Weight(weight)));
            }
        }
        if breathes {
            slots.spawn((Casts(SpellId::SPECIAL_ATTACK_CONE), Weight(0), Breath));
        }
    }))
}

/// A slot's spell, as it is made, gives it its reach, what it does, and how
/// its aims are made: the observer of its kind of aim watches it, at one
/// body (cast at itself, friends or foes), a group, a cone or a blast, or
/// ahead. A spell a monster cannot cast at
/// all (retail `IsSpellUsableByMonster`: monsters may not cast it, the game
/// does not resolve it, or it is aimed at the party, a point, an item or a
/// lock) leaves no slot.
pub(super) fn fill(
    made: On<Add<Casts>>,
    mut commands: Commands,
    slots: Query<(&Casts, Has<Breath>)>,
    content: Res<Catalogs>,
) -> Result {
    let slot = made.entity;
    let (&Casts(id), breath) = slots.get(slot)?;
    let Some(spell) = content.spells.get(id) else { return Ok(()) };
    let row = crate::game::hits::definitions::row(spell);
    let mut slot = commands.entity(slot);
    if !breath {
        let Some(row) = row.filter(|_| spell.monsters_cast) else {
            slot.despawn();
            return Ok(());
        };
        slot.insert(Works(row));
    }
    use super::aim;
    match spell.targeting {
        Targeting::Caster => slot.insert(CastAt::Itself).observe(aim::at_bodies),
        Targeting::Ally => slot.insert(CastAt::Friends).observe(aim::at_bodies),
        Targeting::Enemy => slot.insert(CastAt::Foes).observe(aim::at_bodies),
        Targeting::EnemyGroup => slot.observe(aim::at_groups),
        Targeting::Cone => slot.insert(Area::Cone).observe(aim::at_areas),
        Targeting::Radius => slot.insert(Area::Blast).observe(aim::at_areas),
        Targeting::AllEnemies => slot.observe(aim::ahead),
        Targeting::Party | Targeting::Point | Targeting::Item | Targeting::LockOrTrap => {
            slot.despawn();
            return Ok(());
        }
    };
    slot.insert(Range(spell.reach.metres()));
    if spell.aimed_at_enemies() {
        slot.insert(AtEnemies);
    }
    if spell.books.contains(Spellbook::Alchemy) {
        slot.insert(Alchemy);
    }
    Ok(())
}

/// A slot may be cast while no silence stops it: a silence stops all but
/// Alchemy in the hands of a monster that keeps it.
pub(crate) fn castable(
    slots: Query<(Entity, &SlotOf, Has<Alchemy>, Has<Castable>), Without<Breath>>,
    monsters: Query<(&Conditions, Has<KeepsAlchemy>)>,
    mut commands: Commands,
) {
    for (slot, &SlotOf(monster), alchemy, was) in &slots {
        let Ok((conditions, keeps)) = monsters.get(monster) else { continue };
        let now = !conditions.has(Condition::Silenced) || (alchemy && keeps);
        match (now, was) {
            (true, false) => {
                commands.entity(slot).insert(Castable);
            }
            (false, true) => {
                commands.entity(slot).remove::<Castable>();
            }
            _ => {}
        }
    }
}
