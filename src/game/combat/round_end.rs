//! A round's end (retail `EndCombatRound`, `AdvanceCombatRound`,
//! `AdvanceTimedEffects`). Once everyone has acted, nothing is in flight
//! and the last pause is over, the round ends ([`RoundEnds`]), and what
//! follows is one chain, each step its own event with its own observers, in
//! the order of `docs/rules/round.md`: a round of game time passes
//! ([`Elapsed`]); groups with nothing left to fight leave; those who held
//! back get their breath and special attacks cool; the clouds pulse, and
//! what the pulses did lands and its deaths are settled as they do
//! ([`Fell`]); the spells laid age ([`Aged`]); whether the fight is over;
//! the party looks again at who it fights; and the round is ended.
//! Observers are not ordered, so the order is the order the steps are
//! triggered in, each one's work done before the next is begun.

use avian3d::prelude::Position;
use bevy::prelude::*;

use super::monster;
use super::monster::sight::Sight;
use super::turn::{SpecialCooldown, Turn};
use super::{
    Battle, CombatNews, Combatant, Combatants, PASSIVE_ROUNDS, Pause, ROUND_SECONDS, Round, Spoils,
};
use crate::content::spells::Pulsing;
use crate::game::actors::Body;
use crate::game::actors::Laid;
use crate::game::actors::Places;
use crate::game::actors::spells::Aged;
use crate::game::actors::spells::{LaidOn, Pulses};
use crate::game::actors::time::{Elapsed, Mending, Pace};
use crate::game::characters::Condition;
use crate::game::characters::Vitals;
use crate::game::hits::reach::{Field, Recipients};
use crate::game::hits::{Deed, Landed};
use crate::game::party::body::PartyBody;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::time::{AGING_TICK_MS, GameClock};
use crate::game::world::hostility::Stance;
use crate::game::world::identify::{GroupMember, GroupMembers, GroupRecognition};
use crate::game::world::obstacles::WorldObstacles;
use bevy::ecs::relationship::RelationshipTarget;

/// The timeline is empty and nothing is in the air or walking: the round
/// ends.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct RoundEnds {
    #[event_target]
    pub fight: Entity,
}

/// A round of game time passes for everyone, and what the round's end does
/// to bodies follows, in this order. The round then waits, [`Closing`],
/// until what that did is settled.
pub(super) fn end_the_round(
    ends: On<RoundEnds>,
    mut commands: Commands,
    mut clock: ResMut<GameClock>,
) {
    let fight = ends.fight;
    let ticks = clock.advance(AGING_TICK_MS);
    commands.trigger(Elapsed { ticks, pace: Pace::Fighting });
    commands.trigger(GroupsLeave { fight });
    commands.trigger(Rests { fight });
    commands.trigger(CloudsPulse { fight });
    commands.trigger(Aged { ticks: ticks.aging });
    commands.entity(fight).insert(Closing);
}

/// A fight whose round has done all it does to bodies, and waits for what
/// followed from that (the fallen, the spent) to be settled before it is
/// judged.
#[derive(Component, Clone, Copy, Debug)]
pub struct Closing;

/// What the round's end did is settled: the fight is judged, the party
/// looks again at who it fights, and the round is over.
pub(super) fn close_round(fights: Query<Entity, With<Closing>>, mut commands: Commands) {
    for fight in &fights {
        commands.entity(fight).remove::<Closing>();
        commands.trigger(FightJudged { fight });
        commands.trigger(PartyLooksAgain { fight });
        commands.trigger(RoundEnded { fight });
    }
}

/// Groups with nothing left to fight for leave, as the round ends.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct GroupsLeave {
    #[event_target]
    pub fight: Entity,
}

/// Those who held back get their breath, and special attacks cool.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Rests {
    #[event_target]
    pub fight: Entity,
}

/// What burns or hangs over someone pulses on them.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct CloudsPulse {
    #[event_target]
    pub fight: Entity,
}

/// Whether the fight is over is judged.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct FightJudged {
    #[event_target]
    pub fight: Entity,
}

