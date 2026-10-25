//! The formation board (retail `DrawFormationSlotMarkers`): the cage, the
//! view cone, and each member's facing arrow and marching-order chip in
//! their cell.

use bevy::prelude::*;

use super::{MainScreenArt, layer, loaded};
use crate::content::ui::SpriteSheet;
use crate::game::party::formation::{Formation, Row};
use crate::game::party::roster::PartyRoster;
use crate::presentation::screen_art::sprite;

const BOARD: Vec2 = Vec2::new(519.0, 359.0);

/// Where each seat of a formation row has its marker on the board (retail
/// `g_formation_marker_offsets`).
fn cell(row: Row, seat: usize) -> Vec2 {
    let seats = match row {
        Row::Front => [(38.0, 9.0), (25.0, 13.0), (51.0, 13.0)],
        Row::Right => [(67.0, 38.0), (63.0, 25.0), (63.0, 51.0)],
        Row::Rear => [(38.0, 67.0), (51.0, 63.0), (25.0, 63.0)],
        Row::Left => [(9.0, 38.0), (13.0, 51.0), (13.0, 25.0)],
        Row::Center => [(38.0, 30.0), (30.0, 42.0), (46.0, 42.0)],
    };
    BOARD + Vec2::from(seats[seat])
}

/// The board's markers, with the places they last showed, by slot.
#[derive(Component, Clone, Default)]
pub struct FormationBoard {
    drawn: Option<Vec<Option<(Row, usize)>>>,
}

pub fn panel(art: &MainScreenArt, sheets: &Assets<SpriteSheet>) -> impl SceneList {
    let cage = sprite(loaded(sheets, &art.formation_cage), 0, BOARD);
    let cone = sprite(loaded(sheets, &art.view_cone), 0, BOARD);
    bsn_list! { @{cage} -- @{cone} -- @layer() FormationBoard }
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
    let places: Vec<_> = roster.0.iter().map(|&member| formation.place_of(member)).collect();
    if board.drawn.as_ref() == Some(&places) {
        return;
    }
    let (symbols, jewels) =
        (loaded(&sheets, &art.formation_symbols), loaded(&sheets, &art.formation_jewels));
    let mut markers = vec![];
    for (slot, place) in places.iter().enumerate() {
        let Some((row, seat)) = *place else { continue };
        let at = cell(row, seat);
        // The arrow shows the way the row faces; the center faces ahead
        // (retail `SetFormationPosition`).
        let facing = match row {
            Row::Front | Row::Center => 0,
            Row::Right => 3,
            Row::Rear => 6,
            Row::Left => 9,
        };
        markers.push(sprite(symbols, facing, at));
        markers.push(sprite(jewels, slot * 3, at));
    }
    commands
        .entity(node)
        .despawn_related::<Children>()
        .queue_spawn_related_scenes::<Children>(markers);
    board.drawn = Some(places);
}
