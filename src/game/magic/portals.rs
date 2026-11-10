//! Each caster owns their saved location; return uses ordinary level entry.
use super::Outcome;
use super::casting::{MagicNews, Worked};
use crate::content::spells::Working;
use crate::game::party::body::{PartyBody, PartyMovement, PartyStart};
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

/// Set Portal saves where the party stands on its caster; Return to Portal
/// takes the party back there, entering the saved level as any travel does
/// when it is another. The cast is paid for and rolled as any other
/// ([`super::casting`]), its portal settled before; a fizzle or backfire does
/// nothing.
#[allow(clippy::type_complexity)]
pub(crate) fn cast(
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
    mut level: Single<(&Level, &mut LevelState)>,
    assets: Res<AssetServer>,
    mut news: MessageWriter<MagicNews>,
) {
    let Worked { caster, working, report, .. } = worked.event().clone();
    if !matches!(working, Working::SetPortal | Working::ReturnToPortal) {
        return;
    }
    let outcome = report.outcome;
    news.write(MagicNews::Done(report));
    if outcome != Outcome::Cast {
        return;
    }
    let (body, at, transform, velocity, movement) = &mut *party;
    let (level, state) = &mut *level;
    if working == Working::SetPortal {
        let (eye, level) = (PartyBody::eye(at), level.path.clone());
        commands.entity(caster).insert(Portal { level, eye, yaw: body.yaw, pitch: body.pitch });
        return;
    }
    let Ok(portal) = portals.get(caster).cloned() else { return };
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
}
