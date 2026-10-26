//! The retail magic review page and the resistances of the reviewed actor.
use super::{InventoryArt, InventoryScreen, RetailLabel, absolute, review};
use crate::content::characters::Realm;
use crate::content::items::ItemCatalog;
use crate::game::characters::{Character, resistance::resistances};
use crate::game::data::GameData;
use crate::game::inventory::Carried;
use crate::presentation::screen_art::{BitmapText, Picture};
use bevy::prelude::*;
use enumset::EnumSet;

#[derive(Component)]
pub(super) struct Value(Realm);
#[derive(Component)]
pub(super) struct Segment {
    realm: Realm,
    bonus: bool,
}
#[derive(Component)]
pub(super) struct SegmentImage;

fn picture(
    commands: &mut Commands,
    parent: Entity,
    sheet: Handle<crate::content::ui::SpriteSheet>,
    frame: usize,
    at: Vec2,
) -> Entity {
    commands
        .spawn((
            Picture { sheet, frame, at, bounds: None },
            Node { position_type: PositionType::Absolute, ..default() },
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .id()
}

pub(super) fn build(commands: &mut Commands, root: Entity, art: &InventoryArt) {
    let page = commands
        .spawn((
            review::Page::Spells,
            Node { overflow: Overflow::clip(), ..absolute(Vec2::ZERO, Vec2::new(640.0, 450.0)) },
            Pickable::IGNORE,
            ChildOf(root),
        ))
        .id();
    picture(commands, page, art.magic_page.clone(), 0, Vec2::new(310.0, 0.0));
    picture(commands, page, art.magic_page.clone(), 2, Vec2::new(0.0, 165.0));
    commands.spawn((
        BitmapText {
            font: art.font.clone(),
            at: Vec2::new(324.0, 30.0),
            width: 308.0,
            justify: bevy::text::Justify::Center,
            ..default()
        },
        RetailLabel(0x8ca),
        Node { position_type: PositionType::Absolute, ..default() },
        Pickable::IGNORE,
        ChildOf(page),
    ));
    for (i, realm) in EnumSet::<Realm>::all().iter().enumerate() {
        let left = 324.0 + (i % 2) as f32 * 156.0;
        let top = 53.0 + (i / 2) as f32 * 28.0;
        let frame = [3, 8, 8, 1, 14, 11][i];
        picture(commands, page, art.realms[i].clone(), frame, Vec2::new(left, top));
        for bonus in [false, true] {
            let clip = commands
                .spawn((
                    Segment { realm, bonus },
                    Node {
                        overflow: Overflow::clip_x(),
                        ..absolute(Vec2::new(left + 24.0, top + 4.0), Vec2::new(0.0, 10.0))
                    },
                    Pickable::IGNORE,
                    ChildOf(page),
                ))
                .id();
            let image = picture(commands, clip, art.slider.clone(), usize::from(bonus), Vec2::ZERO);
            commands.entity(image).insert(SegmentImage);
        }
        commands.spawn((
            Value(realm),
            BitmapText {
                font: art.font.clone(),
                at: Vec2::new(left + 18.0, top + 3.0),
                width: 129.0,
                justify: bevy::text::Justify::Right,
                ..default()
            },
            Node { position_type: PositionType::Absolute, ..default() },
            Pickable::IGNORE,
            ChildOf(page),
        ));
        let icon = Vec2::new(5.0 + (i % 3) as f32 * 213.0, 172.0 + (i / 3) as f32 * 140.0);
        picture(commands, page, art.realms[i].clone(), frame, icon);
        // The original keeps the range arrows visible but disabled for an empty realm.
        let x = (i % 3) as f32 * 213.0 + 187.0;
        let y = (i / 3) as f32 * 140.0 + 195.0;
        picture(commands, page, art.scroll.clone(), 3, Vec2::new(x, y));
        picture(commands, page, art.scroll.clone(), 11, Vec2::new(x, y + 86.0));
    }
}

pub(super) fn refresh(
    screen: Option<Single<Ref<InventoryScreen>>>,
    characters: Query<(Ref<Character>, Ref<Carried>)>,
    data: Option<Res<GameData>>,
    catalogs: Option<Res<Assets<ItemCatalog>>>,
    mut values: Query<(Ref<Value>, &mut BitmapText)>,
    mut segments: Query<(Ref<Segment>, &mut Node, &Children)>,
    mut pictures: Query<&mut Picture, With<SegmentImage>>,
) {
    let Some(screen) = screen else { return };
    let Ok((character, carried)) = characters.get(screen.member) else { return };
    let Some(catalog) = data
        .as_ref()
        .and_then(|data| catalogs.as_ref().and_then(|catalogs| catalogs.get(&data.items)))
    else {
        return;
    };
    let changed = screen.is_changed()
        || character.is_changed()
        || carried.is_changed()
        || catalogs.as_ref().is_some_and(|catalogs| catalogs.is_changed());
    if !changed
        && !values.iter().any(|(value, _)| value.is_added())
        && !segments.iter().any(|(segment, _, _)| segment.is_added())
    {
        return;
    }
    let scores = resistances(&character, &carried, catalog);
    for (value, mut text) in &mut values {
        let desired = scores[value.0].total.to_string();
        if text.value != desired {
            text.value = desired;
        }
    }
    for (segment, mut node, children) in &mut segments {
        let value = scores[segment.realm];
        let filled = value.base.min(value.total).clamp(0, 125) as f32;
        let extra = (value.total - value.base).max(0).min(125 - filled as i32) as f32;
        let missing = (value.base - value.total).max(0).min(125 - filled as i32) as f32;
        let left = 324.0
            + (segment.realm as usize % 2) as f32 * 156.0
            + 24.0
            + if segment.bonus { filled } else { 0.0 };
        let width = if segment.bonus { extra.max(missing) } else { filled };
        if node.left != px(left) {
            node.left = px(left);
        }
        if node.width != px(width) {
            node.width = px(width);
        }
        for child in children.iter() {
            if let Ok(mut picture) = pictures.get_mut(child) {
                let x = if segment.bonus { -filled } else { 0.0 };
                let frame =
                    if segment.bonus && missing > 0.0 { 2 } else { usize::from(segment.bonus) };
                if picture.at.x != x {
                    picture.at.x = x;
                }
                if picture.frame != frame {
                    picture.frame = frame;
                }
            }
        }
    }
}
