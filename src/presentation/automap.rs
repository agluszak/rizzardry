//! The automap screen, in native Bevy UI from retail art.
//!
//! Layout and art after Whizzardry `AutomapScreen.cpp`: a 640×480 frame
//! (`map_border.sti`) scaled to the window, the map in the rectangle
//! (12, 32)–(468, 468), and four-state buttons (off, highlighted, on,
//! greyed). Authored ceiling cuts select floors; retail draws only visited
//! geometry, while the current native map shows all walkable surfaces. Notes are pinned with the Add note
//! tool, typed in the retail text font, and erased with Erase note.

mod note_input;
pub(crate) use note_input::{draw_note_input, finish_note_input, sync_note_input};

#[cfg(test)]
use bevy::input::ButtonState;
use bevy::input::keyboard::KeyboardInput;
use bevy::input_focus::tab_navigation::{TabGroup, TabIndex};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::Activate;

use crate::game::screen::Screen;
use crate::presentation::retail_controls::{ControlSkin, set_enabled};
use crate::presentation::screen_art::BitmapText;
use bevy::ui::Selected;
use bevy_enhanced_input::prelude::*;

use crate::content::level::CollisionMesh;
use crate::content::ui::{SpriteSheet, ui_image};
use crate::game::automap::{MapNote, MapNotes, NoteDraft};
use crate::game::party::body::{FLOAT_HEIGHT, PartyBody};
use crate::game::world::level::{Level, PlayedLevel};
use crate::presentation::screen_art::{SCREEN, absolute, centred, screen};

const VIEWPORT_AT: Vec2 = Vec2::new(12.0, 32.0);
const VIEWPORT: Vec2 = Vec2::new(456.0, 436.0);
// AutomapScreen::ZoomIn in both references uses a 25,000-unit minimum.
// Our orthographic map uses that as a 50-metre span, without a camera raycast.
const MIN_SPAN: f32 = 50.0;
/// Markers are drawn this many screen pixels wide.
const MARKER: f32 = 16.0;
const EDIT_BOX: Color = Color::srgb(239.0 / 255.0, 155.0 / 255.0, 86.0 / 255.0);
/// Note markers glow from black to orange and back in this many seconds.
const GLOW_PERIOD: f32 = 2.0;

/// Tab opens the map while the party is in the world.
#[derive(Component, Clone, Default)]
pub struct MapKey;

#[derive(InputAction)]
#[action_output(bool)]
struct OpenMap;

#[derive(InputAction)]
#[action_output(bool)]
struct CloseMap;

/// The arrow keys move the map.
#[derive(InputAction)]
#[action_output(Vec2)]
struct PanMap;

