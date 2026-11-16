//! A fountain owns its supply. Drinking releases a deed; healing and cleansing
//! are ordinary effects, settled by the same rules as spells.

use bevy::prelude::*;

use crate::content::level::{Fountain, Restores};
use crate::content::spells::SpellId;
use crate::game::characters::Vitals;
use crate::game::data::Catalogs;
use crate::game::hits::Deed;
use crate::game::hits::definitions::{EffectKind, EffectSpec, Roll, Rows};
use crate::game::hits::effects::Strike;
use crate::game::magic::Strength;
use crate::game::party::roster::PartyRoster;
use crate::game::time::GameClock;

use super::triggers::{Act, Committed, TriggerEffect};

/// What a fountain has left to give, and where it stands.
#[derive(Component, Debug)]
pub(crate) struct Supply {
    at: Option<Vec3>,
    used: Option<Uses>,
}

impl Supply {
    pub(crate) fn at(at: Option<Vec3>) -> Self {
        Self { at, used: None }
    }
}

#[derive(Debug)]
struct Uses {
    left: Option<u32>,
    refills_at: Option<u64>,
}

/// What a fountain gives, as one of its device's uses: the party drinks,
/// unless it has run dry.
pub(super) fn drink(
    act: On<Act<Fountain>>,
    mut fountains: Query<&mut Supply>,
    clock: Res<GameClock>,
    roster: Single<&PartyRoster>,
    bodies: Query<&Vitals>,
    content: Res<Catalogs>,
    mut outcomes: MessageWriter<TriggerEffect>,
    mut commands: Commands,
) {
    let (water, doing) = (act.what, act.doing);
    let Ok(mut supply) = fountains.get_mut(act.device) else { return };
    let now = clock.ms;
    if supply
        .used
        .as_ref()
        .is_some_and(|s| s.left == Some(0) && !s.refills_at.is_some_and(|at| at <= now))
    {
        return doing.decline(&mut outcomes);
    }
    let used = supply.used.get_or_insert(Uses { left: water.uses, refills_at: None });
    if used.refills_at.is_some_and(|at| at <= now) {
        *used = Uses { left: water.uses, refills_at: None };
    }
    if let Some(left) = &mut used.left {
        if *left == 0 {
            return doing.decline(&mut outcomes);
        }
        *left -= 1;
        if used.refills_at.is_none() {
            used.refills_at = water.refills_after.map(|after| now + after);
        }
    }
    let strike = match water.restores {
        Restores::Restoration => {
            // PointCastSpell: zero does not cast; authored powers above seven use one.
            let power = if water.amount > 7 { 1 } else { water.amount };
            let mut strike = Strike::new(
                Rows::Spell(SpellId::HEAL_ALL),
                Strength { power, ..default() },
                None,
                None,
                &content,
            );
            let mut effects = if power > 0 { strike.effects.to_vec() } else { Vec::new() };
            // Trigger::RunAction 0x2b heals, then removes conditions except infatuation.
            effects.push(EffectSpec { kind: EffectKind::Cleanse, ..default() });
            strike.effects = effects.into();
            strike
        }
        restores => {
            let kind = match restores {
                Restores::Health => EffectKind::Heal,
                Restores::Stamina => EffectKind::Refresh,
                Restores::Magic => EffectKind::Replenish,
                Restores::Restoration => unreachable!(),
            };
            Strike {
                effects: vec![EffectSpec {
                    kind,
                    roll: Some(Roll::Fixed(water.amount)),
                    ..default()
                }]
                .into(),
                spell: None,
                strength: Strength::default(),
                blow: None,
                by: None,
            }
        }
    };
    commands.spawn(Deed::at(
        strike,
        roster.iter().filter(|&entity| bodies.get(entity).is_ok_and(|body| !body.is_dead())),
    ));
    outcomes.write(TriggerEffect::FountainUsed { at: supply.at });
    commands.trigger(Committed { device: act.device, doing });
}
