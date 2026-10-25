//! The formation board (retail `DrawFormationSlotMarkers`): the cage, the
//! view cone, and each member's facing arrow and marching-order chip in
//! their cell.

use bevy::prelude::*;

use super::{MainScreenArt, layer, loaded, put};
use crate::content::ui::SpriteSheet;
use crate::game::party::formation::{Formation, Row};
use crate::game::party::roster::PartyRoster;
use crate::presentation::screen_art::sprite;

const BOARD: Vec2 = Vec2::new(519.0, 359.0);

/// Each formation cell's marker on the board, by row (front, right, rear,
/// left, center) and seat (retail `g_formation_marker_offsets`).
const CELLS: [(f32, f32); 15] = [
    (38.0, 9.0),
    (25.0, 13.0),
    (51.0, 13.0),
    (67.0, 38.0),
    (63.0, 25.0),
    (63.0, 51.0),
    (38.0, 67.0),
    (51.0, 63.0),
    (25.0, 63.0),
    (9.0, 38.0),
    (13.0, 51.0),
    (13.0, 25.0),
    (38.0, 30.0),
    (30.0, 42.0),
    (46.0, 42.0),
];

/// The board's markers, with the places they last showed, by slot.
#[derive(Component, Default)]
pub struct FormationBoard {
    drawn: Option<Vec<Option<(Row, usize)>>>,
}

pub fn spawn(
    commands: &mut Commands,
    frame: Entity,
    art: &MainScreenArt,
    sheets: &Assets<SpriteSheet>,
) {
    put(commands, frame, sprite(loaded(sheets, &art.formation_cage), 0, BOARD));
    put(commands, frame, sprite(loaded(sheets, &art.view_cone), 0, BOARD));
    layer(commands, frame, FormationBoard::default());
}

pub fn draw_formation(
    mut commands: Commands,
    art: Res<MainScreenArt>,
    sheets: Res<Assets<SpriteSheet>>,
    roster: Res<PartyRoster>,
    formation: Res<Formation>,
    mut boards: Query<(Entity, &mut FormationBoard)>,
) {
    let Ok((node, mut board)) = boards.single_mut() else { return };
    let places: Vec<_> = (0..roster.0.len()).map(|slot| formation.place_of(slot)).collect();
    if board.drawn.as_ref() == Some(&places) {
        return;
    }
    commands.entity(node).despawn_related::<Children>();
    let (symbols, jewels) =
        (loaded(&sheets, &art.formation_symbols), loaded(&sheets, &art.formation_jewels));
    for (slot, place) in places.iter().enumerate() {
        let Some((row, seat)) = *place else { continue };
        let (x, y) = CELLS[usize::from(row) * 3 + seat];
        let at = BOARD + Vec2::new(x, y);
        put(&mut commands, node, sprite(symbols, Formation::facing(row) * 3, at));
        put(&mut commands, node, sprite(jewels, slot * 3, at));
    }
    board.drawn = Some(places);
}