pub fn spawn_map_key(mut commands: Commands) {
    commands.spawn_scene(bsn! {
        MapKey
        Actions<MapKey> [
            // Tab, as retail's mgskeyboard.ini binds it.
            Action::<OpenMap>::new()
            Bindings [ Binding(KeyCode::Tab) ]
        ]
        on(|_: On<Start<OpenMap>>, mut open: MessageWriter<OpenAutomap>,
            focus: Res<bevy::input_focus::InputFocus>,
            widgets: Query<(),With<bevy::input_focus::tab_navigation::TabIndex>>| {
            if !focus.get().is_some_and(|entity| widgets.contains(entity)) { open.write(OpenAutomap); }
        })
    });
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MapTool {
    AddNote,
    EraseNote,
}

#[derive(Resource)]
pub struct AutomapArt {
    border: Handle<SpriteSheet>,
    note_buttons: Handle<SpriteSheet>,
    exit_button: Handle<SpriteSheet>,
    zoom_buttons: Handle<SpriteSheet>,
    movement_buttons: Handle<SpriteSheet>,
    layer_buttons: Handle<SpriteSheet>,
    cursors: Handle<SpriteSheet>,
    font: Handle<SpriteSheet>,
    note_marker: Handle<Image>,
    party_marker: Handle<Image>,
}

pub fn load_automap_art(mut commands: Commands, assets: Res<AssetServer>) {
    let art = |name: &str| format!("retail://data/automap/{name}");
    commands.insert_resource(AutomapArt {
        border: assets.load(art("map_border.sti")),
        note_buttons: assets.load(art("map_notebuttons.sti")),
        exit_button: assets.load(art("map_exitbutton.sti")),
        zoom_buttons: assets.load(art("map_zoombuttons.sti")),
        movement_buttons: assets.load(art("map_movementbuttons.sti")),
        layer_buttons: assets.load(art("map_layerbuttons.sti")),
        cursors: assets.load(art("map_cursors.sti")),
        font: assets.load("retail://data/fonts/wiz_text_font.sti"),
        note_marker: assets.load(art("map_textmarker_a.tga")),
        party_marker: assets.load(art("map_partymarker_a.tga")),
    });
}

/// The open automap. Absent while exploring.
#[derive(Component, Debug)]
pub struct Automap {
    level: Entity,
    tool: Option<MapTool>,
    /// The note being typed.
    editing: Option<NoteDraft>,
    /// The world x and z at the middle of the map.
    centre: Vec2,
    /// Horizontal world span; vertical span preserves the viewport aspect ratio.
    span: f32,
    bounds_min: Vec2,
    bounds_max: Vec2,
    full_span: f32,
    pub layer: usize,
}

impl Automap {
    fn new(collision: Option<&CollisionMesh>, layers: &[f32], feet: Vec3) -> Self {
        let mut min = feet.xz();
        let mut max = min;
        if let Some(collision) = collision {
            for (triangle, walkable) in collision.triangles.iter().zip(&collision.walkable) {
                if *walkable {
                    for &index in triangle {
                        let point = collision.vertices[index as usize].xz();
                        min = min.min(point);
                        max = max.max(point);
                    }
                }
            }
        }
        let size = max - min;
        let full_span = size.x.max(size.y * VIEWPORT.x / VIEWPORT.y).max(MIN_SPAN);
        Self {
            level: Entity::PLACEHOLDER,
            tool: None,
            editing: None,
            centre: (min + max) / 2.0,
            span: full_span,
            bounds_min: min,
            bounds_max: max,
            full_span,
            layer: layer_at_height(layers, feet.y),
        }
    }

    fn pixels_per_metre(&self) -> f32 {
        VIEWPORT.x / self.span
    }

    fn project(&self, point: Vec2) -> Vec2 {
        (point - self.centre) * self.pixels_per_metre() + VIEWPORT / 2.0
    }

    fn unproject(&self, pixel: Vec2) -> Vec2 {
        self.centre + (pixel - VIEWPORT / 2.0) / self.pixels_per_metre()
    }

    fn centre_on(&mut self, point: Vec2) {
        self.centre = point.clamp(self.bounds_min, self.bounds_max);
    }

    fn zoom_in(&mut self, point: Vec2) {
        if self.span > MIN_SPAN {
            self.span = if self.span == self.full_span {
                (self.full_span / 2.0).max(MIN_SPAN)
            } else {
                MIN_SPAN
            };
            self.centre_on(point);
        }
    }

    fn zoom_out(&mut self) {
        if self.span >= self.full_span {
            return;
        }
        let halfway = (self.full_span / 2.0).max(MIN_SPAN);
        if self.span < halfway {
            self.span = halfway;
        } else {
            self.reset();
        }
    }

    fn reset(&mut self) {
        self.span = self.full_span;
        self.centre = (self.bounds_min + self.bounds_max) / 2.0;
    }
}

// Both references assign markers to the band above the first lower cut.
fn layer_at_height(layers: &[f32], height: f32) -> usize {
    layers.iter().position(|&cut| cut < height).unwrap_or(layers.len())
}

#[derive(Component, Clone, Default)]
pub struct LayerLabel;

#[derive(Component, Clone, Default)]
pub struct FloorPlan;

#[derive(Component, Clone, Default)]
pub struct PartyMarker;

/// The map screen, with its keys: Escape or Tab closes it and the arrows
/// move the map, before every other screen's keys; off while a note is
/// typed.
#[derive(Component, Clone, Default)]
pub struct AutomapScreen;

#[derive(Component, Clone, Default)]
pub struct MapViewport;

/// Holds the note markers and labels, rebuilt when the notes change.
#[derive(Component, Clone, Default)]
pub struct NotesLayer;

#[derive(Component, Clone, Default)]
pub struct NoteMarker;

/// The note index is view identity only; gameplay validates editing separately.
#[derive(Component)]
pub struct NoteView(Option<usize>);

/// A button drawn from four frames of a sheet starting at `base`.
#[derive(Component)]
pub struct SheetButton {
    action: ButtonAction,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ButtonAction {
    Tool(MapTool),
    ZoomIn,
    ZoomOut,
    Reset,
    Pan(IVec2),
    CentreParty,
    LayerPrevious,
    LayerNext,
    Exit,
}

/// Open the automap from elsewhere (the main screen's radar).
#[derive(Message, Debug, Clone, Copy)]
pub struct OpenAutomap;

pub fn close_ended_map(
    map: Option<Single<&Automap>>,
    levels: Query<(), With<Level>>,
    mut next: ResMut<NextState<Screen>>,
) {
    if map.is_some_and(|map| !levels.contains(map.level)) {
        next.set(Screen::World);
    }
}

pub fn request_automap(
    mut open: MessageReader<OpenAutomap>,
    mut screen: ParamSet<(crate::game::screen::Freedom, ResMut<NextState<Screen>>)>,
) {
    if open.read().count() > 0 && screen.p0().free() {
        screen.p1().set(Screen::Automap);
    }
}

pub fn enter_automap(
    mut commands: Commands,
    party: Query<&Transform, With<PartyBody>>,
    played: PlayedLevel,
    mut next: ResMut<NextState<Screen>>,
) {
    let (Ok(party), Some(level)) = (party.single(), played.entity()) else {
        next.set(Screen::World);
        return;
    };
    let collision = played.definition();
    let layers = collision.map_or(&[][..], |g| g.map_layers.as_slice());
    let mut map = Automap::new(
        collision.map(|g| &g.collision),
        layers,
        party.translation - Vec3::Y * FLOAT_HEIGHT,
    );
    map.level = level;
    commands.spawn_scene(bsn! {
        #Automap
        @centred()
        AutomapScreen
        TabGroup::default()
        on(note_input::cancel_from_controls)
        BackgroundColor(Color::BLACK)
        ContextPriority::<AutomapScreen>::new(1)
        Actions<AutomapScreen> [
            Action::<CloseMap>::new()
            ActionSettings { consume_input: true }
            Bindings [ Binding(KeyCode::Escape) ]
            -- Action::<PanMap>::new()
            ActionSettings { consume_input: true }
            Bindings [ {preset::cardinal::arrows()} ]
        ]
        on(|_: On<Start<CloseMap>>, mut next: ResMut<NextState<Screen>>| { next.set(Screen::World); })
        on(|pan: On<Start<PanMap>>, map: Option<Single<&mut Automap>>| {
            if let Some(mut map) = map {
                let step = Vec2::new(pan.value.x, -pan.value.y) * 0.35 * map.span;
                let point = map.centre + step;
                map.centre_on(point);
            }
        })
    }).insert((map, DespawnOnExit(Screen::Automap)));
}

#[derive(Component)]
pub struct AutomapBuilt;

/// Typing focus changes input routing, independently of simulation activity.
pub fn route_map_keys(
    mut commands: Commands,
    maps: Query<(Entity, &Automap, &ContextActivity<AutomapScreen>)>,
) {
    for (entity, map, map_keys) in &maps {
        let typing = map.editing.is_some();
        if **map_keys == typing {
            commands.entity(entity).insert(ContextActivity::<AutomapScreen>::new(!typing));
        }
    }
}

/// Open the map when asked (Tab, the radar) while the party is in the
/// world, once its art has loaded.
pub fn build_automap(
    mut commands: Commands,
    art: Option<Res<AutomapArt>>,
    sheets: Res<Assets<SpriteSheet>>,
    mut images: ResMut<Assets<Image>>,
    party: Query<&Transform, With<PartyBody>>,
    levels: PlayedLevel,
    roots: Query<(Entity, &Automap), Without<AutomapBuilt>>,
) {
    let Ok((root, automap)) = roots.single() else { return };
    let (Some(art), Ok(party)) = (art, party.single()) else { return };
    let (
        Some(border),
        Some(note_buttons),
        Some(exit_button),
        Some(zoom_buttons),
        Some(movement_buttons),
        Some(layer_buttons),
    ) = (
        sheets.get(&art.border),
        sheets.get(&art.note_buttons),
        sheets.get(&art.exit_button),
        sheets.get(&art.zoom_buttons),
        sheets.get(&art.movement_buttons),
        sheets.get(&art.layer_buttons),
    )
    else {
        // The art is still loading; build once it has.
        return;
    };
    let feet = party.translation - Vec3::Y * FLOAT_HEIGHT;
    let collision = levels.of(automap.level);
    let plan = collision
        .map(|level| images.add(floor_plan(&level.collision, &level.map_layers, automap)))
        .map(|plan| {
            let node = absolute(Vec2::ZERO, VIEWPORT);
            bsn! { FloorPlan ImageNode { image: {plan} } node Pickable::IGNORE }
        });
    let button = |sheet: &SpriteSheet,
                  handle: &Handle<SpriteSheet>,
                  base: usize,
                  at: Vec2,
                  action| {
        let _ = sheet;
        bsn! {
            @crate::presentation::retail_controls::image_button(handle.clone(), at, ControlSkin::new(base, base+1, base+2, base+2, base+3))
            template(move |_| Ok(SheetButton { action }))
            on(press_button)
        }
    };
    let add = ButtonAction::Tool(MapTool::AddNote);
    let erase = ButtonAction::Tool(MapTool::EraseNote);
    let (layer, notes, zoom, moves) =
        (&art.layer_buttons, &art.note_buttons, &art.zoom_buttons, &art.movement_buttons);
    let buttons = vec![
        button(layer_buttons, layer, 0, Vec2::new(490.0, 164.0), ButtonAction::LayerPrevious),
        button(layer_buttons, layer, 4, Vec2::new(591.0, 164.0), ButtonAction::LayerNext),
        button(note_buttons, notes, 0, Vec2::new(490.0, 70.0), add),
        button(note_buttons, notes, 4, Vec2::new(541.0, 70.0), erase),
        button(zoom_buttons, zoom, 0, Vec2::new(490.0, 124.0), ButtonAction::ZoomIn),
        button(zoom_buttons, zoom, 4, Vec2::new(541.0, 124.0), ButtonAction::ZoomOut),
        button(zoom_buttons, zoom, 8, Vec2::new(591.0, 124.0), ButtonAction::Reset),
        button(
            movement_buttons,
            moves,
            0,
            Vec2::new(540.0, 218.0),
            ButtonAction::Pan(IVec2::NEG_Y),
        ),
        button(movement_buttons, moves, 4, Vec2::new(540.0, 326.0), ButtonAction::Pan(IVec2::Y)),
        button(
            movement_buttons,
            moves,
            8,
            Vec2::new(486.0, 272.0),
            ButtonAction::Pan(IVec2::NEG_X),
        ),
        button(movement_buttons, moves, 12, Vec2::new(594.0, 272.0), ButtonAction::Pan(IVec2::X)),
        button(movement_buttons, moves, 16, Vec2::new(540.0, 272.0), ButtonAction::CentreParty),
        button(exit_button, &art.exit_button, 0, Vec2::new(588.0, 441.0), ButtonAction::Exit),
    ];
    let marker = absolute(automap.project(feet.xz()) - MARKER / 2.0, Vec2::splat(MARKER));
    let marker_image = art.party_marker.clone();
    let frame = border.images[0].clone();
    let (whole, map_area) = (absolute(Vec2::ZERO, SCREEN), absolute(VIEWPORT_AT, VIEWPORT));
    let notes_layer = absolute(Vec2::ZERO, VIEWPORT);
    let label_font = art.font.clone();
    commands.entity(root).insert(AutomapBuilt).queue_spawn_related_scenes::<Children>(vec![bsn! {
    @screen()
    Children [
        // The frame goes underneath: its map area is opaque black,
        // and the map is drawn on it.
        ImageNode { image: {frame} } whole Pickable::IGNORE
        --
        MapViewport
        Hovered
        map_area
        Node { overflow: Overflow::clip() }
        BackgroundColor(Color::BLACK)
        on(click_map)
        Children [
            {plan.into_iter().collect::<Vec<_>>()}
            -- PartyMarker ImageNode { image: {marker_image} } marker Pickable::IGNORE
            -- NotesLayer notes_layer Pickable::IGNORE
        ]
        -- LayerLabel template(move |_| Ok(BitmapText { font: label_font.clone(), at: Vec2::new(498.0, 199.0), width: 120.0, justify: bevy::text::Justify::Center, ..default() }))
        -- {buttons}
    ]
    }]);
}

/// Topmost walkable surfaces below the selected authored ceiling, north up.
fn floor_plan(collision: &CollisionMesh, layers: &[f32], automap: &Automap) -> Image {
    let (width, height) = (VIEWPORT.x as usize, VIEWPORT.y as usize);
    let mut rgba = vec![0u8; width * height * 4];
    let mut depth = vec![f32::NEG_INFINITY; width * height];
    let ceiling = automap.layer.checked_sub(1).map_or(f32::INFINITY, |layer| layers[layer]);
    let (min_height, max_height) = collision
        .vertices
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), v| (min.min(v.y), max.max(v.y)));
    let to_map = |v: Vec3| automap.project(v.xz());
    for (triangle, _) in collision.triangles.iter().zip(&collision.walkable).filter(|(_, w)| **w) {
        let [a, b, c] = triangle.map(|i| collision.vertices[i as usize]);
        if a.y.min(b.y).min(c.y) > ceiling {
            continue;
        }
        let [p, q, r] = [to_map(a), to_map(b), to_map(c)];
        let min = p.min(q).min(r).floor().max(Vec2::ZERO);
        let max = p.max(q).max(r).ceil().min(VIEWPORT - 1.0);
        let area = (q - p).perp_dot(r - p);
        if area.abs() < f32::EPSILON || min.x > max.x || min.y > max.y {
            continue; // flat, or off the map
        }
        for y in min.y as usize..=max.y as usize {
            for x in min.x as usize..=max.x as usize {
                let point = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let u = (r - q).perp_dot(point - q) / area;
                let v = (p - r).perp_dot(point - r) / area;
                if u >= 0.0 && v >= 0.0 && u + v <= 1.0 {
                    let height = u * a.y + v * b.y + (1.0 - u - v) * c.y;
                    let at = y * width + x;
                    if height > ceiling || height <= depth[at] {
                        continue;
                    }
                    depth[at] = height;
                    let shade =
                        0.55 + 0.35 * (height - min_height) / (max_height - min_height).max(1.0);
                    let colour =
                        [(150.0 * shade) as u8, (130.0 * shade) as u8, (95.0 * shade) as u8, 255];
                    rgba[at * 4..at * 4 + 4].copy_from_slice(&colour);
                }
            }
        }
    }
    ui_image(width as u32, height as u32, rgba)
}

