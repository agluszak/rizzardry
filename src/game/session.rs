//! The frontend and the one-time boundary that starts a campaign.
use super::characters::creation::CharacterDraft;
use super::party::roster::Party;
use super::world::level::{Destination, Entry, Level};
use bevy::prelude::*;

#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Session {
    #[default]
    MainMenu,
    Creation,
    Playing,
}

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
    if *world.resource::<State<Session>>().get() != Session::Creation {
        return Err("Start a new party from the main menu.");
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{
        characters::{Attribute, Skill},
        level::LevelDefinition,
        ui::TextTable,
    };
    use bevy_landmass::NavMesh3d;

    #[test]
    fn new_adventure_validates_all_members_and_requests_its_level_only_once() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), bevy::state::app::StatesPlugin))
            .init_asset::<LevelDefinition>()
            .init_asset::<NavMesh3d>()
            .init_asset::<TextTable>()
            .insert_state(Session::Creation);
        let mut drafts = vec![CharacterDraft::default(), CharacterDraft::default()];
        drafts[0].name = "First".into();
        drafts[0].adjust_attribute(Attribute::Vitality, true);
        drafts[0].adjust_skill(Skill::Sword, true);
        for draft in &mut drafts {
            crate::game::characters::creation::complete_allocations(draft);
        }
        let expected = drafts[0].preview();
        let target = NewGameDestination {
            path: "levels/monastery/monastery1.pvl".into(),
            start: super::super::party::body::PartyStart {
                entrance: Some("MO105".into()),
                ..default()
            },
        };
        app.insert_resource(target);
        let begin =
            |app: &mut App, drafts: &[CharacterDraft]| begin_adventure(app.world_mut(), drafts);
        // The second draft has no name.
        assert!(begin(&mut app, &drafts).is_err());
        assert_eq!(app.world_mut().query::<&Destination>().iter(app.world()).count(), 0);
        drafts[1].name = "Second".into();
        // Two starts in one go, as from a double click: only the first is
        // taken.
        let (first, second) = (begin(&mut app, &drafts), begin(&mut app, &drafts));
        assert!(first.is_ok() && second.is_err(), "one campaign at a time");
        let arrival = app.world_mut().query::<&Destination>().single(app.world()).unwrap();
        assert_eq!(
            arrival.start.entrance.as_deref(),
            Some("MO105"),
            "creation preserves the requested entrance"
        );
        let Entry::NewGame(party) = &arrival.entry else { panic!("a new game") };
        assert_eq!(party.len(), 2);
        assert_eq!(party[0].0, expected);
        assert_eq!(party[0].1.0, drafts[0].spells().clone());
        assert!(
            app.world_mut().query::<&Party>().iter(app.world()).next().is_none(),
            "nobody joins before the level commits"
        );
    }
}
