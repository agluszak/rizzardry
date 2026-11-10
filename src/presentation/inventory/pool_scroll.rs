//! The item pool's two-column retail scrollbar.
use super::{InventoryArt, POOL_CELLS};
use bevy::prelude::*;
use bevy::ui_widgets::{SliderRange, SliderValue};

#[derive(Component)]
pub(super) struct PoolScroll;

fn maximum(items: usize) -> usize {
    (items.max(POOL_CELLS) - POOL_CELLS).div_ceil(2)
}

pub(super) fn build(commands: &mut Commands, root: Entity, art: &InventoryArt) -> Entity {
    let track = crate::presentation::retail_controls::step_scrollbar(
        commands,
        root,
        art.scroll.clone(),
        Vec2::new(611.0, 193.0),
        192.0,
        0.0,
    );
    commands.entity(track).insert(PoolScroll);
    track
}

pub(super) fn sync(
    slider: Single<(Entity, &SliderValue, &SliderRange), With<PoolScroll>>,
    party: Single<Entity, With<crate::game::party::roster::PartyRoster>>,
    held: crate::game::items::Items,
    mut commands: Commands,
) {
    let (slider, value, range) = *slider;
    let max = maximum(held.pool(*party).len()) as f32;
    crate::presentation::retail_controls::set_scroll_maximum(
        &mut commands,
        slider,
        value,
        range,
        max,
    );
}
