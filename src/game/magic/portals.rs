//! Each caster owns their saved location; return uses ordinary level entry.
use super::{
    Outcome,
    casting::{MagicNews, Refusal, UseContext},
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

/// Set Portal saves where the party stands to the caster; Return to Portal
/// takes the party back there, entering its level when it is another. The
/// destination is prepared before any roll or payment; a load failure later
/// leaves the source level intact through normal entry policy.
pub(crate) fn cast_portals(
    mut casts: MessageReader<super::utilities::UtilityCast>,
    freedom: crate::game::freedom::Freedom,
    battles: Query<(), With<crate::game::combat::Battle>>,
    level: Single<(Entity, &Level)>,
    mut party: Single<(
        &mut PartyBody,
        &mut Position,
        &mut Transform,
        Option<&mut LinearVelocity>,
        &mut PartyMovement,
    )>,
    portals: Query<&Portal>,
    assets: Res<AssetServer>,
    mut casting: ParamSet<(
        super::casting::Payment,
        Query<&mut crate::game::world::triggers::LevelState>,
    )>,
    mut news: MessageWriter<MagicNews>,
    mut commands: Commands,
) {
    let (root, level) = *level;
    for &super::utilities::UtilityCast { caster, choice, spell } in casts.read() {
        if !self::spell(spell) {
            continue;
        }
        if !battles.is_empty() || !freedom.unoccupied() {
            news.write(MagicNews::Refused { caster, refusal: Refusal::WrongTime });
            continue;
        }
        let saved = if spell == SpellId::RETURN_TO_PORTAL {
            match portals.get(caster) {
                Ok(portal) => Some(portal.clone()),
                Err(_) => {
                    news.write(MagicNews::Refused { caster, refusal: Refusal::NoTarget });
                    continue;
                }
            }
        } else {
            None
        };
        let destination =
            saved.as_ref().filter(|portal| portal.level != level.path).map(|portal| {
                Destination::new(
                    Level::load(&portal.level, &assets),
                    PartyStart {
                        eye: Some(portal.eye),
                        yaw: Some(portal.yaw),
                        pitch: portal.pitch,
                        ..default()
                    },
                    Entry::Travel,
                )
            });
        let report = match casting.p0().pay(caster, choice.source, UseContext::World) {
            Ok((report, _)) => report,
            Err(refusal) => {
                news.write(MagicNews::Refused { caster, refusal });
                continue;
            }
        };
        if report.outcome == Outcome::Cast {
            let (body, at, transform, velocity, movement) = &mut *party;
            match saved {
                None => {
                    commands.entity(caster).insert(Portal {
                        level: level.path.clone(),
                        eye: PartyBody::eye(at),
                        yaw: body.yaw,
                        pitch: body.pitch,
                    });
                }
                Some(portal) => {
                    if let Some(destination) = destination {
                        commands.spawn(destination);
                    } else {
                        body.stand(at, transform, portal.eye, portal.yaw);
                        body.pitch = portal.pitch;
                        if let Some(velocity) = velocity {
                            velocity.0 = Vec3::ZERO;
                        }
                        casting.p1().get_mut(root).unwrap().arrive(portal.eye);
                    }
                    **movement = default();
                }
            }
        }
        news.write(MagicNews::Done(report));
    }
}
