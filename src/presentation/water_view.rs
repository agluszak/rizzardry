//! Retail's underwater view-plane sway, independent of the party's facing.
use super::party_view::PartyCamera;
use crate::game::world::triggers::LevelState;
use bevy::{
    camera::{CameraProjection, SubCameraView},
    math::Vec3A,
    prelude::*,
};

/// The original projection is the sole baseline while distortion is active.
/// Resize updates that baseline normally; the two view angles are calculated
/// only when Bevy requests a matrix or frustum. Leaving restores the baseline.
#[derive(Debug, Clone)]
struct WaterProjection {
    base: PerspectiveProjection,
    phase: Vec2,
}
impl WaterProjection {
    fn perspective(&self) -> PerspectiveProjection {
        let mut view = self.base.clone();
        let horizontal = 2.0 * ((view.fov * 0.5).tan() * view.aspect_ratio).atan();
        view.fov += self.phase.y.sin() * 0.63f32.to_radians();
        let horizontal = horizontal + self.phase.x.sin() * 0.75f32.to_radians();
        view.aspect_ratio = (horizontal * 0.5).tan() / (view.fov * 0.5).tan();
        view
    }
}
impl CameraProjection for WaterProjection {
    fn get_clip_from_view(&self) -> Mat4 {
        self.perspective().get_clip_from_view()
    }
    fn get_clip_from_view_for_sub(&self, sub: &SubCameraView) -> Mat4 {
        self.perspective().get_clip_from_view_for_sub(sub)
    }
    fn update(&mut self, width: f32, height: f32) {
        self.base.update(width, height);
    }
    fn far(&self) -> f32 {
        self.base.far
    }
    fn get_frustum_corners(&self, near: f32, far: f32) -> [Vec3A; 8] {
        self.perspective().get_frustum_corners(near, far)
    }
}

/// Set clipping planes on the actual native baseline, including during sway.
/// No second camera/settings copy needs to be kept in sync.
pub(super) fn perspective_mut(projection: &mut Projection) -> Option<&mut PerspectiveProjection> {
    match projection {
        Projection::Perspective(p) => Some(p),
        Projection::Custom(p) => p.get_mut::<WaterProjection>().map(|p| &mut p.base),
        _ => None,
    }
}

pub(super) fn sway(
    time: Res<Time<Virtual>>,
    levels: Query<&LevelState>,
    mut camera: Query<&mut Projection, With<PartyCamera>>,
) {
    let active = levels.iter().any(|state| state.camera_sway);
    for mut projection in &mut camera {
        if !active && matches!(projection.as_ref(), Projection::Perspective(_)) {
            continue;
        }
        match projection.as_mut() {
            Projection::Perspective(base) if active => {
                *projection =
                    Projection::custom(WaterProjection { base: base.clone(), phase: Vec2::ZERO });
            }
            Projection::Custom(custom) => {
                let Some(water) = custom.get_mut::<WaterProjection>() else { continue };
                if active {
                    water.phase = (water.phase
                        + Vec2::new(12f32.to_radians(), 11f32.to_radians()) * time.delta_secs())
                        % std::f32::consts::TAU;
                } else {
                    *projection = Projection::Perspective(water.base.clone());
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn water_sway_changes_both_view_angles_resizes_and_restores_without_turning_the_camera() {
        let mut app = App::new();
        app.init_resource::<Time<Virtual>>().add_systems(Update, sway);
        let root = app.world_mut().spawn(LevelState::default()).id();
        let base =
            PerspectiveProjection { fov: 70f32.to_radians(), aspect_ratio: 1.5, ..default() };
        let pose = Transform::from_rotation(Quat::from_rotation_y(0.4));
        let camera =
            app.world_mut().spawn((PartyCamera, pose, Projection::Perspective(base.clone()))).id();
        app.world_mut().get_mut::<LevelState>(root).unwrap().camera_sway = true;
        app.update();
        app.world_mut()
            .resource_mut::<Time<Virtual>>()
            .advance_by(std::time::Duration::from_secs(1));
        app.update();
        {
            let mut projection = app.world_mut().get_mut::<Projection>(camera).unwrap();
            let Projection::Custom(custom) = projection.as_mut() else {
                panic!("water projection")
            };
            let water = custom.get_mut::<WaterProjection>().unwrap();
            let before = water.get_clip_from_view();
            assert_ne!(before.x_axis.x, base.get_clip_from_view().x_axis.x);
            assert_ne!(before.y_axis.y, base.get_clip_from_view().y_axis.y);
            water.update(1920.0, 1080.0);
            assert_eq!(water.base.aspect_ratio, 1920.0 / 1080.0);
        }
        app.world_mut().despawn(root);
        app.update();
        let Projection::Perspective(restored) = app.world().get::<Projection>(camera).unwrap()
        else {
            panic!("restored")
        };
        assert_eq!(restored.fov, base.fov);
        assert_eq!(restored.aspect_ratio, 1920.0 / 1080.0);
        assert_eq!(*app.world().get::<Transform>(camera).unwrap(), pose);
    }
}
