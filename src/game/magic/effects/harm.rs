//! What hurts: damage and its resistance, conditions and their saves,
//! blows, draining, death and burnt consumables.
use super::*;

/// Consume physical units, not charges, in place; remaining identity and
/// equipment binding metadata belong to the existing owner.
pub(super) fn burn_stack(
    stack: crate::game::inventory::ItemStack,
    items: &ItemCatalog,
    chance: u32,
    rng: &mut GameRng,
    totals: &mut [u32; 3],
) -> Option<crate::game::inventory::ItemStack> {
    let definition = &items[stack.item];
    let category = match definition.class {
        EquipClass::Potion => 0,
        EquipClass::Scroll => 1,
        EquipClass::Spellbook => 2,
        _ => return Some(stack),
    };
    let lost = (0..stack.count()).filter(|_| rng.below(100) < chance).count() as u16;
    totals[category] += u32::from(lost);
    stack.remove_units(lost)
}

/// Each condition's base save (retail `g_condition_resist_base`), by retail
/// number (Drained is 1, Dead 18).
pub(super) const CONDITION_SAVE: [i32; 20] =
    [0, 1, 2, 3, 4, 4, 5, 6, 7, 8, 9, 10, 12, 14, 16, 20, 18, 22, 24, 22];
pub(super) const DEATH_SAVE: i32 = CONDITION_SAVE[18];

/// The monster kinds Dispel Undead harms.
pub(super) const UNDEAD_KINDS: [MonsterKind; 2] = [MonsterKind::Undead, MonsterKind::Demon];

/// The kind only Banish harms besides them.
pub(super) const BANISHED_KIND: MonsterKind = MonsterKind::Spirit;

/// Whether a monster of `kind` shrugs a condition off (retail
/// `g_condition_immunities`).
pub(super) fn immune(kind: MonsterKind, condition: Condition) -> bool {
    use Condition::*;
    let immune: &[_] = match kind {
        MonsterKind::Undead => {
            &[Diseased, Poisoned, Unconscious, Irritated, Afraid, Insane, Asleep, Nauseated]
        }
        MonsterKind::Android => &[Diseased, Poisoned, Nauseated, Irritated, Afraid, Asleep],
        MonsterKind::Plant => &[Insane, Afraid, Nauseated],
        _ => &[],
    };
    immune.contains(&condition)
}

/// The victim as saves and resistance see it.
pub(super) struct Target {
    pub(super) level: u32,
    pub(super) resistance: i32,
    pub(super) side: Option<Stance>,
    pub(super) turncoat: bool,
    pub(super) immune: bool,
    pub(super) dead: bool,
}

pub(super) fn target(
    world: &Field,
    victim: Victim,
    realm: Realm,
    condition: Option<Condition>,
) -> Target {
    match victim {
        Victim::Monster(monster) => {
            let (body, ..) = world.monsters.monsters().get(monster).unwrap();
            let kind = &world.species()[body.species];
            Target {
                level: u32::from(kind.level),
                resistance: body.resistance(kind, realm),
                side: Some(world.monsters.stance(monster, world.now())),
                turncoat: false,
                immune: condition.is_some_and(|c| immune(kind.kind, c)),
                dead: body.is_dead(),
            }
        }
        Victim::Member(member) => {
            let (character, vitals, carried, _) = world.members.get(member).unwrap();
            let resistances = crate::game::characters::resistance::member_resistances(
                &character.stats(&vitals.enchantments),
                carried,
                world.items(),
                &world.enchantments,
            );
            Target {
                level: u32::from(character.level()),
                resistance: resistances[realm].total,
                side: None,
                turncoat: vitals.conditions.has(Condition::Turncoat),
                immune: false,
                dead: vitals.is_dead(),
            }
        }
    }
}

pub(in crate::game::magic) fn condition_chance(
    level: u32,
    resistance: i32,
    against: u32,
    base: i32,
) -> i32 {
    (level as i32 - against as i32) * 3 + base + resistance
}

/// A saving throw at `chance` percent, held within 5..95 (retail
/// `TargetResistsCondition`).
pub(in crate::game::magic) fn save_roll(rng: &mut GameRng, chance: i32) -> bool {
    (rng.below(100) as i32) < chance.clamp(5, 95)
}

