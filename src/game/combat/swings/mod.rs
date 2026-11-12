//! Weapon attacks in a fight (`docs/rules/attack.md`). An actor that fights
//! ([`Strike`]) makes its whole attack here, as the one moment the player
//! sees: the rules of [`super::attack`] are asked in the order of the page,
//! of the fighters as they stand, and what they decide is made to happen. It
//! holds no rule of its own.
//!
//! A blow lands as it is struck rather than as a [`crate::game::hits::Deed`]:
//! the next swing, and the attack's result, depend on what the blow did to its
//! victim.
//!
//! Retail's voices on a miss, a kill or a fumble draw from the same random
//! numbers as the rules; here they draw nothing.

use bevy::prelude::*;
use enum_map::EnumMap;

use super::approach::CombatMove;
use super::attack::{self, Attack, Fighter, FighterReadOnly, Setting, Surprise};
use super::turn::{Fights, Turn};
use super::{Announcement, Battle, CombatNews, Combatants, Pause, Round};
use crate::content::characters::Skill;
use crate::content::items::AttackEffects;
use crate::game::actors::{Arm, Laying, Places, reaches};
use crate::game::characters::{Condition, Practise};
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::hits::Damaged;
use crate::game::hits::definitions::{Blow, Rows};
use crate::game::hits::effects::{self, Log, Rules};
use crate::game::items::{Items, Missile, Spend, Weapon};
use crate::game::magic::Strength;
use crate::game::party::formation::Formation;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::hostility::Provocation;
use crate::game::world::navigation::Walker;
use crate::game::world::obstacles::WorldObstacles;

/// An actor that fights makes its attack ([`strike`]).
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Strike {
    #[event_target]
    pub actor: Entity,
    pub fights: Fights,
}

/// A member practises a skill it may train.
fn practise(actor: Entity, skill: Skill, fighters: &Query<Fighter>, commands: &mut Commands) {
    let trains = |fighter: attack::FighterReadOnlyItem| {
        fighter.body.character.is_some_and(|c| c.skill_available(skill))
    };
    if fighters.get(actor).is_ok_and(trains) {
        commands.trigger(Practise { member: actor, skill, usage: 1 });
    }
}

/// Land a blow on a body: its damage, then its weapon's effects, told apart.
fn deal(
    victim: Entity,
    strength: Strength,
    blow: Blow,
    fighters: &mut Query<Fighter>,
    rules: &Rules,
    rng: &mut GameRng,
    (log, laying): (&mut Log, &mut Laying),
) -> attack::Dealt {
    let Ok(mut fighter) = fighters.get_mut(victim) else { return attack::Dealt::default() };
    let strike = effects::Strike { rows: Rows::Blow, strength, blow: Some(blow) };
    effects::land(&strike, &mut fighter.body, rules, rng, log, laying);
    attack::dealt(log.effects.drain(..))
}

/// A blow told of on its own: one gone astray, or Razor Cloak's answer.
fn told_alone(
    (by, at): (Entity, Entity),
    done: attack::Dealt,
    news: &mut MessageWriter<CombatNews>,
) {
    news.write(CombatNews::Result { attacker: by, target: at, hits: 1, damage: done.damage });
    if !done.rest.is_empty() {
        news.write(CombatNews::Effects(done.rest));
    }
}

/// The living a close swing could go astray onto: those in reach of the arm
/// other than its target, friends and foes alike and the attacker itself. A
/// shot meets nobody on the way.
fn bystanders(
    (actor, target): (Entity, Entity),
    arm: &Arm,
    combatants: &[Entity],
    (fighters, places): (&Query<FighterReadOnly>, &Places),
    formation: &Formation,
) -> Vec<Entity> {
    if attack::is_missile(arm.reach) {
        return Vec::new();
    }
    let from = places.of(actor);
    let in_reach = |other| {
        other == actor || reaches(formation, (actor, &from), (other, &places.of(other)), arm.reach)
    };
    combatants
        .iter()
        .copied()
        .filter(|&other| other != target)
        .filter(|&other| fighters.get(other).is_ok_and(|there| !there.body.vitals.is_dead()))
        .filter(|&other| in_reach(other))
        .collect()
}

