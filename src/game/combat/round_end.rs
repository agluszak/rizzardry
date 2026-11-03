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
use super::sight::reinforces;
use super::turn::{Ordered, SpecialCooldown, Turn};
use super::{
    Battle, CombatNews, Combatant, Combatants, PASSIVE_ROUNDS, PartyMove, Pause, ROUND_SECONDS,
    Round, RoundPhase, Spoils,
};
use crate::content::spells::SpellId;
use crate::game::characters::Vitals;
use crate::game::characters::{Character, Condition};
use crate::game::data::Catalogs;
use crate::game::factions::Factions;
use crate::game::hits::effects::InFlight;
use crate::game::hits::prototypes::{Definitions, Uses};
use crate::game::hits::reach::{Recipients, within};
use crate::game::hits::spells::reach;
use crate::game::hits::{Act, Deed};
use crate::game::magic::Strength;
use crate::game::magic::casting::Cast;
use crate::game::magic::enchantments::{OFFENSIVE, PartyEffects};
use crate::game::party::body::PartyBody;
use crate::game::party::recovery;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::time::{GameClock, Ticks};
use crate::game::world::footing::{Conspicuous, Eyes, Footing};
use crate::game::world::hostility::{self, Side, Stance};
use crate::game::world::identify::{GroupMember, GroupMembers, GroupRecognition, MonsterGroup};
use crate::game::world::level::PlayedLevel;
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::{Monster, Untargetable};
use bevy::ecs::relationship::RelationshipTarget;

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
    battle: Option<
        Single<(Entity, &Round, &PartyMove, &Pause), (With<Battle>, Without<RoundEnding>)>,
    >,
    busy: (Query<(), With<InFlight>>, Query<(), With<CombatMove>>, Query<(), With<Cast>>),
    mut clock: ResMut<GameClock>,
) {
    let Some((fight, round, party_move, pause)) = battle.map(Single::into_inner) else { return };
    let RoundPhase::Executing(step) = round.phase else { return };
    if step <= 100
        || matches!(party_move, PartyMove::Moving { .. })
        || !pause.0.is_finished()
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
pub(super) fn leave_fight(
    mut commands: Commands,
    fight: Single<&Combatants, With<RoundEnding>>,
    fighters: Query<(
        &Monster,
        &Vitals,
        &Footing,
        &Eyes,
        &Conspicuous,
        &hostility::Side,
        &hostility::Loyalty,
        Option<&GroupMember>,
    )>,
    groups: Query<(&MonsterGroup, &GroupMembers)>,
    members: Query<(&Footing, &Conspicuous), With<Character>>,
    roster: Single<&PartyRoster>,
    level: PlayedLevel,
    obstacles: WorldObstacles,
    content: If<Res<Catalogs>>,
    factions: Res<Factions>,
    clock: Res<GameClock>,
) {
    let combatants = fight.entities();
    let far = level.view_distance().unwrap_or(0.0);
    let Some(party) = roster.iter().find_map(|member| members.get(member).ok()) else { return };
    let fighting = |monster: Entity| combatants.contains(&monster);
    let alive =
        |monster: Entity| fighters.get(monster).is_ok_and(|(_, vitals, ..)| !vitals.is_dead());
    // The stance its group holds toward the party, Turncoat aside.
    let stance = |monster: Entity| {
        let (body, .., group) = fighters.get(monster).ok()?;
        let set = group.and_then(|group| groups.get(group.0).ok()).and_then(|(g, _)| g.set);
        Some(hostility::stance(set, &content.species[body.species], &factions, clock.ms, true))
    };
    let in_view = |monster: Entity| {
        fighters.get(monster).is_ok_and(|(_, _, at, ..)| party.0.in_view(at, far, &obstacles))
    };
    let sees_party = |monster: Entity| {
        fighters.get(monster).is_ok_and(|(_, _, at, eyes, ..)| eyes.see(at, party, far, &obstacles))
    };
    // Whether a monster sees a foe of its own in the fight.
    let sees_foe = |monster: Entity| {
        let Ok((_, _, at, eyes, _, side, loyalty, _)) = fighters.get(monster) else { return false };
        combatants.iter().any(|&other| {
            fighters.get(other).is_ok_and(|(_, vitals, there, _, seen, their_side, theirs, _)| {
                !vitals.is_dead()
                    && hostility::between((side.0, *loyalty), (their_side.0, *theirs))
                        == hostility::Stance::Hostile
                    && eyes.see(at, (there, seen), far, &obstacles)
            })
        })
    };
    let hostiles: Vec<Entity> = combatants
        .iter()
        .copied()
        .filter(|&c| alive(c) && in_view(c) && stance(c) == Some(hostility::Stance::Hostile))
        .collect();
    let footing = |monster: Entity| fighters.get(monster).ok().map(|(_, _, at, ..)| at);
    let mut engaged = false;
    let mut leaving = Vec::new();
    // A monster outside any group keeps hostile groups engaged.
    for &combatant in combatants {
        if fighters.get(combatant).is_ok_and(|(.., group)| group.is_none()) && alive(combatant) {
            engaged |= stance(combatant) == Some(hostility::Stance::Hostile);
        }
    }
    let fought: Vec<Entity> = combatants
        .iter()
        .filter_map(|&c| fighters.get(c).ok().and_then(|(.., group)| group).map(|g| g.0))
        .fold(Vec::new(), |mut groups, group| {
            if !groups.contains(&group) {
                groups.push(group);
            }
            groups
        });
    for group in fought {
        let Ok((_, members)) = groups.get(group) else { continue };
        let fellows: Vec<Entity> = members.iter().filter(|&m| fighting(m) && alive(m)).collect();
        let Some(stance) = members.iter().find(|&m| fighting(m)).and_then(stance) else { continue };
        let stays = match stance {
            hostility::Stance::Neutral => {
                let unseen = |m: Entity| {
                    fighters
                        .get(m)
                        .is_ok_and(|(body, ..)| content.species[body.species].temper.camouflaged)
                };
                let standing: Vec<(Entity, &Footing)> = fellows
                    .iter()
                    .filter(|&&m| !unseen(m) && in_view(m))
                    .filter_map(|&m| footing(m).map(|at| (m, at)))
                    .collect();
                let hostiles: Vec<(Entity, &Footing)> =
                    hostiles.iter().filter_map(|&h| footing(h).map(|at| (h, at))).collect();
                reinforces(party.0, &standing, &hostiles)
            }
            hostility::Stance::Hostile => {
                let sees = fellows.iter().any(|&m| sees_party(m));
                engaged |= sees;
                sees
            }
            hostility::Stance::Friendly => fellows.iter().any(|&m| sees_foe(m)),
        };
        if !stays {
            leaving.push(group);
        }
    }
    for &combatant in combatants {
        let group = fighters.get(combatant).ok().and_then(|(.., group)| group).map(|g| g.0);
        if !engaged || group.is_some_and(|g| leaving.contains(&g)) {
            commands.entity(combatant).insert(Leaves);
        }
    }
}

/// A monster that waited, or lies asleep or knocked out, gets its breath
/// back (retail `AdvanceCombatRound`).
pub(super) fn rest_waiters(
    fight: Single<&Combatants, With<RoundEnding>>,
    mut monsters: Query<(&mut crate::game::characters::Vitals, Option<&Turn>)>,
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
    mut monsters: Query<(&mut SpecialCooldown, Option<&Turn>)>,
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
    let (_, ending) = *fight;
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
            &mut rng,
        );
    }
}

