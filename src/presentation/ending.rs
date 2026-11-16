//! The game's ending on screen (retail `BeginEndgameSequence`,
//! `ShowEndingScreen`): the view fades out, the ending's picture and words
//! replace the world and its voice is heard, and the picture fades in.

use bevy::prelude::*;

use crate::game::npcs::scenes::{Ended, Ending};
use crate::presentation::screen_art::{BitmapText, BitmapWrap, GameStrings, SCREEN, absolute};

/// What an ending shows and says: its picture among the loading screens,
/// its words in the game's strings, the sound that goes with them.
fn told(ending: Ending) -> (&'static str, Option<usize>, Option<&'static str>) {
    match ending {
        Ending::Savant => ("darklord", Some(0x787), Some("npcs/voc_endgame1/voc_endgame1_005.mp3")),
        Ending::Cosmic => {
            ("cosmic ending", Some(0x788), Some("npcs/voc_endgame2/voc_endgame2_005.mp3"))
        }
        Ending::Split => {
            ("cosmic ending", Some(0x789), Some("npcs/voc_endgame3/voc_endgame3_000.mp3"))
        }
        Ending::Blowout => ("blowout", Some(0x78a), Some("npcs/voc_endgame4/voc_endgame4_000.mp3")),
        Ending::Untold => ("deathscreen", None, None),
        // The party's death is heard (retail `BeginPartyDeath`).
        Ending::Fallen => ("deathscreen", None, Some("misc/partydead.wav")),
    }
}

/// The view fades out over a second and a half, and the picture in over
/// one and a fifth; standing with the Savant, both are at once.
const OUT_SECONDS: f32 = 1.5;
const IN_SECONDS: f32 = 1.2;

#[derive(Component, Debug)]
pub enum EndingScreen {
    Out(Timer),
    In(Timer),
    Shown,
}

#[derive(Component)]
pub struct EndingVeil;

pub fn show_ending(
    mut commands: Commands,
    time: Res<Time<Real>>,
    ended: Option<Res<Ended>>,
    (assets, strings): (Res<AssetServer>, GameStrings),
    mut screens: Query<(Entity, &mut EndingScreen)>,
    mut veils: Query<&mut BackgroundColor, With<EndingVeil>>,
) {
    let Some(Ended(ending)) = ended.as_deref().copied() else { return };
    let at_once = ending == Ending::Savant;
    // The bomb's ending fades through white.
    let veil = if ending == Ending::Blowout { Color::WHITE } else { Color::BLACK };
    let Ok((screen, mut phase)) = screens.single_mut() else {
        let seconds = if at_once { 0.0 } else { OUT_SECONDS };
        let screen = commands
            .spawn((
                EndingScreen::Out(Timer::from_seconds(seconds, TimerMode::Once)),
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100),
                    height: percent(100),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                GlobalZIndex(i32::MAX - 2),
            ))
            .id();
        commands.spawn((
            EndingVeil,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                ..default()
            },
            BackgroundColor(Color::NONE),
            GlobalZIndex(i32::MAX - 1),
            Pickable::IGNORE,
            ChildOf(screen),
        ));
        return;
    };
    let darkness = match &mut *phase {
        EndingScreen::Out(fade) => {
            if fade.tick(time.delta()).is_finished() {
                let (picture, words, voice) = told(ending);
                let node =
                    Node { position_type: PositionType::Relative, ..absolute(Vec2::ZERO, SCREEN) };
                let page =
                    commands.spawn((node, BackgroundColor(Color::BLACK), ChildOf(screen))).id();
                let sheet = assets.load(format!("retail://data/level load/{picture}.sti"));
                crate::presentation::screen_art::spawn_picture(
                    &mut commands,
                    page,
                    sheet,
                    0,
                    Vec2::ZERO,
                );
                if let Some(words) = words {
                    // Retail lays the words out to the foot of the box from
                    // 70 to 569 across and down to 464.
                    commands.spawn((
                        BitmapText {
                            font: assets.load("retail://data/fonts/opt_detail_font.sti"),
                            value: strings.get(words).to_owned(),
                            at: Vec2::new(70.0, 300.0),
                            width: 499.0,
                            ..default()
                        },
                        BitmapWrap { width: 499, line_spacing: 4 },
                        ChildOf(page),
                    ));
                }
                if let Some(voice) = voice {
                    commands.spawn((
                        AudioPlayer::new(
                            assets.load::<AudioSource>(format!("retail://data/sound/{voice}")),
                        ),
                        PlaybackSettings::DESPAWN,
                    ));
                }
                let seconds = if at_once { 0.0 } else { IN_SECONDS };
                *phase = EndingScreen::In(Timer::from_seconds(seconds, TimerMode::Once));
                1.0
            } else {
                fade.fraction()
            }
        }
        EndingScreen::In(fade) => {
            let left = fade.tick(time.delta()).fraction_remaining();
            if left == 0.0 {
                *phase = EndingScreen::Shown;
            }
            left
        }
        EndingScreen::Shown => 0.0,
    };
    for mut colour in &mut veils {
        colour.set_if_neq(BackgroundColor(veil.with_alpha(darkness)));
    }
}
