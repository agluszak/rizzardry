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
use super::monster::coming;
use super::monster::sight::Sight;
use super::turn::{Ordered, SpecialCooldown, Turn};
use super::{
    Battle, CombatNews, Combatant, Combatants, PASSIVE_ROUNDS, Pause, ROUND_SECONDS, Round, Spoils,
};
use crate::content::spells::Pulsing;
use crate::game::actors::Laid;
use crate::game::actors::Places;
use crate::game::actors::spells::Aged;
use crate::game::actors::spells::{Enchantment, LaidOn, Pulses};
use crate::game::characters::Vitals;
use crate::game::characters::{Character, Condition};
use crate::game::data::Catalogs;
use crate::game::hits::definitions::Rows;
use crate::game::hits::effects::BodyReadOnly;
use crate::game::hits::effects::Strike;
use crate::game::hits::reach::{Field, Recipients};
use crate::game::hits::{Deed, Landed};
use crate::game::magic::{Strength, power};
use crate::game::party::body::PartyBody;
use crate::game::party::recovery::{self, Elapsed, Pace};
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::time::{AGING_TICK_MS, GameClock};
use crate::game::world::hostility::{Side, Stance};
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

/// A round of game time passes for everyone, and the rest of the round's end
/// follows: this is where the steps are put in their order.
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
    // What the pulses do lands as they do, and its deaths are settled.
    commands.trigger(CloudsPulse { fight });
    commands.trigger(Aged { ticks: ticks.aging });
    commands.trigger(FightJudged { fight });
    commands.trigger(PartyLooksAgain { fight });
    commands.trigger(RoundEnded { fight });
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
/// `CheckMonsterGroupsLeaveCombat`): this reads what each group's members
/// make out and where its neighbours stand, and asks [`coming::leaves`] who
/// goes. When no hostile group is left engaged, everyone leaves. One outside
/// any group stays while hostile groups fight on.
pub(super) fn leave_fight(
    leave: On<GroupsLeave>,
    mut fights: Query<(&Combatants, &mut Round)>,
    groups: Query<&GroupMembers>,
    sight: Sight,
) {
    let Ok((combatants, mut round)) = fights.get_mut(leave.fight) else { return };
    let combatants = combatants.entities();
    let hostile = |c: Entity| sight.alive(c) && sight.side(c) == Some(Stance::Hostile);
    let group_of = |monster: Entity| sight.minds.get(monster).ok()?.group.map(|group| group.0);
    let hostiles: Vec<Entity> = combatants.iter().copied().filter(|&c| hostile(c)).collect();
    // A monster outside any group keeps hostile groups engaged.
    let mut engaged =
        combatants.iter().any(|&c| sight.minds.contains(c) && group_of(c).is_none() && hostile(c));
    let mut fought: Vec<Entity> = Vec::new();
    for group in combatants.iter().filter_map(|&c| group_of(c)) {
        if !fought.contains(&group) {
            fought.push(group);
        }
    }
    // Each group fought in, by its members still in the fight.
    let mut going: Vec<Entity> = Vec::new();
    for group in fought {
        let Ok(members) = groups.get(group) else { continue };
        let Some(side) =
            members.iter().find(|m| combatants.contains(m)).and_then(|m| sight.side(m))
        else {
            continue;
        };
        let fellows: Vec<Entity> =
            members.iter().filter(|&m| combatants.contains(&m) && sight.alive(m)).collect();
        let (stays, engages) = coming::stays(&sight, side, &fellows, &hostiles, combatants);
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
}

/// Whoever held back this round (a body that waited, a member who
/// defended), or lies asleep or knocked out, gets its breath back (retail
/// `AdvanceCombatRound`); a Fighter's sheet scales it.
pub(super) fn rest_the_idle(
    rests: On<Rests>,
    fights: Query<&Combatants>,
    mut bodies: Query<(&mut Vitals, Option<&Turn>, Option<&Ordered>, Option<&Character>)>,
    mut rng: ResMut<GameRng>,
) {
    let Ok(combatants) = fights.get(rests.fight) else { return };
    for &body in combatants.entities() {
        let Ok((mut vitals, turn, ordered, character)) = bodies.get_mut(body) else { continue };
        let resting = turn.is_some_and(|turn| turn.waited)
            || ordered.is_some_and(Ordered::defends)
            || matches!(
                vitals.conditions.worst(),
                Some(Condition::Asleep | Condition::Unconscious)
            );
        if vitals.is_dead() || !resting {
            continue;
        }
        let maximum = vitals.stamina.max;
        let rolled = recovery::rest_roll(maximum, &mut rng);
        let gained =
            character.map_or(rolled, |character| recovery::combat_rest_amount(character, rolled));
        if vitals.stamina.current < maximum || vitals.conditions.exhausted() {
            vitals.rest(gained);
        }
    }
}

/// A special attack cools down over the rounds the monster does not make
/// one (retail `AdvanceCombatRound`).
pub(super) fn cool_specials(
    rests: On<Rests>,
    fights: Query<&Combatants>,
    mut monsters: Query<(&mut SpecialCooldown, Option<&Turn>)>,
) {
    let Ok(combatants) = fights.get(rests.fight) else { return };
    for &monster in combatants.entities() {
        if let Ok((mut cooldown, turn)) = monsters.get_mut(monster)
            && cooldown.0 > 0
            && !turn.is_some_and(|turn| turn.special)
        {
            cooldown.0 -= 1;
        }
    }
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
    fights: Query<&Combatants>,
    pulsing: Query<(&Enchantment, &LaidOn, &Pulses)>,
    bodies: Query<BodyReadOnly>,
    grouping: (Query<&'static GroupMember>, Query<&'static GroupMembers>),
    places: Places,
    party: Single<(Entity, &Position, &PartyRoster)>,
    obstacles: WorldObstacles,
    content: Res<Catalogs>,
) {
    let (party, position, roster) = *party;
    let Ok(combatants) = fights.get(pulse.fight) else { return };
    let alive = |body: Entity| bodies.get(body).is_ok_and(|body| !body.vitals.is_dead());
    let field = Field {
        bodies: &bodies,
        places: &places,
        obstacles: &obstacles,
        roster,
        combatants: combatants.entities(),
        memberships: &grouping.0,
        groups: &grouping.1,
    };
    // The party's first, then each monster's, in the order clouds pulse.
    let mut order: Vec<_> = pulsing.iter().collect();
    order.sort_by_key(|(_, on, pulse)| (on.0 != party, pulse.0.order));
    for (laid, &LaidOn(bearer), &Pulses(pulse)) in order {
        let rows = Rows::Pulse(laid.spell);
        if rows.effects(&content).is_empty() {
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
            combatants.entities().iter().copied().filter(|&m| m == bearer && alive(m)).collect()
        };
        let strength = if around {
            Strength { power: laid.power, boost: 0, against: laid.power * 3 }
        } else {
            power::strength(0, laid.power, laid.boost, &content.spells[laid.spell])
        };
        commands.spawn((
            Deed::at(Strike { rows, strength, blow: None }, victims),
            crate::game::magic::landed::Magical,
            Pulse,
        ));
    }
}

/// A deed that is a cloud's pulse.
#[derive(Component)]
pub(super) struct Pulse;

/// A cloud's pulse that did something to someone keeps the fight going as
/// an attack would.
pub(super) fn count_pulse(
    landed: On<Landed>,
    pulses: Query<(), With<Pulse>>,
    mut rounds: Query<&mut Round, With<Battle>>,
) {
    if pulses.contains(landed.deed)
        && let Ok(mut round) = rounds.single_mut()
    {
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
    mut fights: Query<(&mut Battle, &mut Round, &Spoils, &mut Pause, &Combatants)>,
    mut members: Query<(&mut Character, &Vitals)>,
    monsters: Query<(
        &crate::game::actors::Attacks,
        &Vitals,
        &Side,
        Option<&crate::game::world::navigation::Walker>,
    )>,
    places: Places,
    roster: Single<&PartyRoster>,
    mut news: MessageWriter<CombatNews>,
) {
    let Ok((mut battle, mut round, spoils, mut pause, combatants)) = fights.get_mut(judged.fight)
    else {
        return;
    };
    let living: Vec<Entity> =
        roster.iter().filter(|&m| members.get(m).is_ok_and(|(_, v)| !v.is_dead())).collect();
    if living.is_empty() {
        return;
    }
    pause.0 = Timer::from_seconds(ROUND_SECONDS, TimerMode::Once);
    let approaching = combatants.entities().iter().any(|&entity| {
        monsters.get(entity).is_ok_and(|(arms, vitals, _, walker)| {
            let gap = roster.first().map_or(f32::INFINITY, |&member| places.gap(entity, member));
            monster::close::needs_approach(
                !vitals.is_dead() && walker.is_some_and(|walker| walker.speed > 0.0),
                arms.reach(),
                gap,
            )
        })
    });
    let active = round.attacked || approaching;
    battle.passive_rounds = if active { 0 } else { battle.passive_rounds + 1 };
    let hostiles_left = combatants.entities().iter().any(|&entity| {
        !round.leaving.contains(&entity)
            && monsters.get(entity).is_ok_and(|(_, vitals, side, ..)| {
                !vitals.is_dead() && side.now(vitals) == Stance::Hostile
            })
    });
    if hostiles_left && battle.passive_rounds < PASSIVE_ROUNDS {
        return;
    }
    let each = spoils.experience / living.len() as u32;
    if each > 0 {
        for &member in &living {
            members.get_mut(member).unwrap().0.award_experience(each);
        }
    }
    news.write(if spoils.kills > 0 {
        CombatNews::Won { kills: spoils.kills, experience_each: each }
    } else {
        CombatNews::Over
    });
    round.over = true;
}

/// Recognition is part of each round's end, before the next accepted action
/// or the fight's end can change its meaning: the party looks again at who
/// it fights.
pub(super) fn look_again(
    looks: On<PartyLooksAgain>,
    fights: Query<&Combatants>,
    party: Query<(Entity, &Position, &PartyBody)>,
    laid: Laid,
    mut recognition: GroupRecognition,
    mut rng: ResMut<GameRng>,
) {
    let Ok((party, position, body)) = party.single() else { return };
    let Ok(combatants) = fights.get(looks.fight) else { return };
    recognition.detect(
        PartyBody::eye(position),
        body.forward(),
        Some(combatants.entities()),
        laid.bonus(party).warns,
        false,
        &mut rng,
    );
}

/// The round has ended: those leaving leave, the next round waits to be
/// accepted, and a fight that is over goes; what lasted only the fight is
/// lifted as it does.
pub fn end_round(ended: On<RoundEnded>, mut commands: Commands, mut fights: Query<&mut Round>) {
    let Ok(mut round) = fights.get_mut(ended.fight) else { return };
    for monster in round.leaving.drain(..) {
        commands.entity(monster).remove::<Combatant>();
    }
    round.playing = false;
    round.moment = 0;
    if std::mem::take(&mut round.over) {
        commands.entity(ended.fight).despawn();
    }
}
