//! Weapon attacks in a fight (`docs/rules/attack.md`). An attack is an
//! entity: who makes it, at whom, with which arm and how, and what its
//! swings come to. Each swing is an entity of its attack, and each fact of
//! it is a component added by a small observer that reads only what that
//! fact comes from: its [`Aim`], its [`Fate`], whom it [`Strikes`] once a
//! guard has had its say, whether it gets [`Through`], and the blow, a
//! [`Deed`]. Bevy carries each out before the next is begun, so every swing
//! sees what the one before did.
//!
//! Retail's voices on a miss, a kill or a fumble draw from the same random
//! numbers as the rules; here they draw nothing.

use bevy::prelude::*;

use super::approach::{CombatMove, MoveGoal};
use super::attack::{self, Surprise};
use super::effort::{Effort, Exerted, Tires};
use super::monster::close::{self, MoveBudget};
use super::turn::{Fights, Guards, Interceptions, Ordered, Spotted, Turn};
use super::{Announcement, Battle, CombatNews, Combatants, Pause, Round};
use crate::content::characters::{Attribute, Skill};
use crate::content::items::{AttackEffects, AttackMode};
use crate::game::actors::arm::{
    Ambushes, Arms, BlowEffects, Critical, DamagePercent, FatiguePierces, Furthest, Grip,
    GuardReach, Hand, Heft, LandsOnParty, Loaded, Modern, Modes, Rating, Reaches, Rolls, Slays,
    SoundClass, SureThrow, Swings, Sword, Tempo, Tires as TiresToSwing, Trains, Word,
};
use crate::game::actors::{
    Abilities, Against, Agility, AimBonus, Armour, Attributes, Backstab, BlindFighting, Blinded,
    Block, CanFight, CanGuard, Fatigue, HitPlaces, KnockOut, Laying, Level, MissileShield,
    Modifiers, Nature, Places, Pressing, Reduction, Resistances, Skills, Unaware, Unseated,
    reaches,
};
use crate::game::characters::{Calling, Condition, Conditions, Practise, Vitals, skill_available};
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::hits::definitions::{Blow, Rows};
use crate::game::hits::effects;
use crate::game::hits::{Deed, Effect, Landed};
use crate::game::inventory::WeaponSlot;
use crate::game::items::{Burden, Items, Missile, Spend, Weapon};
use crate::game::magic::Strength;
use crate::game::npcs::Embodies;
use crate::game::party::formation::Formation;
use crate::game::party::roster::{PartyMember, PartyRoster};
use crate::game::rng::GameRng;
use crate::game::world::hostility::{self, Provocation, Stance, Standing};
use crate::game::world::navigation::Walker;
use crate::game::world::obstacles::WorldObstacles;

/// An actor that fights makes its attack ([`strike`]).
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Strike {
    #[event_target]
    pub actor: Entity,
    pub fights: Fights,
}

/// An attack: who makes it.
#[derive(Component, Clone, Copy, Debug)]
pub struct AttackBy(pub Entity);

/// At whom an attack is made.
#[derive(Component, Clone, Copy, Debug)]
pub struct AttackAt(pub Entity);

/// The arm an attack is made with, and how.
#[derive(Component, Clone, Copy, Debug)]
pub struct AttackWith {
    pub arm: Entity,
    pub mode: AttackMode,
}

/// A Berserk order's fury, which costs twice the stamina.
#[derive(Component, Clone, Copy, Debug)]
pub struct Fury;

/// How an attack catches its target; a blow gone astray or taken by a guard
/// surprises nobody from then on.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Caught(pub Surprise);

/// The swings an attack makes, and those made.
#[derive(Component, Clone, Copy, Debug)]
pub struct SwingCount {
    pub most: u32,
    pub made: u32,
}

/// What an attack's swings came to.
#[derive(Component, Clone, Debug, Default)]
pub struct Tally {
    pub hits: u32,
    pub damage: u32,
    pub effects: Vec<Effect>,
}

/// A swing of an attack.
#[derive(Component, Clone, Copy, Debug)]
pub struct SwingOf(pub Entity);

/// A swing's aim, to be at least the roll; and the figure the fumble chance
/// is worked from (the base with the blind rule and difficulty applied, but
/// not the target's armour or agility).
#[derive(Component, Clone, Copy, Debug)]
pub struct Aim {
    pub aim: i32,
    pub fumble_odds: i32,
}

/// What became of a swing.
#[derive(Component, Clone, Copy, Debug)]
pub enum Fate {
    Missed,
    /// It reaches its target with `roll`. It `lands` when the aim is at
    /// least the roll (or it is sure, or a shot that struck), and is `sure`
    /// through armour.
    Reached {
        roll: u32,
        lands: bool,
        sure: bool,
    },
    /// It goes astray onto a bystander, with a fresh roll, and lands.
    Astray {
        victim: Entity,
        roll: u32,
    },
}

/// Whom a swing that reached strikes, once a guard has stepped in or not.
#[derive(Component, Clone, Copy, Debug)]
pub struct Strikes(pub Entity);

/// A swing that gets through armour: where it lands, by its order among the
/// places of the one struck.
#[derive(Component, Clone, Copy, Debug)]
pub struct Through(pub usize);

/// A swing has been settled.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct Swung {
    #[event_target]
    swing: Entity,
}

/// An attack swings again, if it may.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct SwingsAgain {
    #[event_target]
    attack: Entity,
}

/// An attack is over.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct AttackEnds {
    #[event_target]
    attack: Entity,
}

/// A blow of an attack: one gone astray is told of on its own.
#[derive(Component, Clone, Copy, Debug)]
pub struct Blown {
    attack: Entity,
    victim: Entity,
    astray: bool,
}

/// Razor Cloak's answer to a close blow, told of on its own.
#[derive(Component, Clone, Copy, Debug)]
pub struct Answer {
    by: Entity,
    at: Entity,
}

