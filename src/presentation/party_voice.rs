//! Device remarks are presentation-only, actor-bound, and never draw gameplay RNG.
use super::{
    PresentationRng,
    voice::{Speech, pc_voice_path, pc_voice_stem},
};
use crate::{
    content::ui::TextTable,
    game::{
        characters::{Character, Condition, Vitals},
        party::roster::PartyRoster,
        world::{
            interaction::LockNews,
            level::{Destination, Level},
        },
    },
};
use bevy::{
    asset::LoadState,
    audio::{AudioPlayer, AudioSource, PlaybackSettings},
    prelude::*,
};
use rand::Rng;

#[derive(Message)]
pub struct SpokenRemark {
    pub actor: Entity,
    pub text: String,
}

#[derive(Component)]
pub(super) struct PendingRemark {
    actor: Entity,
    requested: u16,
    stem: String,
    delay: Timer,
    audio: Option<(u16, Handle<AudioSource>)>,
    quotes: Option<Handle<TextTable>>,
    /// Asked for by the conversation, which it does not wait out.
    dialogue: bool,
}
#[derive(Component)]
pub(super) struct PlayingRemark;

pub(super) fn queue_device_remarks(
    mut commands: Commands,
    mut news: MessageReader<LockNews>,
    roster: Single<&PartyRoster>,
    members: Query<(&Character, &Vitals), Without<crate::game::npcs::Npc>>,
    level: Single<Entity, With<Level>>,
    mut rng: ResMut<PresentationRng>,
) {
    for news in news.read() {
        let event = match news {
            LockNews::Locked | LockNews::NotInCombat => 511,
            LockNews::Trapped { remark: true } => {
                if rng.0.random_range(0..100) < 20 {
                    25
                } else {
                    512
                }
            }
            LockNews::Disarmed { .. } if rng.0.random_range(0..100) < 40 => 516,
            _ => continue,
        };
        let candidates: Vec<_> = roster
            .iter()
            .filter_map(|actor| {
                let (character, vitals) = members.get(actor).ok()?;
                vitals.can_fight().then_some((actor, character))
            })
            .collect();
        if candidates.is_empty() {
            continue;
        }
        // Retail's cyclic skip over eligible party slots, not a global Character query.
        let (actor, character) = candidates[rng.0.random_range(0..8) % candidates.len()];
        commands.spawn((
            PendingRemark {
                actor,
                requested: event,
                stem: pc_voice_stem(character.gender, character.personality, character.voice),
                delay: Timer::from_seconds(0.6, TimerMode::Once),
                audio: None,
                quotes: None,
                dialogue: false,
            },
            ChildOf(*level),
        ));
    }
}

/// A party member remarks on the conversation (retail
/// `PickRandomPartySpeaker`): someone awake other than the selected member,
/// who becomes the selected member.
pub(super) fn queue_dialogue_remarks(
    mut commands: Commands,
    mut remarks: MessageReader<crate::game::npcs::conversation::PartyRemark>,
    roster: Single<&PartyRoster>,
    members: Query<(&Character, &Vitals), Without<crate::game::npcs::Npc>>,
    level: Single<Entity, With<Level>>,
    mut selected: ResMut<super::main_screen::SelectedMember>,
    mut rng: ResMut<PresentationRng>,
) {
    for remark in remarks.read() {
        let candidates: Vec<_> = roster
            .iter()
            .filter(|&actor| selected.0 != Some(actor))
            .filter_map(|actor| {
                let (character, vitals) = members.get(actor).ok()?;
                vitals.can_fight().then_some((actor, character))
            })
            .collect();
        if candidates.is_empty() {
            continue;
        }
        let (actor, character) = candidates[rng.0.random_range(0..candidates.len())];
        selected.0 = Some(actor);
        commands.spawn((
            PendingRemark {
                actor,
                requested: remark.event,
                stem: pc_voice_stem(character.gender, character.personality, character.voice),
                delay: Timer::from_seconds(0.0, TimerMode::Once),
                audio: None,
                quotes: None,
                dialogue: true,
            },
            ChildOf(*level),
        ));
    }
}