/// The offensive clouds pulse (retail `AdvanceTimedEffects`, after
/// personal aging and before the slots expire; their last turn still
/// pulses): the party's on its living members, each monster's own on it,
/// and Ring of Fire on the monsters close around the party, sampled anew
/// each pulse. Each pulse is an act with no caster.
pub(super) fn pulse_clouds(
    mut commands: Commands,
    fight: Single<&Combatants, With<RoundEnding>>,
    bodies: Query<(Entity, &Vitals, &Side, &Footing, Option<&Monster>), Without<Untargetable>>,
    party: Single<(&Position, &PartyRoster, &PartyEffects)>,
    obstacles: WorldObstacles,
    content: If<Res<Catalogs>>,
    definitions: Res<Definitions>,
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
        roster.iter().filter(|&m| bodies.get(m).is_ok_and(|(_, v, ..)| !v.is_dead())).collect();
    for id in OFFENSIVE {
        if let Some(effect) = effects.0.get(id) {
            let strength = Strength::at(0, &content.spells[id], effect.power, effect.boost);
            pulse(&mut commands, id, strength, &living);
        }
    }
    for &monster in fight.entities() {
        let Ok((_, vitals, ..)) = bodies.get(monster) else { continue };
        if vitals.is_dead() {
            continue;
        }
        let clouds: Vec<_> = OFFENSIVE
            .iter()
            .filter_map(|&id| vitals.enchantments.get(id).map(|e| (id, *e)))
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
        let victims = within(
            centre,
            centre,
            reach_of,
            Recipients::Opponents(Stance::Friendly),
            &bodies,
            roster,
            &obstacles,
        );
        let strength = Strength { power: effect.power, boost: 0, against: effect.power * 3 };
        pulse(&mut commands, SpellId::RING_OF_FIRE, strength, &victims);
    }
}

