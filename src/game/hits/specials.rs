//! Special attacks landing: a breath, a gaze (retail
//! `ResolveMonsterGroupAttack`), and the offensive clouds pulsing on whoever
//! they hang over (retail `TickCombatEffectSlots`).

use bevy::prelude::*;

use super::body::{self, Body, BodyItem, wound};
use super::spells::{afflict, duration, magnitude};
use super::{Deed, Effect, HitOf, Strikes};
use crate::content::characters::Realm;
use crate::content::species::SpecialAttackKind;
use crate::content::spells::SpellId;
use crate::game::characters::{Condition, Lasting};
use crate::game::difficulty::Difficulty;
use crate::game::factions::Factions;
use crate::game::magic::Strength;
use crate::game::magic::enchantments::PartyEffects;
use crate::game::party::DamageApplied;
use crate::game::rng::GameRng;
use crate::game::world::identify::MonsterGroup;

/// What one of a special attack's two effects does to a victim (retail
/// `g_special_attack_table`'s entries).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecialEffect {
    /// A condition, for rounds rolled by its strength (entry 1).
    Inflict(Condition),
    /// Damage (entry 2).
    Harm,
    /// Spell points: a member's realm six times over, a monster's budget
    /// by a sixth (entry 3).
    Drain,
    /// Stamina (entry 4).
    Tire,
    /// Damage, resisted only where the definition says (entry 5).
    Wound,
}

/// A special attack's effects, the realm resisting them, and what stops it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpecialDefinition {
    pub effects: [Option<SpecialEffect>; 2],
    /// It summons rather than strikes.
    pub summons: bool,
    /// The realm victims resist it by.
    pub realm: Realm,
    /// Whether resistance cuts it at all; one kind's wound is not resisted.
    pub resisted: bool,
    /// Whether a silence stops it.
    pub silenced: bool,
}

/// A special attack's definition (retail `g_special_attack_table`,
/// `g_special_attack_condition_table`, `g_special_attack_realm_table`,
/// `MonsterSpecialAttackHonorsCastingBlock`).
pub fn special(kind: SpecialAttackKind) -> SpecialDefinition {
    use Condition::*;
    use Realm::*;
    use SpecialAttackKind as K;
    use SpecialEffect::*;
    let strike = |realm, first, second: Option<SpecialEffect>| SpecialDefinition {
        effects: [Some(first), second],
        summons: false,
        realm,
        resisted: true,
        silenced: false,
    };
    let summon = |silenced| SpecialDefinition {
        effects: [None, None],
        summons: true,
        realm: Fire,
        resisted: true,
        silenced,
    };
    let silenced = |definition| SpecialDefinition { silenced: true, ..definition };
    match kind {
        K::BlindingWater | K::BlindingWaterSecond => strike(Water, Inflict(Blind), None),
        K::HarmAndDrainDivine => strike(Divine, Harm, Some(Drain)),
        K::TiringWater => strike(Water, Tire, None),
        K::FearAndInsanityMental => {
            silenced(strike(Mental, Inflict(Afraid), Some(Inflict(Insane))))
        }
        K::FearAndTurncoatDivine => {
            silenced(strike(Divine, Inflict(Afraid), Some(Inflict(Turncoat))))
        }
        K::FearMental => silenced(strike(Mental, Inflict(Afraid), None)),
        K::WoundAndTireFire => strike(Fire, Wound, Some(Tire)),
        K::WoundFire => strike(Fire, Wound, None),
        K::InsanityAndWoundMental => {
            SpecialDefinition { resisted: false, ..strike(Mental, Inflict(Insane), Some(Wound)) }
        }
        K::InsanityAndTurncoatDivine => strike(Divine, Inflict(Insane), Some(Inflict(Turncoat))),
        K::InsanityMental => strike(Mental, Inflict(Insane), None),
        K::IrritationEarth => strike(Earth, Inflict(Irritated), None),
        K::IrritationAir => strike(Air, Inflict(Irritated), None),
        K::NauseaEarth => strike(Earth, Inflict(Nauseated), None),
        K::NauseaAir => strike(Air, Inflict(Nauseated), None),
        K::ParalysisMental => strike(Mental, Inflict(Paralyzed), None),
        K::ParalysisEarth => strike(Earth, Inflict(Paralyzed), None),
        K::PoisonAndIrritationWater => strike(Water, Inflict(Poisoned), Some(Inflict(Irritated))),
        K::PoisonAndWoundWater => strike(Water, Inflict(Poisoned), Some(Wound)),
        K::SleepAir => strike(Air, Inflict(Asleep), None),
        K::SlowAndParalysisWater => strike(Water, Inflict(Slowed), Some(Inflict(Paralyzed))),
        K::SlowEarth => strike(Earth, Inflict(Slowed), None),
        K::WoundWater => strike(Water, Wound, None),
        K::SummonsSpecies153 | K::SummonsSpecies151 | K::SummonsSpecies12 => summon(false),
        K::SummonsSpecies149
        | K::SummonsSpecies150
        | K::SummonsSpecies152
        | K::SummonsSpecies148 => summon(true),
    }
}

