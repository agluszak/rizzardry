//! Monsters closing in on the party during a fight. A monster in the fight
//! gets a walking body sized and paced by its gameplay body and heads for the party
//! through the navmesh until one of its attacks reaches; the dead and those
//! in reach stand. Retail moves monsters as a scheduled action within the
//! round (advance and charge); here they close in continuously, a
//! deliberate simplification until those actions exist.

use crate::game::world::ActorPose;
use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::prelude::*;
use bevy_tnua::builtins::TnuaBuiltinWalkConfig;
use bevy_tnua::prelude::*;
use bevy_tnua_avian3d::prelude::*;

use super::Battle;
use crate::content::species::{MonsterCatalog, Reach};
use crate::game::data::GameData;
use crate::game::party::body::PartyBody;
use crate::game::world::STEP_HEIGHT;
use crate::game::world::navigation::{LevelNavigation, Walker, WalkerMotion, WalkerMotionConfig};
use crate::game::world::population::{Monster, ScaledBody};

/// Half the height of a monster's body. Bodies are flat cylinders as wide
/// as the model's walk radius, however tall the model.
const HALF_HEIGHT: f32 = 0.5;
/// The ground sensor's radius: the ground under the body's centre decides
/// its height, not the highest point anywhere under a wide body.
const SENSOR_RADIUS: f32 = 0.3;
/// Wider monsters navigate the narrowest navmesh that still fits this much:
/// the wider navmeshes lose narrow ground (beaches, ledges) to erosion, so
/// big monsters may brush corners instead.
const MAX_NAVIGATION_RADIUS: f32 = 1.0;
/// Stop this far inside an attack's reach.
const REACH_MARGIN: f32 = 0.3;

/// Give monsters in the fight a walking body.
pub fn embody_fighters(
    mut commands: Commands,
    battle: Option<Res<Battle>>,
    mut monsters: Query<(&Monster, &mut Transform), Without<Walker>>,
    navigation: Query<&LevelNavigation>,
    mut configs: ResMut<Assets<WalkerMotionConfig>>,
) {
    let (Some(battle), Ok(navigation)) = (battle, navigation.single()) else { return };
    for &(entity, _) in &battle.monsters {
        let Ok((monster, mut transform)) = monsters.get_mut(entity) else { continue };
        // In combat retail's navigator moves a monster with its fight
        // radius.
        let ScaledBody { radius, speed, .. } = monster.body;
        if monster.is_dead() || speed <= 0.0 {
            continue;
        }
        // The body floats over its feet, where the monster stood.
        let float_height = STEP_HEIGHT + HALF_HEIGHT;
        transform.translation.y += float_height;
        let config = configs.add(WalkerMotionConfig {
            basis: TnuaBuiltinWalkConfig {
                speed,
                float_height,
                max_slope: 50f32.to_radians(),
                ..default()
            },
        });
        commands.entity(entity).insert((
            RigidBody::Dynamic,
            Collider::cylinder(radius, 2.0 * HALF_HEIGHT),
            LockedAxes::ROTATION_LOCKED,
            TnuaController::<WalkerMotion>::default(),
            TnuaConfig::<WalkerMotion>(config),
            TnuaAvian3dSensorShape(Collider::cylinder(SENSOR_RADIUS.min(radius), 0.0)),
            Walker { speed, float_height },
            Agent3dBundle {
                agent: default(),
                settings: AgentSettings {
                    radius: radius.min(MAX_NAVIGATION_RADIUS),
                    desired_speed: speed * 0.8,
                    max_speed: speed,
                },
                archipelago_ref: ArchipelagoRef3d::new(
                    navigation.archipelago_for(radius.min(MAX_NAVIGATION_RADIUS)),
                ),
            },
            AgentTarget3d::None,
            Velocity3d::default(),
        ));
    }
}

/// Head for the party while no attack reaches it; stand otherwise, when
/// dead, and when there is no fight.
pub fn close_in(
    mut battle: Option<ResMut<Battle>>,
    party: Query<(Entity, &Position), With<PartyBody>>,
    mut walkers: Query<(Entity, &Monster, ActorPose, &Walker, &mut AgentTarget3d)>,
    data: Option<Res<GameData>>,
    species: Res<Assets<MonsterCatalog>>,
) {
    let catalog = data.and_then(|d| species.get(&d.species));
    let Ok((party, party_at)) = party.single() else { return };
    let mut approaching = false;
    let party_eye = PartyBody::eye(party_at);
    for (entity, monster, at, walker, mut target) in &mut walkers {
        let fighting =
            battle.as_ref().is_some_and(|b| b.monsters.iter().any(|&(e, _)| e == entity));
        let reach = catalog
            .and_then(|c| c.0.get(monster.species as usize))
            .and_then(|s| s.attacks.iter().map(|a| a.reach).max())
            .unwrap_or(Reach::Touch);
        let feet = crate::game::world::navigation::feet(at.translation(), Some(walker));
        let distance = super::distance_to_party(party_eye, monster, feet);
        let wanted = if fighting && !monster.is_dead() && distance > reach.metres() - REACH_MARGIN {
            AgentTarget3d::Entity(party)
        } else {
            AgentTarget3d::None
        };
        approaching |= matches!(wanted, AgentTarget3d::Entity(_));
        if !matches!((&*target, &wanted), (AgentTarget3d::None, AgentTarget3d::None))
            && !matches!((&*target, &wanted), (AgentTarget3d::Entity(a), AgentTarget3d::Entity(b)) if a == b)
        {
            *target = wanted;
        }
    }
    if let Some(battle) = &mut battle {
        battle.approaching = approaching;
    }
}
