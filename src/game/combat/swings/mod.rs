//! Weapon attacks in a fight (`docs/rules/attack.md`). An actor that fights
//! ([`Strike`]) is arranged here and nothing more: the observer reads the facts
//! each rule of [`super::attack`] takes ([`facts`]), calls the rules in the
//! order of the page, and makes happen what they decide. It holds no rule of
//! its own.
//!
//! A blow lands as it is struck rather than by a [`crate::game::hits::Release`]:
//! the next swing, and the attack's result, depend on what the blow did to its
//! victim.
//!
//! Retail's voices on a miss, a kill or a fumble draw from the same random
//! numbers as the rules; here they draw nothing.

mod facts;

use bevy::prelude::*;
use enum_map::EnumMap;

use self::facts::{Attack, Bearing, Reached, Scene, Setting};
use super::approach::{CombatMove, MoveGoal};
use super::attack::{self, Surprise};
use super::turn::{Fights, Guards, Interceptions, Ordered, Spotted, Turn};
use super::{Announcement, Battle, CombatNews, Combatants, Pause, Round};
use crate::content::characters::Skill;
use crate::content::items::AttackEffects;
use crate::content::spells::SpellId;
use crate::game::actors::{Laying, Places};
use crate::game::characters::{Condition, Fell, Practise};
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::hits::Damaged;
use crate::game::hits::definitions::{Blow, Definitions};
use crate::game::hits::effects::{self, Body, Log, Rules};
use crate::game::inventory::WeaponSlot;
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
fn practise(actor: Entity, skill: Skill, bodies: &Query<Body>, commands: &mut Commands) {
    if bodies.get(actor).is_ok_and(|body| body.character.is_some_and(|c| c.skill_available(skill)))
    {
        commands.trigger(Practise { member: actor, skill, usage: 1 });
    }
}

/// Land a blow on a body: its damage, then its weapon's effects, told apart.
fn deal(
    victim: Entity,
    strength: Strength,
    blow: Blow,
    bodies: &mut Query<Body>,
    rules: &Rules,
    rng: &mut GameRng,
    (log, laying): (&mut Log, &mut Laying),
) -> attack::Dealt {
    let Ok(mut body) = bodies.get_mut(victim) else { return attack::Dealt::default() };
    let strike =
        effects::Strike { definition: rules.definitions.blow(), strength, blow: Some(blow) };
    effects::land(&strike, &mut body, rules, rng, log, laying);
    attack::dealt(log.effects.drain(..))
}

/// A blow told of on its own: one gone astray, or Razor Cloak's answer.
fn told_alone(
    (by, at): (Entity, Entity),
    done: attack::Dealt,
    news: &mut MessageWriter<CombatNews>,
) {
    let totals = attack::tally(&[attack::Swung { hit: true, damage: done.damage }]);
    news.write(CombatNews::Result {
        attacker: by,
        target: at,
        hits: totals.hits,
        damage: totals.damage,
    });
    if !done.rest.is_empty() {
        news.write(CombatNews::Effects(done.rest));
    }
}