/// One who trained practises a skill they may use.
fn practise(
    actor: Entity,
    skill: Skill,
    trained: &Query<(&Calling, &Abilities)>,
    commands: &mut Commands,
) {
    let may =
        |(calling, abilities): (&Calling, &Abilities)| skill_available(calling, abilities, skill);
    if trained.get(actor).is_ok_and(may) {
        commands.trigger(Practise { member: actor, skill, usage: 1 });
    }
}

/// Whether one is on the move, its guard down to blows that close in: a
/// member while the party runs, anyone else walking this round.
fn moving(member: bool, walking: bool, round: &Round) -> bool {
    if member { round.pace == Some(super::PartyPace::Run) } else { walking }
}

/// An attack being chosen, by an actor that fights as its turn says.
#[derive(Component, Clone, Copy, Debug)]
pub struct Choosing(pub Fights);

/// Those an attack being chosen looks for: the living combatants on the side
/// it seeks, each with whether its eye misses them this time.
#[derive(Component, Clone, Debug)]
pub struct Sought(pub Vec<(Entity, bool)>);

/// A way to strike: an arm, one of its modes, someone it reaches.
#[derive(Clone, Copy, Debug)]
pub struct Way {
    arm: Entity,
    mode: AttackMode,
    target: Entity,
}

/// The ways an attack being chosen could strike, with the hand they are in.
#[derive(Component, Clone, Debug)]
pub struct Ways {
    slot: WeaponSlot,
    ways: Vec<Way>,
}

/// An attack being chosen found nobody to strike.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct Nobody {
    #[event_target]
    attack: Entity,
}

/// An actor that fights chooses whom to strike and how, or what it does
/// instead (section 2). Attacking holds the fight for the action's pause.
pub(super) fn strike(
    strike: On<Strike>,
    mut commands: Commands,
    mut pause: Single<&mut Pause, With<Battle>>,
) {
    pause.0 = Timer::from_seconds(super::ACTION_SECONDS, TimerMode::Once);
    commands.spawn((AttackBy(strike.actor), Choosing(strike.fights)));
}

/// Whom it looks for among the living combatants: foes, or friends when
/// insanity has turned it on its own this round. Anyone not under orders
/// misses a party member with chance `Stealth * 75 / 100` percent.
#[allow(clippy::type_complexity)]
fn seek(
    chosen: On<Add<Choosing>>,
    mut commands: Commands,
    attacks: Query<&AttackBy>,
    actors: Query<(&Turn, &Standing, Has<Ordered>)>,
    candidates: Query<(&Vitals, &Standing, &Skills)>,
    fight: Single<Option<&Combatants>, With<Battle>>,
    roster: Single<&PartyRoster>,
    mut rng: ResMut<GameRng>,
) {
    let attack = chosen.entity;
    let Ok(&AttackBy(actor)) = attacks.get(attack) else { return };
    let Ok((turn, standing, ordered)) = actors.get(actor) else { return };
    let seeks = if turn.berserk { Stance::Friendly } else { Stance::Hostile };
    let mut sought = Vec::new();
    for &other in fight.map_or(&[][..], Combatants::entities).iter().filter(|&&o| o != actor) {
        let Ok((their, theirs, skills)) = candidates.get(other) else { continue };
        if their.is_dead() || hostility::between(*standing, *theirs) != seeks {
            continue;
        }
        let stealth = skills[Skill::Stealth];
        let lost = !ordered && roster.contains(&other) && rng.below(100) < stealth * 75 / 100;
        sought.push((other, lost));
    }
    commands.entity(attack).insert(Sought(sought));
}

/// The ways it could strike: with an arm of the hand due if it has attacks
/// left and an arm that reaches someone it looks for, otherwise the other.
/// A member under orders never strikes an NPC and needs no sight of whom
/// they strike; anyone else needs a clear line. A member its eye missed, who
/// it could otherwise have struck, learns Stealth from it half the time.
#[allow(clippy::type_complexity)]
fn find_ways(
    sought: On<Add<Sought>>,
    mut commands: Commands,
    attacks: Query<(&AttackBy, &Sought)>,
    actors: Query<(&Turn, &Arms, Has<Ordered>)>,
    arms: Query<(&Hand, &Reaches, &Modes)>,
    npcs: Query<(), With<Embodies>>,
    trained: Query<(&Calling, &Abilities)>,
    places: Places,
    formation: Single<&Formation>,
    obstacles: WorldObstacles,
    mut rng: ResMut<GameRng>,
) {
    let attack = sought.entity;
    let Ok((&AttackBy(actor), Sought(sought))) = attacks.get(attack) else { return };
    let Ok((turn, held, ordered)) = actors.get(actor) else { return };
    let at = places.of(actor);
    let mut evaded = Vec::new();
    let mut found = None;
    for slot in [turn.due, turn.due.other()] {
        if turn.attacks[slot] == 0 {
            continue;
        }
        let mut ways = Vec::new();
        for &arm in held.entities() {
            let Ok((hand, reach, modes)) = arms.get(arm) else { continue };
            if hand.0 != slot {
                continue;
            }
            for &(other, lost) in sought {
                let there = places.of(other);
                if (ordered && npcs.contains(other))
                    || !reaches(&formation, (actor, &at), (other, &there), reach.0)
                    || (!ordered && !obstacles.clear(at.eye, there.centre))
                {
                    continue;
                }
                if lost {
                    if !evaded.contains(&other) {
                        evaded.push(other);
                    }
                    continue;
                }
                ways.extend(modes.0.iter().map(|mode| Way { arm, mode, target: other }));
            }
        }
        if !ways.is_empty() {
            found = Some(Ways { slot, ways });
            break;
        }
    }
    for member in evaded.into_iter().filter(|_| rng.below(2) == 0) {
        practise(member, Skill::Stealth, &trained, &mut commands);
    }
    match found {
        Some(ways) => {
            commands.entity(attack).insert(ways);
        }
        None => commands.trigger(Nobody { attack }),
    }
}

