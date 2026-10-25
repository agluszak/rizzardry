//! The automap screen, in native Bevy UI from retail art.
//!
//! Layout and art after Whizzardry `AutomapScreen.cpp`: a 640×480 frame
//! (`map_border.sti`) scaled to the window, the map in the rectangle
//! (12, 32)–(468, 468), and four-state buttons (off, highlighted, on,
//! greyed). The map shows the floors around the party, which retail draws
//! from the visited parts of the level. Notes are pinned with the Add note
//! tool, typed in the retail text font, and erased with Erase note.

use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;

use crate::content::level::{CollisionMesh, LevelGeometry};
use crate::content::ui::{SpriteSheet, ui_image};
use crate::game::automap::MapNotes;
use crate::game::party::body::{Exploration, FLOAT_HEIGHT, PartyBody};
use crate::game::world::level::Level;

const SCREEN: Vec2 = Vec2::new(640.0, 480.0);
const VIEWPORT_AT: Vec2 = Vec2::new(12.0, 32.0);
const VIEWPORT: Vec2 = Vec2::new(456.0, 436.0);
const PIXELS_PER_METRE: f32 = 4.0;
/// Markers are drawn this many screen pixels wide.
const MARKER: f32 = 16.0;
/// Floors this far below or above the party's feet are on its layer.
const LAYER_BELOW: f32 = 2.5;
const LAYER_ABOVE: f32 = 2.5;
const EDIT_BOX: Color = Color::srgb(239.0 / 255.0, 155.0 / 255.0, 86.0 / 255.0);
/// Note markers glow from black to orange and back in this many seconds.
const GLOW_PERIOD: f32 = 2.0;

/// Input context for opening and leaving screens.
#[derive(Component, Default, Clone)]
pub struct Screens;

#[derive(InputAction)]
#[action_output(bool)]
pub struct ToggleMap;

#[derive(InputAction)]
#[action_output(bool)]
pub struct Back;

pub fn spawn_screen_controls(mut commands: Commands) {
    commands.spawn_scene(bsn! {
        Screens
        Actions<Screens> [
            Action::<ToggleMap>::new()
            Bindings [
                Binding(KeyCode::KeyM)
            ]
            --
            Action::<Back>::new()
            Bindings [
                Binding(KeyCode::Escape)
            ]
        ]
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
        font: assets.load("retail://data/fonts/wiz_text_font.sti"),
        note_marker: assets.load(art("map_textmarker_a.tga")),
        party_marker: assets.load(art("map_partymarker_a.tga")),
    });
}

/// The open automap. Absent while exploring.
#[derive(Resource, Debug)]
pub struct Automap {
    tool: Option<MapTool>,
    /// The note being typed.
    editing: Option<usize>,
    /// The world x and z at the middle of the map.
    centre: Vec2,
    close: bool,
}

#[derive(Component)]
pub struct AutomapScreen;

#[derive(Component)]
pub struct MapViewport;

/// Holds the note markers and labels, rebuilt when the notes change.
#[derive(Component)]
pub struct NotesLayer;

#[derive(Component)]
pub struct NoteMarker;

