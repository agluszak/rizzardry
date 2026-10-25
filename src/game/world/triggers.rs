//! Triggers: what using a prop, or another trigger naming it, sets off.
//!
//! After Whizzardry `Trigger::Run`, `SelectAction` and `CommitActionResult`.
//! A run selects an action (initial, alternate or fallback), lets the level
//! script veto it, performs it and, unless the action failed, commits:
//! the trigger's sound, its recipients in order (synchronously), its
//! location variable and its `fired` flag. Effects that presentation shows
//! (sounds, camera shakes, messages) are returned, not played here. See
//! `docs/research/trigger-actions.md`.

use std::collections::HashMap;

use bevy::prelude::*;

use bevy::ecs::system::SystemParam;

use super::level::{Level, PropPart};
use super::props::{Prop, PropMotion};
use super::scripts::LevelScript;
use crate::content::level::{LevelGeometry, StateChange, TriggerDefinition};

/// Retail trigger action codes performed so far.
pub mod action {
    pub const NOTHING: u32 = 0x00;
    pub const DOOR_OPEN: u32 = 0x01;
    pub const DOOR_TOGGLE: u32 = 0x02;
    pub const TOGGLE: u32 = 0x03;
    pub const RELAY: u32 = 0x0f;
    pub const DOOR_TOGGLE_WITH_KEY: u32 = 0x2c;
    pub const START: u32 = 0x32;
    pub const STOP: u32 = 0x33;
    pub const SHAKE: u32 = 0x39;
    pub const DIAL: u32 = 0x40;
}

/// Recipients naming each other in a loop stop here.
const MAX_DEPTH: usize = 16;

/// The live state of a level's triggers and location variables, on the
/// level root. Triggers are indexed as in [`LevelGeometry::triggers`].
#[derive(Component, Debug)]
pub struct LevelState {
    triggers: Vec<TriggerState>,
    /// By name; retail looks names up case-insensitively, newest first.
    index: HashMap<String, usize>,
    variables: HashMap<String, bool>,
    pub script: LevelScript,
}

#[derive(Clone, Debug)]
pub struct TriggerState {
    /// Cleared to switch a trigger off for good (retail `ON`).
    pub on: bool,
    pub fired: bool,
    use_alternate: bool,
    /// It waits for its prop's animation to finish (retail `RUNNING`).
    awaits_animation: bool,
    /// The dial position or door state (retail `state_index`).
    pub position: usize,
}

impl LevelState {
    pub fn new(geometry: &LevelGeometry, script: LevelScript) -> Self {
        let mut state = LevelState {
            triggers: vec![
                TriggerState {
                    on: true,
                    fired: false,
                    use_alternate: false,
                    awaits_animation: false,
                    position: 0,
                };
                geometry.triggers.len()
            ],
            index: HashMap::new(),
            variables: HashMap::new(),
            script,
        };
        for (i, trigger) in geometry.triggers.iter().enumerate() {
            state.index.insert(trigger.name.to_uppercase(), i);
            if let (Some((name, _)), Some(value)) = (&trigger.state_change, trigger.initial_state) {
                state.variables.entry(name.clone()).or_insert(value);
            }
        }
        // A dial's position variables: `<state>0` is set, the rest clear.
        for prop in &geometry.props {
            let Some(trigger) = prop.trigger.as_ref().and_then(|name| state.find(name)) else {
                continue;
            };
            let definition = &geometry.triggers[trigger];
            if let (action::DIAL, Some((name, _))) = (definition.initial, &definition.state_change)
            {
                for position in 0..prop.slots.len() {
                    state.variables.entry(format!("{name}{position}")).or_insert(position == 0);
                }
            }
        }
        state
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.index.get(&name.to_uppercase()).copied()
    }

    pub fn trigger(&self, index: usize) -> &TriggerState {
        &self.triggers[index]
    }

    pub fn trigger_mut(&mut self, index: usize) -> &mut TriggerState {
        &mut self.triggers[index]
    }

