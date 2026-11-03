//! Weapon attacks in a fight, swing by swing (retail `StartCharacterAttack`,
//! `ResolveCharacterAttack`, `StartMonsterAttack`, `ContinueMonsterAttack`).
//!
//! An attack is an entity made [`By`] its attacker at its target
//! ([`AimsAt`]), [`Striking`] with one of the attacker's arms: a member's
//! weapon slot or a monster's natural attack, judged alike on every body
//! ([`Arms`]). It opens from one rule ([`open_attacks`]): a body under
//! orders strikes the most afflicted foe its arm reaches, one deciding for
//! itself picks at random among what it reaches and sees.
//!
//! Swing arithmetic and immediate control flow are ordinary local values in
//! [`resolve`]. Only the accepted attack and the deeds it produces are entities.
//! Each deed settles before the next swing reads the actors' capabilities.
//!
//! Retail's voices on a miss, a kill or a fumble draw from the same random
//! numbers as the rules; here they draw nothing.

mod resolve;

use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;

use super::approach::{self, CombatMove};
use super::attack::{self, Surprise};
use super::fighters::{
    Agility, Arm, ArmourClass, Arms, Backstab, BlindFighting, HitLocations, MissileShield,
    Protection, Reduction, Stride, reaches,
};
use super::turn::{Acting, CurrentAction, Guards, Interceptions, Ordered, Resumes, Spotted, Turn};
use super::{Announcement, Battle, CombatNews, Combatants, Pause, Round, RoundPhase};
use crate::content::characters::{BodyPart, Skill, Trait};
use crate::content::items::{AttackEffect, AttackEffects, AttackMode, Dice};
use crate::content::species::Reach;
use crate::content::spells::SpellId;
use crate::game::characters::{Character, Condition, Vitals};
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::hits::defences::SaveLevel;
use crate::game::hits::definitions::{BlowDamage, Definitions, Uses, WeaponEffects};
use crate::game::hits::{Act, By, Deed, Effect};
use crate::game::inventory::{Burden, Carried, WeaponSlot};
use crate::game::magic::Strength;
use crate::game::npcs::NpcBody;
use crate::game::party::formation::{Formation, Row};
use crate::game::party::roster::{PartyMember, PartyRoster};
use crate::game::rng::GameRng;
use crate::game::world::footing::Footing;
use crate::game::world::hostility::{self, Loyalty, Provocation, Side, Stance};
use crate::game::world::navigation::Walker;
use crate::game::world::obstacles::WorldObstacles;

/// The attack is aimed at this body; a target gone ends it.
#[derive(Component, Debug)]
#[relationship(relationship_target = AimedAt)]
pub(super) struct AimsAt(pub(super) Entity);

/// The attacks aimed at a body.
#[derive(Component, Debug)]
#[relationship_target(relationship = AimsAt)]
pub(super) struct AimedAt(Vec<Entity>);

/// The arm the attack strikes with.
#[derive(Component, Debug)]
struct Striking(Arm);

/// Not yet announced: who it catches unaware is still being settled.
#[derive(Component, Debug)]
struct Unannounced;

/// How it strikes.
#[derive(Component, Clone, Copy, Debug)]
struct Mode(AttackMode);

#[derive(Component, Debug)]
struct SwingCount(u32);

/// The dice a blow rolls.
#[derive(Component, Debug)]
struct DamageDice(Dice);

/// A berserk fury, which costs twice the stamina.
#[derive(Component, Debug)]
struct Berserk;

/// What its blows have done so far: the hits that landed, damage or not,
/// the damage dealt, and what else they did.
#[derive(Component, Debug, Default)]
struct Tally {
    hits: u32,
    damage: u32,
    effects: Vec<Effect>,
}

/// Announcing an accepted attack is distinct from calculating its swings.
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
struct AttackOpening;

pub(super) fn stages(app: &mut App) {
    app.add_systems(
        AttackOpening,
        (
            crate::game::hits::refresh_capabilities,
            super::fighters::judge_fighters,
            catch_unaware,
            provoke,
            hostility::take_offence,
            crate::game::hits::refresh_capabilities,
            announce,
        )
            .chain(),
    );
}

