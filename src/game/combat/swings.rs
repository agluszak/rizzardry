//! Weapon attacks in a fight, swing by swing (retail `StartCharacterAttack`,
//! `ResolveCharacterAttack`, `StartMonsterAttack`, `ContinueMonsterAttack`).
//!
//! An attack is an entity of its own while its swings last: an [`Attack`]
//! made by its attacker ([`AttackBy`]) at its target ([`AimsAt`]), with the
//! hand a member swings ([`Wielding`]) or the attack a monster makes
//! ([`Striking`]). Each swing is a hit of the attack that [`Strikes`] its
//! victim, and goes through the [`Swings`] stages: where it lands, whether a
//! guard takes it, whether it gets through, what it does, what the blow
//! teaches and what Razor Cloak answers, and what it cost. [`run_swings`]
//! runs the stages until every attack is over, within the attacker's step,
//! so each swing sees what the last one did.
//!
//! Retail's voices on a miss, a kill or a fumble draw from the same random
//! numbers as the rules; here they draw nothing.

use avian3d::prelude::Position;
use bevy::ecs::query::QueryData;
use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;
use enumset::EnumSet;

use super::approach::{self, CombatMove};
use super::attack::{self, Aim, Defence, Hand, Surprise};
use super::round::{Acting, Guards, Interceptions, MonsterTurn, PartyTurn, Spotted};
use super::{Announcement, Battle, CombatNews, Combatants, PartyPace, RoundAction, RoundPhase};
use crate::content::characters::{Attribute, Skill, Trait};
use crate::content::items::{AttackEffect, AttackEffects, AttackMode, ItemCatalog};
use crate::content::species::{MonsterCatalog, MonsterKind, Reach};
use crate::game::characters::{Character, Condition};
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::factions::Factions;
use crate::game::hits::body::{Body, BodyItem, BodyReadOnlyItem, wound};
use crate::game::hits::prototypes::{Definitions, Uses, WeaponEffects};
use crate::game::hits::spells::reach;
use crate::game::hits::{Act, Deed, Effect, HitOf, Strikes};
use crate::game::magic::Strength;
use crate::game::magic::enchantments::{Enchantments, PartyEffects};
use crate::game::party::DamageApplied;
use crate::game::party::body::{self as party_body, PartyBody};
use crate::game::party::formation::{self, Formation, Row};
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::time::GameClock;
use crate::game::world::ActorPose;
use crate::game::world::hostility::{self, Allegiance, Provocation, Stance};
use crate::game::world::identify::MonsterGroup;
use crate::game::world::level::{Level, LevelOwned};
use crate::game::world::navigation::{Walker, feet};
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::Monster;

/// An attack under way: how it strikes, the swings it has left, the
/// surprise it still has, and what its blows have done so far. `over` once
/// it can go no further.
#[derive(Component, Debug)]
pub(super) struct Attack {
    mode: AttackMode,
    swings_left: u32,
    surprise: Surprise,
    over: bool,
    hits: u32,
    damage: u32,
    /// Monsters the party killed, with their experience.
    kills: Vec<(Entity, u32)>,
    /// Anyone else who died.
    deaths: Vec<Entity>,
    effects: Vec<Effect>,
    improved: Vec<(Entity, Skill)>,
}

impl Attack {
    fn new(mode: AttackMode, swings: u32, surprise: Surprise) -> Self {
        Self {
            mode,
            swings_left: swings,
            surprise,
            over: false,
            hits: 0,
            damage: 0,
            kills: Vec::new(),
            deaths: Vec::new(),
            effects: Vec::new(),
            improved: Vec::new(),
        }
    }

    /// Count what a blow did: a monster a member kills is credited to the
    /// party, any other death announced. The damage dealt.
    fn struck(&mut self, struck: &Effect, by_party: bool, victim_is_monster: bool) -> u32 {
        let Effect::Damaged { victim, outcome } = *struck else { return 0 };
        if outcome.died {
            if by_party && victim_is_monster {
                self.kills.push((victim, outcome.experience));
            } else {
                self.deaths.push(victim);
            }
        }
        outcome.damage
    }

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
        let mut effects = std::mem::take(&mut self.effects);
        effects.retain(|e| !matches!(e, Effect::Damaged { .. }));
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

/// A member's attack: the hand swinging (on `side`, 0 main, 1 off), with
/// the dice its mode strikes with; what a launcher's ammunition pierces
/// by; and whether it is a berserk fury.
#[derive(Component, Debug)]
pub(super) struct Wielding {
    side: usize,
    hand: Hand,
    ammunition: Option<EnumSet<AttackMode>>,
    berserk: bool,
}

/// A monster's attack: which of its species' attacks, the side it fights
/// on, the phase it began at and the attacks its round has left after it.
#[derive(Component, Debug)]
pub(super) struct Striking {
    attack: usize,
    stance: Stance,
    phase: u32,
    attacks_left: u32,
}

/// A monster that settled on fighting this action; `waits` when it may
/// hold back instead.
#[derive(Component, Debug)]
pub(super) struct Fights {
    pub(super) waits: bool,
}

/// A swing that landed on the body it [`Strikes`]: its roll against its
/// `aim`, and how.
#[derive(Component, Clone, Copy, Debug)]
struct Swing {
    aim: i32,
    roll: u32,
    /// It struck whatever its aim: a roll of 5 or under, a fumble or a
    /// missile that flew true.
    sure: bool,
    /// It gets through armour whatever the roll.
    through: bool,
    fumbled: bool,
    /// Close enough for Razor Cloak to answer.
    close: bool,
}

/// A swing that got through: its damage (0 for one turned aside) and what
/// it may do besides, at the attacker's effect `power`.
#[derive(Component, Debug)]
struct Blow {
    damage: u32,
    effects: AttackEffects,
    power: u32,
    by_monster: bool,
}

/// What a blow took off its victim.
#[derive(Component, Clone, Copy, Debug)]
struct Dealt(u32);

/// The stages a swing goes through, run by [`run_swings`].
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct Swings;

/// Run the swing stages until every attack under way is over.
pub(super) fn run_swings(world: &mut World) {
    let mut passes = 0;
    while world.query_filtered::<(), With<Attack>>().iter(world).next().is_some() {
        passes += 1;
        if passes > 64 {
            error!("attacks left unsettled after {passes} passes");
            break;
        }
        world.run_schedule(Swings);
        // What a blow did besides its damage lands before the next swing.
        crate::game::hits::run_hits(world);
    }
}

pub(super) fn stages(app: &mut App) {
    app.init_schedule(Swings).add_systems(
        Swings,
        (
            swing_members,
            swing_monsters,
            intercept_swings,
            pierce_member_swings,
            pierce_monster_swings,
            land_blows,
            train_strikes,
            razor_cloaks,
            count_blows,
            spend_member_swings,
            spend_monster_swings,
        )
            .chain(),
    );
}

/// Where a monster stands in the world: its pose, how it walks, and the
/// level it belongs to.
#[derive(QueryData)]
pub(super) struct Place {
    pose: ActorPose,
    walker: Option<&'static Walker>,
    owner: &'static LevelOwned,
}

impl PlaceItem<'_, '_> {
    /// Its feet on the ground.
    pub(super) fn feet(&self) -> Vec3 {
        feet(self.pose.translation(), self.walker)
    }

    fn translation(&self) -> Vec3 {
        self.pose.translation()
    }
}

/// What a blow meets besides the body: a member defending, a monster
/// advancing, the level a monster belongs to.
#[derive(QueryData)]
pub(super) struct Footing {
    turn: Option<&'static PartyTurn>,
    advancing: Has<CombatMove>,
    owner: Option<&'static LevelOwned>,
}

/// Where the party stands in the fight: its eye and which way it faces.
#[derive(Clone, Copy)]
pub(super) struct Stand {
    pub(super) eye: Vec3,
    pub(super) yaw: f32,
}

impl Stand {
    pub(super) fn of(position: &Position, body: &PartyBody) -> Self {
        Self { eye: PartyBody::eye(position), yaw: body.yaw }
    }

