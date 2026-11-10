//! Special attacks, a breath or a gaze or a call: what each kind does is a
//! row of `assets/tables/special_attacks.ron`, which
//! [`super::definitions`] turns into effects.

use std::collections::HashMap;
use std::sync::LazyLock;

use serde::Deserialize;

use crate::content::characters::Realm;
use crate::content::species::SpecialAttackKind;
use crate::game::characters::Condition;

/// What one of a special attack's effects does to a victim.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum SpecialEffect {
    /// A condition, for rounds rolled by its strength.
    Inflict(Condition),
    /// Damage.
    Harm,
    /// Spell points: a member's realm six times over, a monster's budget
    /// by a sixth.
    Drain,
    /// Stamina.
    Tire,
    /// Damage, resisted unless its row says otherwise.
    Wound,
}

/// What a kind of special attack does: its row in the table.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SpecialRow {
    /// The realm victims resist it by; a summoning strikes nobody and has
    /// none.
    pub realm: Option<Realm>,
    /// What it does to each victim: the first is escaped one time in ten,
    /// the second one in two.
    pub effects: Vec<SpecialEffect>,
    /// Resistance does not cut its wound.
    pub unresisted: bool,
    /// A silence stops it.
    pub silenced: bool,
    /// The species it calls a group of, rather than striking.
    pub summons: Option<u32>,
}

static SPECIALS: LazyLock<HashMap<SpecialAttackKind, SpecialRow>> = LazyLock::new(|| {
    ron::from_str(include_str!("../../../assets/tables/special_attacks.ron"))
        .expect("assets/tables/special_attacks.ron is a table of special attacks")
});

/// What a kind of special attack does, from the table the game ships.
pub fn special(kind: SpecialAttackKind) -> &'static SpecialRow {
    SPECIALS.get(&kind).expect("every kind of special attack has a row")
}

/// The kind a member's breath is: a Water blast (retail's entry 30).
pub const BREATH: SpecialAttackKind = SpecialAttackKind::WoundWater;

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every kind a record can name has a row, and the rows say what the
    /// original's tables say of a few of them.
    #[test]
    fn every_kind_has_a_row_and_some_are_as_the_original_has_them() {
        use SpecialAttackKind as K;
        use SpecialEffect::*;
        let kinds = (0..=255).filter_map(K::from_record);
        assert_eq!(kinds.map(special).count(), 31);
        let gaze = special(K::FearAndInsanityMental);
        assert_eq!(gaze.effects, [Inflict(Condition::Afraid), Inflict(Condition::Insane)]);
        assert_eq!((gaze.realm, gaze.silenced), (Some(Realm::Mental), true));
        assert!(special(K::InsanityAndWoundMental).unresisted);
        assert!(!special(K::WoundFire).unresisted);
        let call = special(K::SummonsSpecies149);
        assert_eq!((call.summons, call.silenced, call.effects.len()), (Some(149), true, 0));
        assert!(!special(K::SummonsSpecies12).silenced);
        assert_eq!(special(BREATH).effects, [Wound]);
    }
}
