//! The frontend and the one-time boundary that starts a campaign.
use super::characters::creation::CharacterDraft;
use super::party::{
    formation::Formation,
    roster::{PartyRoster, recruit_character},
};
use super::world::level::{Destination, Level};
use bevy::prelude::*;

#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Session {
    #[default]
    MainMenu,
    Creation,
    Playing,
}

#[derive(Resource)]
pub struct NewGameDestination(pub String);

/// Validate the whole proposed party before creating any actors or requesting a
/// level. Taking the drafts only on success makes retry and cancellation safe.
pub fn begin_adventure(
    drafts: &mut Vec<CharacterDraft>,
    session: &State<Session>,
    commands: &mut Commands,
    roster: &mut PartyRoster,
    formation: &mut Formation,
    assets: &AssetServer,
    destination: &NewGameDestination,
    next: &mut NextState<Session>,
) -> Result<(), &'static str> {
    if *session.get() != Session::Creation {
        return Err("Start a new party from the main menu.");
    }
    if !roster.0.is_empty() {
        return Err("A campaign has already begun.");
    }
    if !(1..=6).contains(&drafts.len()) {
        return Err("Create between one and six adventurers.");
    }
    let characters = drafts.iter().map(CharacterDraft::finish).collect::<Result<Vec<_>, _>>()?;
    for character in characters {
        recruit_character(commands, roster, formation, character);
    }
    drafts.clear();
    commands.spawn((
        Name::new(destination.0.clone()),
        Destination::new(Level::load(&destination.0, assets)),
    ));
    next.set(Session::Playing);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{
        characters::{Attribute, Skill},
        level::LevelGeometry,
        ui::TextTable,
    };
    use crate::game::characters::{Character, Vitals};
    use bevy::ecs::system::SystemState;
    use bevy_landmass::NavMesh3d;

    #[test]
    fn new_adventure_validates_all_members_before_committing_and_starts_only_once() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), bevy::state::app::StatesPlugin))
            .init_asset::<LevelGeometry>()
            .init_asset::<NavMesh3d>()
            .init_asset::<TextTable>()
            .init_resource::<PartyRoster>()
            .init_resource::<Formation>()
            .insert_state(Session::Creation);
        let mut drafts = vec![CharacterDraft::default(), CharacterDraft::default()];
        drafts[0].name = "First".into();
        drafts[0].adjust_attribute(Attribute::Vitality, true);
        drafts[0].adjust_skill(Skill::Sword, true);
        let expected = drafts[0].preview();
        let target = NewGameDestination("levels/monastery/monastery1.pvl".into());
        let mut operation = SystemState::<(
            Commands,
            ResMut<PartyRoster>,
            ResMut<Formation>,
            Res<AssetServer>,
            ResMut<NextState<Session>>,
            Res<State<Session>>,
        )>::new(app.world_mut());
        {
            let (mut commands, mut roster, mut formation, assets, mut next, state) =
                operation.get_mut(app.world_mut()).unwrap();
            assert!(
                begin_adventure(
                    &mut drafts,
                    &state,
                    &mut commands,
                    &mut roster,
                    &mut formation,
                    &assets,
                    &target,
                    &mut next
                )
                .is_err()
            );
        }
        operation.apply(app.world_mut());
        assert!(app.world().resource::<PartyRoster>().0.is_empty());
        assert_eq!(app.world_mut().query::<&Destination>().iter(app.world()).count(), 0);
        drafts[1].name = "Second".into();
        {
            let (mut commands, mut roster, mut formation, assets, mut next, state) =
                operation.get_mut(app.world_mut()).unwrap();
            begin_adventure(
                &mut drafts,
                &state,
                &mut commands,
                &mut roster,
                &mut formation,
                &assets,
                &target,
                &mut next,
            )
            .unwrap();
            assert!(
                begin_adventure(
                    &mut drafts,
                    &state,
                    &mut commands,
                    &mut roster,
                    &mut formation,
                    &assets,
                    &target,
                    &mut next
                )
                .is_err()
            );
        }
        operation.apply(app.world_mut());
        assert!(drafts.is_empty());
        let members: Vec<_> = app.world().resource::<PartyRoster>().0.iter().copied().collect();
        assert_eq!(members.len(), 2);
        assert_eq!(app.world().get::<Character>(members[0]), Some(&expected));
        assert_eq!(app.world().get::<Vitals>(members[0]).unwrap().hp, expected.max_hp());
        assert_eq!(app.world_mut().query::<&Destination>().iter(app.world()).count(), 1);
    }
}
