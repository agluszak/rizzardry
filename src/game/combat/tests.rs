//! Headless transition tests using the production plugins and settlement schedule.
use super::*;
use crate::content::characters::Realm;
use crate::content::items::{AttackMode, Dice};
use crate::content::level::LevelDefinition;
use crate::content::monster::MonsterBody;
use crate::content::species::{Disposition, MonsterAttack, MonsterCatalog, MonsterWeapon, Species};
use crate::content::spells::{SpellCatalog, SpellDefinition, SpellId};
use crate::game::actors::{Enchantment, Laid, Laying};
use crate::game::characters::{Gender, Profession, Race};
use crate::game::hits::definitions::Definitions;
use crate::game::hits::effects::Strike;
use crate::game::hits::{self, Act, Deed, Effect, Release};
use crate::game::magic::casting::{Aim, Cast, MagicChoice, MagicNews, Refusal, Source, UseContext};
use crate::game::magic::{self, LearnedSpells, Strength};
use crate::game::party::body::PartyBody;
use crate::game::party::roster;
use crate::game::time::GameClock;
use crate::game::world::hostility::{Loyalty, Side, Stance};
use crate::game::world::identify::{GroupMember, Identified, MonsterGroup};
use crate::game::world::level::Level;
use crate::game::world::navigation::{Walker, WalkerMotionConfig};
use crate::game::world::population::{ScaledBody, Worth};
use avian3d::prelude::Position;

fn app() -> (App, Entity, Entity) {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_resource::<GameRng>()
        .init_resource::<GameClock>()
        .init_resource::<crate::game::factions::Factions>()
        .init_resource::<crate::game::difficulty::Difficulty>()
        .init_resource::<avian3d::collider_tree::ColliderTrees>()
        .init_resource::<Assets<LevelDefinition>>()
        .init_resource::<Assets<MonsterBody>>()
        .init_resource::<Assets<WalkerMotionConfig>>()
        .add_message::<crate::game::factions::FactionNews>()
        .add_message::<Identified>()
        .add_plugins((
            CombatPlugin,
            magic::MagicPlugin,
            hits::HitsPlugin,
            crate::game::actors::plugin,
        ));
    let mut spells: Vec<_> = (0..=255)
        .map(|id| SpellDefinition { id: SpellId(id), realm: Some(Realm::Divine), ..default() })
        .collect();
    spells[SpellId::HEAL_WOUNDS.0 as usize] = magic::tests::heal_wounds();
    spells[SpellId::FIRESTORM.0 as usize].dice = Some(Dice { count: 0, sides: 1, base: 100 });
    let kind = Species {
        disposition: Disposition::Hostile,
        faction: crate::game::factions::UNALIGNED,
        attacks_per_round: 1,
        swings_per_round: 1,
        combat_move_range: 2.0,
        attacks: vec![MonsterAttack {
            word: AttackWord::Claw,
            weapon: MonsterWeapon::Unarmed,
            reach: Reach::Touch,
            score: 50,
            modes: AttackMode::Swing.into(),
            damage: Dice { count: 1, sides: 1, base: 0 },
            effects: default(),
        }],
        ..default()
    };
    app.insert_resource(Catalogs {
        spells: SpellCatalog::new(spells),
        species: MonsterCatalog(vec![kind]),
        ..default()
    });
    let world = app.world_mut();
    let profile = Character::new("Priest", Race::Human, Gender::Male, Profession::Priest);
    let member = world
        .spawn((Vitals::new(&profile), profile, LearnedSpells([SpellId::HEAL_WOUNDS].into())))
        .id();
    let party = roster::test_party(world, &[member]);
    world.entity_mut(party).insert((PartyBody::default(), Position::default()));
    let definition =
        world.resource_mut::<Assets<LevelDefinition>>().add(LevelDefinition::default());
    world.spawn(Level::test(definition));
    world.run_system_cached(hits::definitions::define).unwrap();
    world.flush();
    (app, party, member)
}

