//! Dynamic contributions use live fragment positions; baked mesh colours stay immutable.
use super::environment::LevelAppearance;
use crate::{content::level::LevelGeometry, game::world::triggers::LightActivation};
use bevy::{
    asset::embedded_asset,
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::{
        render_resource::AsBindGroup,
        storage::{ShaderBuffer, ShaderBufferData},
    },
    shader::ShaderRef,
};

pub type SceneMaterial = ExtendedMaterial<StandardMaterial, SceneLighting>;

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone, Default)]
pub struct SceneLighting {
    #[uniform(100)]
    pub compiled: Vec4,
    #[storage(101, read_only)]
    pub lights: Handle<ShaderBuffer>,
}
impl MaterialExtension for SceneLighting {
    fn fragment_shader() -> ShaderRef {
        "embedded://rizzardry/presentation/lighting.wesl".into()
    }
}
#[derive(Resource)]
pub struct SceneLights(pub Handle<ShaderBuffer>);

pub fn plugin(app: &mut App) {
    embedded_asset!(app, "lighting.wesl");
    app.add_plugins(MaterialPlugin::<SceneMaterial>::default());
    let buffer = app
        .world_mut()
        .resource_mut::<Assets<ShaderBuffer>>()
        .add(ShaderBuffer::from(vec![[[0.0f32; 4]; 3]]));
    app.insert_resource(SceneLights(buffer)).add_systems(Update, update_lights);
}