/// The way it strikes. A member under orders strikes whoever is in the
/// lowest condition (a sound one before a hurt one), then one in reach of
/// the party (armed, and on its rear side, or on a side nobody holds while
/// its centre is held), then the nearest, ties keeping the fight's order
/// (retail `CompareMonsterTargetCandidates`), in the berserk mode under a
/// Berserk order and otherwise in one of the arm's modes. For anyone else
/// every way to strike has an equal chance, and so has waiting, for one that
/// may hold back (retail `ChooseMonsterTarget`). The hand's attacks go down
/// by one.
#[allow(clippy::type_complexity)]
fn pick(
    found: On<Add<Ways>>,
    mut commands: Commands,
    attacks: Query<(&AttackBy, &Choosing, &Ways)>,
    mut actors: Query<(&mut Turn, Has<Ordered>)>,
    modes: Query<&Modes>,
    candidates: Query<(&Conditions, &Furthest)>,
    places: Places,
    formation: Single<&Formation>,
    mut rng: ResMut<GameRng>,
) {
    use crate::game::party::formation::Row;
    let attack = found.entity;
    let Ok((&AttackBy(actor), &Choosing(fights), Ways { slot, ways })) = attacks.get(attack) else {
        return;
    };
    let Ok((mut turn, ordered)) = actors.get_mut(actor) else { return };
    let at = places.of(actor);
    let picked = if ordered {
        let key = |way: &Way| {
            let there = places.of(way.target);
            let (worst, armed) = candidates
                .get(way.target)
                .map_or((None, false), |(ailing, reach)| (ailing.worst(), reach.0.is_some()));
            let side = at.quadrant(there.feet);
            let reached = armed
                && (side == Row::Rear
                    || (!formation.occupied(side) && formation.occupied(Row::Center)));
            (worst, !reached, at.gap(&there))
        };
        let best = ways
            .iter()
            .min_by(|a, b| {
                let (a, b) = (key(a), key(b));
                a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.total_cmp(&b.2))
            })
            .copied()
            .unwrap();
        let mode = if fights.berserk {
            AttackMode::Berserk
        } else {
            attack::choose_mode(&mut rng, modes.get(best.arm).map_or(Default::default(), |m| m.0))
        };
        Some(Way { mode, ..best })
    } else {
        let waits = usize::from(fights.waits);
        let pick = rng.below((ways.len() + waits) as u32) as usize;
        (pick >= waits).then(|| ways[pick - waits])
    };
    let Some(Way { arm, mode, target }) = picked else {
        turn.waited = true;
        turn.end();
        commands.trigger(Exerted { actor, effort: Effort::Wait });
        commands.entity(attack).despawn();
        return;
    };
    turn.due = *slot;
    turn.attacks[*slot] -= 1;
    let mut made = commands.entity(attack);
    made.insert((AttackWith { arm, mode }, Caught::default(), Tally::default()));
    if fights.berserk {
        made.insert(Fury);
    }
    made.insert(AttackAt(target));
}

/// With nobody to strike, one that walks and has not moved closes in: on
/// the party if it is hostile to it, on the nearest it looks for if it
/// stands with the party; charging when hurt; and acts again at twice the
/// moment now, at most 100 (retail `SetUpMonsterTurn`). Anything else
/// stands.
#[allow(clippy::type_complexity)]
fn close_in(
    nobody: On<Nobody>,
    mut commands: Commands,
    attacks: Query<(&AttackBy, &Sought)>,
    mut actors: Query<(
        (&mut Turn, Option<&Walker>),
        ((&Nature, &Modifiers), &Furthest, &Standing, &Vitals, &Conditions),
    )>,
    living: Query<(), With<Vitals>>,
    places: Places,
    round: Single<&Round, With<Battle>>,
    roster: Single<&PartyRoster>,
    mut news: MessageWriter<CombatNews>,
) {
    let attack = nobody.attack;
    commands.entity(attack).despawn();
    let Ok((&AttackBy(actor), Sought(sought))) = attacks.get(attack) else { return };
    let Ok(((mut turn, walker), ((nature, laid), furthest, standing, vitals, conditions))) =
        actors.get_mut(actor)
    else {
        return;
    };
    let Some(walker) = walker.filter(|_| nature.stride.is_some() && !turn.moved) else {
        return turn.end();
    };
    let at = places.of(actor);
    let walks = !vitals.is_dead() && walker.speed > 0.0;
    let reach = furthest.0.unwrap_or(crate::content::species::Reach::Touch);
    let goal = match standing.stance {
        Stance::Friendly => sought
            .iter()
            .map(|&(other, _)| other)
            .min_by(|&a, &b| {
                let to = |other| at.feet.distance_squared(places.of(other).feet);
                to(a).total_cmp(&to(b))
            })
            .filter(|_| walker.speed > 0.0)
            .map(MoveGoal::Attack),
        Stance::Hostile => roster
            .iter()
            .find(|&member| living.contains(member))
            .filter(|&member| close::needs_approach(walks, reach, at.gap(&places.of(member))))
            .map(|_| MoveGoal::Party),
        Stance::Neutral => None,
    };
    let Some(goal) = goal else { return turn.end() };
    let health = (vitals.health.current, vitals.health.max);
    let stride = (nature.stride, laid.laid.stride);
    let budget = MoveBudget::of(stride, health, conditions.has(Condition::Slowed));
    turn.moved = true;
    turn.when = Some((round.moment * 2).min(100));
    commands.entity(actor).insert(CombatMove { budget: budget.metres, moved: 0.0, goal });
    news.write(CombatNews::Moves { monster: actor, charge: budget.charge });
    let effort = if budget.charge { Effort::Charge } else { Effort::Approach };
    commands.trigger(Exerted { actor, effort });
}