    /// Whether `member`'s place in `formation` reaches a monster at `at`,
    /// `distance` away from the party, with `reach`.
    fn reaches(
        self,
        formation: &Formation,
        member: Entity,
        at: Vec3,
        reach: Reach,
        distance: f32,
    ) -> bool {
        let side = formation::quadrant(self.eye, self.yaw, at);
        formation.reach(member, side, reach).is_some_and(|r| distance <= r.metres())
    }
}

/// How far a monster at `feet` is from the party, body to body.
fn distance(eye: Vec3, monster: &Monster, feet: Vec3) -> f32 {
    crate::game::world::perception::party_distance(eye, monster, feet)
}

/// Where a monster is struck: the middle of its body.
fn body_centre(feet: Vec3, monster: &Monster) -> Vec3 {
    feet + Vec3::Y * monster.body.eye_height * 0.5
}

/// Whether a monster has its back to the party: facing away by 135° or
/// more (retail `IsPartyLookingAwayFrom`).
fn back_to_party(forward: Vec3, at: Vec3, party_eye: Vec3) -> bool {
    let to_party = (party_eye - at).xz();
    let forward = forward.xz();
    to_party != Vec2::ZERO
        && forward != Vec2::ZERO
        && forward.angle_to(to_party).abs() >= std::f32::consts::FRAC_PI_4 * 3.0
}

/// A monster in play: alive, in the played level.
fn in_play(body: &BodyReadOnlyItem, place: &PlaceItem, levels: &Query<(), With<Level>>) -> bool {
    !body.is_dead() && levels.contains(place.owner.0)
}

/// A body's defence as a blow meets it. A member's: what they wear and the
/// spells on them and the party, braced if they defend, distracted while
/// the party runs. A monster's: its species, fatigue and spells, its guard
/// dropped while it advances. None for a monster dead or gone.
fn blow_defence<'s>(
    body: &BodyReadOnlyItem,
    footing: &FootingItem,
    running: bool,
    levels: &Query<(), With<Level>>,
    items: &ItemCatalog,
    party: &Enchantments,
    species: &'s MonsterCatalog,
) -> Option<Defence<'s>> {
    if let Some(character) = &body.character {
        let (carried, burden) = (body.carried.as_ref()?, body.burden?);
        let defending = footing.turn.is_some_and(PartyTurn::defends);
        let stats = character.stats(&body.vitals.enchantments);
        let armour = attack::enchanted_armour(
            &stats,
            carried,
            items,
            burden.load,
            defending,
            &body.vitals.enchantments,
            Some(party),
        );
        return Some(Defence::member(&stats, armour, running, defending));
    }
    let monster = body.monster.as_ref()?;
    let in_play = footing.owner.is_some_and(|owner| levels.contains(owner.0));
    (in_play && !body.is_dead())
        .then(|| Defence::monster(&species[monster.species], body.vitals, footing.advancing))
}

/// A monster's allegiance now: its species, faction, side and Turncoat.
pub(super) fn allegiance(body: &BodyReadOnlyItem, species: &MonsterCatalog) -> Option<Allegiance> {
    let monster = body.monster.as_ref()?;
    Some(Allegiance {
        species: monster.species,
        stance: body.side.0,
        faction: species[monster.species].faction,
        turncoat: body.vitals.conditions.has(Condition::Turncoat),
    })
}

/// How a monster of allegiance `mine` stands toward a body: a fellow
/// monster by both allegiances, a member by whether they have turned.
pub(super) fn toward(
    mine: Allegiance,
    body: &BodyReadOnlyItem,
    species: &MonsterCatalog,
) -> Stance {
    match allegiance(body, species) {
        Some(theirs) => hostility::between(mine, theirs),
        None => {
            hostility::toward_member(mine.stance, body.vitals.conditions.has(Condition::Turncoat))
        }
    }
}

/// The party attacks a monster: its group turns hostile, whatever the
/// blows then do (retail `MakeTargetGroupHostile` as the action starts). A
/// dead monster or one under Turncoat provokes nobody.
fn provoke(
    body: &BodyReadOnlyItem,
    species: &MonsterCatalog,
    groups: &mut Query<&mut MonsterGroup>,
    factions: &Factions,
    now: u64,
) -> bool {
    let Some(monster) = &body.monster else { return false };
    if body.is_dead() || body.vitals.conditions.has(Condition::Turncoat) {
        return false;
    }
    let kind = &species[monster.species];
    if let Some(member) = body.group
        && let Ok(mut group) = groups.get_mut(member.0)
        && hostility::stance(group.set, kind, factions, now, true) != Stance::Hostile
    {
        group.set = Some(hostility::SetStance { stance: Stance::Hostile, at: now });
    }
    true
}

/// Who a member's hand may strike (retail `ChooseMonsterTarget`): among the
/// monsters it reaches on the side the member fights, the least afflicted,
/// then those "in reach" of the party, then the nearest; ties keep the
/// fight's order. A member turned on the party picks at random among their
/// fellows and that monster.
fn member_target(
    rng: &mut GameRng,
    member: Entity,
    hand: &Hand,
    turned: bool,
    combatants: &[Entity],
    roster: &[Entity],
    bodies: &mut Query<Body>,
    places: &Query<Place, With<Monster>>,
    levels: &Query<(), With<Level>>,
    stand: Stand,
    formation: &Formation,
    species: &MonsterCatalog,
) -> Option<Entity> {
    let monster = combatants
        .iter()
        .filter_map(|&entity| {
            let body = bodies.get(entity).ok()?;
            let place = places.get(entity).ok()?;
            let monster = body.monster.as_ref()?;
            let kind = &species[monster.species];
            let distance = distance(stand.eye, monster, place.feet());
            let side = body.side.0;
            let opposed = if turned { side == Stance::Friendly } else { side != Stance::Friendly };
            if kind.npc.is_some()
                || !opposed
                || !in_play(&body, &place, levels)
                || !stand.reaches(formation, member, place.translation(), hand.reach, distance)
            {
                return None;
            }
            let worst = body.vitals.conditions.worst();
            let quadrant = formation::quadrant(stand.eye, stand.yaw, place.translation());
            let in_reach = !kind.attacks.is_empty()
                && (quadrant == Row::Rear
                    || (!formation.occupied(quadrant) && formation.occupied(Row::Center)));
            Some(((worst, !in_reach, distance), entity))
        })
        .min_by(|a, b| a.0.0.cmp(&b.0.0).then(a.0.1.cmp(&b.0.1)).then(a.0.2.total_cmp(&b.0.2)))
        .map(|(_, target)| target);
    if !turned {
        return monster;
    }
    let mut targets: Vec<Entity> = roster
        .iter()
        .copied()
        .filter(|&other| {
            other != member
                && bodies.get(other).is_ok_and(|body| {
                    !body.is_dead() && !body.vitals.conditions.has(Condition::Turncoat)
                })
        })
        .collect();
    targets.extend(monster);
    (!targets.is_empty()).then(|| targets[rng.below(targets.len() as u32) as usize])
}

