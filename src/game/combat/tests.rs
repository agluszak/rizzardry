//! Headless transition tests using the production plugins and settlement schedule.
use super::*;
use crate::content::characters::Realm;
use crate::content::items::{AttackMode, Dice};
use crate::content::level::LevelDefinition;
use crate::content::monster::MonsterBody;
use crate::content::species::{
    Disposition, MonsterAttack, MonsterCatalog, MonsterWeapon, Reach, Species,
};
use crate::content::spells::{SpellCatalog, SpellDefinition, SpellId};
use crate::game::actors::{Enchantment, Laid, Laying};
use crate::game::characters::{Gender, Profession, Race};
use crate::game::hits::definitions::Rows;
use crate::game::hits::effects::Strike;
use crate::game::hits::{self, Deed, Effect};
use crate::game::magic::casting::{Cast, MagicChoice, MagicNews, Source, learned_chance};
use crate::game::magic::landed::Magical;
use crate::game::magic::may::Refusal;
use crate::game::magic::reach::Aim;
use crate::game::magic::roll::Pace;
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
            sound_class: None,
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
    world.flush();
    (app, party, member)
}

fn monster(world: &mut World, battle: Entity, distance: f32) -> Entity {
    let group = world.spawn(MonsterGroup::default()).id();
    monster_in(world, battle, distance, group)
}

