//! Camping on screen: the view fades to black, retail's camping screen
//! replaces the world while the party rests, and fades back. The party bar
//! and notices stay above it.

use bevy::prelude::*;

use crate::content::ui::SpriteSheet;
use crate::game::camp::{Camp, CampPhase, FADE_SECONDS};

#[derive(Resource)]
pub struct CampArt(Handle<SpriteSheet>);

pub fn load_camp_art(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(CampArt(assets.load("retail://data/level load/camping screen.sti")));
}

#[derive(Component)]
pub struct CampScreen;

#[derive(Component)]
pub struct CampPicture;

#[derive(Component)]
pub struct CampFade;

/// How dark the fade is, and whether the camping screen is behind it: each
/// fade darkens for its first half and clears for its second, swapping the
/// world and the camping screen while black.
fn look(phase: CampPhase) -> (f32, bool) {
    let half = FADE_SECONDS / 2.0;
    let ramp = |elapsed: f32| {
        if elapsed < half { elapsed / half } else { 1.0 - (elapsed - half) / half }
    };
    match phase {
        CampPhase::FadingIn(elapsed) => (ramp(elapsed), elapsed >= half),
        CampPhase::Resting => (0.0, true),
        CampPhase::FadingOut(elapsed) => (ramp(elapsed), elapsed < half),
    }
}

pub fn show_camp(
    mut commands: Commands,
    camp: Option<Res<Camp>>,
    art: Res<CampArt>,
    sheets: Res<Assets<SpriteSheet>>,
    screens: Query<Entity, With<CampScreen>>,
    mut pictures: Query<(&mut ImageNode, &mut Visibility), With<CampPicture>>,
    mut fades: Query<&mut BackgroundColor, With<CampFade>>,
) {
    let Some(camp) = camp else {
        for screen in &screens {
            commands.entity(screen).despawn();
        }
        return;
    };
    if screens.is_empty() {
        let full = || Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        };
        commands.spawn((CampScreen, full(), GlobalZIndex(-10), Pickable::IGNORE)).with_children(
            |screen| {
                screen.spawn((
                    CampPicture,
                    ImageNode::default(),
                    // Retail's screen is 4:3; black fills the rest.
                    BackgroundColor(Color::BLACK),
                    Visibility::Hidden,
                    full(),
                    Pickable::IGNORE,
                ));
                screen.spawn((CampFade, BackgroundColor(Color::NONE), full(), Pickable::IGNORE));
            },
        );
        return;
    }
    let (darkness, picture_shown) = look(camp.phase);
    for mut fade in &mut fades {
        fade.0 = Color::BLACK.with_alpha(darkness);
    }
    let picture = sheets.get(&art.0).and_then(|sheet| sheet.images.first());
    for (mut image, mut visibility) in &mut pictures {
        if let Some(picture) = picture {
            image.image = picture.clone();
        }
        *visibility = if picture_shown { Visibility::Inherited } else { Visibility::Hidden };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fades_swap_the_world_and_camping_screen_while_black() {
        assert_eq!(look(CampPhase::FadingIn(0.0)), (0.0, false));
        assert_eq!(look(CampPhase::FadingIn(0.5)), (1.0, true));
        assert_eq!(look(CampPhase::FadingIn(0.75)), (0.5, true));
        assert_eq!(look(CampPhase::Resting), (0.0, true));
        assert_eq!(look(CampPhase::FadingOut(0.25)), (0.5, true));
        assert_eq!(look(CampPhase::FadingOut(0.5)), (1.0, false));
    }
}