fn monster(world: &mut World, battle: Entity, distance: f32) -> Entity {
    let group = world.spawn(MonsterGroup::default()).id();
    world
        .spawn((
            Monster {
                location: 0,
                species: 0,
                body: ScaledBody { scale: 1.0, radius: 0.5, eye_height: 1.0, speed: 1.0 },
            },
            Vitals::full(20, 100),
            Transform::from_xyz(0.0, 0.0, distance),
            Side::new(Stance::Hostile),
            Loyalty::None,
            Walker { speed: 1.0, float_height: 0.0 },
            Combatant(battle),
            GroupMember(group),
            Worth(120),
        ))
        .id()
}

/// Lay a spell of power one on a bearer, for `turns`, absorbing `absorbs`.
fn lay(world: &mut World, bearer: Entity, spell: SpellId, turns: u32, absorbs: u32) {
    use bevy::ecs::system::RunSystemOnce;
    let laid = Enchantment { spell, power: 1, boost: 0 };
    world
        .run_system_once(move |mut laying: Laying| laying.lay(bearer, laid, turns, absorbs))
        .unwrap();
    world.flush();
}

fn is_laid(world: &mut World, bearer: Entity, spell: SpellId) -> bool {
    use bevy::ecs::system::RunSystemOnce;
    world.run_system_once(move |laid: Laid| laid.has(bearer, spell)).unwrap()
}

fn refresh(world: &mut World) {
    world.flush();
}

/// The fight's round is being played, at this moment.
fn play(world: &mut World, fight: Entity, moment: u32) {
    let mut round = world.get_mut::<Round>(fight).unwrap();
    round.playing = true;
    round.moment = moment;
}

#[test]
fn anti_magic_drains_once_and_then_dispels_the_party() {
    let (mut app, party, member) = app();
    let world = app.world_mut();
    let second = world
        .spawn((
            Character::new("Other", Race::Human, Gender::Male, Profession::Priest),
            Vitals::full(20, 100),
            LearnedSpells([SpellId::HEAL_WOUNDS].into()),
            roster::PartyMember(party),
        ))
        .id();
    lay(world, party, SpellId::MAGIC_SCREEN, 10, 0);
    let definition = world.resource::<Definitions>().spell(SpellId::ANTI_MAGIC).unwrap();
    let strength = Strength { power: 1, against: 1, boost: 0 };
    world.trigger(Release {
        deed: Deed::new(Act::Fight, Strike { definition, strength, blow: None }),
        victims: vec![(member, None), (second, None)],
    });
    world.flush();
    assert!(!is_laid(world, party, SpellId::MAGIC_SCREEN));
    let news: Vec<_> = world.resource_mut::<Messages<CombatNews>>().drain().collect();
    let [CombatNews::Effects(effects)] = news.as_slice() else { panic!("{news:?}") };
    let drained = effects.iter().filter(|e| matches!(e, Effect::Drained { .. })).count();
    assert_eq!(drained, 2, "each pool is drained once, not once per body reached");
}

#[test]
fn final_cloud_pulse_settles_and_credits_the_kill_before_victory() {
    let (mut app, _, member) = app();
    let world = app.world_mut();
    let fight = world.spawn(Battle::default()).id();
    play(world, fight, 101);
    world.get_mut::<Round>(fight).unwrap().attacked = true;
    let actor = monster(world, fight, 3.0);
    lay(world, actor, SpellId::FIRESTORM, 1, 0);
    let before = world.get::<Character>(member).unwrap().experience;
    world.run_schedule(FixedUpdate);
    assert!(world.get::<Vitals>(actor).unwrap().is_dead());
    assert_eq!(world.get::<Character>(member).unwrap().experience, before + 120);
    let news: Vec<_> = world.resource_mut::<Messages<CombatNews>>().drain().collect();
    assert_eq!(
        news.iter()
            .filter(|n| matches!(n, CombatNews::Killed { actor: killed } if *killed == actor))
            .count(),
        1
    );
    assert!(news.iter().any(|n| matches!(n, CombatNews::Won { kills: 1, experience_each: 120 })));
    world.run_schedule(FixedUpdate);
    assert_eq!(world.get::<Character>(member).unwrap().experience, before + 120);
}

