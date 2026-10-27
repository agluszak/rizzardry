//! The layout modes and panels, the roof's mode buttons and the left post's
//! panel arrows, and fitting the 3D view into what the layout leaves open.

use bevy::camera::Viewport;
use bevy::prelude::*;
use bevy::ui::IsDefaultUiCamera;
use bevy::ui_widgets::Activate;
use enumset::{EnumSet, EnumSetType};

use super::{MainScreenArt, button, loaded};
use crate::content::ui::SpriteSheet;
use crate::presentation::party_view::PartyCamera;
use crate::presentation::screen_art::{SCREEN, sprite};
use crate::presentation::sky::SkyCamera;

/// Retail's layout modes (`g_settings.main_ui_mode`; the roof tooltips call
/// them Closed, Wide and Full).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayoutMode {
    /// Full portrait columns either side of a narrow view.
    Closed,
    /// Bar strips at the sides; new games start here.
    #[default]
    Wide,
    /// Strips and no bottom panels.
    Full,
}

/// The bottom panels.
#[derive(Debug, EnumSetType)]
pub enum Panel {
    Radar,
    TextBox,
    Formation,
}

/// The screen's layout: its mode and which bottom panels each mode shows
/// (retail keeps a preference per mode, `ApplyMainGameModeFlag`).
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct ScreenLayout {
    pub mode: LayoutMode,
    /// Closed shows the radar and formation always; the text box by choice.
    closed_text: bool,
    /// The panels Wide shows.
    wide: EnumSet<Panel>,
}

impl Default for ScreenLayout {
    /// New games start Wide with all three panels.
    fn default() -> Self {
        ScreenLayout { mode: LayoutMode::Wide, closed_text: true, wide: EnumSet::all() }
    }
}

impl ScreenLayout {
    pub fn shows(&self, panel: Panel) -> bool {
        match self.mode {
            LayoutMode::Closed => panel != Panel::TextBox || self.closed_text,
            LayoutMode::Wide => self.wide.contains(panel),
            LayoutMode::Full => false,
        }
    }

    /// The 3D view's rectangle in screen pixels (retail `g_viewport_modes`,
    /// chosen by `GetMainGameViewportMode`): the narrow view in Closed,
    /// reaching down over hidden panels; the wide view over the strips.
    pub fn viewport(&self) -> Rect {
        let all = EnumSet::all().iter().all(|panel| self.shows(panel));
        match self.mode {
            LayoutMode::Closed if self.shows(Panel::TextBox) => {
                Rect::new(128.0, 18.0, 512.0, 358.0)
            }
            LayoutMode::Closed => Rect::new(128.0, 18.0, 512.0, 450.0),
            _ if all => Rect::new(23.0, 18.0, 617.0, 358.0),
            _ => Rect::new(23.0, 18.0, 617.0, 450.0),
        }
    }

    /// Backspace: Closed, Wide, Full and round (Shift goes back).
    pub fn cycle(&mut self, back: bool) {
        use LayoutMode::*;
        self.mode = match (self.mode, back) {
            (Closed, false) => Wide,
            (Wide, false) => Full,
            (Full, false) => Closed,
            (Closed, true) => Full,
            (Wide, true) => Closed,
            (Full, true) => Wide,
        };
    }

    /// A layout arrow: Closed toggles only the text box; hiding Wide's last
    /// panel goes Full; showing one in Full goes Wide with only that one.
    pub fn toggle(&mut self, panel: Panel) {
        match self.mode {
            LayoutMode::Closed if panel == Panel::TextBox => self.closed_text = !self.closed_text,
            LayoutMode::Closed => {}
            LayoutMode::Wide => {
                self.wide ^= panel;
                if self.wide.is_empty() {
                    self.mode = LayoutMode::Full;
                }
            }
            LayoutMode::Full => {
                self.wide = EnumSet::only(panel);
                self.mode = LayoutMode::Wide;
            }
        }
    }
}

/// The window is cleared black first, the 3D views draw into the viewport,
/// and the interface draws over the whole window last.
pub fn spawn_screen_cameras(mut commands: Commands) {
    commands.spawn((
        Name::new("Clear camera"),
        Camera2d,
        Camera { order: -10, clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() },
    ));
    commands.spawn((
        Name::new("Interface camera"),
        Camera2d,
        Camera { order: 10, clear_color: ClearColorConfig::None, ..default() },
        IsDefaultUiCamera,
    ));
}