/// The acting member attacks with the hand due (retail
/// `StartCharacterAttack`): the best target that hand reaches, or the other
/// hand's when it reaches nobody; its mode and swings, a Samurai's
/// Lightning Strike and a Rogue's backstab, the announcement, and the
/// target's group provoked. A member whose hands reach nobody is done for
/// the round.
pub(super) fn member_attacks(
    mut commands: Commands,
    actor: Option<Single<(Entity, &mut PartyTurn), With<Acting>>>,
    battle: Single<(&mut Battle, Option<&Combatants>)>,
    mut bodies: Query<Body>,
    (places, levels): (Query<Place, With<Monster>>, Query<(), With<Level>>),
    (stand, roster, formation, party): (
        Single<(&Position, &PartyBody)>,
        Single<&PartyRoster>,
        Single<&Formation>,
        Single<&PartyEffects>,
    ),
    mut groups: Query<&mut MonsterGroup>,
    (content, factions, clock): (If<Res<Catalogs>>, Res<Factions>, Res<GameClock>),
    mut rng: ResMut<GameRng>,
    (mut news, mut provocations): (MessageWriter<CombatNews>, MessageWriter<Provocation>),
) {
    let Some((member, mut turn)) = actor.map(Single::into_inner) else { return };
    let RoundAction::Attack { berserk, .. } = turn.action else { return };
    commands.entity(member).remove::<Acting>();
    let (mut battle, combatants) = battle.into_inner();
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    let (items, species, now) = (&content.items, &content.species, clock.ms);
    let (stand, roster) = (Stand::of(stand.0, stand.1), roster.iter().collect::<Vec<_>>());
    let rng = &mut *rng;
    let attacker = bodies.get(member).unwrap();
    let character = attacker.character.unwrap().clone();
    let turned = attacker.vitals.conditions.has(Condition::Turncoat);
    let slowed = attacker.vitals.conditions.has(Condition::Slowed);
    let mut hands = attack::hands(
        &character.stats(&attacker.vitals.enchantments),
        attacker.carried.unwrap(),
        items,
        attacker.burden.unwrap().load,
    );
    // The party's blessings guide every hand.
    for hand in &mut hands.hands {
        hand.to_hit += party.0.hit_bonus();
    }
    // The hand due attacks if it still can; one that cannot (out of shots,
    // or nobody in its reach) gives up its own attacks and the other takes
    // over (retail `StartCharacterAttack`, `ComputeCharacterActionPhase`).
    let mut due = None;
    for side in [turn.hands.current, 1 - turn.hands.current] {
        let hand = &hands.hands[side];
        let target = (hand.in_play && turn.hands.left[side] > 0)
            .then(|| {
                member_target(
                    rng,
                    member,
                    hand,
                    turned,
                    combatants,
                    &roster,
                    &mut bodies,
                    &places,
                    &levels,
                    stand,
                    &formation,
                    species,
                )
            })
            .flatten();
        match target {
            Some(target) => {
                due = Some((side, target));
                break;
            }
            None => turn.hands.left[side] = 0,
        }
    }
    let Some((side, target)) = due else {
        turn.end();
        return;
    };
    battle.wait = Timer::from_seconds(super::ACTION_SECONDS, TimerMode::Once);
    battle.attacked(true);
    turn.hands.current = side;
    turn.hands.left[side] -= 1;
    let mut hand = hands.hands[side].clone();
    // Retail draws the attack's mode, then its swings.
    let mode = if berserk { AttackMode::Berserk } else { attack::choose_mode(rng, hand.modes) };
    let mut swings = rng.below(hand.swings) + 1;
    swings = (swings as i32 + hand.extra_swings).max(1) as u32;
    // A Samurai's Lightning Strike, readied as the round began.
    if turn.hands.lightning && side == 0 {
        if hand.weapon_skill == Skill::Sword && !slowed {
            let level = u32::from(character.profession_levels[character.profession]);
            swings += if rng.below(11) + level < 20 { 3 } else { 4 };
        }
        turn.hands.lightning = false;
    }
    // A punch strikes with the off hand's fist, a kick as the main hand's.
    hand.dice = match mode {
        AttackMode::Punch => hands.hands[1].dice,
        AttackMode::Kick => hands.hands[0].dice,
        _ => hand.dice,
    };
    // A Rogue's sword or dagger at arm's length strikes where it hurts.
    let backstab = character.has_trait(Trait::Backstab)
        && hand.reach <= Reach::Touch
        && matches!(hand.weapon_skill, Skill::Sword | Skill::Dagger);
    let mut surprise = Surprise { unaware: false, natural: backstab };
    if hand.reach <= Reach::Short
        && let Ok(place) = places.get(target)
    {
        surprise.unaware = back_to_party(place.pose.forward(), place.translation(), stand.eye);
    }
    news.write(CombatNews::Attack {
        attacker: member,
        target,
        how: Announcement::Mode(mode),
        swings,
        surprise,
    });
    if !turned && provoke(&bodies.get(target).unwrap(), species, &mut groups, &factions, now) {
        provocations.write(Provocation(target));
    }
    let ammunition = hand.paired.then(|| hands.hands[1 - side].modes);
    commands.spawn((
        Attack::new(mode, swings, surprise),
        AttackBy(member),
        AimsAt(target),
        Wielding { side, hand, ammunition, berserk },
    ));
}

/// Every (attack, mode, target) a monster reaches and may attack, equally
/// likely: members, then the other monsters in the fight, those it stands
/// `wanted` toward from allegiance `mine` (retail `BuildMonsterActionQueue`'s
/// `MonsterVsCharDisposition` and `MonsterHostility` tests). A member's
/// Stealth may keep them out of its eye this time (three quarters of it, in
/// percent), and one that stayed out of it may learn from it.
fn reachable_targets(
    rng: &mut GameRng,
    entity: Entity,
    mine: Allegiance,
    wanted: Stance,
    combatants: &[Entity],
    roster: &[Entity],
    bodies: &mut Query<Body>,
    places: &Query<Place, With<Monster>>,
    levels: &Query<(), With<Level>>,
    obstacles: &WorldObstacles,
    stand: Stand,
    formation: &Formation,
    species: &MonsterCatalog,
    news: &mut MessageWriter<CombatNews>,
) -> Vec<(usize, AttackMode, Entity)> {
    let me = bodies.get(entity).unwrap();
    let monster = me.monster.unwrap().clone();
    let kind = &species[monster.species];
    let place = places.get(entity).unwrap();
    let (here, at) = (place.feet(), place.translation());
    let reach_of_party = distance(stand.eye, &monster, here);
    let eye = monster.eye(here);
    let mut unseen = Vec::new();
    for &member in roster {
        let body = bodies.get(member).unwrap();
        let stealth = body.character.as_ref().unwrap().skill(Skill::Stealth);
        if rng.below(100) < stealth * 75 / 100 {
            unseen.push(member);
        }
    }
    let mut candidates = Vec::new();
    let mut evaded = Vec::new();
    for (a, attack) in kind.attacks.iter().enumerate() {
        for &member in roster {
            let body = bodies.get(member).unwrap();
            if body.is_dead()
                || toward(mine, &body, species) != wanted
                || !stand.reaches(formation, member, at, attack.reach, reach_of_party)
            {
                continue;
            }
            if unseen.contains(&member) {
                if !evaded.contains(&member) {
                    evaded.push(member);
                }
                continue;
            }
            candidates.extend(attack.modes.iter().map(|mode| (a, mode, member)));
        }
        for &other in combatants {
            let (Ok(body), Ok(there)) = (bodies.get(other), places.get(other)) else {
                continue;
            };
            let theirs = body.monster.unwrap();
            let feet = there.feet();
            if other != entity
                && in_play(&body, &there, levels)
                && obstacles.clear(eye, body_centre(feet, theirs))
                && toward(mine, &body, species) == wanted
                && monster.gap(here, theirs, feet) <= attack.reach.metres()
            {
                candidates.extend(attack.modes.iter().map(|mode| (a, mode, other)));
            }
        }
    }
    for member in evaded {
        let mut body = bodies.get_mut(member).unwrap();
        if rng.below(2) == 0 && body.character.as_mut().unwrap().practise(Skill::Stealth, 1, rng) {
            news.write(CombatNews::Improved { member, skill: Skill::Stealth });
        }
    }
    candidates
}

/// Whether a member notices a monster coming at them in time to turn
/// (retail `CharacterNoticesAttacker`): only one awake, unblinded and
/// defending; the first time each round always, then by senses, 25 harder
/// each time.
fn notices(
    body: &BodyReadOnlyItem,
    defending: bool,
    attempts: &mut u32,
    rng: &mut GameRng,
) -> bool {
    let vitals = &body.vitals;
    let alert = !vitals.is_dead()
        && vitals.stamina.current > 0
        && vitals.conditions.worst().is_none_or(|c| c < Condition::Webbed)
        && !vitals.conditions.has(Condition::Blind);
    if !alert || !defending {
        return false;
    }
    if *attempts == 0 {
        *attempts = 1;
        return true;
    }
    // Retail compares unsigned: once senses fall short, noticing is certain.
    let senses =
        body.character.as_ref().unwrap().stats(&vitals.enchantments).attribute(Attribute::Senses)
            as i32;
    let chance = (senses - *attempts as i32 * 25) as u32;
    *attempts += 1;
    rng.below(100) < chance
}