    pub fn variable(&self, name: &str) -> bool {
        self.variables.get(&name.to_uppercase()).copied().unwrap_or(false)
    }

    pub fn set_variable(&mut self, name: &str, value: bool) {
        self.variables.insert(name.to_uppercase(), value);
    }
}

/// What a trigger's prop can do: move along its path.
pub struct TriggerProp<'a> {
    pub motion: &'a mut PropMotion,
    pub last_frame: f32,
    pub wraps: bool,
    pub slots: &'a [u16],
    /// Where its sounds play.
    pub position: Vec3,
}

impl TriggerProp<'_> {
    fn playing(&self) -> bool {
        self.motion.frame != self.motion.target
    }

    /// Retail `SetRepresentationActive(!playing)`: stop a playing animation,
    /// or play it: from the start for a wrapping one, otherwise reversing
    /// the last play.
    fn toggle(&mut self) {
        if self.playing() {
            self.motion.target = self.motion.frame;
        } else {
            self.start();
        }
    }

    fn start(&mut self) {
        if self.wraps {
            self.motion.frame = 0.0;
            self.motion.target = self.last_frame;
        } else {
            self.motion.target =
                if self.motion.frame >= self.last_frame { 0.0 } else { self.last_frame };
        }
    }

    /// Advance to the next segment between slots and return its index;
    /// the last slot only ends the last segment (retail
    /// `AdvanceAnimationSegment`).
    fn advance_segment(&mut self, current: usize) -> usize {
        if self.slots.len() < 3 {
            return 0;
        }
        let next = (current + 1) % (self.slots.len() - 1);
        self.motion.frame = f32::from(self.slots[next]);
        self.motion.target = f32::from(self.slots[next + 1]);
        next
    }
}

/// Something presentation shows after triggers ran.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum TriggerEffect {
    /// A retail sound file, at a place or heard everywhere.
    Sound {
        path: String,
        volume: f32,
        at: Option<Vec3>,
    },
    Shake {
        seconds: f32,
        intensity: f32,
        fade_out: bool,
    },
    /// A message from the level's message file.
    Message(i32),
    /// The party used something and nothing came of it.
    NothingHappened,
    Locked,
}

/// Running triggers: their definitions, live state and props.
pub struct Triggers<'a> {
    pub definitions: &'a [TriggerDefinition],
    pub state: &'a mut LevelState,
    /// Props by the index of the trigger they carry.
    pub props: HashMap<usize, TriggerProp<'a>>,
    pub effects: Vec<TriggerEffect>,
}

