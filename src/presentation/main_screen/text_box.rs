//! The text box: its channel tabs, the lines it shows, its scroll arrows and
//! thumb (retail `RedrawTextBoxBody`, `RedrawTextBoxScrollChrome`,
//! `TextBoxChannelTabRegionEvent`), and its keys.

use bevy::prelude::*;

use super::{MainScreenArt, hover_art, layer, loaded, put};
use crate::content::ui::SpriteSheet;
use crate::presentation::notices::{TextLog, VISIBLE_LINES};
use crate::presentation::screen_art::{absolute, sprite, text};

/// The text box's node, with what it last drew.
#[derive(Component, Default)]
pub struct TextBox {
    drawn: Option<TextView>,
}

#[derive(Clone, Debug, PartialEq)]
struct TextView {
    channel: usize,
    lines: Vec<String>,
    /// Whether it scrolls up, and down.
    scrolls: (bool, bool),
    /// Where the thumb sits, when the text overflows.
    thumb: Option<f32>,
}

pub fn spawn(commands: &mut Commands, frame: Entity) {
    layer(commands, frame, TextBox::default());
}

pub fn draw_text_box(
    mut commands: Commands,
    art: Res<MainScreenArt>,
    sheets: Res<Assets<SpriteSheet>>,
    mut images: ResMut<Assets<Image>>,
    log: Res<TextLog>,
    mut boxes: Query<(Entity, &mut TextBox)>,
) {
    let Ok((node, mut text_box)) = boxes.single_mut() else { return };
    let view = TextView {
        channel: log.selected(),
        lines: log.visible().to_vec(),
        scrolls: log.can_scroll(),
        thumb: log.thumb(),
    };
    if text_box.drawn.as_ref() == Some(&view) {
        return;
    }
    commands.entity(node).despawn_related::<Children>();
    let tabs = loaded(&sheets, &art.textbox_tabs);
    put(&mut commands, node, sprite(tabs, view.channel, Vec2::new(138.0, 360.0)));
    // Its lines centred in the body, 11 pixels apart.
    let font = loaded(&sheets, &art.font);
    for (i, line) in view.lines.iter().enumerate() {
        let at = Vec2::new(166.0, 362.0 + 11.0 * i as f32);
        commands.spawn((text(font, &mut images, line, at, 290.0), ChildOf(node)));
    }
    // The arrows are grayed where it cannot scroll.
    let scroll = loaded(&sheets, &art.text_scroll);
    let (up, down) = view.scrolls;
    for (enabled, normal, grayed, y, lines) in [(up, 0, 3, 363.0, -1), (down, 8, 11, 429.0, 1)] {
        let image = if enabled { normal } else { grayed };
        if let Some(mut arrow) =
            put(&mut commands, node, sprite(scroll, image, Vec2::new(461.0, y)))
            && enabled
        {
            arrow
                .insert((
                    TextScroll(lines),
                    hover_art(scroll, image, image + 1),
                    Pickable::default(),
                ))
                .observe(scroll_text);
        }
    }
    if let Some(thumb) = view.thumb {
        let at = Vec2::new(461.0, 379.0 + (thumb * 40.0).floor());
        put(&mut commands, node, sprite(scroll, 4, at));
    }
    for (channel, y) in [(0, 362.0), (1, 383.0), (2, 404.0)] {
        commands
            .spawn((
                absolute(Vec2::new(140.0, y), Vec2::new(24.0, 16.0)),
                Channel(channel),
                ChildOf(node),
            ))
            .observe(select_channel);
    }
    text_box.drawn = Some(view);
}

/// A text box scroll arrow: lines to move by.
#[derive(Component, Clone, Copy)]
struct TextScroll(i32);

fn scroll_text(click: On<PointerClick>, arrows: Query<&TextScroll>, mut log: ResMut<TextLog>) {
    match arrows.get(click.entity) {
        Ok(TextScroll(lines)) if *lines < 0 => log.scroll_up(lines.unsigned_abs() as usize),
        Ok(TextScroll(lines)) => log.scroll_down(*lines as usize),
        Err(_) => {}
    }
}

/// A text box channel tab.
#[derive(Component, Clone, Copy)]
struct Channel(usize);

fn select_channel(click: On<PointerClick>, tabs: Query<&Channel>, mut log: ResMut<TextLog>) {
    if let Ok(tab) = tabs.get(click.entity) {
        log.select(tab.0);
    }
}

/// PgUp, PgDn, Home, End and Del (`mgskeyboard.ini` 500-504): scroll the
/// text box a page, to its top or its end, or empty it.
pub fn text_box_keys(keys: Res<ButtonInput<KeyCode>>, mut log: ResMut<TextLog>) {
    if keys.just_pressed(KeyCode::PageUp) {
        log.scroll_up(VISIBLE_LINES);
    }
    if keys.just_pressed(KeyCode::PageDown) {
        log.scroll_down(VISIBLE_LINES);
    }
    if keys.just_pressed(KeyCode::Home) {
        log.scroll_to(0);
    }
    if keys.just_pressed(KeyCode::End) {
        log.scroll_to_end();
    }
    if keys.just_pressed(KeyCode::Delete) {
        log.clear();
    }
}