pub(super) fn run_swings(world: &mut World) {
    let attacks: Vec<_> = world.query_filtered::<Entity, With<Striking>>().iter(world).collect();
    if attacks.is_empty() {
        return;
    }
    world.run_schedule(AttackOpening);
    for attack in attacks {
        let count = world.get::<SwingCount>(attack).map_or(0, |count| count.0);
        for _ in 0..count {
            let able = world
                .get::<By>(attack)
                .and_then(|by| world.get::<Vitals>(by.0))
                .is_some_and(Vitals::can_fight);
            let alive = world
                .get::<AimsAt>(attack)
                .and_then(|at| world.get::<Vitals>(at.0))
                .is_some_and(|v| !v.is_dead());
            if !able || !alive {
                break;
            }
            crate::game::hits::refresh_capabilities(world);
            world
                .run_system_cached(super::fighters::judge_fighters)
                .expect("fighter sources available");
            let deed = world.resource_scope(|world, mut rng: Mut<GameRng>| {
                resolve::swing(world, &mut rng, attack)
            });
            let exhausted = resolve::spend_ammunition(world, attack);
            crate::game::hits::run_hits(world);
            if let Some(deed) = deed
                && let Some(record) = world.get::<Deed>(deed)
            {
                let effects = record.effects.clone();
                if let Some(mut tally) = world.get_mut::<Tally>(attack) {
                    for effect in effects {
                        match effect {
                            Effect::Damaged { amount, .. } => tally.damage += amount,
                            other => tally.effects.push(other),
                        }
                    }
                }
                world.despawn(deed);
            }
            if exhausted {
                break;
            }
        }
    }
    world.run_system_cached(end_attacks).expect("attack completion inputs available");
}

/// The step of the round being played.
fn step(round: &Round) -> u32 {
    match round.phase {
        RoundPhase::Executing(step) => step,
        _ => 1,
    }
}

/// Everyone in the fight: the party's members, then the monsters.
fn everyone(combatants: &[Entity]) -> impl Iterator<Item = Entity> + '_ {
    combatants.iter().copied()
}

