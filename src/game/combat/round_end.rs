//! A round's end (retail `EndCombatRound`, `AdvanceCombatRound`,
//! `AdvanceTimedEffects`). Once everyone has acted, nothing is in flight
//! and the last pause is over, [`judge_round`] passes a round's game time
//! and marks the fight [`RoundEnding`]; then, each its own system: groups
//! with nothing left to fight leave, the round's rest, cooldowns, aging and
//! recovery, the offensive clouds' pulses, spells aging, the defenders'
//! rest, whether the fight is over, the party looking again at who it
//! fights, and the end of the fight.

use avian3d::prelude::Position;
use bevy::prelude::*;

use super::approach::{self, CombatMove};
use super::round::{MonsterTurn, PartyTurn, SpecialCooldown};
use super::sight::{Seen, Sight, reinforces};
use super::swings::{allegiance, toward};
use super::{
    Battle, CombatNews, Combatant, Combatants, PASSIVE_ROUNDS, PartyMove, ROUND_SECONDS, RoundPhase,
};
use crate::content::spells::SpellId;
use crate::game::characters::{Character, Condition, Fate};
use crate::game::data::Catalogs;
use crate::game::factions::Factions;
use crate::game::hits::effects::InFlight;
use crate::game::hits::prototypes::{Definitions, Uses};
use crate::game::hits::reach::{Recipients, radius};
use crate::game::hits::spells::reach;
use crate::game::hits::{Act, Body, Deed};
use crate::game::magic::Strength;
use crate::game::magic::casting::Cast;
use crate::game::magic::enchantments::{OFFENSIVE, PartyEffects};
use crate::game::party::body::PartyBody;
use crate::game::party::recovery;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::time::{GameClock, Ticks};
use crate::game::world::hostility::{self, Stance};
use crate::game::world::identify::{GroupMember, GroupRecognition, MonsterGroup};
use crate::game::world::level::{Level, PlayedLevel};
use crate::game::world::obstacles::WorldObstacles;

/// The round is ending, a round's game time having passed.
#[derive(Component, Debug)]
pub struct RoundEnding(Ticks);

/// The monster's group leaves the fight as the round ends.
#[derive(Component, Debug)]
pub struct Leaves;

/// The fight is over as the round ends.
#[derive(Component, Debug)]
pub struct FightOver;

/// Once everyone has acted, the party's move is done, the last action's
/// pause is over and nothing flies, walks or is being cast, the round ends:
/// a round's game time passes, one aging tick.
#[allow(clippy::type_complexity)]
pub fn judge_round(
    mut commands: Commands,
    battle: Option<Single<(Entity, &Battle), Without<RoundEnding>>>,
    busy: (Query<(), With<InFlight>>, Query<(), With<CombatMove>>, Query<(), With<Cast>>),
    mut clock: ResMut<GameClock>,
) {
    let Some((fight, battle)) = battle.map(Single::into_inner) else { return };
    let RoundPhase::Executing(step) = battle.phase else { return };
    if step <= 100
        || matches!(battle.party_move, PartyMove::Moving { .. })
        || !battle.wait.is_finished()
        || !busy.0.is_empty()
        || !busy.1.is_empty()
        || !busy.2.is_empty()
    {
        return;
    }
    let ticks = clock.advance(crate::game::time::AGING_TICK_MS);
    commands.entity(fight).insert(RoundEnding(ticks));
}