#[test]
fn a_clouds_last_pulse_still_sees_the_targets_protection() {
    let (mut app, _, _) = app();
    let world = app.world_mut();
    let fight = world.spawn(Battle::default()).id();
    play(world, fight, 101);
    let actor = monster(world, fight, 3.0);
    lay(world, actor, SpellId::FIRESTORM, 1, 0);
    lay(world, actor, SpellId::GUARDIAN_ANGEL, 1, 100);
    world.run_schedule(FixedUpdate);
    assert_eq!(world.get::<Vitals>(actor).unwrap().health.current, 20);
    world.flush();
    assert!(!is_laid(world, actor, SpellId::GUARDIAN_ANGEL));
}

#[test]
fn missile_arrival_settles_defeat_before_any_actor_can_start() {
    let (mut app, _, member) = app();
    let world = app.world_mut();
    let fight = world.spawn(Battle::default()).id();
    let actor = monster(world, fight, 3.0);
    play(world, fight, 1);
    refresh(world);
    let arms = world.get::<crate::game::actors::Attacks>(actor).unwrap();
    let turn = Turn {
        when: Some(1),
        attacks: enum_map::EnumMap::from_fn(|slot| arms.attacks(slot)),
        due: crate::game::inventory::WeaponSlot::Primary,
        weapons: arms.weapons(),
        lightning: None,
        moved: false,
        special: false,
        berserk: false,
        waited: false,
    };
    world.entity_mut(actor).insert(turn);
    let definition = world.resource::<Definitions>().pulse(SpellId::FIRESTORM).unwrap();
    let strength = Strength { power: 1, against: 100, boost: 0 };
    world.trigger(Release {
        deed: Deed::new(Act::Fight, Strike { definition, strength, blow: None }),
        victims: vec![(member, Some(1.0))],
    });
    let before = world.get::<Vitals>(member).unwrap().health.current;
    // Released this step, it flies for the next two.
    world.resource_mut::<Time>().advance_by(std::time::Duration::from_millis(500));
    world.run_schedule(FixedUpdate);
    world.run_schedule(FixedUpdate);
    assert_eq!(world.get::<Vitals>(member).unwrap().health.current, before);
    world.run_schedule(FixedUpdate);
    assert!(world.get::<Vitals>(member).unwrap().is_dead());
    assert!(
        world.resource_mut::<Messages<CombatNews>>().drain().any(|n| matches!(n, CombatNews::Lost))
    );
}

#[test]
fn killing_a_turned_ally_awards_nothing_but_an_enemy_counts() {
    use crate::game::characters::{Condition, Lasting};
    let (mut app, _, _) = app();
    let world = app.world_mut();
    let battle = world.spawn(Battle::default()).id();
    let actor = monster(world, battle, 3.0);
    refresh(world);
    world.get_mut::<Vitals>(actor).unwrap().afflict(Condition::Turncoat, Lasting::Rounds(5));
    refresh(world);
    let kill = |world: &mut World, body: Entity| {
        world.get_mut::<Vitals>(body).unwrap().slay();
        world.trigger(crate::game::characters::Settle);
        world.flush();
    };
    kill(world, actor);
    assert_eq!(world.get::<Spoils>(battle).unwrap().experience, 0);
    // An enemy never turned counts.
    let enemy = monster(world, battle, 3.0);
    refresh(world);
    kill(world, enemy);
    assert_eq!(world.get::<Spoils>(battle).unwrap().kills, 1);
    assert_eq!(world.get::<Spoils>(battle).unwrap().experience, 120);
}