// Button tracks presses natively; reject non-primary presses before its later
// click can activate. Keeping this policy local preserves the map canvas gestures.
fn action_enabled(action: ButtonAction, map: &Automap, layers: Option<usize>) -> bool {
    match action {
        ButtonAction::LayerPrevious => map.layer > 0,
        ButtonAction::LayerNext => layers.is_some_and(|count| map.layer < count),
        ButtonAction::ZoomIn => map.span > MIN_SPAN,
        ButtonAction::ZoomOut => map.span < map.full_span,
        _ => true,
    }
}

fn press_button(
    click: On<Activate>,
    buttons: Query<&SheetButton>,
    levels: PlayedLevel,
    party: Query<&Transform, With<PartyBody>>,
    automap: Option<Single<&mut Automap>>,
    mut next: ResMut<NextState<Screen>>,
) {
    let (Some(mut automap), Ok(button)) = (automap, buttons.get(click.entity)) else { return };
    let layers = levels.of(automap.level).map(|g| g.map_layers.len());
    if !action_enabled(button.action, &automap, layers) {
        return;
    }
    automap.editing = None;
    match button.action {
        ButtonAction::Tool(tool) => {
            automap.tool = if automap.tool == Some(tool) { None } else { Some(tool) };
        }
        ButtonAction::ZoomIn => {
            let centre = automap.centre;
            automap.zoom_in(centre);
        }
        ButtonAction::ZoomOut => automap.zoom_out(),
        ButtonAction::Reset => automap.reset(),
        ButtonAction::Pan(direction) => {
            let point = automap.centre + direction.as_vec2() * 0.35 * automap.span;
            automap.centre_on(point);
        }
        ButtonAction::CentreParty => {
            if let Ok(party) = party.single() {
                automap.centre_on(party.translation.xz());
            }
        }
        ButtonAction::LayerPrevious => automap.layer = automap.layer.saturating_sub(1),
        ButtonAction::LayerNext => {
            if let Some(level) = levels.of(automap.level) {
                automap.layer = (automap.layer + 1).min(level.map_layers.len());
            }
        }
        ButtonAction::Exit => next.set(Screen::World),
    }
}