/// An attack is opened (sections 2 and 3): its swings are counted
/// (`below(swings) + 1`, the weapon's extra swings added, at least one; a
/// readied Lightning Strike adds 3 or 4 with a sword in the main hand when
/// not slowed, and is used up by the main hand's first attack); it may catch
/// its target unaware; a member provokes whom they strike; it is announced;
/// and it swings.
#[allow(clippy::type_complexity)]
fn open(
    opened: On<Add<AttackAt>>,
    mut commands: Commands,
    attacks: Query<(&AttackBy, &AttackAt, &AttackWith)>,
    mut attackers: Query<(&mut Turn, &Conditions, Has<Ordered>)>,
    arms: Query<(
        &Hand,
        &Swings,
        &Reaches,
        Has<Sword>,
        Has<Ambushes>,
        Option<&Word>,
        Option<&SoundClass>,
    )>,
    mut targets: Query<(Option<&mut Spotted>, Option<&Ordered>, &Vitals, &Conditions, &Attributes)>,
    places: Places,
    mut fight: Single<&mut Round, With<Battle>>,
    mut formation: Single<&mut Formation>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    let attack = opened.entity;
    let Ok((&AttackBy(actor), &AttackAt(target), &AttackWith { arm, mode })) = attacks.get(attack)
    else {
        return;
    };
    let (
        Ok((mut turn, conditions, ordered)),
        Ok((hand, swings, reach, sword, ambushes, word, sound)),
    ) = (attackers.get_mut(actor), arms.get(arm))
    else {
        return;
    };
    let mut count = (rng.below(swings.most) as i32 + 1 + swings.extra).max(1) as u32;
    let readied = turn.lightning.filter(|_| hand.0 == WeaponSlot::Primary);
    if let Some(level) = readied
        && sword
        && !conditions.has(Condition::Slowed)
    {
        count += if rng.below(11) + u32::from(level) < 20 { 3 } else { 4 };
    }
    if readied.is_some() {
        turn.lightning = None;
    }
    fight.attacked(true);
    let provokes = ordered && !conditions.has(Condition::Turncoat);

    // Catching the target unaware (retail `IsPartyLookingAwayFrom`): a seated
    // member faces the way the formation says, anyone else its own front;
    // only a touch or short blow can; a member keeping watch (defending,
    // alive, not spent, nothing as bad as Webbed, not blind) who notices
    // turns to face it instead; anyone else is caught if the attacker is
    // exactly across from the way they face. An arm that ambushes strikes
    // naturally whether or not the target faces it.
    let attacker_on = places.of(target).quadrant(places.of(actor).feet);
    let facing = formation.facing(target).unwrap_or(crate::game::party::formation::Row::Front);
    let mut surprise = Surprise { unaware: false, natural: ambushes };
    if let Ok((spotted, orders, their, ailing, a)) = targets.get_mut(target)
        && reach.0 <= crate::content::species::Reach::Short
        && facing != attacker_on
    {
        let watching = orders.is_some_and(Ordered::defends)
            && !their.is_dead()
            && their.stamina.current > 0
            && ailing.below(Condition::Webbed)
            && !ailing.has(Condition::Blind);
        let tries = spotted.as_ref().map(|spotted| spotted.0);
        let noticed = match (tries, watching) {
            (Some(tries), true) => {
                if let Some(mut spotted) = spotted {
                    spotted.0 = tries + 1;
                }
                attack::notices(&mut rng, a[Attribute::Senses], tries)
            }
            _ => false,
        };
        if noticed {
            formation.turn(target, attacker_on);
        } else {
            surprise.unaware = facing.opposite() == Some(attacker_on);
        }
    }
    if provokes {
        commands.trigger(Provocation { monster: target });
    }
    let how = match word {
        Some(&Word(word, named)) => Announcement::Monster { word, mode, named },
        None => Announcement::Mode(mode),
    };
    news.write(CombatNews::Attack { attacker: actor, target, how, swings: count, surprise });
    // A member's weapon is heard swung (retail `MakePCAttackSound`).
    if word.is_none() {
        news.write(CombatNews::Swung { sound_class: sound.map(|sound| sound.0) });
    }
    commands.entity(attack).insert((Caught(surprise), SwingCount { most: count, made: 0 }));
    commands.trigger(SwingsAgain { attack });
}

/// An attack swings again while it has swings left, its attacker can fight
/// and its target lives; otherwise it is over.
fn swing_again(
    again: On<SwingsAgain>,
    mut commands: Commands,
    attacks: Query<(&AttackBy, &AttackAt, &SwingCount)>,
    able: Query<(), With<CanFight>>,
    vitals: Query<&Vitals>,
) {
    let attack = again.attack;
    let Ok((&AttackBy(actor), &AttackAt(target), count)) = attacks.get(attack) else { return };
    let alive = vitals.get(target).is_ok_and(|vitals| !vitals.is_dead());
    if count.made < count.most && able.contains(actor) && alive {
        commands.spawn(SwingOf(attack));
    } else {
        commands.trigger(AttackEnds { attack });
    }
}

/// A swing's aim (section 4):
///
/// ```text
/// base = arm rating + aim bonus + mode's to-hit + surprise - fatigue
/// aim  = base - 5 * (target's armour - what the blow gets past) + agility
/// ```
///
/// Fatigue tells half as much on a modern arm. What a blow gets past (retail
/// `GetTargetArmorClassModifier`) is what being caught unaware costs one so
/// caught, and 4 of one on the move that keeps its place unless the blow
/// keeps its distance. A blind attacker's aim past 10 is lost, or kept by a
/// share by level for one who fights blind; then the attacker's difficulty.
#[allow(clippy::type_complexity)]
fn aim(
    swung: On<Add<SwingOf>>,
    mut commands: Commands,
    swings: Query<&SwingOf>,
    attacks: Query<(&AttackBy, &AttackAt, &AttackWith, &Caught)>,
    arms: Query<(&Rating, Has<Modern>)>,
    attackers: Query<(
        &AimBonus,
        &Fatigue,
        &Agility,
        &Standing,
        Option<&BlindFighting>,
        Has<Blinded>,
    )>,
    targets: Query<(&Armour, &Unaware, Has<Unseated>, &Agility, Has<PartyMember>, Has<CombatMove>)>,
    round: Single<&Round, With<Battle>>,
    difficulty: Res<Difficulty>,
) {
    let swing = swung.entity;
    let Ok(&SwingOf(attack)) = swings.get(swing) else { return };
    let Ok((&AttackBy(actor), &AttackAt(target), with, &Caught(surprise))) = attacks.get(attack)
    else {
        return;
    };
    let (Ok((rating, modern)), Ok(attacker), Ok(aimed)) =
        (arms.get(with.arm), attackers.get(actor), targets.get(target))
    else {
        return;
    };
    let (bonus, fatigue, agility, standing, own, blind) = attacker;
    let (armour, unaware, unseated, their_agility, member, walking) = aimed;
    let tired = if modern { fatigue.0 / 2 } else { fatigue.0 };
    let surprised = 10 * i32::from(surprise.unaware) + 10 * i32::from(surprise.natural);
    let base = rating.0 + bonus.0 + attack::mode_to_hit(with.mode) + surprised - tired;
    let in_place = moving(member, walking, &round) && !unseated;
    let past = if surprise.any() { unaware.0 } else { 0 }
        + if in_place && !attack::keeps_distance(with.mode) { 4 } else { 0 };
    let aim =
        base - 5 * (armour.total - past) + attack::agility_difference(agility.0, their_agility.0);
    let capped = |figure| {
        let blinded = attack::blind_cap(figure, blind, own.map(|fights| fights.0));
        difficulty.scale(blinded, standing.stance)
    };
    commands.entity(swing).insert(Aim { aim: capped(aim), fumble_odds: capped(base) });
}

