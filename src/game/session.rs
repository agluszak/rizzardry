//! The one-time boundary that starts a campaign.
use super::characters::creation::CharacterDraft;
use super::party::roster::Party;
use super::world::level::{Destination, Entry, Level};
use bevy::prelude::*;

#[derive(Resource)]
pub struct NewGameDestination {
    pub path: String,
    pub start: super::party::body::PartyStart,
}

/// Validate the whole proposed party and, unless a campaign has begun or is
/// already entering its first level, request that level. The check and the
/// request are one step, so of two starts only the first is accepted.
/// Nothing joins until the level commits ([`Entry::NewGame`]), so a failed
/// load leaves the drafts to retry and no half-started campaign.
pub fn begin_adventure(world: &mut World, drafts: &[CharacterDraft]) -> Result<(), &'static str> {
    let mut begun = world.query_filtered::<(), Or<(With<Party>, With<Destination>)>>();
    if begun.iter(world).next().is_some() {
        return Err("A campaign has already begun.");
    }
    if !(1..=6).contains(&drafts.len()) {
        return Err("Create between one and six adventurers.");
    }
    let characters = drafts
        .iter()
        .map(|draft| {
            draft
                .finish()
                .map(|character| (character, crate::game::magic::LearnedSpells(draft.spells())))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let destination = world.resource::<NewGameDestination>();
    let (path, start) = (destination.path.clone(), destination.start.clone());
    let level = Level::load(&path, world.resource::<AssetServer>());
    world.spawn((Name::new(path), Destination::new(level, start, Entry::NewGame(characters))));
    Ok(())
}
