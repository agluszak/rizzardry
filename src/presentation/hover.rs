//! What the pointer rests on in the world, and retail's help box naming it
//! (`SetCombatSelection`, `SetCombatTarget`, `VideoToolTip`).

use bevy::camera::visibility::RenderLayers;
use bevy::picking::mesh_picking::ray_cast::MeshRayCast;
use bevy::picking::pointer::{PointerId, PointerLocation};
use bevy::prelude::*;
use bevy::time::Stopwatch;

use crate::content::items::ItemCatalog;
use crate::content::species::MonsterCatalog;
use crate::content::ui::SpriteSheet;
use crate::game::characters::Character;
use crate::game::data::GameData;
use crate::game::party::body::PartyCamera;
use crate::game::party::roster::PartyRoster;
use crate::game::world::identify::{MonsterGroups, health_known};
use crate::game::world::items::{REACH, WorldItem};
use crate::game::world::population::Monster;
use crate::game::world::props::{drawn_under_ray, owner, ray_under_pointer};
use crate::presentation::screen_art::absolute;

/// The living monster, or the lying item within reach, drawn under the
/// pointer, and how long it has been (retail `highlighted_item`,
/// `selected_item`).
#[derive(Resource, Debug, Default)]
pub struct Hover {
    entity: Option<Entity>,
    held: Stopwatch,
}

impl Hover {
    pub fn entity(&self) -> Option<Entity> {
        self.entity
    }
}

/// Retail picks the monster whose model is under the cursor, however far
/// (`PickNearestMonsterUnderCursor`), and items within 5000 units
/// (`PickNearestItemUnderCursor`).
pub fn hover(
    time: Res<Time>,
    pointers: Query<(&PointerId, &PointerLocation)>,
    camera: Query<(&Camera, &GlobalTransform), With<PartyCamera>>,
    mut meshes: MeshRayCast,
    sky: Query<(), With<RenderLayers>>,
    parents: Query<&ChildOf>,
    items: Query<&WorldItem>,
    monsters: Query<&Monster>,
    mut hovered: ResMut<Hover>,
) {
    let under = camera.single().ok().and_then(|(camera, transform)| {
        let ray = ray_under_pointer(&pointers, camera, transform)?;
        let (hit, distance) = drawn_under_ray(ray, &mut meshes, &sky)?;
        let entity = owner(hit, |e| items.contains(e) || monsters.contains(e), &parents)?;
        let wanted = match (items.get(entity), monsters.get(entity)) {
            (Ok(item), _) => distance < REACH && !item.hidden,
            (_, Ok(monster)) => !monster.is_dead(),
            _ => false,
        };
        wanted.then_some(entity)
    });
    if under != hovered.entity {
        *hovered = Hover { entity: under, held: Stopwatch::new() };
    }
    hovered.held.tick(time.delta());
}

/// Retail's help box waits the tooltip delay, at most half a second.
const TOOLTIP_DELAY: f32 = 0.5;
/// Retail raises the box by half the cursor's height: its arrow is 32
/// pixels high.
const HALF_CURSOR: f32 = 16.0;

/// The help box (retail `VideoToolTip`): Arial 10 on black inside a tan
/// border, at the pointer, kept on screen.
#[derive(Component)]
pub struct Tooltip;

pub fn show_tooltip(
    mut commands: Commands,
    hovered: Res<Hover>,
    (items, monsters, groups, roster, members): (
        Query<&WorldItem>,
        Query<&Monster>,
        Query<&MonsterGroups>,
        Res<PartyRoster>,
        Query<&Character>,
    ),
    pointers: Query<(&PointerId, &PointerLocation)>,
    scale: Res<UiScale>,
    (data, catalogs, species, sheets, assets): (
        Option<Res<GameData>>,
        Res<Assets<ItemCatalog>>,
        Res<Assets<MonsterCatalog>>,
        Res<Assets<SpriteSheet>>,
        Res<AssetServer>,
    ),
    mut images: ResMut<Assets<Image>>,
    shown: Query<Entity, With<Tooltip>>,
    mut current: Local<Option<(Entity, Vec2)>>,
    mut font: Local<Option<Handle<SpriteSheet>>>,
) {
    let font = font.get_or_insert_with(|| assets.load("retail://data/fonts/font10arial.sti"));
    let cursor = pointers
        .iter()
        .find(|(id, _)| id.is_mouse())
        .and_then(|(_, location)| location.location())
        .map(|location| (location.position / scale.0).round());
    let wanted =
        hovered.entity.filter(|_| hovered.held.elapsed_secs() >= TOOLTIP_DELAY).zip(cursor);
    if wanted == *current && (wanted.is_none() || !shown.is_empty()) {
        return;
    }
    for entity in &shown {
        commands.entity(entity).despawn();
    }
    *current = None;
    let Some((entity, at)) = wanted else { return };
    let Some(sheet) = sheets.get(&*font) else { return };
    let text = if let Ok(item) = items.get(entity) {
        // "Bullets (Unidentified) (16)".
        let catalog = data.as_ref().and_then(|d| catalogs.get(&d.items));
        let Some(definition) = catalog.and_then(|c| c.get(item.item.item)) else { return };
        item.item.display_name(definition, true)
    } else if let Ok(monster) = monsters.get(entity) {
        // "Crustacean (?/12)" (retail `SetCombatSelection`).
        let catalog = data.as_ref().and_then(|d| species.get(&d.species));
        let Some(kind) = catalog.and_then(|c| c.0.get(monster.species as usize)) else { return };
        let identified = groups.single().is_ok_and(|g| g.identified(monster.group));
        let party: Vec<&Character> = roster.0.iter().filter_map(|&m| members.get(m).ok()).collect();
        let health = health_known(&party, monster.hp, monster.max_hp, kind.display_level);
        format!("{} ({health})", kind.shown_name(identified, 1))
    } else {
        return;
    };
    let image = sheet.text_image(&text);
    let size = image.size_f32() + Vec2::new(4.0, 2.0);
    let corner = Vec2::new(at.x.max(2.0), (at.y - HALF_CURSOR).max(2.0))
        .min(Vec2::new(640.0, 480.0) - size - 2.0);
    commands
        .spawn((
            Tooltip,
            BackgroundColor(Color::BLACK),
            BorderColor::all(Color::srgb(0.93, 0.6, 0.33)),
            Node { border: UiRect::all(Val::Px(1.0)), ..absolute(corner, size) },
            GlobalZIndex(30),
            Pickable::IGNORE,
        ))
        .with_child((
            ImageNode::new(images.add(image)),
            Node {
                margin: UiRect::new(Val::Px(1.0), Val::ZERO, Val::ZERO, Val::ZERO),
                ..default()
            },
            Pickable::IGNORE,
        ));
    *current = Some((entity, at));
}
