//! What passing time does to an actor (`docs/rules/actors.md`, "Time"):
//! poison bites, what it wears or is mends or wastes it, disease wears a
//! sheet down, it recovers as its pace allows, and what ails it runs its
//! course.
//!
//! Whatever passes time says so at once, with the party's pace ([`Elapsed`]):
//! exploring and camping in `game::time`, a fight as each round ends. A
//! level that was away is owed the time it missed, at rest. Either way the
//! time is gone through in steps, each an event with its own observers over
//! the components it is about, one step's work done before the next begins.

use bevy::prelude::*;

use super::fitness::Present;
use super::{Abilities, Attributes, Drained, Laying, Level, Skills, Traits};
use crate::content::characters::{Attribute, Trait};
use crate::game::characters::{
    Calling, Condition, Conditions, Dead, Lasting, Recovering, Sickness, Vitals,
};
use crate::game::combat::Combatant;
use crate::game::data::Catalogs;
use crate::game::hits::Damaged;
use crate::game::magic::{LearnedSpells, SpellPoints};
use crate::game::party::roster::{PartyMember, PartyRoster};
use crate::game::rng::GameRng;
use crate::game::time::Ticks;
use crate::game::world::level::{LevelOwned, Woke};

/// How time is spent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    Still,
    Walking,
    Running,
    Camping,
    /// A combat round.
    Fighting,
}

impl Pace {
    /// The share of its stamina a body that breathes gets back at this pace
    /// (retail `RegenCharacterStamina`).
    fn wind(self) -> f32 {
        match self {
            Pace::Camping => 1.0,
            Pace::Still => 0.5,
            Pace::Walking => 0.1,
            Pace::Running | Pace::Fighting => 0.0,
        }
    }

    /// The share of its health a body mends at this pace (retail
    /// `GameTurnsPassedChar`, `AgeMonsterSight`).
    fn mending(self) -> f32 {
        match self {
            Pace::Camping => 1.0,
            Pace::Still => 0.5,
            Pace::Walking | Pace::Running | Pace::Fighting => 0.0,
        }
    }

    /// At rest, a poison bites as hard as it is; otherwise half again.
    fn at_rest(self) -> bool {
        matches!(self, Pace::Still | Pace::Camping)
    }
}

/// Game time has passed, at the party's pace: so many stamina ticks and
/// aging ticks. Whatever time wears on reacts to it: every actor, step by
/// step ([`pass`]), and a stance set on a group; the spells laid wear
/// separately ([`Aged`]).
///
/// [`Aged`]: crate::game::actors::spells::Aged
#[derive(Event, Clone, Copy, Debug)]
pub struct Elapsed {
    pub ticks: Ticks,
    pub pace: Pace,
}

/// How an actor mends as time passes, written when it is refitted.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Mending {
    /// Health, stamina and spell points a minute it gains outright, or
    /// loses when negative: a monster's by its species, a member's by what
    /// they wear (retail `hp_regeneration`, `stamina_regeneration`, the
    /// items' regeneration bonuses).
    pub health: i32,
    pub stamina: i32,
    pub magic: i32,
    /// Its stamina comes back by the breath, every 1.4 seconds (retail
    /// `UpdatePartyStamina`), not by the minute.
    pub breathes: bool,
    /// One who heals fast (Health Regeneration): what its mending is
    /// multiplied by, and it mends at half of that at a pace that gives
    /// none.
    pub hale: Option<f32>,
    /// One who gets their wind back fast (Stamina Regeneration): the share
    /// of it they get back in a fight, where no one else gets any; out of
    /// one, half again.
    pub tireless: Option<f32>,
    /// What its spell points' recovery is multiplied by.
    pub magic_rate: f32,
}

impl Default for Mending {
    fn default() -> Self {
        Self {
            health: 0,
            stamina: 0,
            magic: 0,
            breathes: false,
            hale: None,
            tireless: None,
            magic_rate: 1.0,
        }
    }
}

/// One swallowed whole (retail `W8_DEPENDENCE_SWALLOWED`): missing, and
/// wasting, until whoever swallowed them dies.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[relationship(relationship_target = Swallows)]
pub struct Swallowed(pub Entity);