/// Off, highlighted while hovered, on while its tool is selected.
pub fn paint_buttons(
    levels: PlayedLevel,
    mut commands: Commands,
    buttons: Query<(Entity, &SheetButton, Has<InteractionDisabled>, Has<Selected>)>,
    automap: Option<Single<Ref<Automap>>>,
) {
    let Some(automap) = automap else { return };
    let layers = levels.of(automap.level).map(|g| g.map_layers.len());
    for (entity, button, disabled, selected) in &buttons {
        let on = matches!(button.action, ButtonAction::Tool(tool) if automap.tool == Some(tool));
        set_enabled(
            &mut commands,
            entity,
            action_enabled(button.action, &automap, layers),
            disabled,
        );
        if on != selected {
            if on {
                commands.entity(entity).insert(Selected);
            } else {
                commands.entity(entity).remove::<Selected>();
            }
        }
    }
}

/// Pin, erase, or (with no tool) pick a note to retype.
fn click_map(
    click: On<PointerClick>,
    map: Option<Single<&mut Automap>>,
    mut levels: Query<&mut MapNotes>,
    mut commands: Commands,
    editors: Query<Entity, With<note_input::NoteInput>>,
) {
    let (Some(mut map), Some(hit)) = (map, click.hit.position) else { return };
    for editor in &editors {
        commands.entity(editor).despawn();
    }
    use bevy::picking::pointer::PointerButton;
    if click.button == PointerButton::Secondary {
        map.editing = None;
        map.zoom_out();
        return;
    }
    if click.button != PointerButton::Primary {
        return;
    }
    map.editing = None;
    let Ok(mut notes) = levels.get_mut(map.level) else { return };
    let point = map.unproject((hit.truncate() + 0.5) * VIEWPORT);
    let reach = MARKER / map.pixels_per_metre();
    let original = notes.nearest(point, reach, map.layer);
    match map.tool {
        Some(MapTool::AddNote) if notes.has_room() => {
            map.editing = Some(NoteDraft {
                level: map.level,
                original: None,
                note: MapNote { position: point, layer: map.layer, text: String::new() },
            });
        }
        Some(MapTool::AddNote) => {}
        Some(MapTool::EraseNote) => {
            if let Some(index) = original {
                let _ = notes.erase(index);
            }
        }
        None => {
            if let Some(index) = original {
                map.editing = Some(NoteDraft {
                    level: map.level,
                    original: Some((index, notes.notes[index].clone())),
                    note: notes.notes[index].clone(),
                });
            } else {
                map.zoom_in(point);
            }
        }
    }
}

