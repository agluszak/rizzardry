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
            (body::drive_party.in_set(TnuaUserControlsSystems), walk_in::fly_party)
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
        formation.unseat(member);
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
