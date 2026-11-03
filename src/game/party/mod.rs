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

use crate::game::GameplaySet;
use crate::game::characters::Character;
use crate::game::world::level::level_active;

/// A member's standing choice for the next round. Recruitment starts on
/// Attack (retail `ResetPartySlotRow`).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub enum Order {
    #[default]
    Attack,
    /// A Fighter's fury: attacks that hit harder and less surely, at twice
    /// the stamina (retail Berserk action).
    Berserk,
    /// No attack; +2 armour, and stamina back at the round's end.
    Defend,
    /// Breathe toward the chosen actor; identity survives order edits.
    Breathe(Entity),
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
        .add_message::<DamageApplied>()
        .add_observer(formation::vacate_seat)
        .add_observer(formation::retake_seat)
        .add_systems(
            Update,
            (
                crate::game::characters::sync_member_maxima.before(GameplaySet::LevelActivation),
                // A death settles before travel can retire the level it happened on.
                (crate::game::characters::cheat_death, crate::game::characters::settle_deaths)
                    .chain()
                    .before(GameplaySet::LevelActivation),
                body::look_around
                    .after(GameplaySet::LevelActivation)
                    .before(GameplaySet::Combat)
                    .run_if(level_active),
            ),
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

/// The party's members in roster order, for operations that hold the whole
/// world.
pub fn members_in_world(
    world: &World,
) -> impl Iterator<Item = (Entity, &Character, &crate::game::characters::Vitals)> {
    let party =
        world.try_query_filtered::<Entity, With<roster::Party>>().unwrap().single(world).unwrap();
    let roster = world.get::<roster::PartyRoster>(party).unwrap();
    roster.iter().map(|m| (m, world.get(m).unwrap(), world.get(m).unwrap()))
}

/// The members awake and well enough to fight: the population retail's NPC
/// rules work with (`hp_current != 0 && highest_condition < ASLEEP`, as in
/// `CanNpcJoinParty`, the charm and dialogue practice).
pub fn able_in_world(world: &World) -> impl Iterator<Item = (Entity, &Character)> {
    members_in_world(world).filter(|(_, _, v)| v.can_fight()).map(|(m, c, _)| (m, c))
}

/// The lowest level among those who can fight, which weakens monsters' sight
/// from behind.
pub fn lowest_level<'a>(
    members: impl IntoIterator<Item = (&'a Character, &'a crate::game::characters::Vitals)>,
) -> i32 {
    members
        .into_iter()
        .filter(|(_, v)| v.can_fight())
        .map(|(c, _)| i32::from(c.level()))
        .min()
        .unwrap_or(0)
}

/// The mean of these levels, fractions kept, one when there are none
/// (retail `GetAveragePartyMemberLevel`'s arithmetic; [`averaged_places`]
/// is its population).
pub fn mean_level(levels: impl IntoIterator<Item = u8>) -> f32 {
    let (sum, count) = levels
        .into_iter()
        .fold((0.0, 0.0), |(sum, count), level| (sum + f32::from(level), count + 1.0));
    if count == 0.0 { 1.0 } else { sum / count }
}

/// The members, each with whether it is a recruited NPC, in roster order,
/// that retail's `GetAveragePartyMemberLevel` reads: its party slots 0 to
/// 5. `AddCharacterToParty` gives NPCs slots 0 and 1 and regular members
/// the lowest free of 2 to 7, so those are every NPC and the first four
/// regular members; the last two are left out, as in retail. Regular
/// members join in slot order and do not leave, so roster order is theirs.
pub fn averaged_places<T>(members: impl IntoIterator<Item = (T, bool)>) -> Vec<T> {
    let mut regular = 0;
    members
        .into_iter()
        .filter(|&(_, npc)| {
            regular += usize::from(!npc);
            npc || regular <= 4
        })
        .map(|(member, _)| member)
        .collect()
}

