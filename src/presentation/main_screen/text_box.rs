//! Retained channel tabs, text lines and scroll controls for the notice log.

use super::{MainScreenArt, layer};
use crate::presentation::notices::{TextLog, VISIBLE_LINES};
use crate::presentation::screen_art::visible_if;
use crate::presentation::screen_art::{BitmapText, Picture, absolute, label, picture};
use bevy::input::mouse::MouseScrollUnit;
use bevy::picking::events::{
    PointerCancel, PointerClick, PointerDrag, PointerDragEnd, PointerLeave, PointerPress,
    PointerRelease, PointerScroll,
};
use bevy::picking::hover::Hovered;
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;
use bevy::ui::{ComputedUiRenderTargetInfo, InteractionDisabled, UiGlobalTransform};
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
#[derive(Component, Clone, Default)]
pub(super) struct ThumbTrack;
#[derive(Component, Clone)]
#[require(Hovered)]
pub(super) struct Scroll {
    down: bool,
}
impl Scroll {
    fn apply(&self, log: &mut TextLog, lines: usize) {
        if self.down && log.can_scroll().1 {
            log.scroll_down(lines);
        } else if !self.down && log.can_scroll().0 {
            log.scroll_up(lines);
        }
    }
}
#[derive(Component)]
pub(super) struct ScrollHold {
    button: PointerButton,
    timer: Timer,
}

fn scroll_body(mut event: On<PointerScroll>, mut log: ResMut<TextLog>) {
    let lines = match event.unit {
        MouseScrollUnit::Line => event.y.abs().ceil() as usize,
        MouseScrollUnit::Pixel => usize::from(event.y != 0.0),
    };
    if event.y < 0.0 {
        log.scroll_down(lines);
    } else if event.y > 0.0 {
        log.scroll_up(lines);
    }
    if lines != 0 {
        event.propagate(false);
    }
}

fn scroll_page(event: On<PointerClick>, arrows: Query<&Scroll>, mut log: ResMut<TextLog>) {
    if event.button != PointerButton::Secondary {
        return;
    }
    if let Ok(arrow) = arrows.get(event.entity) {
        arrow.apply(&mut log, VISIBLE_LINES);
    }
}

fn scroll_line(
    event: On<Activate>,
    arrows: Query<&Scroll, Without<InteractionDisabled>>,
    mut log: ResMut<TextLog>,
) {
    if let Ok(arrow) = arrows.get(event.entity) {
        arrow.apply(&mut log, 1);
    }
}

fn hold_arrow(
    event: On<PointerPress>,
    arrows: Query<(), (With<Scroll>, Without<InteractionDisabled>)>,
    mut commands: Commands,
) {
    if matches!(event.button, PointerButton::Primary | PointerButton::Secondary)
        && arrows.contains(event.entity)
    {
        commands.entity(event.entity).insert(ScrollHold {
            button: event.button,
            timer: Timer::new(std::time::Duration::from_millis(250), TimerMode::Once),
        });
    }
}

fn release_arrow(event: On<PointerRelease>, held: Query<&ScrollHold>, mut commands: Commands) {
    if held.get(event.entity).is_ok_and(|hold| hold.button == event.button) {
        commands.entity(event.entity).remove::<ScrollHold>();
    }
}

fn leave_arrow(event: On<PointerLeave>, mut commands: Commands) {
    if event.was_in_bounds {
        commands.entity(event.entity).remove::<ScrollHold>();
    }
}

fn cancel_arrow(event: On<PointerCancel>, mut commands: Commands) {
    commands.entity(event.entity).remove::<ScrollHold>();
}

fn end_arrow_drag(event: On<PointerDragEnd>, mut commands: Commands) {
    commands.entity(event.entity).remove::<ScrollHold>();
}

pub(super) fn repeat_arrows(
    time: Res<Time<Real>>,
    mut held: Query<(&Scroll, &mut ScrollHold), Without<InteractionDisabled>>,
    mut log: ResMut<TextLog>,
) {
    for (arrow, mut hold) in &mut held {
        if hold.timer.tick(time.delta()).just_finished() {
            let lines = if hold.button == PointerButton::Secondary { VISIBLE_LINES } else { 1 };
            arrow.apply(&mut log, lines);
            hold.timer = Timer::new(std::time::Duration::from_millis(50), TimerMode::Once);
        }
    }
}

