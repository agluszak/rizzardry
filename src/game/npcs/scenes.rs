//! The story's scenes that turn on the party's own: the one Al-Sedexus
//! chooses and the rest who lie senseless while she has them, one who was
//! taken and is back, what waits for a battle to be over, and how the game
//! ends.
use bevy::prelude::*;
use serde::Deserialize;

use super::conversation::PartyRemark;
use super::story::{Befalls, Consequence, FACTS, Person, Reading, Readings, StoryFacts};
use super::{ActorId, story};
use crate::content::items::ItemId;
use crate::game::characters::{Character, Condition, Gender, Lasting, Vitals};
use crate::game::combat::Battle;
use crate::game::data::Catalogs;
use crate::game::factions::Factions;
use crate::game::items::Items;
use crate::game::magic::SpellPoints;
use crate::game::party::roster::{PartyMember, PartyRoster};
use crate::game::rng::GameRng;
use crate::game::world::level::Level;

/// The party is to pick one of its own: the screen lets the player, and
/// says who ([`Picked`]).
#[derive(Component, Clone, Copy, Debug)]
pub struct MemberPick;

/// The player has picked a member of the party.
#[derive(Event, Clone, Copy, Debug)]
pub struct Picked {
    pub member: Entity,
}

/// The member Al-Sedexus chose.
#[derive(Component, Clone, Copy, Debug)]
pub struct Chosen;

/// The chosen one is yet to be smitten with her.
#[derive(Component, Clone, Copy, Debug)]
pub struct Pining;

/// What the story says of the one Al-Sedexus is offered.
#[derive(Debug, Deserialize)]
pub(super) struct ChosenTable {
    /// The facts her answer turns on, cleared as one is offered.
    clears: Vec<String>,
    /// A woman was offered though the party has others.
    woman: String,
    /// The one offered does not wear what she asks.
    undressed: String,
    /// The one offered is a companion, or not fit.
    unable: String,
    /// What the one offered must carry.
    wears: Vec<u16>,
    /// The chosen one is not smitten while this person stands in the
    /// level, nor in levels under these paths.
    spared_by: u16,
    spared_in: Vec<String>,
    /// Set as the chosen one is smitten.
    smitten: String,
}

/// What a member says as they are offered and refused (retail event 43),
/// chosen (505), and smitten (44).
const REFUSED: u16 = 43;
const CHOSEN: u16 = 505;
const SMITTEN: u16 = 44;

/// The player offers a member to Al-Sedexus (retail
/// `EndScriptedPortraitPick`): a woman is refused while the party has
/// anyone else; one of the party's own who is fit and carries what she
/// asks is chosen.
pub(super) fn picked(
    picked: On<Picked>,
    picks: Query<Entity, With<MemberPick>>,
    roster: Single<&PartyRoster>,
    members: Query<(&Character, &Vitals, Has<ActorId>)>,
    chosen: Query<Entity, With<Chosen>>,
    held: Items,
    mut remarks: MessageWriter<PartyRemark>,
    mut commands: Commands,
) {
    let member = picked.member;
    if picks.is_empty() || !roster.contains(&member) {
        return;
    }
    for pick in &picks {
        commands.entity(pick).despawn();
    }
    let Ok((character, vitals, companion)) = members.get(member) else { return };
    let table = &FACTS.chosen;
    let mut set = |name: &String, value| {
        commands.trigger(Befalls(Consequence::Fact(name.clone(), value)));
    };
    for name in &table.clears {
        set(name, 0);
    }
    let others = roster
        .iter()
        .any(|other| members.get(other).is_ok_and(|(c, ..)| c.gender != Gender::Female));
    if character.gender == Gender::Female && others {
        set(&table.woman, 1);
        remarks.write(PartyRemark { event: REFUSED, after: 0.0, speaker: Some(member) });
    } else if companion || !vitals.can_fight() {
        set(&table.unable, 1);
    } else if table
        .wears
        .iter()
        .all(|&item| held.carried(member).any(|carried| held.id(carried) == Some(ItemId(item))))
    {
        for earlier in &chosen {
            commands.entity(earlier).remove::<(Chosen, Pining)>();
        }
        commands.entity(member).insert((Chosen, Pining));
        remarks.write(PartyRemark { event: CHOSEN, after: 0.0, speaker: Some(member) });
    } else {
        set(&table.undressed, 1);
    }
}