/// Whether a victim saves (retail `TargetResistsCondition`): `(level −
/// strength)·3 + base + resistance`, scaled by difficulty, within 5..95. A
/// party member who saves practises Iron Will.
pub(super) fn saves(
    rng: &mut GameRng,
    world: &mut Field,
    victim: Victim,
    target: &Target,
    base: i32,
    against: u32,
) -> bool {
    let chance = condition_chance(target.level, target.resistance, against, base);
    let chance = match target.side {
        Some(side) => world.difficulty().for_monster(chance, side),
        None => world.difficulty().for_character(chance, target.turncoat),
    };
    let saved = save_roll(rng, chance);
    if saved
        && let Victim::Member(member) = victim
        && let Ok((mut character, ..)) = world.members.get_mut(member)
        && character.practise(Skill::IronWill, 2, rng)
    {
        world.improved.push((member, Skill::IronWill));
    }
    saved
}

/// Damage one victim by an already rolled magnitude, less resistance.
pub(super) fn damage(
    rng: &mut GameRng,
    world: &mut Field,
    victim: Victim,
    realm: Realm,
    rolled: u32,
    against: u32,
) -> Option<Effect> {
    let amount = resisted_magnitude(rng, world, victim, realm, rolled, against)?;
    if amount == 0 {
        return Some(Effect::Resisted { victim });
    }
    hurt(rng, world, victim, amount)
}

/// The post-resistance magnitude remains a transaction input, independent
/// of absorption, overkill and the HP outcome.
pub(super) fn resisted_magnitude(
    rng: &mut GameRng,
    world: &Field,
    victim: Victim,
    realm: Realm,
    rolled: u32,
    against: u32,
) -> Option<u32> {
    let target = target(world, victim, realm, None);
    if target.dead {
        return None;
    }
    Some(resist(rng, rolled, target.resistance, target.level, against))
}

/// Drain a member's health for good ([`Vitals::drain_health`]): nothing wakes
/// them. A monster has no lasting maximum to lose and is hurt as by any
/// damage.
pub(super) fn sap(
    rng: &mut GameRng,
    world: &mut Field,
    victim: Victim,
    amount: u32,
) -> Option<Effect> {
    let Victim::Member(member) = victim else { return None };
    let fight = world.fight();
    let (mut character, mut vitals, ..) = world.members.get_mut(member).unwrap();
    let mut fate = Fate { fight, rng };
    let lost = crate::game::party::drain_member(
        member,
        (&mut character, &mut vitals),
        &mut world.formation,
        (amount, &mut fate),
        &mut world.party_damage,
    );
    let outcome = DamageOutcome { damage: lost, died: vitals.is_dead(), experience: 0 };
    Some(Effect::Damaged { victim, outcome })
}

/// Take up to `amount` stamina from a victim.
pub(super) fn tire(world: &mut Field, victim: Victim, amount: u32) -> Effect {
    let amount = match victim {
        Victim::Member(member) => world.members.get_mut(member).map_or(0, |(_, mut vitals, ..)| {
            let before = vitals.stamina;
            vitals.spend_stamina(amount);
            before - vitals.stamina
        }),
        Victim::Monster(monster) => world.monsters.fatigue(monster, amount),
    };
    Effect::Tired { victim, amount }
}

/// Take `amount` health from a victim; a member who survives may wake.
pub fn hurt(rng: &mut GameRng, world: &mut Field, victim: Victim, amount: u32) -> Option<Effect> {
    match victim {
        Victim::Monster(monster) => {
            let outcome = world.monsters.apply(
                monster,
                amount,
                (world.party_level(), world.now()),
                &mut world.inventory,
                rng,
            );
            Some(Effect::Damaged { victim, outcome })
        }
        Victim::Member(member) => {
            let fight = world.fight();
            let (character, mut vitals, ..) = world.members.get_mut(member).unwrap();
            let mut fate = Fate { fight, rng: &mut *rng };
            let lost = crate::game::party::damage_member(
                member,
                (&character, &mut vitals),
                &mut world.formation,
                (amount, &mut fate),
                &mut world.party_damage,
            );
            if lost > 0 && !vitals.is_dead() {
                let senses = character.stats(&vitals.enchantments).attribute(Attribute::Senses);
                vitals.conditions.wake_on_hit(senses, rng);
            }
            let outcome = DamageOutcome { damage: lost, died: vitals.is_dead(), experience: 0 };
            Some(Effect::Damaged { victim, outcome })
        }
    }
}

