//! Weapon attacks in a fight (`docs/rules/attack.md`). An actor that fights
//! ([`Strike`]) chooses whom to strike and with what, and is announced; then
//! each of its swings is an event of its own ([`Swing`]), asking the rules of
//! [`super::attack`] in the order of the page. A blow that lands is a
//! [`Deed`], and so is Razor Cloak's answer to it; Bevy lands each before
//! the next swing is begun, so every swing sees what the one before did.
//! What the swings came to is kept on the attacker ([`Attacking`]) and told
//! as the attack ends.
//!
//! Retail's voices on a miss, a kill or a fumble draw from the same random
//! numbers as the rules; here they draw nothing.

use bevy::prelude::*;
use enum_map::EnumMap;

use super::approach::CombatMove;
use super::attack::{self, Attack, Fighter, FighterReadOnly, Setting, Surprise};
use super::effort::{Effort, Exerted, Tires};
use super::turn::{Fights, Turn};
use super::{Announcement, Battle, CombatNews, Combatants, Pause, Round};
use crate::content::characters::Skill;
use crate::content::items::{AttackEffects, AttackMode};
use crate::game::actors::{Arm, Laying, Places, reaches};
use crate::game::characters::{Condition, Practise};
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::hits::definitions::{Blow, Rows};
use crate::game::hits::effects;
use crate::game::hits::{Deed, Effect, Landed};
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

/// An attack under way: what it is made with and how, at whom, and what its
/// swings have come to.
#[derive(Component, Clone, Debug)]
pub struct Attacking {
    target: Entity,
    arm: Arm,
    mode: AttackMode,
    berserk: bool,
    surprise: Surprise,
    swings: u32,
    swung: u32,
    hits: u32,
    damage: u32,
    effects: Vec<Effect>,
}

impl Attacking {
    fn attack(&self) -> Attack<'_> {
        Attack { arm: &self.arm, mode: self.mode, berserk: self.berserk, surprise: self.surprise }
    }
}

/// The attacker swings once more.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Swing {
    #[event_target]
    pub actor: Entity,
}

/// A blow of an attack, landing on whom it reached: one gone astray is told
/// of on its own.
#[derive(Component, Clone, Copy, Debug)]
pub struct Blown {
    attacker: Entity,
    victim: Entity,
    astray: bool,
}

/// Razor Cloak's answer to a close blow, told of on its own.
#[derive(Component, Clone, Copy, Debug)]
pub struct Answer {
    by: Entity,
    at: Entity,
}

