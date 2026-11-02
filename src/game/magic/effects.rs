//! What a blow does to its victim, over the field a fight resolves on:
//! its damage and its weapon's effects (retail `ApplyEffectConditions`).
//! Spells, breaths and clouds land as hits (`crate::game::hits`).

use bevy::prelude::*;

use super::{LearnedSpells, SpellPoints};
use crate::content::characters::{Attribute, Realm, Skill};
use crate::content::items::{Dice, ItemCatalog};
use crate::content::species::MonsterCatalog;
use crate::game::characters::{Condition, Fate, Lasting, Vitals};
use crate::game::difficulty::Difficulty;
use crate::game::hits::Effect;
use crate::game::hits::body::{
    CONDITION_SAVE, DEATH_SAVE, INDEFINITE, condition_chance, immune, resist, save_roll,
};
use crate::game::inventory::PartyInventory;
use crate::game::party::DamageApplied;
use crate::game::party::Members;
use crate::game::party::formation::Formation;
use crate::game::rng::GameRng;
use crate::game::world::damage::{DamageOutcome, MonsterDamage};
use crate::game::world::hostility::Stance;

/// What a spell's effects need from the world.
pub struct Field<'a, 'm, 'ms, 'd, 'ds, 'f, 'p> {
    pub underwater: bool,
    pub visibility: &'a crate::game::world::obstacles::WorldObstacles<'a, 'a>,
    pub enchantments: &'a mut super::enchantments::Enchantments,
    pub members: &'a mut Members<'m, 'ms>,
    pub monsters: &'a mut MonsterDamage<'d, 'ds>,
    /// The party's pool and purse.
    pub inventory: &'a mut PartyInventory,
    pub formation: &'a mut Mut<'f, Formation>,
    pub party_damage: &'a mut MessageWriter<'p, DamageApplied>,
    pub items: &'a ItemCatalog,
    pub species: &'a MonsterCatalog,
    pub spells: &'a crate::content::spells::SpellCatalog,
    /// What monster loot tables scale by.
    pub party_level: u32,
    /// The game time the action happens at, which a kill's offence is
    /// dated by.
    pub now: u64,
    pub difficulty: Difficulty,
    /// The current roster in order, including dead members eligible for revival.
    pub party: &'a [Entity],
    /// Skills that improved while resolving, for the notices.
    pub improved: Vec<(Entity, Skill)>,
    /// Monsters the party acted against, eligible to provoke their groups
    /// when the action was accepted ([`Field::provoke`]).
    pub fight: Option<Entity>,
    pub provocations: &'a mut MessageWriter<'p, crate::game::world::hostility::Provocation>,
}

impl Field<'_, '_, '_, '_, '_, '_, '_> {
    /// A member's or a monster's body.
    pub fn vitals(&self, body: Entity) -> &Vitals {
        match self.members.get(body) {
            Ok((_, vitals, ..)) => vitals,
            Err(_) => self.monsters.monsters().get(body).unwrap().5,
        }
    }

    /// A member's or a monster's body, to change.
    pub fn vitals_mut(&mut self, body: Entity) -> Mut<'_, Vitals> {
        if self.members.contains(body) {
            self.members.get_mut(body).unwrap().1
        } else {
            self.monsters.vitals_mut(body)
        }
    }

    /// The party acts against this monster. Whether that provokes its group
    /// is settled now, as the action is accepted, so nothing the action then
    /// does (a kill, a Turncoat) undoes it (retail `MakeTargetGroupHostile`):
    /// a living monster not under Turncoat does.
    pub fn provoke(&mut self, monster: Entity) {
        if self.monsters.provoke(monster, self.now) {
            self.provocations.write(crate::game::world::hostility::Provocation(monster));
        }
    }
}

impl Field<'_, '_, '_, '_, '_, '_, '_> {
    pub fn mana_maximum(
        &self,
        member: Entity,
        learned: &LearnedSpells,
    ) -> enum_map::EnumMap<Realm, u32> {
        let (profile, vitals, ..) = self.members.get(member).unwrap();
        profile.stats(&vitals.enchantments).max_points(learned, self.spells)
    }
}

/// The victim as saves and resistance see it.
struct Target {
    level: u32,
    resistance: i32,
    side: Option<Stance>,
    turncoat: bool,
    immune: bool,
    dead: bool,
}

fn target(world: &Field, victim: Entity, realm: Realm, condition: Option<Condition>) -> Target {
    if let Ok((character, vitals, carried, _)) = world.members.get(victim) {
        let resistances = crate::game::characters::resistance::member_resistances(
            &character.stats(&vitals.enchantments),
            carried,
            world.items,
            world.enchantments,
        );
        return Target {
            level: u32::from(character.level()),
            resistance: resistances[realm].total,
            side: None,
            turncoat: vitals.conditions.has(Condition::Turncoat),
            immune: false,
            dead: vitals.is_dead(),
        };
    }
    let (body, .., vitals) = world.monsters.monsters().get(victim).unwrap();
    let kind = &world.species[body.species];
    Target {
        level: u32::from(kind.level),
        resistance: crate::game::world::population::resistance(kind, vitals, realm),
        side: Some(world.monsters.stance(victim, world.now)),
        turncoat: false,
        immune: condition.is_some_and(|c| immune(kind.kind, c)),
        dead: vitals.is_dead(),
    }
}

