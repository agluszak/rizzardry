//! Device remarks are presentation-only, actor-bound, and never draw gameplay RNG.
use super::{
    PresentationRng,
    voice::{Speech, pc_voice_path, pc_voice_stem},
};
use crate::{
    content::{npcs::DialogueScript, ui::TextTable},
    game::{
        characters::{Character, Condition, Vitals},
        npcs::{ActorId, NpcAssets},
        party::roster::PartyRoster,
        world::{
            interaction::LockNews,
            level::{Destination, Level},
        },
    },
};
use bevy::ecs::system::SystemParam;
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
    voice: Voice,
    delay: Timer,
    audio: Option<(u16, Handle<AudioSource>)>,
    /// Asked for by the conversation, which it does not wait out.
    dialogue: bool,
}
#[derive(Component)]
pub(super) struct PlayingRemark;

/// Where a party member's remarks come from (retail `PlayEventSound` and
/// `QuoteText`): a made character has the voice it was created with and
/// the quote table of the same stem; a recruited NPC has its own
/// `rpc_<name>` sound set and speaks the lines of its dialogue script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Voice {
    Character { stem: String, quotes: Option<Handle<TextTable>> },
    Recruit { stem: String, script: Handle<DialogueScript> },
}
impl Voice {
    fn stem(&self) -> &str {
        match self {
            Voice::Character { stem, .. } | Voice::Recruit { stem, .. } => stem,
        }
    }
}

/// The party members who can speak a remark, in roster order: everyone
/// awake enough to fight, recruited NPCs included.
#[derive(SystemParam)]
pub(super) struct Speakers<'w, 's> {
    roster: Single<'w, 's, &'static PartyRoster>,
    members: Query<'w, 's, (&'static Character, &'static Vitals, Option<&'static ActorId>)>,
    catalogs: Res<'w, crate::game::data::Catalogs>,
    scripts: Res<'w, NpcAssets>,
}
impl Speakers<'_, '_> {
    fn eligible(&self, excluded: Option<Entity>) -> Vec<(Entity, Voice)> {
        self.roster
            .iter()
            .filter(|&actor| excluded != Some(actor))
            .filter_map(|actor| {
                let (character, vitals, npc) = self.members.get(actor).ok()?;
                vitals.can_fight().then(|| (actor, self.voice(character, npc)))
            })
            .collect()
    }
    fn voice(&self, character: &Character, npc: Option<&ActorId>) -> Voice {
        match npc {
            Some(npc) => {
                let id = npc.npc();
                Voice::Recruit {
                    stem: self.catalogs.npcs[id].party_voice_stem(),
                    script: self.scripts.scripts[&id].clone(),
                }
            }
            None => Voice::Character {
                stem: pc_voice_stem(character.gender, character.personality, character.voice),
                quotes: None,
            },
        }
    }
}

pub(super) fn queue_device_remarks(
    mut commands: Commands,
    mut news: MessageReader<LockNews>,
    speakers: Speakers,
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
        let mut candidates = speakers.eligible(None);
        if candidates.is_empty() {
            continue;
        }
        // Retail's cyclic skip over eligible party slots, not a global Character query.
        let (actor, voice) = candidates.swap_remove(rng.0.random_range(0..8) % candidates.len());
        commands.spawn((
            PendingRemark {
                actor,
                requested: event,
                voice,
                delay: Timer::from_seconds(0.6, TimerMode::Once),
                audio: None,
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
    speakers: Speakers,
    level: Single<Entity, With<Level>>,
    mut selected: ResMut<super::main_screen::SelectedMember>,
    mut rng: ResMut<PresentationRng>,
) {
    for remark in remarks.read() {
        let mut candidates = speakers.eligible(selected.0);
        if candidates.is_empty() {
            continue;
        }
        let (actor, voice) = candidates.swap_remove(rng.0.random_range(0..candidates.len()));
        selected.0 = Some(actor);
        commands.spawn((
            PendingRemark {
                actor,
                requested: remark.event,
                voice,
                delay: Timer::from_seconds(0.0, TimerMode::Once),
                audio: None,
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
    scripts: Res<Assets<DialogueScript>>,
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
            remark.audio = Some((event, assets.load(pc_voice_path(remark.voice.stem(), event))));
            if let Voice::Character { stem, quotes } = &mut remark.voice {
                *quotes = (event == 25)
                    .then(|| assets.load(format!("retail://data/quotes/pcs/{stem}.msg")));
            }
        }
        let sound = &remark.audio.as_ref().unwrap().1;
        if matches!(assets.get_load_state(sound.id()), Some(LoadState::Failed(_))) {
            commands.entity(entity).despawn();
            continue;
        }
        if !audio.contains(sound) {
            continue;
        }
        match &remark.voice {
            Voice::Character { quotes: Some(quotes), .. } => {
                if let Some(table) = tables.get(quotes) {
                    if let Some(line) = table.get(25).filter(|line| !line.trim().is_empty()) {
                        text.write(SpokenRemark {
                            actor: remark.actor,
                            text: line.trim().to_owned(),
                        });
                    }
                } else if !matches!(assets.get_load_state(quotes.id()), Some(LoadState::Failed(_)))
                {
                    continue;
                }
            }
            Voice::Character { quotes: None, .. } => {}
            // The script was loaded before its level was entered; its line
            // for the event is the first text of the quote of that number.
            Voice::Recruit { script, .. } => {
                let line = scripts
                    .get(script)
                    .and_then(|script| script.quotes.get(usize::from(event)))
                    .and_then(|quote| quote.text.first())
                    .map(|line| line.trim())
                    .filter(|line| !line.is_empty());
                if let Some(line) = line {
                    text.write(SpokenRemark { actor: remark.actor, text: line.to_owned() });
                }
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
    use crate::content::npcs::{NpcCatalog, NpcDefinition, NpcId};
    use crate::game::characters::{Gender, Profession, Race};
    use bevy::ecs::system::RunSystemOnce;

    fn member(world: &mut World, name: &str) -> Entity {
        let character = Character::new(name, Race::Human, Gender::Male, Profession::Fighter);
        let vitals = Vitals::new(&character);
        world.spawn((character, vitals)).id()
    }

    /// Retail `PickRandomPartySpeaker` picks over every party place, the
    /// NPC places included; a recruited NPC speaks with its own sound set.
    #[test]
    fn a_recruited_npc_is_an_eligible_speaker_with_its_own_voice() {
        let mut world = World::new();
        let pc = member(&mut world, "Made");
        let recruit = member(&mut world, "Drazic");
        world.entity_mut(recruit).insert(ActorId(NpcId(0)));
        crate::game::party::roster::test_party(&mut world, &[pc, recruit]);
        world.insert_resource(crate::game::data::Catalogs {
            npcs: NpcCatalog(vec![NpcDefinition { source_name: "Drazic".into(), ..default() }]),
            ..default()
        });
        let script = Handle::<DialogueScript>::default();
        world.insert_resource(NpcAssets { scripts: [(NpcId(0), script.clone())].into() });

        let eligible = world.run_system_once(|s: Speakers| s.eligible(None)).unwrap();
        assert_eq!(
            eligible,
            vec![
                (pc, Voice::Character { stem: "m_aggr10".into(), quotes: None }),
                (recruit, Voice::Recruit { stem: "rpc_drazic".into(), script })
            ]
        );
        let others = world.run_system_once(move |s: Speakers| s.eligible(Some(pc))).unwrap();
        assert_eq!(others.len(), 1, "the excluded member does not answer itself");
        assert_eq!(others[0].0, recruit);
    }
}