/// A member's cast in a fight, resolved in one pass: it provokes the foe it
/// is aimed at, pays its points whatever the roll, and a missile that went
/// off is reported as it leaves, then lands on arrival.
#[test]
fn a_members_blast_provokes_its_foe_pays_its_points_and_flies_to_it() {
    use crate::content::characters::Skill;
    use crate::content::spells::{Reach, Spellbook, Targeting, Usable};
    use crate::game::magic::{Outcome, SpellPoints};
    use crate::game::world::hostility::Provoked;
    let (mut app, _, member) = app();
    let world = app.world_mut();
    world.resource_mut::<Catalogs>().spells.0[SpellId::ENERGY_BLAST.0 as usize] = SpellDefinition {
        id: SpellId::ENERGY_BLAST,
        realm: Some(Realm::Fire),
        level: 1,
        cost: 1,
        books: Spellbook::Divinity.into(),
        targeting: Targeting::Enemy,
        reach: Reach::Long,
        usable: Usable::Always,
        missile: Some(1),
        dice: Some(Dice { base: 5, count: 0, sides: 1 }),
        ..default()
    };
    let mut character = world.get_mut::<Character>(member).unwrap();
    character.skill_points[Skill::Divinity] = 100;
    character.skill_points[Skill::FireMagic] = 100;
    world.entity_mut(member).insert((
        LearnedSpells([SpellId::HEAL_WOUNDS, SpellId::ENERGY_BLAST].into()),
        SpellPoints::default(),
    ));
    let fight = world.spawn(Battle::default()).id();
    play(world, fight, 1);
    let foe = monster(world, fight, 3.0);
    refresh(world);
    let points = magic::test_points(world, member, Realm::Fire);
    assert!(points >= 1);
    let spell = &world.resource::<Catalogs>().spells[SpellId::ENERGY_BLAST];
    assert_eq!(magic::fail_chance(world.get::<Character>(member).unwrap(), spell, 1), 0);
    let choice = MagicChoice {
        source: Source::Spell { spell: SpellId::ENERGY_BLAST, power: 1 },
        aim: Aim::Body(foe),
    };
    let context = UseContext::Combat;
    world.trigger(Cast { caster: member, spell: SpellId::ENERGY_BLAST, context, choice });
    world.flush();
    assert_eq!(magic::test_points(world, member, Realm::Fire), points - 1);
    let provoked: Vec<_> = world.resource_mut::<Messages<Provoked>>().drain().collect();
    assert_eq!(provoked, vec![Provoked(foe)]);
    let news: Vec<_> = world.resource_mut::<Messages<MagicNews>>().drain().collect();
    assert!(
        matches!(news.as_slice(), [MagicNews::Done(report)] if report.outcome == Outcome::Cast),
        "{news:?}"
    );
    assert_eq!(
        world.query::<&hits::Flight>().iter(world).count(),
        1,
        "a missile flies to its victim"
    );
    assert!(world.get::<Round>(fight).unwrap().attacked);
    // Three metres at fifteen a second: it lands within the next step.
    world.resource_mut::<Time>().advance_by(std::time::Duration::from_millis(500));
    world.run_schedule(FixedUpdate);
    world.run_schedule(FixedUpdate);
    let news: Vec<_> = world.resource_mut::<Messages<MagicNews>>().drain().collect();
    assert!(
        matches!(news.as_slice(), [MagicNews::Landed { caster, spell: SpellId::ENERGY_BLAST, .. }] if *caster == member),
        "{news:?}"
    );
}

/// Who had a moment, in order, and whether they were dead when it came.
#[derive(Resource, Default)]
struct Moments(Vec<(Entity, bool)>);

fn record(
    moment: On<turn::Moment>,
    dead: Query<(), With<crate::game::characters::Dead>>,
    mut moments: ResMut<Moments>,
) {
    moments.0.push((moment.actor, dead.contains(moment.actor)));
}

/// A turn due at `when` with `attacks` left in the primary slot.
fn due_at(world: &World, actor: Entity, when: u32, attacks: u32) -> Turn {
    let arms = world.get::<crate::game::actors::Attacks>(actor).unwrap();
    let mut due = Turn {
        when: Some(when),
        attacks: enum_map::EnumMap::default(),
        due: crate::game::inventory::WeaponSlot::Primary,
        weapons: arms.weapons(),
        lightning: None,
        moved: false,
        special: false,
        berserk: false,
        waited: false,
    };
    due.attacks[crate::game::inventory::WeaponSlot::Primary] = attacks;
    due
}