pub fn draw_notes(
    mut commands: Commands,
    layer: Query<(Entity, Ref<NotesLayer>)>,
    automap: Option<Single<Ref<Automap>>>,
    notes: Query<Ref<MapNotes>>,
    art: Option<Res<AutomapArt>>,
    mut views: Query<(Entity, &NoteView, &mut Node, &mut Visibility, &Children)>,
    mut labels: Query<(&mut BitmapText, &mut Visibility), Without<NoteView>>,
) {
    let (Some(map), Some(art), Ok((layer, built))) = (automap, art, layer.single()) else {
        return;
    };
    let Ok(notes) = notes.get(map.level) else { return };
    // The layer may be built after its art arrives, with the map unchanged.
    if !map.is_changed() && !notes.is_changed() && !built.is_added() && !art.is_changed() {
        return;
    }
    // Retain each marker/label while panning, editing and changing layers.
    let displayed = notes
        .notes
        .iter()
        .enumerate()
        .map(|(i, n)| (Some(i), n))
        .chain(map.editing.iter().map(|d| (None, &d.note)));
    let hidden = map.editing.as_ref().and_then(|d| d.original.as_ref().map(|(i, _)| *i));
    for (key, note) in displayed {
        let at = map.project(note.position);
        let show =
            key.is_some() && key != hidden && note.layer == map.layer && !note.text.is_empty();
        let text = BitmapText {
            font: art.font.clone(),
            value: note.text.clone(),
            at: Vec2::new(MARKER + 2.0, MARKER / 2.0),
            ..default()
        };
        if let Some((_, _, mut node, mut visible, children)) =
            views.iter_mut().find(|(_, v, _, _, _)| v.0 == key)
        {
            node.set_if_neq(absolute(at - MARKER / 2.0, Vec2::splat(MARKER)));
            visible.set_if_neq(if key.is_some() && key == hidden {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            });
            for child in children.iter() {
                let Ok((mut label, mut visible)) = labels.get_mut(child) else { continue };
                label.set_if_neq(text.clone());
                visible.set_if_neq(if show { Visibility::Inherited } else { Visibility::Hidden });
            }
        } else {
            let marker = commands
                .spawn((
                    NoteMarker,
                    NoteView(key),
                    ImageNode::new(art.note_marker.clone()),
                    absolute(at - MARKER / 2.0, Vec2::splat(MARKER)),
                    Pickable::IGNORE,
                    ChildOf(layer),
                ))
                .id();
            commands.spawn((
                text,
                UiTransform::from_translation(bevy::ui::Val2::new(px(0), percent(-50))),
                if show { Visibility::Inherited } else { Visibility::Hidden },
                ChildOf(marker),
            ));
        }
    }
    for (entity, view, _, _, _) in &views {
        if view.0.is_some_and(|i| i >= notes.notes.len())
            || (view.0.is_none() && map.editing.is_none())
        {
            commands.entity(entity).despawn();
        }
    }
}

