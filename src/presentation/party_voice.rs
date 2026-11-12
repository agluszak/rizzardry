//! Device remarks are presentation-only, actor-bound, and never draw gameplay RNG.
use super::{
    PresentationRng,
    voice::{Speech, pc_voice_path, pc_voice_stem},
};
use crate::{
    content::{npcs::DialogueScript, ui::TextTable},
    game::{
        characters::{Character, Condition, Gender, Vitals},
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
    members: Query<
        'w,
        's,
        (&'static Character, Has<crate::game::actors::CanFight>, Option<&'static ActorId>),
    >,
    catalogs: Res<'w, crate::game::data::Catalogs>,
    scripts: Res<'w, NpcAssets>,
}
impl Speakers<'_, '_> {
    fn eligible(&self, excluded: Option<Entity>) -> Vec<(Entity, Voice)> {
        self.roster
            .iter()
            .filter(|&actor| excluded != Some(actor))
            .filter_map(|actor| {
                let (character, can_fight, npc) = self.members.get(actor).ok()?;
                can_fight.then(|| (actor, self.voice(character, npc)))
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
        // The member named says it; otherwise someone fit other than the
        // selected member, who becomes the selected member.
        let (actor, voice) = match remark.speaker {
            Some(speaker) => {
                let named =
                    speakers.eligible(None).into_iter().find(|(actor, _)| *actor == speaker);
                let Some(named) = named else { continue };
                named
            }
            None => {
                let mut candidates = speakers.eligible(selected.0);
                if candidates.is_empty() {
                    continue;
                }
                let chosen = candidates.swap_remove(rng.0.random_range(0..candidates.len()));
                selected.0 = Some(chosen.0);
                chosen
            }
        };
        commands.spawn((
            PendingRemark {
                actor,
                requested: remark.event,
                voice,
                delay: Timer::from_seconds(remark.after, TimerMode::Once),
                audio: None,
                dialogue: true,
            },
            ChildOf(*level),
        ));
    }
}

/// Below this percent of their health a wounded member says they are going
/// down (retail `g_portrait_low_health_percent`), and below half they may cry
/// out (`g_flee_hp_fraction`).
const GOING_DOWN_BELOW: u32 = 20;
const CRIES_OUT_BELOW: u32 = 50;

/// What a member says: going down (3), crying out (2 or 28); another
/// reacting to a man's death (18), a woman's (19), or one of the first two
/// in the order (20); a lone survivor (22); someone after more than one
/// death (21).
const GOING_DOWN: u16 = 3;
const CRIES_OUT: [u16; 2] = [2, 28];
const MOURNS: [u16; 3] = [18, 19, 20];
const LAST_STANDING: u16 = 22;
/// What a wounded member may say after, one of three at random (retail
/// special events 7, 17 and 18 past the first remapped one).
const AFTER_A_WOUND: [u16; 3] = [507, 517, 518];
const AFTER_LOSSES: u16 = 21;

/// Members who fell since the round began.
#[derive(Resource, Default)]
pub(super) struct Fallen(u32);

fn say(
    commands: &mut Commands,
    level: Entity,
    (actor, voice): (Entity, Voice),
    event: u16,
    after: f32,
) {
    commands.spawn((
        PendingRemark {
            actor,
            requested: event,
            voice,
            delay: Timer::from_seconds(after, TimerMode::Once),
            audio: None,
            dialogue: false,
        },
        ChildOf(level),
    ));
}

/// What a member says of a kill worth speaking of (retail event 25).
const NOTABLE_KILL: u16 = 25;

/// A member speaks of a kill worth speaking of (retail `RecordMonsterKill`).
pub(super) fn react_to_feats(
    mut commands: Commands,
    mut news: MessageReader<crate::game::combat::CombatNews>,
    speakers: Speakers,
    level: Single<Entity, With<Level>>,
) {
    for news in news.read() {
        let crate::game::combat::CombatNews::Feat { member } = news else { continue };
        let voice = speakers.eligible(None).into_iter().find(|(actor, _)| actor == member);
        if let Some(voice) = voice {
            say(&mut commands, *level, voice, NOTABLE_KILL, 0.0);
        }
    }
}

/// The party reacts to its wounds and its dead (retail
/// `QueueDamageReactionEvents`, `QueuePartyDeathReaction`): a member still
/// standing below a fifth of their health says they are going down, below
/// half may cry out, a moment and a half later; and whatever the wound, one
/// of three remarks follows, unless the same is already waiting for them.
/// When one dies, another member reacts three seconds later, one past the
/// first two of the order if it was one of those.
pub(super) fn react_to_wounds(
    mut commands: Commands,
    mut damaged: MessageReader<crate::game::hits::Damaged>,
    speakers: Speakers,
    order: Single<&PartyRoster>,
    members: Query<(&Character, &Vitals, &crate::game::characters::Conditions)>,
    level: Single<Entity, With<Level>>,
    mut fallen: ResMut<Fallen>,
    waiting: Query<&PendingRemark>,
    camp: Option<Res<crate::game::camp::Camp>>,
    mut rng: ResMut<PresentationRng>,
) {
    for hurt in damaged.read() {
        let Some(seat) = order.get_index_of(&hurt.body) else { continue };
        let Ok((character, vitals, conditions)) = members.get(hurt.body) else { continue };
        if hurt.died {
            fallen.0 += 1;
            let first_two = seat < 2;
            let event = if first_two {
                MOURNS[2]
            } else if character.gender == Gender::Male {
                MOURNS[0]
            } else {
                MOURNS[1]
            };
            let mut others: Vec<(Entity, Voice)> = speakers
                .eligible(Some(hurt.body))
                .into_iter()
                .filter(|(other, _)| {
                    !first_two || order.get_index_of(other).is_some_and(|s| s >= 2)
                })
                .collect();
            if !others.is_empty() {
                let mourner = others.swap_remove(rng.0.random_range(0..others.len()));
                say(&mut commands, *level, mourner, event, 3.0);
            }
            continue;
        }
        // Wounds taken in camp, which the original keeps as its "surprise",
        // are not remarked on.
        if camp.is_some() {
            continue;
        }
        let asleep = conditions.has(Condition::Asleep) || conditions.has(Condition::Unconscious);
        let percent = vitals.health.current * 100 / vitals.health.max.max(1);
        let event = if asleep || percent >= CRIES_OUT_BELOW {
            None
        } else if percent < GOING_DOWN_BELOW {
            Some(GOING_DOWN)
        } else {
            Some(CRIES_OUT[rng.0.random_range(0..2)])
        };
        let voice = speakers.eligible(None).into_iter().find(|(actor, _)| *actor == hurt.body);
        if let (Some(event), Some(voice)) = (event, voice.clone()) {
            say(&mut commands, *level, voice, event, 1.5);
        }
        let after = AFTER_A_WOUND[rng.0.random_range(0..AFTER_A_WOUND.len())];
        let already = waiting.iter().any(|w| w.actor == hurt.body && w.requested == after);
        if let (false, Some(voice)) = (already, voice) {
            say(&mut commands, *level, voice, after, 0.0);
        }
    }
}

/// As a round begins after members fell in the last (retail
/// `QueueLastSurvivorEvent`, `QueueTurnReactionEvent`): the last one standing
/// speaks, or, when more than one fell, someone does.
pub(super) fn react_to_losses(
    _: On<crate::game::combat::RoundBegan>,
    mut commands: Commands,
    speakers: Speakers,
    level: Single<Entity, With<Level>>,
    mut fallen: ResMut<Fallen>,
    mut rng: ResMut<PresentationRng>,
) {
    let fell = std::mem::take(&mut fallen.0);
    if fell == 0 {
        return;
    }
    let mut standing = speakers.eligible(None);
    match standing.len() {
        0 => {}
        1 => say(&mut commands, *level, standing.remove(0), LAST_STANDING, 0.0),
        _ if fell > 1 => {
            let one = standing.swap_remove(rng.0.random_range(0..standing.len()));
            say(&mut commands, *level, one, AFTER_LOSSES, 0.0);
        }
        _ => {}
    }
}

/// What a member says as a condition becomes their worst (retail
/// `QueueConditionChangeReaction`), if anything: one of several at random
/// where the original picks.
fn on_taking(condition: Condition) -> &'static [u16] {
    use Condition::*;
    match condition {
        Slowed => &[515],
        Silenced => &[510],
        Irritated | Nauseated => &[502],
        Diseased | Poisoned | Hexed | Infatuated => &[5, 6],
        Blind => &[7],
        Webbed | Paralyzed => &[9],
        Afraid => &[501],
        Insane => &[500],
        Unconscious => &[508, 519, 520],
        Drained | Turncoat | Asleep | Missing => &[],
    }
}

/// What another member says when one goes missing.
const ONE_MISSING: u16 = 40;
/// What a member says once raised, once back or free of an infatuation,
/// and once a condition lifts with nothing left or with something left
/// (retail `QueueConditionClearedReaction`).
const RAISED: u16 = 12;
const BACK: u16 = 11;
const RECOVERED: u16 = 0x55;
const BETTER: u16 = 0x54;

/// What each member's conditions were when last looked at, and whether they
/// were alive.
#[derive(Default)]
pub(super) struct Afflictions(bevy::ecs::entity::EntityHashMap<(Vec<Condition>, bool)>);

/// The party speaks of what takes hold of it and what lifts: as a condition
/// becomes a member's worst, and as one lifts (retail
/// `QueueConditionChangeReaction`, `QueueConditionClearedReaction`).
pub(super) fn react_to_conditions(
    mut commands: Commands,
    members: Query<
        (Entity, &Vitals, &crate::game::characters::Conditions),
        Or<(Changed<Vitals>, Changed<crate::game::characters::Conditions>)>,
    >,
    speakers: Speakers,
    order: Single<&PartyRoster>,
    level: Single<Entity, With<Level>>,
    mut known: Local<Afflictions>,
    mut rng: ResMut<PresentationRng>,
) {
    for (member, vitals, conditions) in &members {
        if !order.contains(&member) {
            continue;
        }
        let now: Vec<Condition> = conditions.iter().map(|&(c, _)| c).collect();
        let alive = !vitals.is_dead();
        let Some((before, was_alive)) = known.0.insert(member, (now.clone(), alive)) else {
            continue;
        };
        let voice = |who: Entity| speakers.eligible(None).into_iter().find(|(a, _)| *a == who);
        let worst = conditions.worst();
        let mut said = None;
        if alive && !was_alive {
            said = Some((member, RAISED));
        } else if let Some(worst) = worst.filter(|w| !before.contains(w)) {
            if worst == Condition::Missing {
                let others = speakers.eligible(Some(member));
                if !others.is_empty() {
                    let one = &others[rng.0.random_range(0..others.len())];
                    said = Some((one.0, ONE_MISSING));
                }
            } else {
                let lines = on_taking(worst);
                if !lines.is_empty() {
                    said = Some((member, lines[rng.0.random_range(0..lines.len())]));
                }
            }
        } else if let Some(&lifted) = before.iter().find(|c| !now.contains(c)) {
            let line = match lifted {
                Condition::Infatuated | Condition::Missing => Some(BACK),
                Condition::Drained | Condition::Turncoat | Condition::Asleep => None,
                _ if worst.is_none() => Some(RECOVERED),
                _ => Some(BETTER),
            };
            said = line.map(|line| (member, line));
        }
        if let Some((speaker, event)) = said
            && let Some(voice) = voice(speaker)
        {
            say(&mut commands, *level, voice, event, 0.0);
        }
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
    members: Query<&crate::game::characters::Conditions, With<crate::game::actors::CanFight>>,
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
        let conditions = members.get(remark.actor).ok();
        if !entering.is_empty()
            || !roster.as_ref().is_some_and(|r| r.contains(&remark.actor))
            || conditions.is_none()
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
        let conditions = conditions.unwrap();
        let event = if conditions.has(Condition::Silenced) {
            510
        } else if conditions.has(Condition::Insane) {
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
        world.insert_resource(NpcAssets {
            scripts: [(NpcId(0), script.clone())].into(),
            ..default()
        });

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