/// The party's spells and the fighting monsters' age by the round's tick,
/// and the next round waits to be accepted.
pub(super) fn age_enchantments(
    fight: Single<(&mut Round, &Combatants, &RoundEnding), With<Battle>>,
    mut party: Single<&mut PartyEffects>,
    mut monsters: Query<&mut crate::game::characters::Vitals>,
) {
    let (mut round, combatants, ending) = fight.into_inner();
    party.0.age(ending.0.aging);
    for &monster in combatants.entities() {
        if let Ok(mut vitals) = monsters.get_mut(monster) {
            vitals.enchantments.age(ending.0.aging);
        }
    }
    round.phase = RoundPhase::Waiting;
}

/// Members who defended, or lie asleep or knocked out, get their breath
/// back after their conditions age, in accepted actor order.
pub(super) fn rest_defenders(
    _fight: Single<(), With<RoundEnding>>,
    roster: Single<&PartyRoster>,
    mut members: Query<(&Character, &mut crate::game::characters::Vitals, &Ordered)>,
    mut rng: ResMut<GameRng>,
) {
    for member in roster.iter() {
        let Ok((character, mut vitals, ordered)) = members.get_mut(member) else { continue };
        let resting = ordered.defends()
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
#[allow(clippy::type_complexity)]
pub(super) fn judge_victory(
    mut commands: Commands,
    fight: Single<
        (Entity, &mut Battle, &Round, &Spoils, &mut Pause, &Combatants),
        With<RoundEnding>,
    >,
    mut members: Query<(&mut Character, &Vitals)>,
    monsters: Query<(
        &Monster,
        &Vitals,
        &Side,
        &Footing,
        Option<&crate::game::world::navigation::Walker>,
    )>,
    footings: Query<&Footing>,
    leaving: Query<(), With<Leaves>>,
    roster: Single<&PartyRoster>,
    content: If<Res<Catalogs>>,
    mut news: MessageWriter<CombatNews>,
) {
    let (fight, mut battle, round, spoils, mut pause, combatants) = fight.into_inner();
    let living: Vec<Entity> =
        roster.iter().filter(|&m| members.get(m).is_ok_and(|(_, v)| !v.is_dead())).collect();
    if living.is_empty() {
        return;
    }
    pause.0 = Timer::from_seconds(ROUND_SECONDS, TimerMode::Once);
    let party_at = roster.iter().find_map(|m| footings.get(m).ok()).copied();
    let species = &content.species;
    let approaching = combatants.entities().iter().any(|&entity| {
        monsters.get(entity).is_ok_and(|(monster, vitals, _, at, walker)| {
            party_at.is_some_and(|party| {
                approach::needs_approach(&species[monster.species], vitals, walker, at.gap(&party))
            })
        })
    });
    let active = round.attacked || approaching;
    battle.passive_rounds = if active { 0 } else { battle.passive_rounds + 1 };
    let hostiles_left = combatants.entities().iter().any(|&entity| {
        !leaving.contains(entity)
            && monsters
                .get(entity)
                .is_ok_and(|(_, vitals, side, ..)| !vitals.is_dead() && side.0 == Stance::Hostile)
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
