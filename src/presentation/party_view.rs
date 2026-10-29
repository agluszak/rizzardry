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
    preferences: Res<crate::presentation::settings::Preferences>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::combat::CombatNews;
    use crate::game::party::body::{PartyTurning, turn_party};
    use crate::game::world::population::{Monster, ScaledBody};
    use crate::presentation::monsters::MonsterVisual;
    use crate::presentation::screen::Screen;
    use crate::presentation::settings::Preferences;
    use avian3d::prelude::Position;
    use std::time::Duration;

    /// A party facing -Z and a drawn monster off to its right (+X); a fight
    /// begins with it nearest, under the given settings, and the game runs
    /// for `seconds` in frames of `frame`, scheduled as in the game: the
    /// request after the fight's update, the turn on the fixed step. Returns
    /// the party's yaw every quarter second.
    fn turned(
        tracking: ViewTracking,
        turning: ViewTurning,
        frame: Duration,
        seconds: u32,
    ) -> Vec<f32> {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<crate::content::level::LevelDefinition>>()
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(frame))
            .insert_resource(State::new(Screen::World))
            .init_resource::<NextState<Screen>>()
            .init_resource::<PartyTurning>()
            .insert_resource(Preferences {
                view_tracking: tracking,
                view_turning: turning,
                ..default()
            })
            .add_message::<TurnParty>()
            .add_message::<CombatNews>()
            .add_systems(Update, track_fights)
            .add_systems(FixedUpdate, turn_party);
        let geometry = app
            .world_mut()
            .resource_mut::<Assets<crate::content::level::LevelDefinition>>()
            .add(crate::content::level::LevelDefinition::default());
        app.world_mut().spawn(crate::game::world::level::Level::test(geometry));
        let party = app.world_mut().spawn((PartyBody::default(), Position::default())).id();
        let monster = app
            .world_mut()
            .spawn((
                Monster {
                    enchantments: default(),
                    location: 1,
                    species: 0,
                    hp: 10,
                    max_hp: 10,
                    stamina: 0,
                    max_stamina: 0,
                    conditions: vec![],
                    spell_points: 0,
                    body: ScaledBody {
                        scale: 1.0,
                        radius: 0.5,
                        eye_height: EYE_ABOVE_CENTRE,
                        speed: 0.0,
                    },
                },
                Transform::from_xyz(5.0, 0.0, 0.0),
                MonsterVisual(default()),
            ))
            .id();
        app.world_mut().write_message(CombatNews::Began { nearest: Some(monster) });
        // The first frame has no time in it.
        app.update();
        let per_sample = (Duration::from_millis(250).as_nanos() / frame.as_nanos()) as u32;
        (0..seconds * 4)
            .map(|_| {
                for _ in 0..per_sample {
                    app.update();
                }
                app.world().get::<PartyBody>(party).unwrap().yaw
            })
            .collect()
    }

    /// Off, the party keeps facing ahead; snapping turns it to the monster
    /// at once; smoothly, it gets there over the time retail gives a
    /// quarter turn (five seconds), passing through the angles between, and
    /// at every moment faces the same way however the frames fall.
    #[test]
    fn a_fight_turns_the_party_as_the_settings_say() {
        let quarter = Duration::from_millis(250);
        let right = -std::f32::consts::FRAC_PI_2;
        let off = turned(ViewTracking::Off, ViewTurning::Snap, quarter, 1);
        assert!(off.iter().all(|&yaw| yaw == 0.0));
        let mine = turned(ViewTracking::SelectedMember, ViewTurning::Snap, quarter, 1);
        assert!(mine.iter().all(|&yaw| yaw == 0.0), "nobody goes for the selected member yet");
        let snap = turned(ViewTracking::AllTargets, ViewTurning::Snap, quarter, 1);
        assert!((snap[0] - right).abs() < 1e-4, "{snap:?}");
        let smooth = turned(ViewTracking::AllTargets, ViewTurning::Smooth, quarter, 6);
        assert!(smooth[3] < 0.0 && smooth[3] > right, "partway: {smooth:?}");
        assert!((smooth[19] - right).abs() < 1e-4, "there after five seconds: {smooth:?}");
        assert!(smooth[18] > right + 1e-3, "not before: {smooth:?}");
        assert_eq!(smooth[19], smooth[23]);
        let fine = turned(
            ViewTracking::AllTargets,
            ViewTurning::Smooth,
            Duration::from_nanos(15_625_000),
            6,
        );
        assert_eq!(fine, smooth, "frames change nothing");
    }
}
