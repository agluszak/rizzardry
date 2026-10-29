//! The party's roster, formation and spatial body.

pub mod body;
pub mod formation;
pub mod recovery;
pub mod roster;
pub mod walk_in;

use avian3d::prelude::{Gravity, PhysicsSystems};
use bevy::prelude::*;
use bevy_tnua::prelude::*;
use bevy_tnua_avian3d::prelude::*;

use crate::game::world::level::level_active;

/// A member's standing choice for the next round. Recruitment starts on
/// Attack (retail `ResetPartySlotRow`).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    #[default]
    Attack,
    /// A Fighter's fury: attacks that hit harder and less surely, at twice
    /// the stamina (retail Berserk action).
    Berserk,
    /// No attack; +2 armour, and stamina back at the round's end.
    Defend,
    /// Cast a learned spell or use an item's spell, at its initiative.
    Magic(crate::game::magic::casting::MagicChoice),
}

pub struct PartyPlugin;

impl Plugin for PartyPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            TnuaControllerPlugin::<body::PartyMotion>::new(FixedUpdate),
            TnuaAvian3dPlugin::new(FixedUpdate),
        ))
        .insert_resource(Gravity(Vec3::NEG_Y * body::GRAVITY))
        .add_message::<body::PartyLook>()
        .add_message::<body::TurnParty>()
        .init_resource::<body::PartyTurning>()
        .add_message::<DamageApplied>()
        .add_systems(
            Update,
            (body::look_around
                .after(crate::game::GameplaySet::LevelActivation)
                .before(crate::game::GameplaySet::Combat)
                .run_if(level_active),),
        )
        // Tnua and Avian step on the fixed timestep; movement input feeds
        // them on the same step.
        .add_systems(
            FixedUpdate,
            (
                body::turn_party,
                body::drive_party.in_set(TnuaUserControlsSystems),
                walk_in::fly_party,
            )
                .chain()
                .run_if(level_active),
        )
        .add_systems(
            FixedPostUpdate,
            (
                walk_in::run_walk_in_triggers.pipe(crate::game::world::triggers::walk),
                walk_in::move_party,
            )
                .chain()
                .after(PhysicsSystems::Writeback)
                .run_if(level_active),
        );
    }
}

/// Party members' profiles, vitals, belongings and burdens, as rules use and
/// change them. Membership itself is the roster's.
pub type Members<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut crate::game::characters::Character,
        &'static mut crate::game::characters::Vitals,
        &'static mut crate::game::inventory::Carried,
        &'static crate::game::inventory::Burden,
    ),
>;

/// The party's average level (retail `GetAveragePartyLevel`), over every
/// member, dead or alive: what loot and stock scale by. 1 for an empty
/// party, where retail divides by zero.
pub fn average_level(levels: impl IntoIterator<Item = u8>) -> u32 {
    let (sum, count) =
        levels.into_iter().fold((0, 0), |(sum, count), level| (sum + u32::from(level), count + 1));
    sum.checked_div(count).unwrap_or(1)
}

/// Actual damage to an actor, after its gameplay consequences have settled.
/// Presentation can retain this result even if healing follows before it draws.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageApplied {
    pub member: Entity,
    pub amount: u32,
    pub died: bool,
}

/// Apply party damage and settle the target's death invariants before another
/// operation can use its formation seat. Retain change tracking until death
/// actually changes a seat.
pub fn damage_member(
    member: Entity,
    vitals: &mut crate::game::characters::Vitals,
    formation: &mut Mut<formation::Formation>,
    amount: u32,
    damage: &mut MessageWriter<DamageApplied>,
) -> u32 {
    let lost = vitals.damage(amount);
    if vitals.is_dead() && formation.row_of(member).is_some() {
        formation.update_seat(member, true);
    }
    if lost > 0 {
        damage.write(DamageApplied { member, amount: lost, died: vitals.is_dead() });
    }
    lost
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::characters::Vitals;
    use bevy::ecs::system::SystemState;

    #[test]
    fn damage_reports_actual_loss_and_death_even_when_healing_follows() {
        let mut world = World::new();
        world.init_resource::<Messages<DamageApplied>>();
        let first = world.spawn(Vitals { hp: 10, stamina: 20, ..default() }).id();
        roster::test_party(&mut world, &[first]);
        let mut operation = SystemState::<(
            Query<&mut Vitals>,
            Single<&mut formation::Formation>,
            MessageWriter<DamageApplied>,
        )>::new(&mut world);
        {
            let (mut members, mut formation, mut damage) = operation.get_mut(&mut world).unwrap();
            let mut vitals = members.get_mut(first).unwrap();
            vitals.enchantments.apply(crate::game::magic::enchantments::Enchantment {
                spell: crate::content::spells::SpellId::GUARDIAN_ANGEL,
                power: 2,
                boost: 0,
                turns: 4,
                absorption: 6,
            });
            assert_eq!(damage_member(first, &mut vitals, &mut formation, 6, &mut damage), 0);
            assert_eq!(vitals.stamina, 20, "Guardian Angel absorbs before fatigue");
            assert!(formation.row_of(first).is_some());
            assert_eq!(damage_member(first, &mut vitals, &mut formation, 3, &mut damage), 3);
            vitals.heal(3, 10);
        }
        let reported: Vec<_> = world.resource_mut::<Messages<DamageApplied>>().drain().collect();
        assert_eq!(reported, [DamageApplied { member: first, amount: 3, died: false }]);
        {
            let (mut members, mut formation, mut damage) = operation.get_mut(&mut world).unwrap();
            let mut vitals = members.get_mut(first).unwrap();
            assert_eq!(damage_member(first, &mut vitals, &mut formation, 100, &mut damage), 10);
            assert!(formation.row_of(first).is_none(), "death settles before another action");
            assert_eq!(damage_member(first, &mut vitals, &mut formation, 1, &mut damage), 0);
        }
        let reported: Vec<_> = world.resource_mut::<Messages<DamageApplied>>().drain().collect();
        assert_eq!(reported, [DamageApplied { member: first, amount: 10, died: true }]);
    }
}
