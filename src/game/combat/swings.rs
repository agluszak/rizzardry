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
//! Each swing is an entity of its own ([`SwingOf`]) against a body
//! ([`At`]): small systems add the retail terms to its [`Aim`], it lands or
//! misses, then to its [`Penetration`], and a blow that gets through deals
//! its [`Damage`] and becomes an act whose effects land through the hit
//! stages. The attack tallies what its swings did ([`Tally`]) and reports
//! once it is spent. [`run_swings`] runs the stages until every attack is
//! over, within the attacker's step, so each swing sees what the last did.
//!
//! Retail's voices on a miss, a kill or a fumble draw from the same random
//! numbers as the rules; here they draw nothing.

use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;

use super::approach::{self, CombatMove};
use super::attack::{self, Location, Surprise};
use super::fighters::{
    Agility, Arm, ArmourClass, Arms, Backstab, BlindFighting, HitLocations, MissileShield,
    Protection, Reduction, Stride, reaches,
};
use super::turn::{
    Acting, Decided, Fights, Guards, Interceptions, Ordered, Resumes, Spotted, Turn,
};
use super::{Announcement, Battle, CombatNews, Combatants, Pause, Round, RoundPhase};
use crate::content::characters::{BodyPart, Skill, Trait};
use crate::content::items::{AttackEffect, AttackEffects, AttackMode, Dice};
use crate::content::species::Reach;
use crate::content::spells::SpellId;
use crate::game::characters::{Character, Condition, Vitals};
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::hits::defences::SaveLevel;
use crate::game::hits::prototypes::{BlowDamage, Definitions, Uses, WeaponEffects};
use crate::game::hits::spells::reach;
use crate::game::hits::{Act, By, Deed, DeedHits, Effect};
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
use crate::game::world::population::Monster;

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

/// The swing belongs to this attack.
#[derive(Component, Debug)]
#[relationship(relationship_target = Swings)]
struct SwingOf(Entity);

/// An attack's swing under way, gone with it.
#[derive(Component, Debug)]
#[relationship_target(relationship = SwingOf, linked_spawn)]
struct Swings(Vec<Entity>);

/// The body the swing is against: its target while it aims, whoever it
/// struck once it lands.
#[derive(Component, Debug)]
#[relationship(relationship_target = Faced)]
struct At(Entity);

/// The swings against a body.
#[derive(Component, Debug)]
#[relationship_target(relationship = At)]
struct Faced(Vec<Entity>);

/// Not yet announced: who it catches unaware is still being settled.
#[derive(Component, Debug)]
struct Unannounced;

/// How it strikes.
#[derive(Component, Clone, Copy, Debug)]
struct Mode(AttackMode);

#[derive(Component, Debug)]
struct SwingsLeft(u32);

/// The dice a blow rolls.
#[derive(Component, Debug)]
struct DamageDice(Dice);

/// A berserk fury, which costs twice the stamina.
#[derive(Component, Debug)]
struct Berserk;

/// It can go no further; it reports once its blows have landed.
#[derive(Component, Debug)]
struct Spent;

/// What its blows have done so far: the hits that landed, damage or not,
/// the damage dealt, and what else they did.
#[derive(Component, Debug, Default)]
struct Tally {
    hits: u32,
    damage: u32,
    effects: Vec<Effect>,
}

/// The swing's aim at its target, before the roll.
#[derive(Component, Debug)]
struct Aim(i32);

/// The swing's aim before its target's guard, which decides whether it goes
/// astray.
#[derive(Component, Debug)]
struct FumbleOdds(i32);

/// The swing landed, with this roll against its aim and penetration.
#[derive(Component, Debug)]
struct Landed {
    roll: u32,
}

/// It struck whatever its aim: a roll of 5 or under, a fumble, or a missile
/// that flew true.
#[derive(Component, Debug)]
struct Sure;

/// It gets through armour whatever the roll.
#[derive(Component, Debug)]
struct Through;

/// It went astray onto a bystander.
#[derive(Component, Debug)]
struct Fumbled;

/// The swing is over without striking.
#[derive(Component, Debug)]
struct Missed;

/// The swing landed and pierces by this mode.
#[derive(Component, Debug)]
struct Pierces(AttackMode);

/// The swing's penetration of what it struck, before the roll.
#[derive(Component, Debug)]
struct Penetration(i32);

/// A Missile Shield turned it aside.
#[derive(Component, Debug)]
struct Deflected;

/// How many times the blow's dice are rolled; only a swing that got through
/// has them.
#[derive(Component, Debug)]
struct DiceCount(u32);

/// What the blow deals; nothing for one turned aside.
#[derive(Component, Debug)]
struct Damage(u32);

/// The blow's chances to do more than damage.
#[derive(Component, Debug)]
struct Chances(AttackEffects);

/// The swing is over.
#[derive(Component, Debug)]
struct Done;

/// The stages an attack goes through, run by [`run_swings`].
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct SwingStages;

/// Run the swing stages until every attack under way is over.
pub(super) fn run_swings(world: &mut World) {
    let mut passes = 0;
    while world.query_filtered::<(), With<AimsAt>>().iter(world).next().is_some() {
        passes += 1;
        if passes > 256 {
            error!("attacks left unsettled after {passes} passes");
            break;
        }
        world.run_schedule(SwingStages);
        // What a blow did lands before the next swing.
        crate::game::hits::run_hits(world);
    }
}

