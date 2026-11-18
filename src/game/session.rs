//! The one-time boundary that starts a campaign.
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
pub fn begin_adventure(world: &mut World, drafts: &[Entity]) -> Result<(), &'static str> {
    use super::characters::draft::{Allotted, Forced, Points, unready};
    let mut entering = world.query_filtered::<(), With<Destination>>();
    if world.contains_resource::<Party>() || entering.iter(world).next().is_some() {
        return Err("A campaign has already begun.");
    }
    if !(1..=6).contains(&drafts.len()) {
        return Err("Create between one and six adventurers.");
    }
    let mut made = world.query::<(
        &super::characters::Identity,
        (&Points, &Forced, &Allotted),
        (&super::actors::Abilities, &super::characters::Training, &super::actors::UsableSkills),
    )>();
    let mut names = Vec::new();
    for &draft in drafts {
        let (identity, spent, sheet) =
            made.get(world, draft).map_err(|_| "A party member is no longer available.")?;
        if let Some(why) = unready(identity, spent, sheet) {
            return Err(why);
        }
        if super::characters::draft::nickname_taken(&identity.name, names.iter().copied()) {
            return Err("A character with that nickname already exists.");
        }
        names.push(identity);
    }
    // The chosen are no longer the session's: they outlive it until they join.
    for &draft in drafts {
        world.entity_mut(draft).remove::<super::characters::draft::MadeIn>();
    }
    let destination = world.resource::<NewGameDestination>();
    let (path, start) = (destination.path.clone(), destination.start.clone());
    let level = Level::load(&path, world.resource::<AssetServer>());
    world.spawn((Name::new(path), Destination::new(level, start, Entry::NewGame(drafts.to_vec()))));
    Ok(())
}