/// Drop interrupted/obsolete remarks instead of building a stale voice backlog.
/// Gameplay never waits for the sound or quote assets.
pub(super) fn play_device_remarks(
    mut commands: Commands,
    time: Res<Time<Real>>,
    assets: Res<AssetServer>,
    audio: Res<Assets<AudioSource>>,
    tables: Res<Assets<TextTable>>,
    mut pending: Query<(Entity, &mut PendingRemark)>,
    roster: Option<Single<&PartyRoster>>,
    members: Query<&Vitals>,
    speaking: Query<(), Or<(With<Speech>, With<PlayingRemark>)>>,
    conversations: Query<
        (),
        (
            With<crate::game::npcs::conversation::Conversation>,
            Without<crate::game::npcs::conversation::Closing>,
        ),
    >,
    entering: Query<(), With<Destination>>,
    mut text: MessageWriter<SpokenRemark>,
) {
    let mut busy = !speaking.is_empty();
    let talking = !conversations.is_empty();
    for (entity, mut remark) in &mut pending {
        let vitals = members.get(remark.actor).ok();
        if !entering.is_empty()
            || !roster.as_ref().is_some_and(|r| r.contains(&remark.actor))
            || vitals.is_none_or(|v| !v.can_fight())
        {
            commands.entity(entity).despawn();
            continue;
        }
        if !remark.delay.tick(time.delta()).is_finished() {
            continue;
        }
        if busy || (talking && !remark.dialogue) {
            commands.entity(entity).despawn();
            continue;
        }
        let vitals = vitals.unwrap();
        let event = if vitals.conditions.has(Condition::Silenced) {
            510
        } else if vitals.conditions.has(Condition::Insane) {
            500
        } else {
            remark.requested
        };
        if remark.audio.as_ref().is_none_or(|(loaded, _)| *loaded != event) {
            remark.audio = Some((event, assets.load(pc_voice_path(&remark.stem, event))));
            remark.quotes = (event == 25)
                .then(|| assets.load(format!("retail://data/quotes/pcs/{}.msg", remark.stem)));
        }
        let sound = &remark.audio.as_ref().unwrap().1;
        if matches!(assets.get_load_state(sound.id()), Some(LoadState::Failed(_))) {
            commands.entity(entity).despawn();
            continue;
        }
        if !audio.contains(sound) {
            continue;
        }
        if let Some(quotes) = &remark.quotes {
            if let Some(table) = tables.get(quotes) {
                if let Some(line) = table.get(25).filter(|line| !line.trim().is_empty()) {
                    text.write(SpokenRemark { actor: remark.actor, text: line.trim().to_owned() });
                }
            } else if !matches!(assets.get_load_state(quotes.id()), Some(LoadState::Failed(_))) {
                continue;
            }
        }
        commands.entity(entity).remove::<PendingRemark>().insert((
            PlayingRemark,
            AudioPlayer::new(sound.clone()),
            PlaybackSettings::DESPAWN,
        ));
        busy = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{
        characters::{Gender, Profession, Race},
        party::roster::{PartyMember, test_party},
        rng::GameRng,
    };
    use rand::SeedableRng;

    #[test]
    fn remarks_keep_the_chosen_actor_cancel_departures_and_leave_gameplay_rng_untouched() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<AudioSource>()
            .init_asset::<TextTable>()
            .insert_resource(PresentationRng(rand::rngs::SmallRng::seed_from_u64(3)))
            .insert_resource(GameRng::seeded(7))
            .add_message::<LockNews>()
            .add_message::<SpokenRemark>()
            .add_systems(Update, (queue_device_remarks, play_device_remarks).chain());
        let profile = Character::new("Speaker", Race::Human, Gender::Male, Profession::Fighter);
        let actor = app.world_mut().spawn((Vitals::new(&profile), profile.clone())).id();
        // A retained non-party profile must never become the selected speaker.
        app.world_mut().spawn((Vitals::new(&profile), profile));
        test_party(app.world_mut(), &[actor]);
        app.world_mut().spawn(Level::test(default()));
        let before = app.world().resource::<GameRng>().clone();
        app.world_mut().write_message(LockNews::Locked);
        app.update();
        let pending =
            app.world_mut().query::<(Entity, &PendingRemark)>().single(app.world()).unwrap().0;
        assert_eq!(app.world().get::<PendingRemark>(pending).unwrap().actor, actor);
        // Headless fixture marks the audio loaded; no audio backend is run.
        let sound = app
            .world_mut()
            .resource_mut::<Assets<AudioSource>>()
            .add(AudioSource { bytes: Vec::new().into() });
        let mut remark = app.world_mut().get_mut::<PendingRemark>(pending).unwrap();
        remark.delay = Timer::from_seconds(0.0, TimerMode::Once);
        remark.audio = Some((511, sound));
        app.update();
        assert!(app.world().get::<PlayingRemark>(pending).is_some());
        app.world_mut().despawn(pending);
        app.world_mut().write_message(LockNews::Locked);
        app.update();
        let pending = app
            .world_mut()
            .query_filtered::<Entity, With<PendingRemark>>()
            .single(app.world())
            .unwrap();
        app.world_mut().entity_mut(actor).remove::<PartyMember>();
        app.update();
        assert!(app.world().get_entity(pending).is_err());
        assert_eq!(*app.world().resource::<GameRng>(), before);
    }
}
