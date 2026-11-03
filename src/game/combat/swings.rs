//! Weapon attacks in a fight, swing by swing (retail `StartCharacterAttack`,
//! `ResolveCharacterAttack`, `StartMonsterAttack`, `ContinueMonsterAttack`).
//!
//! An attack is an entity made by its attacker ([`AttackBy`]) at its target
//! ([`AimsAt`]), carrying what it strikes with: its mode, rating, reach,
//! dice, effects and slaying. A member's attack also carries the weapon slot
//! it is drawn from ([`Wielded`]); the only systems that know member from
//! monster are the two that open attacks, from a member's weapon slot or a
//! monster's species attack, and the two that end them, back into a
//! member's or a monster's turn. Everything between reads the attack, and
//! the [`fighters`](super::fighters) components every body in the fight
//! carries alike.
//!
//! The [`Swings`] stages work an attack one swing at a time, each swing in
//! two passes: aiming ([`Aiming`]) and piercing ([`Piercing`]). In each pass
//! small systems add the retail terms that apply to the swing's running
//! [`Score`]; then the pass is settled: where an aimed swing lands, or what
//! a piercing one does. A blow lands through the hit stages as a deed that
//! is [`PartOf`] the attack, which tallies what its blows did and reports
//! once it is spent. [`run_swings`] runs the stages until every attack is
//! over, within the attacker's step, so each swing sees what the last one
//! did.
//!
//! Retail's voices on a miss, a kill or a fumble draw from the same random
//! numbers as the rules; here they draw nothing.

use avian3d::prelude::Position;
use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;
use enum_map::EnumMap;

use super::approach::{self, CombatMove};
use super::attack::{self, Aim, Grip, Location, Surprise};
use super::fighters::{
    Agility, ArmourClass, Backstab, BlindFighting, HitLocations, MissileShield, Protection,
    Reduction, reaches,
};
use super::monster_turn::{Decided, Fights, Resumes};
use super::round::{Acting, Guards, Interceptions, MonsterTurn, PartyTurn, Spotted};
use super::{Announcement, Battle, CombatNews, Combatants, RoundAction, RoundPhase};
use crate::content::characters::{BodyPart, Skill, Trait};
use crate::content::items::{AttackEffect, AttackEffects, AttackMode, Dice};
use crate::content::species::{AttackWord, MonsterKind, Reach};
use crate::content::spells::SpellId;
use crate::game::characters::{Character, Condition, Vitals};
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::factions::Factions;
use crate::game::hits::prototypes::{BlowDamage, Definitions, Uses, WeaponEffects};
use crate::game::hits::spells::reach;
use crate::game::hits::{Act, Deed, DeedHits, Effect, PartOf, Parts};
use crate::game::inventory::{Burden, Carried, WeaponSlot};
use crate::game::magic::Strength;
use crate::game::magic::enchantments::PartyEffects;
use crate::game::party::body::PartyBody;
use crate::game::party::formation::{Formation, Row};
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::time::GameClock;
use crate::game::world::ActorPose;
use crate::game::world::footing::Footing;
use crate::game::world::hostility::{self, Loyalty, Provocation, Side, Stance};
use crate::game::world::identify::{GroupMember, MonsterGroup};
use crate::game::world::navigation::Walker;
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::Monster;

/// The attack is made by this attacker.
#[derive(Component, Debug)]
#[relationship(relationship_target = Attacks)]
pub(super) struct AttackBy(pub(super) Entity);

/// An attacker's attack under way, gone with it.
#[derive(Component, Debug)]
#[relationship_target(relationship = AttackBy, linked_spawn)]
pub(super) struct Attacks(Vec<Entity>);

/// The attack is aimed at this body; a target gone ends it.
#[derive(Component, Debug)]
#[relationship(relationship_target = AimedAt)]
pub(super) struct AimsAt(pub(super) Entity);

/// The attacks aimed at a body.
#[derive(Component, Debug)]
#[relationship_target(relationship = AimsAt)]
pub(super) struct AimedAt(Vec<Entity>);

/// The body the swing under way is against: its target while it aims,
/// whoever it struck once it lands.
#[derive(Component, Debug)]
#[relationship(relationship_target = FacedBy)]
struct Faces(Entity);

/// The swings against a body.
#[derive(Component, Debug)]
#[relationship_target(relationship = Faces)]
struct FacedBy(Vec<Entity>);

/// Not yet announced: who it catches unaware is still being settled.
#[derive(Component, Debug)]
struct Opening;

/// How it strikes.
#[derive(Component, Clone, Copy, Debug)]
struct Mode(AttackMode);

#[derive(Component, Debug)]
struct SwingsLeft(u32);

/// How well its attacker strikes with it: a member's slot rating and five
/// times its to-hit, a monster's attack score and five times its to-hit
/// bonus, blessings among them.
#[derive(Component, Debug)]
struct Rating(i32);

/// How far it strikes; from long reach on it shoots.
#[derive(Component, Clone, Copy, Debug)]
struct Range(Reach);

/// The dice a blow rolls.
#[derive(Component, Debug)]
struct DamageDice(Dice);

/// What a blow's dice are raised by, in percent: a member's strength.
#[derive(Component, Debug)]
struct DamagePercent(i32);

/// What its blows may do besides damage.
#[derive(Component, Debug)]
struct BlowEffects(AttackEffects);

/// The monster kinds it slays, each a die more.
#[derive(Component, Debug)]
struct Slays(Vec<MonsterKind>);

/// A modern weapon's: fatigue tells on its aim half as much.
#[derive(Component, Debug)]
struct Modern;

/// Fatigue tells on its penetration too: a member's but with a modern
/// weapon; retail's monsters only with one.
#[derive(Component, Debug)]
struct FatiguePierces;

/// A member's attack, with the weapon in this slot.
#[derive(Component, Clone, Copy, Debug)]
struct Wielded(WeaponSlot);

/// Ranged combat: it trains Eagle Eye, and spends what it throws or shoots.
#[derive(Component, Debug)]
struct Ranged;

/// A launcher's: it shoots the ammunition in the other slot, which pierces
/// by its own modes.
#[derive(Component, Debug)]
struct Loaded(enumset::EnumSet<AttackMode>);

/// A Ninja's throw, which gets through armour whenever it strikes.
#[derive(Component, Debug)]
struct SureThrow;

/// The skill whose training adds a kill chance to its blows.
#[derive(Component, Debug)]
struct CriticalSkill(Skill);

/// What swinging its weapon weighs, for the stamina it costs.
#[derive(Component, Debug)]
struct Weight(u16);

/// A berserk fury, which costs twice the stamina.
#[derive(Component, Debug)]
struct Berserk;

/// How a monster's attack is announced.
#[derive(Component, Debug)]
struct Word {
    word: AttackWord,
    named: bool,
}

/// Where its blows land on a member, by its species' chances.
#[derive(Component, Debug)]
struct PartyHits(EnumMap<BodyPart, u8>);

/// When the monster's turn goes on after the attack, with how many attacks.
#[derive(Component, Debug)]
struct Continues {
    phase: u32,
    attacks_left: u32,
}

/// It can go no further; it reports once its blows have landed.
#[derive(Component, Debug)]
struct Spent;

/// What its blows have done so far.
#[derive(Component, Debug, Default)]
struct Tally {
    hits: u32,
    damage: u32,
    effects: Vec<Effect>,
    improved: Vec<(Entity, Skill)>,
    /// Monsters the party killed, with their experience.
    kills: Vec<(Entity, u32)>,
    /// Anyone else who died.
    deaths: Vec<Entity>,
}