/// Whether a victim saves (retail `TargetResistsCondition`): `(level −
/// strength)·3 + base + resistance`, scaled by difficulty, within 5..95. A
/// party member who saves practises Iron Will.
fn saves(
    rng: &mut GameRng,
    world: &mut Field,
    victim: Entity,
    target: &Target,
    base: i32,
    against: u32,
) -> bool {
    let chance = condition_chance(target.level, target.resistance, against, base);
    let side = target.side.unwrap_or(crate::game::world::hostility::member_stance(target.turncoat));
    let chance = world.difficulty.scale(chance, side);
    let saved = save_roll(rng, chance);
    if saved
        && let Ok((mut character, ..)) = world.members.get_mut(victim)
        && character.practise(Skill::IronWill, 2, rng)
    {
        world.improved.push((victim, Skill::IronWill));
    }
    saved
}

/// Drain a member's health for good ([`Vitals::drain_health`]): nothing wakes
/// them. A monster has no lasting maximum to lose and is hurt as by any
/// damage.
fn sap(rng: &mut GameRng, world: &mut Field, victim: Entity, amount: u32) -> Option<Effect> {
    let Ok((mut character, mut vitals, ..)) = world.members.get_mut(victim) else { return None };
    let mut fate = Fate { fight: world.fight, rng };
    let lost = crate::game::party::drain_member(
        victim,
        (&mut character, &mut vitals),
        (amount, &mut fate),
        world.party_damage,
    );
    let outcome = DamageOutcome { damage: lost, died: vitals.is_dead(), experience: 0 };
    Some(Effect::Damaged { victim, outcome })
}

/// Take up to `amount` stamina from a victim.
fn tire(world: &mut Field, victim: Entity, amount: u32) -> Effect {
    Effect::Tired { victim, amount: world.vitals_mut(victim).tire(amount) }
}

/// Take `amount` health from a victim; a member who survives may wake.
pub fn hurt(rng: &mut GameRng, world: &mut Field, victim: Entity, amount: u32) -> Option<Effect> {
    let Ok((character, mut vitals, ..)) = world.members.get_mut(victim) else {
        let outcome = world.monsters.apply(victim, amount, rng);
        return Some(Effect::Damaged { victim, outcome });
    };
    let mut fate = Fate { fight: world.fight, rng: &mut *rng };
    let lost = crate::game::party::damage_member(
        victim,
        (&character, &mut vitals),
        (amount, &mut fate),
        world.party_damage,
    );
    if lost > 0 && !vitals.is_dead() {
        let senses = character.stats(&vitals.enchantments).attribute(Attribute::Senses);
        vitals.conditions.wake_on_hit(senses, rng);
    }
    let outcome = DamageOutcome { damage: lost, died: vitals.is_dead(), experience: 0 };
    Some(Effect::Damaged { victim, outcome })
}

/// Give a victim a condition for `lasting`; a poison bites a member at
/// `poison` (retail's condition argument).
fn inflict_on(
    world: &mut Field,
    victim: Entity,
    (condition, poison): (Condition, u32),
    lasting: Lasting,
) -> Option<()> {
    let member = world.members.contains(victim);
    if condition == Condition::Turncoat
        && !member
        && world.monsters.stance(victim, world.now)
            == crate::game::world::hostility::Stance::Neutral
    {
        return None;
    }
    let mut vitals = world.vitals_mut(victim);
    // Only a member's poison bites by a strength.
    if member && condition == Condition::Poisoned {
        vitals.poison(lasting, poison).then_some(())?;
    } else {
        vitals.inflict(condition, lasting);
    }
    Some(())
}

/// Retail `ResolveAttackOnTarget`: a condition, or death when `condition` is
/// `None`, unless the victim is immune, saves, or shrugs it off by its
/// nature; then resistance cuts its duration, and nothing lands from none
/// (retail `InflictConditionOnTarget`). `rounds` of `None` lasts until
/// lifted.
fn afflict(
    rng: &mut GameRng,
    world: &mut Field,
    victim: Entity,
    (condition, argument): (Option<Condition>, u32),
    realm: Realm,
    against: u32,
    rounds: Option<u32>,
) -> Option<Effect> {
    let target = target(world, victim, realm, condition);
    if target.dead {
        return None;
    }
    // Some species cannot be slain outright.
    let death_immune = condition.is_none()
        && world
            .monsters
            .monsters()
            .get(victim)
            .is_ok_and(|(body, ..)| world.species[body.species].death_immune);
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
        rounds => Lasting::Rounds(rounds),
    };
    let Some(condition) = condition else { return kill(rng, world, victim) };
    inflict_on(world, victim, (condition, argument), lasting)?;
    Some(Effect::Inflicted { victim, condition })
}

