//! Each caster owns their saved location; return uses ordinary level entry.
use super::{
    Outcome,
    casting::{MagicNews, Refusal, Source, UseContext},
};
use crate::content::spells::SpellId;
use crate::game::party::body::{PartyBody, PartyMovement, PartyStart};
use crate::game::world::level::{Destination, Entry, Level};
use avian3d::prelude::{LinearVelocity, Position};
use bevy::prelude::*;

/// A historical location, not another authority for the party's current pose.
/// Remains on the caster across travel and recasting replaces it.
#[derive(Component, Clone, Debug)]
pub struct Portal {
    pub level: String,
    pub eye: Vec3,
    pub yaw: f32,
    pub pitch: f32,
}

pub(crate) fn spell(id: SpellId) -> bool {
    matches!(id, SpellId::SET_PORTAL | SpellId::RETURN_TO_PORTAL)
}

pub(crate) fn cast(world: &mut World, caster: Entity, source: Source) -> Result<(), Refusal> {
    if world.query::<&crate::game::combat::Battle>().iter(world).next().is_some() {
        return Err(Refusal::WrongTime);
    }
    if !world.run_system_cached(crate::game::freedom::unoccupied).unwrap_or(false) {
        return Err(Refusal::WrongTime);
    }
    let (level, path) = world
        .query::<(Entity, &Level)>()
        .single(world)
        .map(|(e, l)| (e, l.path.clone()))
        .map_err(|_| Refusal::WrongTime)?;
    let (party, eye, yaw, pitch) = world
        .query::<(Entity, &PartyBody, &Position)>()
        .single(world)
        .map(|(e, body, at)| (e, PartyBody::eye(at), body.yaw, body.pitch))
        .map_err(|_| Refusal::WrongTime)?;
    let id = super::utilities::source_spell(world, caster, source)?;
    if !spell(id) {
        return Err(Refusal::Unsupported);
    }
    let saved = if id == SpellId::RETURN_TO_PORTAL {
        Some(world.get::<Portal>(caster).cloned().ok_or(Refusal::NoTarget)?)
    } else {
        None
    };
    // Prepare mandatory destination access before any roll or payment. Loading
    // failure later leaves the source level intact through normal entry policy.
    let destination = match &saved {
        Some(portal) if portal.level != path => {
            let assets = world.get_resource::<AssetServer>().ok_or(Refusal::MissingContent)?;
            Some(Destination::new(
                Level::load(&portal.level, assets),
                PartyStart {
                    eye: Some(portal.eye),
                    yaw: Some(portal.yaw),
                    pitch: portal.pitch,
                    ..default()
                },
                Entry::Travel,
            ))
        }
        _ => None,
    };
    let (report, _) = super::sources::cast_source(world, caster, source, UseContext::World)?;
    if report.outcome == Outcome::Cast {
        match saved {
            None => {
                world.entity_mut(caster).insert(Portal { level: path, eye, yaw, pitch });
            }
            Some(portal) => {
                if let Some(destination) = destination {
                    world.spawn(destination);
                } else {
                    let mut actor = world.entity_mut(party);
                    actor.get_mut::<Position>().unwrap().0 = PartyBody::centre(portal.eye);
                    if let Some(mut transform) = actor.get_mut::<Transform>() {
                        transform.translation = PartyBody::centre(portal.eye);
                    }
                    let mut body = actor.get_mut::<PartyBody>().unwrap();
                    body.yaw = portal.yaw;
                    body.pitch = portal.pitch;
                    if let Some(mut velocity) = actor.get_mut::<LinearVelocity>() {
                        velocity.0 = Vec3::ZERO;
                    }
                    if let Some(mut state) =
                        world.get_mut::<crate::game::world::triggers::LevelState>(level)
                    {
                        state.arrive(portal.eye);
                    }
                }
                if let Some(mut movement) = world.get_mut::<PartyMovement>(party) {
                    *movement = default();
                }
            }
        }
    }
    world.write_message(MagicNews::Done(report));
    Ok(())
}