impl Tally {
    /// The summary, then what else the blows did, then the casualties
    /// (retail `ReportCharacterAttackResult`).
    fn report(
        &mut self,
        battle: &mut Battle,
        attacker: Entity,
        target: Option<Entity>,
        news: &mut MessageWriter<CombatNews>,
    ) {
        if let Some(target) = target {
            news.write(CombatNews::Result {
                attacker,
                target,
                hits: self.hits,
                damage: self.damage,
            });
        }
        let effects = std::mem::take(&mut self.effects);
        if !effects.is_empty() {
            news.write(CombatNews::Effects(effects));
        }
        for (member, skill) in self.improved.drain(..) {
            news.write(CombatNews::Improved { member, skill });
        }
        for (monster, experience) in self.kills.drain(..) {
            battle.credit_kill(monster, experience, news);
        }
        for actor in self.deaths.drain(..) {
            news.write(CombatNews::Killed { actor });
        }
    }
}

/// A blow that reports on its own, not in its attack's summary: a fumble's,
/// or Razor Cloak's answer.
#[derive(Component, Debug)]
struct Alone {
    by: Entity,
    victim: Entity,
}

/// The swing is aiming at its target.
#[derive(Component, Debug)]
struct Aiming;

/// The swing landed and is piercing, by this mode.
#[derive(Component, Debug)]
struct Piercing(AttackMode);

/// The swing's running score: its aim, then its penetration.
#[derive(Component, Debug)]
struct Score(i32);

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
struct SwingDone;

/// The stages an attack goes through, run by [`run_swings`].
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct Swings;

/// Run the swing stages until every attack under way is over.
pub(super) fn run_swings(world: &mut World) {
    let mut passes = 0;
    while world.query_filtered::<(), With<AttackBy>>().iter(world).next().is_some() {
        passes += 1;
        if passes > 256 {
            error!("attacks left unsettled after {passes} passes");
            break;
        }
        world.run_schedule(Swings);
        // What a blow did lands before the next swing.
        crate::game::hits::run_hits(world);
    }
}