/// Those something has swallowed, maintained by Bevy from [`Swallowed`].
#[derive(Component, Debug)]
#[relationship_target(relationship = Swallowed)]
pub struct Swallows(Vec<Entity>);

/// What being swallowed takes a minute when the swallower is nowhere about
/// (retail `ApplyConditionModifiers`); otherwise one and half its level.
const SWALLOWER_GONE: u32 = 5;

/// One who dies lets out whoever it swallowed (retail
/// `ReleaseMonsterConditionBindings`).
fn let_out(
    died: On<Add<Dead>>,
    swallowers: Query<&Swallows>,
    mut held: Query<&mut Conditions>,
    mut commands: Commands,
) {
    for member in swallowers.get(died.entity).into_iter().flat_map(|held| held.0.iter().copied()) {
        if let Ok(mut conditions) = held.get_mut(member) {
            conditions.lift(Condition::Missing);
        }
        commands.entity(member).remove::<Swallowed>();
    }
}

/// One no longer missing is no longer anything's to hold, whatever brought
/// them back.
pub(super) fn let_go(
    back: Query<(Entity, &Conditions), (With<Swallowed>, Changed<Conditions>)>,
    mut commands: Commands,
) {
    for (member, conditions) in &back {
        if !conditions.has(Condition::Missing) {
            commands.entity(member).remove::<Swallowed>();
        }
    }
}

/// A party one of whom tends the camp, alive and with nothing ailing them:
/// everything recovers half again as fast in it (retail
/// `RebuildPartyEffectBlock`).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Tended;

const TENDED: f32 = 1.5;
/// Disease leaves a quarter of a body's recovery.
const DISEASED: f32 = 0.25;

/// A party is tended exactly while one of it is up to tending it.
#[allow(clippy::type_complexity)]
pub(super) fn tend(
    changed: Query<
        (),
        (
            With<PartyMember>,
            Or<(Changed<Conditions>, Changed<Vitals>, Changed<Traits>, Changed<PartyMember>)>,
        ),
    >,
    mut left: RemovedComponents<PartyMember>,
    parties: Query<(Entity, &PartyRoster, Has<Tended>)>,
    members: Query<(&Traits, &Conditions, &Vitals)>,
    mut commands: Commands,
) {
    if changed.is_empty() && left.read().count() == 0 {
        return;
    }
    for (party, roster, tended) in &parties {
        let tends = roster.iter().any(|member| {
            members.get(member).is_ok_and(|(traits, conditions, vitals)| {
                traits.has(Trait::CampRecovery) && conditions.is_empty() && !vitals.is_dead()
            })
        });
        super::fitness::mark::<Tended>(&mut commands.entity(party), tended, tends);
    }
}

/// A stretch of time to go through: its ticks, the party's pace, and, for
/// time a level is owed, the level whose actors it passes for.
#[derive(Clone, Copy, Debug)]
pub struct Span {
    pub ticks: Ticks,
    pub pace: Pace,
    pub level: Option<Entity>,
}

/// Poison bites, and what wastes its bearer wastes it.
#[derive(Event, Clone, Copy, Debug)]
pub(crate) struct Bites(Span);
/// Disease wears a sheet down.
#[derive(Event, Clone, Copy, Debug)]
pub(crate) struct Ails(Span);
/// Bodies mend and spell points come back.
#[derive(Event, Clone, Copy, Debug)]
pub(crate) struct Mends(Span);
/// What ails runs its course.
#[derive(Event, Clone, Copy, Debug)]
pub(crate) struct Courses(Span);

/// The steps of a span, in the order the original takes them.
fn go_through(span: Span, commands: &mut Commands) {
    commands.trigger(Bites(span));
    commands.trigger(Ails(span));
    commands.trigger(Mends(span));
    commands.trigger(Courses(span));
}

/// Time passes for everyone.
fn pass(elapsed: On<Elapsed>, mut commands: Commands) {
    let Elapsed { ticks, pace } = *elapsed.event();
    go_through(Span { ticks, pace, level: None }, &mut commands);
}

/// A level that was away owes those in it the time that passed, which they
/// spent at rest (retail `AgeMonsterSight` at full recovery, as the level is
/// entered again).
fn owe(woke: On<Woke>, mut commands: Commands) {
    let aging = (woke.away_ms / crate::game::time::AGING_TICK_MS) as u32;
    if aging > 0 {
        let ticks = Ticks { stamina: 0, aging };
        go_through(Span { ticks, pace: Pace::Camping, level: Some(woke.level) }, &mut commands);
    }
}

