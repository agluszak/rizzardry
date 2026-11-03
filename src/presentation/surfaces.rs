//! Asset surfaces become rendered materials; actor light is a uniform, not geometry.
use crate::content::surface::Surface;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

impl Surface {
    pub fn render(&self) -> impl Bundle {
        (MeshMaterial3d(self.material.clone()), self.animation.clone())
    }

    /// Per-level render instances share their source and bake policy. The
    /// dynamic contribution is sampled at the live GPU world pose.
    pub(super) fn dynamic_instance(
        &self,
        compiled: bool,
        lights: &Handle<bevy::render::storage::ShaderBuffer>,
        instances: &mut HashMap<
            (AssetId<StandardMaterial>, bool),
            Handle<super::lighting::SceneMaterial>,
        >,
        source: &Assets<StandardMaterial>,
        materials: &mut Assets<super::lighting::SceneMaterial>,
    ) -> Option<impl Bundle> {
        let handle = match instances.get(&(self.material.id(), compiled)) {
            Some(handle) => handle.clone(),
            None => {
                let handle = materials.add(super::lighting::SceneMaterial {
                    base: source.get(&self.material)?.clone(),
                    extension: super::lighting::SceneLighting {
                        compiled: if compiled { Vec4::X } else { Vec4::ZERO },
                        lights: lights.clone(),
                    },
                });
                instances.insert((self.material.id(), compiled), handle.clone());
                handle
            }
        };
        Some((MeshMaterial3d(handle), self.animation.clone()))
    }

    /// Each independently lit visual owns its materials, sharing textures/meshes.
    /// Matching parts within that visual reuse one instance of their source.
    pub fn lit_instance(
        &self,
        source: Entity,
        instances: &mut HashMap<AssetId<StandardMaterial>, Handle<StandardMaterial>>,
        materials: &mut Assets<StandardMaterial>,
    ) -> Option<impl Bundle> {
        let material = match instances.get(&self.material.id()) {
            Some(material) => material.clone(),
            None => {
                let material = materials.add(materials.get(&self.material)?.clone());
                instances.insert(self.material.id(), material.clone());
                material
            }
        };
        Some((
            MeshMaterial3d(material),
            self.animation.clone(),
            LightFrom(source),
            SurfaceResponse { ambient: self.ambient, emission: self.emission },
        ))
    }
}

#[derive(Component)]
#[relationship(relationship_target = LitSurfaces)]
pub struct LightFrom(pub Entity);

#[derive(Component, Default)]
#[relationship_target(relationship = LightFrom)]
pub struct LitSurfaces(Vec<Entity>);

#[derive(Component)]
pub struct SurfaceResponse {
    pub ambient: Vec3,
    pub emission: f32,
}

/// The light an actor's surfaces are tinted by: the sky light reaching it
/// and the fixed light on it, composed by [`light_actors`] from its
/// [`LightSample`] and [`SkyResponse`].
#[derive(Component, Default, PartialEq)]
pub struct ActorLight {
    pub sky: Vec3,
    pub fixed: Vec3,
}

/// What the level gives an actor where it was last lit: whether its `SUN`
/// light is in sight, and the fixed light (offset, nearby lights) there.
/// Sampled anew when the actor moves or the level's lights change.
#[derive(Component, Default, Debug)]
pub struct LightSample(Option<(Vec3, bool, Vec3)>);

impl LightSample {
    /// The sample at `at`, taken afresh when it is stale.
    fn at(
        &mut self,
        at: Vec3,
        stale: bool,
        sample: impl FnOnce(Vec3) -> (bool, Vec3),
    ) -> (bool, Vec3) {
        match self.0 {
            Some((lit, sunlit, fixed)) if lit == at && !stale => (sunlit, fixed),
            _ => {
                let (sunlit, fixed) = sample(at);
                self.0 = Some((at, sunlit, fixed));
                (sunlit, fixed)
            }
        }
    }
}

/// How an actor takes the sky light (retail's `light_scale`).
#[derive(Component, Debug)]
pub enum SkyResponse {
    /// A monster fades toward 0.75 in sight of the sun and 0 out of it
    /// (Whizzardry `Monster::Update`).
    Fading { scale: f32 },
    /// An item takes all of it at once, or none.
    Immediate,
}

impl SkyResponse {
    pub fn fading() -> Self {
        Self::Fading { scale: SUNLIT }
    }
}