impl Triggers<'_> {
    /// Whether the party may use a trigger at all (retail picking).
    pub fn usable(&self, index: usize) -> bool {
        let (definition, state) = (&self.definitions[index], &self.state.triggers[index]);
        definition.usable && !(definition.once && state.fired)
    }

    /// Run a trigger by name. Returns whether its action committed.
    pub fn run(&mut self, name: &str) -> bool {
        match self.state.find(name) {
            Some(index) => self.run_index(index, 0),
            None => false,
        }
    }

    fn run_index(&mut self, index: usize, depth: usize) -> bool {
        if depth > MAX_DEPTH {
            warn!("trigger {} recurses too deeply", self.definitions[index].name);
            return false;
        }
        let definitions = self.definitions;
        let definition = &definitions[index];
        let Some((action, use_index)) = self.select(index) else { return false };
        let script = self.state.script;
        if !script.before_run(&definition.name, self) {
            return false;
        }
        let mut changes_state = true;
        match action {
            action::NOTHING => {}
            action::DOOR_OPEN | action::DOOR_TOGGLE | action::DOOR_TOGGLE_WITH_KEY => {
                if definition.door.is_some_and(|door| door.locked) {
                    self.effects.push(TriggerEffect::Locked);
                    return false;
                }
                let Some(prop) = self.props.get_mut(&index) else { return false };
                if prop.playing() {
                    return false;
                }
                let state = &mut self.state.triggers[index];
                if action == action::DOOR_OPEN {
                    if state.position == 1 {
                        return false;
                    }
                    prop.motion.target = prop.last_frame;
                    state.position = 1;
                } else {
                    prop.motion.target = if state.position == 0 { prop.last_frame } else { 0.0 };
                    state.position ^= 1;
                }
                state.awaits_animation = true;
            }
            action::TOGGLE | action::START | action::STOP => {
                let Some(prop) = self.props.get_mut(&index) else { return false };
                match action {
                    action::TOGGLE => prop.toggle(),
                    action::START if !prop.playing() => prop.start(),
                    action::STOP if prop.playing() => prop.motion.target = prop.motion.frame,
                    _ => {}
                }
                let state = &mut self.state.triggers[index];
                state.position ^= 1;
                state.awaits_animation = true;
            }
            action::RELAY => {
                let mut found = false;
                for recipient in &definition.recipients {
                    if let Some(target) = self.state.find(recipient) {
                        found = true;
                        self.run_index(target, depth + 1);
                    }
                }
                if !found {
                    return false;
                }
            }
            action::SHAKE => {
                let [intensity, _, duration] = definition.data;
                self.effects.push(TriggerEffect::Shake {
                    seconds: if duration == -1 {
                        1.0
                    } else {
                        duration.unsigned_abs() as f32 / 1000.0
                    },
                    intensity: if intensity == -1 { 800.0 } else { intensity as f32 },
                    fade_out: duration < -1,
                });
            }
            action::DIAL => {
                let Some(prop) = self.props.get_mut(&index) else { return false };
                let old = self.state.triggers[index].position;
                let new = prop.advance_segment(old);
                self.state.triggers[index].position = new;
                if let Some((name, _)) = &definition.state_change {
                    self.state.set_variable(&format!("{name}{old}"), false);
                    self.state.set_variable(&format!("{name}{new}"), true);
                }
                changes_state = false;
            }
            other => {
                debug!("trigger {}: action {other:#x} is not implemented", definition.name);
                return false;
            }
        }
        self.commit(index, use_index, changes_state, depth);
        true
    }

    /// Retail `SelectAction`: the action this run performs, and which of
    /// initial (0), alternate (1) and fallback (2) it is.
    fn select(&mut self, index: usize) -> Option<(u32, usize)> {
        let definition = &self.definitions[index];
        let running = self.state.triggers[index].awaits_animation
            && self.props.get(&index).is_some_and(TriggerProp::playing);
        let state = &self.state.triggers[index];
        if !state.on
            || (definition.once && state.fired)
            || (running && definition.initial != action::SHAKE)
        {
            return None;
        }
        if !definition.required_states.iter().all(|name| self.state.variable(name)) {
            return (definition.fallback != 0).then_some((definition.fallback, 2));
        }
        let state = &mut self.state.triggers[index];
        if !state.use_alternate {
            state.use_alternate = definition.alternate != 0;
            Some((definition.initial, 0))
        } else {
            state.use_alternate = !definition.alternate_toggles;
            Some((definition.alternate, 1))
        }
    }

    /// Retail `CommitActionResult`.
    fn commit(&mut self, index: usize, use_index: usize, changes_state: bool, depth: usize) {
        let definitions = self.definitions;
        let definition = &definitions[index];
        if let Some(sound) = &definition.sound {
            let at = self.props.get(&index).map(|prop| prop.position);
            self.effects.push(TriggerEffect::Sound {
                path: sound.path.clone(),
                volume: sound.volume,
                at,
            });
        }
        if definition.fire_linked {
            for recipient in &definition.recipients {
                if let Some(target) = self.state.find(recipient) {
                    self.run_index(target, depth + 1);
                }
            }
        }
        if changes_state && let Some((name, change)) = &definition.state_change {
            let value = match change {
                StateChange::Set => true,
                StateChange::Clear => false,
                StateChange::Toggle => !self.state.variable(name),
            };
            self.state.set_variable(name, value);
        }
        if definition.shows_message && definition.data[use_index] >= 0 {
            self.effects.push(TriggerEffect::Message(definition.data[use_index]));
        }
        self.state.triggers[index].fired = true;
    }
}