/// A button drawn from four frames of a sheet starting at `base`.
#[derive(Component)]
pub struct SheetButton {
    sheet: Handle<SpriteSheet>,
    base: usize,
    action: ButtonAction,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ButtonAction {
    Tool(MapTool),
    Exit,
}

/// Fit the 640×480 screen layout into the window.
pub fn scale_ui(windows: Query<&Window>, mut scale: ResMut<UiScale>) {
    if let Ok(window) = windows.single() {
        let fit = (window.width() / SCREEN.x).min(window.height() / SCREEN.y);
        if scale.0 != fit {
            scale.0 = fit;
        }
    }
}

pub fn toggle_automap(
    mut commands: Commands,
    toggle: Single<&ActionEvents, With<Action<ToggleMap>>>,
    back: Single<&ActionEvents, With<Action<Back>>>,
    automap: Option<Res<Automap>>,
    art: Option<Res<AutomapArt>>,
    sheets: Res<Assets<SpriteSheet>>,
    mut images: ResMut<Assets<Image>>,
    party: Query<&Transform, With<PartyBody>>,
    level: Query<&Level>,
    geometry: Res<Assets<LevelGeometry>>,
    screens: Query<Entity, With<AutomapScreen>>,
    exploration: Query<Entity, With<Exploration>>,
) {
    let toggled = toggle.contains(ActionEvents::START);
    let backed = back.contains(ActionEvents::START);
    if let Some(automap) = automap {
        if toggled || backed || automap.close {
            for screen in &screens {
                commands.entity(screen).despawn();
            }
            commands.remove_resource::<Automap>();
            for controls in &exploration {
                commands.entity(controls).insert(ContextActivity::<Exploration>::ACTIVE);
            }
        }
        return;
    }
    if !toggled {
        return;
    }
    let (Some(art), Ok(party)) = (art, party.single()) else { return };
    let (Some(border), Some(note_buttons), Some(exit_button)) =
        (sheets.get(&art.border), sheets.get(&art.note_buttons), sheets.get(&art.exit_button))
    else {
        warn!("the automap's art is still loading");
        return;
    };
    let feet = party.translation - Vec3::Y * FLOAT_HEIGHT;
    let collision = level.single().ok().and_then(|level| geometry.get(&level.geometry));
    let plan = collision.map(|level| images.add(floor_plan(&level.collision, feet)));
    let centre = feet.xz();

    let button = |sheet: &SpriteSheet, handle: &Handle<SpriteSheet>, base, at: Vec2, action| {
        (
            SheetButton { sheet: handle.clone(), base, action },
            Hovered::default(),
            ImageNode::new(sheet.images[base].clone()),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(at.x),
                top: Val::Px(at.y),
                ..default()
            },
        )
    };
    let absolute = |at: Vec2, size: Vec2| Node {
        position_type: PositionType::Absolute,
        left: Val::Px(at.x),
        top: Val::Px(at.y),
        width: Val::Px(size.x),
        height: Val::Px(size.y),
        ..default()
    };
    let screen = commands
        .spawn((
            Name::new("Automap"),
            AutomapScreen,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::BLACK),
        ))
        .id();
    let frame = commands
        .spawn((
            Node { width: Val::Px(SCREEN.x), height: Val::Px(SCREEN.y), ..default() },
            ChildOf(screen),
        ))
        .id();
    // The frame goes underneath: its map area is opaque black, and the map
    // is drawn on it.
    commands.spawn((
        ImageNode::new(border.images[0].clone()),
        absolute(Vec2::ZERO, SCREEN),
        Pickable::IGNORE,
        ChildOf(frame),
    ));
    let viewport = commands
        .spawn((
            MapViewport,
            Node { overflow: Overflow::clip(), ..absolute(VIEWPORT_AT, VIEWPORT) },
            BackgroundColor(Color::BLACK),
            ChildOf(frame),
        ))
        .observe(click_map)
        .id();
    if let Some(plan) = plan {
        commands.spawn((
            ImageNode::new(plan),
            absolute(Vec2::ZERO, VIEWPORT),
            Pickable::IGNORE,
            ChildOf(viewport),
        ));
    }
    commands.spawn((
        ImageNode::new(art.party_marker.clone()),
        absolute(VIEWPORT / 2.0 - MARKER / 2.0, Vec2::splat(MARKER)),
        Pickable::IGNORE,
        ChildOf(viewport),
    ));
    commands.spawn((
        NotesLayer,
        absolute(Vec2::ZERO, VIEWPORT),
        Pickable::IGNORE,
        ChildOf(viewport),
    ));
    let add = ButtonAction::Tool(MapTool::AddNote);
    let erase = ButtonAction::Tool(MapTool::EraseNote);
    for button in [
        button(note_buttons, &art.note_buttons, 0, Vec2::new(490.0, 70.0), add),
        button(note_buttons, &art.note_buttons, 4, Vec2::new(541.0, 70.0), erase),
        button(exit_button, &art.exit_button, 0, Vec2::new(588.0, 441.0), ButtonAction::Exit),
    ] {
        commands.spawn((button, ChildOf(frame))).observe(press_button);
    }
    commands.insert_resource(Automap { tool: None, editing: None, centre, close: false });
    for controls in &exploration {
        commands.entity(controls).insert(ContextActivity::<Exploration>::INACTIVE);
    }
}

