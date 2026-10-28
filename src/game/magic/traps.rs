//! Indirect trap spells: preflight content, capture real party actors and
//! settle consequences synchronously, before the device may unlock.

use super::{
    LearnedSpells, MagicContext, SpellPoints, Strength, Victim, casting::MagicNews, effects,
};
use crate::content::spells::{SpellDefinition, SpellId};
use crate::game::party::Members;
use crate::game::world::{damage::MonsterDamage, interaction::Unable};
use bevy::{ecs::system::RunSystemOnce, prelude::*};

pub const SPELLS: [SpellId; 15] = [
    SpellId(121),
    SpellId(122),
    SpellId(123),
    SpellId(24),
    SpellId(47),
    SpellId(36),
    SpellId(37),
    SpellId(60),
    SpellId(70),
    SpellId(124),
    SpellId(125),
    SpellId(126),
    SpellId(86),
    SpellId(127),
    SpellId(91),
];

/// The spell a trap type discharges, if the game can resolve it.
fn trap_spell(trap: u8) -> Result<SpellId, Unable> {
    let id = *SPELLS.get(usize::from(trap)).ok_or(Unable::UnsupportedTrap)?;
    if !effects::modelled(id) {
        return Err(Unable::UnsupportedTrap);
    }
    Ok(id)
}

/// No attempted inspection/disarm may roll if its failure cannot be paid.
pub(crate) fn validate(world: &mut World, trap: u8) -> Result<(), Unable> {
    trap_spell(trap)?;
    world
        .run_system_once(
            move |members: Members,
                  monsters: MonsterDamage,
                  points: Query<(&LearnedSpells, &mut SpellPoints)>,
                  context: MagicContext| {
                ready(&context, &members, &monsters, &points, trap)
            },
        )
        .map_err(|_| Unable::MissingContent)?
}

fn ready(
    context: &MagicContext,
    members: &Members,
    monsters: &MonsterDamage,
    points: &Query<(&LearnedSpells, &mut SpellPoints)>,
    trap: u8,
) -> Result<(), Unable> {
    let id = trap_spell(trap)?;
    let spell = context.content.spells().and_then(|s| s.get(id)).ok_or(Unable::MissingContent)?;
    let items = context.content.items().ok_or(Unable::MissingContent)?;
    if spell.id != id || spell.realm.is_none() || !monsters.ready() {
        return Err(Unable::MissingContent);
    }
    for actor in context.roster.iter() {
        let (_, vitals, carried, ..) = members.get(actor).map_err(|_| Unable::MissingContent)?;
        if id == SpellId::ANTI_MAGIC && !vitals.is_dead() && points.get(actor).is_err() {
            return Err(Unable::MissingContent);
        }
        if carried.stacks().any(|stack| items.get(stack.item).is_none()) {
            return Err(Unable::MissingContent);
        }
    }
    if context.inventory.pool.stacks.iter().any(|stack| items.get(stack.item).is_none()) {
        return Err(Unable::MissingContent);
    }
    Ok(())
}

/// Returns whether this discharge destroys the trap. The caller still owns
/// the guard and the trigger; an effect report never substitutes for execution.
pub(crate) fn discharge(
    world: &mut World,
    trap: u8,
    difficulty: u8,
    selected: Option<Entity>,
) -> Result<bool, Unable> {
    trap_spell(trap)?;
    world
        .run_system_once(
            move |(mut members, mut monsters): (Members, MonsterDamage),
                  mut points: Query<(&LearnedSpells, &mut SpellPoints)>,
                  mut context: MagicContext,
                  mut rng: ResMut<crate::game::rng::GameRng>|
                  -> Result<bool, Unable> {
                ready(&context, &members, &monsters, &points, trap)?;
                if selected.is_some_and(|actor| !context.roster.contains(&actor)) {
                    return Err(Unable::NotMember);
                }
                let party: Vec<Entity> = context.roster.iter().collect();
                if party.iter().any(|&actor| members.get(actor).is_err()) {
                    return Err(Unable::MissingContent);
                }
                let mut targets: Vec<Entity> = party
                    .iter()
                    .copied()
                    .filter(|&actor| members.get(actor).is_ok_and(|(_, v, ..)| !v.is_dead()))
                    .collect();
                let (spell, effects, improved, destroyed) = context
                    .resolve(&mut members, &mut monsters, |field, spells| {
                        let spell: &SpellDefinition = spells.get(SPELLS[usize::from(trap)])?;
                        let difficulty = difficulty.clamp(1, 7);
                        let destroyed = rng.below(2) == 0;
                        let mut count =
                            u32::from(difficulty) - rng.below(u32::from(difficulty / 2));
                        let extra = difficulty.saturating_sub(
                            crate::game::world::interaction::TRAP_GRADE[usize::from(trap)] as u8,
                        );
                        let power = (extra + 4).min(7);
                        if extra > 3 {
                            count += rng.below(u32::from(extra - 3));
                        }
                        // Retail deletes random slots from roster order, retaining the
                        // selected living actor. Sleeping/unconscious members can be
                        // victims too.
                        while targets.len() > count as usize {
                            let index = rng.below(targets.len() as u32) as usize;
                            if Some(targets[index]) != selected {
                                targets.remove(index);
                            }
                        }
                        let victims: Vec<_> = targets.into_iter().map(Victim::Member).collect();
                        let effects = effects::resolve(
                            spell,
                            Strength::at(0, spell, power, 0),
                            &victims,
                            &mut rng,
                            field,
                            &mut points,
                        );
                        Some((spell.id, effects, std::mem::take(&mut field.improved), destroyed))
                    })
                    .ok()
                    .flatten()
                    .ok_or(Unable::MissingContent)?;
                context.news.write(MagicNews::Triggered { spell, effects, improved });
                Ok(destroyed)
            },
        )
        .map_err(|_| Unable::MissingContent)?
}
