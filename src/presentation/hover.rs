//! What the pointer rests on in the world, and retail's help box naming it
//! (`SetCombatSelection`, `SetCombatTarget`, `VideoToolTip`).

use bevy::picking::pointer::{PointerId, PointerLocation};
use bevy::prelude::*;
use bevy::time::Stopwatch;

use crate::content::ui::SpriteSheet;
use crate::game::characters::{Character, Vitals};
use crate::game::party::roster::PartyRoster;
use crate::game::world::identify::{GroupMember, MonsterGroup, health_known};
use crate::game::world::items::{REACH, WorldItem};
use crate::game::world::population::Monster;
use crate::presentation::screen_art::{BitmapText, absolute};

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
    selection: Res<super::selection::WorldSelection>,
    items: Query<&WorldItem>,
    monsters: Query<&Monster>,
    mut hovered: ResMut<Hover>,
) {
    let under = selection.hit.and_then(|hit| {
        let wanted = match (items.get(hit.entity), monsters.get(hit.entity)) {
            (Ok(item), _) => hit.distance < REACH && !item.hidden,
            (_, Ok(monster)) => !monster.is_dead(),
            _ => false,
        };
        wanted.then_some(hit.entity)
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
#[derive(Component, Clone, Default)]
pub struct Tooltip;

pub fn show_tooltip(
    mut commands: Commands,
    hovered: Res<Hover>,
    (items, monsters, groups, memberships, roster, members): (
        Query<&WorldItem>,
        Query<&Monster>,
        Query<&MonsterGroup>,
        Query<&GroupMember>,
        Single<&PartyRoster>,
        Query<(&Character, &Vitals, Has<crate::game::npcs::Npc>)>,
    ),
    pointers: Query<(&PointerId, &PointerLocation)>,
    scale: Res<UiScale>,
    cameras: Query<&Camera, With<bevy::ui::IsDefaultUiCamera>>,
    (content, sheets, assets): (
        If<Res<crate::game::data::Catalogs>>,
        Res<Assets<SpriteSheet>>,
        Res<AssetServer>,
    ),
    mut shown: Query<(&mut Node, &mut Visibility, &Children), With<Tooltip>>,
    mut labels: Query<&mut BitmapText>,
    mut font: Local<Option<Handle<SpriteSheet>>>,
) {
    let font = font.get_or_insert_with(|| assets.load("retail://data/fonts/font10arial.sti"));
    let cursor = pointers
        .iter()
        .find(|(id, _)| id.is_mouse())
        .and_then(|(_, location)| location.location())
        .map(|location| (location.position / scale.0).round());
    // Resolve the description from its real dependencies every time. The
    // retained label caches only rasterized text, never a target/cursor key.
    let wanted = (|| {
        let (entity, at) =
            hovered.entity.filter(|_| hovered.held.elapsed_secs() >= TOOLTIP_DELAY).zip(cursor)?;
        let target = cameras.single().ok()?.logical_target_size()? / scale.0;
        let sheet = sheets.get(&*font)?;
        let text = if let Ok(item) = items.get(entity) {
            // "Bullets (Unidentified) (16)".
            let catalog = &content.items;
            item.display_name(&catalog[item.appearance()], true)
        } else if let Ok(monster) = monsters.get(entity) {
            // "Crustacean (?/12)" (retail `SetCombatSelection`).
            let catalog = &content.species;
            let kind = &catalog[monster.species];
            let identified = memberships
                .get(entity)
                .ok()
                .and_then(|m| groups.get(m.0).ok())
                .is_some_and(|g| g.identified);
            let party: Vec<(&Character, &Vitals, bool)> =
                members.iter_many(roster.iter()).matched().collect();
            let health = health_known(&party, monster.hp, monster.max_hp, kind.display_level);
            format!("{} ({health})", kind.shown_name(identified, 1))
        } else {
            return None;
        };
        let width = sheet.text_width(&text).max(1) as f32;
        let height = sheet
            .frames
            .first()
            .map_or(1.0, |f| (i32::from(f.offset[1]) + f.height as i32).max(1) as f32);
        let size = Vec2::new(width + 4.0, height + 2.0);
        let corner = tooltip_corner(at, size, target);
        Some((text, corner, size))
    })();
    let Some((value, at, size)) = wanted else {
        for (_, mut visible, _) in &mut shown {
            visible.set_if_neq(Visibility::Hidden);
        }
        return;
    };
    if let Ok((mut node, mut visible, children)) = shown.single_mut() {
        let desired = absolute(at, size);
        if node.left != desired.left {
            node.left = desired.left;
        }
        if node.top != desired.top {
            node.top = desired.top;
        }
        if node.width != desired.width {
            node.width = desired.width;
        }
        if node.height != desired.height {
            node.height = desired.height;
        }
        visible.set_if_neq(Visibility::Inherited);
        for child in children.iter() {
            if let Ok(mut label) = labels.get_mut(child)
                && label.value != value
            {
                label.value = value.clone();
            }
        }
    } else {
        let font = font.clone();
        let node = absolute(at, size);
        commands.spawn_scene(bsn!{
            Tooltip node Node{border:UiRect::all(px(1))}
            BackgroundColor(Color::BLACK) BorderColor::all(Color::srgb(0.93,0.6,0.33)) GlobalZIndex(30) Pickable::IGNORE
            Children [
                template(move |_|Ok(BitmapText{palette:None,font:font.clone(),value:value.clone(),at:Vec2::new(1.0,0.0),width:0.0,justify:bevy::text::Justify::Left}))
                Node{position_type:PositionType::Absolute}
                Pickable::IGNORE
            ]
        });
    }
}

/// Cursor, tooltip and camera target are all in unscaled, full-target UI coordinates.
fn tooltip_corner(cursor: Vec2, size: Vec2, target: Vec2) -> Vec2 {
    (cursor - Vec2::new(0.0, HALF_CURSOR))
        .max(Vec2::splat(2.0))
        .min((target - size - 2.0).max(Vec2::splat(2.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_bounds_follow_the_target_instead_of_the_retail_canvas() {
        let size = Vec2::new(100.0, 20.0);
        for pixels in
            [Vec2::new(1280.0, 960.0), Vec2::new(1920.0, 1080.0), Vec2::new(1080.0, 1920.0)]
        {
            let scale = (pixels.x / 640.0).min(pixels.y / 480.0);
            let target = pixels / scale;
            let cursor = target - Vec2::splat(10.0);
            let at = tooltip_corner(cursor, size, target);
            assert_eq!(at.x, target.x - size.x - 2.0);
            assert!(at.y >= 2.0 && at.y + size.y <= target.y - 2.0);
            assert_eq!(tooltip_corner(Vec2::ZERO, size, target), Vec2::splat(2.0));
        }
        let wide = Vec2::new(1920.0, 1080.0) / 2.25;
        assert_eq!(tooltip_corner(Vec2::new(700.0, 200.0), size, wide), Vec2::new(700.0, 184.0));
        assert_eq!(tooltip_corner(Vec2::new(700.0, 200.0), size, Vec2::new(640.0, 480.0)).x, 538.0);
    }
}
