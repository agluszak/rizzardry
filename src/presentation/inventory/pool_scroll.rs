//! The item pool's two-column retail scrollbar.
use super::{InventoryArt, POOL_CELLS};
use crate::game::inventory::PartyInventory;
use bevy::prelude::*;
use bevy::ui_widgets::{SliderRange, SliderValue};

#[derive(Component)]
pub(super) struct PoolScroll;

fn maximum(items: usize) -> usize {
    items.saturating_sub(POOL_CELLS).div_ceil(2)
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
    party: Single<&PartyInventory>,
    mut commands: Commands,
) {
    let (slider, value, range) = *slider;
    let max = maximum(party.pool.stacks.len()) as f32;
    crate::presentation::retail_controls::set_scroll_maximum(
        &mut commands,
        slider,
        value,
        range,
        max,
    );
}
