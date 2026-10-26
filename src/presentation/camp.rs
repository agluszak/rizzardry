//! Camping on screen: the view fades to black, retail's camping screen
//! replaces the world while the party rests, and fades back. The party bar
//! and notices stay above it.

use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;

use crate::content::ui::SpriteSheet;
use crate::game::camp::{Camp, FADE_SECONDS, LeaveCamp};

#[derive(Resource)]
pub struct CampArt(Handle<SpriteSheet>);

pub fn load_camp_art(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(CampArt(assets.load("retail://data/level load/camping screen.sti")));
}

/// The camping screen, with its key: Escape asks to stop camping, before
/// the other screens' keys.
#[derive(Component, Clone, Default)]
pub struct CampScreen;

#[derive(Component, Clone, Default)]
pub struct CampPicture;

#[derive(Component, Clone, Default)]
pub struct CampFade;

#[derive(InputAction)]
#[action_output(bool)]
struct Leave;

/// How dark the fade is, and whether the camping screen is behind it: each
/// fade darkens for its first half and clears for its second, swapping the
/// world and the camping screen while black.
fn look(phase: &Camp) -> (f32, bool) {
    let half = FADE_SECONDS / 2.0;
    let ramp = |elapsed: f32| {
        if elapsed < half { elapsed / half } else { 1.0 - (elapsed - half) / half }
    };
    match phase {
        Camp::FadingIn { fade, .. } => (ramp(fade.elapsed_secs()), fade.elapsed_secs() >= half),
        Camp::Resting(_) => (0.0, true),
        Camp::FadingOut { fade, .. } => (ramp(fade.elapsed_secs()), fade.elapsed_secs() < half),
    }
}

/// Fills the window.
fn full() -> impl Scene {
    bsn! {
        Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100) }
        Pickable::IGNORE
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
        commands.spawn_scene(bsn! {
            @full()
            CampScreen
            GlobalZIndex(-10)
            ContextPriority::<CampScreen>::new(1)
            Actions<CampScreen> [
                Action::<Leave>::new()
                ActionSettings { consume_input: true }
                Bindings [ Binding(KeyCode::Escape) ]
            ]
            on(|_: On<Start<Leave>>, mut leave: MessageWriter<LeaveCamp>| {
                leave.write(LeaveCamp);
            })
            Children [
                // Retail's screen is 4:3; black fills the rest.
                @full() CampPicture ImageNode BackgroundColor(Color::BLACK) Visibility::Hidden
                --
                @full() CampFade BackgroundColor(Color::NONE)
            ]
        });
        return;
    }
    let (darkness, picture_shown) = look(&camp);
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
        let fade = |seconds: f32| {
            let mut fade = Timer::from_seconds(FADE_SECONDS, TimerMode::Once);
            fade.set_elapsed(std::time::Duration::from_secs_f32(seconds));
            fade
        };
        assert_eq!(look(&Camp::FadingIn { fade: fade(0.0), forced: false }), (0.0, false));
        assert_eq!(look(&Camp::FadingIn { fade: fade(0.5), forced: false }), (1.0, true));
        assert_eq!(look(&Camp::FadingIn { fade: fade(0.75), forced: false }), (0.5, true));
        assert_eq!(look(&Camp::Resting(crate::game::camp::RestGoal::Until(0))), (0.0, true));
        assert_eq!(
            look(&Camp::FadingOut { fade: fade(0.25), outcome: crate::game::camp::CampNews::Done }),
            (0.5, true)
        );
        assert_eq!(
            look(&Camp::FadingOut { fade: fade(0.5), outcome: crate::game::camp::CampNews::Done }),
            (1.0, false)
        );
    }
}