/// The current level's triggers, ready to run.
#[derive(SystemParam)]
pub struct LevelTriggers<'w, 's> {
    levels: Query<'w, 's, (&'static Level, &'static mut LevelState)>,
    geometry: Res<'w, Assets<LevelGeometry>>,
    props: Query<'w, 's, (&'static Prop, &'static mut PropMotion, &'static Children)>,
    parts: Query<'w, 's, &'static GlobalTransform, With<PropPart>>,
    effects: MessageWriter<'w, TriggerEffect>,
}

impl LevelTriggers<'_, '_> {
    /// Run triggers with `run`, then send their effects to presentation.
    /// Nothing runs while no level is built.
    pub fn with<R>(&mut self, run: impl FnOnce(&mut Triggers) -> R) -> Option<R> {
        let Ok((level, state)) = self.levels.single_mut() else { return None };
        let geometry = self.geometry.get(&level.geometry)?;
        let state = state.into_inner();
        let mut props = HashMap::new();
        for (prop, motion, children) in &mut self.props {
            let Some(index) = prop.trigger.as_ref().and_then(|name| state.find(name)) else {
                continue;
            };
            // Sounds play from the middle of the prop's parts.
            let parts: Vec<Vec3> = children
                .iter()
                .filter_map(|c| self.parts.get(c).ok())
                .map(|t| t.translation())
                .collect();
            let position = parts.iter().sum::<Vec3>() / parts.len().max(1) as f32;
            props.insert(
                index,
                TriggerProp {
                    motion: motion.into_inner(),
                    last_frame: prop.last_frame as f32,
                    wraps: prop.wraps,
                    slots: &prop.slots,
                    position,
                },
            );
        }
        let mut triggers =
            Triggers { definitions: &geometry.triggers, state, props, effects: Vec::new() };
        let result = run(&mut triggers);
        self.effects.write_batch(triggers.effects);
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::level::{DoorLock, PropModel};

    fn trigger(name: &str, initial: u32) -> TriggerDefinition {
        TriggerDefinition {
            name: name.into(),
            usable: false,
            once: false,
            fire_linked: false,
            initial,
            alternate: 0,
            fallback: 0,
            alternate_toggles: false,
            recipients: vec![],
            data: [-1; 3],
            sound: None,
            shows_message: false,
            required_states: vec![],
            state_change: None,
            initial_state: None,
            range: (0.0, 10.0),
            door: None,
        }
    }

    fn prop(trigger: &str, last_frame: f32, wraps: bool, slots: &[u16]) -> (PropModel, f32) {
        let model = PropModel {
            name: trigger.to_lowercase(),
            collidable: false,
            face_camera: false,
            frames_per_second: 10.0,
            parts: vec![],
            wraps,
            slots: slots.to_vec(),
            trigger: Some(trigger.into()),
        };
        (model, last_frame)
    }

    /// A level's triggers with their props held still or finished.
    struct Level {
        geometry: LevelGeometry,
        state: LevelState,
        last_frames: Vec<f32>,
        motions: Vec<PropMotion>,
    }

    impl Level {
        fn new(
            triggers: Vec<TriggerDefinition>,
            props: Vec<(PropModel, f32)>,
            script: LevelScript,
        ) -> Self {
            let (props, last_frames): (Vec<_>, Vec<_>) = props.into_iter().unzip();
            let motions = props.iter().map(|_| PropMotion::default()).collect();
            let geometry = LevelGeometry { triggers, props, ..default() };
            let state = LevelState::new(&geometry, script);
            Level { geometry, state, last_frames, motions }
        }

        /// Run a trigger as the party; returns whether it committed and the
        /// effects.
        fn run(&mut self, name: &str) -> (bool, Vec<TriggerEffect>) {
            let mut props = HashMap::new();
            for ((model, motion), &last_frame) in
                self.geometry.props.iter().zip(&mut self.motions).zip(&self.last_frames)
            {
                let index = self.state.find(model.trigger.as_ref().unwrap()).unwrap();
                props.insert(
                    index,
                    TriggerProp {
                        motion,
                        last_frame,
                        wraps: model.wraps,
                        slots: &model.slots,
                        position: Vec3::ZERO,
                    },
                );
            }
            let mut triggers = Triggers {
                definitions: &self.geometry.triggers,
                state: &mut self.state,
                props,
                effects: vec![],
            };
            let committed = triggers.run(name);
            (committed, triggers.effects)
        }

        fn motion(&self, trigger: &str) -> &PropMotion {
            let index =
                self.geometry.props.iter().position(|p| p.trigger.as_deref() == Some(trigger));
            &self.motions[index.unwrap()]
        }

        /// Let every animation finish.
        fn settle(&mut self) {
            for motion in &mut self.motions {
                motion.frame = motion.target;
            }
        }
    }

    #[test]
    fn a_door_opens_then_toggles_and_a_locked_one_stays_shut() {
        let mut door = trigger("DOOR", action::DOOR_OPEN);
        door.alternate = action::DOOR_TOGGLE;
        let mut locked = trigger("LOCKED", action::DOOR_OPEN);
        locked.door = Some(DoorLock { locked: true, key: None });
        let mut level = Level::new(
            vec![door, locked],
            vec![prop("DOOR", 15.0, false, &[]), prop("LOCKED", 15.0, false, &[])],
            LevelScript::None,
        );
        assert!(level.run("DOOR").0);
        assert_eq!(level.motion("DOOR").target, 15.0);
        // Busy until the door finishes opening.
        assert!(!level.run("DOOR").0);
        level.settle();
        assert!(level.run("DOOR").0);
        assert_eq!(level.motion("DOOR").target, 0.0);
        level.settle();
        assert!(level.run("DOOR").0);
        assert_eq!(level.motion("DOOR").target, 15.0);
        assert_eq!(level.run("LOCKED"), (false, vec![TriggerEffect::Locked]));
        assert_eq!(level.motion("LOCKED").target, 0.0);
    }

    /// The upper monastery's bells, as authored (`docs/research/trigger-actions.md`).
    fn bells() -> Level {
        let mut dial = trigger("BELLRINGSWITCH", action::DIAL);
        dial.usable = true;
        dial.state_change = Some(("RING".into(), StateChange::Set));
        dial.initial_state = Some(false);
        let mut button = trigger("BELL_BUTTON", action::TOGGLE);
        button.usable = true;
        button.fire_linked = true;
        button.recipients = ["BELLTRIGGER01", "BELLTRIGGERC", "BELL_ACTIVATE_2", "BELL_ACTIVATE_3"]
            .map(String::from)
            .to_vec();
        let mut windows = trigger("BELLTRIGGER01", action::NOTHING);
        windows.once = true;
        windows.fire_linked = true;
        windows.required_states = vec!["RING2".into()];
        windows.recipients = vec!["GLASS1".into(), "MONACAMSHAKE".into()];
        let relay = |name: &str, ring: &str, recipients: &[&str]| {
            let mut relay = trigger(name, action::RELAY);
            relay.fire_linked = true;
            relay.required_states = vec![ring.into()];
            relay.recipients = recipients.iter().map(|r| r.to_string()).collect();
            relay
        };
        let mut shake = trigger("MONACAMSHAKE", action::SHAKE);
        shake.data = [90, 40, -2400];
        Level::new(
            vec![
                dial,
                button,
                windows,
                relay("BELLTRIGGERC", "RING0", &["BELL_1"]),
                relay("BELL_ACTIVATE_2", "RING1", &["BELL_2"]),
                relay("BELL_ACTIVATE_3", "RING2", &["BELL_3", "MONACAMSHAKE"]),
                trigger("BELL_1", action::TOGGLE),
                trigger("BELL_2", action::TOGGLE),
                trigger("BELL_3", action::TOGGLE),
                trigger("GLASS1", action::TOGGLE),
                shake,
            ],
            vec![
                prop("BELLRINGSWITCH", 20.0, false, &[0, 1, 10, 20]),
                prop("BELL_BUTTON", 11.0, false, &[]),
                prop("BELL_1", 36.0, true, &[]),
                prop("BELL_2", 36.0, true, &[]),
                prop("BELL_3", 36.0, true, &[]),
                prop("GLASS1", 2.0, false, &[]),
            ],
            LevelScript::Monastery2,
        )
    }

    fn bell_sound(n: u32) -> TriggerEffect {
        TriggerEffect::Sound {
            path: format!("data/sound/ambients/mon2bell{n}.wav"),
            volume: 1.0,
            at: None,
        }
    }

    #[test]
    fn each_dial_position_rings_its_own_bell() {
        let mut level = bells();
        assert!(level.state.variable("RING0"));
        let (committed, effects) = level.run("BELL_BUTTON");
        assert!(committed);
        assert_eq!(effects, [bell_sound(1)]);
        assert_eq!(level.motion("BELL_1").target, 36.0);
        assert_eq!(level.motion("BELL_2").target, 0.0);
        level.settle();
        assert!(level.run("BELLRINGSWITCH").0);
        assert!(!level.state.variable("RING0") && level.state.variable("RING1"));
        assert_eq!(
            (level.motion("BELLRINGSWITCH").frame, level.motion("BELLRINGSWITCH").target),
            (1.0, 10.0)
        );
        level.settle();
        assert_eq!(level.run("BELL_BUTTON").1, [bell_sound(2)]);
        assert_eq!(level.motion("BELL_2").target, 36.0);
        assert_eq!(level.motion("GLASS1").target, 0.0, "the windows hold until the third bell");
    }

    #[test]
    fn the_third_bell_breaks_the_windows_once_and_ends_the_puzzle() {
        let mut level = bells();
        level.run("BELLRINGSWITCH");
        level.settle();
        level.run("BELLRINGSWITCH");
        level.settle();
        let (committed, effects) = level.run("BELL_BUTTON");
        assert!(committed);
        assert_eq!(effects[0], bell_sound(3));
        let shake = TriggerEffect::Shake { seconds: 2.4, intensity: 90.0, fade_out: true };
        assert!(effects.contains(&shake));
        assert_eq!(level.motion("GLASS1").target, 2.0);
        assert_eq!(level.motion("BELL_3").target, 36.0);
        level.settle();
        // Dial and button are off for good; the windows trigger has fired.
        assert_eq!(level.run("BELL_BUTTON"), (false, vec![]));
        assert_eq!(level.run("BELLRINGSWITCH"), (false, vec![]));
        let windows = level.state.find("BELLTRIGGER01").unwrap();
        assert!(level.state.trigger(windows).fired);
    }

    #[test]
    fn a_swinging_bell_is_not_restarted() {
        let mut level = bells();
        level.run("BELL_BUTTON");
        level.motions.iter_mut().for_each(|m| m.frame = (m.target / 2.0).floor());
        // The button finished; the bell is halfway.
        let button = level
            .geometry
            .props
            .iter()
            .position(|p| p.trigger.as_deref() == Some("BELL_BUTTON"))
            .unwrap();
        level.motions[button].frame = level.motions[button].target;
        let bell = level.motion("BELL_1").frame;
        assert_eq!(level.run("BELL_BUTTON").1, [bell_sound(1)]);
        assert_eq!(level.motion("BELL_1").frame, bell);
        assert_eq!(level.motion("BELL_1").target, 36.0);
    }
}
