//! Acting on a click in the view (retail `MainGameScreen`'s view click):
//! use the prop under the pointer; else, with an item in hand, drop it
//! along the pointer's ray; else take the item under the pointer.

use avian3d::prelude::{RigidBody, SpatialQuery};
use bevy::prelude::*;

use super::items::{WorldItem, drop_held, take_item};
use super::level::Level;
use super::props::Prop;
use super::triggers::{LevelTriggers, TriggerEffect};
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

pub fn act_on_click(
    mut commands: Commands,
    mut clicks: MessageReader<WorldClick>,
    props: Query<(&Name, &Prop)>,
    // Triggers give the party things too: one or the other at a time.
    mut acting: ParamSet<(LevelTriggers, ResMut<PartyInventory>)>,
    mut items: Query<&mut WorldItem>,
    (levels, bodies, spatial): (Query<Entity, With<Level>>, Query<&RigidBody>, SpatialQuery),
    (data, catalogs, mut news): (
        Option<Res<GameData>>,
        Res<Assets<ItemCatalog>>,
        MessageWriter<ItemNews>,
    ),
) {
    let Some(catalog) = data.and_then(|d| catalogs.get(&d.items)) else { return };
    for click in clicks.read() {
        if let Some((name, prop, distance)) =
            click.under.and_then(|(e, d)| props.get(e).ok().map(|(n, p)| (n, p, d)))
        {
            use_prop(name, prop, distance, &mut acting.p0());
            continue;
        }
        let mut inventory = acting.p1();
        if inventory.held.is_some() {
            let Ok(level) = levels.single() else { continue };
            let solid = |entity| bodies.get(entity).is_ok_and(RigidBody::is_static);
            drop_held(&mut commands, level, click.ray, &spatial, &solid, &mut inventory);
        } else if let Some((entity, distance)) = click.under
            && let Ok(mut item) = items.get_mut(entity)
        {
            let tell = &mut |n| {
                news.write(n);
            };
            if take_item(&mut item, distance, &mut inventory, catalog, tell) {
                commands.entity(entity).despawn();
            }
        }
    }
}

/// Use a prop clicked `distance` away, if its trigger can be used from
/// there. Retail measures from the middle of the prop; the hit point is
/// close.
fn use_prop(name: &Name, prop: &Prop, distance: f32, triggers: &mut LevelTriggers) {
    let Some(trigger) = &prop.trigger else {
        debug!("{name} does nothing");
        return;
    };
    triggers.with(|triggers| {
        let Some(index) = triggers.state.find(trigger) else { return };
        let (near, far) = triggers.definitions[index].range;
        if !triggers.usable(index) {
            debug!("{name} ({trigger}) cannot be used");
        } else if !(near..=far).contains(&distance) {
            debug!("{name} ({trigger}) is out of reach: {distance:.1} m, not {near:.1}..{far:.1}");
        } else if triggers.run(trigger) {
            info!("used {name} ({trigger})");
        } else {
            info!("used {name} ({trigger}): nothing happened");
            triggers.effects.push(TriggerEffect::NothingHappened);
        }
    });
}
