//! Each caster owns their saved location; return uses ordinary level entry.
use super::{
    Outcome, Utility,
    casting::{MagicNews, Refusal, UseContext},
};
use crate::content::spells::SpellId;
use crate::game::combat::Battle;
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::freedom::Freedom;
use crate::game::hits::effects::Body;
use crate::game::items::Items;
use crate::game::party::body::{PartyBody, PartyMovement, PartyStart};
use crate::game::rng::GameRng;
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

pub(crate) fn spell(id: SpellId) -> bool {
    matches!(id, SpellId::SET_PORTAL | SpellId::RETURN_TO_PORTAL)
}

/// Set Portal saves where the party stands on its caster; Return to Portal
/// takes the party back there, entering the saved level as any travel does
/// when it is another. Where to return is settled before any roll or
/// payment; a failed load later leaves the level being left intact.
#[allow(clippy::type_complexity)]
pub(crate) fn cast(
    mut commands: Commands,
    mut accepted: MessageReader<Utility>,
    mut bodies: Query<Body>,
    portals: Query<&Portal>,
    mut party: Single<(
        &mut PartyBody,
        &mut Position,
        &mut Transform,
        Option<&mut LinearVelocity>,
        &mut PartyMovement,
    )>,
    held: Items,
    mut level: Single<(&Level, &mut LevelState)>,
    (freedom, battle): (Freedom, Query<(), With<Battle>>),
    assets: Res<AssetServer>,
    (content, difficulty): (Res<Catalogs>, Res<Difficulty>),
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<MagicNews>,
) {
    for &Utility { caster, spell: id, choice } in accepted.read().filter(|cast| spell(cast.spell)) {
        let (body, at, transform, velocity, movement) = &mut *party;
        let (level, state) = &mut *level;
        let mut cast = || -> Result<_, Refusal> {
            if !battle.is_empty() || !freedom.unoccupied() {
                return Err(Refusal::WrongTime);
            }
            let saved = match id {
                SpellId::RETURN_TO_PORTAL => {
                    Some(portals.get(caster).cloned().map_err(|_| Refusal::NoTarget)?)
                }
                _ => None,
            };
            let (report, _) = super::casting::pay(
                &mut bodies.get_mut(caster).map_err(|_| Refusal::CannotAct)?,
                &held,
                (choice.source, UseContext::World),
                (&content, *difficulty, state.camera_sway),
                &mut rng,
                &mut commands,
            )?;
            if report.outcome != Outcome::Cast {
                return Ok(report);
            }
            let Some(portal) = saved else {
                let (eye, level) = (PartyBody::eye(at), level.path.clone());
                commands.entity(caster).insert(Portal {
                    level,
                    eye,
                    yaw: body.yaw,
                    pitch: body.pitch,
                });
                return Ok(report);
            };
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
            Ok(report)
        };
        news.write(match cast() {
            Ok(report) => MagicNews::Done(report),
            Err(refusal) => MagicNews::Refused { caster, refusal },
        });
    }
}
