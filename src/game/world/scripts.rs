//! Level-specific behaviour that retail keeps in per-level code rather than
//! in level data.

use crate::content::level::LevelGeometry;
use bevy::prelude::*;

use super::triggers::{DialState, TriggerEffect, TriggerState};

/// The level whose special behaviour applies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LevelScript {
    #[default]
    None,
    /// The upper monastery: its bells.
    Monastery2,
}

impl LevelScript {
    /// The script for a level, by its location code (e.g. `MO2`).
    pub fn for_location(code: &str) -> Self {
        match code {
            "MO2" => LevelScript::Monastery2,
            _ => LevelScript::None,
        }
    }

    /// Resolve this level's exceptional behavior once, while identities are
    /// allocated. Ordinary trigger execution does not dispatch by level/name.
    pub fn bind(self, commands: &mut Commands, geometry: &LevelGeometry, devices: &[Entity]) {
        if self != Self::Monastery2 {
            return;
        }
        let find = |name: &str| {
            geometry
                .triggers
                .iter()
                .rposition(|t| t.name.eq_ignore_ascii_case(name))
                .map(|i| devices[i])
        };
        if let (Some(button), Some(dial)) = (find("BELL_BUTTON"), find("BELLRINGSWITCH")) {
            commands.entity(button).insert(BellButton { dial });
        }
    }
}

/// The bell button's actual dial. Grouping is not lifetime ownership.
#[derive(Component, Clone, Copy, Debug)]
pub struct BellButton {
    pub dial: Entity,
}

impl BellButton {
    /// The selected bell rings before the button's ordinary recipients run.
    /// The third bell also disables the button and dial for good.
    pub fn ring(
        &self,
        button: Entity,
        dials: &Query<&DialState>,
        triggers: &mut Query<&mut TriggerState>,
    ) -> TriggerEffect {
        let position = dials.get(self.dial).unwrap().position;
        if position == 2 {
            triggers.get_mut(self.dial).unwrap().on = false;
            triggers.get_mut(button).unwrap().on = false;
        }
        TriggerEffect::BellRang(position + 1)
    }
}