/// Only a GPU projection of current game inputs, never another light-state owner.
/// Rebuilding this small buffer covers initialization, motion, expiry, removal,
/// travel and asset replacement without a separate invalidation flag/cache.
fn update_lights(
    levels: Query<(&LevelAppearance, Option<&LightActivation>)>,
    geometry: Res<Assets<LevelGeometry>>,
    torch: super::environment::PartyTorch,
    scene: Res<SceneLights>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
) {
    let mut data = Vec::new();
    if let Some((level, state)) = levels
        .iter()
        .find_map(|(appearance, state)| geometry.get(&appearance.0).map(|level| (level, state)))
    {
        for light in &level.lighting.lights {
            let active = state
                .map_or(light.enabled, |state| state.light_enabled(&light.name, light.enabled));
            let props = i32::from(active);
            let compiled = i32::from(active) - i32::from(light.enabled && !light.runtime);
            if props == 0 && compiled == 0 {
                continue;
            }
            data.push([
                light.position.extend(light.range).to_array(),
                light.colour.extend(0.0).to_array(),
                [props as f32, compiled as f32, 0.0, 0.0],
            ]);
        }
        if let Some(light) = torch.light() {
            data.push([
                light.position.extend(light.range).to_array(),
                light.colour.extend(0.0).to_array(),
                [1.0, 1.0, 0.0, 0.0],
            ]);
        }
    }
    // Storage bindings need nonzero allocation; the zero-range sentinel contributes nothing.
    if data.is_empty() {
        data.push([[0.0; 4]; 3]);
    }
    let next = ShaderBuffer::from(data);
    let equal = buffers.get(&scene.0).is_some_and(|current| match (&current.data, &next.data) {
        (
            ShaderBufferData::Initialized { data: a, .. },
            ShaderBufferData::Initialized { data: b, .. },
        ) => a.as_slice() == b.as_slice(),
        _ => false,
    });
    if !equal {
        buffers.insert(scene.0.id(), next).expect("scene light buffer remains registered");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{magic::enchantments::PartyEffects, party::body::PartyBody};
    use crate::{
        content::spells::SpellId,
        game::{magic::enchantments::Enchantment, world::triggers::LevelState},
        presentation::{environment::apply_environment, party_view::PartyCamera},
    };

    #[test]
    fn moving_a_prop_then_disabling_its_original_lamp_leaves_no_negative_rest_pose_correction() {
        use crate::{
            content::level::{PointLightSource, StateChange, TriggerAction},
            game::world::{
                scripts::LevelScript,
                triggers::tests::{Scenario, trigger},
            },
        };
        let off = trigger(
            "OFF",
            TriggerAction::Lights {
                sources: vec![("LAMP".into(), true)],
                change: StateChange::Clear,
            },
        );
        let mut scene = Scenario::new(vec![off], &[], LevelScript::None);
        scene.world.init_resource::<Assets<LevelGeometry>>();
        scene.world.init_resource::<Assets<ShaderBuffer>>();
        let geometry = scene.world.resource_mut::<Assets<LevelGeometry>>().add(LevelGeometry {
            lighting: crate::content::level::LevelLighting {
                lights: vec![PointLightSource {
                    name: "LAMP".into(),
                    runtime: false,
                    enabled: true,
                    position: Vec3::Y,
                    colour: Vec3::ONE,
                    range: 3.0,
                }],
                ..default()
            },
            ..default()
        });
        scene.world.entity_mut(scene.root).insert(LevelAppearance(geometry));
        let buffer = scene
            .world
            .resource_mut::<Assets<ShaderBuffer>>()
            .add(ShaderBuffer::from(vec![[[0.0f32; 4]; 3]]));
        scene.world.insert_resource(SceneLights(buffer.clone()));
        let prop = scene.world.spawn(Transform::default()).id();
        scene.world.run_system_cached(update_lights).unwrap();
        let data = |world: &World| match &world
            .resource::<Assets<ShaderBuffer>>()
            .get(&buffer)
            .unwrap()
            .data
        {
            ShaderBufferData::Initialized { data, .. } => data.as_slice().to_vec(),
            _ => unreachable!(),
        };
        let active = data(&scene.world);
        assert_eq!(
            f32::from_le_bytes(active[32..36].try_into().unwrap()),
            1.0,
            "props receive the complete live light"
        );
        *scene.world.get_mut::<Transform>(prop).unwrap() =
            Transform::from_xyz(20.0, 0.0, 0.0).with_rotation(Quat::from_rotation_z(1.0));
        scene.world.run_system_cached(update_lights).unwrap();
        assert_eq!(data(&scene.world), active, "world transforms are sampled by the shader");
        assert!(scene.run("OFF").0);
        scene.world.run_system_cached(update_lights).unwrap();
        let disabled = data(&scene.world);
        assert_eq!(
            f32::from_le_bytes(disabled[32..36].try_into().unwrap()),
            0.0,
            "props need neither a light nor a rest-pose subtraction"
        );
        assert_eq!(
            f32::from_le_bytes(disabled[36..40].try_into().unwrap()),
            -1.0,
            "immobile compiled geometry still cancels its original bake"
        );
    }

    #[test]
    fn movement_updates_only_the_light_buffer_and_stationary_light_does_not_rewrite_assets() {
        let mut app = App::new();
        app.init_resource::<Assets<LevelGeometry>>()
            .init_resource::<Assets<ShaderBuffer>>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<ClearColor>()
            .init_resource::<crate::game::time::GameClock>()
            .add_systems(Update, (update_lights, apply_environment));
        let buffer = app
            .world_mut()
            .resource_mut::<Assets<ShaderBuffer>>()
            .add(ShaderBuffer::from(vec![[[0.0f32; 4]; 3]]));
        app.insert_resource(SceneLights(buffer.clone()));
        let mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, default()));
        let level = app.world_mut().resource_mut::<Assets<LevelGeometry>>().add(LevelGeometry {
            parts: vec![crate::content::level::LevelPart {
                mesh: mesh.clone(),
                surface: crate::content::surface::Surface {
                    material: default(),
                    ambient: Vec3::ONE,
                    emission: 0.0,
                    animation: default(),
                },
                lighting: default(),
            }],
            environment: Some(crate::content::level::LevelEnvironment {
                fog_enabled: true,
                fog_range: (0.2, 1.0),
                view_distance: 80.0,
                intensity: 1.0,
                fog_colours: vec![Vec3::ONE; 256],
                ambient_colours: vec![Vec3::ONE; 256],
            }),
            ..default()
        });
        let root = app
            .world_mut()
            .spawn((LevelAppearance(level), LightActivation::default(), LevelState::default()))
            .id();
        app.world_mut().spawn((PartyCamera, Projection::default()));
        let mut effects = PartyEffects::default();
        effects.0.apply(Enchantment {
            spell: SpellId::LIGHT,
            power: 3,
            boost: 0,
            turns: 10,
            absorption: 0,
        });
        let party = app
            .world_mut()
            .spawn((PartyBody::default(), avian3d::prelude::Position::default(), effects))
            .id();
        app.update();
        let data = |app: &App| match &app
            .world()
            .resource::<Assets<ShaderBuffer>>()
            .get(&buffer)
            .unwrap()
            .data
        {
            ShaderBufferData::Initialized { data, .. } => data.as_slice().to_vec(),
            _ => panic!("initialized light buffer"),
        };
        let initial = data(&app);
        app.update();
        let tick = app.world().resource_ref::<Assets<ShaderBuffer>>().last_changed();
        app.update();
        assert_eq!(tick, app.world().resource_ref::<Assets<ShaderBuffer>>().last_changed());
        let mesh_tick = app.world().resource_ref::<Assets<Mesh>>().last_changed();
        app.world_mut().get_mut::<LevelState>(root).unwrap().arrive(Vec3::X);
        app.world_mut().get_mut::<avian3d::prelude::Position>(party).unwrap().0 += Vec3::X;
        app.update();
        assert_ne!(data(&app), initial);
        assert_eq!(mesh_tick, app.world().resource_ref::<Assets<Mesh>>().last_changed());
        app.world_mut().get_mut::<PartyEffects>(party).unwrap().0.0.clear();
        app.update();
        assert_ne!(data(&app), initial);
        assert_eq!(mesh_tick, app.world().resource_ref::<Assets<Mesh>>().last_changed());
    }
}