fn move_thumb(
    entity: Entity,
    pointer: Vec2,
    tracks: &Query<
        (&ComputedNode, &ComputedUiRenderTargetInfo, &UiGlobalTransform),
        With<ThumbTrack>,
    >,
    scale: f32,
    log: &mut TextLog,
) {
    let Ok((node, target, transform)) = tracks.get(entity) else { return };
    let Some(point) = node.normalize_point(*transform, pointer * target.scale_factor() / scale)
    else {
        return;
    };
    if point.x.abs() > 0.5 || point.y.abs() > 0.5 || log.thumb().is_none() {
        return;
    }
    let y = ((point.y + 0.5) * 50.0).floor() as i32 - 1;
    let position = if y < 0 { 0 } else { (y + 2).min(46) };
    log.scroll_thumb(position as u32);
}

fn press_thumb(
    event: On<PointerPress>,
    tracks: Query<
        (&ComputedNode, &ComputedUiRenderTargetInfo, &UiGlobalTransform),
        With<ThumbTrack>,
    >,
    scale: Res<UiScale>,
    mut log: ResMut<TextLog>,
) {
    if event.button == PointerButton::Primary {
        move_thumb(event.entity, event.pointer.position, &tracks, scale.0, &mut log);
    }
}

fn drag_thumb(
    event: On<PointerDrag>,
    tracks: Query<
        (&ComputedNode, &ComputedUiRenderTargetInfo, &UiGlobalTransform),
        With<ThumbTrack>,
    >,
    scale: Res<UiScale>,
    mut log: ResMut<TextLog>,
) {
    if event.button == PointerButton::Primary {
        move_thumb(event.entity, event.pointer.position, &tracks, scale.0, &mut log);
    }
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
                    on(hold_arrow) on(release_arrow) on(leave_arrow) on(cancel_arrow) on(end_arrow_drag)
                    on(scroll_page) on(scroll_line)
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
                -- template(|_| Ok(absolute(Vec2::new(166.0, 362.0), Vec2::new(290.0, 83.0)))) Pickable::default() on(scroll_body)
                -- {text} -- {arrows}
                -- @layer() TabList { orientation: bevy::ui_widgets::ControlOrientation::Vertical, activation: bevy::ui_widgets::TabActivation::Manual } Channels on(select_channel) Children [ {channels} ]
                -- ThumbTrack template(|_| Ok(absolute(Vec2::new(461.0, 379.0), Vec2::new(8.0, 50.0)))) Pickable::default() on(press_thumb) on(drag_thumb)
                -- @picture(art.text_scroll.clone(), 4, Vec2::new(461.0, 379.0)) Thumb
            },
        );
    }
    // The parts built above arrive later; they are projected once they exist.
    if !log.is_changed() && added.is_empty() {
        return;
    }
    for (line, mut text, mut visibility) in &mut lines {
        let justify = if log.selected() == 2 {
            bevy::text::Justify::Left
        } else {
            bevy::text::Justify::Center
        };
        if text.justify != justify {
            text.justify = justify;
        }
        let value = log.visible().get(line.0);
        visibility.set_if_neq(visible_if(value.is_some()));
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
            visibility.set_if_neq(visible_if(log.thumb().is_some()));
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
    use crate::content::{species::MonsterCatalog, ui::SpriteSheet, ui::TextTable};
    use crate::game::{
        camp::CampNews,
        combat::CombatNews,
        inventory::ItemNews,
        world::{identify::Identified, search::SearchNews, triggers::TriggerEffect},
    };
    use crate::presentation::notices::{load_notice_art, queue_notices};
    use bevy::input::{mouse::MouseScrollUnit, touch::TouchPhase};
    use bevy::picking::backend::HitData;

    #[test]
    fn notice_scrolling_follows_pointer_and_held_arrows() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            bevy::scene::ScenePlugin,
            bevy::ui_widgets::ButtonPlugin,
        ))
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(50),
        ))
        .add_observer(crate::presentation::retail_controls::primary_buttons_only)
        .init_asset::<SpriteSheet>()
        .init_asset::<TextTable>()
        .init_asset::<MonsterCatalog>()
        .init_asset::<crate::content::spells::SpellCatalog>()
        .init_resource::<TextLog>()
        .add_message::<TriggerEffect>()
        .add_message::<CampNews>()
        .add_message::<CombatNews>()
        .add_message::<ItemNews>()
        .add_message::<SearchNews>()
        .add_message::<Identified>()
        .add_message::<crate::game::magic::casting::MagicNews>()
        .add_message::<crate::game::world::interaction::LockNews>()
        .add_message::<crate::presentation::party_voice::SpokenRemark>()
        .add_message::<crate::game::factions::FactionNews>()
        .add_systems(Startup, (super::super::load_main_screen_art, load_notice_art))
        .add_systems(Update, (queue_notices, draw_text_box, repeat_arrows).chain());
        app.world_mut().register_component::<Window>();
        app.world_mut().spawn(TextBox);
        app.update();
        for gold in 0..12 {
            app.world_mut().write_message(TriggerEffect::Gold(gold));
        }
        app.update();
        let body =
            app.world_mut().query_filtered::<Entity, With<TextBox>>().single(app.world()).unwrap();
        let body = app
            .world()
            .get::<Children>(body)
            .unwrap()
            .iter()
            .find(|entity| {
                app.world().get::<Node>(*entity).is_some_and(|node| {
                    node.left == px(166) && node.top == px(362) && node.height == px(83)
                })
            })
            .unwrap();
        let hit = || HitData::new(Entity::PLACEHOLDER, 0.0, None, None);
        app.world_mut().trigger(PointerScroll {
            entity: body,
            pointer: crate::presentation::test_input::mouse(),
            unit: MouseScrollUnit::Line,
            x: 0.0,
            y: 2.0,
            hit: hit(),
            phase: TouchPhase::Moved,
        });
        assert!(app.world().resource::<TextLog>().visible()[0].contains('3'));
        let down = app
            .world_mut()
            .query::<(Entity, &Scroll)>()
            .iter(app.world())
            .find_map(|(entity, arrow)| arrow.down.then_some(entity))
            .unwrap();
        app.world_mut().trigger(PointerClick {
            entity: down,
            pointer: crate::presentation::test_input::mouse(),
            button: PointerButton::Secondary,
            hit: hit(),
            duration: default(),
            count: 1,
        });
        assert!(app.world().resource::<TextLog>().visible()[0].contains("10"));
        app.update();
        let up = app
            .world_mut()
            .query::<(Entity, &Scroll)>()
            .iter(app.world())
            .find_map(|(entity, arrow)| (!arrow.down).then_some(entity))
            .unwrap();
        app.world_mut().trigger(PointerPress {
            entity: up,
            pointer: crate::presentation::test_input::mouse(),
            button: PointerButton::Primary,
            hit: hit(),
            count: 1,
        });
        for _ in 0..4 {
            app.update();
        }
        assert!(app.world().resource::<TextLog>().visible()[0].contains("10"));
        app.update();
        assert!(app.world().resource::<TextLog>().visible()[0].contains('9'));
        app.update();
        assert!(app.world().resource::<TextLog>().visible()[0].contains('8'));
        app.world_mut().trigger(PointerRelease {
            entity: up,
            pointer: crate::presentation::test_input::mouse(),
            button: PointerButton::Primary,
            hit: hit(),
        });
        for _ in 0..5 {
            app.update();
        }
        assert!(app.world().resource::<TextLog>().visible()[0].contains('8'));
        app.world_mut().resource_mut::<TextLog>().select(2);
        app.update();
        assert!(
            app.world_mut()
                .query_filtered::<&BitmapText, With<Line>>()
                .iter(app.world())
                .all(|text| text.justify == bevy::text::Justify::Left)
        );
    }

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