/// What becomes of a swing (section 4): a close one rolls against its aim
/// and may go astray onto one of the living in reach of its arm other than
/// its target (friends, foes and the attacker alike), by odds cubic in what
/// the aim falls short of 100; a shot is scattered about its target and
/// strikes or misses. Then the swing is settled.
#[allow(clippy::type_complexity)]
fn fate(
    aimed: On<Add<Aim>>,
    mut commands: Commands,
    swings: Query<(&SwingOf, &Aim)>,
    attacks: Query<(&AttackBy, &AttackAt, &AttackWith)>,
    mut caught: Query<&mut Caught>,
    arms: Query<(&Reaches, Has<SureThrow>)>,
    blinded: Query<(), With<Blinded>>,
    vitals: Query<&Vitals>,
    places: Places,
    fight: Single<Option<&Combatants>, With<Battle>>,
    formation: Single<&Formation>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    let swing = aimed.entity;
    let Ok((&SwingOf(attack), aim)) = swings.get(swing) else { return };
    let Ok((&AttackBy(actor), &AttackAt(target), with)) = attacks.get(attack) else { return };
    let Ok((reach, sure_throw)) = arms.get(with.arm) else { return };
    let combatants = fight.map_or(&[][..], Combatants::entities);
    let (from, at) = (places.of(actor), places.of(target));
    let fate = if attack::is_missile(reach.0) {
        let point =
            attack::scatter(&mut rng, from.eye, at.centre, aim.aim, blinded.contains(actor));
        if attack::strikes(at.centre, point, at.radius, at.half_height) {
            Fate::Reached { roll: rng.below(100) + 1, lands: true, sure: sure_throw }
        } else {
            Fate::Missed
        }
    } else {
        let astray: Vec<Entity> = combatants
            .iter()
            .copied()
            .filter(|&other| other != target)
            .filter(|&other| vitals.get(other).is_ok_and(|vitals| !vitals.is_dead()))
            .filter(|&other| {
                other == actor
                    || reaches(&formation, (actor, &from), (other, &places.of(other)), reach.0)
            })
            .collect();
        let roll = rng.below(100) + 1;
        let odds = attack::fumble_chance(astray.len(), aim.fumble_odds);
        match attack::verdict(roll, astray.len(), odds) {
            attack::Verdict::Misses => Fate::Missed,
            attack::Verdict::Reaches { sure } => {
                Fate::Reached { roll, lands: sure || aim.aim >= roll as i32, sure }
            }
            attack::Verdict::Fumbles => {
                let victim = astray[rng.below(astray.len() as u32) as usize];
                if let Ok(mut caught) = caught.get_mut(attack) {
                    caught.0 = Surprise::default();
                }
                news.write(CombatNews::Fumble { attacker: actor, victim });
                Fate::Astray { victim, roll: rng.below(100) + 1 }
            }
        }
    };
    commands.entity(swing).insert(fate);
    commands.trigger(Swung { swing });
}

/// A guard of the one a swing reached may step in to take the blow (section
/// 4): each able guard with the ward within reach of its first arm and
/// interceptions left this round steps in with its pressing against the
/// attacker's, `guard - attacker + 50` held to 5 to 95, the attacker's raised
/// by 10 for an unaware blow and 10 for a natural one; one of those at random
/// takes the blow, tired `below(3) + 2` by it. Whom the swing strikes is then
/// known.
#[allow(clippy::type_complexity)]
fn step_in(
    fated: On<Add<Fate>>,
    mut commands: Commands,
    swings: Query<(&SwingOf, &Fate)>,
    attacks: Query<(&AttackBy, &AttackAt)>,
    mut caught: Query<&mut Caught>,
    pressing: Query<&Pressing>,
    wards: Query<&Guards>,
    mut guards: Query<(Has<CanGuard>, &GuardReach, &mut Interceptions)>,
    places: Places,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    let swing = fated.entity;
    let Ok((&SwingOf(attack), fate)) = swings.get(swing) else { return };
    let Ok((&AttackBy(actor), &AttackAt(target))) = attacks.get(attack) else { return };
    let ward = match *fate {
        Fate::Missed => return,
        Fate::Reached { .. } => target,
        Fate::Astray { victim, .. } => victim,
    };
    let Ok(mut surprise) = caught.get_mut(attack) else { return };
    let theirs = pressing.get(actor).map_or(0, |pressing| pressing.0);
    let at = places.of(ward);
    let stepping: Vec<Entity> = wards
        .get(ward)
        .map(|ward| ward.entities().to_vec())
        .unwrap_or_default()
        .into_iter()
        .filter(|&guard| {
            guards.get(guard).is_ok_and(|(able, reach, made)| {
                let pressed = theirs
                    + 10 * i32::from(surprise.0.unaware)
                    + 10 * i32::from(surprise.0.natural);
                let own = pressing.get(guard).map_or(0, |pressing| pressing.0);
                let chance = (own - pressed + 50).clamp(5, 95) as u32;
                able && reach.0.is_some_and(|reach| places.of(guard).gap(&at) <= reach.metres())
                    && made.taken < made.allowed
                    && rng.below(100) < chance
            })
        })
        .collect();
    let mut victim = ward;
    if !stepping.is_empty() {
        let guard = stepping[rng.below(stepping.len() as u32) as usize];
        if let Ok((.., mut made)) = guards.get_mut(guard) {
            made.take();
        }
        commands.trigger(Tires { actor: guard, stamina: rng.below(3) + 2 });
        surprise.0 = Surprise::default();
        news.write(CombatNews::Intercepts { guard, ward });
        victim = guard;
    }
    commands.entity(swing).insert(Strikes(victim));
}

