//! Weapon attacks in a fight (`docs/rules/attack.md`). An attack is an
//! entity related to who makes it ([`AttackBy`]) and, once chosen, to whom
//! it is made at ([`AttackAt`]); each swing is an entity of its attack
//! ([`SwingOf`]). What happens to them goes by events, each a moment of the
//! rules: an attack is chosen step by step ([`Seeks`], [`Found`]), opened
//! ([`Opens`]), announced, and swings until it ends ([`AttackEnds`]); a
//! swing aims, fares, is guarded, lands, hits and is settled, each a stage
//! whose observers are independent of one another and add the facts later
//! stages read: its [`Aim`], its [`Fate`], whom it [`Strikes`], whether it
//! gets [`Through`], and the deeds of its blow ([`BlowOf`]) and of a cloak's
//! answer ([`AnswerTo`]), whose outcomes are kept on it ([`Blew`],
//! [`Answered`]). The one that starts a sequence queues its stages in order;
//! Bevy carries out each, and all it sets off, before the next.
//!
//! Retail's voices on a miss, a kill or a fumble draw from the same random
//! numbers as the rules; here they draw nothing.

use bevy::prelude::*;

use super::action::Fights;
use super::approach::{CombatMove, MoveGoal};
use super::attack::{self, Surprise};
use super::effort::{Effort, Exerted, Tires};
use super::monster::close::{self, Charges, Move};
use super::turn::{
    AttacksLeft, Berserk, Done, Guards, Interceptions, Lightning, Moved, NextMoment, Ordered,
    Shown, Spotted, Waited,
};
use super::{Announcement, Battle, CombatNews, Combatants, Round};
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
    Modifiers, Places, Pressing, Reduction, Skills, Unaware, Unseated, reaches,
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
#[relationship(relationship_target = Attacking)]
pub struct AttackBy(pub Entity);

/// The attack an actor is making, or choosing; it goes with the actor.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = AttackBy, linked_spawn)]
pub struct Attacking(Vec<Entity>);

/// At whom an attack is made.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = AttackedBy)]
pub struct AttackAt(pub Entity);

/// The attacks made at an actor.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = AttackAt)]
pub struct AttackedBy(Vec<Entity>);

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

/// The swings an attack makes, and those begun.
#[derive(Component, Clone, Copy, Debug)]
pub struct SwingCount {
    pub most: u32,
    pub made: u32,
}

/// What an attack shoots or throws is used up: it swings no more.
#[derive(Component, Clone, Copy, Debug)]
pub struct RunOut;

/// What an attack's swings came to.
#[derive(Component, Clone, Debug, Default)]
pub struct Tally {
    pub hits: u32,
    pub damage: u32,
    pub effects: Vec<Effect>,
}

/// A swing of an attack.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = Swinging)]
pub struct SwingOf(pub Entity);

/// The swing an attack is making; it goes with the attack.
#[derive(Component, Debug)]
#[relationship_target(relationship = SwingOf, linked_spawn)]
pub struct Swinging(Entity);

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
#[relationship(relationship_target = StruckBy)]
pub struct Strikes(pub Entity);

/// The swings striking an actor.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = Strikes)]
pub struct StruckBy(Vec<Entity>);

/// A swing that gets through armour: where it lands, by its order among the
/// places of the one struck.
#[derive(Component, Clone, Copy, Debug)]
pub struct Through(pub usize);

/// A deed that is a swing's blow.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = SwingBlow)]
pub struct BlowOf(pub Entity);

/// The deed of a swing's blow, while it lands.
#[derive(Component, Debug)]
#[relationship_target(relationship = BlowOf, linked_spawn)]
pub struct SwingBlow(Entity);

/// A deed that is Razor Cloak's answer to a swing.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = SwingAnswer)]
pub struct AnswerTo(pub Entity);

/// The deed of Razor Cloak's answer to a swing, while it lands.
#[derive(Component, Debug)]
#[relationship_target(relationship = AnswerTo, linked_spawn)]
pub struct SwingAnswer(Entity);

/// What a swing's blow did once it landed.
#[derive(Component, Clone, Debug)]
pub struct Blew(pub attack::Dealt);

/// What Razor Cloak's answer to a swing did.
#[derive(Component, Clone, Debug)]
pub struct Answered(pub attack::Dealt);

/// A monster strikes: at its friends when berserk, else at its foes; with
/// nobody to strike, it closes in. Set on every monster as it is made.
pub fn monster_strikes(
    strike: On<Strike>,
    mut commands: Commands,
    monsters: Query<(Has<Berserk>, Has<AttacksLeft>)>,
) -> Result {
    let Strike { actor, fights } = *strike;
    let (berserk, has_attacks) = monsters.get(actor)?;
    // One that took up no attacks this round takes them up now (retail
    // `PrepareCharacterAttacks` before an attack its ailments forced).
    if !has_attacks {
        commands.trigger(super::turn::TakesUpAttacks { actor });
    }
    let looks_for = if berserk { Stance::Friendly } else { Stance::Hostile };
    let attack = begin(&mut commands, actor, fights, looks_for);
    commands.entity(attack).observe(close_in);
    commands.trigger(Seeks { attack });
    Ok(())
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

/// An attack being chosen looks for whom it could strike.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Seeks {
    #[event_target]
    pub attack: Entity,
}

/// An attack being chosen looks for the ways it could strike them.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct FindsWays {
    #[event_target]
    attack: Entity,
}

/// An attack being chosen found ways to strike, with the hand they are in.
#[derive(EntityEvent, Clone, Debug)]
pub struct Found {
    #[event_target]
    pub attack: Entity,
    slot: WeaponSlot,
    ways: Vec<Way>,
}

