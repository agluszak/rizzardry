//! What the party holds that is not an item: its purse. Items are things
//! ([`super::items`]); the slots they are worn in are named here.

pub mod omnigun;

use bevy::prelude::*;
use enum_map::Enum;
use enumset::EnumSetType;

pub(crate) fn plugin(app: &mut App) {
    app.add_plugins((super::items::plugin, omnigun::plugin));
}

/// Where an item is worn, in retail's slot order.
#[derive(Debug, Hash, Enum, EnumSetType)]
pub enum EquipSlot {
    Head,
    Misc1,
    Misc2,
    Cloak,
    Torso,
    Hands,
    /// Main hand of the weapon set in use.
    Primary,
    /// Off hand of the weapon set in use.
    Secondary,
    /// The stored weapon set; not worn, so curses do not hold there.
    AlternatePrimary,
    AlternateSecondary,
    Legs,
    Feet,
}

/// Bevy builds a relationship from its holder alone before the slot is set,
/// which needs some slot to start from; no rule reads this.
impl Default for EquipSlot {
    fn default() -> Self {
        Self::Head
    }
}

/// The two slots of the weapon set in use, which attack.
#[derive(Debug, Hash, Enum, EnumSetType)]
pub enum WeaponSlot {
    Primary,
    Secondary,
}

impl WeaponSlot {
    pub fn equip(self) -> EquipSlot {
        match self {
            Self::Primary => EquipSlot::Primary,
            Self::Secondary => EquipSlot::Secondary,
        }
    }

    pub fn other(self) -> Self {
        match self {
            Self::Primary => Self::Secondary,
            Self::Secondary => Self::Primary,
        }
    }
}

impl EquipSlot {
    pub fn is_active(self) -> bool {
        !matches!(self, Self::AlternatePrimary | Self::AlternateSecondary)
    }

    /// The other hand of the same weapon set.
    pub fn partner(self) -> Option<EquipSlot> {
        Some(match self {
            EquipSlot::Primary => EquipSlot::Secondary,
            EquipSlot::Secondary => EquipSlot::Primary,
            EquipSlot::AlternatePrimary => EquipSlot::AlternateSecondary,
            EquipSlot::AlternateSecondary => EquipSlot::AlternatePrimary,
            _ => return None,
        })
    }
}

/// On the party: its purse. What it shares and has in hand are items related
/// to it ([`super::items::Pool`], [`super::items::Hand`]).
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct PartyInventory {
    pub gold: u32,
}
