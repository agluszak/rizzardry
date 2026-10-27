//! Retained channel tabs, text lines and scroll controls for the notice log.

use super::{MainScreenArt, layer};
use crate::presentation::notices::{TextLog, VISIBLE_LINES};
use crate::presentation::screen_art::{BitmapText, Picture, absolute, label, picture};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::{Activate, SelectedTab, Tab, TabList, ValueChange};

#[derive(Component, Clone, Default)]
pub(super) struct Channels;
#[derive(Component)]
pub(super) struct Channel(usize);

fn select_channel(
    change: On<ValueChange<Option<Entity>>>,
    channels: Query<&Channel>,
    mut log: ResMut<TextLog>,
) {
    if let Some(channel) = change.value.and_then(|e| channels.get(e).ok()) {
        log.select(channel.0);
    }
}

/// TextLog owns selection across HUD rebuilds and combat-driven changes.
/// Native tabs are a controlled projection, not another writable selection.
pub(super) fn sync_channels(
    log: Res<TextLog>,
    mut lists: Query<(&Children, &mut SelectedTab), With<Channels>>,
    channels: Query<&Channel>,
) {
    for (children, mut selection) in &mut lists {
        let selected =
            children.iter().find(|e| channels.get(*e).is_ok_and(|c| c.0 == log.selected()));
        selection.set_if_neq(SelectedTab(selected));
    }
}

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
            Entity,
            &mut Picture,
            &mut Visibility,
            Option<&Tabs>,
            Option<&Thumb>,
            Option<(&Scroll, Has<InteractionDisabled>)>,
        ),
        (Without<Line>, Or<(With<Tabs>, With<Thumb>, With<Scroll>)>),
    >,
    added: Query<(), Or<(Added<Line>, Added<Tabs>, Added<Thumb>, Added<Scroll>)>>,
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
                    @crate::presentation::retail_controls::image_button(art.text_scroll.clone(), at, { let base = if down { 8 } else { 0 }; crate::presentation::retail_controls::ControlSkin::new(base, base+1, base+2, base+2, base+3) })
                    template(move |_| Ok(Scroll { down }))
                    on(move |_: On<Activate>, mut log: ResMut<TextLog>| {
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
                    Tab crate::presentation::retail_controls::FocusRing
                    template(move |_| Ok(Channel(channel)))
                }
            })
            .collect();
        commands.entity(root).insert(TextBoxBuilt).queue_spawn_related_scenes::<Children>(
            bsn_list! {
                @picture(art.textbox_tabs.clone(), 0, Vec2::new(138.0, 360.0)) Tabs
                -- {text} -- {arrows}
                -- @layer() TabList { orientation: bevy::ui_widgets::ControlOrientation::Vertical, activation: bevy::ui_widgets::TabActivation::Manual } Channels on(select_channel) Children [ {channels} ]
                -- @picture(art.text_scroll.clone(), 4, Vec2::new(461.0, 379.0)) Thumb
            },
        );
    }
    // The parts built above arrive later; they are projected once they exist.
    if !log.is_changed() && added.is_empty() {
        return;
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
    for (entity, mut picture, mut visibility, tabs, thumb, scroll) in &mut pictures {
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
        if let Some((scroll, disabled)) = scroll {
            let enabled = if scroll.down { log.can_scroll().1 } else { log.can_scroll().0 };
            crate::presentation::retail_controls::set_enabled(
                &mut commands,
                entity,
                enabled,
                disabled,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_tabs_follow_log_selection_and_survive_view_rebuilds() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<TextLog>()
            .add_systems(Update, sync_channels);
        for _ in 0..2 {
            let root =
                app.world_mut().spawn((TabList::default(), Channels)).observe(select_channel).id();
            let tabs: Vec<_> = (0..3)
                .map(|i| app.world_mut().spawn((Tab, Channel(i), ChildOf(root))).id())
                .collect();
            app.world_mut().resource_mut::<TextLog>().select(1);
            app.update();
            assert_eq!(app.world().get::<SelectedTab>(root).unwrap().0, Some(tabs[1]));
            app.world_mut().trigger(ValueChange {
                source: root,
                value: Some(tabs[2]),
                is_final: true,
            });
            app.update();
            assert_eq!(app.world().resource::<TextLog>().selected(), 2);
            assert_eq!(app.world().get::<SelectedTab>(root).unwrap().0, Some(tabs[2]));
            app.world_mut().despawn(root);
            assert_eq!(app.world().resource::<TextLog>().selected(), 2);
        }
    }
}
