//! What mends: cures, purification, healing and restoring points, and
//! enchantments laid on the party and its members.
use super::*;

/// Whether a spell restores what is missing, so that "auto" power and
/// repeating aim at what the victim lacks.
pub(in crate::game::magic) fn restores(spell: SpellId) -> bool {
    matches!(kind(spell), Some(Kind::Heal | Kind::Stamina | Kind::Restoration))
}

/// The rounds a cure takes off a timed condition: `power²`, boosted.
pub(super) fn cure_rounds(strength: Strength) -> u32 {
    let per = u32::from(strength.power).pow(2);
    per + per * strength.boost / 100
}

/// Retail `TryCureConditionOnTargets`: a timed condition loses `power²`
/// rounds (and goes if that is all of them); a lasting one goes with chance
/// `power·15`%.
pub(super) fn cure(
    rng: &mut GameRng,
    conditions: &mut Conditions,
    cured: &[Condition],
    strength: Strength,
    forced: bool,
) -> Vec<Condition> {
    let per = cure_rounds(strength);
    let mut lifted = Vec::new();
    for &condition in cured {
        let Some(lasting) = conditions.lasting_mut(condition) else { continue };
        let gone = match lasting {
            _ if forced => true,
            Lasting::Rounds(left) if u32::from(*left) <= per => true,
            Lasting::Rounds(left) => {
                *left -= per as u16;
                false
            }
            Lasting::Indefinite => rng.below(100) < u32::from(strength.power) * 15,
        };
        if gone {
            conditions.lift(condition);
            lifted.push(condition);
        }
    }
    lifted
}

/// Shorten the offensive slots on the party and the monsters hit, by more
/// the stronger Purify is than each cloud.
pub(super) fn purify(
    spell: &SpellDefinition,
    strength: Strength,
    victims: &[Victim],
    rng: &mut GameRng,
    world: &mut Field,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    let catalog = world.spells();
    // Purify rolls from power alone, rather than its authored duration.
    let base = u32::from(strength.power) + 1;
    let amount = match rng.below(4) {
        0 if base > 1 => base + 1,
        1 if base < 3 => base + 1,
        _ => base,
    };
    let amount = amount + amount * strength.boost / 100;
    let mut party_done = false;
    for &victim in victims {
        let purify = |slots: &mut crate::game::magic::enchantments::Enchantments| {
            slots.0.retain_mut(|slot| {
                if crate::game::magic::enchantments::OFFENSIVE.contains(&slot.spell) {
                    let cloud = &catalog[slot.spell];
                    let removed =
                        (amount + u32::from(spell.level)).saturating_sub(u32::from(cloud.level));
                    slot.turns = slot.turns.saturating_sub(removed);
                }
                slot.turns > 0
            });
        };
        match victim {
            Victim::Member(_) if !party_done => {
                purify(&mut world.enchantments);
                party_done = true;
            }
            Victim::Member(_) => continue,
            Victim::Monster(actor) => {
                purify(&mut world.monsters.enchantments_mut(actor));
            }
        }
        effects.push(Effect::Purified { victim });
    }
    effects
}

