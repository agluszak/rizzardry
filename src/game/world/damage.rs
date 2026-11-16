//! What a monster's death brings about, whatever the cause.

use super::navigation::{WalkerMotion, links::Crossing};
use crate::game::world::ActorPose;
use crate::game::world::level::LevelOwned;
use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use bevy_landmass::{AgentTarget3d, Velocity3d};
use bevy_tnua::{TnuaController, TnuaMotor};

use super::items::drop_loot;
use super::navigation::{Walker, feet};
use super::population::Monster;
use crate::game::inventory::PartyInventory;
use crate::game::rng::GameRng;

/// The dead of a level are gone when the party comes back to it: they stay
/// dead, and nothing of them lies there any more.
pub fn clear_the_fallen(
    woke: On<super::level::Woke>,
    fallen: Query<
        (Entity, &super::level::LevelOwned),
        (
            With<Monster>,
            With<crate::game::characters::Dead>,
            bevy::ecs::query::Allow<bevy::ecs::entity_disabling::Disabled>,
        ),
    >,
    mut commands: Commands,
) {
    for (monster, owner) in &fallen {
        if owner.0 == woke.level {
            commands.entity(monster).despawn();
        }
    }
}

/// A dead monster's autonomous work stops; its body's geometry and
/// vertical physics stay.
pub fn halt(
    death: On<Add<crate::game::characters::Dead>>,
    mut motion: Query<(
        Option<&mut AgentTarget3d>,
        Option<&mut TnuaController<WalkerMotion>>,
        Option<&mut TnuaMotor>,
        Option<&mut LinearVelocity>,
        Option<&mut Velocity3d>,
        Has<Monster>,
    )>,
    mut commands: Commands,
) -> Result {
    let (targeting, controller, motor, velocity, navigation_velocity, monster) =
        motion.get_mut(death.entity)?;
    // Only a monster walks on its own.
    if !monster {
        return Ok(());
    }
    if let Some(mut targeting) = targeting {
        *targeting = AgentTarget3d::None;
    }
    if let Some(mut controller) = controller {
        controller.basis = default();
    }
    if let Some(mut motor) = motor {
        *motor = default();
    }
    if let Some(mut velocity) = velocity {
        velocity.0 = Vec3::Y * velocity.y;
    }
    if let Some(mut velocity) = navigation_velocity {
        velocity.velocity = Vec3::ZERO;
    }
    commands.entity(death.entity).remove_with_requires::<Crossing>();
    Ok(())
}

/// A dead monster drops its loot where it stood and pays its gold into the
/// party's purse: an NPC's body its NPC's, any other what its species
/// drops, made now. One summoned leaves nothing.
pub fn leave_loot(
    death: On<Add<crate::game::characters::Dead>>,
    mut commands: Commands,
    monsters: Query<
        Option<(
            (&Monster, ActorPose, Option<&Walker>, &LevelOwned),
            Has<crate::game::magic::pointcasts::SummonedBy>,
        )>,
    >,
    (bodies, npcs): (Query<&crate::game::npcs::Embodies>, Query<&crate::game::npcs::ActorId>),
    mut purse: Single<&mut PartyInventory>,
    members: Query<
        &crate::game::characters::Calling,
        With<crate::game::party::roster::PartyMember>,
    >,
    content: If<Res<crate::game::data::Catalogs>>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let target = death.entity;
    // A member leaves nothing, nor does one summoned.
    let Some(((monster, at, walker, owner), false)) = monsters.get(target)? else { return Ok(()) };
    let (catalog, tables) = (&content.items, &content.loot);
    let rng = &mut *rng;
    let npc = bodies.get(target).ok().and_then(|body| npcs.get(body.0).ok());
    let (found, gold) = match npc {
        Some(id) => crate::game::loot::npc_loot(&content.npcs[id.npc()], tables, catalog, rng),
        None => {
            let party_level = crate::game::party::average_level(
                members.iter().map(crate::game::characters::Calling::level),
            );
            let kind = &content.species[monster.species];
            crate::game::loot::monster_loot(kind, tables, catalog, party_level, rng)
        }
    };
    purse.gold += gold;
    let position = feet(at.translation(), walker);
    drop_loot(&mut commands, owner.0, position, &found, (catalog, rng));
    Ok(())
}
