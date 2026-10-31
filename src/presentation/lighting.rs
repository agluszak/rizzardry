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