/// The party's average level (retail `GetAveragePartyLevel`), over every
/// member, dead or alive: what loot and stock scale by. 1 for an empty
/// party, where retail divides by zero.
pub fn average_level(levels: impl IntoIterator<Item = u8>) -> u32 {
    let (sum, count) =
        levels.into_iter().fold((0, 0), |(sum, count), level| (sum + u32::from(level), count + 1));
    sum.max(1) / count.max(1)
}

/// Actual damage to an actor, after its gameplay consequences have settled.
/// Presentation can retain this result even if healing follows before it draws.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageApplied {
    pub member: Entity,
    pub amount: u32,
    pub died: bool,
}

/// Hurt a member ([`crate::game::characters::Vitals::hurt`]) and report
/// what it cost them. Death is settled afterwards, as every death is.
pub fn damage_member(
    member: Entity,
    vitals: &mut crate::game::characters::Vitals,
    amount: u32,
    damage: &mut MessageWriter<DamageApplied>,
) -> u32 {
    let lost = vitals.hurt(amount);
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

    /// Retail averages its slots 0 to 5: the NPC places and the first four
    /// regular places, whatever order the NPCs joined in.
    #[test]
    fn the_averaged_places_are_every_npc_and_the_first_four_regular_members() {
        let regulars = [1, 1, 1, 1, 50, 50].map(|level| (level, false));
        assert_eq!(mean_level(averaged_places(regulars)), 1.0);
        let mut party = regulars.to_vec();
        party.push((10, true));
        assert_eq!(averaged_places(party.clone()), [1, 1, 1, 1, 10]);
        party.insert(2, (4, true));
        assert_eq!(averaged_places(party), [1, 1, 4, 1, 1, 10]);
        assert_eq!(mean_level(averaged_places([(7, true)])), 7.0);
        assert_eq!(mean_level(averaged_places(Vec::<(u8, bool)>::new())), 1.0);
    }

    #[test]
    fn damage_reports_actual_loss_and_death_even_when_healing_follows() {
        let mut world = World::new();
        world.init_resource::<Messages<DamageApplied>>();
        let first = world.spawn(Vitals::full(10, 20)).id();
        roster::test_party(&mut world, &[first]);
        world.add_observer(formation::vacate_seat);
        let mut operation =
            SystemState::<(Query<&mut Vitals>, MessageWriter<DamageApplied>)>::new(&mut world);
        {
            let (mut members, mut damage) = operation.get_mut(&mut world).unwrap();
            let mut vitals = members.get_mut(first).unwrap();
            vitals.enchantments.apply(crate::game::magic::enchantments::Enchantment {
                spell: crate::content::spells::SpellId::GUARDIAN_ANGEL,
                power: 2,
                boost: 0,
                turns: 4,
                absorption: 6,
            });
            assert_eq!(damage_member(first, &mut vitals, 6, &mut damage), 0);
            assert_eq!(vitals.stamina.current, 20, "Guardian Angel absorbs before fatigue");
            assert_eq!(damage_member(first, &mut vitals, 3, &mut damage), 3);
            vitals.heal(3);
        }
        let reported: Vec<_> = world.resource_mut::<Messages<DamageApplied>>().drain().collect();
        assert_eq!(reported, [DamageApplied { member: first, amount: 3, died: false }]);
        {
            let (mut members, mut damage) = operation.get_mut(&mut world).unwrap();
            let mut vitals = members.get_mut(first).unwrap();
            assert_eq!(damage_member(first, &mut vitals, 100, &mut damage), 10);
            assert_eq!(damage_member(first, &mut vitals, 1, &mut damage), 0);
        }
        let reported: Vec<_> = world.resource_mut::<Messages<DamageApplied>>().drain().collect();
        assert_eq!(reported, [DamageApplied { member: first, amount: 10, died: true }]);
        // Settling the death gives up the member's seat.
        use bevy::ecs::system::RunSystemOnce;
        world.run_system_once(crate::game::characters::settle_deaths).unwrap();
        let formation = world.query::<&formation::Formation>().single(&world).unwrap();
        assert!(formation.row_of(first).is_none(), "the dead give up their seat");
    }
}