/// An attack being chosen found nobody to strike; `evaded` when its eye
/// missed someone it could otherwise have struck. What follows is set by
/// its maker.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Nobody {
    #[event_target]
    pub attack: Entity,
    pub evaded: bool,
}

/// An attack is made, at whom and how, and opens.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct Opens {
    #[event_target]
    attack: Entity,
}

/// An opened attack is announced.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct Announces {
    #[event_target]
    attack: Entity,
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

/// A swing is aimed.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct Aiming {
    #[event_target]
    swing: Entity,
}

/// What becomes of an aimed swing.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct Faring {
    #[event_target]
    swing: Entity,
}

/// A swing that reached someone may be taken by a guard.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct Guarding {
    #[event_target]
    swing: Entity,
}

/// A swing comes down on whom it strikes.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct Landing {
    #[event_target]
    swing: Entity,
}

/// A swing that got through is a blow.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct Hitting {
    #[event_target]
    swing: Entity,
}

/// A swing is settled.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct Settling {
    #[event_target]
    swing: Entity,
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

/// An actor that fights begins choosing whom to strike and how (section
/// 2), looking for those who stand so toward it; attacking holds the fight
/// for the action's pause. Its maker sets what follows finding nobody, and
/// then has it seek ([`Seeks`]).
pub fn begin(commands: &mut Commands, actor: Entity, fights: Fights, looks_for: Stance) -> Entity {
    commands.trigger(Shown { actor });
    commands.spawn((AttackBy(actor), Choosing(fights), LooksFor(looks_for))).id()
}

/// Whom an attack being chosen looks for: those who stand so toward its
/// maker.
#[derive(Component, Clone, Copy, Debug)]
pub struct LooksFor(pub Stance);

/// It looks only among its maker's fellow members: a member turned or
/// berserk strikes only at the party (retail `PickReachableSlotByDisposition`,
/// `CanPartySlotAttackAnyTarget` with no monster scan).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct AmongFellows;

/// Whom it looks for among the living combatants, or its maker's fellow
/// members. Anyone not under orders misses a party member with chance
/// `Stealth * 75 / 100` percent.
fn seek(
    seeks: On<Seeks>,
    mut commands: Commands,
    attacks: Query<(&AttackBy, &LooksFor, Has<AmongFellows>)>,
    actors: Query<(&Standing, Has<Ordered>)>,
    candidates: Query<(&Vitals, &Standing, &Skills)>,
    fight: Single<Option<&Combatants>, With<Battle>>,
    roster: Single<&PartyRoster>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let attack = seeks.attack;
    let (&AttackBy(actor), &LooksFor(seeks), fellows) = attacks.get(attack)?;
    let (standing, ordered) = actors.get(actor)?;
    let among: Vec<Entity> = match fellows {
        true => roster.iter().collect(),
        false => fight.map_or(vec![], |fight| fight.entities().to_vec()),
    };
    let mut sought = Vec::new();
    for other in among.into_iter().filter(|&o| o != actor) {
        let (their, theirs, skills) = candidates.get(other)?;
        if their.is_dead() || hostility::between(*standing, *theirs) != seeks {
            continue;
        }
        let stealth = skills[Skill::Stealth];
        let lost = !ordered && roster.contains(&other) && rng.below(100) < stealth * 75 / 100;
        sought.push((other, lost));
    }
    commands.entity(attack).insert(Sought(sought));
    commands.trigger(FindsWays { attack });
    Ok(())
}

/// The ways it could strike: with an arm of the hand due if it has attacks
/// left and an arm that reaches someone it looks for, otherwise the other.
/// A member under orders never strikes an NPC and needs no sight of whom
/// they strike; anyone else needs a clear line. A member its eye missed, who
/// it could otherwise have struck, learns Stealth from it half the time.
#[allow(clippy::type_complexity)]
fn find_ways(
    finds: On<FindsWays>,
    mut commands: Commands,
    attacks: Query<(&AttackBy, &Sought)>,
    actors: Query<(Option<&AttacksLeft>, &Arms, Has<Ordered>)>,
    arms: Query<(&Hand, &Reaches, &Modes)>,
    npcs: Query<(), With<Embodies>>,
    trained: Query<(&Calling, &Abilities)>,
    places: Places,
    formation: Single<&Formation>,
    obstacles: WorldObstacles,
    mut rng: ResMut<GameRng>,
) -> Result {
    let attack = finds.attack;
    let (&AttackBy(actor), Sought(sought)) = attacks.get(attack)?;
    let (left, held, ordered) = actors.get(actor)?;
    // One that took up no attacks (it has no arm that strikes) has nobody
    // to strike.
    let Some(left) = left else {
        commands.trigger(Nobody { attack, evaded: false });
        return Ok(());
    };
    let at = places.of(actor);
    let mut evaded = Vec::new();
    let mut found = None;
    for slot in [left.due, left.due.other()] {
        if left.left[slot] == 0 {
            continue;
        }
        let mut ways = Vec::new();
        for &arm in held.entities() {
            let (hand, reach, modes) = arms.get(arm)?;
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
            found = Some((slot, ways));
            break;
        }
    }
    let missed = !evaded.is_empty();
    for member in evaded.into_iter().filter(|_| rng.below(2) == 0) {
        practise(member, Skill::Stealth, &trained, &mut commands);
    }
    match found {
        Some((slot, ways)) => commands.trigger(Found { attack, slot, ways }),
        None => commands.trigger(Nobody { attack, evaded: missed }),
    }
    Ok(())
}