/// Let the fight's pause run out and the fixed step come.
fn step(world: &mut World) {
    world.resource_mut::<Time>().advance_by(std::time::Duration::from_millis(1500));
    world.run_schedule(FixedUpdate);
}

#[test]
fn the_earliest_moment_comes_first_and_ties_go_by_the_fights_order() {
    let (mut app, ..) = app();
    let world = app.world_mut();
    world.init_resource::<Moments>();
    world.add_observer(record);
    let fight = world.spawn(Battle::default()).id();
    let late = monster(world, fight, 3.0);
    let early = monster(world, fight, 3.0);
    let tied = monster(world, fight, 3.0);
    refresh(world);
    for (actor, when) in [(late, 30), (early, 10), (tied, 30)] {
        let turn = due_at(world, actor, when, 0);
        world.entity_mut(actor).insert((turn, Ordered(Action::Defend)));
    }
    play(world, fight, 0);
    for _ in 0..3 {
        step(world);
    }
    let order: Vec<_> = world.resource::<Moments>().0.iter().map(|&(actor, _)| actor).collect();
    assert_eq!(order, [early, late, tied]);
    assert_eq!(world.get::<Round>(fight).unwrap().progress(), Some(0.3));
    // Nothing is left to come: the round is over.
    step(world);
    assert!(world.get::<Round>(fight).is_none_or(|round| !round.executing()));
}

#[test]
fn an_actor_with_two_attacks_acts_again_after_those_whose_moments_fall_between() {
    let (mut app, _, member) = app();
    let world = app.world_mut();
    world.init_resource::<Moments>();
    world.add_observer(record);
    let fight = world.spawn(Battle::default()).id();
    let striker = monster(world, fight, 0.8);
    let between = monster(world, fight, 3.0);
    let last = monster(world, fight, 3.0);
    refresh(world);
    let turn = due_at(world, striker, 10, 2);
    world.entity_mut(striker).insert(turn);
    for (actor, when) in [(between, 30), (last, 60)] {
        let turn = due_at(world, actor, when, 0);
        world.entity_mut(actor).insert((turn, Ordered(Action::Defend)));
    }
    play(world, fight, 0);
    let before = world.get::<Vitals>(member).unwrap().health.current;
    for _ in 0..4 {
        step(world);
    }
    let order: Vec<_> = world.resource::<Moments>().0.iter().map(|&(actor, _)| actor).collect();
    assert_eq!(order, [striker, between, striker, last]);
    let attacks = world
        .resource_mut::<Messages<CombatNews>>()
        .drain()
        .filter(|news| matches!(news, CombatNews::Attack { attacker, .. } if *attacker == striker))
        .count();
    assert_eq!(attacks, 2);
    assert!(world.get::<Vitals>(member).unwrap().health.current <= before);
}

/// A member's cast of a spell that kills, ordered for the moment it comes.
fn deadly_order(world: &mut World, member: Entity, foe: Entity) -> Ordered {
    use crate::content::characters::Skill;
    use crate::content::spells::{Reach, Spellbook, Targeting, Usable};
    world.resource_mut::<Catalogs>().spells.0[SpellId::ENERGY_BLAST.0 as usize] = SpellDefinition {
        id: SpellId::ENERGY_BLAST,
        realm: Some(Realm::Fire),
        level: 1,
        cost: 1,
        books: Spellbook::Divinity.into(),
        targeting: Targeting::Enemy,
        reach: Reach::Long,
        usable: Usable::Always,
        dice: Some(Dice { base: 100, count: 0, sides: 1 }),
        ..default()
    };
    let mut character = world.get_mut::<Character>(member).unwrap();
    character.skill_points[Skill::Divinity] = 100;
    character.skill_points[Skill::FireMagic] = 100;
    world.entity_mut(member).insert((
        LearnedSpells([SpellId::HEAL_WOUNDS, SpellId::ENERGY_BLAST].into()),
        magic::SpellPoints::default(),
    ));
    world.run_system_cached(hits::definitions::define).unwrap();
    Ordered(Action::Magic(MagicChoice {
        source: Source::Spell { spell: SpellId::ENERGY_BLAST, power: 1 },
        aim: Aim::Body(foe),
    }))
}