/// Groups with nothing left to fight leave it (retail
/// `CheckMonsterGroupsLeaveCombat`): a neutral group once it no longer
/// stands with the fight ([`reinforces`]); a friendly group unless one of
/// its members sees an enemy; a hostile group unless a living member sees
/// the party. When no hostile group is left engaged, everyone leaves. One
/// outside any group stays while hostile groups fight on.
#[allow(clippy::too_many_arguments)]
pub(super) fn leave_fight(
    mut commands: Commands,
    fight: Single<&Combatants, With<RoundEnding>>,
    mut bodies: Query<Body>,
    memberships: Query<&GroupMember>,
    (stand, roster, party, level): (
        Single<&Position, With<PartyBody>>,
        Single<&PartyRoster>,
        Single<&PartyEffects>,
        PlayedLevel,
    ),
    (groups, obstacles): (Query<&MonsterGroup>, WorldObstacles),
    content: If<Res<Catalogs>>,
    (factions, clock): (Res<Factions>, Res<GameClock>),
) {
    let combatants = fight.entities();
    let (species, sides) = (&content.species, (&groups, &*factions, clock.ms));
    let members: Vec<_> = roster
        .iter()
        .filter_map(|member| {
            let body = bodies.get(member).ok()?;
            Some((body.character?.clone(), body.vitals.clone()))
        })
        .collect();
    let sight = Sight {
        eye: PartyBody::eye(*stand),
        far: level.view_distance().unwrap_or(0.0),
        lowest_level: crate::game::party::lowest_level(members.iter().map(|(c, v)| (c, v))),
        concealment: party.0.concealment(),
        obstacles: &obstacles,
    };
    let group_of = |monster: Entity| memberships.get(monster).ok().map(|m| m.0);
    let seen = |monster: Entity, bodies: &mut Query<Body>| -> Option<(Seen, bool, Stance)> {
        let body = bodies.get(monster).ok()?;
        let me = body.monster?;
        let kind = &species[me.species];
        let pose = body.pose.as_ref()?;
        if body.is_dead() {
            return None;
        }
        let stance = hostility::stance(
            body.group.and_then(|g| groups.get(g.0).ok()).and_then(|g| g.set),
            kind,
            &factions,
            clock.ms,
            body.fighting,
        );
        let seen = sight.seen(monster, ((me, body.vitals), pose, body.walker, kind));
        Some((seen, kind.temper.camouflaged, stance))
    };
    let hostiles: Vec<Seen> = combatants
        .iter()
        .filter_map(|&c| seen(c, &mut bodies))
        .filter(|(s, _, stance)| *stance == Stance::Hostile && s.in_view)
        .map(|(s, ..)| s)
        .collect();
    let (mut checked, mut leaving, mut engaged) = (Vec::new(), Vec::new(), false);
    for &combatant in combatants {
        let stance = seen(combatant, &mut bodies).map(|(.., stance)| stance);
        let Some(group) = group_of(combatant) else {
            engaged |= stance == Some(Stance::Hostile);
            continue;
        };
        if checked.contains(&group) {
            continue;
        }
        checked.push(group);
        let fellows: Vec<(Seen, bool)> = combatants
            .iter()
            .filter(|&&c| group_of(c) == Some(group))
            .filter_map(|&c| seen(c, &mut bodies).map(|(s, camouflaged, _)| (s, camouflaged)))
            .collect();
        let stays = match stance {
            Some(Stance::Neutral) => reinforces(&fellows, &hostiles),
            Some(Stance::Hostile) => {
                let sees = fellows.iter().any(|(s, _)| s.sees);
                engaged |= sees;
                sees
            }
            Some(Stance::Friendly) => fellows.iter().any(|(fellow, _)| {
                let observer = bodies.get(fellow.entity).unwrap();
                let mine = allegiance(&observer, species, sides).unwrap();
                let watcher = observer.monster.unwrap().clone();
                let vitals = observer.vitals.clone();
                let feet = observer.feet(sight.eye);
                let forward = observer.pose.as_ref().unwrap().forward();
                combatants.iter().any(|&enemy| {
                    bodies.get(enemy).is_ok_and(|target| {
                        let them = target.monster.as_ref().unwrap();
                        !target.is_dead()
                            && toward(mine, &target, species, sides) == Stance::Hostile
                            && sight.sees_from(
                                (&watcher, &vitals, &species[watcher.species]),
                                (feet, forward),
                                (them, target.feet(sight.eye), &species[them.species]),
                            )
                    })
                })
            }),
            None => false,
        };
        if !stays {
            leaving.push(group);
        }
    }
    for &combatant in combatants {
        if !engaged || group_of(combatant).is_some_and(|g| leaving.contains(&g)) {
            commands.entity(combatant).insert(Leaves);
        }
    }
}