/// An actor that fights makes its attack: it chooses whom to strike and with
/// what, or closes in, waits or stands (retail `StartCharacterAttack`,
/// `StartMonsterAttack`); may catch its target unaware; provokes whom a
/// member strikes (retail `MakeTargetGroupHostile` as the action starts,
/// whatever the blows then do); is announced; swings while it can and its
/// target lives; and reports what its blows came to (retail
/// `ReportCharacterAttackResult`). Attacking holds the fight for the action's
/// pause.
#[allow(clippy::type_complexity)]
pub(super) fn strike(
    strike: On<Strike>,
    mut commands: Commands,
    mut turns: Query<(&mut Turn, Has<Ordered>, Option<&Walker>)>,
    mut bodies: Query<Body>,
    mut laying: Laying,
    places: Places,
    bearing: Query<Bearing>,
    (guards, mut interceptions, strikers): (
        Query<&Guards>,
        Query<&mut Interceptions>,
        Query<(Option<&Weapon>, Option<&Missile>)>,
    ),
    mut watchful: Query<(&Ordered, &mut Spotted)>,
    fight: Single<(Entity, &mut Round, &mut Pause, Option<&Combatants>), With<Battle>>,
    party: Single<(&mut Formation, &PartyRoster)>,
    held: Items,
    obstacles: WorldObstacles,
    (content, definitions, difficulty): (Res<Catalogs>, Res<Definitions>, Res<Difficulty>),
    mut rng: ResMut<GameRng>,
    (mut news, mut damaged): (MessageWriter<CombatNews>, MessageWriter<Damaged>),
) {
    let Strike { actor, fights } = *strike;
    let (fight, mut round, mut pause, combatants) = fight.into_inner();
    let (mut formation, roster) = party.into_inner();
    let rules = Rules {
        content: &content,
        definitions: &definitions,
        difficulty: *difficulty,
        fight: Some(fight),
        held: &held,
    };
    let setting = Setting {
        combatants: combatants.map_or(&[][..], Combatants::entities),
        difficulty: *difficulty,
        running: round.pace == Some(super::PartyPace::Run),
    };
    let now = round.moment;
    let mut log = Log::default();
    let Ok((mut turn, ordered, walker)) = turns.get_mut(actor) else { return };
    pause.0 = Timer::from_seconds(super::ACTION_SECONDS, TimerMode::Once);

    // Whom to strike and with what, or what to do instead.
    let scene = Scene::new(&bodies, &places, &bearing, &formation, setting);
    let Ok(own) = bodies.get(actor) else {
        turn.end();
        return;
    };
    let (arms, foes, targets) = scene.look_about(actor, &own, (roster, &obstacles));
    let choice = attack::choose(
        &mut rng,
        &attack::Choosing {
            ordered,
            turned: turn.berserk,
            berserk: fights.berserk,
            waits: fights.waits,
            due: turn.due,
            attacks_left: turn.attacks,
            arms: &arms,
            foes: &foes,
            occupied: EnumMap::from_fn(|row| formation.occupied(row)),
            closing: scene.closing(&own, walker, &turn, now, roster),
        },
    );
    for &foe in &choice.learns_stealth {
        practise(targets[foe], Skill::Stealth, &bodies, &mut commands);
    }
    let attack::Chosen::Strike { slot, arm, mode, target, left } = choice.chosen else {
        match choice.chosen {
            attack::Chosen::Advance { toward, budget, again } => {
                turn.moved = true;
                turn.when = Some(again);
                let goal = match toward {
                    attack::Toward::Party => MoveGoal::Party,
                    attack::Toward::Foe(foe) => MoveGoal::Attack(targets[foe]),
                };
                commands.entity(actor).insert(CombatMove {
                    budget: budget.metres,
                    moved: 0.0,
                    goal,
                });
                news.write(CombatNews::Moves { monster: actor, charge: budget.charge });
            }
            attack::Chosen::Wait => {
                turn.waited = true;
                turn.end();
            }
            _ => turn.end(),
        }
        return;
    };
    let (target, arm) = (targets[target], own.attacks.arms[arm].clone());
    turn.due = slot;
    turn.attacks[slot] = left;
    let swings = attack::swing_count(
        &mut rng,
        &attack::SwingCount {
            swings: arm.swings,
            extra: arm.extra_swings,
            main_slot: slot == WeaponSlot::Primary,
            lightning: turn.lightning,
            sword: arm.sword,
            slowed: own.vitals.conditions.has(Condition::Slowed),
        },
    );
    if swings.lightning_spent {
        turn.lightning = None;
    }
    let provokes = attack::provokes(own.member, own.vitals.conditions.has(Condition::Turncoat));
    let on = Attack {
        actor,
        target,
        arm: &arm,
        mode,
        berserk: fights.berserk,
        strength: attack::effect_strength(own.level.0),
    };
    round.attacked(true);

    // Catching the target unaware, and provoking it.
    let caught = attack::catch_unaware(&mut rng, &scene.unaware(&on, &watchful));
    if let Ok((_, mut spotted)) = watchful.get_mut(target) {
        spotted.0 = caught.tries;
    }
    if let Some(side) = caught.turns_to {
        formation.turn(target, side);
    }
    if provokes {
        commands.trigger(Provocation { monster: target });
    }
    let mut surprise = caught.surprise;
    let how = match arm.word {
        Some((word, named)) => Announcement::Monster { word, mode, named },
        None => Announcement::Mode(mode),
    };
    news.write(CombatNews::Attack { attacker: actor, target, how, swings: swings.count, surprise });

    // The swings.
    let mut swung = Vec::new();
    let mut effects = Vec::new();
    for _ in 0..swings.count {
        // Its aim, and whether and where it reaches.
        let scene = Scene::new(&bodies, &places, &bearing, &formation, setting);
        let (Some(me), Some(aimed)) = (scene.striker(&on), scene.victim(target)) else { break };
        if !me.able || !aimed.alive {
            break;
        }
        let chance = attack::chance_to_hit(&on.hitting(surprise, &me, &aimed));
        let bystanders = scene.bystanders(&on);
        let fate = attack::fate(&mut rng, &scene.reaching(&on, chance, bystanders.len(), &me));
        let mut victim = target;
        let reached = match fate {
            attack::Fate::Missed => None,
            attack::Fate::Reached { roll, lands, through } => {
                Some(Reached { roll, lands, through, fumbled: false })
            }
            attack::Fate::Fumbled { bystander, roll } => {
                surprise = Surprise::default();
                victim = bystanders[bystander];
                news.write(CombatNews::Fumble { attacker: actor, victim });
                Some(Reached { roll, lands: true, through: false, fumbled: true })
            }
        };

        // A guard of the victim may step in and take the blow.
        let ward = victim;
        let (defenders, stand_ins) = scene.guards_of(ward, &guards, &interceptions);
        let stepping = reached.as_ref().and_then(|_| {
            attack::steps_in(
                &mut rng,
                &attack::StepIn { guards: &stand_ins, attacker: me.press, surprise },
            )
        });
        if let Some(stepping) = stepping {
            let guard = defenders[stepping.guard];
            if let Ok(mut taken) = interceptions.get_mut(guard) {
                taken.take();
            }
            if let Ok(mut body) = bodies.get_mut(guard) {
                body.vitals.tire(stepping.tires);
            }
            surprise = Surprise::default();
            news.write(CombatNews::Intercepts { guard, ward });
            victim = guard;
        }

        // Where it lands, whether it gets through armour, and what the blow does.
        let mut swing = attack::Swung { hit: false, damage: 0 };
        if let Some(reached) = reached.filter(|reached| reached.lands) {
            let Some(struck) =
                Scene::new(&bodies, &places, &bearing, &formation, setting).victim(victim)
            else {
                continue;
            };
            swing.hit = !reached.fumbled;
            let place = attack::where_it_lands(
                &mut rng,
                &attack::Landing {
                    monster_locations: struck.defence.armour.hit_locations,
                    on_party: arm.lands_on_party,
                },
            );
            let by = attack::piercing_mode(&mut rng, mode, arm.loaded);
            let piercing = on.piercing(surprise, &me, &struck, (place, by), &reached);
            let blow = attack::gets_through(&mut rng, &piercing).then(|| {
                attack::blow(&mut rng, &on.blowing(surprise, &me, &struck, reached.fumbled))
            });
            if blow.is_some() {
                practise(victim, Skill::IronSkin, &bodies, &mut commands);
            }
            match blow.filter(|blow| attack::lands(blow.damage)) {
                Some(struck) => {
                    let blow = Blow {
                        damage: struck.damage,
                        effects: struck.effects,
                        by_monster: arm.word.is_some(),
                    };
                    let done = deal(
                        victim,
                        on.strength,
                        blow,
                        &mut bodies,
                        &rules,
                        &mut rng,
                        (&mut log, &mut laying),
                    );
                    if reached.fumbled {
                        told_alone((actor, victim), done, &mut news);
                    } else {
                        swing.damage = done.damage;
                        effects.extend(done.rest);
                        for &skill in attack::practised_by_blow(arm.ranged, me.thrown_criticals) {
                            practise(actor, skill, &bodies, &mut commands);
                        }
                    }
                }
                None if reached.fumbled => {
                    told_alone((actor, victim), attack::Dealt::default(), &mut news);
                }
                None => {}
            }

            // Razor Cloak answers a close blow.
            if !attack::is_missile(arm.reach)
                && let Some(charge) = laying.spend(victim, SpellId::RAZOR_CLOAK)
            {
                let answer = attack::riposte(
                    &mut rng,
                    &attack::Riposte {
                        dice: content.spells[SpellId::RAZOR_CLOAK].dice,
                        power: charge.power,
                        reduction: me.reduction,
                    },
                );
                let done = if attack::lands(answer) {
                    let blow = Blow {
                        damage: answer,
                        effects: AttackEffects::default(),
                        by_monster: false,
                    };
                    let nothing = Strength::default();
                    let landing = (&mut log, &mut laying);
                    deal(actor, nothing, blow, &mut bodies, &rules, &mut rng, landing)
                } else {
                    attack::Dealt::default()
                };
                told_alone((victim, actor), done, &mut news);
            }
        }
        swung.push(swing);

        // What the swing costs, whether it lands or misses.
        let cost = attack::swing_stamina(&mut rng, &on.cost(&me));
        if let Ok(mut body) = bodies.get_mut(actor) {
            body.vitals.tire(cost);
        }

        // What is shot or thrown is spent with the swing.
        let (item, ammunition) = on.ammunition(swung.len(), &held, &strikers);
        let used = attack::uses_up(&ammunition);
        if let Some((item, amount)) = item.zip(used.spends) {
            commands.trigger(Spend { item, amount });
        }
        if used.used_up {
            turn.run_out(arm.slot);
            break;
        }
    }

    // What the attack came to.
    let totals = attack::tally(&swung);
    news.write(CombatNews::Result {
        attacker: actor,
        target,
        hits: totals.hits,
        damage: totals.damage,
    });
    if !effects.is_empty() {
        news.write(CombatNews::Effects(effects));
    }

    // The attacker's next attack.
    if let Ok(own) = bodies.get(actor) {
        let next = attack::next_attack(&attack::Hands {
            left: turn.attacks,
            initiative: EnumMap::from_fn(|slot| own.attacks.slot_initiative(slot)),
            current: turn.due,
            now,
        });
        match next {
            Some(next) => {
                turn.due = next.slot;
                turn.when = Some(next.moment);
            }
            None => turn.end(),
        }
    }
    log.scorch(&mut commands);
    for hurt in log.damaged.iter().filter(|hurt| hurt.died) {
        commands.trigger(Fell { body: hurt.body });
    }
    damaged.write_batch(log.damaged);
    for (member, skill, usage) in log.practised {
        commands.trigger(Practise { member, skill, usage });
    }
}