/// A near miss is blocked, and heard, where a miss is not: by its target's
/// block, when it saw the blow coming or was caught unaware.
fn block(
    struck: On<Add<Strikes>>,
    swings: Query<(&SwingOf, &Fate, &Aim, &Strikes)>,
    attacks: Query<(&AttackWith, &Caught)>,
    blocks: Query<&Block>,
    sounds: Query<&SoundClass>,
    mut news: MessageWriter<CombatNews>,
) {
    let Ok((&SwingOf(attack), &fate, aim, &Strikes(victim))) = swings.get(struck.entity) else {
        return;
    };
    let Fate::Reached { roll, lands: false, .. } = fate else { return };
    let (Ok((with, caught)), Ok(block)) = (attacks.get(attack), blocks.get(victim)) else { return };
    let within = if caught.0.any() { block.unaware } else { block.aware };
    if roll as i32 - aim.aim <= within {
        let sound_class = sounds.get(with.arm).ok().map(|sound| sound.0);
        news.write(CombatNews::Blocked { victim, sound_class });
    }
}

/// A swing that lands (section 4) lands somewhere on the one struck, by the
/// arm's own chances for a member's parts where it has them; the mode that
/// pierces is a launcher's missile's, drawn from its modes, or the swing's.
/// It gets through when it is not turned aside by a Missile Shield and is
/// sure, or its penetration is at least the roll:
///
/// ```text
/// penetration = scaled( mode's penetration + rating + aim bonus
///                       - fatigue (only if it tells on penetration)
///                       - 5 * armour at the place + agility )
/// ```
///
/// where the armour at the place is less what it has against the mode, and
/// what the blow gets past counts twice in it (retail
/// `TargetArmorClassAtLocation`). A swing gone astray that does not get
/// through is told of on its own.
#[allow(clippy::type_complexity)]
fn land(
    struck: On<Add<Strikes>>,
    mut commands: Commands,
    swings: Query<(&SwingOf, &Fate, &Strikes)>,
    mut attacks: Query<(&AttackBy, &AttackWith, &Caught, &mut Tally)>,
    arms: Query<(&Rating, Has<FatiguePierces>, Option<&Loaded>, Option<&LandsOnParty>)>,
    attackers: Query<(&AimBonus, &Fatigue, &Agility, &Standing)>,
    victims: Query<(
        (&HitPlaces, &Armour, &Against, &MissileShield),
        (&Unaware, Has<Unseated>, &Agility, Has<PartyMember>, Has<CombatMove>),
    )>,
    round: Single<&Round, With<Battle>>,
    difficulty: Res<Difficulty>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    let swing = struck.entity;
    let Ok((&SwingOf(attack), &fate, &Strikes(victim))) = swings.get(swing) else { return };
    let (roll, sure, astray) = match fate {
        Fate::Reached { roll, lands: true, sure } => (roll, sure, false),
        Fate::Astray { roll, .. } => (roll, false, true),
        _ => return,
    };
    let Ok((&AttackBy(actor), with, caught, mut tally)) = attacks.get_mut(attack) else { return };
    tally.hits += u32::from(!astray);
    let (Ok((rating, pierces_tired, loaded, lands_on_party)), Ok(attacker), Ok(there)) =
        (arms.get(with.arm), attackers.get(actor), victims.get(victim))
    else {
        return;
    };
    let (bonus, fatigue, agility, standing) = attacker;
    let ((places, armour, against, shield), (unaware, unseated, their_agility, member, walking)) =
        there;
    let place = attack::lands_at(rng.below(100), places, lands_on_party.map(|lands| &lands.0));
    let by = attack::piercing_mode(&mut rng, with.mode, loaded.map(|loaded| loaded.0));
    let in_place = moving(member, walking, &round) && !unseated;
    let past = if caught.0.any() { unaware.0 } else { 0 }
        + if in_place && !attack::keeps_distance(by) { 4 } else { 0 };
    let at_place = armour.at(&places.places[place.0]) + against.0[by] - 2 * past;
    let tired = if pierces_tired { fatigue.0 } else { 0 };
    let penetration = difficulty.scale(
        rating.0 + bonus.0 + attack::mode_penetration(by) - tired - 5 * at_place
            + attack::agility_difference(agility.0, their_agility.0),
        standing.stance,
    );
    let aside = attack::turned_aside(with.mode, shield.0, rng.below(100));
    if !aside && (sure || penetration >= roll as i32) {
        commands.entity(swing).insert(Through(place.0));
    } else if astray {
        news.write(CombatNews::Result { attacker: actor, target: victim, hits: 1, damage: 0 });
    }
}