/// The party looks again at who it fights.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct PartyLooksAgain {
    #[event_target]
    pub fight: Entity,
}

/// The round is over: those leaving have left, and a fight that is over
/// goes.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct RoundEnded {
    #[event_target]
    pub fight: Entity,
}

/// Groups with nothing left to fight leave it (retail
/// `CheckMonsterGroupsLeaveCombat`), each by its members still fighting: a
/// hostile group stays while a member makes out the party or sees a foe of
/// its own, and keeps the fight engaged only by making out the party; a
/// friendly one stays while a member sees a foe; a neutral one while a
/// member stands with the fight. When no hostile group is left engaged,
/// everyone leaves. A foe of its own a member sees is another fighting
/// monster against it that its eyes make out, or the party, if a member
/// stands against it and its eyes make the party out.
#[allow(clippy::type_complexity)]
pub(super) fn leave_fight(
    leave: On<GroupsLeave>,
    mut fights: Query<(Option<&Combatants>, &mut Round)>,
    groups: Query<&GroupMembers>,
    members: Query<(), With<crate::game::party::roster::PartyMember>>,
    making_out: Query<(), With<super::monster::sight::MakesOutParty>>,
    reinforcing: Query<(), With<super::monster::sight::Reinforces>>,
    sight: Sight,
) -> Result {
    let (combatants, mut round) = fights.get_mut(leave.fight)?;
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    let group_of = |monster: Entity| sight.groups.get(monster).ok().map(|group| group.0);
    let sees_foe = |m: Entity| {
        combatants.iter().any(|&other| {
            let foe = sight.toward(m, other) == Some(Stance::Hostile);
            foe && match members.contains(other) {
                true => making_out.contains(m),
                false => sight.sees(m, other),
            }
        })
    };
    let mut fought: Vec<Entity> = Vec::new();
    for group in combatants.iter().filter_map(|&c| group_of(c)) {
        if !fought.contains(&group) {
            fought.push(group);
        }
    }
    let mut engaged = false;
    let mut going: Vec<Entity> = Vec::new();
    for group in fought {
        let Ok(group_members) = groups.get(group) else { continue };
        let fellows: Vec<Entity> =
            group_members.iter().filter(|&m| combatants.contains(&m) && sight.alive(m)).collect();
        let Some(side) = fellows.first().and_then(|&m| sight.side(m)) else {
            going.push(group);
            continue;
        };
        let (stays, engages) = match side {
            Stance::Neutral => (fellows.iter().any(|&m| reinforcing.contains(m)), false),
            Stance::Hostile => {
                let engages = fellows.iter().any(|&m| making_out.contains(m));
                (engages || fellows.iter().any(|&m| sees_foe(m)), engages)
            }
            Stance::Friendly => (fellows.iter().any(|&m| sees_foe(m)), false),
        };
        engaged |= engages;
        if !stays {
            going.push(group);
        }
    }
    round.leaving = combatants
        .iter()
        .copied()
        .filter(|&combatant| !engaged || group_of(combatant).is_some_and(|g| going.contains(&g)))
        .collect();
    Ok(())
}

/// Whoever held back this round (a body that waited, a member who
/// defended), or lies asleep or knocked out, gets its breath back (retail
/// `AdvanceCombatRound`): two dice of a twentieth of its most, less one,
/// from one to 255 sides, which one who gets their wind back fast scales.
pub(super) fn rest_the_idle(
    rests: On<Rests>,
    fights: Query<Option<&Combatants>>,
    mut bodies: Query<(
        (&mut Vitals, &crate::game::characters::Conditions),
        Option<&Turn>,
        &Mending,
    )>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let combatants = fights.get(rests.fight)?;
    for &body in combatants.map_or(&[][..], Combatants::entities) {
        let ((mut vitals, conditions), turn, mending) = bodies.get_mut(body)?;
        let resting = turn.is_some_and(|turn| turn.waited)
            || matches!(conditions.worst(), Some(Condition::Asleep | Condition::Unconscious));
        if vitals.is_dead() || !resting {
            continue;
        }
        let maximum = vitals.stamina.max;
        let sides = (maximum / 20).clamp(2, 256) as i32 - 1;
        let rolled = rng.roll(crate::content::items::Dice { count: 2, sides, base: 0 });
        let gained = mending.tireless.map_or(rolled, |scale| (scale * rolled as f32) as u32);
        if vitals.stamina.current < maximum || conditions.exhausted() {
            vitals.rest(gained);
        }
    }
    Ok(())
}