pub(super) fn stages(app: &mut App) {
    use super::fighters::{judge_members, judge_monsters};
    use crate::game::world::footing::{place_members, place_monsters};
    app.init_schedule(Swings).add_systems(
        Swings,
        (
            (place_members, place_monsters, judge_members, judge_monsters).chain(),
            (back_turned, catch_unaware, provoke, announce, opened).chain(),
            (tally_blows, start_swings).chain(),
            (
                rate,
                mode_terms,
                surprise_to_hit,
                fatigue,
                fumble_odds,
                armour,
                agility,
                blind,
                difficulty,
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
            (
                iron_skin,
                critical_strike,
                knockout,
                land_blows,
                train_strikes,
                riposte,
                count_hit,
            )
                .chain(),
            (shoot, swing_at, intercept, locate, dodge).chain(),
            (tire_members, spend_ammunition, finish_swing).chain(),
            (end_member_attacks, end_monster_attacks).chain(),
        )
            .chain(),
    );
}

/// The step of the round being played.
fn step(battle: &Battle) -> u32 {
    match battle.phase {
        RoundPhase::Executing(step) => step,
        _ => 1,
    }
}

/// Everyone in the fight: the party's members, then the monsters.
fn everyone<'a>(roster: &'a PartyRoster, combatants: &'a [Entity]) -> impl Iterator<Item = Entity> + 'a {
    roster.iter().chain(combatants.iter().copied())
}

/// The acting member attacks with the slot due (retail
/// `StartCharacterAttack`): at the best target that slot reaches, or with
/// the other slot when it reaches nobody; its mode and swings, a Samurai's
/// Lightning Strike and a Rogue's backstab. A member whose slots reach
/// nobody is done for the round.
pub(super) fn open_member_attacks(
    mut commands: Commands,
    mut actors: Query<
        (Entity, &mut PartyTurn, &Character, &Vitals, &Carried, &Burden, &Footing),
        With<Acting>,
    >,
    battle: Single<(&mut Battle, Option<&Combatants>)>,
    monsters: Query<(&Monster, &Vitals, &Side, &Footing)>,
    vitals: Query<&Vitals>,
    (formation, roster, party): (Single<&Formation>, Single<&PartyRoster>, Single<&PartyEffects>),
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    let (mut battle, combatants) = battle.into_inner();
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    let species = &content.species;
    // Who a slot of `reach` may strike (retail `ChooseMonsterTarget`):
    // among the monsters it reaches on the side the member fights, the
    // least afflicted, then those "in reach" of the party, then the
    // nearest; ties keep the fight's order. A member turned on the party
    // picks at random among their fellows and that monster.
    let target = |rng: &mut GameRng, member: Entity, at: &Footing, reach: Reach, turned: bool| {
        let monster = combatants
            .iter()
            .filter_map(|&entity| {
                let (monster, vitals, side, there) = monsters.get(entity).ok()?;
                let kind = &species[monster.species];
                let opposed = (side.0 == Stance::Friendly) == turned;
                if kind.npc.is_some()
                    || !opposed
                    || vitals.is_dead()
                    || !reaches(&formation, (member, at), (entity, there), reach)
                {
                    return None;
                }
                let quadrant = at.quadrant(there.feet);
                let in_reach = !kind.attacks.is_empty()
                    && (quadrant == Row::Rear
                        || (!formation.occupied(quadrant) && formation.occupied(Row::Center)));
                Some(((vitals.conditions.worst(), !in_reach, at.gap(there)), entity))
            })
            .min_by(|a, b| a.0.0.cmp(&b.0.0).then(a.0.1.cmp(&b.0.1)).then(a.0.2.total_cmp(&b.0.2)))
            .map(|(_, target)| target);
        if !turned {
            return monster;
        }
        let mut targets: Vec<Entity> = roster
            .iter()
            .filter(|&other| {
                other != member
                    && vitals.get(other).is_ok_and(|vitals| {
                        !vitals.is_dead() && !vitals.conditions.has(Condition::Turncoat)
                    })
            })
            .collect();
        targets.extend(monster);
        (!targets.is_empty()).then(|| targets[rng.below(targets.len() as u32) as usize])
    };
    for (member, mut turn, character, own, carried, burden, at) in &mut actors {
        let RoundAction::Attack { berserk, .. } = turn.action else { continue };
        commands.entity(member).remove::<Acting>();
        let turned = own.conditions.has(Condition::Turncoat);
        let grip = Grip::of(carried, &content.items);
        // The slot due attacks if it still can; one that cannot (out of
        // shots, or nobody in its reach) gives up its own attacks and the
        // other takes over (retail `StartCharacterAttack`,
        // `ComputeCharacterActionPhase`).
        let mut due = None;
        for slot in [turn.due, turn.due.other()] {
            let able = grip.in_play(slot) && turn.attacks[slot] > 0;
            match able.then(|| target(&mut rng, member, at, grip.reach(slot), turned)).flatten() {
                Some(target) => {
                    due = Some((slot, target));
                    break;
                }
                None => turn.attacks[slot] = 0,
            }
        }
        let Some((slot, target)) = due else {
            turn.end();
            continue;
        };
        battle.wait = Timer::from_seconds(super::ACTION_SECONDS, TimerMode::Once);
        battle.attacked(true);
        turn.due = slot;
        turn.attacks[slot] -= 1;
        let stats = character.stats(&own.enchantments);
        let load = burden.load;
        let reach = grip.reach(slot);
        // Retail draws the attack's mode, then its swings.
        let mode =
            if berserk { AttackMode::Berserk } else { attack::choose_mode(&mut rng, grip.modes(&stats, slot)) };
        let mut swings = rng.below(grip.swings(&stats, load, slot)) + 1;
        swings = (swings as i32 + grip.extra_swings(slot)).max(1) as u32;
        // A Samurai's Lightning Strike, readied as the round began, with a
        // sword in the primary slot.
        if turn.lightning && slot == WeaponSlot::Primary {
            turn.lightning = false;
            if grip.weapon_skill(slot) == Skill::Sword && !own.conditions.has(Condition::Slowed) {
                let level = u32::from(character.profession_levels[character.profession]);
                swings += if rng.below(11) + level < 20 { 3 } else { 4 };
            }
        }
        // A punch strikes with the secondary slot's fist, a kick with the
        // primary's.
        let dice = match mode {
            AttackMode::Punch => grip.dice(&stats, WeaponSlot::Secondary),
            AttackMode::Kick => grip.dice(&stats, WeaponSlot::Primary),
            _ => grip.dice(&stats, slot),
        };
        // A Rogue's sword or dagger at arm's length strikes where it hurts.
        let backstab = character.has_trait(Trait::Backstab)
            && reach <= Reach::Touch
            && matches!(grip.weapon_skill(slot), Skill::Sword | Skill::Dagger);
        let to_hit = grip.to_hit(&stats, load, slot) + party.0.hit_bonus();
        let power = attack::effect_power(u32::from(character.level()));
        let mut attack = commands.spawn((
            Opening,
            AttackBy(member),
            AimsAt(target),
            Wielded(slot),
            Mode(mode),
            SwingsLeft(swings),
            Surprise { unaware: false, natural: backstab },
            Tally::default(),
            Strength { power, against: power, boost: 0 },
            Rating(grip.rating(&stats, slot) + to_hit * 5),
            Range(reach),
            DamageDice(dice),
            DamagePercent(grip.damage_percent(&stats, slot)),
            BlowEffects(grip.effects(&stats, slot)),
            Slays(grip.slays(slot)),
        ));
        if grip.weapon_skill(slot) == Skill::ModernWeapon {
            attack.insert(Modern);
        } else {
            attack.insert(FatiguePierces);
        }
        if grip.ranged(slot) {
            attack.insert(Ranged);
        }
        if let Some(ammunition) = grip.ammunition(slot) {
            attack.insert(Loaded(ammunition.attack_modes));
        }
        if grip.sure_throw(&stats, slot) {
            attack.insert(SureThrow);
        }
        if let Some(skill) = grip.critical_skill(&stats, slot) {
            attack.insert(CriticalSkill(skill));
        }
        if let Some(weapon) = grip.weapon(slot) {
            attack.insert(Weight(weapon.weight));
        }
        if berserk {
            attack.insert(Berserk);
        }
    }
}

/// A monster that settled on fighting attacks (retail `StartMonsterAttack`):
/// one of its attacks and modes at someone it may attack, reaches and sees,
/// all equally likely, a wary one's wait among them (retail
/// `BuildMonsterActionQueue`). A member's Stealth may keep them out of its
/// eye this time (three quarters of it, in percent), and one that stayed
/// out of it may learn from it. With nobody to attack it closes in on a foe
/// ([`close_in`]) and acts again later in the round.
pub(super) fn open_monster_attacks(
    mut commands: Commands,
    mut fighters: Query<(
        Entity,
        &Fights,
        &mut MonsterTurn,
        &Monster,
        &Vitals,
        &Side,
        &Loyalty,
        &Footing,
        ActorPose,
        Option<&Walker>,
    )>,
    battle: Single<(&mut Battle, Option<&Combatants>)>,
    bodies: Query<(&Vitals, &Side, &Loyalty, &Footing), Without<Fights>>,
    mut characters: Query<&mut Character>,
    (formation, roster, party): (Single<&Formation>, Single<&PartyRoster>, Single<&Position, With<PartyBody>>),
    obstacles: WorldObstacles,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    let (mut battle, combatants) = battle.into_inner();
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    let species = &content.species;
    let rng = &mut *rng;
    let step = step(&battle);
    for (monster, fights, mut turn, body, vitals, side, loyalty, at, pose, walker) in &mut fighters {
        // Fighting is its action: it holds the fight for the action's pause.
        commands.entity(monster).remove::<(Fights, Decided, Resumes, Acting)>();
        battle.wait = Timer::from_seconds(super::ACTION_SECONDS, TimerMode::Once);
        let kind = &species[body.species];
        let mine = (side.0, *loyalty);
        let wanted = if turn.berserk { Stance::Friendly } else { Stance::Hostile };
        let unseen: Vec<Entity> = roster
            .iter()
            .filter(|&member| {
                let stealth = characters.get(member).map_or(0, |c| c.skill(Skill::Stealth));
                rng.below(100) < stealth * 75 / 100
            })
            .collect();
        let mut candidates = Vec::new();
        let mut evaded = Vec::new();
        for (a, swung) in kind.attacks.iter().enumerate() {
            for other in everyone(&roster, combatants) {
                let Ok((their_vitals, their_side, their_loyalty, there)) = bodies.get(other) else {
                    continue;
                };
                if their_vitals.is_dead()
                    || hostility::between(mine, (their_side.0, *their_loyalty)) != wanted
                    || !reaches(&formation, (monster, at), (other, there), swung.reach)
                    || !obstacles.clear(at.eye, there.centre)
                {
                    continue;
                }
                if unseen.contains(&other) {
                    if !evaded.contains(&other) {
                        evaded.push(other);
                    }
                    continue;
                }
                candidates.extend(swung.modes.iter().map(|mode| (a, mode, other)));
            }
        }
        for member in evaded {
            let mut character = characters.get_mut(member).unwrap();
            if rng.below(2) == 0 && character.practise(Skill::Stealth, 1, rng) {
                news.write(CombatNews::Improved { member, skill: Skill::Stealth });
            }
        }
        let (phase, attacks_left) = (turn.phase, turn.attacks_left);
        turn.end();
        // A wary monster's wait comes first among its choices.
        let waits = usize::from(fights.waits);
        if candidates.len() + waits == 0 {
            if turn.moved {
                continue;
            }
            let here = pose.translation();
            let party_eye = PartyBody::eye(&party);
            let goal = match side.0 {
                // A friendly monster closes on the nearest monster it may
                // attack, charging when hurt.
                Stance::Friendly => combatants
                    .iter()
                    .filter_map(|&other| {
                        let (their_vitals, their_side, their_loyalty, there) = bodies.get(other).ok()?;
                        let toward = hostility::between(mine, (their_side.0, *their_loyalty));
                        (!their_vitals.is_dead() && toward == wanted)
                            .then_some((at.feet.distance_squared(there.feet), other))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .filter(|_| walker.is_some_and(|walker| walker.speed > 0.0))
                    .map(|(_, target)| {
                        let charge = vitals.charges();
                        let metres = approach::move_metres(vitals, kind, charge);
                        (approach::MoveBudget { metres, charge }, approach::MoveGoal::Attack(target))
                    }),
                // A hostile one closes on the party.
                Stance::Hostile => {
                    approach::move_budget(body, vitals, here, walker, party_eye, species)
                        .map(|budget| (budget, approach::MoveGoal::Party))
                }
                Stance::Neutral => None,
            };
            if let Some((budget, goal)) = goal {
                turn.moved = true;
                turn.phase = (step * 2).min(100);
                turn.attacks_left = attacks_left;
                commands.entity(monster).insert(CombatMove { budget: budget.metres, moved: 0.0, goal });
                news.write(CombatNews::Moves { monster, charge: budget.charge });
            }
            continue;
        }
        let chosen = rng.below((candidates.len() + waits) as u32) as usize;
        if chosen < waits {
            turn.waited = true;
            continue;
        }
        battle.attacked(true);
        let (a, mode, target) = candidates[chosen - waits];
        let swung = &kind.attacks[a];
        // Retail draws for the mode again, from just the one chosen.
        let mode = attack::choose_mode(rng, enumset::EnumSet::only(mode));
        let swings = rng.below(u32::from(kind.swings_per_round.max(1))) + 1;
        // A natural ambusher strikes from hiding at arm's length.
        let ambush = kind.kind == MonsterKind::Rogue && swung.reach <= Reach::Touch;
        let power = attack::effect_power(u32::from(kind.level));
        let rating = i32::from(swung.score) + 5 * vitals.enchantments.hit_bonus();
        let mut attack = commands.spawn((
            Opening,
            AttackBy(monster),
            AimsAt(target),
            Mode(mode),
            SwingsLeft(swings),
            Surprise { unaware: false, natural: ambush },
            Tally::default(),
            Strength { power, against: power, boost: 0 },
            Rating(rating),
            Range(swung.reach),
            DamageDice(swung.damage),
            BlowEffects(swung.effects),
            Word { word: swung.word, named: swung.named() },
            PartyHits(kind.hits_on_party),
            Continues { phase, attacks_left: attacks_left.max(1) - 1 },
        ));
        if swung.tires_little() {
            attack.insert((Modern, FatiguePierces));
        }
    }
}

/// Whether a body has its back to someone at `from`: facing away by 135° or
/// more (retail `IsPartyLookingAwayFrom`).
fn back_to(footing: &Footing, from: Vec3) -> bool {
    let toward = (from - footing.feet).xz();
    let forward = footing.forward().xz();
    toward != Vec2::ZERO
        && forward != Vec2::ZERO
        && forward.angle_to(toward).abs() >= std::f32::consts::FRAC_PI_4 * 3.0
}

/// A member's close blow catches a monster with its back to the party
/// unaware.
fn back_turned(
    mut attacks: Query<(&AttackBy, &AimsAt, &Range, &mut Surprise), (With<Opening>, With<Wielded>)>,
    footings: Query<&Footing>,
    formation: Single<&Formation>,
) {
    for (by, aims, range, mut surprise) in &mut attacks {
        if range.0 > Reach::Short || formation.row_of(aims.0).is_some() {
            continue;
        }
        let (Ok(attacker), Ok(target)) = (footings.get(by.0), footings.get(aims.0)) else {
            continue;
        };
        surprise.unaware = back_to(target, attacker.eye);
    }
}

/// A member not facing a monster's close blow may turn in time, or be
/// struck from behind (retail `CharacterNoticesAttacker`): only one awake,
/// unblinded and defending notices; the first time each round always, then
/// by senses, 25 harder each time.
fn catch_unaware(
    mut attacks: Query<(&AttackBy, &AimsAt, &Range, &mut Surprise), (With<Opening>, Without<Wielded>)>,
    footings: Query<&Footing>,
    mut members: Query<(&Character, &Vitals, &PartyTurn, &mut Spotted)>,
    mut formation: Single<&mut Formation>,
    mut rng: ResMut<GameRng>,
) {
    for (by, aims, range, mut surprise) in &mut attacks {
        let Ok((character, vitals, turn, mut spotted)) = members.get_mut(aims.0) else { continue };
        let (Ok(attacker), Ok(target)) = (footings.get(by.0), footings.get(aims.0)) else {
            continue;
        };
        if range.0 > Reach::Short {
            continue;
        }
        let side = target.quadrant(attacker.feet);
        let Some(facing) = formation.facing(aims.0).filter(|&facing| facing != side) else {
            continue;
        };
        let alert = !vitals.is_dead()
            && vitals.stamina.current > 0
            && vitals.conditions.worst().is_none_or(|c| c < Condition::Webbed)
            && !vitals.conditions.has(Condition::Blind);
        let notices = alert
            && turn.defends()
            && if spotted.0 == 0 {
                spotted.0 = 1;
                true
            } else {
                // Retail compares unsigned: once senses fall short, noticing
                // is certain.
                let senses = character
                    .stats(&vitals.enchantments)
                    .attribute(crate::content::characters::Attribute::Senses);
                let chance = (senses as i32 - spotted.0 as i32 * 25) as u32;
                spotted.0 += 1;
                rng.below(100) < chance
            };
        if notices {
            formation.turn(aims.0, side);
        } else {
            surprise.unaware = facing.opposite() == Some(side);
        }
    }
}

/// The party attacks a monster: its group turns hostile, whatever the blows
/// then do (retail `MakeTargetGroupHostile` as the action starts). A member
/// under Turncoat, or a monster dead or under it, provokes nobody.
fn provoke(
    attacks: Query<(&AttackBy, &AimsAt), (With<Opening>, With<Wielded>)>,
    attackers: Query<&Vitals>,
    targets: Query<(&Monster, &Vitals, Option<&GroupMember>)>,
    mut groups: Query<&mut MonsterGroup>,
    (content, factions, clock): (If<Res<Catalogs>>, Res<Factions>, Res<GameClock>),
    mut provocations: MessageWriter<Provocation>,
) {
    let turned = |vitals: &Vitals| vitals.conditions.has(Condition::Turncoat);
    for (by, aims) in &attacks {
        if attackers.get(by.0).is_ok_and(turned) {
            continue;
        }
        let Ok((monster, vitals, group)) = targets.get(aims.0) else { continue };
        if vitals.is_dead() || turned(vitals) {
            continue;
        }
        let kind = &content.species[monster.species];
        if let Some(member) = group
            && let Ok(mut group) = groups.get_mut(member.0)
            && hostility::stance(group.set, kind, &factions, clock.ms, true) != Stance::Hostile
        {
            group.set = Some(hostility::SetStance { stance: Stance::Hostile, at: clock.ms });
        }
        provocations.write(Provocation(aims.0));
    }
}

/// Each attack is announced as it begins (retail `StartCharacterAttack`,
/// `AnnounceMonsterAttack`).
fn announce(
    attacks: Query<(&AttackBy, &AimsAt, &Mode, &SwingsLeft, &Surprise, Option<&Word>), With<Opening>>,
    mut news: MessageWriter<CombatNews>,
) {
    for (by, aims, mode, swings, surprise, word) in &attacks {
        let how = match word {
            Some(word) => Announcement::Monster { word: word.word, mode: mode.0, named: word.named },
            None => Announcement::Mode(mode.0),
        };
        news.write(CombatNews::Attack {
            attacker: by.0,
            target: aims.0,
            how,
            swings: swings.0,
            surprise: *surprise,
        });
    }
}

/// Its opening settled, the attack begins.
fn opened(mut commands: Commands, attacks: Query<Entity, With<Opening>>) {
    for attack in &attacks {
        commands.entity(attack).remove::<Opening>();
    }
}

/// Each blow that has landed is counted on its attack: its damage toward the
/// summary, or reported on its own; its kills, deaths, other effects and
/// what it taught.
fn tally_blows(
    mut commands: Commands,
    blows: Query<(Entity, &Deed, &PartOf, Option<&Alone>), Without<DeedHits>>,
    mut attacks: Query<(&mut Tally, &AttackBy)>,
    members: Query<(), With<Character>>,
    mut news: MessageWriter<CombatNews>,
) {
    for (blow, deed, part_of, alone) in &blows {
        commands.entity(blow).despawn();
        let Ok((mut tally, by)) = attacks.get_mut(part_of.0) else { continue };
        let striker = alone.map_or(by.0, |alone| alone.by);
        let mut damage = 0;
        for effect in &deed.effects {
            let Effect::Damaged { victim, outcome } = *effect else {
                tally.effects.push(*effect);
                continue;
            };
            damage += outcome.damage;
            if outcome.died {
                if members.contains(striker) && !members.contains(victim) {
                    tally.kills.push((victim, outcome.experience));
                } else {
                    tally.deaths.push(victim);
                }
            }
        }
        tally.improved.extend(deed.improved.iter().copied());
        match alone {
            Some(alone) => {
                news.write(CombatNews::Result {
                    attacker: alone.by,
                    target: alone.victim,
                    hits: 1,
                    damage,
                });
            }
            None => tally.damage += damage,
        }
    }
}

/// An attack with swings left begins its next one while the attacker can
/// still fight and the target is alive in the fight; otherwise it is spent.
fn start_swings(
    mut commands: Commands,
    attacks: Query<
        (Entity, &AttackBy, Option<&AimsAt>),
        (Without<Opening>, Without<Aiming>, Without<Piercing>, Without<Spent>),
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
                commands.entity(attack).insert((Aiming, Score(0), Faces(target)));
            }
            None => {
                commands.entity(attack).insert(Spent);
            }
        }
    }
}