/// Those time passes for: the living who are with us (or swallowed, which
/// is not being away), and no sheet without a seat in the party.
#[derive(bevy::ecs::query::QueryFilter)]
pub(crate) struct InTime {
    alive: Without<Dead>,
    here: Or<(With<Present>, With<Swallowed>)>,
    seated: Or<(With<PartyMember>, Without<Calling>)>,
}

/// How an actor spends a span.
#[derive(bevy::ecs::query::QueryData)]
pub(crate) struct Timed {
    member: Option<&'static PartyMember>,
    fights: Has<Combatant>,
    owner: Option<&'static LevelOwned>,
}

impl TimedItem<'_, '_> {
    /// The pace the span passes at for this actor, if it passes for it at
    /// all: the party's for a member; one not of the party keeps still,
    /// unless it camps with the party or is in the fight.
    fn pace(&self, span: &Span) -> Option<Pace> {
        if span.level.is_some_and(|level| self.owner.map(|owner| owner.0) != Some(level)) {
            return None;
        }
        Some(match self.member {
            Some(_) => span.pace,
            None if span.pace == Pace::Camping => Pace::Camping,
            None if self.fights => Pace::Fighting,
            None => Pace::Still,
        })
    }
}

/// Poison bites: its strength a minute, half again unless its bearer is at
/// rest. What wastes its bearer a minute goes with it, and what being
/// swallowed takes: one and half the swallower's level. All past any
/// Guardian Angel, with nobody behind them.
fn bite(
    bites: On<Bites>,
    mut actors: Query<
        (Entity, &mut Vitals, &Conditions, &Mending, Option<&Swallowed>, Timed),
        InTime,
    >,
    levels: Query<&Level>,
    mut laying: Laying,
    mut damaged: MessageWriter<Damaged>,
) {
    let span = bites.0;
    let minutes = span.ticks.aging;
    for (entity, mut vitals, conditions, mending, swallowed, timed) in &mut actors {
        let Some(pace) = timed.pace(&span) else { continue };
        let digested = swallowed.map_or(0, |&Swallowed(by)| {
            levels.get(by).map_or(SWALLOWER_GONE, |level| 1 + level.0 / 2)
        });
        let wasting = ((-mending.health).max(0) as u32 + digested) * minutes;
        let amount = conditions.poison_damage(minutes, pace.at_rest()) + wasting;
        if amount == 0 {
            continue;
        }
        let lost = vitals.hurt(laying.absorb(entity, amount), None);
        if lost > 0 {
            damaged.write(Damaged { body: entity, amount: lost, died: vitals.is_dead() });
        }
    }
}

/// Disease wears one who trained down ([`Vitals::sicken`]): a lasting
/// condition, a point of an ability for good, or some of the most health or
/// stamina. A slowing without end ends Haste.
#[allow(clippy::type_complexity)]
fn sicken(
    ails: On<Ails>,
    mut trained: Query<
        (Entity, &mut Vitals, &mut Conditions, &mut Abilities, &mut Drained, &Attributes, Timed),
        (InTime, With<Calling>),
    >,
    mut laying: Laying,
    mut rng: ResMut<GameRng>,
) {
    let span = ails.0;
    for (entity, mut vitals, mut conditions, mut abilities, mut drained, attributes, timed) in
        &mut trained
    {
        if timed.pace(&span).is_none() || !conditions.has(Condition::Diseased) {
            continue;
        }
        let vitality = attributes[Attribute::Vitality];
        let minutes = span.ticks.aging;
        // Worked on copies, so that a roll that takes nothing marks
        // nothing changed.
        let (mut base, mut lost) = (*abilities, *drained);
        let struck =
            vitals.sicken(&mut conditions, (&mut base, &mut lost), vitality, minutes, &mut rng);
        abilities.set_if_neq(base);
        drained.set_if_neq(lost);
        if struck == Some(Sickness::Condition(Condition::Slowed)) {
            laying.slow(entity, Lasting::Indefinite);
        }
    }
}