/// The blow of a swing that got through (section 5): heard where it lands,
/// and training the struck one's Iron Skin. Its dice are rolled twice for a
/// close blow that is unaware or at one on the move, with the backstab's
/// extra dice for a natural close blow, one more in a berserk mode, one less
/// (if more than one) at one defending a blow not unaware, and one more for
/// each kind the arm slays that the one struck is; raised by the arm's
/// percent, at least one, and cut by the one struck's reduction. One gone
/// astray takes at most a fifth of its victim's health, at least one, and
/// trains nothing; otherwise the attacker's training raises the Kill chance
/// (a point per 25 of its critical skill, one more with chance `(skill mod
/// 25) / 25`) and the Knock Out chance (5 scaled by the level of one with the
/// trait), each held to 100. A blow that does damage is a deed of its own,
/// and trains the arm's skills.
#[allow(clippy::type_complexity)]
fn blow(
    through: On<Add<Through>>,
    mut commands: Commands,
    swings: Query<(&SwingOf, &Fate, &Strikes, &Through)>,
    attacks: Query<(&AttackBy, &AttackWith, &Caught)>,
    arms: Query<(
        (&Rolls, &DamagePercent, &BlowEffects, &Slays),
        (Option<&Critical>, Option<&Trains>, Option<&Word>, Option<&SoundClass>),
    )>,
    attackers: Query<(&Backstab, Option<&KnockOut>, &Level)>,
    victims: Query<(
        &Reduction,
        &Vitals,
        &Resistances,
        Option<&Ordered>,
        Has<PartyMember>,
        Has<CombatMove>,
    )>,
    trained: Query<(&Calling, &Abilities)>,
    round: Single<&Round, With<Battle>>,
    content: Res<Catalogs>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    use crate::content::items::AttackEffect;
    let Ok((&SwingOf(attack), fate, &Strikes(victim), &Through(place))) =
        swings.get(through.entity)
    else {
        return;
    };
    let astray = matches!(fate, Fate::Astray { .. });
    let Ok((&AttackBy(actor), with, caught)) = attacks.get(attack) else { return };
    let (Ok(arm), Ok((backstab, knock_out, level)), Ok(struck)) =
        (arms.get(with.arm), attackers.get(actor), victims.get(victim))
    else {
        return;
    };
    let ((rolls, percent, effects, slays), (critical, trains, word, sound)) = arm;
    let (reduction, vitals, resistances, orders, member, walking) = struck;
    news.write(CombatNews::Struck { victim, place, sound_class: sound.map(|sound| sound.0) });
    practise(victim, Skill::IronSkin, &trained, &mut commands);

    let close = !attack::keeps_distance(with.mode);
    let surprise = caught.0;
    let on_the_move = moving(member, walking, &round);
    let mut count = if (surprise.unaware || on_the_move) && close { 2 } else { 1 };
    if surprise.natural && close {
        count += attack::backstab_extra(rng.below(100) as i32, backstab.0);
    }
    if with.mode == AttackMode::Berserk {
        count += 1;
    }
    if count > 1 && orders.is_some_and(Ordered::defends) && !surprise.unaware {
        count -= 1;
    }
    if let Some(kind) = resistances.kind {
        count += slays.0.iter().filter(|&&slain| slain == kind).count() as u32;
    }
    let rolled = attack::roll_dice(&mut rng, rolls.of(with.mode), count);
    let mut damage = attack::reduce(attack::damage_of(percent.0, rolled) as i32, reduction.0);
    let mut carried = effects.0;
    if astray {
        if damage > 0 {
            damage = damage.min((vitals.health.max / 5).max(1));
        }
    } else {
        if let Some(&Critical(skill)) = critical {
            let kill = &mut carried.chances[AttackEffect::Kill];
            *kill = (*kill + skill / 25 + u32::from(rng.below(25) < skill % 25)).min(100);
        }
        if let Some(&KnockOut(level)) = knock_out {
            let knock_out = &mut carried.chances[AttackEffect::KnockOut];
            *knock_out =
                (*knock_out + crate::game::characters::scale_by_level(level, 5.0) as u32).min(100);
        }
    }
    if damage == 0 {
        if astray {
            news.write(CombatNews::Result { attacker: actor, target: victim, hits: 1, damage: 0 });
        }
        return;
    }
    let blow = Blow { damage, effects: carried, by_monster: word.is_some() };
    let strength = attack::effect_strength(level.0);
    let strike = effects::Strike::new(Rows::Blow, strength, Some(blow), Some(actor), &content);
    commands.spawn((Deed::at(strike, [victim]), Blown { attack, victim, astray }));
    if !astray {
        for &skill in trains.map_or(&[][..], |trains| trains.0) {
            practise(actor, skill, &trained, &mut commands);
        }
    }
}

/// A spell laid on one a close swing lands on may answer it (Razor Cloak,
/// section 7): the spell's dice rolled `count * power` times at the power the
/// cloak had, less the attacker's own reduction, a deed at the attacker.
#[allow(clippy::type_complexity)]
fn answer(
    struck: On<Add<Strikes>>,
    mut commands: Commands,
    swings: Query<(&SwingOf, &Fate, &Strikes)>,
    attacks: Query<(&AttackBy, &AttackWith)>,
    arms: Query<&Reaches>,
    reductions: Query<&Reduction>,
    mut laying: Laying,
    content: Res<Catalogs>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    let Ok((&SwingOf(attack), fate, &Strikes(victim))) = swings.get(struck.entity) else { return };
    if matches!(fate, Fate::Reached { lands: false, .. } | Fate::Missed) {
        return;
    }
    let Ok((&AttackBy(actor), with)) = attacks.get(attack) else { return };
    if arms.get(with.arm).is_ok_and(|reach| attack::is_missile(reach.0)) {
        return;
    }
    let Some((cloak, dice)) = laying.answer_blow(victim) else { return };
    let reduction = reductions.get(actor).map_or(0, |reduction| reduction.0);
    let answer = attack::riposte(&mut rng, Some(dice), cloak.power, reduction);
    if answer == 0 {
        news.write(CombatNews::Result { attacker: victim, target: actor, hits: 1, damage: 0 });
        return;
    }
    let blow = Blow { damage: answer, effects: AttackEffects::default(), by_monster: false };
    let strike =
        effects::Strike::new(Rows::Blow, Strength::default(), Some(blow), Some(victim), &content);
    commands.spawn((Deed::at(strike, [actor]), Answer { by: victim, at: actor }));
}