pub(super) fn stages(app: &mut App) {
    use super::fighters::{judge_members, judge_monsters};
    use crate::game::hits::defences;
    use crate::game::world::footing::{place_members, place_monsters};
    app.init_schedule(SwingStages).add_systems(
        SwingStages,
        (
            (
                place_members,
                place_monsters,
                defences::judge_characters,
                defences::judge_monsters,
                judge_members,
                judge_monsters,
            )
                .chain(),
            (catch_unaware, provoke, hostility::take_offence, announce).chain(),
            (tally_swings, start_swings).chain(),
            (
                rate,
                mode_to_hit,
                surprise_to_hit,
                fatigue_to_hit,
                fumble_odds,
                armour_to_hit,
                agility_to_hit,
                blind,
                difficulty_to_hit,
            )
                .chain(),
            (shoot, swing_at, intercept, locate, dodge).chain(),
            (
                mode_penetration,
                fatigue_penetration,
                armour_penetration,
                agility_penetration,
                difficulty_penetration,
            )
                .chain(),
            (
                deflect,
                pierce,
                exposed_dice,
                natural_dice,
                berserk_dice,
                braced,
                slaying_dice,
                roll_damage,
                damage_percent,
                at_least_one,
                reduce,
                fumble_caps,
            )
                .chain(),
            (iron_skin, critical_strike, knockout, land_blows, train_strikes, riposte, count_hit)
                .chain(),
            (tire_swingers, spend_ammunition, finish_swing, end_attacks).chain(),
        )
            .chain(),
    );
}

/// The step of the round being played.
fn step(round: &Round) -> u32 {
    match round.phase {
        RoundPhase::Executing(step) => step,
        _ => 1,
    }
}