/// The acting body attacks (retail `StartCharacterAttack`,
/// `StartMonsterAttack`): with the slot due, at someone it may attack and
/// reaches, by one of the slot's arms and modes. One under orders strikes
/// the most afflicted foe, then one "in reach" of the party, then the
/// nearest; ties keep the fight's order. One deciding for itself picks at
/// random among what it reaches and sees, a wary one's wait among them
/// (retail `ChooseMonsterTarget`, `BuildMonsterActionQueue`); a member's
/// Stealth may keep them out of its eye this time, and one that stayed
/// out of it may learn from it. A slot that cannot strike (out of shots,
/// or nobody in reach) gives up its attacks and the other takes over. With
/// nobody to strike, a body that walks closes in on a foe and acts again
/// later in the round; anyone else is done for the round.
#[allow(clippy::type_complexity)]
pub(super) fn open_attacks(
    mut commands: Commands,
    mut actors: Query<
        (
            Entity,
            &CurrentAction,
            &mut Turn,
            (&Arms, &Vitals, &Side, &Loyalty, &Footing, &SaveLevel),
            (Has<Ordered>, Option<(&Stride, &Walker)>),
        ),
        With<Acting>,
    >,
    bodies: Query<(&Vitals, &Side, &Loyalty, &Footing, (Option<&Arms>, Has<NpcBody>))>,
    mut characters: Query<&mut Character>,
    fight: Single<(&mut Round, &mut Pause, Option<&Combatants>), With<Battle>>,
    (formation, roster): (Single<&Formation>, Single<&PartyRoster>),
    obstacles: WorldObstacles,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    let (mut round, mut pause, combatants) = fight.into_inner();
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    let rng = &mut *rng;
    let step = step(&round);
    let party_at = roster.iter().find_map(|member| bodies.get(member).ok().map(|(.., at, _)| *at));
    for (actor, fights, mut turn, (arms, vitals, side, loyalty, at, level), (ordered, walks)) in
        &mut actors
    {
        let CurrentAction::Fight(fights) = fights else { continue };
        // Attacking is its action: it holds the fight for the action's pause.
        commands.entity(actor).remove::<(CurrentAction, Resumes, Acting)>();
        pause.0 = Timer::from_seconds(super::ACTION_SECONDS, TimerMode::Once);
        let mine = (side.0, *loyalty);
        let wanted = if turn.berserk { Stance::Friendly } else { Stance::Hostile };
        // Those a wild attacker's eye misses this time.
        let unseen: Vec<Entity> = if ordered {
            Vec::new()
        } else {
            roster
                .iter()
                .filter(|&member| {
                    let stealth = characters.get(member).map_or(0, |c| c.skill(Skill::Stealth));
                    rng.below(100) < stealth * 75 / 100
                })
                .collect()
        };
        let mut evaded = Vec::new();
        let mut chosen = None;
        for slot in [turn.due, turn.due.other()] {
            if turn.attacks[slot] == 0 {
                continue;
            }
            // Whom each of the slot's arms reaches on the side it fights.
            let mut candidates: Vec<(&Arm, AttackMode, Entity)> = Vec::new();
            for arm in arms.of(slot) {
                for other in everyone(combatants) {
                    let Ok((theirs, their_side, their_loyalty, there, (_, npc))) =
                        bodies.get(other)
                    else {
                        continue;
                    };
                    if other == actor
                        || theirs.is_dead()
                        || hostility::between(mine, (their_side.0, *their_loyalty)) != wanted
                        || (ordered && npc)
                        || !reaches(&formation, (actor, at), (other, there), arm.reach)
                        || (!ordered && !obstacles.clear(at.eye, there.centre))
                    {
                        continue;
                    }
                    if unseen.contains(&other) {
                        if !evaded.contains(&other) {
                            evaded.push(other);
                        }
                        continue;
                    }
                    candidates.extend(arm.modes.iter().map(|mode| (arm, mode, other)));
                }
            }
            if candidates.is_empty() {
                continue;
            }
            chosen = Some((slot, candidates));
            break;
        }
        for member in evaded {
            if let Ok(mut character) = characters.get_mut(member)
                && rng.below(2) == 0
                && character.practise(Skill::Stealth, 1, rng)
            {
                news.write(CombatNews::Improved { member, skill: Skill::Stealth });
            }
        }
        let Some((slot, candidates)) = chosen else {
            // A body that walks closes in on a foe, hurt ones charging.
            let goal = walks.filter(|_| !turn.moved).and_then(|(stride, walker)| match side.0 {
                Stance::Friendly => combatants
                    .iter()
                    .filter_map(|&other| {
                        let (theirs, their_side, their_loyalty, there, _) =
                            bodies.get(other).ok()?;
                        let toward = hostility::between(mine, (their_side.0, *their_loyalty));
                        (!theirs.is_dead() && toward == wanted)
                            .then_some((at.feet.distance_squared(there.feet), other))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .filter(|_| walker.speed > 0.0)
                    .map(|(_, target)| {
                        let charge = vitals.charges();
                        let metres = approach::move_metres(vitals, stride.0, charge);
                        (
                            approach::MoveBudget { metres, charge },
                            approach::MoveGoal::Attack(target),
                        )
                    }),
                Stance::Hostile => party_at.and_then(|party| {
                    approach::move_budget(
                        (arms.reach(), stride.0),
                        vitals,
                        Some(walker),
                        at.gap(&party),
                    )
                    .map(|budget| (budget, approach::MoveGoal::Party))
                }),
                Stance::Neutral => None,
            });
            match goal {
                Some((budget, goal)) => {
                    turn.moved = true;
                    turn.phase = (step * 2).min(100);
                    commands.entity(actor).insert(CombatMove {
                        budget: budget.metres,
                        moved: 0.0,
                        goal,
                    });
                    news.write(CombatNews::Moves { monster: actor, charge: budget.charge });
                }
                None => turn.end(),
            }
            continue;
        };
        // A wary body's wait comes first among its choices.
        let waits = usize::from(fights.waits);
        let (arm, mode, target) = if ordered {
            let best = candidates
                .iter()
                .map(|&(arm, _, other)| {
                    let (theirs, _, _, there, (their_arms, _)) = bodies.get(other).unwrap();
                    let quadrant = at.quadrant(there.feet);
                    let in_reach = their_arms.is_some_and(|arms| !arms.0.is_empty())
                        && (quadrant == Row::Rear
                            || (!formation.occupied(quadrant) && formation.occupied(Row::Center)));
                    ((theirs.conditions.worst(), !in_reach, at.gap(there)), arm, other)
                })
                .min_by(|a, b| {
                    a.0.0.cmp(&b.0.0).then(a.0.1.cmp(&b.0.1)).then(a.0.2.total_cmp(&b.0.2))
                })
                .map(|(_, arm, other)| (arm, other))
                .unwrap();
            // Retail draws the attack's mode, then its swings.
            let mode = if fights.berserk {
                AttackMode::Berserk
            } else {
                attack::choose_mode(rng, best.0.modes)
            };
            (best.0, mode, best.1)
        } else {
            let pick = rng.below((candidates.len() + waits) as u32) as usize;
            if pick < waits {
                turn.waited = true;
                turn.end();
                continue;
            }
            let (arm, mode, target) = candidates[pick - waits];
            // Retail draws for the mode again, from just the one chosen.
            (arm, attack::choose_mode(rng, enumset::EnumSet::only(mode)), target)
        };
        round.attacked(true);
        turn.due = slot;
        turn.attacks[slot] -= 1;
        let mut swings = rng.below(arm.swings) + 1;
        swings = (swings as i32 + arm.extra_swings).max(1) as u32;
        // A Samurai's Lightning Strike, readied as the round began, with a
        // sword in the primary slot.
        if slot == WeaponSlot::Primary
            && let Some(samurai) = turn.lightning.take()
            && arm.sword
            && !vitals.conditions.has(Condition::Slowed)
        {
            swings += if rng.below(11) + u32::from(samurai) < 20 { 3 } else { 4 };
        }
        let dice = match mode {
            AttackMode::Punch => arm.punch,
            AttackMode::Kick => arm.kick,
            _ => arm.dice,
        };
        let power = attack::effect_power(level.0);
        let mut attack = commands.spawn((
            Unannounced,
            By(actor),
            AimsAt(target),
            Striking(arm.clone()),
            Mode(mode),
            SwingCount(swings),
            DamageDice(dice),
            Surprise { unaware: false, natural: arm.ambushes },
            Strength { power, against: power, boost: 0 },
            Tally::default(),
        ));
        if fights.berserk {
            attack.insert(Berserk);
        }
    }
}

/// A close blow from a side its target does not face catches it unaware
/// (retail `IsPartyLookingAwayFrom`, `CharacterNoticesAttacker`): a
/// member seated in the formation may turn in time, if awake, unblinded
/// and defending; the first time each round always, then by senses, 25
/// harder each time.
fn catch_unaware(
    mut attacks: Query<(&By, &AimsAt, &Striking, &mut Surprise), With<Unannounced>>,
    footings: Query<&Footing>,
    mut members: Query<(&Character, &Vitals, &Ordered, &mut Spotted)>,
    mut formation: Single<&mut Formation>,
    mut rng: ResMut<GameRng>,
) {
    for (by, aims, striking, mut surprise) in &mut attacks {
        if striking.0.reach > Reach::Short {
            continue;
        }
        let (Ok(attacker), Ok(target)) = (footings.get(by.0), footings.get(aims.0)) else {
            continue;
        };
        let quadrant = target.quadrant(attacker.feet);
        let facing = formation.facing(aims.0).unwrap_or(Row::Front);
        if facing == quadrant {
            continue;
        }
        let notices =
            members.get_mut(aims.0).is_ok_and(|(character, vitals, ordered, mut spotted)| {
                let alert = !vitals.is_dead()
                    && vitals.stamina.current > 0
                    && vitals.conditions.worst().is_none_or(|c| c < Condition::Webbed)
                    && !vitals.conditions.has(Condition::Blind);
                alert
                    && ordered.defends()
                    && if spotted.0 == 0 {
                        spotted.0 = 1;
                        true
                    } else {
                        // Retail compares unsigned: once senses fall short,
                        // noticing is certain.
                        let senses = character
                            .stats(&vitals.enchantments)
                            .attribute(crate::content::characters::Attribute::Senses);
                        let chance = (senses as i32 - spotted.0 as i32 * 25) as u32;
                        spotted.0 += 1;
                        rng.below(100) < chance
                    }
            });
        if notices {
            formation.turn(aims.0, quadrant);
        } else {
            surprise.unaware = facing.opposite() == Some(quadrant);
        }
    }
}

/// The party attacks a monster: it is provoked, whatever the blows then do
/// (retail `MakeTargetGroupHostile` as the action starts). A member under
/// Turncoat provokes nobody.
fn provoke(
    attacks: Query<(&By, &AimsAt), With<Unannounced>>,
    members: Query<&Vitals, With<PartyMember>>,
    mut provocations: MessageWriter<Provocation>,
) {
    for (by, aims) in &attacks {
        if members.get(by.0).is_ok_and(|vitals| !vitals.conditions.has(Condition::Turncoat)) {
            provocations.write(Provocation(aims.0));
        }
    }
}

/// Each attack is announced as it begins (retail `StartCharacterAttack`,
/// `AnnounceMonsterAttack`), and is under way.
fn announce(
    mut commands: Commands,
    attacks: Query<
        (Entity, &By, &AimsAt, &Mode, &SwingCount, &Surprise, &Striking),
        With<Unannounced>,
    >,
    mut news: MessageWriter<CombatNews>,
) {
    for (attack, by, aims, mode, swings, surprise, striking) in &attacks {
        let how = match striking.0.word {
            Some((word, named)) => Announcement::Monster { word, mode: mode.0, named },
            None => Announcement::Mode(mode.0),
        };
        news.write(CombatNews::Attack {
            attacker: by.0,
            target: aims.0,
            how,
            swings: swings.0,
            surprise: *surprise,
        });
        commands.entity(attack).remove::<Unannounced>();
    }
}

/// A spent attack reports once its blows have landed (retail
/// `ReportCharacterAttackResult`), and the attacker's next attack follows
/// (retail `ComputeCharacterActionPhase`): with the other slot while it
/// has attacks, unless it has fewer and less initiative; spread over the
/// rest of the round, a switch costing the difference in the slots'
/// initiative.
fn end_attacks(
    mut commands: Commands,
    mut attacks: Query<(Entity, &mut Tally, &By, Option<&AimsAt>), With<Striking>>,
    mut attackers: Query<(&mut Turn, &Arms)>,
    round: Single<&Round>,
    mut news: MessageWriter<CombatNews>,
) {
    let step = step(&round);
    for (attack, mut tally, by, aims) in &mut attacks {
        if let Some(aims) = aims {
            news.write(CombatNews::Result {
                attacker: by.0,
                target: aims.0,
                hits: tally.hits,
                damage: tally.damage,
            });
        }
        let effects = std::mem::take(&mut tally.effects);
        if !effects.is_empty() {
            news.write(CombatNews::Effects(effects));
        }
        commands.entity(attack).despawn();
        let Ok((mut turn, arms)) = attackers.get_mut(by.0) else { continue };
        let current = turn.due;
        let other = current.other();
        let (now, then) = (turn.attacks[current], turn.attacks[other]);
        let chosen =
            if then != 0 && !(then < now && arms.initiative(other) < arms.initiative(current)) {
                other
            } else {
                current
            };
        let left = turn.attacks[chosen];
        let phase = if left == 0 {
            None
        } else if chosen == current || then <= now {
            Some(step + (100 - step) / (left + 1))
        } else {
            Some(step)
        };
        match phase.filter(|&phase| phase <= 100) {
            Some(phase) => {
                let switch = if chosen != current {
                    arms.initiative(current) - arms.initiative(chosen)
                } else {
                    0
                };
                turn.due = chosen;
                turn.phase = (phase as i32 + switch).clamp(step as i32, 100) as u32;
            }
            None => turn.end(),
        }
    }
}