/// Give a victim a condition for `lasting`; a poison bites a member at
/// `poison` (retail's condition argument).
pub(super) fn inflict_on(
    world: &mut Field,
    victim: Victim,
    (condition, poison): (Condition, u32),
    lasting: Lasting,
) -> Option<()> {
    if condition == Condition::Turncoat
        && let Victim::Monster(entity) = victim
        && world.monsters.stance(entity, world.now())
            == crate::game::world::hostility::Stance::Neutral
    {
        return None;
    }
    match victim {
        Victim::Monster(monster) => {
            let monster = &mut *world.monsters.monster_mut(monster);
            if condition == Condition::Slowed {
                crate::game::magic::enchantments::slow(
                    &mut monster.conditions,
                    &mut monster.enchantments,
                    lasting,
                );
            } else {
                monster.conditions.inflict(condition, lasting);
            }
        }
        Victim::Member(member) => {
            let mut vitals = world.members.get_mut(member).unwrap().1;
            if condition == Condition::Poisoned {
                vitals.poison(lasting, poison).then_some(())?;
            } else {
                vitals.inflict(condition, lasting);
            }
        }
    }
    Some(())
}

/// Retail `InflictConditionAttack` on one victim: the chance, a fresh
/// duration, then [`afflict`].
#[allow(clippy::too_many_arguments)]
pub(super) fn attack(
    rng: &mut GameRng,
    world: &mut Field,
    spell: &SpellDefinition,
    victim: Victim,
    realm: Realm,
    condition: Condition,
    chance: u32,
    strength: Strength,
) -> Option<Effect> {
    if rng.below(100) >= chance {
        return None;
    }
    // A victim that died or left is not attacked at all.
    if target(world, victim, realm, Some(condition)).dead {
        return None;
    }
    let rounds = duration(rng, spell, strength);
    afflict(rng, world, victim, (Some(condition), 0), realm, strength.against, Some(rounds))
}

/// Retail `ResolveAttackOnTarget`: a condition, or death when `condition` is
/// `None`, unless the victim is immune, saves, or shrugs it off by its
/// nature; then resistance cuts its duration, and nothing lands from none
/// (retail `InflictConditionOnTarget`). `rounds` of `None` lasts until
/// lifted.
pub(super) fn afflict(
    rng: &mut GameRng,
    world: &mut Field,
    victim: Victim,
    (condition, argument): (Option<Condition>, u32),
    realm: Realm,
    against: u32,
    rounds: Option<u32>,
) -> Option<Effect> {
    let target = target(world, victim, realm, condition);
    if target.dead {
        return None;
    }
    let death_immune = match victim {
        Victim::Monster(monster) => {
            condition.is_none() && {
                let (body, ..) = world.monsters.monsters().get(monster).unwrap();
                world.species()[body.species].death_immune
            }
        }
        Victim::Member(_) => false,
    };
    if target.immune || death_immune {
        return Some(Effect::Resisted { victim });
    }
    let base = condition.map_or(DEATH_SAVE, |c| CONDITION_SAVE[c as usize + 1]);
    if saves(rng, world, victim, &target, base, against) {
        return Some(Effect::Resisted { victim });
    }
    if condition.is_some_and(|condition| shrugs(world, victim, condition, rounds.is_none())) {
        return Some(Effect::Resisted { victim });
    }
    let lasting = rounds.unwrap_or(INDEFINITE);
    let lasting = match resist(rng, lasting, target.resistance, target.level, against) {
        0 => return Some(Effect::Resisted { victim }),
        _ if rounds.is_none() => Lasting::Indefinite,
        1 => Lasting::Rounds(2),
        rounds => Lasting::Rounds(rounds.min(u32::from(u16::MAX)) as u16),
    };
    let Some(condition) = condition else { return kill(rng, world, victim) };
    inflict_on(world, victim, (condition, argument), lasting)?;
    Some(Effect::Inflicted { victim, condition })
}

/// Constitution and profession immunities are checked after the save.
pub(super) fn shrugs(
    world: &Field,
    victim: Victim,
    condition: Condition,
    indefinite: bool,
) -> bool {
    use crate::content::characters::Trait;
    let Victim::Member(member) = victim else { return false };
    let (character, ..) = world.members.get(member).unwrap();
    let android = character.has_trait(Trait::AndroidConditionImmunity);
    match condition {
        Condition::Afraid => character.has_trait(Trait::Fearless) || android,
        Condition::Diseased
        | Condition::Irritated
        | Condition::Nauseated
        | Condition::Poisoned
        | Condition::Asleep => android,
        Condition::Insane if indefinite => false,
        Condition::Insane | Condition::Turncoat => {
            character.has_trait(Trait::MentalConditionImmunity)
        }
        _ => false,
    }
}

/// Retail's marker for a condition without end (`W8_CONDITION_INDEFINITE`),
/// which resistance cuts like any other duration.
pub(super) const INDEFINITE: u32 = 9999;

