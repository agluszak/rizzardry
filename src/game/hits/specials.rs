//! Special attacks, a breath or a gaze or a call: what each kind does is a
//! row of `assets/tables/special_attacks.ron`.

use std::collections::HashMap;
use std::sync::LazyLock;

use serde::Deserialize;

use super::definitions::EffectSpec;
use crate::content::species::SpecialAttackKind;

/// What a kind of special attack does: its row in the table.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SpecialRow {
    /// What it does to each body it lands on; a summoning strikes nobody
    /// and has none.
    pub effects: Vec<EffectSpec>,
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
