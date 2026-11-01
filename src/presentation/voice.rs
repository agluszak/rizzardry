//! Shared voice paths and the marker for developer speech.

use bevy::prelude::*;

/// Shared PC voice/quote identity used by creation and live party remarks.
pub fn pc_voice_stem(
    gender: crate::content::characters::Gender,
    personality: crate::content::characters::Personality,
    voice: crate::content::characters::Voice,
) -> String {
    use crate::content::characters::{Gender, Voice};
    let gender = if gender == Gender::Male { "m" } else { "f" };
    let personality = ["aggr", "intell", "burly", "chaos", "cun", "ecc", "kind", "laid", "loner"]
        [personality as usize];
    let voice = if voice == Voice::First { 1 } else { 2 };
    format!("{gender}_{personality}{voice}0")
}

pub fn pc_voice_path(stem: &str, event: u16) -> String {
    format!("retail://data/sound/pcs/{stem}/{stem}_{event:03}.mp3")
}

/// Developer speech also holds off party remarks.
#[derive(Component, Debug)]
pub struct Speech;

/// The retail file of a voice line: `data/sound/npcs/<speaker>/<line>.mp3`,
/// where the speaker is the line name without its quote number. Retail
/// scripts ask for `.wav` and fall back to the shipped `.mp3`.
pub fn voice_path(line: &str) -> String {
    let line = line.to_lowercase();
    let speaker = line.rsplit_once('_').map_or(line.as_str(), |(speaker, _)| speaker);
    format!("retail://data/sound/npcs/{speaker}/{line}.mp3")
}