#[test]
fn what_a_moment_did_has_landed_and_its_deaths_are_settled_before_the_next_moment() {
    let (mut app, _, member) = app();
    let world = app.world_mut();
    world.init_resource::<Moments>();
    world.add_observer(record);
    let fight = world.spawn(Battle::default()).id();
    let foe = monster(world, fight, 3.0);
    refresh(world);
    let order = deadly_order(world, member, foe);
    let (turn, foes) = (due_at(world, member, 5, 0), due_at(world, foe, 10, 0));
    world.entity_mut(member).insert((turn, order));
    world.entity_mut(foe).insert((foes, Ordered(Action::Defend)));
    play(world, fight, 0);
    step(world);
    assert_eq!(world.get::<Vitals>(foe).unwrap().health.current, 0, "the blast has landed");
    assert!(world.get::<crate::game::characters::Dead>(foe).is_some(), "and the death is settled");
    assert_eq!(world.get::<Spoils>(fight).unwrap().kills, 1);
    step(world);
    assert_eq!(world.resource::<Moments>().0, [(member, false), (foe, true)]);
}

#[test]
fn a_cast_refused_in_a_fight_falls_back_to_defending() {
    let (mut app, _, member) = app();
    let world = app.world_mut();
    let fight = world.spawn(Battle::default()).id();
    let foe = monster(world, fight, 3.0);
    refresh(world);
    // Not a spell the member has learned.
    let choice = MagicChoice {
        source: Source::Spell { spell: SpellId::ENERGY_BLAST, power: 1 },
        aim: Aim::Body(foe),
    };
    world.entity_mut(member).insert(Ordered(Action::Magic(choice)));
    world.trigger(Cast {
        caster: member,
        spell: SpellId::ENERGY_BLAST,
        context: UseContext::Combat,
        choice,
    });
    world.flush();
    let news: Vec<_> = world.resource_mut::<Messages<MagicNews>>().drain().collect();
    assert!(matches!(news.as_slice(), [MagicNews::Refused { refusal: Refusal::NotLearned, .. }]));
    assert!(world.get::<Ordered>(member).unwrap().defends());
    // The same when the item it would use is no longer there.
    let gone = world.spawn_empty().id();
    let choice = MagicChoice { source: Source::Item(gone), aim: Aim::Body(foe) };
    world.entity_mut(member).insert(Ordered(Action::Magic(choice)));
    play(world, fight, 1);
    let turn = due_at(world, member, 1, 0);
    world.entity_mut(member).insert(turn);
    step(world);
    assert!(world.get::<Ordered>(member).unwrap().defends());
}