/// How well the attacker strikes with it.
fn rate(mut swings: Query<(&mut Score, &Rating)>) {
    for (mut score, rating) in &mut swings {
        score.0 += rating.0;
    }
}

/// What the mode adds: to the aim by its own, to penetration by the mode
/// the blow pierces with (retail `AttackModeMod`).
fn mode_terms(mut swings: Query<(&mut Score, &Mode, Option<&Piercing>)>) {
    for (mut score, mode, piercing) in &mut swings {
        score.0 += match piercing {
            Some(piercing) => attack::mode_penetration(piercing.0),
            None => attack::mode_to_hit(mode.0),
        };
    }
}

/// A surprise steadies the aim: ten for an unaware target, ten for a
/// natural strike.
fn surprise_to_hit(mut swings: Query<(&mut Score, &Surprise), With<Aiming>>) {
    for (mut score, surprise) in &mut swings {
        score.0 += 10 * i32::from(surprise.unaware) + 10 * i32::from(surprise.natural);
    }
}

/// Fatigue tells on the attacker's aim, half as much with a modern weapon,
/// and on penetration where it pierces.
fn fatigue(
    mut swings: Query<(&mut Score, &AttackBy, Has<Piercing>, Has<Modern>, Has<FatiguePierces>)>,
    vitals: Query<&Vitals>,
) {
    for (mut score, by, piercing, modern, pierces) in &mut swings {
        let fatigue = attack::fatigue(vitals.get(by.0).unwrap());
        score.0 -= match (piercing, modern, pierces) {
            (false, true, _) => fatigue >> 1,
            (false, false, _) | (true, _, true) => fatigue,
            (true, _, false) => 0,
        };
    }
}