/// A monster that waited, or lies asleep or knocked out, gets its breath
/// back (retail `AdvanceCombatRound`).
pub(super) fn rest_waiters(
    fight: Single<&Combatants, With<RoundEnding>>,
    mut monsters: Query<(&mut crate::game::characters::Vitals, Option<&MonsterTurn>)>,
    mut rng: ResMut<GameRng>,
) {
    for &monster in fight.entities() {
        let Ok((mut vitals, turn)) = monsters.get_mut(monster) else { continue };
        let resting = turn.is_some_and(|turn| turn.waited)
            || vitals
                .conditions
                .iter()
                .any(|&(c, _)| matches!(c, Condition::Asleep | Condition::Unconscious));
        if resting {
            let maximum = vitals.stamina.max;
            vitals.rest(recovery::rest_roll(maximum, &mut rng));
        }
    }
}

/// A special attack cools down over the rounds the monster does not make
/// one (retail `AdvanceCombatRound`).
pub(super) fn cool_specials(
    fight: Single<&Combatants, With<RoundEnding>>,
    mut monsters: Query<(&mut SpecialCooldown, Option<&MonsterTurn>)>,
) {
    for &monster in fight.entities() {
        if let Ok((mut cooldown, turn)) = monsters.get_mut(monster)
            && cooldown.0 > 0
            && !turn.is_some_and(|turn| turn.special)
        {
            cooldown.0 -= 1;
        }
    }
}

/// The fighting monsters' conditions age by the round's tick.
pub(super) fn age_conditions(
    fight: Single<(&Combatants, &RoundEnding)>,
    mut monsters: Query<&mut crate::game::characters::Vitals>,
) {
    let (combatants, ending) = *fight;
    for &monster in combatants.entities() {
        if let Ok(mut vitals) = monsters.get_mut(monster)
            && vitals.conditions.any_timed()
        {
            vitals.conditions.age(ending.0.aging);
        }
    }
}

/// The members recover as a fighting round allows: spell points at half
/// rate, nothing else.
pub(super) fn recover_members(
    fight: Single<(Entity, &RoundEnding)>,
    roster: Single<&PartyRoster>,
    mut members: Query<(
        &mut Character,
        &mut crate::game::characters::Vitals,
        &crate::game::magic::LearnedSpells,
        &mut crate::game::magic::SpellPoints,
    )>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    let (fight, ending) = *fight;
    for member in roster.iter() {
        let Ok((mut profile, mut vitals, known, mut pool)) = members.get_mut(member) else {
            continue;
        };
        recovery::pass(
            &mut profile,
            &mut vitals,
            ending.0,
            recovery::Pace::Fighting,
            (known, &mut pool),
            &content.spells,
            &mut Fate { fight: Some(fight), rng: &mut rng },
        );
    }
}

