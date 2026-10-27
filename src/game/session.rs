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

/// Whether a campaign has begun or is entering its first level; a new one
/// may not start alongside it.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Campaign<'w, 's> {
    begun: Query<'w, 's, (), Or<(With<Party>, With<Destination>)>>,
}

impl Campaign<'_, '_> {
    pub fn begun(&self) -> bool {
        !self.begun.is_empty()
    }
}

/// Validate the whole proposed party and request its first level. Nothing
/// joins until that level commits ([`Entry::NewGame`]), so a failed load
/// leaves the drafts to retry and no half-started campaign.
pub fn begin_adventure(
    drafts: &[CharacterDraft],
    session: &State<Session>,
    campaign: &Campaign,
    commands: &mut Commands,
    assets: &AssetServer,
    destination: &NewGameDestination,
) -> Result<(), &'static str> {
    if *session.get() != Session::Creation {
        return Err("Start a new party from the main menu.");
    }
    if campaign.begun() {
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
    commands.spawn((
        Name::new(destination.path.clone()),
        Destination::new(
            Level::load(&destination.path, assets),
            destination.start.clone(),
            Entry::NewGame(characters),
        ),
    ));
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
    use bevy::ecs::system::SystemState;
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
        let mut operation =
            SystemState::<(Commands, Campaign, Res<AssetServer>, Res<State<Session>>)>::new(
                app.world_mut(),
            );
        let mut begin = |app: &mut App, drafts: &[CharacterDraft]| {
            let (mut commands, campaign, assets, state) =
                operation.get_mut(app.world_mut()).unwrap();
            let result =
                begin_adventure(drafts, &state, &campaign, &mut commands, &assets, &target);
            operation.apply(app.world_mut());
            result
        };
        // The second draft has no name.
        assert!(begin(&mut app, &drafts).is_err());
        assert_eq!(app.world_mut().query::<&Destination>().iter(app.world()).count(), 0);
        drafts[1].name = "Second".into();
        begin(&mut app, &drafts).unwrap();
        assert!(begin(&mut app, &drafts).is_err(), "one campaign at a time");
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