/// A member practises a skill it may train.
fn practise(
    actor: Entity,
    skill: Skill,
    fighters: &Query<FighterReadOnly>,
    commands: &mut Commands,
) {
    let trains = |fighter: attack::FighterReadOnlyItem| {
        fighter.body.trained().is_some_and(|c| c.can_use(skill))
    };
    if fighters.get(actor).is_ok_and(trains) {
        commands.trigger(Practise { member: actor, skill, usage: 1 });
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
/// then do; is announced; and swings. Attacking holds the fight for the
/// action's pause.
pub(super) fn strike(
    strike: On<Strike>,
    mut commands: Commands,
    mut turns: Query<(&mut Turn, Option<&Walker>)>,
    mut fighters: Query<Fighter>,
    places: Places,
    fight: Single<(&mut Round, &mut Pause, Option<&Combatants>), With<Battle>>,
    party: Single<(&mut Formation, &PartyRoster)>,
    obstacles: WorldObstacles,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    let Strike { actor, fights } = *strike;
    let (mut round, mut pause, combatants) = fight.into_inner();
    let (mut formation, roster) = party.into_inner();
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    let now = round.moment;
    let Ok((mut turn, walker)) = turns.get_mut(actor) else { return };
    pause.0 = Timer::from_seconds(super::ACTION_SECONDS, TimerMode::Once);

    // Whom to strike and with what, or what to do instead.
    let seen = fighters.as_readonly();
    let choice = attack::choose(
        &mut rng,
        actor,
        (&turn, fights, now),
        walker,
        combatants,
        (&seen, &places),
        (&formation, roster),
        &obstacles,
    );
    for &member in &choice.learns_stealth {
        practise(member, Skill::Stealth, &seen, &mut commands);
    }
    let (slot, arm, mode, target, left) = match choice.chosen {
        attack::Chosen::Strike { slot, arm, mode, target, left } => (slot, arm, mode, target, left),
        attack::Chosen::Advance { goal, budget, again } => {
            turn.moved = true;
            turn.when = Some(again);
            commands.entity(actor).insert(CombatMove { budget: budget.metres, moved: 0.0, goal });
            news.write(CombatNews::Moves { monster: actor, charge: budget.charge });
            let effort = if budget.charge { Effort::Charge } else { Effort::Approach };
            commands.trigger(Exerted { actor, effort });
            return;
        }
        attack::Chosen::Wait => {
            turn.waited = true;
            turn.end();
            commands.trigger(Exerted { actor, effort: Effort::Wait });
            return;
        }
        attack::Chosen::Stand => {
            turn.end();
            return;
        }
    };
    let Ok(own) = seen.get(actor) else { return };
    let arm = own.body.attacks.arms[arm].clone();
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
    let Ok(caught) =
        seen.get(target).map(|aimed| attack::catch_unaware(&mut rng, &arm, sides, &aimed))
    else {
        return;
    };
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
    commands.entity(actor).insert(Attacking {
        target,
        arm,
        mode,
        berserk: fights.berserk,
        surprise,
        swings: swings.count,
        swung: 0,
        hits: 0,
        damage: 0,
        effects: Vec::new(),
    });
    commands.trigger(Swing { actor });
}

/// One swing: its aim, whether and whom it reaches, whether a guard of that
/// one steps in to take the blow, whether a near miss is blocked, where it
/// lands and whether it gets through armour; the blow, and a cloak's answer
/// to it; what the swing costs and uses up. Then the next, while it can
/// swing and its target lives.
#[allow(clippy::too_many_arguments)]
fn swing(
    swing: On<Swing>,
    mut commands: Commands,
    mut attacking: Query<&mut Attacking>,
    mut turns: Query<&mut Turn>,
    mut fighters: Query<Fighter>,
    mut laying: Laying,
    places: Places,
    strikers: Query<(Option<&Weapon>, Option<&Missile>)>,
    fight: Single<(&Round, Option<&Combatants>), With<Battle>>,
    formation: Single<&Formation>,
    held: Items,
    (content, difficulty): (Res<Catalogs>, Res<Difficulty>),
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    let actor = swing.actor;
    let Ok(mut on) = attacking.get_mut(actor) else { return };
    let (round, combatants) = *fight;
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    let setting =
        Setting { difficulty: *difficulty, running: round.pace == Some(super::PartyPace::Run) };
    let target = on.target;
    let arm = on.arm.clone();
    let seen = fighters.as_readonly();
    let (Ok(me), Ok(aimed)) = (seen.get(actor), seen.get(target)) else {
        commands.trigger(AttackEnds { actor });
        return;
    };
    if on.swung == on.swings || !me.body.can_fight || aimed.body.vitals.is_dead() {
        commands.trigger(AttackEnds { actor });
        return;
    }
    let chance = attack::chance_to_hit(&on.attack(), &me, &aimed, setting);
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
    let stepping =
        reached.and_then(|_| attack::steps_in(&mut rng, victim, &me, on.surprise, &seen, &places));
    if let Some(stepping) = stepping {
        if let Ok(mut guard) = fighters.get_mut(stepping.guard)
            && let Some(made) = guard.interceptions.as_mut()
        {
            made.take();
        }
        commands.trigger(Tires { actor: stepping.guard, stamina: stepping.tires });
        on.surprise = Surprise::default();
        news.write(CombatNews::Intercepts { guard: stepping.guard, ward: victim });
        victim = stepping.guard;
    }
    let seen = fighters.as_readonly();
    let (Ok(me), Ok(struck)) = (seen.get(actor), seen.get(victim)) else {
        commands.trigger(AttackEnds { actor });
        return;
    };

    // A near miss is blocked: heard, where a miss is not.
    if let Some((roll, false, _)) = reached
        && !fumbled
        && roll as i32 - chance.aim <= attack::block(&on.attack(), &struck)
    {
        news.write(CombatNews::Blocked { victim, sound_class: arm.sound_class });
    }

    // Where it lands, whether it gets through armour, and what the blow does.
    if let Some((roll, true, through)) = reached {
        on.hits += u32::from(!fumbled);
        let place = attack::where_it_lands(&mut rng, &arm, &struck);
        let by = attack::piercing_mode(&mut rng, on.mode, arm.loaded);
        let through = attack::gets_through(
            &mut rng,
            &on.attack(),
            (place, by),
            (roll, through),
            &me,
            &struck,
            setting,
        );
        let blow =
            through.then(|| attack::blow(&mut rng, &on.attack(), &me, &struck, setting, fumbled));
        // The blow is heard where it lands (retail `MakePCMeleeHitSound`,
        // `MakeMonsterHitSound`).
        if through {
            news.write(CombatNews::Struck { victim, place: place.0, sound_class: arm.sound_class });
            practise(victim, Skill::IronSkin, &seen, &mut commands);
        }
        if let Some(blow) = blow.filter(|blow| blow.damage > 0) {
            let blow =
                Blow { damage: blow.damage, effects: blow.effects, by_monster: arm.word.is_some() };
            let strength = attack::effect_strength(me.body.level.0);
            let strike =
                effects::Strike::new(Rows::Blow, strength, Some(blow), Some(actor), &content);
            commands.spawn((
                Deed::at(strike, [victim]),
                Blown { attacker: actor, victim, astray: fumbled },
            ));
            if !fumbled {
                for &skill in arm.practises {
                    practise(actor, skill, &seen, &mut commands);
                }
            }
        } else if fumbled {
            news.write(CombatNews::Result { attacker: actor, target: victim, hits: 1, damage: 0 });
        }

        // A spell laid on the victim may answer a close blow (Razor Cloak).
        if !attack::is_missile(arm.reach)
            && let Some((cloak, dice)) = laying.answer_blow(victim)
        {
            let answer = attack::riposte(&mut rng, Some(dice), cloak.power, me.body.reduction.0);
            if answer > 0 {
                let blow =
                    Blow { damage: answer, effects: AttackEffects::default(), by_monster: false };
                let strike = effects::Strike::new(
                    Rows::Blow,
                    Strength::default(),
                    Some(blow),
                    Some(victim),
                    &content,
                );
                commands.spawn((Deed::at(strike, [actor]), Answer { by: victim, at: actor }));
            } else {
                news.write(CombatNews::Result {
                    attacker: victim,
                    target: actor,
                    hits: 1,
                    damage: 0,
                });
            }
        }
    }
    on.swung += 1;

    // What the swing costs, whether it lands or misses, and what is shot or
    // thrown is spent with it.
    let cost = attack::swing_stamina(&mut rng, &on.attack(), &me);
    commands.trigger(Tires { actor, stamina: cost });
    let used = attack::uses_up(&arm, actor, on.swung as usize, &held, &strikers);
    if let Some(item) = used.spends {
        commands.trigger(Spend { item, amount: 1 });
    }
    if used.used_up {
        if let Ok(mut turn) = turns.get_mut(actor) {
            turn.run_out(arm.slot);
        }
        commands.trigger(AttackEnds { actor });
        return;
    }
    commands.trigger(Swing { actor });
}

/// What a blow did counts toward its attack's tally; one gone astray is told
/// of on its own.
fn blown(
    landed: On<Landed>,
    blows: Query<&Blown>,
    mut attacking: Query<&mut Attacking>,
    mut news: MessageWriter<CombatNews>,
) {
    let Ok(&Blown { attacker, victim, astray }) = blows.get(landed.deed) else { return };
    let done = attack::dealt(landed.effects.iter().copied());
    if astray {
        told_alone((attacker, victim), done, &mut news);
    } else if let Ok(mut on) = attacking.get_mut(attacker) {
        on.damage += done.damage;
        on.effects.extend(done.rest);
    }
}

/// Razor Cloak's answer is told of on its own.
fn answered(landed: On<Landed>, answers: Query<&Answer>, mut news: MessageWriter<CombatNews>) {
    let Ok(&Answer { by, at }) = answers.get(landed.deed) else { return };
    told_alone((by, at), attack::dealt(landed.effects.iter().copied()), &mut news);
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

/// An attack is over.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct AttackEnds {
    #[event_target]
    actor: Entity,
}

/// What the attack came to is told, and took out of one that tires by what
/// it does; the attacker's next attack is set.
fn attack_ends(
    ends: On<AttackEnds>,
    mut commands: Commands,
    attacking: Query<&Attacking>,
    mut turns: Query<&mut Turn>,
    fighters: Query<FighterReadOnly>,
    round: Single<&Round, With<Battle>>,
    mut news: MessageWriter<CombatNews>,
) {
    let actor = ends.actor;
    let Ok(on) = attacking.get(actor) else { return };
    commands.entity(actor).remove::<Attacking>();
    let fury = on.berserk || on.mode == AttackMode::Berserk;
    let effort = if fury { Effort::Fury } else { Effort::Attack };
    commands.trigger(Exerted { actor, effort });
    news.write(CombatNews::Result {
        attacker: actor,
        target: on.target,
        hits: on.hits,
        damage: on.damage,
    });
    if !on.effects.is_empty() {
        news.write(CombatNews::Effects(on.effects.clone()));
    }
    let (Ok(mut turn), Ok(own)) = (turns.get_mut(actor), fighters.get(actor)) else { return };
    let initiative = EnumMap::from_fn(|slot| own.body.attacks.slot_initiative(slot));
    match attack::next_attack(turn.attacks, initiative, turn.due, round.moment) {
        Some(next) => {
            turn.due = next.slot;
            turn.when = Some(next.moment);
        }
        None => turn.end(),
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_observer(strike)
        .add_observer(swing)
        .add_observer(blown)
        .add_observer(answered)
        .add_observer(attack_ends);
}
