//! Indirect trap spells: preflight content, capture real party actors and
//! settle consequences synchronously, before the device may unlock.

use super::{
    Field, LearnedSpells, MagicContext, Members, SpellPoints, Strength, Victim, casting::MagicNews,
    effects,
};
use crate::content::spells::{SpellDefinition, SpellId};
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

/// No attempted inspection/disarm may roll if its failure cannot be paid.
pub(crate) fn validate(world: &mut World, trap: u8) -> Result<(), Unable> {
    let id = *SPELLS.get(usize::from(trap)).ok_or(Unable::UnsupportedTrap)?;
    if !effects::modelled(id) {
        return Err(Unable::UnsupportedTrap);
    }
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
    let id = *SPELLS.get(usize::from(trap)).ok_or(Unable::UnsupportedTrap)?;
    if !effects::modelled(id) {
        return Err(Unable::UnsupportedTrap);
    }
    let data = context.data.as_ref().ok_or(Unable::MissingContent)?;
    let spell =
        context.spells.get(&data.spells).and_then(|s| s.get(id)).ok_or(Unable::MissingContent)?;
    let items = context.items.get(&data.items).ok_or(Unable::MissingContent)?;
    if spell.id != id || spell.realm.is_none() || !monsters.ready() {
        return Err(Unable::MissingContent);
    }
    for actor in context.roster.iter() {
        let (_, vitals, carried) = members.get(actor).map_err(|_| Unable::MissingContent)?;
        if id == SpellId(127) && !vitals.is_dead() && points.get(actor).is_err() {
            return Err(Unable::MissingContent);
        }
        if carried.stacks().any(|stack| items.get(stack.item).is_none()) {
            return Err(Unable::MissingContent);
        }
    }
    if monsters.inventory.pool.stacks.iter().any(|stack| items.get(stack.item).is_none()) {
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
    world
        .run_system_once(
            move |(mut members, mut monsters): (Members, MonsterDamage),
                  mut points: Query<(&LearnedSpells, &mut SpellPoints)>,
                  mut context: MagicContext|
                  -> Result<bool, Unable> {
                ready(&context, &members, &monsters, &points, trap)?;
                let party_level = context.party_level(&members);
                let MagicContext {
                    roster,
                    data,
                    items,
                    species,
                    spells,
                    difficulty: setting,
                    formation,
                    party_damage,
                    rng,
                    news,
                    ..
                } = &mut context;
                let data = data.as_ref().ok_or(Unable::MissingContent)?;
                let (Some(items), Some(species), Some(spells)) =
                    (items.get(&data.items), species.get(&data.species), spells.get(&data.spells))
                else {
                    return Err(Unable::MissingContent);
                };
                let spell: &SpellDefinition =
                    spells.get(SPELLS[usize::from(trap)]).ok_or(Unable::MissingContent)?;
                // Validate all effect dependencies before the first consequence roll.
                if !monsters.ready() {
                    return Err(Unable::MissingContent);
                }
                let party: Vec<Entity> = roster.iter().collect();
                if selected.is_some_and(|actor| !roster.contains(&actor)) {
                    return Err(Unable::NotMember);
                }
                if party.iter().any(|&actor| members.get(actor).is_err()) {
                    return Err(Unable::MissingContent);
                }
                let mut targets: Vec<Entity> = party
                    .iter()
                    .copied()
                    .filter(|&actor| members.get(actor).is_ok_and(|(_, v, _)| !v.is_dead()))
                    .collect();
                let difficulty = difficulty.clamp(1, 7);
                let destroyed = rng.below(2) == 0;
                let mut count = u32::from(difficulty) - rng.below(u32::from(difficulty / 2));
                let extra = difficulty.saturating_sub(
                    crate::game::world::interaction::TRAP_GRADE[usize::from(trap)] as u8,
                );
                let power = (extra + 4).min(7);
                if extra > 3 {
                    count += rng.below(u32::from(extra - 3));
                }
                // Retail deletes random slots from roster order, retaining the selected
                // living actor. Sleeping/unconscious members can be victims too.
                while targets.len() > count as usize {
                    let index = rng.below(targets.len() as u32) as usize;
                    if Some(targets[index]) != selected {
                        targets.remove(index);
                    }
                }
                let victims: Vec<_> = targets.into_iter().map(Victim::Member).collect();
                let mut field = Field {
                    members: &mut members,
                    monsters: &mut monsters,
                    formation,
                    party_damage,
                    items,
                    species,
                    party_level,
                    difficulty: **setting,
                    party: &party,
                    improved: Vec::new(),
                };
                let effects = effects::resolve(
                    spell,
                    Strength::at(0, spell, power, 0),
                    &victims,
                    rng,
                    &mut field,
                    (spells, &mut points),
                );
                news.write(MagicNews::Triggered {
                    spell: spell.id,
                    effects,
                    improved: field.improved,
                });
                Ok(destroyed)
            },
        )
        .map_err(|_| Unable::MissingContent)?
}
