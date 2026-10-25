//! The text box: its channel tabs, the lines it shows, its scroll arrows and
//! thumb (retail `RedrawTextBoxBody`, `RedrawTextBoxScrollChrome`,
//! `TextBoxChannelTabRegionEvent`), and its keys.

use bevy::prelude::*;

use super::{MainScreenArt, button, layer, loaded};
use crate::content::ui::SpriteSheet;
use crate::presentation::notices::TextLog;
use crate::presentation::screen_art::{absolute, hover_art, sprite, text};

/// The text box's node, with what it last drew.
#[derive(Component, Clone, Default)]
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

pub fn panel() -> impl SceneList {
    bsn_list! { @layer() TextBox }
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
    let tabs = sprite(loaded(&sheets, &art.textbox_tabs), view.channel, Vec2::new(138.0, 360.0));
    // Its lines centred in the body, 11 pixels apart.
    let font = loaded(&sheets, &art.font);
    let lines: Vec<_> = view
        .lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            text(font, &mut images, line, Vec2::new(166.0, 362.0 + 11.0 * i as f32), 290.0)
        })
        .collect();
    // The arrows are grayed where it cannot scroll; the thumb shows when
    // the text overflows.
    let scroll = loaded(&sheets, &art.text_scroll);
    let (up, down) = view.scrolls;
    let (mut arrows, mut grayed) = (vec![], vec![]);
    for (enabled, normal, gray, y, lines) in [(up, 0, 3, 363.0, -1_i32), (down, 8, 11, 429.0, 1)] {
        let at = Vec2::new(461.0, y);
        if !enabled {
            grayed.push(sprite(scroll, gray, at));
            continue;
        }
        arrows.push(bsn! {
            @button(scroll, normal, at)
            @hover_art(scroll, normal, normal + 1)
            on(move |_: On<PointerClick>, mut log: ResMut<TextLog>| {
                if lines < 0 {
                    log.scroll_up(lines.unsigned_abs() as usize);
                } else {
                    log.scroll_down(lines as usize);
                }
            })
        });
    }
    let thumb = view
        .thumb
        .and_then(|thumb| sprite(scroll, 4, Vec2::new(461.0, 379.0 + (thumb * 40.0).floor())));
    let channels: Vec<_> = [(0, 362.0), (1, 383.0), (2, 404.0)]
        .into_iter()
        .map(|(channel, y)| {
            let node = absolute(Vec2::new(140.0, y), Vec2::new(24.0, 16.0));
            bsn! {
                node
                on(move |_: On<PointerClick>, mut log: ResMut<TextLog>| log.select(channel))
            }
        })
        .collect();
    commands.entity(node).despawn_related::<Children>().queue_spawn_related_scenes::<Children>(
        bsn_list! {
            @{tabs} -- {lines} -- {arrows} -- {grayed} -- @{thumb} -- {channels}
        },
    );
    text_box.drawn = Some(view);
}