/// Constitution and profession immunities are checked after the save.
fn shrugs(world: &Field, victim: Entity, condition: Condition, indefinite: bool) -> bool {
    use crate::content::characters::Trait;
    let Ok((character, ..)) = world.members.get(victim) else { return false };
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

/// Rounds, or an amount, a blow's effect rolls (retail `RollEffectDuration`
/// on an empty definition, then `count`d2 + `base` and one more when that
/// came to under two).
fn blow_roll(rng: &mut GameRng, count: u32, sides: i32, base: u32) -> u32 {
    if rng.below(4) == 1 {
        return 2;
    }
    let dice = Dice { count: count as i32, sides, base: base as i32 };
    rng.roll(dice) + 1
}

/// Retail `ApplyEffectConditions`: what a weapon or a monster's blow that
/// drew blood does besides its damage. Each effect with a chance rolls for
/// it in turn until the victim dies; `power` is the attacker's effect
/// strength, and only a monster swallows or possesses.
pub fn blow(
    rng: &mut GameRng,
    world: &mut Field,
    points: &mut Query<(&LearnedSpells, &mut SpellPoints)>,
    victim: Entity,
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
        if chance == 0 || rng.below(100) >= chance {
            continue;
        }
        let count = power / 3 + 1;
        let rounds = blow_roll(rng, count, 2, 1).clamp(1, 12);
        let member = world.members.contains(victim);
        let mut afflicts = |rng: &mut GameRng, condition, realm, rounds| {
            afflict(rng, world, victim, (condition, 0), realm, power, rounds)
        };
        let outcome = match effect {
            E::Sleep => afflicts(rng, Some(Condition::Asleep), Realm::Air, Some(rounds)),
            E::Paralyze => afflicts(rng, Some(Condition::Paralyzed), Realm::Water, Some(rounds)),
            // A poison bites at the weapon's strength (retail
            // `ApplyEffectConditions` passes its magnitude base).
            E::Poison => {
                let rounds = blow_roll(rng, count * 3, 2, effects.strength);
                afflict(
                    rng,
                    world,
                    victim,
                    (Some(Condition::Poisoned), effects.strength),
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
            E::DrainSpellPoints if member => {
                drain(rng, world, victim, Realm::Divine, power, (count, 6, count)).map(|amount| {
                    let (learned, mut pool) = points.get_mut(victim).unwrap();
                    let maximum = world.mana_maximum(victim, learned);
                    let amount = pool.drain(amount, &maximum, rng);
                    Effect::Drained { victim, amount }
                })
            }
            E::DrainSpellPoints => None,
            E::Nauseate => afflicts(rng, Some(Condition::Nauseated), Realm::Air, Some(rounds)),
            E::Insane => afflicts(rng, Some(Condition::Insane), Realm::Mental, Some(rounds)),
        };
        landed.extend(outcome);
    }
    landed
}

/// A blow's drain: the victim's save, then `dice` (count, sides, base),
/// one more, cut by resistance. `None` when nothing is left to take.
fn drain(
    rng: &mut GameRng,
    world: &mut Field,
    victim: Entity,
    realm: Realm,
    power: u32,
    (count, sides, base): (u32, i32, u32),
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

/// Death bypasses absorption, while retaining the common death/loot transaction.
fn kill(rng: &mut GameRng, world: &mut Field, victim: Entity) -> Option<Effect> {
    // A failed death save inflicts Dead; it is not absorbable damage.
    if world.members.contains(victim) {
        return perish(rng, world, victim);
    }
    let mut vitals = world.vitals_mut(victim);
    vitals.enchantments.0.retain(|e| e.spell != crate::content::spells::SpellId::GUARDIAN_ANGEL);
    let hp = vitals.health.current;
    hurt(rng, world, victim, hp)
}

/// Death inflicted on a member outright ([`crate::game::party::kill_member`]):
/// no damage first, so nothing absorbs or tires, but death may be cheated.
fn perish(rng: &mut GameRng, world: &mut Field, victim: Entity) -> Option<Effect> {
    let Ok((character, mut vitals, ..)) = world.members.get_mut(victim) else { return None };
    let mut fate = Fate { fight: world.fight, rng };
    let lost = crate::game::party::kill_member(
        victim,
        (&character, &mut vitals),
        &mut fate,
        world.party_damage,
    );
    let outcome = DamageOutcome { damage: lost, died: vitals.is_dead(), experience: 0 };
    Some(Effect::Damaged { victim, outcome })
}

/// Whether every victim of a restoring spell is now whole, so a repeated
/// cast can stop.
pub fn all_restored(
    spell: crate::content::spells::SpellId,
    victims: &[Entity],
    members: &Members,
) -> bool {
    use crate::game::hits::spells::{Kind, kind};
    victims.iter().all(|&victim| {
        let Ok((_, vitals, ..)) = members.get(victim) else { return true };
        vitals.is_dead()
            || match kind(spell) {
                Some(Kind::Stamina) => vitals.stamina.current >= vitals.stamina.max,
                _ => vitals.health.current >= vitals.health.max,
            }
    })
}