/// A swing's odds before its target's guard decide whether it goes astray.
fn fumble_odds(mut commands: Commands, swings: Query<(Entity, &Score, &Range), With<Aiming>>) {
    for (swing, score, range) in &swings {
        if range.0 < Reach::Long {
            commands.entity(swing).insert(FumbleOdds(score.0));
        }
    }
}

/// The faced body's armour turns the swing: all of it against the aim,
/// where the blow struck against penetration.
fn armour(
    mut swings: Query<(&mut Score, &Faces, &Mode, &Surprise, Option<&Piercing>, Option<&Location>)>,
    armours: Query<&ArmourClass>,
) {
    for (mut score, faces, mode, surprise, piercing, location) in &mut swings {
        let Ok(armour) = armours.get(faces.0) else { continue };
        score.0 -= 5 * match (piercing, location) {
            (Some(piercing), Some(location)) => armour.at(piercing.0, *surprise, location.0),
            _ => armour.against_aim(mode.0, *surprise),
        };
    }
}

/// The agility between attacker and faced body counts a tenth.
fn agility(mut swings: Query<(&mut Score, &AttackBy, &Faces)>, agilities: Query<&Agility>) {
    for (mut score, by, faces) in &mut swings {
        let (Ok(theirs), Ok(mine)) = (agilities.get(faces.0), agilities.get(by.0)) else {
            continue;
        };
        score.0 += (mine.0 - theirs.0) / 10;
    }
}

/// A blinded attacker's aim is worth 10 at most, more by profession for
/// one effective while blind.
fn blind(
    mut swings: Query<(&mut Score, Option<&mut FumbleOdds>, &AttackBy), With<Aiming>>,
    attackers: Query<(&Vitals, &BlindFighting)>,
) {
    for (mut score, odds, by) in &mut swings {
        let Ok((vitals, fighting)) = attackers.get(by.0) else { continue };
        if !vitals.conditions.has(Condition::Blind) {
            continue;
        }
        let cap = |aim: &mut i32| {
            if *aim > 10 {
                *aim = match fighting.0 {
                    Some(level) => crate::game::characters::scale_by_level(level, (*aim - 10) as f32) as i32 + 10,
                    None => 10,
                };
            }
        };
        cap(&mut score.0);
        if let Some(mut odds) = odds {
            cap(&mut odds.0);
        }
    }
}

/// The difficulty scales each swing by the side the attacker is on.
fn difficulty(
    mut swings: Query<(&mut Score, Option<&mut FumbleOdds>, &AttackBy)>,
    sides: Query<&Side>,
    difficulty: Res<Difficulty>,
) {
    for (mut score, odds, by) in &mut swings {
        let side = sides.get(by.0).unwrap().0;
        score.0 = difficulty.scale(score.0, side);
        if let Some(mut odds) = odds {
            odds.0 = difficulty.scale(odds.0, side);
        }
    }
}