/// Fit the 3D cameras to the layout's viewport, in the pixels of what the
/// interface draws into (the window, or an offscreen image), with the
/// 640×480 screen scaled and centred as the interface is.
pub fn fit_viewport(
    layout: Res<ScreenLayout>,
    interface: Query<&Camera, With<IsDefaultUiCamera>>,
    scale: Res<UiScale>,
    mut cameras: Query<
        &mut Camera,
        (Or<(With<PartyCamera>, With<SkyCamera>)>, Without<IsDefaultUiCamera>),
    >,
) {
    let Some((size, factor)) = interface
        .single()
        .ok()
        .and_then(|c| Some((c.logical_target_size()?, c.target_scaling_factor()?)))
    else {
        return;
    };
    let origin = (size - SCREEN * scale.0) / 2.0;
    let rect = layout.viewport();
    let physical = |v: Vec2| ((origin + v * scale.0) * factor).round().as_uvec2();
    let viewport = Viewport {
        physical_position: physical(rect.min),
        physical_size: (physical(rect.max) - physical(rect.min)).max(UVec2::ONE),
        ..default()
    };
    for mut camera in &mut cameras {
        if camera.viewport.as_ref().is_none_or(|v| {
            v.physical_position != viewport.physical_position
                || v.physical_size != viewport.physical_size
        }) {
            camera.viewport = Some(viewport.clone());
        }
    }
}

/// The roof's mode buttons, the current mode pressed, and the left post's
/// arrows: hide a panel while shown, show it while hidden; Closed keeps the
/// radar and formation (grayed).
pub fn buttons(
    layout: &ScreenLayout,
    art: &MainScreenArt,
    sheets: &Assets<SpriteSheet>,
) -> impl SceneList {
    let modes: Vec<_> =
        [(LayoutMode::Closed, 10.0), (LayoutMode::Wide, 39.0), (LayoutMode::Full, 68.0)]
            .into_iter()
            .enumerate()
            .map(|(i, (mode, x))| {
                let image = if mode == layout.mode { 3 + i } else { i };
                bsn! {
                    @button(art.roof_buttons.clone(), image, Vec2::new(x, 1.0))
                    on(move |_: On<Activate>, mut layout: ResMut<ScreenLayout>| {
                        layout.mode = mode;
                    })
                }
            })
            .collect();
    let closed = layout.mode == LayoutMode::Closed;
    let arrows = loaded(sheets, &art.layout_arrows);
    // In order: neighbouring arrows overlap.
    let mut panels: Vec<Box<dyn Scene>> = vec![];
    for (panel, y, hide, show, gray) in [
        (Panel::Radar, 371.0, 0, 3, 6),
        (Panel::TextBox, 394.0, 8, 11, 14),
        (Panel::Formation, 417.0, 16, 19, 22),
    ] {
        let at = Vec2::new(0.0, y);
        if closed && panel != Panel::TextBox {
            panels.push(Box::new(sprite(arrows, gray, at)));
            continue;
        }
        let image = if layout.shows(panel) { hide } else { show };
        panels.push(Box::new(bsn! {
            @button(art.layout_arrows.clone(), image, at)
            on(move |_: On<Activate>, mut layout: ResMut<ScreenLayout>| layout.toggle(panel))
        }));
    }
    (modes, panels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_viewport_follows_the_mode_and_panels() {
        let mut layout = ScreenLayout::default();
        assert_eq!(layout.viewport(), Rect::new(23.0, 18.0, 617.0, 358.0));
        layout.toggle(Panel::Radar);
        assert_eq!(layout.viewport(), Rect::new(23.0, 18.0, 617.0, 450.0));
        layout.mode = LayoutMode::Closed;
        assert_eq!(layout.viewport(), Rect::new(128.0, 18.0, 512.0, 358.0));
        layout.toggle(Panel::Radar);
        assert!(layout.shows(Panel::Radar), "Closed keeps the radar");
        layout.toggle(Panel::TextBox);
        assert_eq!(layout.viewport(), Rect::new(128.0, 18.0, 512.0, 450.0));
    }

    #[test]
    fn hiding_the_last_panel_goes_full_and_showing_one_goes_wide() {
        let mut layout = ScreenLayout::default();
        layout.toggle(Panel::Radar);
        layout.toggle(Panel::TextBox);
        layout.toggle(Panel::Formation);
        assert_eq!(layout.mode, LayoutMode::Full);
        layout.toggle(Panel::TextBox);
        assert_eq!(layout.mode, LayoutMode::Wide);
        assert!(layout.shows(Panel::TextBox) && !layout.shows(Panel::Radar));
        layout.cycle(false);
        assert_eq!(layout.mode, LayoutMode::Full);
        layout.cycle(false);
        assert_eq!(layout.mode, LayoutMode::Closed);
    }
}
