//! Acting on a click in the view (retail `MainGameScreen`'s view click):
//! use the prop under the pointer; else, with an item in hand, drop it
//! along the pointer's ray; else take the item under the pointer.

use avian3d::prelude::{RigidBody, SpatialQuery};
use bevy::prelude::*;

use super::items::{WorldItem, drop_held, take_item};
use super::level::Level;
use super::props::Prop;
use super::triggers::{self, Device};
use crate::content::items::ItemCatalog;
use crate::game::data::GameData;
use crate::game::inventory::{ItemNews, PartyInventory};

/// A click in the view: the pointer's ray, and the prop or lying item
/// drawn first under it with its distance along the ray.
#[derive(Message, Debug, Clone, Copy)]
pub struct WorldClick {
    pub ray: Ray3d,
    pub under: Option<(Entity, f32)>,
}

pub fn act_on_click(mut commands: Commands, mut clicks: MessageReader<WorldClick>) {
    for &click in clicks.read() {
        commands.queue(move |world: &mut World| {
            if let Some((entity, distance)) = click.under
                && world.get::<Prop>(entity).is_some()
                && world.get::<Name>(entity).is_some()
            {
                if world.get::<Device>(entity).is_some()
                    && triggers::activate(world, entity, Some(distance))
                {
                    info!("used {}", world.get::<Name>(entity).unwrap());
                }
            } else {
                world.run_system_cached_with(act_on_item, click).unwrap();
            }
        });
    }
}

fn act_on_item(
    In(click): In<WorldClick>,
    mut commands: Commands,
    mut inventory: ResMut<PartyInventory>,
    mut items: Query<&mut WorldItem>,
    (levels, bodies, spatial): (Query<Entity, With<Level>>, Query<&RigidBody>, SpatialQuery),
    (data, catalogs, mut news): (
        Option<Res<GameData>>,
        Res<Assets<ItemCatalog>>,
        MessageWriter<ItemNews>,
    ),
) {
    if inventory.held.is_some() {
        let Ok(level) = levels.single() else { return };
        let solid = |entity| bodies.get(entity).is_ok_and(RigidBody::is_static);
        drop_held(&mut commands, level, click.ray, &spatial, &solid, &mut inventory);
    } else if let Some((entity, distance)) = click.under
        && let Ok(mut item) = items.get_mut(entity)
    {
        let tell = &mut |n| {
            news.write(n);
        };
        let Some(catalog) = data.as_ref().and_then(|d| catalogs.get(&d.items)) else {
            return;
        };
        if take_item(&mut item, distance, &mut inventory, catalog, tell) {
            commands.entity(entity).despawn();
        }
    }
}