/// An actor that fights makes its attack: it chooses whom to strike and with
/// what, or closes in, waits or stands; may catch its target unaware;
/// provokes whom a member strikes as the attack starts, whatever the blows
/// then do; is announced; swings while it can and its target lives; and
/// reports what its blows came to. Attacking holds the fight for the
/// action's pause.
pub(super) fn strike(
    strike: On<Strike>,
    mut commands: Commands,
    mut turns: Query<(&mut Turn, Option<&Walker>)>,
    mut fighters: Query<Fighter>,
    mut laying: Laying,
    places: Places,
    strikers: Query<(Option<&Weapon>, Option<&Missile>)>,
    fight: Single<(Entity, &mut Round, &mut Pause, Option<&Combatants>), With<Battle>>,
    party: Single<(&mut Formation, &PartyRoster)>,
    held: Items,
    obstacles: WorldObstacles,
    (content, difficulty): (Res<Catalogs>, Res<Difficulty>),
    mut rng: ResMut<GameRng>,
    (mut news, mut damaged): (MessageWriter<CombatNews>, MessageWriter<Damaged>),
) {
    let Strike { actor, fights } = *strike;
    let (fight, mut round, mut pause, combatants) = fight.into_inner();
    let (mut formation, roster) = party.into_inner();
    let rules =
        Rules { content: &content, difficulty: *difficulty, fight: Some(fight), held: &held };
    let setting =
        Setting { difficulty: *difficulty, running: round.pace == Some(super::PartyPace::Run) };
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    let now = round.moment;
    let mut log = Log::default();
    let Ok((mut turn, walker)) = turns.get_mut(actor) else { return };
    pause.0 = Timer::from_seconds(super::ACTION_SECONDS, TimerMode::Once);

    // Whom to strike and with what, or what to do instead.
    let choice = attack::choose(
        &mut rng,
        actor,
        (&turn, fights, now),
        walker,
        combatants,
        (&fighters.as_readonly(), &places),
        (&formation, roster),
        &obstacles,
    );
    for &member in &choice.learns_stealth {
        practise(member, Skill::Stealth, &fighters, &mut commands);
    }
    let (slot, arm, mode, target, left) = match choice.chosen {
        attack::Chosen::Strike { slot, arm, mode, target, left } => (slot, arm, mode, target, left),
        attack::Chosen::Advance { goal, budget, again } => {
            turn.moved = true;
            turn.when = Some(again);
            commands.entity(actor).insert(CombatMove { budget: budget.metres, moved: 0.0, goal });
            news.write(CombatNews::Moves { monster: actor, charge: budget.charge });
            return;
        }
        attack::Chosen::Wait => {
            turn.waited = true;
            turn.end();
            return;
        }
        attack::Chosen::Stand => {
            turn.end();
            return;
        }
    };
    let Ok(own) = fighters.get(actor) else { return };
    let arm = own.body.attacks.arms[arm].clone();
    let strength = attack::effect_strength(own.body.level.0);
    let provokes = own.body.member && !own.has(Condition::Turncoat);
    let swings = attack::swing_count(&mut rng, &arm, turn.lightning, own.has(Condition::Slowed));
    turn.due = slot;
    turn.attacks[slot] = left;
    if swings.lightning_spent {
        turn.lightning = None;
    }
    round.attacked(true);

    // Catching the target unaware, and provoking it.
    let sides = (places.of(target).quadrant(places.of(actor).feet), formation.facing(target));
    let caught = {
        let seen = fighters.as_readonly();
        seen.get(target).map(|aimed| attack::catch_unaware(&mut rng, &arm, sides, &aimed))
    };
    let Ok(caught) = caught else { return };
    if let Ok(mut aimed) = fighters.get_mut(target)
        && let Some(spotted) = aimed.spotted.as_mut()
    {
        spotted.0 = caught.tries;
    }
    if let Some(side) = caught.turns_to {
        formation.turn(target, side);
    }
    if provokes {
        commands.trigger(Provocation { monster: target });
    }
    let how = match arm.word {
        Some((word, named)) => Announcement::Monster { word, mode, named },
        None => Announcement::Mode(mode),
    };
    let surprise = caught.surprise;
    news.write(CombatNews::Attack { attacker: actor, target, how, swings: swings.count, surprise });
    // A member's weapon is heard swung (retail `MakePCAttackSound`).
    if arm.word.is_none() {
        news.write(CombatNews::Swung { sound_class: arm.sound_class });
    }

    // The swings.
    let mut on = Attack { arm: &arm, mode, berserk: fights.berserk, surprise };
    let (mut swung, mut hits, mut damage) = (0, 0, 0);
    let mut effects = Vec::new();
    for _ in 0..swings.count {
        // Its aim, whether and whom it reaches, and whether a guard of that
        // one steps in to take the blow.
        let seen = fighters.as_readonly();
        let (Ok(me), Ok(aimed)) = (seen.get(actor), seen.get(target)) else { break };
        if !me.body.can_fight || aimed.body.vitals.is_dead() {
            break;
        }
        let chance = attack::chance_to_hit(&on, &me, &aimed, setting);
        let astray = bystanders((actor, target), &arm, combatants, (&seen, &places), &formation);
        let line = (&places.of(actor), &places.of(target));
        let blind = me.has(Condition::Blind);
        let mut victim = target;
        let mut fumbled = false;
        let reached = match attack::fate(&mut rng, &arm, chance, astray.len(), blind, line) {
            attack::Fate::Missed => None,
            attack::Fate::Reached { roll, lands, through } => Some((roll, lands, through)),
            attack::Fate::Fumbled { bystander, roll } => {
                on.surprise = Surprise::default();
                (victim, fumbled) = (astray[bystander], true);
                news.write(CombatNews::Fumble { attacker: actor, victim });
                Some((roll, true, false))
            }
        };
        let stepping = reached
            .and_then(|_| attack::steps_in(&mut rng, victim, &me, on.surprise, &seen, &places));
        if let Some(stepping) = stepping
            && let Ok(mut guard) = fighters.get_mut(stepping.guard)
        {
            if let Some(made) = guard.interceptions.as_mut() {
                made.take();
            }
            guard.body.vitals.tire(stepping.tires);
            on.surprise = Surprise::default();
            news.write(CombatNews::Intercepts { guard: stepping.guard, ward: victim });
            victim = stepping.guard;
        }

        // Where it lands, whether it gets through armour, and what the blow does.
        if let Some((roll, true, through)) = reached {
            hits += u32::from(!fumbled);
            let seen = fighters.as_readonly();
            let mut landed = None;
            let blow = seen.get(actor).ok().zip(seen.get(victim).ok()).and_then(|(me, struck)| {
                let place = attack::where_it_lands(&mut rng, &arm, &struck);
                let by = attack::piercing_mode(&mut rng, mode, arm.loaded);
                let sure = (roll, through);
                let through =
                    attack::gets_through(&mut rng, &on, (place, by), sure, &me, &struck, setting);
                landed = through.then_some(place);
                through.then(|| attack::blow(&mut rng, &on, &me, &struck, setting, fumbled))
            });
            // The blow is heard where it lands (retail `MakePCMeleeHitSound`,
            // `MakeMonsterHitSound`).
            if let Some(place) = landed {
                news.write(CombatNews::Struck {
                    victim,
                    place: place.0,
                    sound_class: arm.sound_class,
                });
            }
            if blow.is_some() {
                practise(victim, Skill::IronSkin, &fighters, &mut commands);
            }
            let done = blow.filter(|blow| blow.damage > 0).map(|blow| {
                let blow = Blow {
                    damage: blow.damage,
                    effects: blow.effects,
                    by_monster: arm.word.is_some(),
                };
                let landing = (&mut log, &mut laying);
                deal(victim, strength, blow, &mut fighters, &rules, &mut rng, landing)
            });
            match done {
                Some(done) if !fumbled => {
                    damage += done.damage;
                    effects.extend(done.rest);
                    for &skill in arm.practises {
                        practise(actor, skill, &fighters, &mut commands);
                    }
                }
                done if fumbled => {
                    told_alone((actor, victim), done.unwrap_or_default(), &mut news);
                }
                _ => {}
            }

            // A spell laid on the victim may answer a close blow (Razor Cloak).
            if !attack::is_missile(arm.reach)
                && let Some(cloak) = laying.answer_blow(victim)
            {
                let dice = content.spells[cloak.spell].dice;
                let reduction = fighters.get(actor).map_or(0, |me| me.body.armour.reduction);
                let answer = attack::riposte(&mut rng, dice, cloak.power, reduction);
                let done = if answer > 0 {
                    let blow = Blow {
                        damage: answer,
                        effects: AttackEffects::default(),
                        by_monster: false,
                    };
                    let landing = (&mut log, &mut laying);
                    deal(actor, Strength::default(), blow, &mut fighters, &rules, &mut rng, landing)
                } else {
                    attack::Dealt::default()
                };
                told_alone((victim, actor), done, &mut news);
            }
        }
        swung += 1;

        // What the swing costs, whether it lands or misses.
        let cost = fighters.get(actor).map(|me| attack::swing_stamina(&mut rng, &on, &me));
        if let (Ok(cost), Ok(mut me)) = (cost, fighters.get_mut(actor)) {
            me.body.vitals.tire(cost);
        }

        // What is shot or thrown is spent with the swing.
        let used = attack::uses_up(&arm, actor, swung, &held, &strikers);
        if let Some(item) = used.spends {
            commands.trigger(Spend { item, amount: 1 });
        }
        if used.used_up {
            turn.run_out(arm.slot);
            break;
        }
    }

    // What the attack came to.
    news.write(CombatNews::Result { attacker: actor, target, hits, damage });
    if !effects.is_empty() {
        news.write(CombatNews::Effects(effects));
    }

    // The attacker's next attack.
    if let Ok(own) = fighters.get(actor) {
        let initiative = EnumMap::from_fn(|slot| own.body.attacks.slot_initiative(slot));
        match attack::next_attack(turn.attacks, initiative, turn.due, now) {
            Some(next) => {
                turn.due = next.slot;
                turn.when = Some(next.moment);
            }
            None => turn.end(),
        }
    }
    log.scorch(&mut commands);
    damaged.write_batch(log.damaged);
    for (member, skill, usage) in log.practised {
        commands.trigger(Practise { member, skill, usage });
    }
}
