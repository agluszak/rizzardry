//! Level-specific behaviour that retail keeps in per-level code rather than
//! in level data.

use super::triggers::{TriggerEffect, Triggers};

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

    /// Runs before a trigger's action; returning false cancels the run.
    pub fn before_run(self, trigger: &str, triggers: &mut Triggers) -> bool {
        match (self, trigger) {
            (LevelScript::Monastery2, "BELL_BUTTON") => ring_bell(triggers),
            _ => true,
        }
    }
}

/// The bell button rings the bell the dial points at (Whizzardry
/// `Monastery2.cpp`). The third bell, which also breaks the windows through
/// the button's recipients, ends the puzzle: dial and button switch off.
fn ring_bell(triggers: &mut Triggers) -> bool {
    let Some(dial) = triggers.state.find("BELLRINGSWITCH") else { return true };
    let position = triggers.state.trigger(dial).position;
    triggers.effects.push(TriggerEffect::Sound {
        path: format!("data/sound/ambients/mon2bell{}.wav", position + 1),
        volume: 1.0,
        at: None,
    });
    if position == 2 {
        for name in ["BELLRINGSWITCH", "BELL_BUTTON"] {
            if let Some(trigger) = triggers.state.find(name) {
                triggers.state.trigger_mut(trigger).on = false;
            }
        }
    }
    true
}