/// Heal a living victim, restore its stamina, or both and lift its
/// conditions (Restoration). What it reports is the first thing it did.
pub(super) fn restore_victim(
    kind: Kind,
    spell: &SpellDefinition,
    strength: Strength,
    v: Victim,
    rng: &mut GameRng,
    world: &mut Field,
) -> Option<Effect> {
    let dead = match v {
        Victim::Member(member) => world.members.get(member).unwrap().1.is_dead(),
        Victim::Monster(entity) => world.monsters.monsters().get(entity).unwrap().0.is_dead(),
    };
    if dead {
        return None;
    }
    let dice = if kind == Kind::Restoration { Some(RESTORATION_DICE) } else { spell.dice };
    let mut effect = Effect::Nothing { victim: v };
    if kind != Kind::Stamina
        && let Some(amount) = restore(rng, world, v, Pool::Health, dice, strength)
    {
        effect = Effect::Healed { victim: v, amount };
    }
    if kind != Kind::Heal
        && let Some(amount) = restore(rng, world, v, Pool::Stamina, dice, strength)
        && kind == Kind::Stamina
    {
        effect = Effect::Refreshed { victim: v, amount };
    }
    if kind == Kind::Restoration {
        use Condition::*;
        let all = [
            Diseased,
            Irritated,
            Nauseated,
            Slowed,
            Afraid,
            Poisoned,
            Silenced,
            Hexed,
            Insane,
            Blind,
            Turncoat,
            Webbed,
            Asleep,
            Paralyzed,
            Unconscious,
        ];
        let lifted = match v {
            Victim::Member(member) => {
                let (_, mut vitals, ..) = world.members.get_mut(member).unwrap();
                cure(rng, &mut vitals.conditions, &all, strength, true)
            }
            Victim::Monster(entity) => {
                let mut conditions = world.monsters.conditions_mut(entity);
                cure(rng, &mut conditions, &all, strength, true)
            }
        };
        if matches!(effect, Effect::Nothing { .. })
            && let Some(&condition) = lifted.first()
        {
            effect = Effect::Cured { victim: v, condition };
        }
    }
    Some(effect)
}

/// Enchant the victims: a personal enchantment on each member and monster
/// that accepts it, the party's own slots once for all its members.
pub(super) fn enchant(
    spell: &SpellDefinition,
    strength: Strength,
    victims: &[Victim],
    rng: &mut GameRng,
    world: &mut Field,
) -> Vec<Effect> {
    let turns = duration(rng, spell, strength);
    let mut effects = Vec::new();
    let mut party_applied = false;
    for &victim in victims {
        let personal = crate::game::magic::enchantments::personal(spell.id);
        let effect = crate::game::magic::enchantments::Enchantment {
            spell: spell.id,
            power: strength.power,
            boost: strength.boost,
            turns,
            absorption: 0,
        };
        let mut apply = |enchantments: &mut crate::game::magic::enchantments::Enchantments, monster: bool| {
            let accepts = if monster {
                enchantments.monster_accepts(spell.id, strength.power)
            } else {
                enchantments.accepts(spell.id, strength.power)
            };
            if !accepts {
                return false;
            }
            let absorption = if spell.id == SpellId::GUARDIAN_ANGEL {
                let rolled =
                    spell.dice.map_or(0, |dice| rng.roll(dice)) * u32::from(strength.power);
                rolled + rolled * strength.boost / 100
            } else {
                0
            };
            let effect = crate::game::magic::enchantments::Enchantment { absorption, ..effect };
            if monster { enchantments.apply_to_monster(effect) } else { enchantments.apply(effect) }
        };
        let changed = match victim {
            Victim::Member(member) if personal => {
                let (_, mut v, ..) = world.members.get_mut(member).unwrap();
                let v = &mut *v;
                if v.is_dead() {
                    false
                } else if spell.id == SpellId::HASTE {
                    crate::game::magic::enchantments::haste(
                        &mut v.conditions,
                        &mut v.enchantments,
                        effect,
                        false,
                    )
                } else {
                    apply(&mut v.enchantments, false)
                }
            }
            Victim::Member(_) => {
                if party_applied {
                    continue;
                }
                party_applied = true;
                apply(&mut world.enchantments, false)
            }
            Victim::Monster(entity) => {
                if spell.id == SpellId::HASTE {
                    let monster = &mut *world.monsters.monster_mut(entity);
                    crate::game::magic::enchantments::haste(
                        &mut monster.conditions,
                        &mut monster.enchantments,
                        effect,
                        true,
                    )
                } else {
                    apply(&mut world.monsters.enchantments_mut(entity), true)
                }
            }
        };
        effects.push(if changed {
            Effect::Enchanted { victim, spell: spell.id }
        } else {
            Effect::Nothing { victim }
        });
    }
    effects
}