/// The chosen one is smitten once out of battle and away from her halls;
/// if the one who spares them stands in the level, they never are.
pub(super) fn pine(
    mut pining: Query<(Entity, &Character, &mut Vitals), (With<Pining>, With<PartyMember>)>,
    battles: Query<(), With<Battle>>,
    (people, content): (Query<Person>, Res<Catalogs>),
    level: Single<&Level>,
    mut remarks: MessageWriter<PartyRemark>,
    mut commands: Commands,
) {
    if !battles.is_empty() {
        return;
    }
    let table = &FACTS.chosen;
    let spared =
        matches!(story::one_of_kind(table.spared_by, &people, &content), Some((_, Some(_))));
    for (member, character, mut vitals) in &mut pining {
        if spared {
            commands.entity(member).remove::<Pining>();
        } else if !table.spared_in.iter().any(|path| level.path.starts_with(path.as_str())) {
            commands.entity(member).remove::<Pining>();
            vitals.afflict(Condition::Infatuated, Lasting::Indefinite);
            if character.gender == Gender::Male {
                remarks.write(PartyRemark { event: SMITTEN, after: 0.0, speaker: Some(member) });
            }
            commands.trigger(Befalls(Consequence::Fact(table.smitten.clone(), 1)));
        }
    }
}

/// Something the story waits to do until a battle has been fought and is
/// over, and a reading holds.
#[derive(Component, Debug)]
pub struct Awaited {
    reading: Reading,
    rows: Vec<Consequence>,
    fought: bool,
}

/// Something the story does again and again, so long apart, while a
/// reading holds; once it does not, never again.
#[derive(Component, Debug)]
pub struct Recurring {
    timer: Timer,
    reading: Reading,
    rows: Vec<Consequence>,
}

pub(super) fn recur(
    mut recurring: Query<(Entity, &mut Recurring)>,
    time: Res<Time>,
    readings: Readings,
    (story, factions): (Res<StoryFacts>, Res<Factions>),
    mut commands: Commands,
) {
    for (entity, mut again) in &mut recurring {
        if !again.timer.tick(time.delta()).just_finished() {
            continue;
        }
        if readings.answer(&again.reading, None, &story, &factions) == 0 {
            commands.entity(entity).despawn();
            continue;
        }
        for row in &again.rows {
            commands.trigger(Befalls(row.clone()));
        }
    }
}

/// Something the story does once the party has been spoken with and the
/// talk is over.
#[derive(Component, Debug)]
pub struct Parted {
    rows: Vec<Consequence>,
    spoken: bool,
}

pub(super) fn parted(
    mut waits: Query<(Entity, &mut Parted)>,
    talks: Query<(), With<super::conversation::Conversation>>,
    mut commands: Commands,
) {
    for (wait, mut parted) in &mut waits {
        if !talks.is_empty() {
            parted.spoken = true;
        } else if parted.spoken {
            for row in parted.rows.drain(..) {
                commands.trigger(Befalls(row));
            }
            commands.entity(wait).despawn();
        }
    }
}

pub(super) fn awaited(
    mut waits: Query<(Entity, &mut Awaited)>,
    battles: Query<(), With<Battle>>,
    readings: Readings,
    (story, factions): (Res<StoryFacts>, Res<Factions>),
    mut commands: Commands,
) {
    for (wait, mut awaited) in &mut waits {
        if !battles.is_empty() {
            awaited.fought = true;
        } else if awaited.fought && readings.answer(&awaited.reading, None, &story, &factions) != 0
        {
            for row in awaited.rows.drain(..) {
                commands.trigger(Befalls(row));
            }
            commands.entity(wait).despawn();
        }
    }
}

/// One of the game's endings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum Ending {
    /// The party stood with the Dark Savant.
    Savant,
    /// The party ascended.
    Cosmic,
    /// The party ascended, the Savant and Phoonzang parted.
    Split,
    /// The Savant's bomb went off.
    Blowout,
    /// None the story tells of.
    Untold,
}

/// The game has ended, and how.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ended(pub Ending);

/// The game ends, as the first ending whose reading holds (retail
/// `ShowEndingScreen`).
pub(super) fn ends(
    befalls: On<Befalls>,
    readings: Readings,
    (story, factions): (Res<StoryFacts>, Res<Factions>),
    mut commands: Commands,
) {
    if let Consequence::Ends = &befalls.0 {
        let holds = |reading: &Reading| readings.answer(reading, None, &story, &factions) != 0;
        let ending = FACTS.endings.iter().find(|(when, _)| holds(when));
        commands.insert_resource(Ended(ending.map_or(Ending::Untold, |(_, ending)| *ending)));
    }
}

