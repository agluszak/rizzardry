//! An impact's immediate sequence is a local loop. Only impacts waiting for
//! flight and their owning acts persist in the world between simulation steps.
use super::definitions::{self, BlowDamage, Definitions, EffectKind, Uses, WeaponEffects};
use super::effects::{self, InFlight, Outcome};
use super::{HitOf, Strikes};
use crate::game::magic::Strength;
use crate::game::party::roster::{Party, PartyMember};
use crate::game::rng::GameRng;
use bevy::ecs::entity::EntityHashMap;
use bevy::prelude::*;

pub fn impacts(world: &mut World) {
    let ready: Vec<_> = world
        .query_filtered::<(Entity, &HitOf, &Strikes), Without<InFlight>>()
        .iter(world)
        .map(|(id, act, target)| (id, act.0, target.0))
        .collect();
    for (impact, act, victim) in ready {
        let Some(uses) = world.get::<Uses>(act) else {
            world.despawn(impact);
            continue;
        };
        let sequence = world.resource::<Definitions>().get(uses.0).effects.clone();
        let mut previous = None;
        for spec in sequence {
            // Every consequence sees mutations made by its predecessor.
            world.run_schedule(super::Capabilities);
            let kind = world.get::<super::defences::Kind>(victim).map(|k| k.0);
            let member = world.get::<PartyMember>(victim).is_some();
            let party = world.get::<Party>(victim).is_some();
            let weapon = world.get::<WeaponEffects>(act);
            if (spec.members && !member)
                || (spec.monsters && kind.is_none())
                || spec.kinds.is_some_and(|kinds| !kind.is_some_and(|k| kinds.contains(&k)))
                || (spec.by_monsters && !weapon.is_some_and(|w| w.by_monster))
                || spec.party != party
            {
                continue;
            }
            let chance = spec.weapon.map(|effect| weapon.map_or(0, |w| w.effects.chances[effect]));
            if chance == Some(0) {
                continue;
            }
            if (spec.landed && previous != Some(Outcome::Landed))
                || (spec.resisted && previous != Some(Outcome::Resisted))
            {
                previous = Some(Outcome::Missed);
                continue;
            }
            let strength = world.get::<Strength>(act).copied().unwrap_or_default();
            let damage = world.get::<BlowDamage>(act).map_or(0, |d| d.0);
            let input = world.resource_scope(|world, mut rng: Mut<GameRng>| {
                let weapon = world.get::<WeaponEffects>(act);
                let mut spec = spec.pick(&mut rng);
                if let Some(chance) = chance {
                    spec.chance = Some(effects::Chance(chance));
                    if let EffectKind::Afflict(ref mut afflict) = spec.kind
                        && afflict.condition == crate::game::characters::Condition::Poisoned
                    {
                        afflict.poison = weapon.map_or(0, |w| w.effects.strength);
                    }
                }
                let magnitude = spec
                    .roll
                    .map_or(0, |roll| definitions::roll(roll, strength, weapon, damage, &mut rng));
                effects::EffectInput { act, victim, strength, magnitude, spec }
            });
            let (input, stopped) = world
                .run_system_cached_with(effects::defend, input)
                .expect("effect defences available");
            previous = Some(if let Some(outcome) = stopped {
                outcome
            } else {
                let harm = matches!(input.spec.kind, EffectKind::Harm);
                let applied = effects::apply(world, input);
                if let Some(amount) = applied.dealt {
                    world
                        .run_system_cached_with(effects::hurt, (act, victim, amount, harm))
                        .expect("damage consequences available");
                }
                applied.outcome
            });
            world.run_schedule(super::AfterEffect);
        }
        world.despawn(impact);
    }
}

/// A released act reaches each affected party once, independently of its
/// body reaches. The party's sequence includes every party effect and
/// starts when the first of its members is reached. Added<Uses> is the act's
/// release boundary; finishing a reach never makes another one.
pub fn party_reaches(
    mut commands: Commands,
    acts: Query<(Entity, &Uses, &super::DeedHits), Added<Uses>>,
    definitions: Res<Definitions>,
    reaches: Query<(&Strikes, Option<&InFlight>), With<HitOf>>,
    members: Query<&PartyMember>,
) {
    for (act, uses, hits) in &acts {
        let definition = definitions.get(uses.0);
        if !definition.effects.iter().any(|effect| effect.party) {
            continue;
        }
        let mut parties = EntityHashMap::<f32>::default();
        for (strikes, flight) in reaches.iter_many(hits.iter()).matched() {
            let Ok(member) = members.get(strikes.0) else { continue };
            let seconds = flight.map_or(0.0, |flight| flight.0.remaining_secs());
            parties
                .entry(member.0)
                .and_modify(|first| *first = first.min(seconds))
                .or_insert(seconds);
        }
        for (party, seconds) in parties {
            let mut reach = commands.spawn((HitOf(act), Strikes(party)));
            if seconds > 0.0 {
                reach.insert(InFlight(Timer::from_seconds(seconds, TimerMode::Once)));
            }
        }
    }
}
