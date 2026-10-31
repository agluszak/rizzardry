//! The rendering camera follows the authoritative party body.

use bevy::prelude::*;

use crate::content::UNITS_PER_METRE;
use crate::content::level::DEFAULT_VIEW_DISTANCE;
use crate::game::party::body::{EYE_ABOVE_CENTRE, PartyBody, TurnParty};

/// The first-person camera on the party's body.
#[derive(Component, Debug)]
pub struct PartyCamera;

/// Put the camera on the party's body when it appears.
pub fn attach_camera(add: On<Add<PartyBody>>, mut commands: Commands) {
    let party = add.entity;
    commands.entity(party).insert(Visibility::default());
    commands.spawn((
        Name::new("Party camera"),
        PartyCamera,
        SpatialListener::new(0.2),
        Camera3d::default(),
        // The sky camera clears the frame and draws first.
        Camera { clear_color: ClearColorConfig::None, ..default() },
        Projection::Perspective(PerspectiveProjection {
            fov: 70f32.to_radians(),
            // Retail's near clip of 250 units; the level sets the far one.
            near: 250.0 / UNITS_PER_METRE,
            far: DEFAULT_VIEW_DISTANCE,
            ..default()
        }),
        Transform::from_xyz(0.0, EYE_ABOVE_CENTRE, 0.0),
        ChildOf(party),
    ));
}

/// Point the camera along the party's facing.
pub fn aim_camera(
    party: Query<(&PartyBody, crate::game::world::ActorPose), Without<PartyCamera>>,
    conversations: Query<
        &crate::game::npcs::conversation::Conversation,
        Without<crate::game::npcs::conversation::Closing>,
    >,
    npc_bodies: Query<&crate::game::npcs::NpcBodies>,
    actors: Query<
        (
            crate::game::world::ActorPose,
            &crate::game::world::population::Monster,
            Option<&crate::game::world::navigation::Walker>,
        ),
        Without<PartyCamera>,
    >,
    mut camera: Query<&mut Transform, With<PartyCamera>>,
) {
    if let (Ok((body, pose)), Ok(mut camera)) = (party.single(), camera.single_mut()) {
        let ordinary = Quat::from_rotation_y(body.yaw) * Quat::from_rotation_x(body.pitch);
        let rotation = conversations
            .single()
            .ok()
            .and_then(|c| npc_bodies.get(c.actor).ok())
            .and_then(|bodies| bodies.iter().find_map(|body| actors.get(body).ok()))
            .map_or(ordinary, |(target, monster, walker)| {
                let direction = monster
                    .eye(crate::game::world::navigation::feet(target.translation(), walker))
                    - (pose.translation() + camera.translation);
                if direction.length_squared() < 0.0001 {
                    ordinary
                } else {
                    Transform::IDENTITY.looking_to(direction, Vec3::Y).rotation
                }
            });
        if camera.rotation != rotation {
            camera.rotation = rotation;
        }
    }
}

/// Whom the view turns to in a fight (retail `camera_rotation_mode`; new
/// games track every target, `Gameplay Init.cpp`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ViewTracking {
    Off,
    /// Only a monster going for the selected member.
    SelectedMember,
    #[default]
    AllTargets,
}

/// How the view turns (retail `camera_rotation_style`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ViewTurning {
    #[default]
    Smooth,
    Snap,
}

/// When a fight begins, turn the party to the nearest monster in it, as the
/// player's settings say (retail `StartCombat`'s `PointCameraAtMonster`):
/// not with tracking off, nor in selected-member mode (no monster goes for
/// anyone yet), nor at one not drawn. The game carries the turn out.
pub fn track_fights(
    mut news: MessageReader<crate::game::combat::CombatNews>,
    preferences: Res<crate::settings::Preferences>,
    party: Query<&avian3d::prelude::Position, With<PartyBody>>,
    monsters: Query<
        (
            crate::game::world::ActorPose,
            &crate::game::world::population::Monster,
            Option<&crate::game::world::navigation::Walker>,
        ),
        With<crate::presentation::monsters::MonsterVisual>,
    >,
    mut turns: MessageWriter<TurnParty>,
) {
    for notice in news.read() {
        let crate::game::combat::CombatNews::Began { nearest: Some(nearest) } = *notice else {
            continue;
        };
        if preferences.view_tracking != ViewTracking::AllTargets {
            continue;
        }
        let (Ok(at), Ok((pose, monster, walker))) = (party.single(), monsters.get(nearest)) else {
            continue;
        };
        let aim = monster.eye(crate::game::world::navigation::feet(pose.translation(), walker));
        let toward = aim - PartyBody::eye(at);
        turns.write(TurnParty {
            to: Vec2::new(
                crate::game::party::body::yaw_toward(toward),
                toward.y.atan2(toward.xz().length()),
            ),
            smooth: preferences.view_turning == ViewTurning::Smooth,
        });
    }
}
