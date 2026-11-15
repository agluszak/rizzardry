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

use super::triggers::TriggerEffect;

#[derive(Component, Debug, Default)]
pub(super) struct Supply {
    used: Option<Uses>,
}

#[derive(Debug)]
struct Uses {
    left: Option<u32>,
    refills_at: Option<u64>,
}

/// A selected fountain action. Its owner answers whether a drink was available.
#[derive(EntityEvent)]
pub(super) struct Drink {
    #[event_target]
    pub fountain: Entity,
    pub water: Fountain,
    pub at: Option<Vec3>,
    pub consumed: bool,
}

pub(super) fn drink(
    mut request: On<Drink>,
    mut fountains: Query<&mut Supply>,
    clock: Res<GameClock>,
    roster: Single<&PartyRoster>,
    bodies: Query<&Vitals>,
    content: Res<Catalogs>,
    mut outcomes: MessageWriter<TriggerEffect>,
    mut commands: Commands,
) {
    let water = request.water;
    let mut supply = fountains.get_mut(request.fountain).unwrap();
    let now = clock.ms;
    if supply
        .used
        .as_ref()
        .is_some_and(|s| s.left == Some(0) && !s.refills_at.is_some_and(|at| at <= now))
    {
        return;
    }
    let used = supply.used.get_or_insert(Uses { left: water.uses, refills_at: None });
    if used.refills_at.is_some_and(|at| at <= now) {
        *used = Uses { left: water.uses, refills_at: None };
    }
    if let Some(left) = &mut used.left {
        if *left == 0 {
            return;
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
            }
        }
    };
    commands.spawn(Deed::at(
        strike,
        roster.iter().filter(|&entity| bodies.get(entity).is_ok_and(|body| !body.is_dead())),
    ));
    request.consumed = true;
    outcomes.write(TriggerEffect::FountainUsed { at: request.at });
}