/// What befalls the party's own: one is to be picked, the rest faint or
/// wake, the chosen one speaks, one taken is back, something waits for a
/// battle.
pub(super) fn befalls_the_party(
    befalls: On<Befalls>,
    roster: Single<&PartyRoster>,
    mut members: Query<(&Character, &mut Vitals, Has<Chosen>)>,
    mut points: Query<&mut SpellPoints>,
    mut rng: ResMut<GameRng>,
    mut remarks: MessageWriter<PartyRemark>,
    mut commands: Commands,
) {
    match &befalls.0 {
        Consequence::PickMember => {
            commands.spawn(MemberPick);
        }
        // Everyone living but the chosen one lies senseless (retail
        // `BeginSedexusCapture`), until they are woken.
        Consequence::OthersFaint | Consequence::OthersWake => {
            for member in roster.iter() {
                let Ok((_, mut vitals, chosen)) = members.get_mut(member) else { continue };
                if chosen || vitals.is_dead() {
                    continue;
                }
                if matches!(befalls.0, Consequence::OthersFaint) {
                    vitals.afflict(Condition::Unconscious, Lasting::Indefinite);
                } else {
                    vitals.conditions.lift(Condition::Unconscious);
                }
            }
        }
        Consequence::ChosenRemarks(event) => {
            let chosen = roster.iter().find(|&m| members.get(m).is_ok_and(|(.., chosen)| chosen));
            if let Some(speaker) = chosen {
                remarks.write(PartyRemark { event: *event, after: 0.0, speaker: Some(speaker) });
            }
        }
        Consequence::Returns => {
            for member in roster.iter() {
                if let Ok((_, mut vitals, _)) = members.get_mut(member) {
                    vitals.conditions.lift(Condition::Missing);
                }
            }
        }
        // One of those wanted least, at random (retail
        // `MartensBluff2IdolGasVictim`), is missing from then on.
        Consequence::Takes { spares, wanted } => {
            let score = |member: Entity| {
                let profession = members.get(member).map(|(character, ..)| character.profession);
                let named = wanted.iter().find(|(wanted, _)| Ok(*wanted) == profession);
                named.map_or(1, |(_, score)| *score)
            };
            let open: Vec<Entity> = roster.iter().skip(*spares).collect();
            let least = open.iter().map(|&member| score(member)).min();
            let least: Vec<Entity> =
                open.iter().copied().filter(|&member| Some(score(member)) == least).collect();
            if !least.is_empty() {
                let taken = least[rng.below(least.len() as u32) as usize];
                if let Ok((_, mut vitals, _)) = members.get_mut(taken) {
                    vitals.afflict(Condition::Missing, Lasting::Indefinite);
                }
            }
        }
        Consequence::Every(seconds, reading, rows) => {
            commands.spawn(Recurring {
                timer: Timer::from_seconds(*seconds, TimerMode::Repeating),
                reading: reading.clone(),
                rows: rows.clone(),
            });
        }
        // One living member at random, for so many rounds and up to so
        // many more.
        Consequence::AfflictOne(condition, rounds, spread) => {
            let living: Vec<Entity> = roster
                .iter()
                .filter(|&m| members.get(m).is_ok_and(|(_, vitals, _)| !vitals.is_dead()))
                .collect();
            if !living.is_empty() {
                let one = living[rng.below(living.len() as u32) as usize];
                let lasts = Lasting::Rounds(*rounds + rng.below(*spread));
                if let Ok((_, mut vitals, _)) = members.get_mut(one) {
                    vitals.afflict(*condition, lasts);
                }
            }
        }
        // Every living member's spell points, in full (retail
        // `RestorePartySpellPoints` with nothing held back).
        Consequence::RestoreMagic => {
            for member in roster.iter() {
                let alive = members.get(member).is_ok_and(|(_, vitals, _)| !vitals.is_dead());
                if let (true, Ok(mut points)) = (alive, points.get_mut(member)) {
                    points.restore_evenly(u32::MAX);
                }
            }
        }
        Consequence::Heal(amount) | Consequence::Rest(amount) => {
            for member in roster.iter() {
                let Ok((_, mut vitals, _)) = members.get_mut(member) else { continue };
                if matches!(befalls.0, Consequence::Heal(_)) {
                    vitals.heal(*amount);
                } else {
                    vitals.rest(*amount);
                }
            }
        }
        Consequence::AfterTalk(rows) => {
            commands.spawn(Parted { rows: rows.clone(), spoken: false });
        }
        Consequence::AfterBattle(reading, rows) => {
            commands.spawn(Awaited { reading: reading.clone(), rows: rows.clone(), fought: false });
        }
        _ => {}
    }
}