/// A body mends: what it gains or loses outright, a minute (and, for one
/// that breathes, stamina's with each breath out of a fight); then its own
/// recovery of health by the minute and of stamina by the breath or the
/// minute, by its pace.
fn mend(
    mends: On<Mends>,
    mut bodies: Query<(&mut Vitals, &mut Recovering, &Conditions, &Mending, Timed), InTime>,
    tended: Query<(), With<Tended>>,
) {
    let span = mends.0;
    let Ticks { stamina: breaths, aging: minutes } = span.ticks;
    for (mut vitals, mut parts, conditions, mending, timed) in &mut bodies {
        let Some(pace) = timed.pace(&span) else { continue };
        let whole = vitals.health.current >= vitals.health.max
            && vitals.stamina.current >= vitals.stamina.max
            && *parts == Recovering::default();
        if whole && mending.stamina >= 0 {
            continue;
        }
        let fighting = pace == Pace::Fighting;
        let beats = if mending.breathes && !fighting { breaths + minutes } else { minutes };
        if mending.health > 0 {
            vitals.heal(mending.health as u32 * minutes);
        }
        if mending.stamina > 0 {
            vitals.rest(mending.stamina as u32 * beats);
        } else {
            vitals.tire((-mending.stamina) as u32 * beats);
        }
        let boost =
            if timed.member.is_some_and(|member| tended.contains(member.0)) { TENDED } else { 1.0 };
        let ill = if conditions.has(Condition::Diseased) { DISEASED } else { 1.0 };
        let (winds, wind) = if fighting {
            (minutes, mending.tireless.unwrap_or(0.0))
        } else if mending.breathes {
            let quick = if mending.tireless.is_some() { 1.5 } else { 1.0 };
            (breaths, pace.wind() * ill * quick)
        } else {
            (minutes, pace.mending() * ill)
        };
        if winds > 0 {
            vitals.recover_stamina(&mut parts, winds, wind * boost);
        }
        if minutes > 0 {
            let share = pace.mending() * ill;
            let share = match mending.hale {
                Some(hale) => hale * if share == 0.0 { 0.5 } else { share },
                None => share,
            };
            vitals.recover_health(&mut parts, minutes, share * boost);
        }
    }
}

/// Spell points come back by the minute: what is worn gives or takes
/// outright; then each realm's own recovery, which follows health's share
/// of the pace, never under a half.
fn replenish(
    mends: On<Mends>,
    mut casters: Query<
        (
            (&LearnedSpells, &mut SpellPoints),
            (&Level, &Attributes, &Skills, &Conditions, &Mending),
            Timed,
        ),
        InTime,
    >,
    tended: Query<(), With<Tended>>,
    content: Res<Catalogs>,
) {
    let span = mends.0;
    let minutes = span.ticks.aging;
    if minutes == 0 {
        return;
    }
    for ((known, mut pool), (level, attributes, skills, conditions, mending), timed) in &mut casters
    {
        let Some(pace) = timed.pace(&span) else { continue };
        if mending.magic == 0 && !pool.needs_recovery() {
            continue;
        }
        let maximum =
            crate::game::magic::max_points(level.0, attributes, known, &content.spells, skills);
        if mending.magic != 0 {
            pool.adjust(mending.magic * minutes as i32, &maximum);
        }
        let boost =
            if timed.member.is_some_and(|member| tended.contains(member.0)) { TENDED } else { 1.0 };
        let ill = if conditions.has(Condition::Diseased) { DISEASED } else { 1.0 };
        let share = (pace.mending() * ill).max(0.5) * mending.magic_rate * boost;
        pool.recover(minutes, share, &maximum);
    }
}

/// What ails runs its course by the minutes.
fn run_course(
    courses: On<Courses>,
    mut ailing: Query<(&mut Conditions, Timed), InTime>,
    mut rng: ResMut<GameRng>,
) {
    let span = courses.0;
    for (mut conditions, timed) in &mut ailing {
        if span.ticks.aging > 0 && timed.pace(&span).is_some() && conditions.any_timed() {
            conditions.age(span.ticks.aging, &mut rng);
        }
    }
}