pub fn glow_note_markers(time: Res<Time>, mut markers: Query<&mut ImageNode, With<NoteMarker>>) {
    let phase = (time.elapsed_secs() / GLOW_PERIOD * std::f32::consts::TAU).sin() * 0.5 + 0.5;
    let colour = Color::BLACK.mix(&Color::srgb(1.0, 0.4, 0.0), phase);
    for mut marker in &mut markers {
        marker.color = colour;
    }
}

/// Redraw the raster only when its view changes, not while typing or glowing.
pub fn update_map_view(
    automap: Option<Single<Ref<Automap>>>,
    levels: PlayedLevel,
    plans: Query<(Entity, &ImageNode), With<FloorPlan>>,
    mut images: ResMut<Assets<Image>>,
    party: Query<&Transform, With<PartyBody>>,
    mut markers: Query<&mut Node, With<PartyMarker>>,
    mut previous: Local<Option<(Entity, Vec2, f32, usize)>>,
) {
    let Some(automap) = automap else { return };
    let Some(level) = levels.of(automap.level) else {
        return;
    };
    if let Ok(party) = party.single() {
        let at = automap.project(party.translation.xz()) - MARKER / 2.0;
        for mut marker in &mut markers {
            if (marker.left, marker.top) != (Val::Px(at.x), Val::Px(at.y)) {
                (marker.left, marker.top) = (Val::Px(at.x), Val::Px(at.y));
            }
        }
    }
    let Ok((entity, image)) = plans.single() else { return };
    let view = (entity, automap.centre, automap.span, automap.layer);
    if *previous == Some(view) {
        return;
    }
    if let Some(mut image) = images.get_mut(&image.image) {
        *image = floor_plan(&level.collision, &level.map_layers, &automap);
        *previous = Some(view);
    }
}

pub fn draw_layer_label(
    automap: Option<Single<&Automap>>,
    levels: PlayedLevel,
    mut labels: Query<&mut BitmapText, With<LayerLabel>>,
) {
    let Some(map) = automap else { return };
    let Some(level) = levels.of(map.level) else {
        return;
    };
    let value = format!("Layer {}/{}", map.layer + 1, level.map_layers.len() + 1);
    for mut label in &mut labels {
        if label.value != value {
            label.value.clone_from(&value);
        }
    }
}

