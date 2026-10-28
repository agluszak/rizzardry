//! The retail magic review page and the resistances of the reviewed actor.
use super::{InventoryArt, InventoryScreen, RetailLabel, absolute, review};
use crate::content::characters::Realm;
use crate::game::characters::resistance::Resistances;
use crate::presentation::screen_art::spawn_picture as picture;
use crate::presentation::screen_art::{BitmapText, Picture, PictureCrop};
use bevy::prelude::*;
use enumset::EnumSet;

#[derive(Component)]
pub(super) struct Value(Realm);
#[derive(Component)]
pub(super) struct Segment {
    realm: Realm,
    bonus: bool,
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
        ChildOf(page),
    ));
    for (i, realm) in EnumSet::<Realm>::all().iter().enumerate() {
        let left = 324.0 + (i % 2) as f32 * 156.0;
        let top = 53.0 + (i / 2) as f32 * 28.0;
        let frame = [3, 8, 8, 1, 14, 11][i];
        picture(commands, page, art.realms[i].clone(), frame, Vec2::new(left, top));
        for bonus in [false, true] {
            commands.spawn((
                Segment { realm, bonus },
                Picture {
                    sheet: art.slider.clone(),
                    frame: usize::from(bonus),
                    at: Vec2::new(left + 24.0, top + 4.0),
                    bounds: None,
                },
                PictureCrop::horizontal(0.0, 0.0),
                ChildOf(page),
            ));
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
            ChildOf(page),
        ));
        let icon = Vec2::new(5.0 + (i % 3) as f32 * 213.0, 172.0 + (i / 3) as f32 * 140.0);
        picture(commands, page, art.realms[i].clone(), frame, icon);
        // The original keeps the range arrows visible but disabled for an empty realm.
        let x = (i % 3) as f32 * 213.0 + 187.0;
        let y = (i / 3) as f32 * 140.0 + 195.0;
        // Retail fills the thumb's track even when its range is disabled.
        commands.spawn((
            absolute(Vec2::new(x, y + 16.0), Vec2::new(16.0, 70.0)),
            BackgroundColor(Color::BLACK),
            Pickable::IGNORE,
            ChildOf(page),
        ));
        picture(commands, page, art.scroll.clone(), 3, Vec2::new(x, y));
        picture(commands, page, art.scroll.clone(), 11, Vec2::new(x, y + 86.0));
    }
}

pub(super) fn refresh(
    screen: Option<Single<&InventoryScreen>>,
    characters: Query<&Resistances>,
    mut values: Query<(&Value, &mut BitmapText)>,
    mut segments: Query<(&Segment, &mut PictureCrop, &mut Picture)>,
) {
    let Some(screen) = screen else { return };
    let Ok(Resistances(scores)) = characters.get(screen.member) else { return };
    for (value, mut text) in &mut values {
        let desired = scores[value.0].total.to_string();
        if text.value != desired {
            text.value = desired;
        }
    }
    for (segment, mut crop, mut picture) in &mut segments {
        let value = scores[segment.realm];
        let filled = value.base.min(value.total).clamp(0, 125) as f32;
        let extra = (value.total - value.base).max(0).min(125 - filled as i32) as f32;
        let missing = (value.base - value.total).max(0).min(125 - filled as i32) as f32;
        let start = if segment.bonus { filled } else { 0.0 };
        let width = if segment.bonus { extra.max(missing) } else { filled };
        crop.set_if_neq(PictureCrop::horizontal(start, start + width));
        let frame = if segment.bonus && missing > 0.0 { 2 } else { usize::from(segment.bonus) };
        if picture.frame != frame {
            picture.frame = frame;
        }
    }
}
