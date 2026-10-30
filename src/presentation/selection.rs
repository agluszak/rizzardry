//! One mesh-selection policy shared by hover and activation.

use super::controls::Interact;
use super::morph_picking::MorphRayCast;
use super::party_view::PartyCamera;
use crate::game::inventory::PartyInventory;
use crate::game::world::WorldRequest;
use crate::game::world::items::WorldItem;
use crate::game::world::level::Level;
use crate::game::world::population::Monster;
use crate::game::world::props::Prop;
use crate::game::world::triggers::Device;
use bevy::camera::visibility::RenderLayers;
use bevy::picking::hover::HoverMap;
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::picking::pointer::{PointerId, PointerLocation};
use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;

#[derive(Clone, Copy, Debug)]
pub struct WorldHit {
    pub entity: Entity,
    pub distance: f32,
}

#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct WorldSelection {
    pub level: Option<Entity>,
    pub ray: Option<Ray3d>,
    pub hit: Option<WorldHit>,
}

pub fn select_world_target(
    input: super::screen::WorldInput,
    pointers: Query<(&PointerId, &PointerLocation)>,
    camera: Query<(&Camera, &GlobalTransform), With<PartyCamera>>,
    mut meshes: MeshRayCast,
    mut morphs: MorphRayCast,
    sky: Query<(), With<RenderLayers>>,
    parents: Query<&ChildOf>,
    owners: Query<(), Or<(With<Prop>, With<WorldItem>, With<Monster>)>>,
    levels: Query<Entity, With<Level>>,
    (hovers, ui): (Option<Res<HoverMap>>, Query<(), With<Node>>),
    mut selection: ResMut<WorldSelection>,
) {
    *selection = WorldSelection::default();
    if !input.free() {
        return;
    }
    // A pointer over a control or panel drawn over the view (the spell
    // list, a device panel) points at that, not at the world behind it.
    let over_ui = hovers.as_ref().is_some_and(|hovers| {
        hovers
            .iter()
            .filter(|(id, _)| id.is_mouse())
            .any(|(_, hovered)| hovered.keys().any(|&entity| ui.contains(entity)))
    });
    if over_ui {
        return;
    }
    let Ok((camera, transform)) = camera.single() else { return };
    let Some(ray) = ray_under_pointer(&pointers, camera, transform) else { return };
    selection.level = levels.single().ok();
    selection.ray = Some(ray);
    selection.hit =
        drawn_under_ray(ray, &mut meshes, &sky, &mut morphs).and_then(|(hit, distance)| {
            Some(WorldHit { entity: owner(hit, |e| owners.contains(e), &parents)?, distance })
        });
}

pub fn activate_selection(
    interact: Single<&ActionEvents, With<Action<Interact>>>,
    selection: Res<WorldSelection>,
    props: Query<Has<Device>, With<Prop>>,
    held: Single<&PartyInventory>,
    npcs: Query<&crate::game::npcs::NpcBody>,
    mut conversations: MessageWriter<crate::game::npcs::conversation::ConversationRequest>,
    mut requests: MessageWriter<WorldRequest>,
) {
    if !interact.contains(ActionEvents::START) {
        return;
    }
    if held.hand.held.is_none()
        && let Some(hit) = selection.hit
        && let Ok(npc) = npcs.get(hit.entity)
    {
        conversations
            .write(crate::game::npcs::conversation::ConversationRequest::Begin { actor: npc.0 });
        return;
    }
    if let Some(hit) = selection.hit
        && let Ok(device) = props.get(hit.entity)
    {
        if device {
            requests.write(WorldRequest::Activate { device: hit.entity });
        }
        return;
    }
    if held.hand.held.is_some() {
        // Only where the pointer aims matters; gameplay throws from the
        // party's eye, not the drawn (perhaps shaking) camera.
        let (Some(level), Some(ray)) = (selection.level, selection.ray) else { return };
        requests.write(WorldRequest::DropHeld { level, aim: ray.direction });
    } else if let Some(hit) = selection.hit {
        requests.write(WorldRequest::TakeItem { item: hit.entity });
    }
}

/// What an entity belongs to: itself or the nearest ancestor that
/// `is_owner`.
pub fn owner(
    mut entity: Entity,
    is_owner: impl Fn(Entity) -> bool,
    parents: &Query<&ChildOf>,
) -> Option<Entity> {
    loop {
        if is_owner(entity) {
            return Some(entity);
        }
        entity = parents.get(entity).ok()?.parent();
    }
}

/// The ray through the mouse pointer, when it is over the view (retail
/// picks props and items under the cursor). Both the pointer and the
/// camera's viewport rectangle are in logical pixels of the window, which
/// is what `viewport_to_world` takes.
pub fn ray_under_pointer(
    pointers: &Query<(&PointerId, &PointerLocation)>,
    camera: &Camera,
    transform: &GlobalTransform,
) -> Option<Ray3d> {
    let (_, location) = pointers.iter().find(|(id, _)| id.is_mouse())?;
    let position = location.location()?.position;
    let view = camera.logical_viewport_rect()?;
    if !view.contains(position) {
        return None;
    }
    camera.viewport_to_world(transform, position).ok()
}

/// The mesh drawn first along a ray, and how far along the ray it is. Like
/// retail, this picks what is drawn, not what collides: a pressed pedal
/// below the floor's collider is still there to click.
pub fn drawn_under_ray(
    ray: Ray3d,
    meshes: &mut MeshRayCast,
    sky: &Query<(), With<RenderLayers>>,
    morphs: &mut MorphRayCast,
) -> Option<(Entity, f32)> {
    // The sky world is drawn by its own camera, around the world origin.
    let not_sky = |entity| !sky.contains(entity) && !morphs.contains(entity);
    let settings = MeshRayCastSettings::default().with_filter(&not_sky);
    let nearest =
        meshes.cast_ray(ray, &settings).first().map(|(entity, hit)| (*entity, hit.distance));
    morphs.nearest(ray, nearest)
}
