//! Camping on screen: the view fades to black, retail's camping screen
//! replaces the world while the party rests, and fades back. The party bar
//! and notices stay above it.

use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;

use crate::content::ui::SpriteSheet;
use crate::game::camp::{Camp, LeaveCamp};
use crate::presentation::screen_art::visible_if;

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

/// The black veil the fades draw over the world.
#[derive(Component, Clone, Default)]
pub struct CampVeil;

/// Each fade: half a second to black, half a second from it.
const FADE_SECONDS: f32 = 1.0;

/// Where the camping screen is in its own animation, which follows the
/// rest without holding it: fading in when the party starts camping, shown
/// while it rests, fading out once it stops.
#[derive(Component, Clone, Debug)]
pub enum CampFade {
    In(Timer),
    Shown,
    Out(Timer),
}

fn fade() -> Timer {
    Timer::from_seconds(FADE_SECONDS, TimerMode::Once)
}

/// A fade turned back midway: the other fade from the mirrored point, so
/// the darkness and the picture behind it carry on unchanged.
fn reversed(fade: &Timer) -> Timer {
    let mut back = self::fade();
    back.set_elapsed(back.duration().saturating_sub(fade.elapsed()));
    back
}

#[derive(InputAction)]
#[action_output(bool)]
struct Leave;

/// How dark the fade is, and whether the camping screen is behind it: each
/// fade darkens for its first half and clears for its second, swapping the
/// world and the camping screen while black.
fn look(phase: &CampFade) -> (f32, bool) {
    let half = FADE_SECONDS / 2.0;
    let ramp = |elapsed: f32| {
        if elapsed < half { elapsed / half } else { 1.0 - (elapsed - half) / half }
    };
    match phase {
        CampFade::In(fade) => (ramp(fade.elapsed_secs()), fade.elapsed_secs() >= half),
        CampFade::Shown => (0.0, true),
        CampFade::Out(fade) => (ramp(fade.elapsed_secs()), fade.elapsed_secs() < half),
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
    time: Res<Time>,
    camp: Option<Res<Camp>>,
    art: Res<CampArt>,
    sheets: Res<Assets<SpriteSheet>>,
    mut screens: Query<(Entity, &mut CampFade), With<CampScreen>>,
    mut pictures: Query<(&mut ImageNode, &mut Visibility), With<CampPicture>>,
    mut veils: Query<&mut BackgroundColor, With<CampVeil>>,
) {
    let Ok((screen, mut phase)) = screens.single_mut() else {
        if camp.is_some() {
            spawn_camp_screen(&mut commands);
        }
        return;
    };
    let delta = time.delta();
    match (&mut *phase, camp.is_some()) {
        (CampFade::In(fade), true) => {
            if fade.tick(delta).is_finished() {
                *phase = CampFade::Shown;
            }
        }
        (CampFade::Shown, true) => {}
        (CampFade::In(fade), false) => *phase = CampFade::Out(reversed(fade)),
        (CampFade::Shown, false) => *phase = CampFade::Out(fade()),
        (CampFade::Out(fade), false) => {
            if fade.tick(delta).is_finished() {
                commands.entity(screen).despawn();
                return;
            }
        }
        (CampFade::Out(fade), true) => *phase = CampFade::In(reversed(fade)),
    }
    let (darkness, picture_shown) = look(&phase);
    for mut veil in &mut veils {
        veil.set_if_neq(BackgroundColor(Color::BLACK.with_alpha(darkness)));
    }
    let picture = sheets.get(&art.0).and_then(|sheet| sheet.images.first());
    for (mut image, mut visibility) in &mut pictures {
        if let Some(picture) = picture
            && image.image != *picture
        {
            image.image = picture.clone();
        }
        visibility.set_if_neq(visible_if(picture_shown));
    }
}

fn spawn_camp_screen(commands: &mut Commands) {
    commands
        .spawn_scene(bsn! {
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
                @full() CampVeil BackgroundColor(Color::NONE)
            ]
        })
        .insert(CampFade::In(fade()));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fade turned back midway carries on from the same darkness and
    /// picture; only a shown camp fades out from the start.
    #[test]
    fn a_reversed_fade_keeps_the_look_it_had() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<SpriteSheet>()
            .insert_resource(CampArt(Handle::default()))
            .add_systems(Update, show_camp);
        let quarter = {
            let mut fade = fade();
            fade.set_elapsed(std::time::Duration::from_secs_f32(0.25));
            fade
        };
        let screen = app.world_mut().spawn((CampScreen, CampFade::In(quarter.clone()))).id();
        let look_now = |app: &App| look(app.world().get::<CampFade>(screen).unwrap());
        let before = look(&CampFade::In(quarter));
        // Left before the screen went black: it fades back from where it was.
        app.update();
        assert!(matches!(app.world().get::<CampFade>(screen), Some(CampFade::Out(_))));
        assert_eq!(look_now(&app), before);
        // Camping again while it fades out turns it back the same way.
        app.insert_resource(Camp::UntilAble);
        app.update();
        assert!(matches!(app.world().get::<CampFade>(screen), Some(CampFade::In(_))));
        assert_eq!(look_now(&app), before);
        // Only a camp that was shown fades out from the start.
        *app.world_mut().get_mut::<CampFade>(screen).unwrap() = CampFade::Shown;
        app.world_mut().remove_resource::<Camp>();
        app.update();
        assert_eq!(look_now(&app), (0.0, true));
    }

    #[test]
    fn fades_swap_the_world_and_camping_screen_while_black() {
        let at = |seconds: f32| {
            let mut fade = fade();
            fade.set_elapsed(std::time::Duration::from_secs_f32(seconds));
            fade
        };
        assert_eq!(look(&CampFade::In(at(0.0))), (0.0, false));
        assert_eq!(look(&CampFade::In(at(0.5))), (1.0, true));
        assert_eq!(look(&CampFade::In(at(0.75))), (0.5, true));
        assert_eq!(look(&CampFade::Shown), (0.0, true));
        assert_eq!(look(&CampFade::Out(at(0.25))), (0.5, true));
        assert_eq!(look(&CampFade::Out(at(0.5))), (1.0, false));
    }
}