/// Rounds, or an amount, a blow's effect rolls (retail `RollEffectDuration`
/// on an empty definition, then `count`d2 + `base` and one more when that
/// came to under two).
pub(super) fn blow_roll(rng: &mut GameRng, count: u32, sides: u8, base: u32) -> u32 {
    if rng.below(4) == 1 {
        return 2;
    }
    let dice = Dice { count: count.min(255) as u8, sides, base: base.min(i16::MAX as u32) as i16 };
    rng.roll(dice) + 1
}

/// Retail `ApplyEffectConditions`: what a weapon or a monster's blow that
/// drew blood does besides its damage. Each effect with a chance rolls for
/// it in turn until the victim dies; `power` is the attacker's effect
/// strength, and only a monster swallows or possesses.
pub fn blow(
    rng: &mut GameRng,
    world: &mut Field,
    victim: Victim,
    effects: &crate::content::items::AttackEffects,
    power: u32,
    by_monster: bool,
) -> Vec<Effect> {
    use crate::content::items::AttackEffect as E;
    let mut landed = Vec::new();
    for (effect, &chance) in &effects.chances {
        if target(world, victim, Realm::Fire, None).dead {
            break;
        }
        if chance == 0 || rng.below(100) >= u32::from(chance) {
            continue;
        }
        let count = power / 3 + 1;
        let rounds = blow_roll(rng, count, 2, 1).clamp(1, 12);
        let mut afflicts = |rng: &mut GameRng, condition, realm, rounds| {
            afflict(rng, world, victim, (condition, 0), realm, power, rounds)
        };
        let member = matches!(victim, Victim::Member(_));
        let outcome = match effect {
            E::Sleep => afflicts(rng, Some(Condition::Asleep), Realm::Air, Some(rounds)),
            E::Paralyze => afflicts(rng, Some(Condition::Paralyzed), Realm::Water, Some(rounds)),
            // A poison bites at the weapon's strength (retail
            // `ApplyEffectConditions` passes its magnitude base).
            E::Poison => {
                let rounds = blow_roll(rng, count * 3, 2, u32::from(effects.strength));
                afflict(
                    rng,
                    world,
                    victim,
                    (Some(Condition::Poisoned), u32::from(effects.strength)),
                    Realm::Water,
                    power,
                    Some(rounds),
                )
            }
            E::Hex => afflicts(rng, Some(Condition::Hexed), Realm::Divine, Some(rounds)),
            E::Disease => afflicts(rng, Some(Condition::Diseased), Realm::Water, None),
            E::Kill => afflicts(rng, None, Realm::Divine, None),
            E::KnockOut => afflicts(rng, Some(Condition::Unconscious), Realm::Earth, Some(rounds)),
            E::Blind => afflicts(rng, Some(Condition::Blind), Realm::Fire, Some(rounds)),
            E::Frighten => afflicts(rng, Some(Condition::Afraid), Realm::Mental, Some(rounds)),
            // Being swallowed (retail Missing) is not modelled.
            E::Swallow => None,
            E::Possess if by_monster && member => {
                afflicts(rng, Some(Condition::Turncoat), Realm::Divine, Some(rounds))
            }
            E::Possess => None,
            E::DrainHealth if member => {
                drain(rng, world, victim, Realm::Earth, power, (count, 2, 0))
                    .and_then(|amount| sap(rng, world, victim, amount))
            }
            E::DrainHealth => None,
            E::DrainStamina => {
                drain(rng, world, victim, Realm::Water, power, (count * 2, 5, count * 3))
                    .map(|amount| tire(world, victim, amount))
            }
            E::DrainSpellPoints => match victim {
                Victim::Member(member) => {
                    drain(rng, world, victim, Realm::Divine, power, (count, 6, count)).map(
                        |amount| {
                            let maximum = world.mana_maximum(member);
                            let (_, mut pool) = world.points.get_mut(member).unwrap();
                            let amount = pool.drain(amount, &maximum, rng);
                            Effect::Drained { victim, amount }
                        },
                    )
                }
                Victim::Monster(_) => None,
            },
            E::Nauseate => afflicts(rng, Some(Condition::Nauseated), Realm::Air, Some(rounds)),
            E::Insane => afflicts(rng, Some(Condition::Insane), Realm::Mental, Some(rounds)),
        };
        landed.extend(outcome);
    }
    landed
}

/// A blow's drain: the victim's save, then `dice` (count, sides, base),
/// one more, cut by resistance. `None` when nothing is left to take.
pub(super) fn drain(
    rng: &mut GameRng,
    world: &mut Field,
    victim: Victim,
    realm: Realm,
    power: u32,
    (count, sides, base): (u32, u8, u32),
) -> Option<u32> {
    let target = target(world, victim, realm, None);
    if target.dead {
        return None;
    }
    if saves(rng, world, victim, &target, 0, power) {
        return None;
    }
    let amount = blow_roll(rng, count, sides, base);
    Some(resist(rng, amount, target.resistance, target.level, power)).filter(|&a| a != 0)
}