/// A Missile Shield on the struck body turns aside what is shot or thrown.
fn deflect(
    mut commands: Commands,
    swings: Query<(Entity, &Faces, &Mode), With<Piercing>>,
    shields: Query<&MissileShield>,
    mut rng: ResMut<GameRng>,
) {
    for (swing, faces, mode) in &swings {
        let Ok(shield) = shields.get(faces.0) else { continue };
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
    swings: Query<(Entity, &Score, &Landed, &BlowEffects, Has<Through>, Has<Deflected>), With<Piercing>>,
) {
    for (swing, score, landed, effects, through, deflected) in &swings {
        let mut swing = commands.entity(swing);
        swing.insert(Damage(0));
        if !deflected && (through || score.0 >= landed.roll as i32) {
            swing.insert((DiceCount(1), Chances(effects.0)));
        }
    }
}

/// A target unaware, or distracted on the move, takes twice the dice of a
/// blow that closes in on it.
fn exposed_dice(
    mut blows: Query<(&mut DiceCount, &Mode, &Surprise, &Faces)>,
    armours: Query<&ArmourClass>,
) {
    for (mut count, mode, surprise, faces) in &mut blows {
        let distracted = armours.get(faces.0).is_ok_and(|armour| armour.distracted);
        if (surprise.unaware || distracted) && !attack::keeps_distance(mode.0) {
            count.0 = 2;
        }
    }
}

/// A natural strike that closes in rolls extra dice by the attacker's
/// backstab (retail `RollBackstabExtraDice`).
fn natural_dice(
    mut blows: Query<(&mut DiceCount, &Mode, &Surprise, &AttackBy)>,
    backstabs: Query<&Backstab>,
    mut rng: ResMut<GameRng>,
) {
    for (mut count, mode, surprise, by) in &mut blows {
        if surprise.natural
            && !attack::keeps_distance(mode.0)
            && let Ok(backstab) = backstabs.get(by.0)
        {
            count.0 += attack::backstab_dice(&mut rng, backstab.0);
        }
    }
}

/// A berserk blow rolls a die more.
fn berserk_dice(mut blows: Query<(&mut DiceCount, &Mode)>) {
    for (mut count, mode) in &mut blows {
        if mode.0 == AttackMode::Berserk {
            count.0 += 1;
        }
    }
}

/// A braced body that saw it coming takes a die less of a blow that rolls
/// more than one.
fn braced(mut blows: Query<(&mut DiceCount, &Faces, &Surprise)>, armours: Query<&ArmourClass>) {
    for (mut count, faces, surprise) in &mut blows {
        let braced = armours.get(faces.0).is_ok_and(|armour| armour.braced);
        if count.0 > 1 && braced && !surprise.unaware {
            count.0 -= 1;
        }
    }
}

/// Each weapon that slays the struck monster's kind rolls a die more.
fn slaying_dice(
    mut blows: Query<(&mut DiceCount, &Faces, &Slays)>,
    monsters: Query<&Monster>,
    content: If<Res<Catalogs>>,
) {
    for (mut count, faces, slays) in &mut blows {
        let Ok(monster) = monsters.get(faces.0) else { continue };
        let kind = content.species[monster.species].kind;
        count.0 += slays.0.iter().filter(|&&slays| slays == kind).count() as u32;
    }
}

/// The blow's dice, rolled so many times.
fn roll_damage(mut blows: Query<(&mut Damage, &DiceCount, &DamageDice)>, mut rng: ResMut<GameRng>) {
    for (mut damage, count, dice) in &mut blows {
        damage.0 = attack::roll_dice(&mut rng, dice.0, count.0);
    }
}

/// The blow is raised by its damage percent (retail
/// `ResolveCharacterAttackDamage`).
fn damage_percent(mut blows: Query<(&mut Damage, &DamagePercent), With<DiceCount>>) {
    for (mut damage, percent) in &mut blows {
        damage.0 = (((100 + percent.0) * damage.0 as i32 + 50) / 100).max(0) as u32;
    }
}

/// A blow that got through deals at least one before reduction.
fn at_least_one(mut blows: Query<&mut Damage, With<DiceCount>>) {
    for mut damage in &mut blows {
        damage.0 = damage.0.max(1);
    }
}

/// The struck body's damage reduction cuts the blow (retail
/// `ApplyDamageReduction`, `ApplyCharacterDamageReduction`).
fn reduce(mut blows: Query<(&mut Damage, &Faces), With<DiceCount>>, reductions: Query<&Reduction>) {
    for (mut damage, faces) in &mut blows {
        if let Ok(reduction) = reductions.get(faces.0) {
            damage.0 = attack::reduce(damage.0 as i32, reduction.0);
        }
    }
}

/// A blow that went astray takes a fifth of its victim's health at most.
fn fumble_caps(mut blows: Query<(&mut Damage, &Faces), With<Fumbled>>, vitals: Query<&Vitals>) {
    for (mut damage, faces) in &mut blows {
        if damage.0 > 0 {
            damage.0 = attack::fumble_cap(damage.0, vitals.get(faces.0).unwrap().health.max);
        }
    }
}

/// A member a blow got through to practises Iron Skin as its reduction is
/// worked out.
fn iron_skin(
    mut blows: Query<(&Faces, &mut Tally), With<DiceCount>>,
    mut characters: Query<&mut Character>,
    mut rng: ResMut<GameRng>,
) {
    for (faces, mut tally) in &mut blows {
        if let Ok(mut character) = characters.get_mut(faces.0)
            && character.skill_available(Skill::IronSkin)
            && character.practise(Skill::IronSkin, 1, &mut rng)
        {
            tally.improved.push((faces.0, Skill::IronSkin));
        }
    }
}

/// Training adds to a blow's chance to kill (retail
/// `ResolveCharacterAttack`), not when it went astray.
fn critical_strike(
    mut blows: Query<(&mut Chances, &AttackBy, &CriticalSkill), Without<Fumbled>>,
    characters: Query<(&Character, &Vitals)>,
    mut rng: ResMut<GameRng>,
) {
    for (mut chances, by, skill) in &mut blows {
        let (character, vitals) = characters.get(by.0).unwrap();
        let skill = character.stats(&vitals.enchantments).skill(skill.0);
        let kill = &mut chances.0.chances[AttackEffect::Kill];
        *kill = (*kill + attack::critical_chance(&mut rng, skill)).min(100);
    }
}

/// A Monk's blow may knock out, by their profession.
fn knockout(mut blows: Query<(&mut Chances, &AttackBy), Without<Fumbled>>, characters: Query<&Character>) {
    for (mut chances, by) in &mut blows {
        let Ok(character) = characters.get(by.0) else { continue };
        if character.has_trait(Trait::Knockout) {
            let knockout = &mut chances.0.chances[AttackEffect::KnockOut];
            *knockout = (*knockout + character.scale_by_profession(5.0) as u32).min(100);
        }
    }
}

/// A blow that deals damage lands as a deed of its attack's: its damage,
/// then what its weapon does besides, at the attack's strength.
fn land_blows(
    mut commands: Commands,
    blows: Query<(Entity, &Faces, &Damage, &Chances, &Strength, &AttackBy, Has<Fumbled>, Has<Wielded>)>,
    definitions: Res<Definitions>,
) {
    for (attack, faces, damage, chances, &strength, by, fumbled, wielded) in &blows {
        if damage.0 == 0 {
            continue;
        }
        let weapon = WeaponEffects { effects: chances.0, by_monster: !wielded };
        let mut blow = commands.spawn((
            Deed::new(Act::Fight),
            Uses(definitions.blow()),
            strength,
            weapon,
            BlowDamage(damage.0),
            PartOf(attack),
        ));
        if fumbled {
            blow.insert(Alone { by: by.0, victim: faces.0 });
        }
        let blow = blow.id();
        reach(&mut commands, blow, &[(faces.0, None)]);
    }
}

/// A member's blow that dealt damage, not by a fumble, trains their
/// strikes: close blows Critical and Power Strike, ranged ones Eagle Eye,
/// and a thrower's Critical Strike too.
fn train_strikes(
    mut blows: Query<(&mut Tally, &Damage, &AttackBy, Has<Ranged>), (With<Piercing>, Without<Fumbled>)>,
    mut characters: Query<&mut Character>,
    mut rng: ResMut<GameRng>,
) {
    for (mut tally, damage, by, ranged) in &mut blows {
        let Ok(mut character) = characters.get_mut(by.0) else { continue };
        if damage.0 == 0 {
            continue;
        }
        let skills: &[Skill] = if !ranged {
            &[Skill::CriticalStrike, Skill::PowerStrike]
        } else if character.has_trait(Trait::ThrownCriticals) {
            &[Skill::EagleEye, Skill::CriticalStrike]
        } else {
            &[Skill::EagleEye]
        };
        for &skill in skills {
            if character.skill_available(skill) && character.practise(skill, 1, &mut rng) {
                tally.improved.push((by.0, skill));
            }
        }
    }
}

/// A close blow that connected, even for nothing, sets off its victim's
/// Razor Cloak: a charge's roll, cut by the attacker's reduction, strikes
/// the attacker back as a deed of the attack's, reported on its own.
fn riposte(
    mut commands: Commands,
    blows: Query<(Entity, &Faces, &Range, &AttackBy), With<Piercing>>,
    mut vitals: Query<&mut Vitals>,
    reductions: Query<&Reduction>,
    content: If<Res<Catalogs>>,
    definitions: Res<Definitions>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    for (attack, faces, range, by) in &blows {
        if range.0 >= Reach::Long {
            continue;
        }
        let Ok(mut defender) = vitals.get_mut(faces.0) else { continue };
        let Some(charge) = defender.enchantments.spend_charge(SpellId::RAZOR_CLOAK) else {
            continue;
        };
        let strength = Strength { power: charge.power, boost: 0, against: 0 };
        let dice = content.spells[SpellId::RAZOR_CLOAK].dice;
        let rolled = crate::game::hits::prototypes::magnitude(&mut rng, dice, strength);
        let reduction = reductions.get(by.0).map_or(0, |reduction| reduction.0);
        let amount = attack::reduce(rolled as i32, reduction);
        if amount == 0 {
            news.write(CombatNews::Result { attacker: faces.0, target: by.0, hits: 1, damage: 0 });
            continue;
        }
        let answer = commands
            .spawn((
                Deed::new(Act::Fight),
                Uses(definitions.blow()),
                Strength::default(),
                WeaponEffects { effects: AttackEffects::default(), by_monster: false },
                BlowDamage(amount),
                PartOf(attack),
                Alone { by: faces.0, victim: by.0 },
            ))
            .id();
        reach(&mut commands, answer, &[(by.0, None)]);
    }
}

/// A blow counts toward its attack's summary, but a fumble's, which reports
/// itself; the swing is done.
fn count_hit(
    mut commands: Commands,
    mut blows: Query<(Entity, &mut Tally, &AttackBy, &Faces, &Damage, Has<Fumbled>), With<Piercing>>,
    mut news: MessageWriter<CombatNews>,
) {
    for (attack, mut tally, by, faces, damage, fumbled) in &mut blows {
        if !fumbled {
            tally.hits += 1;
        } else if damage.0 == 0 {
            news.write(CombatNews::Result { attacker: by.0, target: faces.0, hits: 1, damage: 0 });
        }
        commands.entity(attack).insert(SwingDone);
    }
}

/// A shot scatters by its aim from the attacker's eye and lands where it
/// strikes its target's body (retail `ScatterMissileAimPoint`). A Ninja's
/// throw gets through whatever its roll.
fn shoot(
    mut commands: Commands,
    shots: Query<(Entity, &AttackBy, &AimsAt, &Score, &Range, Has<SureThrow>), With<Aiming>>,
    bodies: Query<(&Vitals, &Footing)>,
    mut rng: ResMut<GameRng>,
) {
    for (shot, by, aims, score, range, sure_throw) in &shots {
        if range.0 < Reach::Long {
            continue;
        }
        let (Ok((vitals, from)), Ok((_, target))) = (bodies.get(by.0), bodies.get(aims.0)) else {
            continue;
        };
        let blind = vitals.conditions.has(Condition::Blind);
        let aimed = attack::scatter(&mut rng, from.eye, target.centre, score.0, blind);
        let mut shot = commands.entity(shot);
        if !attack::strikes(target.centre, aimed, target.radius, target.half_height) {
            shot.insert(SwingDone);
            continue;
        }
        shot.insert((Landed { roll: rng.below(100) + 1 }, Sure));
        if sure_throw {
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
    mut swings: Query<(Entity, &AttackBy, &AimsAt, &Range, &FumbleOdds, &mut Surprise), With<Aiming>>,
    bodies: Query<(&Vitals, &Footing)>,
    battle: Single<Option<&Combatants>, With<Battle>>,
    (formation, roster): (Single<&Formation>, Single<&PartyRoster>),
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    let combatants = battle.map_or(&[][..], Combatants::entities);
    for (swing, by, aims, range, odds, mut surprise) in &mut swings {
        let Ok((_, from)) = bodies.get(by.0) else { continue };
        let bystanders: Vec<Entity> = everyone(&roster, combatants)
            .filter(|&other| other != aims.0)
            .filter(|&other| {
                bodies.get(other).is_ok_and(|(vitals, there)| {
                    !vitals.is_dead()
                        && (other == by.0 || reaches(&formation, (by.0, from), (other, there), range.0))
                })
            })
            .collect();
        let fumble = attack::fumble_chance(bystanders.len(), odds.0);
        let mut swing = commands.entity(swing);
        match attack::swing(&mut rng, bystanders.len(), fumble) {
            Aim::Missed => {
                swing.insert(SwingDone);
            }
            Aim::Reaches { roll, sure } => {
                swing.insert(Landed { roll });
                if sure {
                    swing.insert((Sure, Through));
                }
            }
            Aim::Fumbled { bystander, roll } => {
                *surprise = Surprise::default();
                let victim = bystanders[bystander];
                swing.insert((Landed { roll }, Sure, Fumbled, Faces(victim)));
                news.write(CombatNews::Fumble { attacker: by.0, victim });
            }
        }
    }
}

/// A guard may take a swing that landed on its ward (retail
/// `ResolveGuardianInterception`): each guard of the victim still able
/// (alive, not blinded or worse, within its first attack's reach, and
/// under its attacks a round in blows taken) rolls its protection against
/// the attacker's, raised for an unaware or natural strike, within 5 and 95
/// percent; one that succeeds, picked at random, takes the blow, tiring by
/// 2 to 4, and the attack's surprise is spent.
fn intercept(
    mut commands: Commands,
    mut swings: Query<(Entity, &AttackBy, &Faces, &mut Surprise), (With<Aiming>, With<Landed>)>,
    wards: Query<(&Guards, &Vitals, &Footing)>,
    protections: Query<&Protection>,
    mut guards: Query<(&Monster, &mut Vitals, &Footing, &mut Interceptions), Without<Guards>>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    for (swing, by, faces, mut surprise) in &mut swings {
        let ward = faces.0;
        let Ok((guarding, ward_vitals, ward_at)) = wards.get(ward) else { continue };
        let Ok(pressing) = protections.get(by.0) else { continue };
        if ward_vitals.is_dead() {
            continue;
        }
        let pressing =
            pressing.0 + 10 * i32::from(surprise.unaware) + 10 * i32::from(surprise.natural);
        let mut stepping = Vec::new();
        for &guard in guarding.entities() {
            let (Ok((monster, vitals, at, taken)), Ok(protection)) =
                (guards.get(guard), protections.get(guard))
            else {
                continue;
            };
            let kind = &content.species[monster.species];
            let Some(first) = kind.attacks.first() else { continue };
            if !(vitals.can_guard()
                && at.gap(ward_at) <= first.reach.metres()
                && taken.0 < u32::from(kind.attacks_per_round))
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
        taken.0 += 1;
        vitals.tire(rng.below(3) + 2);
        *surprise = Surprise::default();
        commands.entity(swing).insert(Faces(guard));
        news.write(CombatNews::Intercepts { guard, ward });
    }
}

/// Where a landed blow strikes: by the struck body's own chances, or on a
/// member by the attacker's where its species has them, else the party's.
fn locate(
    mut commands: Commands,
    swings: Query<(Entity, &Faces, Option<&PartyHits>), (With<Aiming>, With<Landed>)>,
    tables: Query<&HitLocations>,
    mut rng: ResMut<GameRng>,
) {
    for (swing, faces, party_hits) in &swings {
        let at = match (tables.get(faces.0), party_hits) {
            (Ok(table), _) => attack::locate(&mut rng, &table.0, attack::MONSTER_BODY),
            (Err(_), Some(hits)) => {
                attack::locate(&mut rng, hits.0.as_slice(), BodyPart::Body as usize)
            }
            (Err(_), None) => {
                attack::locate(&mut rng, &attack::MEMBER_LOCATIONS, BodyPart::Body as usize)
            }
        };
        commands.entity(swing).insert(at);
    }
}

/// A landed swing whose roll beats its aim is dodged, unless it was sure;
/// one that was not goes on to pierce, a launcher's by a mode its
/// ammunition strikes with.
fn dodge(
    mut commands: Commands,
    swings: Query<(Entity, &Score, &Landed, &Mode, Has<Sure>, Option<&Loaded>), With<Aiming>>,
    mut rng: ResMut<GameRng>,
) {
    for (swing, score, landed, mode, sure, loaded) in &swings {
        let mut swing = commands.entity(swing);
        if !sure && score.0 < landed.roll as i32 {
            swing.insert(SwingDone);
            continue;
        }
        let piercing = loaded.map_or(mode.0, |loaded| attack::choose_mode(&mut rng, loaded.0));
        swing.remove::<(Aiming, FumbleOdds)>().insert((Piercing(piercing), Score(0)));
    }
}

/// A member's swing costs stamina by its weight, twice in a berserk fury,
/// and twice more when diseased, scaled by their load.
fn tire_members(
    swings: Query<(&AttackBy, &Mode, Option<&Weight>, Has<Berserk>), (With<SwingDone>, With<Wielded>)>,
    mut members: Query<(&mut Vitals, &Burden)>,
    mut rng: ResMut<GameRng>,
) {
    for (by, mode, weight, berserk) in &swings {
        let (mut vitals, burden) = members.get_mut(by.0).unwrap();
        let diseased = vitals.conditions.has(Condition::Diseased);
        let cost = attack::swing_stamina(&mut rng, mode.0, weight.map(|w| w.0), berserk, diseased);
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
    swings: Query<(Entity, &AttackBy, &Wielded, Has<Loaded>), (With<SwingDone>, With<Ranged>)>,
    mut members: Query<(&mut Carried, &mut PartyTurn)>,
    content: If<Res<Catalogs>>,
) {
    for (attack, by, wielded, loaded) in &swings {
        let (mut carried, mut turn) = members.get_mut(by.0).unwrap();
        let slot = if loaded { wielded.0.other() } else { wielded.0 };
        let Some(stack) = carried.worn(slot.equip()) else { continue };
        let (id, definition) = (stack.id, &content.items[stack.item]);
        let thrown = definition.class == crate::content::items::EquipClass::ThrownWeapon;
        if !loaded && (definition.spell.is_some() || (thrown && definition.never_depletes)) {
            continue;
        }
        carried.consume_use(id, definition, definition.keeps_empty());
        if carried.worn(slot.equip()).is_none_or(|stack| stack.remaining_uses() == Some(0)) {
            commands.entity(attack).insert(Spent);
            turn.attacks[wielded.0] = 0;
        }
    }
}

/// The swing is over: it counts against the attack's swings, and what it
/// left on the attack goes.
fn finish_swing(mut commands: Commands, mut attacks: Query<(Entity, &mut SwingsLeft), With<SwingDone>>) {
    for (attack, mut swings) in &mut attacks {
        swings.0 -= 1;
        let mut attack = commands.entity(attack);
        attack.remove::<(Aiming, Piercing, Score, FumbleOdds, Landed, Sure, Through, Fumbled)>();
        attack.remove::<(Location, Deflected, DiceCount, Damage, Chances, SwingDone, Faces)>();
        if swings.0 == 0 {
            attack.insert(Spent);
        }
    }
}

/// A member's spent attack reports once its blows have landed, and their
/// slot due next and its phase follow (retail
/// `ComputeCharacterActionPhase`): the other slot while it has attacks,
/// unless it has fewer and less initiative; spread over the rest of the
/// round, a switch costing the difference in the slots' initiative.
fn end_member_attacks(
    mut commands: Commands,
    mut attacks: Query<(Entity, &mut Tally, &AttackBy, Option<&AimsAt>), (With<Spent>, With<Wielded>, Without<Parts>)>,
    mut members: Query<(&mut PartyTurn, &Character, &Vitals, &Carried)>,
    mut battle: Single<&mut Battle>,
    content: If<Res<Catalogs>>,
    mut news: MessageWriter<CombatNews>,
) {
    let step = step(&battle);
    for (attack, mut tally, by, aims) in &mut attacks {
        tally.report(&mut battle, by.0, aims.map(|aims| aims.0), &mut news);
        commands.entity(attack).despawn();
        let (mut turn, character, vitals, carried) = members.get_mut(by.0).unwrap();
        let stats = character.stats(&vitals.enchantments);
        let grip = Grip::of(carried, &content.items);
        let initiative = |slot| grip.initiative(&stats, slot);
        let current = turn.due;
        let other = current.other();
        let (now, then) = (turn.attacks[current], turn.attacks[other]);
        let chosen = if then != 0 && !(then < now && initiative(other) < initiative(current)) {
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
                let switch =
                    if chosen != current { initiative(current) - initiative(chosen) } else { 0 };
                turn.due = chosen;
                turn.phase = (phase as i32 + switch).clamp(step as i32, 100) as u32;
            }
            None => turn.end(),
        }
    }
}

/// A monster's spent attack reports once its blows have landed, and its
/// next attack comes later in the round if it has one left.
fn end_monster_attacks(
    mut commands: Commands,
    mut attacks: Query<(Entity, &mut Tally, &AttackBy, Option<&AimsAt>, &Continues), (With<Spent>, Without<Parts>)>,
    mut turns: Query<&mut MonsterTurn>,
    mut battle: Single<&mut Battle>,
    mut news: MessageWriter<CombatNews>,
) {
    for (attack, mut tally, by, aims, continues) in &mut attacks {
        tally.report(&mut battle, by.0, aims.map(|aims| aims.0), &mut news);
        commands.entity(attack).despawn();
        let mut turn = turns.get_mut(by.0).unwrap();
        turn.phase = super::next_phase(continues.phase, continues.attacks_left);
        turn.attacks_left = continues.attacks_left;
    }
}