/// The way it strikes. A member under orders strikes whoever is in the
/// lowest condition (a sound one before a hurt one), then one in reach of
/// the party (armed, and on its rear side, or on a side nobody holds while
/// its centre is held), then the nearest, ties keeping the fight's order
/// (retail `CompareMonsterTargetCandidates`), in the berserk mode under a
/// Berserk order and otherwise in one of the arm's modes. For anyone else
/// every way to strike has an equal chance, and so has waiting, for one that
/// may hold back (retail `ChooseMonsterTarget`). The hand's attacks go down
/// by one, and the attack is made: it opens, is announced and swings.
#[allow(clippy::type_complexity)]
fn pick(
    found: On<Found>,
    mut commands: Commands,
    attacks: Query<(&AttackBy, &Choosing)>,
    mut actors: Query<(&mut AttacksLeft, Has<Ordered>)>,
    modes: Query<&Modes>,
    candidates: Query<(&Conditions, &Furthest)>,
    places: Places,
    formation: Single<&Formation>,
    mut rng: ResMut<GameRng>,
) -> Result {
    use crate::game::party::formation::Row;
    let Found { attack, slot, ref ways } = *found;
    let (&AttackBy(actor), &Choosing(fights)) = attacks.get(attack)?;
    let (mut left, ordered) = actors.get_mut(actor)?;
    let at = places.of(actor);
    let picked = if ordered {
        let mut ranked = Vec::with_capacity(ways.len());
        for &way in ways {
            let there = places.of(way.target);
            let (ailing, reach) = candidates.get(way.target)?;
            let side = at.quadrant(there.feet);
            let reached = reach.0.is_some()
                && (side == Row::Rear
                    || (!formation.occupied(side) && formation.occupied(Row::Center)));
            ranked.push((way, (ailing.worst(), !reached, at.gap(&there))));
        }
        let (best, _) = ranked
            .into_iter()
            .min_by(|(_, a), (_, b)| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.total_cmp(&b.2)))
            .expect("ways are found only when there is one");
        let mode = if fights.berserk {
            AttackMode::Berserk
        } else {
            attack::choose_mode(&mut rng, modes.get(best.arm)?.0)
        };
        Some(Way { mode, ..best })
    } else {
        let waits = usize::from(fights.waits);
        let pick = rng.below((ways.len() + waits) as u32) as usize;
        (pick >= waits).then(|| ways[pick - waits])
    };
    let Some(Way { arm, mode, target }) = picked else {
        commands.entity(actor).insert(Waited);
        commands.trigger(Done { actor });
        commands.trigger(Exerted { actor, effort: Effort::Wait });
        commands.entity(attack).despawn();
        return Ok(());
    };
    left.due = slot;
    left.left[slot] -= 1;
    let mut made = commands.entity(attack);
    made.insert((AttackWith { arm, mode }, AttackAt(target), Tally::default()));
    if fights.berserk {
        made.insert(Fury);
    }
    commands.trigger(Opens { attack });
    commands.trigger(Announces { attack });
    commands.trigger(SwingsAgain { attack });
    Ok(())
}