/// A monster that settled on fighting attacks (retail `StartMonsterAttack`):
/// one of its attacks and modes at someone it may attack and reaches, all
/// equally likely, a wary one's wait among them. A member not facing it may
/// turn in time, or be struck from behind. With nobody to attack it closes
/// in on a foe (a friendly monster on the nearest monster it may attack, a
/// hostile one on the party) and acts again later in the round.
pub(super) fn monster_attacks(
    mut commands: Commands,
    fighters: Query<(Entity, &Fights, &mut MonsterTurn)>,
    battle: Single<(&mut Battle, Option<&Combatants>)>,
    (party_turns, mut spotted): (Query<&PartyTurn>, Query<&mut Spotted>),
    mut bodies: Query<Body>,
    (places, levels, obstacles): (
        Query<Place, With<Monster>>,
        Query<(), With<Level>>,
        WorldObstacles,
    ),
    (stand, roster, mut formation): (
        Single<(&Position, &PartyBody)>,
        Single<&PartyRoster>,
        Single<&mut Formation>,
    ),
    groups: Query<&MonsterGroup>,
    (content, factions, clock): (If<Res<Catalogs>>, Res<Factions>, Res<GameClock>),
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    let (mut battle, combatants) = battle.into_inner();
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    let (species, now) = (&content.species, clock.ms);
    let (stand, roster) = (Stand::of(stand.0, stand.1), roster.iter().collect::<Vec<_>>());
    let _sides = (&groups, &*factions, now);
    let rng = &mut *rng;
    let step = match battle.phase {
        RoundPhase::Executing(step) => step,
        _ => 1,
    };
    let mut fighters = fighters;
    for (monster, fights, mut turn) in &mut fighters {
        // Fighting is its action: it holds the fight for the action's pause.
        commands.entity(monster).remove::<(
            Fights,
            super::monster_turn::Decided,
            super::monster_turn::Resumes,
            Acting,
        )>();
        battle.wait = Timer::from_seconds(super::ACTION_SECONDS, TimerMode::Once);
        let me = bodies.get(monster).unwrap();
        let mine = allegiance(&me, species).unwrap();
        let kind = &species[me.monster.as_ref().unwrap().species];
        let wanted = if turn.berserk { Stance::Friendly } else { Stance::Hostile };
        let candidates = reachable_targets(
            rng,
            monster,
            mine,
            wanted,
            combatants,
            &roster,
            &mut bodies,
            &places,
            &levels,
            &obstacles,
            stand,
            &formation,
            species,
            &mut news,
        );
        let (phase, attacks_left) = (turn.phase, turn.attacks_left);
        turn.end();
        // A wary monster's wait comes first among its choices.
        let waits = usize::from(fights.waits);
        if candidates.len() + waits == 0 {
            if !turn.moved {
                close_in(
                    &mut commands,
                    monster,
                    &mut turn,
                    step,
                    attacks_left,
                    mine,
                    wanted,
                    combatants,
                    &mut bodies,
                    &places,
                    stand,
                    species,
                    &mut news,
                );
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
        let attack = &kind.attacks[a];
        // Retail draws for the mode again, from just the one chosen.
        let mode = attack::choose_mode(rng, EnumSet::only(mode));
        let swings = rng.below(u32::from(kind.swings_per_round.max(1))) + 1;
        // A natural ambusher strikes from hiding at arm's length.
        let mut surprise = Surprise {
            unaware: false,
            natural: kind.kind == MonsterKind::Rogue && attack.reach <= Reach::Touch,
        };
        // A member not facing it may turn in time, or be struck from behind.
        let position = places.get(monster).unwrap().translation();
        if attack.reach <= Reach::Short
            && let Ok(body) = bodies.get(target)
            && body.character.is_some()
        {
            let side = formation::quadrant(stand.eye, stand.yaw, position);
            let facing = formation.facing(target);
            if facing.is_some_and(|f| f != side) {
                let defending = party_turns.get(target).is_ok_and(PartyTurn::defends);
                let attempts = &mut spotted.get_mut(target).unwrap().0;
                if notices(&body, defending, attempts, rng) {
                    formation.turn(target, side);
                } else {
                    surprise.unaware = facing.is_some_and(|f| f.opposite() == Some(side));
                }
            }
        }
        news.write(CombatNews::Attack {
            attacker: monster,
            target,
            how: Announcement::Monster { word: attack.word, mode, named: attack.named() },
            swings,
            surprise,
        });
        commands.spawn((
            Attack::new(mode, swings, surprise),
            AttackBy(monster),
            AimsAt(target),
            Striking {
                attack: a,
                stance: mine.stance,
                phase,
                attacks_left: attacks_left.max(1) - 1,
            },
        ));
    }
}

/// A monster with nobody to attack closes in on a foe: a friendly one on
/// the nearest monster it may attack (charging when hurt), a hostile one on
/// the party; a neutral one, or one that cannot walk, stays. One that moves
/// acts again later in the round with the attacks it had.
fn close_in(
    commands: &mut Commands,
    monster: Entity,
    turn: &mut MonsterTurn,
    step: u32,
    attacks_left: u32,
    mine: Allegiance,
    wanted: Stance,
    combatants: &[Entity],
    bodies: &mut Query<Body>,
    places: &Query<Place, With<Monster>>,
    stand: Stand,
    species: &MonsterCatalog,
    news: &mut MessageWriter<CombatNews>,
) {
    let place = places.get(monster).unwrap();
    let here = place.translation();
    let walks = place.walker.is_some_and(|walker| walker.speed > 0.0);
    let move_to = if mine.stance == Stance::Friendly {
        let target = combatants
            .iter()
            .filter_map(|&entity| {
                let body = bodies.get(entity).ok()?;
                let at = places.get(entity).ok()?.translation();
                (entity != monster && !body.is_dead() && toward(mine, &body, species) == wanted)
                    .then_some((here.distance_squared(at), entity))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, entity)| entity);
        let me = bodies.get(monster).unwrap();
        target.filter(|_| walks).map(|target| {
            let charge = me.vitals.charges();
            let kind = &species[me.monster.as_ref().unwrap().species];
            let metres = approach::move_metres(me.vitals, kind, charge);
            (approach::MoveBudget { metres, charge }, approach::MoveGoal::Attack(target))
        })
    } else if mine.stance == Stance::Hostile {
        let me = bodies.get(monster).unwrap();
        approach::move_budget(
            me.monster.as_ref().unwrap(),
            me.vitals,
            here,
            place.walker,
            stand.eye,
            species,
        )
        .map(|budget| (budget, approach::MoveGoal::Party))
    } else {
        None
    };
    let Some((budget, goal)) = move_to else { return };
    turn.moved = true;
    turn.phase = (step * 2).min(100);
    turn.attacks_left = attacks_left;
    commands.entity(monster).insert(CombatMove { budget: budget.metres, moved: 0.0, goal });
    news.write(CombatNews::Moves { monster, charge: budget.charge });
}

/// Each member's next swing: the attacker still able and the target still
/// there, its aim, and where it lands; a swing that lands becomes a hit on
/// whoever it struck. One that cannot go on ends the attack.
fn swing_members(
    mut commands: Commands,
    mut attacks: Query<(Entity, &mut Attack, &Wielding, &AttackBy, Option<&AimsAt>)>,
    mut bodies: Query<Body>,
    footings: Query<Footing>,
    places: Query<Place, With<Monster>>,
    levels: Query<(), With<Level>>,
    battle: Single<(&Battle, Option<&Combatants>)>,
    stand: Single<(&Position, &PartyBody)>,
    roster: Single<&PartyRoster>,
    formation: Single<&Formation>,
    party: Single<&PartyEffects>,
    content: If<Res<Catalogs>>,
    difficulty: Res<Difficulty>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    let (battle, combatants) = *battle;
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    let (items, species, difficulty) = (&content.items, &content.species, *difficulty);
    let running = battle.pace == Some(PartyPace::Run);
    let stand = Stand::of(stand.0, stand.1);
    for (attack_entity, mut attack, wielding, by, aims) in &mut attacks {
        let member = by.0;
        if attack.over {
            continue;
        }
        let attacker = bodies.get(member).unwrap();
        let (character, vitals) = (attacker.character.unwrap().clone(), attacker.vitals.clone());
        let aimed_at = aims.map(|aims| aims.0).and_then(|target| {
            let body = bodies.get(target).ok()?;
            let footing = footings.get(target).ok()?;
            blow_defence(&body, &footing, running, &levels, items, &party.0, species)
                .map(|defence| (target, defence))
        });
        let (true, Some((target, aimed_at))) = (vitals.can_fight(), aimed_at) else {
            attack.over = true;
            continue;
        };
        let stats = character.stats(&vitals.enchantments);
        let (hand, mode, surprise) = (&wielding.hand, attack.mode, attack.surprise);
        let aim =
            attack::member_aim(&stats, &vitals, hand, mode, surprise, Some(&aimed_at), difficulty);
        let delivery = if hand.reach >= Reach::Long {
            Delivery::Shot {
                from: stand.eye,
                blind: vitals.conditions.has(Condition::Blind),
                through: attack::sure_penetration(&stats, hand),
            }
        } else {
            // Who the swing could go astray onto (retail
            // `BuildCharacterTargetList`): the attacker, the members their
            // hand reaches, and the monsters it reaches but the one aimed at.
            let mut bystanders: Vec<Entity> = roster
                .iter()
                .filter(|&m| bodies.get(m).is_ok_and(|b| !b.is_dead()))
                .filter(|&m| {
                    m == member || hand.reach > Reach::Touch || !formation.screens(member, m)
                })
                .collect();
            for &entity in combatants {
                let (Ok(body), Ok(place)) = (bodies.get(entity), places.get(entity)) else {
                    continue;
                };
                let distance = distance(stand.eye, body.monster.as_ref().unwrap(), place.feet());
                if entity != target
                    && in_play(&body, &place, &levels)
                    && stand.reaches(&formation, member, place.translation(), hand.reach, distance)
                {
                    bystanders.push(entity);
                }
            }
            Delivery::Swing {
                bystanders,
                odds: attack::member_aim(&stats, &vitals, hand, mode, surprise, None, difficulty),
            }
        };
        let close = hand.reach < Reach::Long;
        let landed = land(
            &mut rng,
            target,
            aim,
            delivery,
            &mut attack.surprise,
            stand.eye,
            &mut bodies,
            &places,
        );
        settle_landing(
            &mut commands,
            attack_entity,
            &mut attack,
            member,
            landed,
            aim,
            close,
            &mut news,
        );
    }
}

/// What came of a swing's landing: an error ends the attack, a miss is
/// nothing, and a swing that landed becomes a hit on its victim.
fn settle_landing(
    commands: &mut Commands,
    attack_entity: Entity,
    attack: &mut Attack,
    attacker: Entity,
    landed: Result<Option<Landing>, ()>,
    aim: i32,
    close: bool,
    news: &mut MessageWriter<CombatNews>,
) {
    match landed {
        Err(()) => attack.over = true,
        Ok(None) => {}
        Ok(Some(landing)) => {
            if landing.fumbled {
                news.write(CombatNews::Fumble { attacker, victim: landing.victim });
            }
            commands.spawn((
                HitOf(attack_entity),
                Strikes(landing.victim),
                Swing {
                    aim,
                    roll: landing.roll,
                    sure: landing.sure,
                    through: landing.through,
                    fumbled: landing.fumbled,
                    close,
                },
            ));
        }
    }
}

/// Each monster's next swing, as a member's: the attacker able and the
/// target there, its aim, and where it lands (retail
/// `ContinueMonsterAttack`). A fumble may stop it between swings.
fn swing_monsters(
    mut commands: Commands,
    mut attacks: Query<(Entity, &mut Attack, &Striking, &AttackBy, Option<&AimsAt>)>,
    mut bodies: Query<Body>,
    footings: Query<Footing>,
    places: Query<Place, With<Monster>>,
    levels: Query<(), With<Level>>,
    battle: Single<(&Battle, Option<&Combatants>)>,
    stand: Single<(&Position, &PartyBody)>,
    roster: Single<&PartyRoster>,
    formation: Single<&Formation>,
    party: Single<&PartyEffects>,
    content: If<Res<Catalogs>>,
    difficulty: Res<Difficulty>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    let (battle, combatants) = *battle;
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    let (items, species, difficulty) = (&content.items, &content.species, *difficulty);
    let running = battle.pace == Some(PartyPace::Run);
    let stand = Stand::of(stand.0, stand.1);
    for (attack_entity, mut attack, striking, by, aims) in &mut attacks {
        let entity = by.0;
        if attack.over {
            continue;
        }
        let attacker = bodies.get(entity).unwrap();
        let (monster, vitals) = (attacker.monster.unwrap().clone(), attacker.vitals.clone());
        let kind = &species[monster.species];
        let swung = &kind.attacks[striking.attack];
        let aimed_at = aims.map(|aims| aims.0).and_then(|target| {
            let body = bodies.get(target).ok()?;
            let footing = footings.get(target).ok()?;
            blow_defence(&body, &footing, running, &levels, items, &party.0, species)
                .map(|defence| (target, defence))
        });
        let (true, Some((target, aimed_at))) = (vitals.can_fight(), aimed_at) else {
            attack.over = true;
            continue;
        };
        let (mode, surprise) = (attack.mode, attack.surprise);
        let me = (kind, &vitals, striking.stance);
        let aim = attack::monster_aim(
            me.0,
            me.1,
            me.2,
            swung,
            mode,
            surprise,
            Some(&aimed_at),
            difficulty,
        );
        let place = places.get(entity).unwrap();
        let here = place.feet();
        let delivery = if swung.reach >= Reach::Long {
            Delivery::Shot {
                from: monster.eye(here),
                blind: vitals.conditions.has(Condition::Blind),
                through: false,
            }
        } else {
            // Who the swing could go astray onto (retail
            // `BuildMonsterTargetList`): the members its attack reaches but
            // the one aimed at, itself, and the monsters within its reach.
            let at = place.translation();
            let reach_of_party = distance(stand.eye, &monster, here);
            let mut bystanders: Vec<Entity> = roster
                .iter()
                .filter(|&m| m != target)
                .filter(|&m| bodies.get(m).is_ok_and(|b| !b.is_dead()))
                .filter(|&m| stand.reaches(&formation, m, at, swung.reach, reach_of_party))
                .collect();
            for &other in combatants {
                let (Ok(body), Ok(there)) = (bodies.get(other), places.get(other)) else {
                    continue;
                };
                if other == target || !in_play(&body, &there, &levels) {
                    continue;
                }
                let gap = monster.gap(here, body.monster.as_ref().unwrap(), there.feet());
                if other == entity || gap <= swung.reach.metres() {
                    bystanders.push(other);
                }
            }
            Delivery::Swing {
                bystanders,
                odds: attack::monster_aim(
                    me.0, me.1, me.2, swung, mode, surprise, None, difficulty,
                ),
            }
        };
        let close = swung.reach < Reach::Long;
        let landed = land(
            &mut rng,
            target,
            aim,
            delivery,
            &mut attack.surprise,
            stand.eye,
            &mut bodies,
            &places,
        );
        settle_landing(
            &mut commands,
            attack_entity,
            &mut attack,
            entity,
            landed,
            aim,
            close,
            &mut news,
        );
    }
}

/// A guard may take a swing that landed on its ward (retail
/// `ResolveGuardianInterception`): each guard of the victim still able
/// (alive, not blinded or worse, within its first attack's reach, and
/// under its attacks a round in blows taken) rolls its protection against
/// the attacker's, raised for an unaware or natural strike, within 5 and 95
/// percent; one that succeeds, picked at random, takes the blow, tiring by
/// 2 to 4, and the attack's surprise is spent.
fn intercept_swings(
    mut commands: Commands,
    swings: Query<(Entity, &Strikes, &HitOf), With<Swing>>,
    mut attacks: Query<(&mut Attack, &AttackBy)>,
    mut bodies: Query<Body>,
    places: Query<Place, With<Monster>>,
    guards: Query<&Guards>,
    mut interceptions: Query<&mut Interceptions>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    let (items, species) = (&content.items, &content.species);
    for (swing, strikes, hit_of) in &swings {
        let ward = strikes.0;
        let Ok(guarding) = guards.get(ward) else { continue };
        let (mut attack, by) = attacks.get_mut(hit_of.0).unwrap();
        // The attacker's protection: a member's with their main hand, a
        // monster's by its species.
        let attacker = bodies.get(by.0).unwrap();
        let pressing = match (&attacker.character, &attacker.monster) {
            (Some(character), _) => {
                let stats = character.stats(&attacker.vitals.enchantments);
                let hands = attack::hands(
                    &stats,
                    attacker.carried.unwrap(),
                    items,
                    attacker.burden.unwrap().load,
                );
                attack::member_protection(&stats, attacker.vitals, &hands.hands[0])
            }
            (None, Some(monster)) => {
                attack::monster_protection(&species[monster.species], attacker.vitals)
            }
            (None, None) => continue,
        };
        let surprise = attack.surprise;
        let pressing =
            pressing + 10 * i32::from(surprise.unaware) + 10 * i32::from(surprise.natural);
        let Some((ward_monster, ward_feet)) = bodies.get(ward).ok().and_then(|body| {
            (!body.is_dead()).then_some(())?;
            Some((body.monster?.clone(), places.get(ward).ok()?.feet()))
        }) else {
            continue;
        };
        let mut stepping = Vec::new();
        for &guard in guarding.entities() {
            let (Ok(body), Ok(place)) = (bodies.get(guard), places.get(guard)) else {
                continue;
            };
            let monster = body.monster.as_ref().unwrap();
            let kind = &species[monster.species];
            let Some(first) = kind.attacks.first() else { continue };
            let gap = monster.gap(place.feet(), &ward_monster, ward_feet);
            let taken = interceptions.get(guard).map_or(0, |taken| taken.0);
            if !(body.vitals.can_guard()
                && gap <= first.reach.metres()
                && taken < u32::from(kind.attacks_per_round))
            {
                continue;
            }
            let guarding = attack::monster_protection(kind, body.vitals);
            let chance = (guarding - pressing + 50).clamp(5, 95) as u32;
            if rng.below(100) < chance {
                stepping.push(guard);
            }
        }
        if stepping.is_empty() {
            continue;
        }
        let guard = stepping[rng.below(stepping.len() as u32) as usize];
        interceptions.get_mut(guard).unwrap().0 += 1;
        let tired = rng.below(3) + 2;
        bodies.get_mut(guard).unwrap().vitals.tire(tired);
        attack.surprise = Surprise::default();
        commands.entity(swing).insert(Strikes(guard));
        news.write(CombatNews::Intercepts { guard, ward });
    }
}

/// Whether each member's swing gets through, and with what (retail
/// `ResolveCharacterAttack`): dodged when its roll falls short of its aim,
/// turned aside when it cannot pierce the armour where it struck or a
/// monster's Missile Shield deflects it; otherwise its dice, a slayer's
/// more, cut by the victim's reduction, a fumble's capped. Struck members
/// practise Iron Skin; a blow's chance to kill rises with the attacker's
/// critical strike, and a Monk's to knock out.
fn pierce_member_swings(
    mut commands: Commands,
    swings: Query<(Entity, &Strikes, &HitOf, &Swing)>,
    mut attacks: Query<(&mut Attack, &Wielding, &AttackBy)>,
    mut bodies: Query<Body>,
    footings: Query<Footing>,
    levels: Query<(), With<Level>>,
    battle: Single<&Battle>,
    party: Single<&PartyEffects>,
    content: If<Res<Catalogs>>,
    difficulty: Res<Difficulty>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    let (items, species, difficulty) = (&content.items, &content.species, *difficulty);
    let running = battle.pace == Some(PartyPace::Run);
    let rng = &mut *rng;
    for (swing_entity, strikes, hit_of, swing) in &swings {
        let Ok((mut attack, wielding, by)) = attacks.get_mut(hit_of.0) else { continue };
        let (victim, hand, mode, surprise) =
            (strikes.0, &wielding.hand, attack.mode, attack.surprise);
        let attacker = bodies.get(by.0).unwrap();
        let (character, vitals) = (attacker.character.unwrap().clone(), attacker.vitals.clone());
        let stats = character.stats(&vitals.enchantments);
        let struck = bodies.get(victim).ok().and_then(|body| {
            let footing = footings.get(victim).ok()?;
            blow_defence(&body, &footing, running, &levels, items, &party.0, species)
        });
        let located = struck.map(|struck| {
            let at = struck.locate(rng, None);
            (struck, at)
        });
        let Some((struck, at)) = located.filter(|_| swing.sure || swing.aim >= swing.roll as i32)
        else {
            // Dodged; a fumble still reports its blow.
            if swing.fumbled {
                news.write(CombatNews::Result {
                    attacker: by.0,
                    target: victim,
                    hits: 1,
                    damage: 0,
                });
            }
            commands.entity(swing_entity).despawn();
            continue;
        };
        let glancing =
            Blow { damage: 0, effects: AttackEffects::default(), power: 0, by_monster: false };
        // A launcher pierces by what it shoots.
        let piercing = wielding.ammunition.map_or(mode, |modes| attack::choose_mode(rng, modes));
        let through = attack::member_penetration(
            &stats, &vitals, hand, piercing, surprise, &struck, at, difficulty,
        );
        let mut body = bodies.get_mut(victim).unwrap();
        // Missile Shield turns aside what is shot or thrown.
        if (!swing.through && through < swing.roll as i32)
            || (body.monster.is_some() && body.vitals.enchantments.deflects(mode, rng))
        {
            commands.entity(swing_entity).insert(glancing);
            continue;
        }
        let slaying = struck.species().map_or(0, |kind| {
            hand.slays.iter().flatten().filter(|&&slays| slays == kind.kind).count() as u32
        });
        let backstab = character.scale_by_profession(100.0) as i32;
        let count = attack::dice_count(rng, mode, &struck, surprise, backstab, slaying);
        let rolled = attack::roll_dice(rng, hand.dice, count);
        let mut damage = attack::member_damage(hand, rolled, &struck);
        iron_skin(&mut body, rng, &mut attack.improved);
        if damage == 0 {
            commands.entity(swing_entity).insert(glancing);
            continue;
        }
        if swing.fumbled {
            damage = struck.fumble_cap(damage);
        }
        let mut effects = hand.effects;
        if !swing.fumbled {
            let kill = &mut effects.chances[AttackEffect::Kill];
            *kill = (*kill + attack::critical_chance(rng, &stats, hand)).min(100);
            if character.has_trait(Trait::Knockout) {
                let knockout = &mut effects.chances[AttackEffect::KnockOut];
                *knockout = (*knockout + character.scale_by_profession(5.0) as u32).min(100);
            }
        }
        let power = attack::effect_power(u32::from(character.level()));
        commands.entity(swing_entity).insert(Blow { damage, effects, power, by_monster: false });
    }
}

/// Whether each monster's swing gets through, and with what (retail
/// `ContinueMonsterAttack`): dodged when its roll falls short of its aim or
/// the party's Missile Shield deflects it; glancing when it cannot pierce
/// the armour where it struck; otherwise its attack's dice, cut by the
/// victim's reduction, a fumble's capped.
fn pierce_monster_swings(
    mut commands: Commands,
    swings: Query<(Entity, &Strikes, &HitOf, &Swing)>,
    mut attacks: Query<(&mut Attack, &Striking, &AttackBy)>,
    mut bodies: Query<Body>,
    footings: Query<Footing>,
    levels: Query<(), With<Level>>,
    battle: Single<&Battle>,
    party: Single<&PartyEffects>,
    content: If<Res<Catalogs>>,
    difficulty: Res<Difficulty>,
    mut rng: ResMut<GameRng>,
) {
    let (items, species, difficulty) = (&content.items, &content.species, *difficulty);
    let running = battle.pace == Some(PartyPace::Run);
    let rng = &mut *rng;
    for (swing_entity, strikes, hit_of, swing) in &swings {
        let Ok((mut attack, striking, by)) = attacks.get_mut(hit_of.0) else { continue };
        let (victim, mode, surprise) = (strikes.0, attack.mode, attack.surprise);
        let attacker = bodies.get(by.0).unwrap();
        let (monster, vitals) = (attacker.monster.unwrap().clone(), attacker.vitals.clone());
        let kind = &species[monster.species];
        let swung = &kind.attacks[striking.attack];
        let me = (kind, &vitals, striking.stance);
        let Some((struck, member)) = bodies.get(victim).ok().and_then(|body| {
            let footing = footings.get(victim).ok()?;
            blow_defence(&body, &footing, running, &levels, items, &party.0, species)
                .map(|defence| (defence, body.character.is_some()))
        }) else {
            commands.entity(swing_entity).despawn();
            continue;
        };
        let at = struck.locate(rng, member.then_some(kind));
        // The party's Missile Shield turns aside what is shot or thrown.
        if (!swing.sure && swing.aim < swing.roll as i32) || (member && party.0.deflects(mode, rng))
        {
            commands.entity(swing_entity).despawn();
            continue;
        }
        let through = attack::monster_penetration(
            me.0, me.1, me.2, swung, mode, surprise, &struck, at, difficulty,
        );
        let mut damage = 0;
        if swing.through || through >= swing.roll as i32 {
            let backstab = crate::game::characters::scale_by_level(kind.level, 100.0) as i32;
            let count = attack::dice_count(rng, mode, &struck, surprise, backstab, 0);
            let rolled = attack::roll_dice(rng, swung.damage, count);
            damage = attack::monster_damage(rolled, &struck);
            iron_skin(&mut bodies.get_mut(victim).unwrap(), rng, &mut attack.improved);
            if damage > 0 && swing.fumbled {
                damage = struck.fumble_cap(damage);
            }
        }
        let power = attack::effect_power(u32::from(kind.level));
        commands.entity(swing_entity).insert(Blow {
            damage,
            effects: swung.effects,
            power,
            by_monster: true,
        });
    }
}

/// Each blow lands: its damage wounds its victim, and one that drew blood
/// does what its weapon or attack does besides. What it did is gathered on
/// its attack.
fn land_blows(
    mut commands: Commands,
    blows: Query<(Entity, &Strikes, &HitOf, &Blow)>,
    mut attacks: Query<(&mut Attack, &AttackBy)>,
    mut bodies: Query<Body>,
    members: Query<(), With<Character>>,
    fight: Single<Entity, With<Battle>>,
    content: If<Res<Catalogs>>,
    definitions: Res<Definitions>,
    mut rng: ResMut<GameRng>,
    mut notices: MessageWriter<DamageApplied>,
) {
    let species = &content.species;
    let (fight, rng) = (Some(*fight), &mut *rng);
    for (hit, strikes, hit_of, blow) in &blows {
        let (mut attack, by) = attacks.get_mut(hit_of.0).unwrap();
        let by_party = members.contains(by.0);
        let mut victim = bodies.get_mut(strikes.0).unwrap();
        let monster = victim.monster.is_some();
        let dealt = if blow.damage == 0 {
            0
        } else {
            let hurt =
                wound(&mut victim, blow.damage, rng, species, fight, &mut commands, &mut notices);
            attack.struck(&hurt, by_party, monster)
        };
        // What the weapon does besides, once it drew blood, at the
        // attacker's effect power: an act of its own.
        if dealt > 0 {
            let strength = Strength { power: blow.power, against: blow.power, boost: 0 };
            let weapon = WeaponEffects { effects: blow.effects, by_monster: blow.by_monster };
            let act = commands
                .spawn((Deed::new(Act::Fight), Uses(definitions.blow()), strength, weapon))
                .id();
            reach(&mut commands, act, &[(strikes.0, None)]);
        }
        commands.entity(hit).insert(Dealt(dealt));
    }
}

/// A member's blow that drew blood, not by a fumble, trains their strikes:
/// close blows Critical and Power Strike, ranged ones Eagle Eye, and a
/// thrower's Critical Strike too.
fn train_strikes(
    blows: Query<(&HitOf, &Swing, &Dealt)>,
    mut attacks: Query<(&mut Attack, &Wielding, &AttackBy)>,
    mut characters: Query<&mut Character>,
    mut rng: ResMut<GameRng>,
) {
    for (hit_of, swing, dealt) in &blows {
        let Ok((mut attack, wielding, by)) = attacks.get_mut(hit_of.0) else { continue };
        if swing.fumbled || dealt.0 == 0 {
            continue;
        }
        let mut character = characters.get_mut(by.0).unwrap();
        let skills: &[Skill] = if !wielding.hand.ranged {
            &[Skill::CriticalStrike, Skill::PowerStrike]
        } else if character.has_trait(Trait::ThrownCriticals) {
            &[Skill::EagleEye, Skill::CriticalStrike]
        } else {
            &[Skill::EagleEye]
        };
        for &skill in skills {
            if character.skill_available(skill) && character.practise(skill, 1, &mut rng) {
                attack.improved.push((by.0, skill));
            }
        }
    }
}

/// A close blow that connected, even for nothing, sets off the victim's
/// Razor Cloak: one charge strikes the attacker back, reduced by the
/// attacker's own defence.
fn razor_cloaks(
    mut commands: Commands,
    blows: Query<(&Strikes, &HitOf, &Swing), With<Dealt>>,
    mut attacks: Query<(&mut Attack, &AttackBy)>,
    mut bodies: Query<Body>,
    footings: Query<Footing>,
    levels: Query<(), With<Level>>,
    members: Query<(), With<Character>>,
    fight: Single<Entity, With<Battle>>,
    battle: Single<&Battle>,
    party: Single<&PartyEffects>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
    mut notices: MessageWriter<DamageApplied>,
    mut news: MessageWriter<CombatNews>,
) {
    use crate::content::spells::SpellId;
    let (items, species, spells) = (&content.items, &content.species, &content.spells);
    let running = battle.pace == Some(PartyPace::Run);
    let rng = &mut *rng;
    for (strikes, hit_of, swing) in &blows {
        if !swing.close {
            continue;
        }
        let (defender, (mut attack, by)) = (strikes.0, attacks.get_mut(hit_of.0).unwrap());
        let Ok(mut defending) = bodies.get_mut(defender) else { continue };
        let Some(charge) = defending.vitals.enchantments.spend_charge(SpellId::RAZOR_CLOAK) else {
            continue;
        };
        let rolled = crate::game::hits::prototypes::magnitude(
            rng,
            spells[SpellId::RAZOR_CLOAK].dice,
            crate::game::magic::Strength { power: charge.power, boost: 0, against: 0 },
        );
        let attacker = by.0;
        let footing = footings.get(attacker).unwrap();
        let defence = bodies.get(attacker).ok().and_then(|struck| {
            blow_defence(&struck, &footing, running, &levels, items, &party.0, species)
        });
        let Some(amount) = defence.map(|defence| defence.reduce(rolled as i32)) else {
            continue;
        };
        let mut struck = bodies.get_mut(attacker).unwrap();
        let monster = struck.monster.is_some();
        let hurt =
            wound(&mut struck, amount, rng, species, Some(*fight), &mut commands, &mut notices);
        let damage = attack.struck(&hurt, members.contains(defender), monster);
        news.write(CombatNews::Result { attacker: defender, target: attacker, hits: 1, damage });
    }
}

/// Each blow counts toward its attack's summary, but a fumble's, which
/// reports itself; then the swing is done.
fn count_blows(
    mut commands: Commands,
    blows: Query<(Entity, &Strikes, &HitOf, &Swing, &Dealt)>,
    mut attacks: Query<(&mut Attack, &AttackBy)>,
    mut news: MessageWriter<CombatNews>,
) {
    for (hit, strikes, hit_of, swing, dealt) in &blows {
        let (mut attack, by) = attacks.get_mut(hit_of.0).unwrap();
        if swing.fumbled {
            news.write(CombatNews::Result {
                attacker: by.0,
                target: strikes.0,
                hits: 1,
                damage: dealt.0,
            });
        } else {
            attack.hits += 1;
            attack.damage += dealt.0;
        }
        commands.entity(hit).despawn();
    }
}

/// What each member's swing cost: stamina, twice as much in a berserk
/// fury, and the arrow, bolt, stone, thrown weapon or shot it spent; a hand
/// with nothing left to fire is done for the round. An attack whose swings
/// are spent or that can go no further reports, and the member's next hand
/// and its phase follow.
fn spend_member_swings(
    mut commands: Commands,
    mut attacks: Query<(Entity, &mut Attack, &Wielding, &AttackBy, Option<&AimsAt>)>,
    mut bodies: Query<Body>,
    mut turns: Query<&mut PartyTurn>,
    mut battle: Single<&mut Battle>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    let items = &content.items;
    for (attack_entity, mut attack, wielding, by, aims) in &mut attacks {
        let member = by.0;
        let mut turn = turns.get_mut(member).unwrap();
        let mut body = bodies.get_mut(member).unwrap();
        let hand = &wielding.hand;
        if !attack.over {
            let cost = attack::swing_stamina(
                &mut rng,
                attack.mode,
                hand.weight,
                wielding.berserk,
                body.vitals.conditions.has(Condition::Diseased),
            );
            let slowed = body.vitals.conditions.has(Condition::Slowed);
            let hasted = body.vitals.enchantments.has(crate::content::spells::SpellId::HASTE);
            let load = body.burden.unwrap().load;
            body.vitals.tire(load.scale_fatigue(cost, slowed, hasted));
            let carried = body.carried.as_mut().unwrap();
            if (hand.paired || hand.ranged) && !spend_shot(carried, items, wielding.side, hand) {
                attack.over = true;
                turn.hands.left[wielding.side] = 0;
            }
            attack.swings_left -= 1;
            attack.over |= attack.swings_left == 0;
        }
        if !attack.over {
            continue;
        }
        attack.report(&mut battle, member, aims.map(|aims| aims.0), &mut news);
        commands.entity(attack_entity).despawn();
        // The hand due next, by the hands as they are now.
        let character = body.character.as_ref().unwrap();
        let hands = attack::hands(
            &character.stats(&body.vitals.enchantments),
            body.carried.as_deref().unwrap(),
            items,
            body.burden.unwrap().load,
        );
        let step = match battle.phase {
            RoundPhase::Executing(step) => step,
            _ => 1,
        };
        next_hand(&mut turn, step, &hands);
    }
}

/// A monster's attack ends once its swings are spent or it can go no
/// further: it reports, and its next attack comes later in the round if it
/// has one left.
fn spend_monster_swings(
    mut commands: Commands,
    mut attacks: Query<(Entity, &mut Attack, &Striking, &AttackBy, Option<&AimsAt>)>,
    mut turns: Query<&mut MonsterTurn>,
    mut battle: Single<&mut Battle>,
    mut news: MessageWriter<CombatNews>,
) {
    for (attack_entity, mut attack, striking, by, aims) in &mut attacks {
        if !attack.over {
            attack.swings_left -= 1;
            attack.over = attack.swings_left == 0;
        }
        if !attack.over {
            continue;
        }
        attack.report(&mut battle, by.0, aims.map(|aims| aims.0), &mut news);
        commands.entity(attack_entity).despawn();
        let mut turn = turns.get_mut(by.0).unwrap();
        turn.phase = super::next_phase(striking.phase, striking.attacks_left);
        turn.attacks_left = striking.attacks_left;
    }
}

/// The hand due next and when (retail `ComputeCharacterActionPhase`): the
/// other hand while it has attacks, unless it has fewer and less
/// initiative; spread over the rest of the round, and a switch costs the
/// difference in the hands' initiative.
fn next_hand(turn: &mut PartyTurn, step: u32, hands: &attack::Hands) {
    let current = turn.hands.current;
    let other = 1 - current;
    let (now, then) = (turn.hands.left[current], turn.hands.left[other]);
    let initiative = |hand: usize| hands.hands[hand].initiative;
    let chosen = if then != 0 && !(then < now && initiative(other) < initiative(current)) {
        other
    } else {
        current
    };
    let left = turn.hands.left[chosen];
    let phase = if left == 0 {
        None
    } else if chosen == current || then <= now {
        Some(step + (100 - step) / (left + 1))
    } else {
        Some(step)
    };
    match phase.filter(|&phase| phase <= 100) {
        Some(phase) => {
            let phase = phase as i32
                + if chosen != current { initiative(current) - initiative(chosen) } else { 0 };
            turn.hands.current = chosen;
            turn.phase = phase.clamp(step as i32, 100) as u32;
        }
        None => turn.end(),
    }
}

/// A body struck practises Iron Skin as its reduction is worked out (retail
/// `ApplyCharacterDamageReduction`).
fn iron_skin(victim: &mut BodyItem, rng: &mut GameRng, improved: &mut Vec<(Entity, Skill)>) {
    let member = victim.entity;
    if let Some(character) = victim.character.as_mut()
        && character.skill_available(Skill::IronSkin)
        && character.practise(Skill::IronSkin, 1, rng)
    {
        improved.push((member, Skill::IronSkin));
    }
}
/// How an attack goes at its target: a shot from `from` (blinded, perhaps,
/// and sure to get through armour for some), or a swing among the
/// `bystanders` it may fumble onto at the `odds` it has.
enum Delivery {
    Shot { from: Vec3, blind: bool, through: bool },
    Swing { bystanders: Vec<Entity>, odds: i32 },
}

/// Whoever a swing landed on, and how.
#[derive(Clone, Copy)]
struct Landing {
    victim: Entity,
    roll: u32,
    /// It struck whatever its aim: a roll of 5 or under, a fumble or a
    /// missile that flew true.
    sure: bool,
    /// It gets through armour whatever the roll.
    through: bool,
    fumbled: bool,
}

/// Where a body is struck: a monster's middle, or the party's.
fn struck_body(
    victim: Entity,
    eye: Vec3,
    bodies: &mut Query<Body>,
    places: &Query<Place, With<Monster>>,
) -> Option<(Vec3, f32, f32)> {
    let body = bodies.get(victim).ok()?;
    if let (Some(monster), Ok(place)) = (&body.monster, places.get(victim)) {
        return Some((
            body_centre(place.feet(), monster),
            monster.body.radius,
            monster.body.eye_height * 0.5,
        ));
    }
    // The party's members share its body.
    (!body.is_dead()).then(|| (PartyBody::centre(eye), party_body::RADIUS, party_body::RADIUS))
}

/// Where an attack aimed at `target` lands, if anywhere: a shot scatters
/// by its aim and lands where it strikes the target's body; a swing
/// reaches, misses, or fumbles onto a bystander, which spends its surprise.
/// An error when the target has no body to aim at.
fn land(
    rng: &mut GameRng,
    target: Entity,
    aim: i32,
    delivery: Delivery,
    surprise: &mut Surprise,
    eye: Vec3,
    bodies: &mut Query<Body>,
    places: &Query<Place, With<Monster>>,
) -> Result<Option<Landing>, ()> {
    match delivery {
        Delivery::Shot { from, blind, through } => {
            let (centre, radius, half) = struck_body(target, eye, bodies, places).ok_or(())?;
            let shot = attack::scatter(rng, from, centre, aim, blind);
            Ok(attack::strikes(centre, shot, radius, half).then(|| Landing {
                victim: target,
                roll: rng.below(100) + 1,
                sure: true,
                through,
                fumbled: false,
            }))
        }
        Delivery::Swing { bystanders, odds } => {
            let fumble = attack::fumble_chance(bystanders.len(), odds);
            Ok(match attack::swing(rng, bystanders.len(), fumble) {
                Aim::Missed => None,
                Aim::Reaches { roll, sure } => {
                    Some(Landing { victim: target, roll, sure, through: sure, fumbled: false })
                }
                Aim::Fumbled { bystander, roll } => {
                    *surprise = Surprise::default();
                    Some(Landing {
                        victim: bystanders[bystander],
                        roll,
                        sure: true,
                        through: false,
                        fumbled: true,
                    })
                }
            })
        }
    }
}

/// Spend what a ranged swing used (retail `ResolveCharacterAttack`'s
/// tail): the ammunition a launcher shot, a thrown weapon unless it never
/// runs out, or a gun's shot. Returns whether the hand can go on.
fn spend_shot(
    carried: &mut crate::game::inventory::Carried,
    items: &ItemCatalog,
    current: usize,
    hand: &Hand,
) -> bool {
    use crate::game::inventory::EquipSlot;
    let slots = [EquipSlot::Primary, EquipSlot::Secondary];
    let spent = if hand.paired { slots[1 - current] } else { slots[current] };
    let Some(stack) = carried.worn(spent) else { return false };
    let definition = &items[stack.item];
    if !hand.paired && definition.spell.is_some() {
        return true;
    }
    let thrown = definition.class == crate::content::items::EquipClass::ThrownWeapon;
    if !hand.paired && thrown && definition.never_depletes {
        return true;
    }
    carried.consume_use(stack.id, definition, definition.keeps_empty());
    carried.worn(spent).is_some_and(|s| s.remaining_uses() != Some(0))
}

/// A Berserk order needs the trait, a hand that can attack and a main hand
/// for close quarters (retail `CanCharacterBerserk`).
pub(super) fn can_berserk(character: &Character, hands: &attack::Hands) -> bool {
    let [main, off] = &hands.hands;
    character.has_trait(Trait::Berserk)
        && (main.in_play || off.in_play)
        && (!main.in_play || main.reach <= Reach::Short)
}
