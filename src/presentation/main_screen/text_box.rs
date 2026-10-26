//! Retained channel tabs, text lines and scroll controls for the notice log.

use super::{MainScreenArt, layer};
use crate::presentation::notices::{TextLog, VISIBLE_LINES};
use crate::presentation::screen_art::{BitmapText, Picture, absolute, label, picture};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;

#[derive(Component, Clone, Default)]
pub struct TextBox;
#[derive(Component)]
pub(super) struct TextBoxBuilt;
#[derive(Component, Clone)]
pub(super) struct Line(usize);
#[derive(Component, Clone, Default)]
pub(super) struct Tabs;
#[derive(Component, Clone, Default)]
pub(super) struct Thumb;
#[derive(Component, Clone)]
#[require(Hovered)]
pub(super) struct Scroll {
    down: bool,
}

pub fn panel() -> impl SceneList {
    bsn_list! { @layer() TextBox }
}

pub fn draw_text_box(
    mut commands: Commands,
    art: Res<MainScreenArt>,
    log: Res<TextLog>,
    roots: Query<Entity, (With<TextBox>, Without<TextBoxBuilt>)>,
    mut lines: Query<(&Line, &mut BitmapText, &mut Visibility), Without<Thumb>>,
    mut pictures: Query<
        (
            &mut Picture,
            &mut Visibility,
            Option<&Tabs>,
            Option<&Thumb>,
            Option<(&Scroll, &Hovered, &mut Pickable)>,
        ),
        (Without<Line>, Or<(With<Tabs>, With<Thumb>, With<Scroll>)>),
    >,
) {
    for root in &roots {
        let text: Vec<_> = (0..VISIBLE_LINES)
            .map(|i| {
                bsn! {
                    @label(art.font.clone(), Vec2::new(166.0, 362.0 + 11.0 * i as f32), 290.0)
                    template(move |_| Ok(Line(i)))
                }
            })
            .collect();
        let arrows: Vec<_> = [false, true]
            .into_iter()
            .map(|down| {
                let at = Vec2::new(461.0, if down { 429.0 } else { 363.0 });
                bsn! {
                    @picture(art.text_scroll.clone(), if down { 8 } else { 0 }, at)
                    template(move |_| Ok(Scroll { down })) Pickable::default()
                    on(move |_: On<PointerClick>, mut log: ResMut<TextLog>| {
                        if down && log.can_scroll().1 { log.scroll_down(1); }
                        else if !down && log.can_scroll().0 { log.scroll_up(1); }
                    })
                }
            })
            .collect();
        let channels: Vec<_> = [(0, 362.0), (1, 383.0), (2, 404.0)]
            .into_iter()
            .map(|(channel, y)| {
                bsn! {
                    template(move |_| Ok(absolute(Vec2::new(140.0, y), Vec2::new(24.0, 16.0))))
                    on(move |_: On<PointerClick>, mut log: ResMut<TextLog>| log.select(channel))
                }
            })
            .collect();
        commands.entity(root).insert(TextBoxBuilt).queue_spawn_related_scenes::<Children>(
            bsn_list! {
                @picture(art.textbox_tabs.clone(), 0, Vec2::new(138.0, 360.0)) Tabs
                -- {text} -- {arrows} -- {channels}
                -- @picture(art.text_scroll.clone(), 4, Vec2::new(461.0, 379.0)) Thumb
            },
        );
    }
    for (line, mut text, mut visibility) in &mut lines {
        let value = log.visible().get(line.0);
        visibility.set_if_neq(if value.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if let Some(value) = value
            && text.value != *value
        {
            text.value.clone_from(value);
        }
    }
    for (mut picture, mut visibility, tabs, thumb, scroll) in &mut pictures {
        if tabs.is_some() {
            picture.reborrow().map_unchanged(|p| &mut p.frame).set_if_neq(log.selected());
        }
        if thumb.is_some() {
            visibility.set_if_neq(if log.thumb().is_some() {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            });
            if let Some(thumb) = log.thumb() {
                picture
                    .reborrow()
                    .map_unchanged(|p| &mut p.at)
                    .set_if_neq(Vec2::new(461.0, 379.0 + (thumb * 40.0).floor()));
            }
        }
        if let Some((scroll, hovered, mut pickable)) = scroll {
            let enabled = if scroll.down { log.can_scroll().1 } else { log.can_scroll().0 };
            let normal = if scroll.down { 8 } else { 0 };
            let frame = normal
                + if !enabled {
                    3
                } else if hovered.get() {
                    1
                } else {
                    0
                };
            let next = Picture { frame, bounds: Some(normal), ..picture.clone() };
            picture.set_if_neq(next);
            pickable.set_if_neq(if enabled { Pickable::default() } else { Pickable::IGNORE });
        }
    }
}