/// A character's breath (retail special-attack entry 30): a Water blast.
pub const CHARACTER_BREATH: SpecialDefinition = SpecialDefinition {
    effects: [Some(SpecialEffect::Harm), None],
    summons: false,
    realm: Realm::Water,
    resisted: true,
    silenced: false,
};

/// A special attack landing on one body. Its strength follows its maker's
/// level and how fresh it is: `against` for saves and resistance, `bound`
/// for its rolls.
#[derive(Component, Clone, Copy, Debug)]
pub struct SpecialHit {
    pub definition: SpecialDefinition,
    pub against: u32,
    pub bound: u32,
}

/// Make a special attack at `victims` as part of `deed`: a hit on each. Its
/// strength follows `level` and how much of `stamina` its maker has left.
pub fn breathe(
    commands: &mut Commands,
    deed: Entity,
    definition: SpecialDefinition,
    (level, stamina): (u8, crate::game::characters::Pool),
    victims: &[Entity],
) {
    if definition.summons {
        return;
    }
    let level = u32::from(level);
    let fresh = |extra: u32| {
        (stamina.current * (level + level.min(15) + extra) / stamina.max.max(1)).max(1)
    };
    // Retail's doubling of a fifth-entry effect reads the table's first
    // rows, which never hold one.
    let (against, bound) = (fresh(0), fresh(5));
    for &victim in victims {
        commands.spawn((SpecialHit { definition, against, bound }, Strikes(victim), HitOf(deed)));
    }
}

/// Three rolls under `bound`, summed (retail `RollSpecialAttackSum`).
fn three(rng: &mut GameRng, bound: u32) -> u32 {
    rng.below(bound) + rng.below(bound) + rng.below(bound)
}

