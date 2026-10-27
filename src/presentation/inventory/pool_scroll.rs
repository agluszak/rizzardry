//! The item pool's two-column retail scrollbar.
use super::{Cell, InventoryArt, ItemCell, POOL_CELLS, absolute, review};
use crate::game::inventory::PartyInventory;
use bevy::picking::events::PointerScroll;
use bevy::prelude::*;
use bevy::ui_widgets::{SetSliderValue, SliderRange, SliderValue, SliderValueChange};

#[derive(Component)]
pub(super) struct PoolScroll;

fn maximum(items: usize) -> usize {
    items.saturating_sub(POOL_CELLS).div_ceil(2)
}

pub(super) fn build(commands: &mut Commands, root: Entity, art: &InventoryArt) {
    let panel = commands
        .spawn((
            review::Page::Items,
            absolute(Vec2::ZERO, Vec2::new(640.0, 450.0)),
            Pickable::IGNORE,
            ChildOf(root),
        ))
        .id();
    let track = crate::presentation::retail_controls::step_scrollbar(
        commands,
        panel,
        art.scroll.clone(),
        Vec2::new(611.0, 193.0),
        192.0,
        0.0,
    );
    commands.entity(track).insert(PoolScroll);
}

pub(super) fn wheel(
    event: On<PointerScroll>,
    cells: Query<&ItemCell>,
    slider: Option<Single<Entity, With<PoolScroll>>>,
    mut commands: Commands,
) {
    if let (Ok(cell), Some(slider)) = (cells.get(event.entity), slider)
        && matches!(cell.cell, Cell::Pool(_))
        && event.y != 0.0
    {
        commands.trigger(SetSliderValue {
            entity: *slider,
            change: SliderValueChange::RelativeStep(if event.y < 0.0 { 1.0 } else { -1.0 }),
        });
    }
}

pub(super) fn sync(
    slider: Option<Single<(Entity, &SliderValue, &SliderRange), With<PoolScroll>>>,
    party: Option<Res<PartyInventory>>,
    mut commands: Commands,
) {
    let (Some((slider, value, range)), Some(party)) = (slider.as_deref(), party) else { return };
    let max = maximum(party.pool.len()) as f32;
    if range.end() != max {
        commands.entity(*slider).insert(SliderRange::new(0.0, max));
    }
    if value.0 > max {
        commands.entity(*slider).insert(SliderValue(max));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::items::ItemId;
    use crate::game::inventory::{ItemInstance, ItemInstanceId, Quantity};
    use bevy::input::{mouse::MouseScrollUnit, touch::TouchPhase};
    use bevy::picking::{
        backend::HitData,
        events::{PointerCancel, PointerDragStart},
        hover::PointerCaptureMap,
        pointer::PointerButton,
    };
    use bevy::ui_widgets::{Slider, SliderStep, slider_self_update};
    use bevy::ui_widgets::{SliderDragState, SliderPlugin};

    #[test]
    fn pool_scroll_clamps_after_items_are_removed_and_cancels_drag() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, SliderPlugin))
            .init_resource::<PointerCaptureMap>()
            .init_resource::<UiScale>()
            .add_systems(Update, sync);
        let items = (0..11)
            .map(|id| ItemInstance {
                id: ItemInstanceId(id),
                item: ItemId(id as u16),
                quantity: Quantity::One,
                identified: true,
                spell_known: false,
                curse_known: false,
                remarked: false,
            })
            .collect();
        app.insert_resource(PartyInventory { pool: items, ..default() });
        let slider = app
            .world_mut()
            .spawn((PoolScroll, Slider::default(), SliderStep(1.0)))
            .observe(slider_self_update)
            .observe(crate::presentation::retail_controls::cancel_slider_drag)
            .id();
        let cell = app.world_mut().spawn(ItemCell { cell: Cell::Pool(0) }).observe(wheel).id();
        app.update();
        let pointer = crate::presentation::test_input::mouse();
        for _ in 0..2 {
            app.world_mut().trigger(PointerScroll {
                entity: cell,
                pointer: pointer.clone(),
                unit: MouseScrollUnit::Line,
                x: 0.0,
                y: -1.0,
                hit: HitData::new(Entity::PLACEHOLDER, 0.0, None, None),
                phase: TouchPhase::Moved,
            });
            app.update();
        }
        assert_eq!(app.world().get::<SliderValue>(slider).unwrap().0, 2.0);
        let shown = app.world().get::<SliderValue>(slider).unwrap().0 as usize * 2;
        assert_eq!(app.world().resource::<PartyInventory>().pool[shown].id, ItemInstanceId(4));
        app.world_mut().trigger(PointerDragStart {
            entity: slider,
            pointer: pointer.clone(),
            button: PointerButton::Primary,
            hit: HitData::new(Entity::PLACEHOLDER, 0.0, None, None),
        });
        assert!(app.world().get::<SliderDragState>(slider).unwrap().dragging);
        app.world_mut().trigger(PointerCancel {
            entity: slider,
            pointer: pointer.clone(),
            hit: HitData::new(Entity::PLACEHOLDER, 0.0, None, None),
        });
        assert!(!app.world().get::<SliderDragState>(slider).unwrap().dragging);
        assert!(!app.world().resource::<PointerCaptureMap>().0.contains_key(&pointer.id));
        app.world_mut().resource_mut::<PartyInventory>().pool.truncate(8);
        app.update();
        assert_eq!(app.world().get::<SliderValue>(slider).unwrap().0, 0.0);
        assert_eq!(app.world().get::<SliderRange>(slider).unwrap().end(), 0.0);
    }
}
