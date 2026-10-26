//! The formation board (retail `DrawFormationSlotMarkers`): the cage, the
//! view cone, and each member's facing arrow and marching-order chip in
//! their cell.

use bevy::prelude::*;

use super::{MainScreenArt, layer, loaded};
use crate::content::ui::SpriteSheet;
use crate::game::party::formation::{Formation, Row};
use crate::game::party::roster::PartyRoster;
use crate::presentation::screen_art::{Picture, picture, sprite};

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

#[derive(Component, Clone)]
pub(super) struct FormationMarker {
    slot: usize,
    arrow: bool,
}

pub fn panel(art: &MainScreenArt, sheets: &Assets<SpriteSheet>) -> impl SceneList {
    let cage = sprite(loaded(sheets, &art.formation_cage), 0, BOARD);
    let cone = sprite(loaded(sheets, &art.view_cone), 0, BOARD);
    let markers: Vec<_> = (0..8).flat_map(|slot| [true, false].map(move |arrow| (slot, arrow)))
        .map(|(slot, arrow)| {
            let sheet = if arrow { art.formation_symbols.clone() } else { art.formation_jewels.clone() };
            bsn! { @picture(sheet, 0, BOARD) template(move |_| Ok(FormationMarker { slot, arrow })) Visibility::Hidden }
        }).collect();
    bsn_list! { @{cage} -- @{cone} -- @layer() Children [{markers}] }
}

pub fn draw_formation(
    roster: Res<PartyRoster>,
    formation: Res<Formation>,
    mut markers: Query<(&FormationMarker, &mut Picture, &mut Visibility)>,
) {
    for (marker, mut picture, mut visibility) in &mut markers {
        let place = roster.0.get_index(marker.slot).and_then(|&actor| formation.place_of(actor));
        let Some((row, seat)) = place else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };
        visibility.set_if_neq(Visibility::Inherited);
        let frame = if marker.arrow {
            match row {
                Row::Front | Row::Center => 0,
                Row::Right => 3,
                Row::Rear => 6,
                Row::Left => 9,
            }
        } else {
            marker.slot * 3
        };
        let source = Picture { frame, at: cell(row, seat), ..picture.clone() };
        picture.set_if_neq(source);
    }
}