/// The sky light an actor in sight of the sun receives.
const SUNLIT: f32 = 0.75;
/// Retail steps the sky scale by 0.05 every 25 ms.
const SKY_SCALE_PER_SECOND: f32 = 2.0;
/// A full rise and fall of the item highlight: 15 steps up and 15 down,
/// 50 ms each.
const PULSE_SECONDS: f32 = 1.5;

/// Light every actor as retail does models without baked light: sky light
/// while it can see the `SUN` light, by its response and each material's
/// ambient, plus the level's offset, nearby lights and the party's torch.
/// Retail adds point lights per vertex with N·L; here they light the whole
/// actor by their falloff at its centre. The hovered item pulses green:
/// retail raises and lowers its level by 0.05 every 50 ms between 0.25 and
/// 1; how its renderer mixes the highlight in is not recovered, so here
/// it adds green light.
pub fn light_actors(
    time: Res<Time>,
    torch: super::environment::PartyTorch,
    hovered: Res<super::hover::Hover>,
    clock: Res<crate::game::time::GameClock>,
    levels: Query<(
        Entity,
        &super::environment::LevelAppearance,
        Option<Ref<crate::game::world::triggers::LightActivation>>,
    )>,
    geometry: Res<Assets<crate::content::level::LevelGeometry>>,
    spatial: avian3d::prelude::SpatialQuery,
    mut actors: Query<(
        Entity,
        &GlobalTransform,
        &mut LightSample,
        &mut SkyResponse,
        &mut ActorLight,
        Has<crate::game::world::items::WorldItem>,
    )>,
) {
    let Some((level_entity, level, state)) =
        levels.iter().find_map(|(e, l, state)| geometry.get(&l.0).map(|g| (e, g, state)))
    else {
        return;
    };
    let sky = super::environment::sky_now(level, &clock);
    let torch = torch.light();
    let stale = geometry.is_changed() || state.as_ref().is_some_and(|s| s.is_changed());
    let phase = (time.elapsed_secs() / PULSE_SECONDS).fract();
    let pulse = 0.25 + 0.75 * (1.0 - (2.0 * phase - 1.0).abs());
    for (entity, transform, mut sample, mut response, mut light, item) in &mut actors {
        let at = transform.translation();
        let (sunlit, fixed) = sample.at(at, stale, |at| {
            (
                sunlit(&spatial, level_entity, level, at),
                super::environment::actor_light(&level.lighting, state.as_deref(), at),
            )
        });
        let scale = match &mut *response {
            SkyResponse::Fading { scale } => {
                let target = if sunlit { SUNLIT } else { 0.0 };
                if *scale != target {
                    let step = SKY_SCALE_PER_SECOND * time.delta_secs();
                    *scale += (target - *scale).clamp(-step, step);
                }
                *scale
            }
            SkyResponse::Immediate => {
                if sunlit {
                    1.0
                } else {
                    0.0
                }
            }
        };
        let highlight =
            if item && hovered.entity() == Some(entity) { Vec3::Y * pulse } else { Vec3::ZERO };
        light.set_if_neq(ActorLight {
            sky: sky * scale,
            fixed: fixed + highlight + torch.as_ref().map_or(Vec3::ZERO, |light| light.at(at)),
        });
    }
}

/// Whether the level's `SUN` light is in sight from `at`, past the level's
/// own geometry. A level without one has no sunlight.
pub fn sunlit(
    spatial: &avian3d::prelude::SpatialQuery,
    level_entity: Entity,
    level: &crate::content::level::LevelGeometry,
    at: Vec3,
) -> bool {
    level.lighting.sun.is_some_and(|sun| {
        let Ok(direction) = Dir3::new(sun - at) else { return true };
        spatial
            .cast_ray_predicate(
                at,
                direction,
                at.distance(sun),
                true,
                &avian3d::prelude::SpatialQueryFilter::default(),
                &|hit| hit == level_entity,
            )
            .is_none()
    })
}

pub fn tint_surfaces(
    sources: Query<&ActorLight>,
    surfaces: Query<(&LightFrom, &SurfaceResponse, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (source, response, handle) in &surfaces {
        let Ok(light) = sources.get(source.0) else { continue };
        let rgb = light.sky * response.ambient + light.fixed + Vec3::splat(response.emission);
        let Some(material) = materials.get(&handle.0) else { continue };
        let tint = Color::linear_rgba(rgb.x, rgb.y, rgb.z, material.base_color.alpha());
        if material.base_color != tint {
            materials.get_mut(&handle.0).unwrap().base_color = tint;
        }
    }
}