/// A special attack cools down over the rounds the monster does not make
/// one (retail `AdvanceCombatRound`).
pub(super) fn cool_specials(
    rests: On<Rests>,
    fights: Query<Option<&Combatants>>,
    mut monsters: Query<(Option<&mut SpecialCooldown>, Option<&Turn>)>,
) -> Result {
    let combatants = fights.get(rests.fight)?;
    for &monster in combatants.map_or(&[][..], Combatants::entities) {
        // Only one with a special attack has a cooldown.
        if let (Some(mut cooldown), turn) = monsters.get_mut(monster)?
            && cooldown.0 > 0
            && !turn.is_some_and(|turn| turn.special)
        {
            cooldown.0 -= 1;
        }
    }
    Ok(())
}

/// Each pulsing spell laid pulses on whoever it hangs over (retail
/// `AdvanceTimedEffects`; a spell's last turn still pulses): one on the
/// party on its living members, one on a monster on it, and Ring of Fire
/// on the monsters close around the party, sampled anew each pulse. A pulse
/// is a deed with no caster. A character without a seat is not among those
/// close around the party.
pub(super) fn pulse_clouds(
    pulse: On<CloudsPulse>,
    mut commands: Commands,
    fights: Query<Option<&Combatants>>,
    pulsing: Query<(&LaidOn, &Pulses)>,
    bodies: Query<Body>,
    grouping: (Query<&'static GroupMember>, Query<&'static GroupMembers>),
    places: Places,
    party: Single<(Entity, &Position, &PartyRoster)>,
    obstacles: WorldObstacles,
) -> Result {
    let (party, position, roster) = *party;
    let combatants = fights.get(pulse.fight)?.map_or(&[][..], Combatants::entities);
    let alive = |body: Entity| bodies.get(body).is_ok_and(|body| !body.vitals.is_dead());
    let field = Field {
        bodies: &bodies,
        places: &places,
        obstacles: &obstacles,
        roster,
        combatants,
        memberships: &grouping.0,
        groups: &grouping.1,
    };
    // The party's first, then each monster's, in the order clouds pulse.
    let mut order: Vec<_> = pulsing.iter().collect();
    order.sort_by_key(|(on, pulses)| (on.0 != party, pulses.pulse.order));
    for (&LaidOn(bearer), Pulses { pulse, strike, .. }) in order {
        if strike.effects.is_empty() {
            continue;
        }
        let around = pulse.on == Pulsing::AroundParty;
        let victims: Vec<Entity> = if around {
            let centre = PartyBody::centre(PartyBody::eye(position));
            let reach =
                crate::game::party::body::RADIUS + crate::content::species::Reach::Short.metres();
            field.within(centre, centre, reach, Recipients::Opponents(Stance::Friendly))
        } else if bearer == party {
            roster.iter().filter(|&member| alive(member)).collect()
        } else {
            combatants.iter().copied().filter(|&m| m == bearer && alive(m)).collect()
        };
        // Watched before it lands, which it does as it is made.
        commands
            .spawn(crate::game::magic::landed::Magical)
            .observe(count_pulse)
            .insert(Deed::at(strike.clone(), victims));
    }
    Ok(())
}

/// A cloud's pulse that did something to someone keeps the fight going as
/// an attack would: its deed's own observer.
fn count_pulse(landed: On<Landed>, round: Option<Single<&mut Round, With<Battle>>>) {
    if let Some(mut round) = round {
        round.attacked(landed.affected());
    }
}

