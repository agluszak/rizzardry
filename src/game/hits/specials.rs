//! Special attacks, a breath or a gaze (retail `ResolveMonsterGroupAttack`):
//! retail's table of what each does, which [`super::prototypes`] decodes
//! into effects.

use crate::content::characters::Realm;
use crate::content::species::SpecialAttackKind;
use crate::game::characters::Condition;

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

/// A special attack's strength, by its maker's level and how much of its
/// stamina it has left: `against` for saves and resistance, `power` the
/// bound its rolls stay under. Retail's doubling of a fifth-entry effect
/// reads the table's first rows, which never hold one.
pub fn strength(level: u8, stamina: crate::game::characters::Pool) -> crate::game::magic::Strength {
    let level = u32::from(level);
    let fresh = |extra: u32| {
        (stamina.current * (level + level.min(15) + extra) / stamina.max.max(1)).max(1)
    };
    crate::game::magic::Strength { power: fresh(5), against: fresh(0), boost: 0 }
}