/// With nobody to strike, one that walks and has not moved closes in: on
/// the party if it is hostile to it, on the nearest it looks for if it
/// stands with the party; charging when hurt; and acts again at twice the
/// moment now, at most 100 (retail `SetUpMonsterTurn`). Anything else
/// stands.
#[allow(clippy::type_complexity)]
pub fn close_in(
    nobody: On<Nobody>,
    mut commands: Commands,
    attacks: Query<(&AttackBy, &Sought)>,
    actors: Query<(
        (Has<Moved>, Option<&Walker>, Option<&Move>, Has<Charges>),
        (&Furthest, &Standing, &Vitals),
    )>,
    living: Query<(), With<Vitals>>,
    places: Places,
    round: Single<&Round, With<Battle>>,
    roster: Single<&PartyRoster>,
    mut news: MessageWriter<CombatNews>,
) -> Result {
    let attack = nobody.attack;
    commands.entity(attack).despawn();
    let (&AttackBy(actor), Sought(sought)) = attacks.get(attack)?;
    let ((moved, walker, moves, charges), (furthest, standing, vitals)) = actors.get(actor)?;
    let (Some(walker), Some(moves)) = (walker, moves.filter(|_| !moved)) else {
        commands.trigger(Done { actor });
        return Ok(());
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
    let Some(goal) = goal else {
        commands.trigger(Done { actor });
        return Ok(());
    };
    let budget = if charges { moves.charging } else { moves.walking };
    commands.entity(actor).insert((
        Moved,
        NextMoment((round.moment * 2).min(100)),
        CombatMove { budget, moved: 0.0, goal },
    ));
    news.write(CombatNews::Moves { monster: actor, charge: charges });
    let effort = if charges { Effort::Charge } else { Effort::Approach };
    commands.trigger(Exerted { actor, effort });
    Ok(())
}

/// An attack made keeps the fight going this round.
fn count_attack(_: On<Opens>, mut fight: Single<&mut Round, With<Battle>>) {
    fight.attacked(true);
}

/// A member under orders, and neither turned nor berserk, provokes whom
/// they strike (retail `Combat.cpp:2238`).
fn provoke(
    opens: On<Opens>,
    mut commands: Commands,
    attacks: Query<(&AttackBy, &AttackAt, &LooksFor)>,
    attackers: Query<(&Conditions, Has<Ordered>)>,
) -> Result {
    let (&AttackBy(actor), &AttackAt(target), &LooksFor(stance)) = attacks.get(opens.attack)?;
    let (ailing, ordered) = attackers.get(actor)?;
    // An attack at friends is a berserk one.
    if ordered && stance == Stance::Hostile && !ailing.has(Condition::Turncoat) {
        commands.trigger(Provocation { monster: target });
    }
    Ok(())
}

/// How an attack catches its target (section 3, retail
/// `IsPartyLookingAwayFrom`): a seated member faces the way the formation
/// says, anyone else its own front; only a touch or short blow can catch one
/// unaware; a member keeping watch (defending, alive, not spent, nothing as
/// bad as Webbed, not blind) who notices turns to face it instead; anyone
/// else is caught if the attacker is exactly across from the way they face.
/// An arm that ambushes strikes naturally whether or not the target faces
/// it.
#[allow(clippy::type_complexity)]
fn catch(
    opens: On<Opens>,
    mut commands: Commands,
    attacks: Query<(&AttackBy, &AttackAt, &AttackWith)>,
    arms: Query<(&Reaches, Has<Ambushes>)>,
    mut targets: Query<(Option<&mut Spotted>, Option<&Ordered>, &Vitals, &Conditions, &Attributes)>,
    places: Places,
    mut formation: Single<&mut Formation>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let attack = opens.attack;
    let (&AttackBy(actor), &AttackAt(target), with) = attacks.get(attack)?;
    let (reach, ambushes) = arms.get(with.arm)?;
    let attacker_on = places.of(target).quadrant(places.of(actor).feet);
    let facing = formation.facing(target).unwrap_or(crate::game::party::formation::Row::Front);
    let mut surprise = Surprise { unaware: false, natural: ambushes };
    if reach.0 <= crate::content::species::Reach::Short && facing != attacker_on {
        let (spotted, orders, their, ailing, a) = targets.get_mut(target)?;
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
    commands.entity(attack).insert(Caught(surprise));
    Ok(())
}

/// The swings an attack makes (section 4): `below(swings) + 1`, the
/// weapon's extra swings added, at least one. A readied Lightning Strike
/// adds 3 or 4 with a sword in the main hand when not slowed, and is used
/// up by the main hand's first attack whether or not it adds any.
fn count(
    opens: On<Opens>,
    mut commands: Commands,
    attacks: Query<(&AttackBy, &AttackWith)>,
    attackers: Query<(Option<&Lightning>, &Conditions)>,
    arms: Query<(&Hand, &Swings, Has<Sword>)>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let attack = opens.attack;
    let (&AttackBy(actor), with) = attacks.get(attack)?;
    let (lightning, conditions) = attackers.get(actor)?;
    let (hand, swings, sword) = arms.get(with.arm)?;
    let mut most = (rng.below(swings.most) as i32 + 1 + swings.extra).max(1) as u32;
    if let Some(&Lightning(level)) = lightning.filter(|_| hand.0 == WeaponSlot::Primary) {
        if sword && !conditions.has(Condition::Slowed) {
            most += if rng.below(11) + u32::from(level) < 20 { 3 } else { 4 };
        }
        commands.entity(actor).remove::<Lightning>();
    }
    commands.entity(attack).insert(SwingCount { most, made: 0 });
    Ok(())
}

/// The attack is announced (section 3): who strikes whom, how, with how many
/// swings, and whether by surprise; a member's weapon is heard swung (retail
/// `MakePCAttackSound`).
fn announce(
    announces: On<Announces>,
    attacks: Query<(&AttackBy, &AttackAt, &AttackWith, &Caught, &SwingCount)>,
    arms: Query<(Option<&Word>, Option<&SoundClass>)>,
    mut news: MessageWriter<CombatNews>,
) -> Result {
    let (&AttackBy(attacker), &AttackAt(target), with, &Caught(surprise), count) =
        attacks.get(announces.attack)?;
    let (word, sound) = arms.get(with.arm)?;
    let how = match word {
        Some(&Word(word, named)) => Announcement::Monster { word, mode: with.mode, named },
        None => Announcement::Mode(with.mode),
    };
    news.write(CombatNews::Attack { attacker, target, how, swings: count.most, surprise });
    if word.is_none() {
        news.write(CombatNews::Swung { sound_class: sound.map(|sound| sound.0) });
    }
    Ok(())
}

/// An attack swings again while it has swings left, has not run out of what
/// it shoots, its attacker can fight and its target lives: the swing aims,
/// fares, may be taken by a guard, lands, hits, is settled and is gone, and
/// the attack swings again. Otherwise the attack ends, and is gone.
fn swing_again(
    again: On<SwingsAgain>,
    mut commands: Commands,
    mut attacks: Query<(&AttackBy, &AttackAt, &mut SwingCount, Has<RunOut>)>,
    able: Query<(), With<CanFight>>,
    vitals: Query<&Vitals>,
) -> Result {
    let attack = again.attack;
    let (&AttackBy(actor), &AttackAt(target), mut count, run_out) = attacks.get_mut(attack)?;
    let alive = !vitals.get(target)?.is_dead();
    if count.made < count.most && !run_out && able.contains(actor) && alive {
        count.made += 1;
        let swing = commands.spawn(SwingOf(attack)).id();
        commands.trigger(Aiming { swing });
        commands.trigger(Faring { swing });
        commands.trigger(Guarding { swing });
        commands.trigger(Landing { swing });
        commands.trigger(Hitting { swing });
        commands.trigger(Settling { swing });
        commands.entity(swing).despawn();
        commands.trigger(SwingsAgain { attack });
    } else {
        commands.trigger(AttackEnds { attack });
        commands.entity(attack).despawn();
    }
    Ok(())
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
    aiming: On<Aiming>,
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
) -> Result {
    let swing = aiming.swing;
    let &SwingOf(attack) = swings.get(swing)?;
    let (&AttackBy(actor), &AttackAt(target), with, &Caught(surprise)) = attacks.get(attack)?;
    let (rating, modern) = arms.get(with.arm)?;
    let (bonus, fatigue, agility, standing, own, blind) = attackers.get(actor)?;
    let (armour, unaware, unseated, their_agility, member, walking) = targets.get(target)?;
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
    Ok(())
}

/// What becomes of a swing (section 4): a close one rolls against its aim
/// and may go astray onto one of the living in reach of its arm other than
/// its target (friends, foes and the attacker alike), by odds cubic in what
/// the aim falls short of 100; a shot is scattered about its target and
/// strikes or misses.
#[allow(clippy::type_complexity)]
fn fate(
    faring: On<Faring>,
    mut commands: Commands,
    swings: Query<(&SwingOf, &Aim)>,
    mut attacks: Query<(&AttackBy, &AttackAt, &AttackWith, &mut Caught)>,
    arms: Query<(&Reaches, Has<SureThrow>)>,
    blinded: Query<(), With<Blinded>>,
    vitals: Query<&Vitals>,
    places: Places,
    fight: Single<Option<&Combatants>, With<Battle>>,
    formation: Single<&Formation>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) -> Result {
    let swing = faring.swing;
    let (&SwingOf(attack), aim) = swings.get(swing)?;
    let (&AttackBy(actor), &AttackAt(target), with, mut caught) = attacks.get_mut(attack)?;
    let (reach, sure_throw) = arms.get(with.arm)?;
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
        let mut astray = Vec::new();
        for &other in combatants.iter().filter(|&&other| other != target) {
            if !vitals.get(other)?.is_dead()
                && (other == actor
                    || reaches(&formation, (actor, &from), (other, &places.of(other)), reach.0))
            {
                astray.push(other);
            }
        }
        let roll = rng.below(100) + 1;
        let odds = attack::fumble_chance(astray.len(), aim.fumble_odds);
        match attack::verdict(roll, astray.len(), odds) {
            attack::Verdict::Misses => Fate::Missed,
            attack::Verdict::Reaches { sure } => {
                Fate::Reached { roll, lands: sure || aim.aim >= roll as i32, sure }
            }
            attack::Verdict::Fumbles => {
                let victim = astray[rng.below(astray.len() as u32) as usize];
                caught.0 = Surprise::default();
                news.write(CombatNews::Fumble { attacker: actor, victim });
                Fate::Astray { victim, roll: rng.below(100) + 1 }
            }
        }
    };
    commands.entity(swing).insert(fate);
    Ok(())
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
    guarding: On<Guarding>,
    mut commands: Commands,
    swings: Query<(&SwingOf, &Fate)>,
    mut attacks: Query<(&AttackBy, &AttackAt, &mut Caught)>,
    pressing: Query<&Pressing>,
    wards: Query<&Guards>,
    mut guards: Query<(Has<CanGuard>, &GuardReach, &mut Interceptions)>,
    places: Places,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) -> Result {
    let swing = guarding.swing;
    let (&SwingOf(attack), fate) = swings.get(swing)?;
    let (&AttackBy(actor), &AttackAt(target), mut caught) = attacks.get_mut(attack)?;
    let ward = match *fate {
        Fate::Missed => return Ok(()),
        Fate::Reached { .. } => target,
        Fate::Astray { victim, .. } => victim,
    };
    let pressed = pressing.get(actor)?.0
        + 10 * i32::from(caught.0.unaware)
        + 10 * i32::from(caught.0.natural);
    let at = places.of(ward);
    let mut stepping = Vec::new();
    for &guard in wards.get(ward).map_or(&[][..], |ward| ward.entities()) {
        let (able, reach, made) = guards.get(guard)?;
        let chance = (pressing.get(guard)?.0 - pressed + 50).clamp(5, 95) as u32;
        if able
            && reach.0.is_some_and(|reach| places.of(guard).gap(&at) <= reach.metres())
            && made.taken < made.allowed
            && rng.below(100) < chance
        {
            stepping.push(guard);
        }
    }
    let mut victim = ward;
    if !stepping.is_empty() {
        let guard = stepping[rng.below(stepping.len() as u32) as usize];
        guards.get_mut(guard)?.2.take();
        commands.trigger(Tires { actor: guard, stamina: rng.below(3) + 2 });
        caught.0 = Surprise::default();
        news.write(CombatNews::Intercepts { guard, ward });
        victim = guard;
    }
    commands.entity(swing).insert(Strikes(victim));
    Ok(())
}

/// A near miss is blocked, and heard, where a miss is not: by its target's
/// block, when it saw the blow coming or was caught unaware.
fn block(
    landing: On<Landing>,
    swings: Query<(&SwingOf, &Fate, &Aim, Option<&Strikes>)>,
    attacks: Query<(&AttackWith, &Caught)>,
    blocks: Query<&Block>,
    sounds: Query<Option<&SoundClass>>,
    mut news: MessageWriter<CombatNews>,
) -> Result {
    let (&SwingOf(attack), &fate, aim, strikes) = swings.get(landing.swing)?;
    let (Fate::Reached { roll, lands: false, .. }, Some(&Strikes(victim))) = (fate, strikes) else {
        return Ok(());
    };
    let (with, caught) = attacks.get(attack)?;
    let block = blocks.get(victim)?;
    let within = if caught.0.any() { block.unaware } else { block.aware };
    if roll as i32 - aim.aim <= within {
        let sound_class = sounds.get(with.arm)?.map(|sound| sound.0);
        news.write(CombatNews::Blocked { victim, sound_class });
    }
    Ok(())
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
/// `TargetArmorClassAtLocation`).
#[allow(clippy::type_complexity)]
fn land(
    landing: On<Landing>,
    mut commands: Commands,
    swings: Query<(&SwingOf, &Fate, Option<&Strikes>)>,
    attacks: Query<(&AttackBy, &AttackWith, &Caught)>,
    arms: Query<(&Rating, Has<FatiguePierces>, Option<&Loaded>, Option<&LandsOnParty>)>,
    attackers: Query<(&AimBonus, &Fatigue, &Agility, &Standing)>,
    victims: Query<(
        (&HitPlaces, &Armour, &Against, &MissileShield),
        (&Unaware, Has<Unseated>, &Agility, Has<PartyMember>, Has<CombatMove>),
    )>,
    round: Single<&Round, With<Battle>>,
    difficulty: Res<Difficulty>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let swing = landing.swing;
    let (&SwingOf(attack), &fate, strikes) = swings.get(swing)?;
    let (roll, sure) = match fate {
        Fate::Reached { roll, lands: true, sure } => (roll, sure),
        Fate::Astray { roll, .. } => (roll, false),
        _ => return Ok(()),
    };
    let &Strikes(victim) = strikes.expect("a swing that lands strikes someone");
    let (&AttackBy(actor), with, caught) = attacks.get(attack)?;
    let (rating, pierces_tired, loaded, lands_on_party) = arms.get(with.arm)?;
    let (bonus, fatigue, agility, standing) = attackers.get(actor)?;
    let ((places, armour, against, shield), (unaware, unseated, their_agility, member, walking)) =
        victims.get(victim)?;
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
    }
    Ok(())
}

/// A spell laid on one a close swing lands on may answer it (Razor Cloak,
/// section 7): the spell's dice rolled `count * power` times at the power the
/// cloak had, less the attacker's own reduction, a deed at the attacker if
/// it comes to anything.
#[allow(clippy::type_complexity)]
fn answer(
    landing: On<Landing>,
    mut commands: Commands,
    swings: Query<(&SwingOf, &Fate, Option<&Strikes>)>,
    attacks: Query<(&AttackBy, &AttackWith)>,
    arms: Query<&Reaches>,
    reductions: Query<&Reduction>,
    mut laying: Laying,
    content: Res<Catalogs>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let swing = landing.swing;
    let (&SwingOf(attack), fate, strikes) = swings.get(swing)?;
    let Some(&Strikes(victim)) = strikes else { return Ok(()) };
    if matches!(fate, Fate::Reached { lands: false, .. } | Fate::Missed) {
        return Ok(());
    }
    let (&AttackBy(actor), with) = attacks.get(attack)?;
    if attack::is_missile(arms.get(with.arm)?.0) {
        return Ok(());
    }
    let Some((cloak, dice)) = laying.answer_blow(victim) else { return Ok(()) };
    let reduction = reductions.get(actor)?.0;
    let answer = attack::riposte(&mut rng, Some(dice), cloak.power, reduction);
    if answer == 0 {
        commands.entity(swing).insert(Answered(attack::Dealt::default()));
        return Ok(());
    }
    let blow = Blow { damage: answer, effects: AttackEffects::default(), by_monster: false };
    let strike =
        effects::Strike::new(Rows::Blow(blow), Strength::default(), Some(victim), &content);
    commands.spawn(AnswerTo(swing)).observe(answered).insert(Deed::at(strike, [actor]));
    Ok(())
}

/// A swing that got through is heard where it lands, and the one struck
/// practises Iron Skin, whether or not the blow then does damage (sections 5
/// and 6).
fn struck(
    hitting: On<Hitting>,
    mut commands: Commands,
    swings: Query<(&SwingOf, Option<&Strikes>, Option<&Through>)>,
    attacks: Query<&AttackWith>,
    sounds: Query<Option<&SoundClass>>,
    trained: Query<(&Calling, &Abilities)>,
    mut news: MessageWriter<CombatNews>,
) -> Result {
    let (&SwingOf(attack), strikes, through) = swings.get(hitting.swing)?;
    let (Some(&Strikes(victim)), Some(&Through(place))) = (strikes, through) else {
        return Ok(());
    };
    let with = attacks.get(attack)?;
    let sound_class = sounds.get(with.arm)?.map(|sound| sound.0);
    news.write(CombatNews::Struck { victim, place, sound_class });
    practise(victim, Skill::IronSkin, &trained, &mut commands);
    Ok(())
}

/// The blow of a swing that got through (section 5). Its dice are rolled
/// twice for a close blow that is unaware or at one on the move, with the
/// backstab's extra dice for a natural close blow, one more in a berserk
/// mode, one less (if more than one) at one defending a blow not unaware,
/// and one more for each kind the arm slays that the one struck is; raised
/// by the arm's percent, at least one, and cut by the one struck's
/// reduction. One gone astray takes at most a fifth of its victim's health,
/// at least one; otherwise the attacker's training raises the Kill chance (a
/// point per 25 of its critical skill, one more with chance `(skill mod 25) /
/// 25`) and the Knock Out chance (5 scaled by the level of one with the
/// trait), each held to 100. A blow that does damage is a deed of its own.
#[allow(clippy::type_complexity)]
fn blow(
    hitting: On<Hitting>,
    mut commands: Commands,
    swings: Query<(&SwingOf, &Fate, Option<&Strikes>, Has<Through>)>,
    attacks: Query<(&AttackBy, &AttackWith, &Caught)>,
    arms: Query<(&Rolls, &DamagePercent, &BlowEffects, &Slays, Option<&Critical>, Has<Word>)>,
    attackers: Query<(&Backstab, Option<&KnockOut>, &Level)>,
    victims: Query<(&Reduction, &Vitals, Option<&Ordered>, Has<PartyMember>, Has<CombatMove>)>,
    kinds: Query<&crate::game::actors::Kind>,
    round: Single<&Round, With<Battle>>,
    content: Res<Catalogs>,
    mut rng: ResMut<GameRng>,
) -> Result {
    use crate::content::items::AttackEffect;
    let swing = hitting.swing;
    let (&SwingOf(attack), fate, strikes, through) = swings.get(swing)?;
    // Only a swing that got through is a blow.
    if !through {
        return Ok(());
    }
    let &Strikes(victim) = strikes.expect("a swing that got through strikes someone");
    let astray = matches!(fate, Fate::Astray { .. });
    let (&AttackBy(actor), with, caught) = attacks.get(attack)?;
    let (rolls, percent, effects, slays, critical, by_monster) = arms.get(with.arm)?;
    let (backstab, knock_out, level) = attackers.get(actor)?;
    let (reduction, vitals, orders, member, walking) = victims.get(victim)?;

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
    if let Ok(&crate::game::actors::Kind(kind)) = kinds.get(victim) {
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
        return Ok(());
    }
    let blow = Blow { damage, effects: carried, by_monster };
    let strength = attack::effect_strength(level.0);
    let strike = effects::Strike::new(Rows::Blow(blow), strength, Some(actor), &content);
    // Watched before it lands, which it does as it is made.
    commands.spawn(BlowOf(swing)).observe(blew).insert(Deed::at(strike, [victim]));
    Ok(())
}

/// What a swing's blow did, once it has landed: its deed's own observer.
fn blew(landed: On<Landed>, blows: Query<&BlowOf>, mut commands: Commands) -> Result {
    let &BlowOf(swing) = blows.get(landed.deed)?;
    let done = attack::dealt(landed.effects.iter().copied());
    commands.entity(swing).insert(Blew(done));
    Ok(())
}

/// What Razor Cloak's answer did, once it has landed: its deed's own
/// observer.
fn answered(landed: On<Landed>, answers: Query<&AnswerTo>, mut commands: Commands) -> Result {
    let &AnswerTo(swing) = answers.get(landed.deed)?;
    let done = attack::dealt(landed.effects.iter().copied());
    commands.entity(swing).insert(Answered(done));
    Ok(())
}

/// A settled swing is told (section 7). One gone astray is told on its own
/// as a single hit, whatever its blow did; one that landed on whom it was
/// meant for, or a guard, is a hit of its attack, and what its blow did
/// counts toward the attack's tally. Then Razor Cloak's answer, if one was
/// made, is told on its own.
fn tell(
    settling: On<Settling>,
    swings: Query<(&SwingOf, &Fate, Option<&Strikes>, Option<&Blew>, Option<&Answered>)>,
    mut attacks: Query<(&AttackBy, &mut Tally)>,
    mut news: MessageWriter<CombatNews>,
) -> Result {
    let (&SwingOf(attack), fate, strikes, blew, answered) = swings.get(settling.swing)?;
    let Some(&Strikes(victim)) = strikes else { return Ok(()) };
    let (&AttackBy(attacker), mut tally) = attacks.get_mut(attack)?;
    let mut alone = |by, at, done: Option<&attack::Dealt>| {
        let damage = done.map_or(0, |done| done.damage);
        news.write(CombatNews::Result { attacker: by, target: at, hits: 1, damage });
        if let Some(done) = done.filter(|done| !done.rest.is_empty()) {
            news.write(CombatNews::Effects(done.rest.clone()));
        }
    };
    match fate {
        Fate::Astray { .. } => alone(attacker, victim, blew.map(|blew| &blew.0)),
        Fate::Reached { lands: true, .. } => {
            tally.hits += 1;
            if let Some(Blew(done)) = blew {
                tally.damage += done.damage;
                tally.effects.extend(done.rest.iter().copied());
            }
        }
        _ => {}
    }
    if let Some(Answered(done)) = answered {
        alone(victim, attacker, Some(done));
    }
    Ok(())
}

/// A blow that did damage, not gone astray, trains the arm's skills
/// (section 6).
fn trains(
    settling: On<Settling>,
    mut commands: Commands,
    swings: Query<(&SwingOf, &Fate, Has<Blew>)>,
    attacks: Query<(&AttackBy, &AttackWith)>,
    arms: Query<Option<&Trains>>,
    trained: Query<(&Calling, &Abilities)>,
) -> Result {
    let (&SwingOf(attack), fate, blew) = swings.get(settling.swing)?;
    if !blew || matches!(fate, Fate::Astray { .. }) {
        return Ok(());
    }
    let (&AttackBy(actor), with) = attacks.get(attack)?;
    for &skill in arms.get(with.arm)?.map_or(&[][..], |trains| trains.0) {
        practise(actor, skill, &trained, &mut commands);
    }
    Ok(())
}

/// A settled swing costs its attacker stamina, landed or not, if its arm
/// tires them (section 6): the draw (a punch `2 + below(3)`, a kick `3 +
/// below(3)`, a weapon without a weight `3 + below(4)`, one of weight `w`
/// `below(band) + 1 + band` with `band = w / 40 + 1`), doubled in a fury and
/// again by disease, then scaled by the load (retail `FatigueCharacter`).
/// One who carries no load (a monster) pays none.
#[allow(clippy::type_complexity)]
fn tire(
    settling: On<Settling>,
    mut commands: Commands,
    swings: Query<&SwingOf>,
    attacks: Query<(&AttackBy, &AttackWith, Has<Fury>)>,
    arms: Query<(Option<&Heft>, Has<TiresToSwing>)>,
    attackers: Query<(&Conditions, &Modifiers, Option<&Burden>)>,
    mut rng: ResMut<GameRng>,
) -> Result {
    let &SwingOf(attack) = swings.get(settling.swing)?;
    let (&AttackBy(actor), with, fury) = attacks.get(attack)?;
    let (heft, tires) = arms.get(with.arm)?;
    let (conditions, own, burden) = attackers.get(actor)?;
    // An arm that tires nobody, or one who carries no load, costs nothing.
    let Some(burden) = burden.filter(|_| tires) else { return Ok(()) };
    let (below, add) = attack::stamina_draw(with.mode, heft.map(|heft| heft.0));
    let cost = (rng.below(below) + add)
        * if fury { 2 } else { 1 }
        * if conditions.has(Condition::Diseased) { 2 } else { 1 };
    let slowed = conditions.has(Condition::Slowed);
    let stamina = burden.load.scale_fatigue(cost, slowed, own.laid.stride > 0.0);
    commands.trigger(Tires { actor, stamina });
    Ok(())
}

/// A settled swing of a missile weapon spends what it shoots or throws
/// (section 6): a launcher one of the missiles in the other hand, a thrown
/// weapon or a gun one of itself; a weapon that casts its spell, or a thrown
/// missile that is kept, spends nothing. Once what it counts is used up, the
/// hand has no attacks left and the attack has run out.
#[allow(clippy::type_complexity)]
fn spend(
    settling: On<Settling>,
    mut commands: Commands,
    swings: Query<&SwingOf>,
    attacks: Query<(&AttackBy, &AttackWith, &SwingCount)>,
    arms: Query<(&Hand, Option<&Grip>, Has<Loaded>)>,
    mut turns: Query<&mut AttacksLeft>,
    strikers: Query<(Option<&Weapon>, Option<&Missile>)>,
    held: Items,
) -> Result {
    let &SwingOf(attack) = swings.get(settling.swing)?;
    let (&AttackBy(actor), with, count) = attacks.get(attack)?;
    let (hand, grip, loaded) = arms.get(with.arm)?;
    // A monster's natural attack has no grip, and shoots nothing.
    let ranged = grip
        .and_then(|grip| grip.weapon())
        .and_then(|item| strikers.get(item).ok())
        .is_some_and(|(weapon, _)| weapon.is_some_and(|weapon| weapon.ranged));
    let slot = if loaded { hand.0.other() } else { hand.0 };
    let Some(item) = held.worn(actor, slot.equip()).filter(|_| ranged) else { return Ok(()) };
    let kept = strikers.get(item).is_ok_and(|(weapon, missile)| {
        weapon.is_some_and(|weapon| weapon.casts) || missile.is_some_and(|missile| missile.kept)
    });
    if kept && !loaded {
        return Ok(());
    }
    commands.trigger(Spend { item, amount: 1 });
    let left = held.charges(item).or(held.count(item));
    if attack::runs_out(left, count.made as usize) {
        turns.get_mut(actor)?.run_out(hand.0);
        commands.entity(attack).insert(RunOut);
    }
    Ok(())
}

/// An attack that ends took out of one that tires by what it does: twice
/// as much in a fury.
fn exert(
    ends: On<AttackEnds>,
    mut commands: Commands,
    attacks: Query<(&AttackBy, &AttackWith, Has<Fury>)>,
) -> Result {
    let (&AttackBy(actor), with, fury) = attacks.get(ends.attack)?;
    let effort =
        if fury || with.mode == AttackMode::Berserk { Effort::Fury } else { Effort::Attack };
    commands.trigger(Exerted { actor, effort });
    Ok(())
}

/// What the attack came to is told (section 7): its hits and damage, then
/// what else its blows did.
fn report(
    ends: On<AttackEnds>,
    attacks: Query<(&AttackBy, &AttackAt, &Tally)>,
    mut news: MessageWriter<CombatNews>,
) -> Result {
    let (&AttackBy(attacker), &AttackAt(target), tally) = attacks.get(ends.attack)?;
    news.write(CombatNews::Result { attacker, target, hits: tally.hits, damage: tally.damage });
    if !tally.effects.is_empty() {
        news.write(CombatNews::Effects(tally.effects.clone()));
    }
    Ok(())
}

/// The attacker's next attack is set (section 7): the slot and moment it
/// comes at, or its turn is over.
fn next(
    ends: On<AttackEnds>,
    attacks: Query<&AttackBy>,
    mut commands: Commands,
    mut turns: Query<(&mut AttacksLeft, &Tempo)>,
    round: Single<&Round, With<Battle>>,
) -> Result {
    let &AttackBy(actor) = attacks.get(ends.attack)?;
    let (mut left, tempo) = turns.get_mut(actor)?;
    match attack::next_attack(left.left, tempo.initiative, left.due, round.moment) {
        Some(next) => {
            left.due = next.slot;
            commands.entity(actor).insert(NextMoment(next.moment));
        }
        None => commands.trigger(Done { actor }),
    }
    Ok(())
}

pub(super) fn plugin(app: &mut App) {
    app.add_observer(seek)
        .add_observer(find_ways)
        .add_observer(pick)
        .add_observer(count_attack)
        .add_observer(provoke)
        .add_observer(catch)
        .add_observer(count)
        .add_observer(announce)
        .add_observer(swing_again)
        .add_observer(aim)
        .add_observer(fate)
        .add_observer(step_in)
        .add_observer(block)
        .add_observer(land)
        .add_observer(answer)
        .add_observer(struck)
        .add_observer(blow)
        .add_observer(tell)
        .add_observer(trains)
        .add_observer(tire)
        .add_observer(spend)
        .add_observer(exert)
        .add_observer(report)
        .add_observer(next);
}