pub(crate) fn plugin(app: &mut App) {
    app.add_observer(pass)
        .add_observer(owe)
        .add_observer(bite)
        .add_observer(sicken)
        .add_observer(mend)
        .add_observer(replenish)
        .add_observer(run_course)
        .add_observer(let_out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::characters::Character;
    use crate::game::characters::{Gender, Profession, Race};

    /// A world where time can pass over actors.
    fn world() -> World {
        let mut app = App::new();
        app.insert_resource(Catalogs::default())
            .insert_resource(GameRng::default())
            .add_message::<Damaged>()
            .add_plugins(plugin);
        std::mem::take(app.world_mut())
    }

    /// Time passes, and every step of it is gone through.
    fn pass_time(world: &mut World, elapsed: Elapsed) {
        world.trigger(elapsed);
        world.flush();
    }

    fn hurt(world: &mut World, body: Entity) {
        let mut vitals = world.get_mut::<Vitals>(body).unwrap();
        vitals.health.current = 1;
        vitals.stamina.current = 10;
        world.entity_mut(body).insert(Recovering::default());
    }

    fn left(world: &World, body: Entity) -> (u32, u32) {
        let vitals = world.get::<Vitals>(body).unwrap();
        (vitals.health.current, vitals.stamina.current)
    }

    /// Retail `AgeMonsterSight`: a monster mends at half its rate out of a
    /// fight, not at all in one, and by its species' points a minute
    /// wherever it is; a poison in it bites half again in a fight.
    #[test]
    fn a_monster_mends_out_of_a_fight_and_is_poisoned_in_one() {
        let mut world = world();
        let resting = world.spawn(Vitals::full(100, 100)).id();
        let fight = world.spawn_empty().id();
        let fighting = world.spawn((Vitals::full(100, 100), Combatant(fight))).id();
        let troll = Mending { health: 3, ..default() };
        let regenerating = world.spawn((Vitals::full(100, 100), troll, Combatant(fight))).id();
        for body in [resting, fighting, regenerating] {
            hurt(&mut world, body);
        }
        let ticks = Ticks { stamina: 85, aging: 10 };
        pass_time(&mut world, Elapsed { ticks, pace: Pace::Walking });
        // (0.4 × 100 + 20) / 240 × 0.5 a minute is 1.25 in ten; stamina
        // (0.9 × 100 + 20) / 240 × 0.5 is 2.29.
        assert_eq!(left(&world, resting), (2, 12));
        assert_eq!(left(&world, fighting), (1, 10));
        assert_eq!(left(&world, regenerating), (31, 10));
        let mut poisoned = world.get_mut::<Conditions>(fighting).unwrap();
        poisoned.poison(Lasting::Indefinite, 4);
        world.get_mut::<Vitals>(fighting).unwrap().health.current = 50;
        let round = Ticks { stamina: 0, aging: 1 };
        pass_time(&mut world, Elapsed { ticks: round, pace: Pace::Fighting });
        assert_eq!(left(&world, fighting).0, 44, "four, half again in a fight");
    }

    /// Retail `GameTurnsPassedChar`, `RegenCharacterStamina`: a member gets
    /// their breath back standing still and less of it walking, heals only
    /// at rest, and half again as fast in a party a Bard tends, while the
    /// Bard is up to it.
    #[test]
    fn a_member_recovers_by_the_partys_pace() {
        let mut world = world();
        let party = world.spawn(PartyRoster::default()).id();
        let member = |world: &mut World, profession| {
            let sheet = Character::new("M", Race::Human, Gender::Male, profession);
            let body = (Vitals::full(100, 100), Mending { breathes: true, ..default() });
            world.spawn((sheet, body, PartyMember(party))).id()
        };
        let fighter = member(&mut world, Profession::Fighter);
        let rest = |world: &mut World, pace| {
            world.run_system_cached(crate::game::actors::figures::follow_calling).unwrap();
            world.run_system_cached(tend).unwrap();
            hurt(world, fighter);
            pass_time(world, Elapsed { ticks: Ticks { stamina: 100, aging: 10 }, pace });
            left(world, fighter)
        };
        // Health (0.4 × 100 + 20) / 240 a minute, stamina (0.9 × 100 + 20)
        // / 240 a breath.
        assert_eq!(rest(&mut world, Pace::Still), (2, 32));
        assert_eq!(rest(&mut world, Pace::Walking), (1, 14));
        assert_eq!(rest(&mut world, Pace::Running), (1, 10));
        assert_eq!(rest(&mut world, Pace::Camping), (3, 55));
        let bard = member(&mut world, Profession::Bard);
        assert_eq!(rest(&mut world, Pace::Camping), (4, 78), "half again, tended");
        world.get_mut::<Conditions>(bard).unwrap().afflict(Condition::Afraid, Lasting::Indefinite);
        assert_eq!(rest(&mut world, Pace::Camping), (3, 55), "the Bard is not up to it");
    }

    /// What a member wears gives or takes a point a minute, and stamina's
    /// with each breath (retail `GameTurnsPassedChar`,
    /// `RegenCharacterStamina`); a Fighter alone gets wind back in a fight.
    #[test]
    fn what_is_worn_mends_or_wastes_and_a_fighter_rests_in_a_fight() {
        let mut world = world();
        let party = world.spawn_empty().id();
        let sheet = Character::new("M", Race::Human, Gender::Male, Profession::Fighter);
        let ring = Mending { health: 2, stamina: -1, breathes: true, ..default() };
        let wearer = world.spawn((sheet.clone(), Vitals::full(100, 100), ring, PartyMember(party)));
        let wearer = wearer.id();
        hurt(&mut world, wearer);
        world.get_mut::<Vitals>(wearer).unwrap().stamina.current = 100;
        let ticks = Ticks { stamina: 20, aging: 5 };
        pass_time(&mut world, Elapsed { ticks, pace: Pace::Running });
        assert_eq!(left(&world, wearer), (11, 75));
        let tireless = Mending { breathes: true, tireless: Some(3.3), ..default() };
        let fighter = world.spawn((sheet, Vitals::full(100, 100), tireless, PartyMember(party)));
        let fighter = fighter.id();
        hurt(&mut world, fighter);
        let round = Ticks { stamina: 85, aging: 1 };
        pass_time(&mut world, Elapsed { ticks: round, pace: Pace::Fighting });
        // (0.9 × 100 + 20) / 240 × 3.3 is 1.51 a round.
        assert_eq!(left(&world, fighter), (1, 11));
    }

    /// Retail `ApplyConditionModifiers`, `ReleaseMonsterConditionBindings`:
    /// one swallowed loses one and half the swallower's level a minute, five
    /// when it is nowhere about, and is let out when it dies; one missing
    /// otherwise is out of time's reach.
    #[test]
    fn one_swallowed_wastes_until_the_swallower_dies() {
        let mut world = world();
        let party = world.spawn_empty().id();
        let missing = |world: &mut World| {
            let sheet = Character::new("M", Race::Human, Gender::Male, Profession::Fighter);
            let body = (Vitals::full(100, 100), Mending { breathes: true, ..default() });
            let member = world.spawn((sheet, body, PartyMember(party))).id();
            let mut conditions = world.get_mut::<Conditions>(member).unwrap();
            conditions.afflict(Condition::Missing, Lasting::Indefinite);
            conditions.poison(Lasting::Indefinite, 3);
            world.entity_mut(member).remove::<Present>();
            member
        };
        let worm = world.spawn((Vitals::full(50, 50), Level(8))).id();
        let elsewhere = world.spawn_empty().id();
        let (taken, eaten, lost) = (missing(&mut world), missing(&mut world), missing(&mut world));
        world.entity_mut(eaten).insert(Swallowed(worm));
        world.entity_mut(lost).insert(Swallowed(elsewhere));
        let minute = Elapsed { ticks: Ticks { stamina: 0, aging: 1 }, pace: Pace::Camping };
        pass_time(&mut world, minute);
        let health = |world: &World, member| world.get::<Vitals>(member).unwrap().health.current;
        assert_eq!(health(&world, taken), 100, "nothing passes for one only missing");
        assert_eq!(health(&world, eaten), 100 - 3 - 5, "the poison, and one and half of eight");
        assert_eq!(health(&world, lost), 100 - 3 - 5, "the poison, and five");
        world.entity_mut(worm).insert(Dead::default());
        assert!(!world.get::<Conditions>(eaten).unwrap().has(Condition::Missing));
        assert!(world.get::<Swallowed>(eaten).is_none());
        assert!(world.get::<Conditions>(lost).unwrap().has(Condition::Missing));
    }
}