/// Death spells: a failed save kills, unless resistance cuts the attempt's
/// duration to nothing (retail `InflictConditionOnTarget` for Dead).
pub(super) fn slay(
    rng: &mut GameRng,
    world: &mut Field,
    spell: &SpellDefinition,
    victim: Victim,
    realm: Realm,
    strength: Strength,
) -> Option<Effect> {
    let target = target(world, victim, realm, None);
    if target.dead {
        return None;
    }
    let rounds = duration(rng, spell, strength);
    if saves(rng, world, victim, &target, DEATH_SAVE, strength.against)
        || resist(rng, rounds, target.resistance, target.level, strength.against) == 0
    {
        return Some(Effect::Resisted { victim });
    }
    kill(rng, world, victim)
}

/// Death bypasses absorption, while retaining the common death/loot transaction.
pub(super) fn kill(rng: &mut GameRng, world: &mut Field, victim: Victim) -> Option<Effect> {
    // A failed death save inflicts Dead; it is not absorbable damage.
    let Victim::Monster(monster) = victim else { return perish(rng, world, victim) };
    let hp = world.monsters.monsters().get(monster).unwrap().0.hp;
    world
        .monsters
        .enchantments_mut(monster)
        .0
        .retain(|e| e.spell != crate::content::spells::SpellId::GUARDIAN_ANGEL);
    hurt(rng, world, victim, hp)
}

/// Death inflicted on a member outright ([`crate::game::party::kill_member`]):
/// no damage first, so nothing absorbs or tires, but death may be cheated.
pub(super) fn perish(rng: &mut GameRng, world: &mut Field, victim: Victim) -> Option<Effect> {
    let Victim::Member(member) = victim else { return None };
    let fight = world.fight();
    let (character, mut vitals, ..) = world.members.get_mut(member).unwrap();
    let mut fate = Fate { fight, rng };
    let lost = crate::game::party::kill_member(
        member,
        (&character, &mut vitals),
        &mut world.formation,
        &mut fate,
        &mut world.party_damage,
    );
    let outcome = DamageOutcome { damage: lost, died: vitals.is_dead(), experience: 0 };
    Some(Effect::Damaged { victim, outcome })
}

/// Prismic spells' conditions and their weights (retail `ME:251-253`).
pub(super) const AFFLICTIONS: [(Option<Condition>, u32); 9] = [
    (Some(Condition::Nauseated), 10),
    (Some(Condition::Afraid), 10),
    (Some(Condition::Hexed), 5),
    (Some(Condition::Insane), 5),
    (Some(Condition::Blind), 5),
    (Some(Condition::Turncoat), 1),
    (Some(Condition::Asleep), 10),
    (Some(Condition::Paralyzed), 3),
    (None, 1),
];

/// Burn potions, scrolls and books the members and the party carry.
pub(super) fn destroy_consumables(
    strength: Strength,
    victims: &[Victim],
    rng: &mut GameRng,
    world: &mut Field,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    let mut totals = [0; 3];
    let chance = u32::from(strength.power) * 10;
    let items = world.items();
    for &victim in victims {
        let Victim::Member(member) = victim else { continue };
        let (_, _, mut carried, ..) = world.members.get_mut(member).unwrap();
        // Retail visits all twelve equipped slots, including alternate hands.
        for (_, slot) in &mut carried.equipment {
            if let Some(equipped) = slot.take() {
                *slot = burn_stack(equipped.stack, items, chance, rng, &mut totals).map(|stack| {
                    crate::game::inventory::Equipped {
                        stack,
                        curse_released: equipped.curse_released,
                    }
                });
            }
        }
        for slot in &mut carried.backpack {
            if let Some(stack) = slot.take() {
                *slot = burn_stack(stack, items, chance, rng, &mut totals);
            }
        }
    }
    if victims.iter().any(|v| matches!(v, Victim::Member(_))) {
        let pool = &mut world.inventory.pool.stacks;
        *pool = std::mem::take(pool)
            .into_iter()
            .filter_map(|stack| burn_stack(stack, items, chance, rng, &mut totals))
            .collect();
    }
    for (class, amount) in
        [EquipClass::Potion, EquipClass::Scroll, EquipClass::Spellbook].into_iter().zip(totals)
    {
        if amount != 0 {
            effects.push(Effect::Destroyed { class, amount });
        }
    }
    effects
}