/// The level's floors near the feet' height, seen from above, north up.
fn floor_plan(collision: &CollisionMesh, feet: Vec3) -> Image {
    let (width, height) = (VIEWPORT.x as usize, VIEWPORT.y as usize);
    let mut rgba = vec![0u8; width * height * 4];
    let to_map = |v: Vec3| (v.xz() - feet.xz()) * PIXELS_PER_METRE + VIEWPORT / 2.0;
    for (triangle, _) in collision.triangles.iter().zip(&collision.walkable).filter(|(_, w)| **w) {
        let [a, b, c] = triangle.map(|i| collision.vertices[i as usize]);
        let rise = (a.y + b.y + c.y) / 3.0 - feet.y;
        if !(-LAYER_BELOW..=LAYER_ABOVE).contains(&rise) {
            continue;
        }
        // Higher floors are drawn lighter.
        let shade = 0.55 + 0.35 * (rise + LAYER_BELOW) / (LAYER_BELOW + LAYER_ABOVE);
        let colour = [(150.0 * shade) as u8, (130.0 * shade) as u8, (95.0 * shade) as u8, 255];
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
                    let at = (y * width + x) * 4;
                    rgba[at..at + 4].copy_from_slice(&colour);
                }
            }
        }
    }
    ui_image(width as u32, height as u32, rgba)
}

fn press_button(
    click: On<PointerClick>,
    buttons: Query<&SheetButton>,
    automap: Option<ResMut<Automap>>,
    mut notes: ResMut<MapNotes>,
) {
    let (Some(mut automap), Ok(button)) = (automap, buttons.get(click.entity)) else { return };
    finish_editing(&mut automap, &mut notes);
    match button.action {
        ButtonAction::Tool(tool) => {
            automap.tool = if automap.tool == Some(tool) { None } else { Some(tool) };
        }
        ButtonAction::Exit => automap.close = true,
    }
}

/// Off, highlighted while hovered, on while its tool is selected.
pub fn paint_buttons(
    mut buttons: Query<(&Hovered, &SheetButton, &mut ImageNode)>,
    automap: Option<Res<Automap>>,
    sheets: Res<Assets<SpriteSheet>>,
) {
    let Some(automap) = automap else { return };
    for (hovered, button, mut image) in &mut buttons {
        let on = matches!(button.action, ButtonAction::Tool(tool) if automap.tool == Some(tool));
        let state = if on { 2 } else { hovered.get() as usize };
        if let Some(frame) =
            sheets.get(&button.sheet).and_then(|s| s.images.get(button.base + state))
            && image.image != *frame
        {
            image.image = frame.clone();
        }
    }
}

/// Pin, erase, or (with no tool) pick a note to retype.
fn click_map(
    click: On<PointerClick>,
    automap: Option<ResMut<Automap>>,
    mut notes: ResMut<MapNotes>,
) {
    // UI hits are relative to the node's centre, in node sizes.
    let (Some(mut automap), Some(hit)) = (automap, click.hit.position) else { return };
    finish_editing(&mut automap, &mut notes);
    let point = automap.centre + hit.truncate() * VIEWPORT / PIXELS_PER_METRE;
    let reach = MARKER / PIXELS_PER_METRE;
    match automap.tool {
        Some(MapTool::AddNote) => automap.editing = notes.add(point),
        Some(MapTool::EraseNote) => {
            if let Some(note) = notes.nearest(point, reach) {
                notes.notes.remove(note);
            }
        }
        None => automap.editing = notes.nearest(point, reach),
    }
}

