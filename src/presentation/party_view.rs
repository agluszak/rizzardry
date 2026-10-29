//! The rendering camera follows the authoritative party body.

use bevy::prelude::*;

use crate::content::UNITS_PER_METRE;
use crate::content::level::DEFAULT_VIEW_DISTANCE;
use crate::game::party::body::{EYE_ABOVE_CENTRE, MAX_PITCH, PartyBody, PartyLook};

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
    conversations: Query<&crate::game::npcs::conversation::Conversation>,
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
                let direction = crate::game::world::navigation::feet(target.translation(), walker)
                    + Vec3::Y * monster.body.eye_height
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

/// A view turn under way (retail `GDCamera`'s orientation transition): from
/// where the party looked to where it is turning, as yaw and pitch, eased
/// over the time retail gives the larger of the two angles. Turning the
/// view by hand ends it.
#[derive(Resource, Debug, Default)]
pub struct ViewTurn(Option<Turning>);

#[derive(Debug)]
struct Turning {
    from: Vec2,
    to: Vec2,
    duration: f32,
    elapsed: f32,
    /// Where this turn last left the view.
    last: Vec2,
}

/// Retail's turn takes ten seconds per half turn of the larger angle.
const SECONDS_PER_RADIAN: f32 = 10.0 / std::f32::consts::PI;

/// An angle brought within half a turn either way.
fn wrapped(angle: f32) -> f32 {
    (angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

/// When a fight begins, turn the view to the nearest monster in it, as the
/// player's settings say (retail `StartCombat`'s `PointCameraAtMonster`):
/// not with tracking off, nor in selected-member mode (no monster goes for
/// anyone yet), nor at one not drawn. The turn is the player's look, given
/// as any other look is ([`PartyLook`]).
pub fn track_fights(
    mut news: MessageReader<crate::game::combat::CombatNews>,
    preferences: Res<crate::presentation::settings::Preferences>,
    party: Query<(&PartyBody, &avian3d::prelude::Position)>,
    monsters: Query<
        (
            crate::game::world::ActorPose,
            &crate::game::world::population::Monster,
            Option<&crate::game::world::navigation::Walker>,
        ),
        With<crate::presentation::monsters::MonsterVisual>,
    >,
    mut turn: ResMut<ViewTurn>,
    mut looks: MessageWriter<PartyLook>,
) {
    for notice in news.read() {
        let crate::game::combat::CombatNews::Began { nearest: Some(nearest) } = *notice else {
            continue;
        };
        if preferences.view_tracking != ViewTracking::AllTargets {
            continue;
        }
        let (Ok((body, at)), Ok((pose, monster, walker))) = (party.single(), monsters.get(nearest))
        else {
            continue;
        };
        let aim = crate::game::world::navigation::feet(pose.translation(), walker)
            + Vec3::Y * monster.body.eye_height;
        let toward = aim - PartyBody::eye(at);
        let yaw = (-toward.x).atan2(-toward.z);
        let pitch = toward.y.atan2(toward.xz().length()).clamp(-MAX_PITCH, MAX_PITCH);
        let from = Vec2::new(body.yaw, body.pitch);
        let to = Vec2::new(body.yaw + wrapped(yaw - body.yaw), pitch);
        match preferences.view_turning {
            ViewTurning::Snap => {
                looks.write(PartyLook(to - from));
            }
            ViewTurning::Smooth => {
                let duration = (to - from).abs().max_element() * SECONDS_PER_RADIAN;
                turn.0 = Some(Turning { from, to, duration, elapsed: 0.0, last: from });
            }
        }
    }
}

/// Carry a smooth view turn on, easing in and out, until it is done or the
/// player turns the view another way.
pub fn turn_view(
    time: Res<Time>,
    party: Query<&PartyBody>,
    mut turn: ResMut<ViewTurn>,
    mut looks: MessageWriter<PartyLook>,
) {
    let Some(turning) = turn.0.as_mut() else { return };
    let Ok(body) = party.single() else {
        turn.0 = None;
        return;
    };
    let now = Vec2::new(body.yaw, body.pitch);
    if (now - turning.last).abs().max_element() > 1e-4 {
        turn.0 = None;
        return;
    }
    turning.elapsed += time.delta_secs();
    let share = (turning.elapsed / turning.duration.max(f32::EPSILON)).min(1.0);
    let eased = (1.0 - (share * std::f32::consts::PI).cos()) / 2.0;
    let wanted = turning.from + (turning.to - turning.from) * eased;
    if wanted != now {
        looks.write(PartyLook(wanted - now));
    }
    turning.last = wanted;
    if share >= 1.0 {
        turn.0 = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::combat::CombatNews;
    use crate::game::party::body::look_around;
    use crate::game::screen::Screen;
    use crate::game::world::population::{Monster, ScaledBody};
    use crate::presentation::monsters::MonsterVisual;
    use crate::presentation::settings::Preferences;
    use avian3d::prelude::Position;

    /// A party facing -Z and a drawn monster off to its right (+X); a fight
    /// begins with it nearest, under the given settings. Returns the party's
    /// yaw after each of `frames` quarter-second frames.
    fn turned(tracking: ViewTracking, turning: ViewTurning, frames: usize) -> Vec<f32> {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<crate::content::level::LevelDefinition>>()
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(250),
            ))
            .insert_resource(State::new(Screen::World))
            .init_resource::<NextState<Screen>>()
            .init_resource::<ViewTurn>()
            .insert_resource(Preferences {
                view_tracking: tracking,
                view_turning: turning,
                ..default()
            })
            .add_message::<PartyLook>()
            .add_message::<CombatNews>()
            .add_systems(Update, (track_fights, turn_view, look_around).chain());
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
        (0..frames)
            .map(|_| {
                app.update();
                app.world().get::<PartyBody>(party).unwrap().yaw
            })
            .collect()
    }

    /// Off, the party keeps facing ahead; snapping turns it to the monster
    /// at once; smoothly, it gets there over the time retail gives a
    /// quarter turn (five seconds), passing through the angles between.
    #[test]
    fn a_fight_turns_the_view_as_the_settings_say() {
        let right = -std::f32::consts::FRAC_PI_2;
        let off = turned(ViewTracking::Off, ViewTurning::Snap, 3);
        assert!(off.iter().all(|&yaw| yaw == 0.0));
        let mine = turned(ViewTracking::SelectedMember, ViewTurning::Snap, 3);
        assert!(mine.iter().all(|&yaw| yaw == 0.0), "nobody goes for the selected member yet");
        let snap = turned(ViewTracking::AllTargets, ViewTurning::Snap, 2);
        assert!((snap[0] - right).abs() < 1e-4, "{snap:?}");
        let smooth = turned(ViewTracking::AllTargets, ViewTurning::Smooth, 24);
        assert!(smooth[3] < 0.0 && smooth[3] > right, "partway: {smooth:?}");
        // The first frame has no time in it.
        assert!((smooth[20] - right).abs() < 1e-4, "there after five seconds: {smooth:?}");
        assert!(smooth[19] > right, "not before: {smooth:?}");
        assert_eq!(smooth[20], smooth[23]);
    }
}