/// The offensive clouds pulse (retail `AdvanceTimedEffects`, after
/// personal aging and before the slots expire; their last turn still
/// pulses): the party's on its living members, each monster's own on it,
/// and Ring of Fire on the monsters close around the party, sampled anew
/// each pulse. Each pulse is an act with no caster.
#[allow(clippy::too_many_arguments)]
pub(super) fn pulse_clouds(
    mut commands: Commands,
    fight: Single<&Combatants, With<RoundEnding>>,
    mut bodies: Query<Body>,
    party: Single<(&Position, &PartyRoster, &PartyEffects)>,
    (levels, groups, obstacles): (Query<(), With<Level>>, Query<&MonsterGroup>, WorldObstacles),
    (content, definitions): (If<Res<Catalogs>>, Res<Definitions>),
    (factions, clock): (Res<Factions>, Res<GameClock>),
) {
    let (position, roster, effects) = *party;
    let eye = PartyBody::eye(position);
    let pulse = |commands: &mut Commands, spell: SpellId, strength, victims: &[Entity]| {
        let Some(uses) = definitions.pulse(spell) else { return };
        let act = commands.spawn((Deed::new(Act::Triggered { spell }), Uses(uses), strength)).id();
        let victims: Vec<_> = victims.iter().map(|&victim| (victim, None)).collect();
        reach(commands, act, &victims);
    };
    let living: Vec<Entity> =
        roster.iter().filter(|&m| bodies.get(m).is_ok_and(|b| !b.is_dead())).collect();
    for id in OFFENSIVE {
        if let Some(effect) = effects.0.get(id) {
            let strength = Strength::at(0, &content.spells[id], effect.power, effect.boost);
            pulse(&mut commands, id, strength, &living);
        }
    }
    for &monster in fight.entities() {
        let Ok(body) = bodies.get(monster) else { continue };
        if body.is_dead() {
            continue;
        }
        let clouds: Vec<_> = OFFENSIVE
            .iter()
            .filter_map(|&id| body.vitals.enchantments.get(id).map(|e| (id, *e)))
            .collect();
        for (id, effect) in clouds {
            let strength = Strength::at(0, &content.spells[id], effect.power, effect.boost);
            pulse(&mut commands, id, strength, &[monster]);
        }
    }
    // Ring of Fire follows the party and takes whoever is close around it
    // now, including those that came close after it was cast.
    if let Some(effect) = effects.0.get(SpellId::RING_OF_FIRE).copied() {
        let centre = PartyBody::centre(eye);
        let reach_of =
            crate::game::party::body::RADIUS + crate::content::species::Reach::Short.metres();
        let victims = radius(
            (centre, centre, reach_of),
            Recipients::Opponents(Stance::Friendly),
            None,
            &mut bodies,
            &levels,
            (&content.species, (&groups, &factions, clock.ms)),
            &obstacles,
        );
        let strength = Strength { power: effect.power, boost: 0, against: effect.power * 3 };
        pulse(&mut commands, SpellId::RING_OF_FIRE, strength, &victims);
    }
}

/// The party's spells and the fighting monsters' age by the round's tick,
/// and the next round waits to be accepted.
pub(super) fn age_enchantments(
    fight: Single<(&mut Battle, &Combatants, &RoundEnding)>,
    mut party: Single<&mut PartyEffects>,
    mut monsters: Query<&mut crate::game::characters::Vitals>,
) {
    let (mut battle, combatants, ending) = fight.into_inner();
    party.0.age(ending.0.aging);
    for &monster in combatants.entities() {
        if let Ok(mut vitals) = monsters.get_mut(monster) {
            vitals.enchantments.age(ending.0.aging);
        }
    }
    battle.phase = RoundPhase::Waiting;
}

/// Members who defended, or lie asleep or knocked out, get their breath
/// back after their conditions age, in accepted actor order.
pub(super) fn rest_defenders(
    _fight: Single<(), With<RoundEnding>>,
    roster: Single<&PartyRoster>,
    mut members: Query<(&Character, &mut crate::game::characters::Vitals, &PartyTurn)>,
    mut rng: ResMut<GameRng>,
) {
    for member in roster.iter() {
        let Ok((character, mut vitals, turn)) = members.get_mut(member) else { continue };
        let resting = turn.defends()
            || matches!(
                vitals.conditions.worst(),
                Some(Condition::Asleep | Condition::Unconscious)
            );
        if vitals.is_dead() || !resting {
            continue;
        }
        let maximum = vitals.stamina.max;
        let gained =
            recovery::combat_rest_amount(character, recovery::rest_roll(maximum, &mut rng));
        if vitals.stamina.current < maximum || vitals.conditions.exhausted() {
            vitals.rest(gained);
        }
    }
}

