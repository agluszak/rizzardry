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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::items::ItemId;
    use crate::game::inventory::{ItemIds, ItemPool, ItemStack, StackId};
    use bevy::input::{mouse::MouseScrollUnit, touch::TouchPhase};
    use bevy::picking::{
        backend::HitData,
        events::{PointerCancel, PointerDragStart, PointerScroll},
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
        app.world_mut().register_component::<Window>();
        let mut ids = ItemIds::default();
        let items = (0..11)
            .map(|id| {
                ItemStack::new(
                    &crate::content::items::ItemDefinition { id: ItemId(id), ..default() },
                    &mut ids,
                    1,
                    true,
                )
            })
            .collect();
        let party = app
            .world_mut()
            .spawn((
                crate::game::party::roster::Party,
                PartyInventory { pool: ItemPool { stacks: items }, ..default() },
            ))
            .id();
        let slider = app
            .world_mut()
            .spawn((PoolScroll, Slider::default(), SliderStep(1.0)))
            .observe(slider_self_update)
            .observe(crate::presentation::retail_controls::cancel_slider_drag)
            .id();
        let region = app
            .world_mut()
            .spawn(crate::presentation::retail_controls::ScrollWheel(slider))
            .observe(crate::presentation::retail_controls::scroll_wheel)
            .id();
        let cell = app.world_mut().spawn(ChildOf(region)).id();
        let backpack_cell = app.world_mut().spawn_empty().id();
        app.update();
        let pointer = crate::presentation::test_input::mouse();
        app.world_mut().trigger(PointerScroll {
            entity: backpack_cell,
            pointer: pointer.clone(),
            unit: MouseScrollUnit::Line,
            x: 0.0,
            y: -1.0,
            hit: HitData::new(Entity::PLACEHOLDER, 0.0, None, None),
            phase: TouchPhase::Moved,
        });
        app.update();
        assert_eq!(app.world().get::<SliderValue>(slider).unwrap().0, 0.0);
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
        assert_eq!(
            app.world().get::<PartyInventory>(party).unwrap().pool.stacks[shown].id,
            StackId(5)
        );
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
        app.world_mut().get_mut::<PartyInventory>(party).unwrap().pool.stacks.truncate(8);
        app.update();
        assert_eq!(app.world().get::<SliderValue>(slider).unwrap().0, 0.0);
        assert_eq!(app.world().get::<SliderRange>(slider).unwrap().end(), 0.0);
    }
}
