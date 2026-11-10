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

/// A monster has died: its autonomous work stops, and unless it was
/// summoned its death counts against its faction (retail
/// `RecordFactionOffense`), an NPC's body leaves the NPC dead for good, and
/// it drops its loot and pays its gold into the party's purse.
#[allow(clippy::too_many_arguments)]
pub fn monster_died(
    death: On<Add<crate::game::characters::Dead>>,
    mut commands: Commands,
    monsters: Query<(
        &Monster,
        ActorPose,
        Option<&Walker>,
        &LevelOwned,
        Has<crate::game::magic::pointcasts::SummonedBy>,
    )>,
    mut motion: Query<(
        Option<&mut AgentTarget3d>,
        Option<&mut TnuaController<WalkerMotion>>,
        Option<&mut TnuaMotor>,
        Option<&mut LinearVelocity>,
        Option<&mut Velocity3d>,
    )>,
    (bodies, mut npcs): (
        Query<&crate::game::npcs::NpcBody>,
        Query<(&mut crate::game::npcs::Npc, &crate::game::npcs::ActorId)>,
    ),
    (mut purse, roster, characters): (
        Single<&mut PartyInventory>,
        Single<&crate::game::party::roster::PartyRoster>,
        Query<&crate::game::characters::Character>,
    ),
    content: If<Res<crate::game::data::Catalogs>>,
    (mut rng, mut factions, clock): (
        ResMut<GameRng>,
        ResMut<crate::game::factions::Factions>,
        Res<crate::game::time::GameClock>,
    ),
    mut faction_news: MessageWriter<crate::game::factions::FactionNews>,
) {
    let target = death.entity;
    let Ok((monster, at, walker, owner, summoned)) = monsters.get(target) else { return };
    // Stop autonomous work, not the body's geometry or vertical physics.
    let (targeting, controller, motor, velocity, navigation_velocity) =
        motion.get_mut(target).unwrap();
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
    commands.entity(target).remove_with_requires::<Crossing>();
    if summoned {
        return;
    }
    let (catalog, tables) = (&content.items, &content.loot);
    let kind = &content.species[monster.species];
    if let Some(news) = factions.record_offence(kind.faction, kind.npc.is_some(), clock.ms) {
        faction_news.write(news);
    }
    let rng = &mut *rng;
    let npc = bodies.get(target).ok().and_then(|body| npcs.get_mut(body.0).ok());
    let (found, gold) = match npc {
        Some((mut npc, id)) => {
            npc.dead = true;
            crate::game::loot::npc_loot(&content.npcs[id.npc()], tables, catalog, rng)
        }
        None => {
            let party_level = crate::game::party::average_level(
                roster.iter().map(|member| characters.get(member).unwrap().level()),
            );
            crate::game::loot::monster_loot(kind, tables, catalog, party_level, rng)
        }
    };
    purse.gold += gold;
    let position = feet(at.translation(), walker);
    drop_loot(&mut commands, owner.0, position, &found, (catalog, rng));
}