/// Land each special attack on its body: each victim may escape each of
/// its two effects (the first one time in ten, the second one in two), and
/// resists each by its realm.
#[allow(clippy::too_many_arguments)]
pub fn land_specials(
    mut commands: Commands,
    hits: Query<(Entity, &SpecialHit, &Strikes, &HitOf)>,
    mut bodies: Query<Body>,
    mut deeds: Query<&mut Deed>,
    party: Single<&PartyEffects>,
    groups: Query<&MonsterGroup>,
    content: If<Res<crate::game::data::Catalogs>>,
    (difficulty, factions, clock): (
        Res<Difficulty>,
        Res<Factions>,
        Res<crate::game::time::GameClock>,
    ),
    fight: Query<Entity, With<crate::game::combat::Battle>>,
    mut rng: ResMut<GameRng>,
    mut notices: MessageWriter<DamageApplied>,
) {
    let (items, species, spells) = (&content.items, &content.species, &content.spells);
    let (difficulty, fight) = (*difficulty, fight.single().ok());
    let rng = &mut *rng;
    for (hit, &SpecialHit { definition, against, bound }, &Strikes(victim), &HitOf(of)) in &hits {
        commands.entity(hit).despawn();
        let Ok(mut body) = bodies.get_mut(victim) else { continue };
        let body = &mut body;
        let mut deed = deeds.get_mut(of).unwrap();
        let deed = &mut *deed;
        let monster = body.monster.is_some();
        let realm = definition.realm;
        let defence =
            body.defence(realm, (items, species, &party.0), (&groups, &*factions, clock.ms));
        for (slot, effect) in definition.effects.iter().enumerate() {
            let escapes = if slot == 0 { rng.below(100) <= 9 } else { rng.below(100) <= 49 };
            let Some(effect) = effect.filter(|_| !escapes) else { continue };
            let out = (&mut commands, &mut notices);
            let resisted = |rng: &mut GameRng, body: &BodyItem, rolled| {
                (!body.is_dead())
                    .then(|| body::resist(rng, rolled, defence.resistance, defence.level, against))
            };
            let landed = match effect {
                SpecialEffect::Inflict(condition) => {
                    let rounds = if monster { (bound / 3).max(1) } else { rng.below(bound) };
                    // Retail rolls a poison's strength, which is not kept.
                    if condition == Condition::Poisoned {
                        rng.below(if monster { 2 } else { bound });
                    }
                    afflict(
                        rng,
                        body,
                        &defence,
                        (Some(condition), 0),
                        (Some(rounds), against),
                        (species, difficulty, fight),
                        &mut deed.improved,
                        out,
                    )
                }
                SpecialEffect::Harm => {
                    let rolled = three(rng, bound) / 3 + 1;
                    match resisted(rng, body, rolled) {
                        None => None,
                        Some(0) => Some(Effect::Resisted { victim }),
                        Some(amount) => Some(wound(body, amount, rng, (species, fight), out)),
                    }
                }
                SpecialEffect::Drain => {
                    let rolled = three(rng, bound) / if monster { 6 } else { 3 } + 1;
                    match resisted(rng, body, rolled) {
                        None => None,
                        Some(0) => Some(Effect::Resisted { victim }),
                        Some(amount) if monster => {
                            body.drain_magic(amount / 2, rng, (spells, species));
                            Some(Effect::Drained { victim, amount })
                        }
                        // Retail drains a member's realm six times over.
                        Some(amount) => {
                            let maximum = body.mana_maximum(spells).unwrap();
                            let (_, pool) = body.magic.as_mut().unwrap();
                            let amount = pool.drain_realm(realm, amount * 6, &maximum);
                            Some(Effect::Drained { victim, amount })
                        }
                    }
                }
                SpecialEffect::Tire => {
                    let rolled = three(rng, bound) + u32::from(!monster);
                    match resisted(rng, body, rolled) {
                        None => None,
                        Some(0) => Some(Effect::Resisted { victim }),
                        Some(amount) => {
                            Some(Effect::Tired { victim, amount: body.vitals.tire(amount) })
                        }
                    }
                }
                SpecialEffect::Wound => {
                    let rolled = three(rng, bound) / 3 + 1;
                    match definition.resisted {
                        true => match resisted(rng, body, rolled) {
                            None => None,
                            Some(0) => Some(Effect::Resisted { victim }),
                            Some(amount) => Some(wound(body, amount, rng, (species, fight), out)),
                        },
                        false if body.is_dead() => None,
                        false => Some(wound(body, rolled, rng, (species, fight), out)),
                    }
                }
            };
            deed.effects.extend(landed);
        }
    }
}

/// An offensive cloud pulsing on one body it hangs over: the party's on a
/// member, a monster's own on it.
#[derive(Component, Clone, Copy, Debug)]
pub struct CloudHit {
    pub spell: SpellId,
    pub strength: Strength,
}