/// The living member furthest below full health, or below full spell points
/// when not `health`, whom a drain restores.
pub(super) fn neediest(
    world: &Field,
    points: &Query<(&LearnedSpells, &mut SpellPoints)>,
    health: bool,
) -> Option<Entity> {
    world
        .party
        .iter()
        .filter_map(|m| {
            let (c, v, ..) = world.members.get(m).unwrap();
            if v.is_dead() {
                return None;
            }
            let percent = if health {
                i64::from(v.hp * 100 / c.stats(&v.enchantments).max_hp().max(1))
            } else {
                let (learned, pool) = points.get(m).unwrap();
                let maxima = world.mana_maximum(m, learned);
                let maximum: u32 = maxima.values().sum();
                if maximum == 0 {
                    return None;
                }
                maxima
                    .iter()
                    .map(|(realm, _)| i64::from(pool.available(realm, &maxima)))
                    .sum::<i64>()
                    * 100
                    / i64::from(maximum)
            };
            (percent < 100).then_some((m, percent))
        })
        .min_by_key(|&(_, percent)| percent)
        .map(|(m, _)| m)
}

/// What Restoration restores health and stamina by, whatever its record says.
pub(super) const RESTORATION_DICE: Dice = Dice { base: 0, count: 8, sides: 8 };

/// The share of Lifesteal's damage that heals the most wounded member, in
/// percent.
pub(super) const LIFESTEAL_SHARE: u32 = 70;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Pool {
    Health,
    Stamina,
}

/// Restore a living actor's health or stamina by rolled `dice`, up to the
/// maximum: the amount, or `None` (nothing rolled) when the actor lacks
/// none or cannot be restored.
pub(super) fn restore(
    rng: &mut GameRng,
    world: &mut Field,
    victim: Victim,
    pool: Pool,
    dice: Option<Dice>,
    strength: Strength,
) -> Option<u32> {
    let (current, maximum) = match victim {
        Victim::Member(member) => {
            let (character, vitals, ..) = world.members.get(member).unwrap();
            if vitals.is_dead() {
                return None;
            }
            let stats = character.stats(&vitals.enchantments);
            match pool {
                Pool::Health => (vitals.hp, stats.max_hp()),
                Pool::Stamina => (vitals.stamina, stats.max_stamina()),
            }
        }
        Victim::Monster(entity) => {
            let (body, ..) = world.monsters.monsters().get(entity).unwrap();
            if body.is_dead() {
                return None;
            }
            match pool {
                Pool::Health => (body.hp, body.max_hp),
                Pool::Stamina => (body.stamina, body.max_stamina),
            }
        }
    };
    let missing = maximum.saturating_sub(current);
    if missing == 0 {
        return None;
    }
    let amount = magnitude(rng, dice, strength).min(missing);
    match victim {
        Victim::Member(member) => {
            let (_, mut vitals, ..) = world.members.get_mut(member).unwrap();
            match pool {
                Pool::Health => vitals.heal(amount, maximum),
                Pool::Stamina => vitals.restore_stamina(amount, maximum),
            }
        }
        Victim::Monster(entity) => {
            match pool {
                Pool::Health => world.monsters.restore_health(entity, amount),
                Pool::Stamina => world.monsters.restore_stamina(entity, amount),
            };
        }
    }
    Some(amount)
}

/// Whether every victim of a restoring spell is now whole, so a repeated
/// cast can stop.
pub fn all_restored(spell: SpellId, victims: &[Victim], members: &Members) -> bool {
    victims.iter().all(|&victim| {
        let Victim::Member(member) = victim else { return true };
        let (character, vitals, ..) = members.get(member).unwrap();
        vitals.is_dead()
            || match kind(spell) {
                Some(Kind::Stamina) => {
                    vitals.stamina >= character.stats(&vitals.enchantments).max_stamina()
                }
                _ => vitals.hp >= character.stats(&vitals.enchantments).max_hp(),
            }
    })
}
