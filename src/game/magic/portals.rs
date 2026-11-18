//! Each caster owns their saved location; return uses ordinary level entry.
use super::Outcome;
use super::casting::{Allowed, CastBy, MagicNews, Worked, marked};
use super::may::Refusal;
use super::reach::Marked;
use crate::game::party::body::{PartyBody, PartyMovement, PartyStart};
use crate::game::world::level::InPlay;
use crate::game::world::level::{Destination, Entry, Level};
use crate::game::world::triggers::LevelState;
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

/// Set Portal is cast at nothing: where the party stands.
pub fn mark_here(allowed: On<Allowed>, casts: Query<&CastBy>, mut commands: Commands) -> Result {
    let &CastBy(caster) = casts.get(allowed.cast)?;
    marked(&mut commands, (allowed.cast, caster), Ok(Marked::Nothing));
    Ok(())
}

/// Return to Portal needs a portal its caster saved.
pub fn mark_saved(
    allowed: On<Allowed>,
    casts: Query<&CastBy>,
    portals: Query<(), With<Portal>>,
    mut commands: Commands,
) -> Result {
    let &CastBy(caster) = casts.get(allowed.cast)?;
    let saved = portals.contains(caster).then_some(Marked::Nothing).ok_or(Refusal::NoTarget);
    marked(&mut commands, (allowed.cast, caster), saved);
    Ok(())
}

/// Set Portal saves where the party stands on its caster, once it comes
/// off; a fizzle or backfire does nothing.
pub fn set(
    worked: On<Worked>,
    mut commands: Commands,
    party: Single<(&PartyBody, &Position)>,
    level: Single<&Level, With<InPlay>>,
    mut news: MessageWriter<MagicNews>,
) {
    let Worked { caster, report, .. } = worked.event().clone();
    let outcome = report.outcome;
    news.write(MagicNews::Done(report));
    if outcome != Outcome::Cast {
        return;
    }
    let (body, at) = *party;
    let (eye, level) = (PartyBody::eye(at), level.path.clone());
    commands.entity(caster).insert(Portal { level, eye, yaw: body.yaw, pitch: body.pitch });
}

/// Return to Portal takes the party back to its caster's portal, entering
/// the saved level as any travel does when it is another; a fizzle or
/// backfire does nothing.
#[allow(clippy::type_complexity)]
pub fn go_back(
    worked: On<Worked>,
    mut commands: Commands,
    portals: Query<&Portal>,
    mut party: Single<(
        &mut PartyBody,
        &mut Position,
        &mut Transform,
        Option<&mut LinearVelocity>,
        &mut PartyMovement,
    )>,
    mut level: Single<(&Level, &mut LevelState), With<InPlay>>,
    assets: Res<AssetServer>,
    mut news: MessageWriter<MagicNews>,
) -> Result {
    let Worked { caster, report, .. } = worked.event().clone();
    let outcome = report.outcome;
    news.write(MagicNews::Done(report));
    if outcome != Outcome::Cast {
        return Ok(());
    }
    let (body, at, transform, velocity, movement) = &mut *party;
    let (level, state) = &mut *level;
    let portal = portals.get(caster)?.clone();
    if portal.level != level.path {
        commands.spawn(Destination::new(
            Level::load(&portal.level, &assets),
            PartyStart {
                eye: Some(portal.eye),
                yaw: Some(portal.yaw),
                pitch: portal.pitch,
                ..default()
            },
            Entry::Travel,
        ));
    } else {
        body.stand(at, transform, portal.eye, portal.yaw);
        body.pitch = portal.pitch;
        if let Some(velocity) = velocity {
            velocity.0 = Vec3::ZERO;
        }
        state.arrive(portal.eye);
    }
    **movement = default();
    Ok(())
}
