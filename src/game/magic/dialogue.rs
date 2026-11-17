//! Charm and Mindread (`docs/rules/casting.md` section 8), cast at the one the
//! party is talking with while they wait for the party's word. The cast is paid
//! for and rolled as any other ([`super::casting`]); this is what it then does:
//! the speaker's answer is [`super::workings::speak`], made to happen to the
//! conversation.

use super::casting::{Allowed, CastBy, CastSpell, Chosen, MagicNews, Worked, marked};
use super::may::Refusal;
use super::reach::Aim;
use super::reach::Marked;
use super::workings::{self, Reply};
use crate::content::characters::{Attribute, Realm};
use crate::content::spells::Working;
use crate::game::actors::{Attributes, Level, Resistances};
use crate::game::characters::Vitals;
use crate::game::difficulty::Difficulty;
use crate::game::hits::Effect;
use crate::game::npcs::Npc;
use crate::game::npcs::People;
use crate::game::npcs::conversation::{Continue, Conversation, Script};
use crate::game::rng::GameRng;
use crate::game::world::hostility::Standing;
use crate::game::world::level::{InPlay, LevelOwned};
use crate::game::world::population::Monster;
use bevy::prelude::*;

/// Charm and Mindread are cast at the living body of the one the party is
/// talking with, in the level played.
#[allow(clippy::type_complexity)]
pub fn mark(
    allowed: On<Allowed>,
    casts: Query<(&CastBy, &Chosen)>,
    conversation: Option<Single<&Conversation>>,
    people: People,
    bodies: Query<(&Vitals, &LevelOwned), With<Monster>>,
    played: Query<(), With<InPlay>>,
    mut commands: Commands,
) -> Result {
    let (&CastBy(caster), &Chosen(choice)) = casts.get(allowed.cast)?;
    let speaker =
        conversation.and_then(|conversation| people.body(conversation.actor)).filter(|&body| {
            bodies
                .get(body)
                .is_ok_and(|(vitals, level)| !vitals.is_dead() && played.contains(level.0))
        });
    let aimed = match choice.aim {
        Aim::Body(aimed) if speaker == Some(aimed) => Ok(Marked::Body(aimed)),
        _ => Err(Refusal::NoTarget),
    };
    marked(&mut commands, (allowed.cast, caster), aimed);
    Ok(())
}

/// Charm or Mindread, once cast at the one the party is talking with: a failed
/// save charms them by the cast's amount or opens their mind; a backfire may be
/// noticed.
pub fn cast(
    worked: On<Worked>,
    casts: Query<&CastSpell>,
    conversation: Single<(Entity, &mut Conversation, &mut Script)>,
    mut speakers: Query<&mut Npc>,
    minds: Query<(&Level, &Resistances, &Attributes, &Standing)>,
    difficulty: Res<Difficulty>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<MagicNews>,
    mut commands: Commands,
) -> Result {
    let (talk, mut conversation, mut script) = conversation.into_inner();
    let Worked { cast, mut report, strength, marked, .. } = worked.event().clone();
    let CastSpell(spell) = casts.get(cast)?;
    let mindread = spell.facts.working == Some(Working::Mindread);
    let Marked::Body(body) = marked else { return Ok(()) };
    let mut npc = speakers.get_mut(conversation.actor)?;
    let (level, resistances, attributes, standing) = minds.get(body)?;
    let spoken = workings::speak(
        &mut rng,
        (mindread, report.outcome),
        strength,
        *difficulty,
        (level.0, resistances.realms[Realm::Mental]),
        (attributes[Attribute::Senses], attributes[Attribute::Intelligence]),
        standing.stance,
        (conversation.charm(), npc.mindread_refusals),
    );
    npc.mindread_refusals = spoken.refusals;
    if spoken.resisted {
        report.effects.push(Effect::Resisted { victim: body });
    }
    // One won over greets the party as a friend as the conversation goes on.
    match spoken.reply {
        Reply::Unnoticed => {}
        Reply::Quote(quote) => script.say(quote),
        Reply::Charmed(amount) => conversation.charmed(amount),
    }
    commands.trigger(Continue { conversation: talk });
    news.write(MagicNews::Done(report));
    Ok(())
}