/// Stop typing; a note left empty is removed.
fn finish_editing(automap: &mut Automap, notes: &mut MapNotes) {
    if let Some(note) = automap.editing.take()
        && notes.notes.get(note).is_some_and(|n| n.text.is_empty())
    {
        notes.notes.remove(note);
    }
}

pub fn type_note(
    mut keys: MessageReader<KeyboardInput>,
    automap: Option<ResMut<Automap>>,
    mut notes: ResMut<MapNotes>,
    mut screens: Query<Entity, With<Screens>>,
    mut commands: Commands,
) {
    let Some(mut automap) = automap else {
        keys.clear();
        return;
    };
    // While typing, M and Escape are text keys, not screen keys.
    let typing = automap.editing.is_some();
    for controls in &mut screens {
        commands.entity(controls).insert(ContextActivity::<Screens>::new(!typing));
    }
    let Some(note) = automap.editing else {
        keys.clear();
        return;
    };
    for key in keys.read().filter(|k| k.state == ButtonState::Pressed) {
        match &key.logical_key {
            Key::Enter | Key::Escape => {
                finish_editing(&mut automap, &mut notes);
                return;
            }
            Key::Backspace => {
                notes.notes[note].text.pop();
            }
            _ => {
                for c in key.text.iter().flat_map(|text| text.chars()) {
                    notes.type_char(note, c);
                }
            }
        }
    }
}

pub fn draw_notes(
    mut commands: Commands,
    layer: Query<Entity, With<NotesLayer>>,
    automap: Option<Res<Automap>>,
    notes: Res<MapNotes>,
    art: Option<Res<AutomapArt>>,
    sheets: Res<Assets<SpriteSheet>>,
    mut images: ResMut<Assets<Image>>,
    mut drawn: Local<Option<Entity>>,
) {
    let (Some(automap), Some(art), Ok(layer)) = (automap, art, layer.single()) else { return };
    let fresh = *drawn != Some(layer);
    if !fresh && !notes.is_changed() && !automap.is_changed() {
        return;
    }
    *drawn = Some(layer);
    let Some(font) = sheets.get(&art.font) else { return };
    commands.entity(layer).despawn_related::<Children>();
    for (index, note) in notes.notes.iter().enumerate() {
        let at = (note.position - automap.centre) * PIXELS_PER_METRE + VIEWPORT / 2.0;
        commands.spawn((
            NoteMarker,
            ImageNode::new(art.note_marker.clone()),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(at.x - MARKER / 2.0),
                top: Val::Px(at.y - MARKER / 2.0),
                width: Val::Px(MARKER),
                height: Val::Px(MARKER),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(layer),
        ));
        let editing = automap.editing == Some(index);
        if note.text.is_empty() && !editing {
            continue;
        }
        let label = font.text_image(&note.text);
        let size = label.size_f32();
        let border = if editing { 1.0 } else { 0.0 };
        let label_box = commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(at.x + MARKER / 2.0 + 2.0 - border),
                    top: Val::Px(at.y - size.y / 2.0 - border),
                    border: UiRect::all(Val::Px(border)),
                    min_width: Val::Px(4.0),
                    ..default()
                },
                BorderColor::from(EDIT_BOX),
                Pickable::IGNORE,
                ChildOf(layer),
            ))
            .id();
        if !note.text.is_empty() {
            commands.spawn((
                ImageNode::new(images.add(label)),
                Node { width: Val::Px(size.x), height: Val::Px(size.y), ..default() },
                Pickable::IGNORE,
                ChildOf(label_box),
            ));
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