/// Land each cloud's pulse (retail `TickCombatEffectSlots`), with its
/// indirect source's difficulty rather than a caster's: Death Cloud makes a
/// death save at once; the others hurt, Draining Cloud also draining spell
/// points and stamina, Toxic Cloud also nauseating, poisoning and knocking
/// out for one duration cut three separate saves.
#[allow(clippy::too_many_arguments)]
pub fn land_clouds(
    mut commands: Commands,
    hits: Query<(Entity, &CloudHit, &Strikes, &HitOf)>,
    mut bodies: Query<Body>,
    mut deeds: Query<&mut Deed>,
    party: Single<&PartyEffects>,
    groups: Query<&MonsterGroup>,
    content: If<Res<crate::game::data::Catalogs>>,
    (difficulty, factions, clock): (
        Res<Difficulty>,
        Res<Factions>,
        Res<crate::game::time::GameClock>,
    ),
    fight: Query<Entity, With<crate::game::combat::Battle>>,
    mut rng: ResMut<GameRng>,
    mut notices: MessageWriter<DamageApplied>,
) {
    let (items, species, spells) = (&content.items, &content.species, &content.spells);
    let (difficulty, fight) = (*difficulty, fight.single().ok());
    let rng = &mut *rng;
    for (hit, &CloudHit { spell, strength }, &Strikes(victim), &HitOf(of)) in &hits {
        commands.entity(hit).despawn();
        let Ok(mut body) = bodies.get_mut(victim) else { continue };
        let body = &mut body;
        if body.is_dead() {
            continue;
        }
        let mut deed = deeds.get_mut(of).unwrap();
        let deed = &mut *deed;
        let definition = &spells[spell];
        let Some(realm) = definition.realm else { continue };
        let defence =
            body.defence(realm, (items, species, &party.0), (&groups, &*factions, clock.ms));
        let ruling = (species, difficulty, fight);
        if spell == SpellId::DEATH_CLOUD {
            let effect = if body.death_immune(species) {
                Some(Effect::Resisted { victim })
            } else {
                afflict(
                    rng,
                    body,
                    &defence,
                    (None, 0),
                    (None, strength.against),
                    ruling,
                    &mut deed.improved,
                    (&mut commands, &mut notices),
                )
            };
            deed.effects.extend(effect);
            continue;
        }
        let rolled = magnitude(rng, definition.dice, strength);
        let amount = body::resist(rng, rolled, defence.resistance, defence.level, strength.against);
        deed.effects.push(match amount {
            0 => Effect::Resisted { victim },
            amount => wound(body, amount, rng, (species, fight), (&mut commands, &mut notices)),
        });
        if spell == SpellId::DRAINING_CLOUD && amount > 0 {
            match body.mana_maximum(spells) {
                Some(maximum) => {
                    let (_, pool) = body.magic.as_mut().unwrap();
                    for realm in enumset::EnumSet::<Realm>::all() {
                        pool.drain_realm(realm, amount, &maximum);
                    }
                }
                None => {
                    body.drain_magic(amount / 2, rng, (spells, species));
                }
            }
            body.vitals.tire(amount);
        }
        if spell == SpellId::TOXIC_CLOUD && !body.is_dead() {
            let turns = duration(rng, definition, strength);
            let turns =
                body::resist(rng, turns, defence.resistance, defence.level, strength.against);
            if turns == 0 {
                continue;
            }
            let lasting = Lasting::Rounds(turns.max(2));
            // One duration reduction, followed by three separate saves.
            for condition in [Condition::Nauseated, Condition::Poisoned, Condition::Unconscious] {
                let base = body::condition_save(condition);
                let immune = body.immune(condition, species);
                let (saved, practised) = if immune {
                    (true, None)
                } else {
                    body.saves(rng, &defence, (base, strength.against), difficulty)
                };
                deed.improved.extend(practised.map(|skill| (victim, skill)));
                if saved || body.shrugs(condition, false) {
                    deed.effects.push(Effect::Resisted { victim });
                } else if body.inflict((condition, 0), lasting, defence.side) {
                    deed.effects.push(Effect::Inflicted { victim, condition });
                }
            }
        }
    }
}