/// Let the window backend draw retail's cursors with their original
/// hotspots: the automap's tools, and elsewhere retail's arrow (frame 0 of
/// `2D-Cursors.sti`, the first entry of retail's video object catalog,
/// hotspot at its tip).
pub fn update_map_cursor(
    mut commands: Commands,
    windows: Query<(Entity, Option<&bevy::window::CursorIcon>), With<bevy::window::PrimaryWindow>>,
    automap: Option<Single<Ref<Automap>>>,
    art: Option<Res<AutomapArt>>,
    sheets: Res<Assets<SpriteSheet>>,
    viewport: Query<&Hovered, With<MapViewport>>,
    assets: Res<AssetServer>,
    mut arrow: Local<Option<Handle<SpriteSheet>>>,
) {
    use bevy::window::{CursorIcon, CustomCursor, CustomCursorImage};
    let Ok((window, current)) = windows.single() else { return };
    let arrow = arrow.get_or_insert_with(|| assets.load("retail://data/cursors/2d-cursors.sti"));
    let cursor = match (automap, art) {
        (Some(map), Some(art)) => {
            let inside = viewport.single().is_ok_and(|hovered| hovered.get());
            let index = match map.tool {
                Some(MapTool::AddNote) => 2,
                Some(MapTool::EraseNote) => 3,
                None if inside && map.span > MIN_SPAN => 1,
                None if inside => 4,
                None => 0,
            };
            // AutomapScreen.cpp's cursor offsets, in both references.
            let hotspot = [(0, 0), (8, 7), (1, 24), (1, 24), (8, 7)][index];
            sheets.get(&art.cursors).and_then(|sheet| sheet.images.get(index)).map_or_else(
                CursorIcon::default,
                |handle| {
                    CursorIcon::Custom(CustomCursor::Image(CustomCursorImage {
                        handle: handle.clone(),
                        hotspot,
                        ..default()
                    }))
                },
            )
        }
        _ => sheets.get(&*arrow).and_then(|sheet| sheet.images.first()).map_or_else(
            CursorIcon::default,
            |handle| {
                CursorIcon::Custom(CustomCursor::Image(CustomCursorImage {
                    handle: handle.clone(),
                    ..default()
                }))
            },
        ),
    };
    if current != Some(&cursor) {
        commands.entity(window).insert(cursor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ui_widgets::Button;

    fn large_map() -> Automap {
        let collision = CollisionMesh {
            vertices: vec![Vec3::ZERO, Vec3::new(400.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 200.0)],
            triangles: vec![[0, 1, 2]],
            walkable: vec![true],
        };
        Automap::new(Some(&collision), &[], Vec3::ZERO)
    }

    #[test]
    fn disabled_map_actions_leave_the_note_draft_untouched() {
        use bevy::input::keyboard::Key;
        use bevy::input_focus::{FocusedInput, InputFocus};
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::ui_widgets::ButtonPlugin))
            .init_resource::<Assets<crate::content::level::LevelDefinition>>()
            .init_resource::<NextState<Screen>>()
            .init_resource::<InputFocus>();
        let mut map = Automap::new(None, &[], Vec3::ZERO);
        map.editing = Some(NoteDraft {
            level: Entity::PLACEHOLDER,
            original: None,
            note: MapNote { position: Vec2::ZERO, layer: 0, text: "keep me".into() },
        });
        let root = app.world_mut().spawn(map).id();
        for action in [
            ButtonAction::ZoomIn,
            ButtonAction::ZoomOut,
            ButtonAction::LayerPrevious,
            ButtonAction::LayerNext,
        ] {
            let button =
                app.world_mut().spawn((Button, SheetButton { action })).observe(press_button).id();
            // Even a queued activation must recheck prerequisites before clearing the draft.
            app.world_mut().trigger(Activate { entity: button });
            app.update();
            assert_eq!(
                app.world().get::<Automap>(root).unwrap().editing.as_ref().unwrap().note.text,
                "keep me"
            );
        }
        let button = app
            .world_mut()
            .spawn((Button, SheetButton { action: ButtonAction::Tool(MapTool::AddNote) }))
            .observe(press_button)
            .id();
        app.world_mut().trigger(FocusedInput::new(
            button,
            KeyboardInput {
                key_code: KeyCode::Enter,
                logical_key: Key::Enter,
                state: ButtonState::Pressed,
                text: None,
                repeat: false,
                window: Entity::PLACEHOLDER,
            },
            Entity::PLACEHOLDER,
        ));
        app.update();
        let map = app.world().get::<Automap>(root).unwrap();
        assert_eq!(map.tool, Some(MapTool::AddNote));
        assert!(map.editing.is_none());
    }

    #[test]
    fn authored_cuts_reveal_lower_floors_without_triangle_order_artifacts() {
        let mut collision = CollisionMesh::default();
        for height in [0.0, 10.0] {
            let start = collision.vertices.len() as u32;
            collision.vertices.extend([
                Vec3::new(-20.0, height, -20.0),
                Vec3::new(20.0, height, -20.0),
                Vec3::new(0.0, height, 20.0),
            ]);
            collision.triangles.push([start, start + 1, start + 2]);
            collision.walkable.push(true);
        }
        let layers = [5.0];
        let mut map = Automap::new(Some(&collision), &layers, Vec3::new(0.0, 10.0, 0.0));
        assert_eq!(map.layer, 0);
        assert_eq!(layer_at_height(&layers, 5.0), 1);
        assert_eq!(layer_at_height(&layers, 0.0), 1);
        let upper = floor_plan(&collision, &layers, &map);
        collision.triangles.reverse();
        let reversed = floor_plan(&collision, &layers, &map);
        assert_eq!(upper.data, reversed.data);
        map.layer = 1;
        let lower = floor_plan(&collision, &layers, &map);
        let pixel = ((VIEWPORT.y as usize / 2) * VIEWPORT.x as usize + VIEWPORT.x as usize / 2) * 4;
        let upper = &upper.data.unwrap()[pixel..pixel + 4];
        let lower = &lower.data.unwrap()[pixel..pixel + 4];
        assert_eq!(upper[3], 255);
        assert_eq!(lower[3], 255);
        assert!(upper[0] > lower[0], "the high floor should be lighter");
    }

    #[test]
    fn a_slope_is_clipped_at_the_cut_instead_of_discarded_as_a_whole() {
        let collision = CollisionMesh {
            vertices: vec![
                Vec3::new(-20.0, 0.0, -20.0),
                Vec3::new(20.0, 10.0, -20.0),
                Vec3::new(0.0, 5.0, 20.0),
            ],
            triangles: vec![[0, 1, 2]],
            walkable: vec![true],
        };
        let map = Automap::new(Some(&collision), &[5.0], Vec3::ZERO);
        let image = floor_plan(&collision, &[5.0], &map);
        let rgba = image.data.unwrap();
        let alpha = |world| {
            let pixel = map.project(world).as_uvec2();
            rgba[(pixel.y as usize * VIEWPORT.x as usize + pixel.x as usize) * 4 + 3]
        };
        assert_eq!(alpha(Vec2::new(-10.0, -10.0)), 255);
        assert_eq!(alpha(Vec2::new(10.0, -10.0)), 0);
    }

    #[test]
    fn navigating_keeps_notes_and_clicks_in_world_coordinates() {
        let mut map = large_map();
        let note = Vec2::new(100.0, 80.0);
        let mut notes = MapNotes::default();
        notes.notes.push(MapNote { position: note, layer: 0, text: "Note".into() });
        for span in [map.full_span, map.full_span / 2.0, MIN_SPAN] {
            map.span = span;
            map.centre_on(Vec2::new(90.0, 75.0));
            let pixel = map.project(note);
            let hit = map.unproject(pixel);
            assert!(hit.distance(note) < 0.0001);
            assert_eq!(notes.nearest(hit, MARKER / map.pixels_per_metre(), 0), Some(0));
            // A world metre has the same scale on either viewport axis.
            assert!(
                (map.project(note + Vec2::X).x - pixel.x - map.pixels_per_metre()).abs() < 0.0001
            );
            assert!(
                (map.project(note + Vec2::Y).y - pixel.y - map.pixels_per_metre()).abs() < 0.0001
            );
        }
    }

    #[test]
    fn zoom_stops_at_limits_and_panning_stays_in_bounds() {
        let mut map = large_map();
        map.zoom_in(Vec2::splat(100.0));
        assert_eq!(map.span, 200.0);
        map.zoom_in(Vec2::splat(100.0));
        assert_eq!(map.span, MIN_SPAN);
        map.zoom_in(Vec2::ZERO);
        assert_eq!(map.span, MIN_SPAN);
        assert_eq!(map.centre, Vec2::splat(100.0));
        map.centre_on(Vec2::new(-1000.0, 1000.0));
        assert_eq!(map.centre, Vec2::new(0.0, 200.0));
        map.zoom_out();
        assert_eq!(map.span, 200.0);
        map.zoom_out();
        assert_eq!(map.span, 400.0);
        assert_eq!(map.centre, Vec2::new(200.0, 100.0));
        map.zoom_out();
        assert_eq!(map.span, map.full_span);
        let mut empty = Automap::new(None, &[], Vec3::ZERO);
        empty.zoom_in(Vec2::ONE);
        empty.zoom_out();
        assert_eq!(empty.span, MIN_SPAN);
        assert_eq!(empty.centre, Vec2::ZERO);
    }
}