fn monster_in(world: &mut World, battle: Entity, distance: f32, group: Entity) -> Entity {
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
    let rows = Rows::Spell(SpellId::ANTI_MAGIC);
    let strength = Strength { power: 1, against: 1, boost: 0 };
    world.spawn((Deed::at(Strike { rows, strength, blow: None }, [member, second]), Magical));
    world.flush();
    assert!(!is_laid(world, party, SpellId::MAGIC_SCREEN));
    let news: Vec<_> = world.resource_mut::<Messages<MagicNews>>().drain().collect();
    let [MagicNews::Landed { effects }] = news.as_slice() else { panic!("{news:?}") };
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
    let rows = Rows::Pulse(SpellId::FIRESTORM);
    let strength = Strength { power: 1, against: 100, boost: 0 };
    world.spawn(Deed::flying(Strike { rows, strength, blow: None }, [(member, Some(1.0))]));
    let before = world.get::<Vitals>(member).unwrap().health.current;
    // A second's flight is two steps of half a second.
    world.resource_mut::<Time>().advance_by(std::time::Duration::from_millis(500));
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
        world.trigger(crate::game::characters::Fell { body });
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
    let pace = Pace { difficulty: crate::game::difficulty::Difficulty::Normal, moment: 0 };
    assert_eq!(learned_chance(world.get::<Character>(member).unwrap(), spell, 1, pace, false), 0);
    let choice = MagicChoice {
        source: Source::Spell { spell: SpellId::ENERGY_BLAST, power: 1 },
        aim: Aim::Body(foe),
    };
    world.trigger(Cast { caster: member, spell: SpellId::ENERGY_BLAST, choice });
    world.flush();
    assert_eq!(magic::test_points(world, member, Realm::Fire), points - 1);
    let group = world.get::<GroupMember>(foe).unwrap().0;
    assert!(world.get::<Provoked>(group).is_some(), "its group is provoked");
    let news: Vec<_> = world.resource_mut::<Messages<MagicNews>>().drain().collect();
    assert!(
        matches!(news.as_slice(), [MagicNews::Done(report)] if report.outcome == Outcome::Cast),
        "{news:?}"
    );
    assert_eq!(
        world.query::<&hits::Deed>().iter(world).count(),
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
        matches!(news.as_slice(), [MagicNews::Landed { effects }] if !effects.is_empty()),
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
    world.trigger(Cast { caster: member, spell: SpellId::ENERGY_BLAST, choice });
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

/// A provoked neutral group stands hostile as the action starts, before what
/// provoked it lands; a dead monster or one under Turncoat provokes nobody.
/// Engagement is told, and the hostile group enters the fight at once.
#[test]
fn a_provoked_neutral_group_stands_hostile_and_enters_the_fight() {
    use crate::game::characters::{Condition, Lasting};
    use crate::game::world::hostility::{Allegiance, Provocation, Stand};
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
    world.run_system_cached(engage::engage).unwrap();
    assert!(world.resource_mut::<Messages<engage::Engaged>>().drain().next().is_none());
    let (foe, _) = neutral(world);
    world.trigger(Provocation { monster: foe });
    world.flush();
    assert_eq!(world.get::<Side>(foe).unwrap().own(), Stance::Hostile);
    // Engagement looks at it the same step: the provoked hostile group
    // enters, far off as it is, which nothing else brings it to do.
    world.run_system_cached(engage::engage).unwrap();
    let engaged: Vec<_> = world.resource_mut::<Messages<engage::Engaged>>().drain().collect();
    assert!(engaged.iter().any(|engage::Engaged(group)| group.contains(&foe)));
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

/// A spell for the member to cast: aimed as `targeting`, of this level,
/// costing one point, and doing `dice`.
fn spell_for(
    world: &mut World,
    id: SpellId,
    targeting: crate::content::spells::Targeting,
    level: u32,
    dice: Dice,
) {
    use crate::content::spells::{Reach, Spellbook, Usable};
    let mut spells = std::mem::take(&mut world.resource_mut::<Catalogs>().spells.0);
    spells[id.0 as usize] = SpellDefinition {
        id,
        realm: Some(Realm::Divine),
        level,
        cost: 1,
        books: Spellbook::Divinity.into(),
        targeting,
        reach: Reach::Long,
        usable: Usable::Always,
        dice: Some(dice),
        ..default()
    };
    // With the facts the table gives it (whether it can backfire).
    world.resource_mut::<Catalogs>().spells = SpellCatalog::new(spells);
}

/// The member knows these spells, has this skill in Divinity and Divine
/// magic and a hundred points to cast them with.
fn teach(world: &mut World, member: Entity, skill: u32, spells: &[SpellId]) {
    use crate::content::characters::Skill;
    let mut character = world.get_mut::<Character>(member).unwrap();
    character.skill_points = default();
    character.skill_points[Skill::Divinity] = skill;
    character.skill_points[Realm::Divine.skill()] = skill;
    world
        .entity_mut(member)
        .insert((LearnedSpells(spells.iter().copied().collect()), magic::SpellPoints::default()));
    magic::test_set_points(world, member, Realm::Divine, 100);
}

/// A cast, resolved in one pass; what it reported.
fn cast_in(
    world: &mut World,
    member: Entity,
    (spell, choice): (SpellId, MagicChoice),
) -> Vec<MagicNews> {
    world.trigger(Cast { caster: member, spell, choice });
    world.flush();
    world.resource_mut::<Messages<MagicNews>>().drain().collect()
}

fn spell_choice(spell: SpellId, power: u32, aim: Aim) -> (SpellId, MagicChoice) {
    (spell, MagicChoice { source: Source::Spell { spell, power }, aim })
}

/// A failure chance is raised for a cast late in a round, and is not held to
/// a hundred. An untrained priest casting a level 7 spell at power 7 fails
/// 70 + 7 * (18 - 1 - 1 + 7) = 231 in a hundred; on Expert (pace 40) at the
/// round's last moment that is (100 + 50 - 40) * 231 * 2 / 100 = 508, a third
/// of it, 169, the chance to backfire: every cast backfires. A Stamina cast
/// that does tires its caster by the spell's level and power, 7 + 7 = 14, and
/// then by its dice, 7d1 = 7, instead of restoring (`Magic Effects.cpp:3246`).
#[test]
fn a_late_cast_backfires_and_a_backfired_stamina_tires() {
    use crate::content::spells::Targeting;
    use crate::game::difficulty::Difficulty;
    let (mut app, _, member) = app();
    let world = app.world_mut();
    world.insert_resource(Difficulty::Expert);
    spell_for(world, SpellId::STAMINA, Targeting::Ally, 7, Dice { base: 0, count: 1, sides: 1 });
    teach(world, member, 0, &[SpellId::STAMINA]);
    let fight = world.spawn(Battle::default()).id();
    play(world, fight, 100);
    for _ in 0..10 {
        world.get_mut::<Vitals>(member).unwrap().stamina = crate::game::characters::Pool::full(100);
        magic::test_set_points(world, member, Realm::Divine, 100);
        let choice = spell_choice(SpellId::STAMINA, 7, Aim::Body(member));
        let news = cast_in(world, member, choice);
        assert!(
            matches!(news.first(), Some(MagicNews::Done(report)) if report.outcome == magic::Outcome::Backfired),
            "{news:?}"
        );
        assert_eq!(world.get::<Vitals>(member).unwrap().stamina.current, 100 - 14 - 7);
    }
}

/// A silenced member can still cast an Alchemy spell if they have the skill
/// (`Magic.cpp:138-146`); any other spell, or an Alchemy one without it, is
/// refused.
#[test]
fn silence_stops_all_but_alchemy_in_the_hands_of_an_alchemist() {
    use crate::content::characters::Skill;
    use crate::content::spells::Targeting;
    use crate::game::characters::{Condition, Lasting};
    let (mut app, _, member) = app();
    let world = app.world_mut();
    // Heal Wounds is in the Alchemy book too; this one is Divinity's alone.
    spell_for(
        world,
        SpellId::ENERGY_BLAST,
        Targeting::Ally,
        1,
        Dice { base: 0, count: 0, sides: 1 },
    );
    teach(world, member, 100, &[SpellId::HEAL_WOUNDS, SpellId::ENERGY_BLAST]);
    world.get_mut::<Vitals>(member).unwrap().afflict(Condition::Silenced, Lasting::Rounds(9));
    let casts = |world: &mut World, spell| {
        let choice = spell_choice(spell, 1, Aim::Body(member));
        let news = cast_in(world, member, choice);
        matches!(news.as_slice(), [MagicNews::Refused { refusal: Refusal::Silenced, .. }])
    };
    assert!(casts(world, SpellId::HEAL_WOUNDS), "no Alchemy, no voice");
    world.get_mut::<Character>(member).unwrap().skill_points[Skill::Alchemy] = 5;
    assert!(!casts(world, SpellId::HEAL_WOUNDS), "an alchemist is not silenced for Alchemy");
    assert!(casts(world, SpellId::ENERGY_BLAST), "but is for the rest");
}

/// In a fight only what a member carries is within reach, not the party's
/// pool (`Magic.cpp:1317-1319`).
#[test]
fn a_pooled_item_cannot_be_used_in_a_fight() {
    let (mut app, party, member) = app();
    let world = app.world_mut();
    let item = world.spawn(crate::game::items::Pooled(party)).id();
    let choice =
        (SpellId::HEAL_WOUNDS, MagicChoice { source: Source::Item(item), aim: Aim::Body(member) });
    let fight = world.spawn(Battle::default()).id();
    let news = cast_in(world, member, choice);
    assert!(matches!(news.as_slice(), [MagicNews::Refused { refusal: Refusal::CannotUse, .. }]));
    world.despawn(fight);
    let news = cast_in(world, member, choice);
    assert!(!matches!(news.as_slice(), [MagicNews::Refused { refusal: Refusal::CannotUse, .. }]));
}

/// A monster's cast tires it by the spell's level and power, 1 + 2
/// (`Monster.cpp:3914-3917`); the cast costs it nothing else.
#[test]
fn a_monsters_cast_tires_it() {
    let (mut app, ..) = app();
    let world = app.world_mut();
    let fight = world.spawn(Battle::default()).id();
    let caster = monster(world, fight, 3.0);
    refresh(world);
    let source = Source::Innate { spell: SpellId::HEAL_WOUNDS, power: 2, budget: 100, level: 5 };
    let choice = (SpellId::HEAL_WOUNDS, MagicChoice { source, aim: Aim::Body(caster) });
    let news = cast_in(world, caster, choice);
    assert!(matches!(news.first(), Some(MagicNews::Done(_))), "{news:?}");
    assert_eq!(world.get::<Vitals>(caster).unwrap().stamina.current, 100 - 3);
}

/// A spell at an enemy group takes the whole group of the foe aimed at, near
/// or far (`Magic.cpp:3714-3733`).
#[test]
fn a_group_spell_reaches_the_whole_group_however_far() {
    use crate::content::spells::Targeting;
    let (mut app, _, member) = app();
    let world = app.world_mut();
    spell_for(
        world,
        SpellId::ENERGY_BLAST,
        Targeting::EnemyGroup,
        1,
        Dice { base: 5, count: 0, sides: 1 },
    );
    teach(world, member, 100, &[SpellId::ENERGY_BLAST]);
    let fight = world.spawn(Battle::default()).id();
    let group = world.spawn(MonsterGroup::default()).id();
    let near = monster_in(world, fight, 3.0, group);
    let far = monster_in(world, fight, 40.0, group);
    refresh(world);
    let choice = spell_choice(SpellId::ENERGY_BLAST, 1, Aim::Body(near));
    cast_in(world, member, choice);
    assert_eq!(world.get::<Vitals>(near).unwrap().health.current, 15);
    assert_eq!(world.get::<Vitals>(far).unwrap().health.current, 15, "forty metres away");
}

/// What a leeching spell drained is the magnitude cut to the victim's
/// health, whether or not a Guardian Angel took the blow (`Magic Effects.cpp:1626-1650`):
/// a Lifesteal of 10 on a foe the angel guards still heals 70%, 7.
#[test]
fn a_drain_gives_back_what_a_guardian_angel_took() {
    use crate::content::spells::Targeting;
    let (mut app, _, member) = app();
    let world = app.world_mut();
    spell_for(
        world,
        SpellId::LIFESTEAL,
        Targeting::Enemy,
        1,
        Dice { base: 10, count: 0, sides: 1 },
    );
    teach(world, member, 100, &[SpellId::LIFESTEAL]);
    let fight = world.spawn(Battle::default()).id();
    let foe = monster(world, fight, 3.0);
    refresh(world);
    lay(world, foe, SpellId::GUARDIAN_ANGEL, 10, 100);
    world.get_mut::<Vitals>(member).unwrap().health.current = 1;
    let choice = spell_choice(SpellId::LIFESTEAL, 1, Aim::Body(foe));
    cast_in(world, member, choice);
    assert_eq!(world.get::<Vitals>(foe).unwrap().health.current, 20, "the angel took it");
    assert_eq!(world.get::<Vitals>(member).unwrap().health.current, 1 + 7);
}

/// An instrument's toll, its spell's level times 8 plus 20, tires like any
/// other fatigue: a Slowed player pays a quarter less (`Combat.cpp:2347-2354`,
/// `PC Item.cpp:1217`). Level 1 asks 28; slowed that is 28 - 25 * 28 / 100 =
/// 21. Played well or badly, the toll is the same.
#[test]
fn an_instruments_toll_is_scaled_like_other_fatigue() {
    use crate::content::characters::Skill;
    use crate::content::items::{EquipClass, ItemCatalog, ItemDefinition, ItemId, UseType};
    use crate::content::spells::ItemSpell;
    use crate::game::characters::{Condition, Lasting, Pool};
    use crate::game::items::{Place, Put, make};
    use bevy::ecs::system::RunSystemOnce;
    let (mut app, _, member) = app();
    app.add_plugins(crate::game::items::plugin);
    let world = app.world_mut();
    let lute = ItemDefinition {
        id: ItemId(0),
        name: "Lute".into(),
        class: EquipClass::Instrument,
        use_type: UseType::Instrument,
        professions: enumset::EnumSet::all(),
        races: enumset::EnumSet::all(),
        genders: enumset::EnumSet::all(),
        spell: Some(ItemSpell { spell: SpellId::HEAL_WOUNDS, power: 0 }),
        ..default()
    };
    world.resource_mut::<Catalogs>().items = ItemCatalog(vec![lute]);
    let item = world
        .run_system_once(|mut commands: Commands, content: Res<Catalogs>| {
            make(&mut commands, &content.items[ItemId(0)], 1, true)
        })
        .unwrap();
    world.flush();
    world.trigger(Put { item, at: Place::Pack { member, slot: 0 } });
    world.flush();
    world.get_mut::<Vitals>(member).unwrap().afflict(Condition::Slowed, Lasting::Rounds(9));
    let choice =
        (SpellId::HEAL_WOUNDS, MagicChoice { source: Source::Item(item), aim: Aim::Body(member) });
    // A virtuoso plays it, a beginner (with no skill) cannot.
    for (music, played) in [(125, true), (0, false)] {
        world.get_mut::<Character>(member).unwrap().skill_points[Skill::Music] = music;
        world.get_mut::<Vitals>(member).unwrap().stamina = Pool::full(100);
        let news = cast_in(world, member, choice);
        let badly =
            matches!(news.as_slice(), [MagicNews::Refused { refusal: Refusal::PlayedBadly, .. }]);
        assert_eq!(badly, !played, "{news:?}");
        assert_eq!(world.get::<Vitals>(member).unwrap().stamina.current, 100 - 21, "{news:?}");
    }
}