/// The fight is over when no hostile stays in it, or nobody attacked or
/// closed in for two rounds; the living, the unconscious included, share
/// its experience. Poison may have killed the last of the party as the
/// round passed: that is their defeat, not a victory.
#[allow(clippy::type_complexity)]
pub(super) fn judge_victory(
    judged: On<FightJudged>,
    mut fights: Query<(&mut Battle, &mut Round, &Spoils, &mut Pause, Option<&Combatants>)>,
    mut members: Query<(&mut crate::game::characters::Experience, &Vitals)>,
    monsters: Query<(
        &crate::game::actors::arm::Furthest,
        &Vitals,
        &crate::game::world::hostility::Standing,
        Option<&crate::game::world::navigation::Walker>,
    )>,
    places: Places,
    roster: Single<&PartyRoster>,
    mut news: MessageWriter<CombatNews>,
) -> Result {
    let (mut battle, mut round, spoils, mut pause, combatants) = fights.get_mut(judged.fight)?;
    // A fight nobody is in is not judged.
    let Some(combatants) = combatants else { return Ok(()) };
    let living: Vec<Entity> =
        roster.iter().filter(|&m| members.get(m).is_ok_and(|(_, v)| !v.is_dead())).collect();
    if living.is_empty() {
        return Ok(());
    }
    pause.0 = Timer::from_seconds(ROUND_SECONDS, TimerMode::Once);
    let approaching = combatants.entities().iter().any(|&entity| {
        monsters.get(entity).is_ok_and(|(arms, vitals, _, walker)| {
            let gap = roster.first().map_or(f32::INFINITY, |&member| places.gap(entity, member));
            monster::close::needs_approach(
                !vitals.is_dead() && walker.is_some_and(|walker| walker.speed > 0.0),
                arms.0.unwrap_or(crate::content::species::Reach::Touch),
                gap,
            )
        })
    });
    let active = round.attacked || approaching;
    battle.passive_rounds = if active { 0 } else { battle.passive_rounds + 1 };
    let hostiles_left = combatants.entities().iter().any(|&entity| {
        !round.leaving.contains(&entity)
            && monsters.get(entity).is_ok_and(|(_, vitals, standing, ..)| {
                !vitals.is_dead() && standing.stance == Stance::Hostile
            })
    });
    if hostiles_left && battle.passive_rounds < PASSIVE_ROUNDS {
        return Ok(());
    }
    let each = spoils.experience / living.len() as u32;
    if each > 0 {
        for &member in &living {
            members.get_mut(member).unwrap().0.earned += u64::from(each);
        }
    }
    news.write(if spoils.kills > 0 {
        CombatNews::Won { kills: spoils.kills, experience_each: each }
    } else {
        CombatNews::Over
    });
    round.over = true;
    Ok(())
}

/// Recognition is part of each round's end, before the next accepted action
/// or the fight's end can change its meaning: the party looks again at who
/// it fights.
pub(super) fn look_again(
    looks: On<PartyLooksAgain>,
    fights: Query<Option<&Combatants>>,
    party: Single<(Entity, &Position, &PartyBody)>,
    laid: Laid,
    mut recognition: GroupRecognition,
    mut rng: ResMut<GameRng>,
) -> Result {
    let (party, position, body) = *party;
    let combatants = fights.get(looks.fight)?.map_or(&[][..], Combatants::entities);
    recognition.detect(
        PartyBody::eye(position),
        body.forward(),
        Some(combatants),
        laid.bonus(party).warns,
        false,
        &mut rng,
    );
    Ok(())
}

/// The round has ended: those leaving leave, the next round waits to be
/// accepted, and a fight that is over goes; what lasted only the fight is
/// lifted as it does.
pub fn end_round(
    ended: On<RoundEnded>,
    mut commands: Commands,
    mut fights: Query<&mut Round>,
) -> Result {
    let mut round = fights.get_mut(ended.fight)?;
    for monster in round.leaving.drain(..) {
        commands.entity(monster).remove::<Combatant>();
    }
    round.playing = false;
    round.moment = 0;
    if std::mem::take(&mut round.over) {
        commands.entity(ended.fight).despawn();
    }
    Ok(())
}