/// A swing is settled (section 6): it costs its attacker stamina, landed or
/// not, if its arm tires them: the draw (a punch `2 + below(3)`, a kick `3 +
/// below(3)`, a weapon without a weight `3 + below(4)`, one of weight `w`
/// `below(band) + 1 + band` with `band = w / 40 + 1`), doubled in a fury and
/// again by disease, then scaled by the load (retail `FatigueCharacter`).
/// What a missile weapon shoots or throws is spent: a launcher one of the
/// missiles in the other hand, a thrown weapon or a gun one of itself; a
/// weapon that casts its spell, or a thrown missile that is kept, spends
/// nothing. Once what it counts is used up, the hand has no attacks left and
/// the attack ends; otherwise it swings again.
#[allow(clippy::type_complexity)]
fn settle(
    swung: On<Swung>,
    mut commands: Commands,
    swings: Query<&SwingOf>,
    mut attacks: Query<(&AttackBy, &AttackWith, &mut SwingCount, Has<Fury>)>,
    arms: Query<(&Hand, Option<&Grip>, Option<&Heft>, Has<TiresToSwing>, Has<Loaded>)>,
    mut attackers: Query<(&mut Turn, &Conditions, &Modifiers, Option<&Burden>)>,
    strikers: Query<(Option<&Weapon>, Option<&Missile>)>,
    held: Items,
    mut rng: ResMut<GameRng>,
) {
    let swing = swung.swing;
    let Ok(&SwingOf(attack)) = swings.get(swing) else { return };
    commands.entity(swing).despawn();
    let Ok((&AttackBy(actor), with, mut count, fury)) = attacks.get_mut(attack) else { return };
    count.made += 1;
    let (Ok((hand, grip, heft, tires, loaded)), Ok((mut turn, conditions, own, burden))) =
        (arms.get(with.arm), attackers.get_mut(actor))
    else {
        return;
    };
    if let Some(load) = burden.map(|burden| burden.load).filter(|_| tires) {
        let (below, add) = attack::stamina_draw(with.mode, heft.map(|heft| heft.0));
        let cost = (rng.below(below) + add)
            * if fury { 2 } else { 1 }
            * if conditions.has(Condition::Diseased) { 2 } else { 1 };
        let slowed = conditions.has(Condition::Slowed);
        commands.trigger(Tires {
            actor,
            stamina: load.scale_fatigue(cost, slowed, own.laid.stride > 0.0),
        });
    }
    let weapon = grip.and_then(|grip| grip.weapon());
    let ranged = weapon
        .and_then(|item| strikers.get(item).ok())
        .is_some_and(|(w, _)| w.is_some_and(|w| w.ranged));
    if ranged {
        let slot = if loaded { hand.0.other() } else { hand.0 };
        if let Some(item) = held.worn(actor, slot.equip()) {
            let kept = strikers.get(item).is_ok_and(|(weapon, missile)| {
                weapon.is_some_and(|weapon| weapon.casts) || missile.is_some_and(|m| m.kept)
            });
            if !(kept && !loaded) {
                commands.trigger(Spend { item, amount: 1 });
                let left = held.charges(item).or(held.count(item));
                if attack::runs_out(left, count.made as usize) {
                    turn.run_out(hand.0);
                    return commands.trigger(AttackEnds { attack });
                }
            }
        }
    }
    commands.trigger(SwingsAgain { attack });
}

/// What a blow did counts toward its attack's tally; one gone astray is told
/// of on its own.
fn blown(
    landed: On<Landed>,
    blows: Query<&Blown>,
    mut tallies: Query<(&AttackBy, &mut Tally)>,
    mut news: MessageWriter<CombatNews>,
) {
    let Ok(&Blown { attack, victim, astray }) = blows.get(landed.deed) else { return };
    let done = attack::dealt(landed.effects.iter().copied());
    let Ok((&AttackBy(attacker), mut tally)) = tallies.get_mut(attack) else { return };
    if astray {
        told_alone((attacker, victim), done, &mut news);
    } else {
        tally.damage += done.damage;
        tally.effects.extend(done.rest);
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

/// What the attack came to is told, and took out of one that tires by what
/// it does; the attacker's next attack is set (section 7); the attack is
/// gone.
fn attack_ends(
    ends: On<AttackEnds>,
    mut commands: Commands,
    attacks: Query<(&AttackBy, &AttackAt, &AttackWith, &Tally, Has<Fury>)>,
    mut turns: Query<(&mut Turn, &Tempo)>,
    round: Single<&Round, With<Battle>>,
    mut news: MessageWriter<CombatNews>,
) {
    let attack = ends.attack;
    let Ok((&AttackBy(actor), &AttackAt(target), with, tally, fury)) = attacks.get(attack) else {
        return;
    };
    commands.entity(attack).despawn();
    let effort =
        if fury || with.mode == AttackMode::Berserk { Effort::Fury } else { Effort::Attack };
    commands.trigger(Exerted { actor, effort });
    news.write(CombatNews::Result {
        attacker: actor,
        target,
        hits: tally.hits,
        damage: tally.damage,
    });
    if !tally.effects.is_empty() {
        news.write(CombatNews::Effects(tally.effects.clone()));
    }
    let Ok((mut turn, tempo)) = turns.get_mut(actor) else { return };
    match attack::next_attack(turn.attacks, tempo.initiative, turn.due, round.moment) {
        Some(next) => {
            turn.due = next.slot;
            turn.when = Some(next.moment);
        }
        None => turn.end(),
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_observer(strike)
        .add_observer(seek)
        .add_observer(find_ways)
        .add_observer(pick)
        .add_observer(close_in)
        .add_observer(open)
        .add_observer(swing_again)
        .add_observer(aim)
        .add_observer(fate)
        .add_observer(step_in)
        .add_observer(block)
        .add_observer(land)
        .add_observer(blow)
        .add_observer(answer)
        .add_observer(settle)
        .add_observer(blown)
        .add_observer(answered)
        .add_observer(attack_ends);
}