/// The fight is over when no hostile stays in it, or nobody attacked or
/// closed in for two rounds; the living, the unconscious included, share
/// its experience. Poison may have killed the last of the party as the
/// round passed: that is their defeat, not a victory.
#[allow(clippy::too_many_arguments)]
pub(super) fn judge_victory(
    mut commands: Commands,
    fight: Single<(Entity, &mut Battle, &Combatants), With<RoundEnding>>,
    mut bodies: Query<Body>,
    leaving: Query<(), With<Leaves>>,
    (stand, roster): (Single<&Position, With<PartyBody>>, Single<&PartyRoster>),
    groups: Query<&MonsterGroup>,
    content: If<Res<Catalogs>>,
    (factions, clock): (Res<Factions>, Res<GameClock>),
    mut news: MessageWriter<CombatNews>,
) {
    let (fight, mut battle, combatants) = fight.into_inner();
    let living: Vec<Entity> =
        roster.iter().filter(|&m| bodies.get(m).is_ok_and(|b| !b.is_dead())).collect();
    if living.is_empty() {
        return;
    }
    battle.wait = Timer::from_seconds(ROUND_SECONDS, TimerMode::Once);
    let (eye, species) = (PartyBody::eye(*stand), &content.species);
    let approaching = combatants.entities().iter().any(|&entity| {
        bodies.get(entity).is_ok_and(|body| {
            let monster = body.monster.as_ref().unwrap();
            let pose = body.pose.as_ref().unwrap();
            approach::needs_approach(
                (monster, body.vitals),
                pose.translation(),
                body.walker,
                eye,
                species,
            )
        })
    });
    let active = battle.attacked || approaching;
    battle.passive_rounds = if active { 0 } else { battle.passive_rounds + 1 };
    let hostiles_left = combatants.entities().iter().any(|&entity| {
        !leaving.contains(entity)
            && bodies.get(entity).is_ok_and(|body| {
                !body.is_dead()
                    && body.side(species, (&groups, &factions, clock.ms)) == Stance::Hostile
            })
    });
    if hostiles_left && battle.passive_rounds < PASSIVE_ROUNDS {
        return;
    }
    let each = battle.experience / living.len() as u32;
    if each > 0 {
        for &member in &living {
            bodies.get_mut(member).unwrap().character.as_mut().unwrap().award_experience(each);
        }
    }
    news.write(if battle.kills > 0 {
        CombatNews::Won { kills: battle.kills, experience_each: each }
    } else {
        CombatNews::Over
    });
    commands.entity(fight).insert(FightOver);
}

/// Recognition is part of each round's end, before the next accepted action
/// or the fight's end can change its meaning: the party looks again at who
/// it fights.
pub(super) fn look_again(
    fight: Single<&Combatants, With<RoundEnding>>,
    party: Query<(&Position, &PartyBody, &PartyEffects)>,
    mut recognition: GroupRecognition,
    mut rng: ResMut<GameRng>,
) {
    let Ok((position, body, effects)) = party.single() else { return };
    recognition.detect(
        PartyBody::eye(position),
        body.forward(),
        Some(fight.entities()),
        effects.0.get(SpellId::SHADOW_HOUND).is_some(),
        false,
        &mut rng,
    );
}

/// The round has ended: those leaving leave; a fight that is over ends what
/// lasts a fight on the party and those in it, and goes.
pub fn end_round(
    mut commands: Commands,
    fight: Single<(Entity, &Combatants, Has<FightOver>), With<RoundEnding>>,
    leaving: Query<Entity, With<Leaves>>,
    mut party: Single<&mut PartyEffects>,
    mut monsters: Query<&mut crate::game::characters::Vitals>,
) {
    let (fight, combatants, over) = *fight;
    for monster in &leaving {
        commands.entity(monster).remove::<(Leaves, Combatant)>();
    }
    commands.entity(fight).remove::<(RoundEnding, FightOver)>();
    if !over {
        return;
    }
    for &monster in combatants.entities() {
        if let Ok(mut vitals) = monsters.get_mut(monster) {
            vitals.enchantments.end_combat();
        }
    }
    party.0.end_combat();
    commands.entity(fight).despawn();
}