/// Everyone in the fight: the party's members, then the monsters.
fn everyone<'a>(
    roster: &'a PartyRoster,
    combatants: &'a [Entity],
) -> impl Iterator<Item = Entity> + 'a {
    roster.iter().chain(combatants.iter().copied())
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
            &Fights,
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
        // Attacking is its action: it holds the fight for the action's pause.
        commands.entity(actor).remove::<(Fights, Decided, Resumes, Acting)>();
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
                for other in everyone(&roster, combatants) {
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
                turn.attacks[slot] = 0;
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
            SwingsLeft(swings),
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
        (Entity, &By, &AimsAt, &Mode, &SwingsLeft, &Surprise, &Striking),
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

/// Each swing whose blow has landed is counted on its attack: its damage
/// toward the summary, and what else it did.
fn tally_swings(
    mut commands: Commands,
    swings: Query<(Entity, &Deed, &SwingOf), (With<Done>, Without<DeedHits>)>,
    mut attacks: Query<&mut Tally>,
) {
    for (swing, deed, swing_of) in &swings {
        if deed.act != Act::Swing {
            continue;
        }
        commands.entity(swing).despawn();
        let Ok(mut tally) = attacks.get_mut(swing_of.0) else { continue };
        for effect in &deed.effects {
            match *effect {
                Effect::Damaged { amount, .. } => tally.damage += amount,
                effect => tally.effects.push(effect),
            }
        }
    }
}

/// An attack with swings left begins its next one while the attacker can
/// still fight and the target is alive in the fight; otherwise it is spent.
fn start_swings(
    mut commands: Commands,
    attacks: Query<
        (Entity, &By, Option<&AimsAt>),
        (Without<Unannounced>, Without<Swings>, Without<Spent>),
    >,
    bodies: Query<&Vitals, With<Footing>>,
) {
    for (attack, by, aims) in &attacks {
        let able = bodies.get(by.0).is_ok_and(Vitals::can_fight);
        let target = aims
            .map(|aims| aims.0)
            .filter(|&target| bodies.get(target).is_ok_and(|vitals| !vitals.is_dead()));
        match target.filter(|_| able) {
            Some(target) => {
                commands.spawn((SwingOf(attack), At(target), Aim(0)));
            }
            None => {
                commands.entity(attack).insert(Spent);
            }
        }
    }
}

/// How well the attacker strikes with its arm.
fn rate(mut swings: Query<(&mut Aim, &SwingOf), Without<Landed>>, attacks: Query<&Striking>) {
    for (mut aim, swing_of) in &mut swings {
        if let Ok(striking) = attacks.get(swing_of.0) {
            aim.0 += striking.0.rating;
        }
    }
}

/// What the mode adds to the aim (retail `AttackModeMod`).
fn mode_to_hit(mut swings: Query<(&mut Aim, &SwingOf), Without<Landed>>, attacks: Query<&Mode>) {
    for (mut aim, swing_of) in &mut swings {
        if let Ok(mode) = attacks.get(swing_of.0) {
            aim.0 += attack::mode_to_hit(mode.0);
        }
    }
}

/// A surprise steadies the aim: ten for an unaware target, ten for a
/// natural strike.
fn surprise_to_hit(
    mut swings: Query<(&mut Aim, &SwingOf), Without<Landed>>,
    attacks: Query<&Surprise>,
) {
    for (mut aim, swing_of) in &mut swings {
        if let Ok(surprise) = attacks.get(swing_of.0) {
            aim.0 += 10 * i32::from(surprise.unaware) + 10 * i32::from(surprise.natural);
        }
    }
}

/// Fatigue tells on the attacker's aim, half as much with a modern weapon.
fn fatigue_to_hit(
    mut swings: Query<(&mut Aim, &SwingOf), Without<Landed>>,
    attacks: Query<(&By, &Striking)>,
    vitals: Query<&Vitals>,
) {
    for (mut aim, swing_of) in &mut swings {
        let Some((by, striking)) = attacks.get(swing_of.0).ok() else { continue };
        let fatigue = attack::fatigue(vitals.get(by.0).unwrap());
        aim.0 -= if striking.0.modern { fatigue >> 1 } else { fatigue };
    }
}

/// A swing's odds before its target's guard decide whether it goes astray.
fn fumble_odds(
    mut commands: Commands,
    swings: Query<(Entity, &Aim, &SwingOf), Without<Landed>>,
    attacks: Query<&Striking>,
) {
    for (swing, aim, swing_of) in &swings {
        if attacks.get(swing_of.0).ok().is_some_and(|striking| striking.0.reach < Reach::Long) {
            commands.entity(swing).insert(FumbleOdds(aim.0));
        }
    }
}

/// The faced body's armour turns the swing aside.
fn armour_to_hit(
    mut swings: Query<(&mut Aim, &At, &SwingOf), Without<Landed>>,
    attacks: Query<(&Mode, &Surprise)>,
    armours: Query<&ArmourClass>,
) {
    for (mut aim, at, swing_of) in &mut swings {
        let (Some((mode, surprise)), Ok(armour)) =
            (attacks.get(swing_of.0).ok(), armours.get(at.0))
        else {
            continue;
        };
        aim.0 -= 5 * armour.against_aim(mode.0, *surprise);
    }
}

/// The agility between attacker and faced body counts a tenth.
fn agility_to_hit(
    mut swings: Query<(&mut Aim, &At, &SwingOf), Without<Landed>>,
    attacks: Query<&By>,
    agilities: Query<&Agility>,
) {
    for (mut aim, at, swing_of) in &mut swings {
        let Some(by) = attacks.get(swing_of.0).ok() else { continue };
        let (Ok(theirs), Ok(mine)) = (agilities.get(at.0), agilities.get(by.0)) else { continue };
        aim.0 += (mine.0 - theirs.0) / 10;
    }
}

/// A blinded attacker's aim is worth 10 at most, more by level for one
/// effective while blind.
fn blind(
    mut swings: Query<(&mut Aim, Option<&mut FumbleOdds>, &SwingOf), Without<Landed>>,
    attacks: Query<&By>,
    attackers: Query<(&Vitals, &BlindFighting)>,
) {
    for (mut aim, odds, swing_of) in &mut swings {
        let Some(by) = attacks.get(swing_of.0).ok() else { continue };
        let Ok((vitals, fighting)) = attackers.get(by.0) else { continue };
        if !vitals.conditions.has(Condition::Blind) {
            continue;
        }
        let cap = |aim: &mut i32| {
            if *aim > 10 {
                *aim = match fighting.0 {
                    Some(level) => {
                        crate::game::characters::scale_by_level(level, (*aim - 10) as f32) as i32
                            + 10
                    }
                    None => 10,
                };
            }
        };
        cap(&mut aim.0);
        if let Some(mut odds) = odds {
            cap(&mut odds.0);
        }
    }
}

/// The difficulty scales each swing's aim by the side the attacker is on.
fn difficulty_to_hit(
    mut swings: Query<(&mut Aim, Option<&mut FumbleOdds>, &SwingOf), Without<Landed>>,
    attacks: Query<&By>,
    sides: Query<&Side>,
    difficulty: Res<Difficulty>,
) {
    for (mut aim, odds, swing_of) in &mut swings {
        let Some(by) = attacks.get(swing_of.0).ok() else { continue };
        let side = sides.get(by.0).unwrap().0;
        aim.0 = difficulty.scale(aim.0, side);
        if let Some(mut odds) = odds {
            odds.0 = difficulty.scale(odds.0, side);
        }
    }
}

/// A shot scatters by its aim from the attacker's eye and lands where it
/// strikes its target's body (retail `ScatterMissileAimPoint`). A Ninja's
/// throw gets through whatever its roll.
fn shoot(
    mut commands: Commands,
    swings: Query<(Entity, &Aim, &At, &SwingOf), (Without<Landed>, Without<Missed>)>,
    attacks: Query<(&By, &Striking)>,
    bodies: Query<(&Vitals, &Footing)>,
    mut rng: ResMut<GameRng>,
) {
    for (swing, aim, at, swing_of) in &swings {
        let Some((by, striking)) = attacks.get(swing_of.0).ok() else { continue };
        if striking.0.reach < Reach::Long {
            continue;
        }
        let (Ok((vitals, from)), Ok((_, target))) = (bodies.get(by.0), bodies.get(at.0)) else {
            continue;
        };
        let blind = vitals.conditions.has(Condition::Blind);
        let aimed = attack::scatter(&mut rng, from.eye, target.centre, aim.0, blind);
        let mut shot = commands.entity(swing);
        if !attack::strikes(target.centre, aimed, target.radius, target.half_height) {
            shot.insert(Missed);
            continue;
        }
        shot.insert((Landed { roll: rng.below(100) + 1 }, Sure));
        if striking.0.sure_throw {
            shot.insert(Through);
        }
    }
}

/// A swing may go astray onto those around (retail
/// `BuildCharacterTargetList`, `BuildMonsterTargetList`): the attacker
/// itself and everyone else alive in the fight it reaches, but the one aimed
/// at. It reaches its target, misses, or goes astray, which spends its
/// surprise.
fn swing_at(
    mut commands: Commands,
    swings: Query<(Entity, &At, &FumbleOdds, &SwingOf), (Without<Landed>, Without<Missed>)>,
    mut attacks: Query<(&By, &Striking, &mut Surprise)>,
    bodies: Query<(&Vitals, &Footing)>,
    fight: Single<Option<&Combatants>, With<Battle>>,
    (formation, roster): (Single<&Formation>, Single<&PartyRoster>),
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    let combatants = fight.map_or(&[][..], Combatants::entities);
    for (swing, at, odds, swing_of) in &swings {
        let Ok((by, striking, mut surprise)) = attacks.get_mut(swing_of.0) else { continue };
        let Ok((_, from)) = bodies.get(by.0) else { continue };
        let bystanders: Vec<Entity> = everyone(&roster, combatants)
            .filter(|&other| other != at.0)
            .filter(|&other| {
                bodies.get(other).is_ok_and(|(vitals, there)| {
                    !vitals.is_dead()
                        && (other == by.0
                            || reaches(&formation, (by.0, from), (other, there), striking.0.reach))
                })
            })
            .collect();
        let fumble = attack::fumble_chance(bystanders.len(), odds.0);
        let mut swing = commands.entity(swing);
        match attack::swing(&mut rng, bystanders.len(), fumble) {
            attack::Aim::Missed => {
                swing.insert(Missed);
            }
            attack::Aim::Reaches { roll, sure } => {
                swing.insert(Landed { roll });
                if sure {
                    swing.insert((Sure, Through));
                }
            }
            attack::Aim::Fumbled { bystander, roll } => {
                *surprise = Surprise::default();
                let victim = bystanders[bystander];
                swing.insert((Landed { roll }, Sure, Fumbled, At(victim)));
                news.write(CombatNews::Fumble { attacker: by.0, victim });
            }
        }
    }
}

/// A guard may take a swing that landed on its ward (retail
/// `ResolveGuardianInterception`): each guard of the victim still able
/// (alive, not blinded or worse, within its first arm's reach, and under
/// its attacks a round in blows taken) rolls its protection against the
/// attacker's, raised for an unaware or natural strike, within 5 and 95
/// percent; one that succeeds, picked at random, takes the blow, tiring by
/// 2 to 4, and the attack's surprise is spent.
fn intercept(
    mut commands: Commands,
    swings: Query<(Entity, &At, &SwingOf), (With<Landed>, Without<Pierces>)>,
    mut attacks: Query<(&By, &mut Surprise)>,
    wards: Query<(&Guards, &Vitals, &Footing)>,
    protections: Query<&Protection>,
    mut guards: Query<(&Arms, &mut Vitals, &Footing, &mut Interceptions), Without<Guards>>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    for (swing, at, swing_of) in &swings {
        let ward = at.0;
        let Ok((guarding, ward_vitals, ward_at)) = wards.get(ward) else { continue };
        let Ok((by, mut surprise)) = attacks.get_mut(swing_of.0) else { continue };
        let Ok(pressing) = protections.get(by.0) else { continue };
        if ward_vitals.is_dead() {
            continue;
        }
        let pressing =
            pressing.0 + 10 * i32::from(surprise.unaware) + 10 * i32::from(surprise.natural);
        let mut stepping = Vec::new();
        for &guard in guarding.entities() {
            let (Ok((arms, vitals, there, taken)), Ok(protection)) =
                (guards.get(guard), protections.get(guard))
            else {
                continue;
            };
            let Some(first) = arms.0.first() else { continue };
            if !(vitals.can_guard()
                && there.gap(ward_at) <= first.reach.metres()
                && taken.taken < taken.allowed)
            {
                continue;
            }
            let chance = (protection.0 - pressing + 50).clamp(5, 95) as u32;
            if rng.below(100) < chance {
                stepping.push(guard);
            }
        }
        if stepping.is_empty() {
            continue;
        }
        let guard = stepping[rng.below(stepping.len() as u32) as usize];
        let (_, mut vitals, _, mut taken) = guards.get_mut(guard).unwrap();
        taken.taken += 1;
        vitals.tire(rng.below(3) + 2);
        *surprise = Surprise::default();
        commands.entity(swing).insert(At(guard));
        news.write(CombatNews::Intercepts { guard, ward });
    }
}

/// Where a landed blow strikes: by the struck body's own chances, or on a
/// member by the attacker's where its species has them, else the party's.
fn locate(
    mut commands: Commands,
    swings: Query<(Entity, &At, &SwingOf), (With<Landed>, Without<Pierces>)>,
    attacks: Query<&Striking>,
    tables: Query<&HitLocations>,
    mut rng: ResMut<GameRng>,
) {
    for (swing, at, swing_of) in &swings {
        let Some(striking) = attacks.get(swing_of.0).ok() else { continue };
        let location = match (tables.get(at.0), striking.0.lands_on_party) {
            (Ok(table), _) => attack::locate(&mut rng, &table.0, attack::MONSTER_BODY),
            (Err(_), Some(hits)) => {
                attack::locate(&mut rng, hits.as_slice(), BodyPart::Body as usize)
            }
            (Err(_), None) => {
                attack::locate(&mut rng, &attack::MEMBER_LOCATIONS, BodyPart::Body as usize)
            }
        };
        commands.entity(swing).insert(location);
    }
}

/// A landed swing whose roll beats its aim is dodged, unless it was sure;
/// one that was not goes on to pierce, a launcher's by a mode its
/// ammunition strikes with.
fn dodge(
    mut commands: Commands,
    swings: Query<(Entity, &Aim, &Landed, Has<Sure>, &SwingOf), Without<Pierces>>,
    attacks: Query<(&Mode, &Striking)>,
    mut rng: ResMut<GameRng>,
) {
    for (swing, aim, landed, sure, swing_of) in &swings {
        let Some((mode, striking)) = attacks.get(swing_of.0).ok() else { continue };
        let mut swing = commands.entity(swing);
        if !sure && aim.0 < landed.roll as i32 {
            swing.insert(Missed);
            continue;
        }
        let piercing =
            striking.0.loaded.map_or(mode.0, |loaded| attack::choose_mode(&mut rng, loaded));
        swing.remove::<FumbleOdds>().insert((Pierces(piercing), Penetration(0)));
    }
}

/// What the mode the blow pierces with adds (retail `AttackModeMod`).
fn mode_penetration(mut swings: Query<(&mut Penetration, &Pierces), Without<Damage>>) {
    for (mut penetration, pierces) in &mut swings {
        penetration.0 += attack::mode_penetration(pierces.0);
    }
}

/// Fatigue tells on penetration where the arm's does.
fn fatigue_penetration(
    mut swings: Query<(&mut Penetration, &SwingOf), Without<Damage>>,
    attacks: Query<(&By, &Striking)>,
    vitals: Query<&Vitals>,
) {
    for (mut penetration, swing_of) in &mut swings {
        let Some((by, striking)) = attacks.get(swing_of.0).ok() else { continue };
        if striking.0.fatigue_pierces {
            penetration.0 -= attack::fatigue(vitals.get(by.0).unwrap());
        }
    }
}

/// The struck body's armour where the blow struck.
fn armour_penetration(
    mut swings: Query<(&mut Penetration, &At, &Pierces, &Location, &SwingOf), Without<Damage>>,
    attacks: Query<&Surprise>,
    armours: Query<&ArmourClass>,
) {
    for (mut penetration, at, pierces, location, swing_of) in &mut swings {
        let (Some(surprise), Ok(armour)) = (attacks.get(swing_of.0).ok(), armours.get(at.0)) else {
            continue;
        };
        penetration.0 -= 5 * armour.at(pierces.0, *surprise, location.0);
    }
}

/// The agility between attacker and struck body counts a tenth.
fn agility_penetration(
    mut swings: Query<(&mut Penetration, &At, &SwingOf), Without<Damage>>,
    attacks: Query<&By>,
    agilities: Query<&Agility>,
) {
    for (mut penetration, at, swing_of) in &mut swings {
        let Some(by) = attacks.get(swing_of.0).ok() else { continue };
        let (Ok(theirs), Ok(mine)) = (agilities.get(at.0), agilities.get(by.0)) else { continue };
        penetration.0 += (mine.0 - theirs.0) / 10;
    }
}

/// The difficulty scales each blow's penetration by the attacker's side.
fn difficulty_penetration(
    mut swings: Query<(&mut Penetration, &SwingOf), Without<Damage>>,
    attacks: Query<&By>,
    sides: Query<&Side>,
    difficulty: Res<Difficulty>,
) {
    for (mut penetration, swing_of) in &mut swings {
        let Some(by) = attacks.get(swing_of.0).ok() else { continue };
        penetration.0 = difficulty.scale(penetration.0, sides.get(by.0).unwrap().0);
    }
}

/// A Missile Shield on the struck body turns aside what is shot or thrown.
fn deflect(
    mut commands: Commands,
    swings: Query<(Entity, &At, &SwingOf), (With<Pierces>, Without<Damage>)>,
    attacks: Query<&Mode>,
    shields: Query<&MissileShield>,
    mut rng: ResMut<GameRng>,
) {
    for (swing, at, swing_of) in &swings {
        let (Some(mode), Ok(shield)) = (attacks.get(swing_of.0).ok(), shields.get(at.0)) else {
            continue;
        };
        let shot = matches!(mode.0, AttackMode::Shoot | AttackMode::Throw);
        if shot && shield.0 > 0 && rng.below(100) < shield.0 {
            commands.entity(swing).insert(Deflected);
        }
    }
}

/// A swing gets through when it must or its penetration beats its roll;
/// otherwise, or turned aside, it glances off for nothing.
fn pierce(
    mut commands: Commands,
    swings: Query<
        (Entity, &Penetration, &Landed, Has<Through>, Has<Deflected>, &SwingOf),
        Without<Damage>,
    >,
    attacks: Query<&Striking>,
) {
    for (swing, penetration, landed, through, deflected, swing_of) in &swings {
        let Some(striking) = attacks.get(swing_of.0).ok() else { continue };
        let mut swing = commands.entity(swing);
        swing.insert(Damage(0));
        if !deflected && (through || penetration.0 >= landed.roll as i32) {
            swing.insert((DiceCount(1), Chances(striking.0.effects)));
        }
    }
}

/// A target unaware, or distracted on the move, takes twice the dice of a
/// blow that closes in on it.
fn exposed_dice(
    mut swings: Query<(&mut DiceCount, &At, &SwingOf)>,
    attacks: Query<(&Mode, &Surprise)>,
    armours: Query<&ArmourClass>,
) {
    for (mut count, at, swing_of) in &mut swings {
        let Some((mode, surprise)) = attacks.get(swing_of.0).ok() else { continue };
        let distracted = armours.get(at.0).is_ok_and(|armour| armour.distracted);
        if (surprise.unaware || distracted) && !attack::keeps_distance(mode.0) {
            count.0 = 2;
        }
    }
}

/// A natural strike that closes in rolls extra dice by the attacker's
/// backstab (retail `RollBackstabExtraDice`).
fn natural_dice(
    mut swings: Query<(&mut DiceCount, &SwingOf)>,
    attacks: Query<(&By, &Mode, &Surprise)>,
    backstabs: Query<&Backstab>,
    mut rng: ResMut<GameRng>,
) {
    for (mut count, swing_of) in &mut swings {
        let Some((by, mode, surprise)) = attacks.get(swing_of.0).ok() else { continue };
        if surprise.natural
            && !attack::keeps_distance(mode.0)
            && let Ok(backstab) = backstabs.get(by.0)
        {
            count.0 += attack::backstab_dice(&mut rng, backstab.0);
        }
    }
}

/// A berserk blow rolls a die more.
fn berserk_dice(mut swings: Query<(&mut DiceCount, &SwingOf)>, attacks: Query<&Mode>) {
    for (mut count, swing_of) in &mut swings {
        if attacks.get(swing_of.0).ok().is_some_and(|mode| mode.0 == AttackMode::Berserk) {
            count.0 += 1;
        }
    }
}

/// A braced body that saw it coming takes a die less of a blow that rolls
/// more than one.
fn braced(
    mut swings: Query<(&mut DiceCount, &At, &SwingOf)>,
    attacks: Query<&Surprise>,
    armours: Query<&ArmourClass>,
) {
    for (mut count, at, swing_of) in &mut swings {
        let Some(surprise) = attacks.get(swing_of.0).ok() else { continue };
        let braced = armours.get(at.0).is_ok_and(|armour| armour.braced);
        if count.0 > 1 && braced && !surprise.unaware {
            count.0 -= 1;
        }
    }
}

/// Each weapon that slays the struck monster's kind rolls a die more.
fn slaying_dice(
    mut swings: Query<(&mut DiceCount, &At, &SwingOf)>,
    attacks: Query<&Striking>,
    monsters: Query<&Monster>,
    content: If<Res<Catalogs>>,
) {
    for (mut count, at, swing_of) in &mut swings {
        let (Some(striking), Ok(monster)) = (attacks.get(swing_of.0).ok(), monsters.get(at.0))
        else {
            continue;
        };
        let kind = content.species[monster.species].kind;
        count.0 += striking.0.slays.iter().filter(|&&slays| slays == kind).count() as u32;
    }
}

/// The blow's dice, rolled so many times.
fn roll_damage(
    mut swings: Query<(&mut Damage, &DiceCount, &SwingOf)>,
    attacks: Query<&DamageDice>,
    mut rng: ResMut<GameRng>,
) {
    for (mut damage, count, swing_of) in &mut swings {
        if let Ok(dice) = attacks.get(swing_of.0) {
            damage.0 = attack::roll_dice(&mut rng, dice.0, count.0);
        }
    }
}

/// The blow is raised by its arm's damage percent (retail
/// `ResolveCharacterAttackDamage`).
fn damage_percent(
    mut swings: Query<(&mut Damage, &SwingOf), With<DiceCount>>,
    attacks: Query<&Striking>,
) {
    for (mut damage, swing_of) in &mut swings {
        if let Ok(striking) = attacks.get(swing_of.0) {
            damage.0 = (((100 + striking.0.percent) * damage.0 as i32 + 50) / 100).max(0) as u32;
        }
    }
}

/// A blow that got through deals at least one before reduction.
fn at_least_one(mut swings: Query<&mut Damage, With<DiceCount>>) {
    for mut damage in &mut swings {
        damage.0 = damage.0.max(1);
    }
}

/// The struck body's damage reduction cuts the blow (retail
/// `ApplyDamageReduction`, `ApplyCharacterDamageReduction`).
fn reduce(mut swings: Query<(&mut Damage, &At), With<DiceCount>>, reductions: Query<&Reduction>) {
    for (mut damage, at) in &mut swings {
        if let Ok(reduction) = reductions.get(at.0) {
            damage.0 = attack::reduce(damage.0 as i32, reduction.0);
        }
    }
}

/// A blow that went astray takes a fifth of its victim's health at most.
fn fumble_caps(mut swings: Query<(&mut Damage, &At), With<Fumbled>>, vitals: Query<&Vitals>) {
    for (mut damage, at) in &mut swings {
        if damage.0 > 0 {
            damage.0 = attack::fumble_cap(damage.0, vitals.get(at.0).unwrap().health.max);
        }
    }
}

/// A member a blow got through to practises Iron Skin as its reduction is
/// worked out.
fn iron_skin(
    swings: Query<&At, With<DiceCount>>,
    mut characters: Query<&mut Character>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    for at in &swings {
        if let Ok(mut character) = characters.get_mut(at.0)
            && character.skill_available(Skill::IronSkin)
            && character.practise(Skill::IronSkin, 1, &mut rng)
        {
            news.write(CombatNews::Improved { member: at.0, skill: Skill::IronSkin });
        }
    }
}

/// Training adds to a blow's chance to kill (retail
/// `ResolveCharacterAttack`), not when it went astray.
fn critical_strike(
    mut swings: Query<(&mut Chances, &SwingOf), Without<Fumbled>>,
    attacks: Query<(&By, &Striking)>,
    characters: Query<(&Character, &Vitals)>,
    mut rng: ResMut<GameRng>,
) {
    for (mut chances, swing_of) in &mut swings {
        let Some((by, striking)) = attacks.get(swing_of.0).ok() else { continue };
        let (Some(skill), Ok((character, vitals))) =
            (striking.0.critical_skill, characters.get(by.0))
        else {
            continue;
        };
        let skill = character.stats(&vitals.enchantments).skill(skill);
        let kill = &mut chances.0.chances[AttackEffect::Kill];
        *kill = (*kill + attack::critical_chance(&mut rng, skill)).min(100);
    }
}

/// A Monk's blow may knock out, by their profession.
fn knockout(
    mut swings: Query<(&mut Chances, &SwingOf), Without<Fumbled>>,
    attacks: Query<&By>,
    characters: Query<&Character>,
) {
    for (mut chances, swing_of) in &mut swings {
        let Some(by) = attacks.get(swing_of.0).ok() else { continue };
        let Ok(character) = characters.get(by.0) else { continue };
        if character.has_trait(Trait::Knockout) {
            let knockout = &mut chances.0.chances[AttackEffect::KnockOut];
            *knockout = (*knockout + character.scale_by_profession(5.0) as u32).min(100);
        }
    }
}

/// A blow that deals damage becomes the act that lands it: its damage,
/// then what its weapon does besides, at the attack's strength; a blow
/// gone astray is reported on its own.
fn land_blows(
    mut commands: Commands,
    swings: Query<(Entity, &At, &Damage, &Chances, Has<Fumbled>, &SwingOf), Without<Deed>>,
    attacks: Query<(&By, &Strength, &Striking)>,
    definitions: Res<Definitions>,
) {
    for (swing, at, damage, chances, fumbled, swing_of) in &swings {
        if damage.0 == 0 {
            continue;
        }
        let Some((by, &strength, striking)) = attacks.get(swing_of.0).ok() else { continue };
        let act = if fumbled { Act::Blow { by: by.0, at: at.0 } } else { Act::Swing };
        commands.entity(swing).insert((
            Deed::new(act),
            Uses(definitions.blow()),
            strength,
            WeaponEffects { effects: chances.0, by_monster: striking.0.word.is_some() },
            BlowDamage(damage.0),
        ));
        reach(&mut commands, swing, &[(at.0, None)]);
    }
}

/// A member's blow that dealt damage, not by a fumble, trains their
/// strikes: close blows Critical and Power Strike, ranged ones Eagle Eye,
/// and a thrower's Critical Strike too.
fn train_strikes(
    swings: Query<(&Damage, &SwingOf), (With<Pierces>, Without<Fumbled>)>,
    attacks: Query<(&By, &Striking)>,
    mut characters: Query<&mut Character>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    for (damage, swing_of) in &swings {
        let Some((by, striking)) = attacks.get(swing_of.0).ok() else { continue };
        let Ok(mut character) = characters.get_mut(by.0) else { continue };
        if damage.0 == 0 {
            continue;
        }
        let skills: &[Skill] = if !striking.0.ranged {
            &[Skill::CriticalStrike, Skill::PowerStrike]
        } else if character.has_trait(Trait::ThrownCriticals) {
            &[Skill::EagleEye, Skill::CriticalStrike]
        } else {
            &[Skill::EagleEye]
        };
        for &skill in skills {
            if character.skill_available(skill) && character.practise(skill, 1, &mut rng) {
                news.write(CombatNews::Improved { member: by.0, skill });
            }
        }
    }
}

/// A close blow that connected, even for nothing, sets off its victim's
/// Razor Cloak: a charge's roll, cut by the attacker's reduction, strikes
/// the attacker back as a blow of the victim's, reported on its own.
fn riposte(
    mut commands: Commands,
    swings: Query<(&At, &SwingOf), (With<Pierces>, Without<Done>)>,
    attacks: Query<(&By, &Striking)>,
    mut vitals: Query<&mut Vitals>,
    reductions: Query<&Reduction>,
    content: If<Res<Catalogs>>,
    definitions: Res<Definitions>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    for (at, swing_of) in &swings {
        let Some((by, striking)) = attacks.get(swing_of.0).ok() else { continue };
        if striking.0.reach >= Reach::Long {
            continue;
        }
        let Ok(mut defender) = vitals.get_mut(at.0) else { continue };
        let Some(charge) = defender.enchantments.spend_charge(SpellId::RAZOR_CLOAK) else {
            continue;
        };
        let strength = Strength { power: charge.power, boost: 0, against: 0 };
        let dice = content.spells[SpellId::RAZOR_CLOAK].dice;
        let rolled = crate::game::hits::prototypes::magnitude(&mut rng, dice, strength);
        let reduction = reductions.get(by.0).map_or(0, |reduction| reduction.0);
        let amount = attack::reduce(rolled as i32, reduction);
        if amount == 0 {
            news.write(CombatNews::Result { attacker: at.0, target: by.0, hits: 1, damage: 0 });
            continue;
        }
        let answer = commands
            .spawn((
                Deed::new(Act::Blow { by: at.0, at: by.0 }),
                Uses(definitions.blow()),
                Strength::default(),
                WeaponEffects { effects: AttackEffects::default(), by_monster: false },
                BlowDamage(amount),
            ))
            .id();
        reach(&mut commands, answer, &[(by.0, None)]);
    }
}

/// A blow counts toward its attack's summary, but a fumble's, which reports
/// itself; the swing is done.
fn count_hit(
    mut commands: Commands,
    swings: Query<(Entity, &At, &Damage, Has<Fumbled>, &SwingOf), (With<Pierces>, Without<Done>)>,
    mut attacks: Query<(&By, &mut Tally)>,
    mut news: MessageWriter<CombatNews>,
) {
    for (swing, at, damage, fumbled, swing_of) in &swings {
        let Ok((by, mut tally)) = attacks.get_mut(swing_of.0) else { continue };
        if !fumbled {
            tally.hits += 1;
        } else if damage.0 == 0 {
            news.write(CombatNews::Result { attacker: by.0, target: at.0, hits: 1, damage: 0 });
        }
        commands.entity(swing).insert(Done);
    }
}

/// A swing with an arm that tires costs stamina by its weight, twice in a
/// berserk fury, and twice more when diseased, scaled by the body's load.
fn tire_swingers(
    swings: Query<&SwingOf, Or<(Added<Done>, Added<Missed>)>>,
    attacks: Query<(&By, &Mode, &Striking, Has<Berserk>)>,
    mut bodies: Query<(&mut Vitals, &Burden)>,
    mut rng: ResMut<GameRng>,
) {
    for swing_of in &swings {
        let Some((by, mode, striking, berserk)) = attacks.get(swing_of.0).ok() else { continue };
        if !striking.0.tires {
            continue;
        }
        let Ok((mut vitals, burden)) = bodies.get_mut(by.0) else { continue };
        let diseased = vitals.conditions.has(Condition::Diseased);
        let cost = attack::swing_stamina(&mut rng, mode.0, striking.0.weight, berserk, diseased);
        let slowed = vitals.conditions.has(Condition::Slowed);
        let hasted = vitals.enchantments.has(SpellId::HASTE);
        vitals.tire(burden.load.scale_fatigue(cost, slowed, hasted));
    }
}

/// A ranged swing spends what it used (retail `ResolveCharacterAttack`'s
/// tail): the ammunition a launcher shot, a thrown weapon unless it never
/// runs out, or a gun's shot. A slot with nothing left to fire is done for
/// the round.
fn spend_ammunition(
    mut commands: Commands,
    swings: Query<&SwingOf, Or<(Added<Done>, Added<Missed>)>>,
    attacks: Query<(Entity, &By, &Striking)>,
    mut bodies: Query<(&mut Carried, &mut Turn)>,
    content: If<Res<Catalogs>>,
) {
    for swing_of in &swings {
        let Some((attack, by, striking)) = attacks.get(swing_of.0).ok() else { continue };
        if !striking.0.ranged || striking.0.weapon.is_none() {
            continue;
        }
        let Ok((mut carried, mut turn)) = bodies.get_mut(by.0) else { continue };
        let loaded = striking.0.loaded.is_some();
        let slot = if loaded { striking.0.slot.other() } else { striking.0.slot };
        let Some(stack) = carried.worn(slot.equip()) else { continue };
        let (id, definition) = (stack.id, &content.items[stack.item]);
        let thrown = definition.class == crate::content::items::EquipClass::ThrownWeapon;
        if !loaded && (definition.spell.is_some() || (thrown && definition.never_depletes)) {
            continue;
        }
        carried.consume_use(id, definition, definition.keeps_empty());
        if carried.worn(slot.equip()).is_none_or(|stack| stack.remaining_uses() == Some(0)) {
            commands.entity(attack).insert(Spent);
            turn.attacks[striking.0.slot] = 0;
        }
    }
}

/// The swing is over: it counts against the attack's swings; one that
/// struck nothing goes, one whose blow is landing stays to be tallied.
fn finish_swing(
    mut commands: Commands,
    swings: Query<(Entity, &SwingOf, Has<Deed>), Or<(Added<Done>, Added<Missed>)>>,
    mut attacks: Query<&mut SwingsLeft>,
) {
    for (swing, swing_of, lands) in &swings {
        if let Ok(mut left) = attacks.get_mut(swing_of.0) {
            left.0 -= 1;
            if left.0 == 0 {
                commands.entity(swing_of.0).insert(Spent);
            }
        }
        if !lands {
            commands.entity(swing).despawn();
        }
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
    mut attacks: Query<(Entity, &mut Tally, &By, Option<&AimsAt>), (With<Spent>, Without<Swings>)>,
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