/// What happened at a round's end, in the order it happened.
#[derive(Resource, Default)]
struct Ending(Vec<&'static str>);

fn note(world: &mut World) {
    use crate::game::actors::spells::Aged;
    use crate::game::party::recovery::Elapsed;
    use round_end::{CloudsPulse, FightJudged, GroupsLeave, PartyLooksAgain, Rests, RoundEnded};
    world.init_resource::<Ending>();
    world.add_observer(|_: On<Elapsed>, mut end: ResMut<Ending>| end.0.push("time passes"));
    world.add_observer(|_: On<GroupsLeave>, mut end: ResMut<Ending>| end.0.push("groups leave"));
    world.add_observer(|_: On<Rests>, mut end: ResMut<Ending>| end.0.push("rest"));
    // By the time the clouds pulse, time has passed for everyone: a
    // condition of one round is gone.
    world.add_observer(
        |_: On<CloudsPulse>,
         members: Query<&Vitals, With<roster::PartyMember>>,
         mut end: ResMut<Ending>| {
            let afraid = members
                .iter()
                .any(|v| v.conditions.has(crate::game::characters::Condition::Afraid));
            end.0.push(if afraid { "pulses, time not passed" } else { "pulses" });
        },
    );
    // A cloud's last turn has pulsed before it ages.
    world.add_observer(
        |_: On<Aged>, hit: Query<&Vitals, With<Monster>>, mut end: ResMut<Ending>| {
            let pulsed = hit.iter().all(|v| v.is_dead());
            end.0.push(if pulsed { "spells age" } else { "spells age before the pulse" });
        },
    );
    world.add_observer(|_: On<FightJudged>, mut end: ResMut<Ending>| end.0.push("judged"));
    world.add_observer(|_: On<PartyLooksAgain>, mut end: ResMut<Ending>| end.0.push("looks"));
    world.add_observer(|_: On<RoundEnded>, mut end: ResMut<Ending>| end.0.push("ended"));
}

/// The round's end is one chain in the order of the rules: time passes, groups
/// leave, the idle rest, the clouds pulse and what they killed is settled, the
/// spells age, the fight is judged (the pulse's kill counted, so it is won),
/// the party looks again, and the round is ended.
#[test]
fn a_round_ends_in_the_order_of_the_rules() {
    use crate::game::characters::{Condition, Lasting};
    let (mut app, _, member) = app();
    let world = app.world_mut();
    note(world);
    let fight = world.spawn(Battle::default()).id();
    play(world, fight, 101);
    world.get_mut::<Round>(fight).unwrap().attacked = true;
    let foe = monster(world, fight, 3.0);
    lay(world, foe, SpellId::FIRESTORM, 1, 0);
    world.get_mut::<Vitals>(member).unwrap().afflict(Condition::Afraid, Lasting::Rounds(1));
    let before = world.get::<Character>(member).unwrap().experience;
    world.run_schedule(FixedUpdate);
    assert_eq!(
        world.resource::<Ending>().0,
        ["time passes", "groups leave", "rest", "pulses", "spells age", "judged", "looks", "ended"]
    );
    assert!(world.get_entity(fight).is_err(), "the fight is won, its last kill counted");
    assert_eq!(world.get::<Character>(member).unwrap().experience, before + 120);
}

/// A provoked neutral group stands hostile as the action starts, before what
/// provoked it lands; a dead monster or one under Turncoat provokes nobody.
/// Engagement is told, and the hostile group enters the fight at once.
#[test]
fn a_provoked_neutral_group_stands_hostile_and_enters_the_fight() {
    use crate::game::characters::{Condition, Lasting};
    use crate::game::world::hostility::{Allegiance, Provocation, Provoked, Stand};
    use crate::game::world::level::LevelOwned;
    use crate::game::world::population::{Special, Spellcasting, Temper};
    let (mut app, ..) = app();
    let world = app.world_mut();
    let level = world.query_filtered::<Entity, With<Level>>().single(world).unwrap();
    let fight = world.spawn(Battle::default()).id();
    play(world, fight, 1);
    let neutral = |world: &mut World| {
        let foe = monster(world, fight, 500.0);
        let group = world.get::<GroupMember>(foe).unwrap().0;
        world.entity_mut(group).insert(LevelOwned(level));
        world.entity_mut(foe).remove::<Combatant>().insert((
            Allegiance::of(&Species::default()),
            Temper(default()),
            Special(default()),
            Spellcasting { chance: 0, spells: [None; 10], silent_alchemy: false },
        ));
        world.trigger(Stand { group, stance: Some(Stance::Neutral) });
        world.flush();
        assert_eq!(world.get::<Side>(foe).unwrap().own(), Stance::Neutral);
        (foe, group)
    };
    // Under Turncoat, or dead, it provokes nobody.
    let (turned, _) = neutral(world);
    world.get_mut::<Vitals>(turned).unwrap().afflict(Condition::Turncoat, Lasting::Rounds(5));
    world.trigger(Provocation { monster: turned });
    world.flush();
    assert_eq!(world.get::<Side>(turned).unwrap().own(), Stance::Neutral);
    let (foe, _) = neutral(world);
    world.trigger(Provocation { monster: foe });
    world.flush();
    assert_eq!(world.get::<Side>(foe).unwrap().own(), Stance::Hostile);
    let provoked: Vec<_> = world.resource_mut::<Messages<Provoked>>().drain().collect();
    assert_eq!(provoked, [Provoked(turned), Provoked(foe)]);
    // Engagement is told the same step: the provoked hostile group enters,
    // far off as it is, which nothing else brings it to do.
    world.run_system_cached(engage::engage).unwrap();
    assert!(world.resource_mut::<Messages<engage::Engaged>>().drain().next().is_none());
    world.resource_mut::<Messages<Provoked>>().write(Provoked(foe));
    world.run_system_cached(engage::engage).unwrap();
    let engaged: Vec<_> = world.resource_mut::<Messages<engage::Engaged>>().drain().collect();
    assert!(engaged.iter().any(|engage::Engaged(group)| group.contains(&foe)));
}

/// A utility cast refused in a fight makes its caster defend.
#[test]
fn a_utility_cast_refused_in_a_fight_makes_its_caster_defend() {
    use crate::game::magic::Utility;
    let (mut app, _, member) = app();
    let world = app.world_mut();
    let fight = world.spawn(Battle::default()).id();
    let _foe = monster(world, fight, 3.0);
    refresh(world);
    // Identify Item cannot be cast in a fight, and not at a foe.
    let choice = MagicChoice {
        source: Source::Spell { spell: SpellId::IDENTIFY_ITEM, power: 1 },
        aim: Aim::Point(Vec3::ZERO),
    };
    world.entity_mut(member).insert(Ordered(Action::Magic(choice)));
    world.trigger(Utility { caster: member, spell: SpellId::IDENTIFY_ITEM, choice });
    world.flush();
    assert!(world.get::<Ordered>(member).unwrap().defends());
    let news: Vec<_> = world.resource_mut::<Messages<MagicNews>>().drain().collect();
    assert!(matches!(news.as_slice(), [MagicNews::Refused { refusal: Refusal::WrongTime, .. }]));
}

/// Outside a fight, time passing ages conditions, spells laid and a stance
/// set on a group, each by its own observer of what time did.
#[test]
fn outside_a_fight_time_passing_ages_conditions_spells_and_a_set_stance() {
    use crate::game::characters::{Condition, Lasting};
    use crate::game::world::hostility::{Allegiance, Stand};
    let (mut app, party, member) = app();
    let world = app.world_mut();
    world.init_resource::<crate::game::world::search::Search>();
    world.entity_mut(party).insert(avian3d::prelude::LinearVelocity::default());
    *world.resource_mut::<GameClock>() = GameClock::at(10 * crate::game::time::DAY_MS);
    let group = world.spawn(MonsterGroup::default()).id();
    world.spawn((Vitals::full(20, 100), Allegiance::of(&Species::default()), GroupMember(group)));
    world.trigger(Stand { group, stance: Some(Stance::Hostile) });
    world.flush();
    assert!(world.get::<MonsterGroup>(group).unwrap().set.is_some());
    lay(world, member, SpellId::BLESS, 1, 0);
    world.get_mut::<Vitals>(member).unwrap().afflict(Condition::Afraid, Lasting::Rounds(1));
    // A whole aging tick of game time, at twelve times real time.
    world.resource_mut::<Time>().advance_by(std::time::Duration::from_secs(10));
    world.run_system_cached(crate::game::time::pass_time).unwrap();
    world.flush();
    assert!(!world.get::<Vitals>(member).unwrap().conditions.has(Condition::Afraid));
    assert!(!is_laid(world, member, SpellId::BLESS));
    assert!(world.get::<MonsterGroup>(group).unwrap().set.is_none(), "the stance wore off");
}
