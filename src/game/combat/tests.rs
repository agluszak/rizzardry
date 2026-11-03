//! Headless transition tests using the production plugins and settlement schedule.
use super::*;
use crate::content::characters::{Realm, Skill};
use crate::content::items::{AttackMode, Dice};
use crate::content::level::LevelDefinition;
use crate::content::monster::MonsterBody;
use crate::content::species::{Disposition, MonsterAttack, MonsterCatalog, MonsterWeapon};
use crate::content::spells::{SpellCatalog, SpellDefinition, SpellId};
use crate::game::characters::{Gender, Profession, Race};
use crate::game::hits::prototypes::{Definitions, Uses};
use crate::game::hits::{self, Act, Deed, Effect, HitOf};
use crate::game::magic::casting::{Aim, MagicChoice, Practice, Recast, Source};
use crate::game::magic::enchantments::{Enchantment, PartyEffects};
use crate::game::magic::{self, LearnedSpells, Strength};
use crate::game::party::body::PartyBody;
use crate::game::party::roster;
use crate::game::world::footing::{self, Footing};
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
        .init_resource::<crate::game::inventory::ItemIds>()
        .init_resource::<avian3d::collider_tree::ColliderTrees>()
        .init_resource::<Assets<LevelDefinition>>()
        .init_resource::<Assets<MonsterBody>>()
        .init_resource::<Assets<WalkerMotionConfig>>()
        .add_message::<crate::game::factions::FactionNews>()
        .add_message::<crate::game::party::DamageApplied>()
        .add_message::<Identified>()
        .add_plugins((CombatPlugin, magic::MagicPlugin, hits::HitsPlugin));
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
        spells: SpellCatalog(spells),
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
    hits::prototypes::define(world);
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
            Walker { speed: 1.0, float_height: 0.0 },
            Combatant(battle),
            GroupMember(group),
            Worth(120),
        ))
        .id()
}

fn enchantment(spell: SpellId, turns: u32) -> Enchantment {
    Enchantment { spell, power: 1, boost: 0, turns, absorption: 0 }
}

fn refresh(world: &mut World) {
    world.run_system_cached(hostility::judge_sides).unwrap();
    world.run_system_cached(footing::place_bodies).unwrap();
    world.run_system_cached(hits::defences::judge_defences).unwrap();
    world.run_system_cached(fighters::judge_fighters).unwrap();
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
    world.get_mut::<PartyEffects>(party).unwrap().0.apply(enchantment(SpellId::MAGIC_SCREEN, 10));
    let definition = world.resource::<Definitions>().spell(SpellId::ANTI_MAGIC).unwrap();
    // Keep the completed deed to inspect its outcomes, as an attack does.
    let act = world
        .spawn((
            Deed::new(Act::Swing),
            Uses(definition),
            Strength { power: 1, against: 1, boost: 0 },
        ))
        .id();
    hits::spells::reach(&mut world.commands(), act, &[(member, None), (second, None)]);
    world.flush();
    hits::run_hits(world);
    assert!(world.get::<PartyEffects>(party).unwrap().0.get(SpellId::MAGIC_SCREEN).is_none());
    let drained = world
        .get::<Deed>(act)
        .unwrap()
        .effects
        .iter()
        .filter(|e| matches!(e, Effect::Drained { .. }))
        .count();
    assert_eq!(drained, 2, "each pool is drained once, not once per body reach");
    hits::run_hits(world);
    assert_eq!(world.get::<Deed>(act).unwrap().effects.len(), drained);
    assert_eq!(world.query::<&HitOf>().iter(world).count(), 0);
}

#[test]
fn completed_cast_consumes_practice_and_one_repeat_request() {
    let (mut app, _, member) = app();
    let world = app.world_mut();
    world.get_mut::<Vitals>(member).unwrap().health.current = 1;
    let choice = MagicChoice {
        source: Source::Spell { spell: SpellId::HEAL_WOUNDS, power: casting::AUTO_POWER },
        aim: Aim::Body(member),
    };
    let mut expected = world.get::<Character>(member).unwrap().clone();
    let mut rng = world.resource::<GameRng>().clone();
    expected.practise(Skill::Divinity, 50, &mut rng);
    let act = world
        .spawn((
            Deed::new(Act::Swing),
            Practice { caster: member, skills: vec![(Skill::Divinity, 50)], sure: true },
            Recast { caster: member, choice, victims: vec![member] },
        ))
        .id();
    hits::run_hits(world);
    assert_eq!(*world.get::<Character>(member).unwrap(), expected);
    assert_eq!(*world.resource::<GameRng>(), rng);
    assert_eq!(world.resource_mut::<Messages<magic::MagicRequest>>().drain().count(), 1);
    assert!(world.get::<Practice>(act).is_none());
    assert!(world.get::<Recast>(act).is_none());
    hits::run_hits(world);
    assert_eq!(*world.resource::<GameRng>(), rng);
    assert_eq!(world.resource_mut::<Messages<magic::MagicRequest>>().drain().count(), 0);
}

#[test]
fn closing_in_preserves_the_attack_for_the_resumed_turn() {
    let (mut app, _, member) = app();
    let world = app.world_mut();
    let fight = world.spawn(Battle::default()).id();
    world.get_mut::<Round>(fight).unwrap().phase = RoundPhase::Executing(10);
    let actor = monster(world, fight, 20.0);
    refresh(world);
    let arms = world.get::<fighters::Arms>(actor).unwrap();
    let turn = Turn {
        phase: 10,
        attacks: enum_map::EnumMap::from_fn(|slot| arms.attacks(slot)),
        due: crate::game::inventory::WeaponSlot::Primary,
        weapons: arms.weapons(),
        lightning: None,
        moved: false,
        special: false,
        berserk: false,
        waited: false,
    };
    world.entity_mut(actor).insert((
        turn,
        turn::Acting,
        turn::Fights { waits: false, berserk: false },
    ));
    world.run_system_cached(swings::open_attacks).unwrap();
    assert!(world.get::<approach::CombatMove>(actor).is_some());
    assert_eq!(
        world.get::<Turn>(actor).unwrap().attacks[crate::game::inventory::WeaponSlot::Primary],
        1
    );
    // Navigation has reached the party. Resume the same accepted turn.
    let there = world.get::<Footing>(member).unwrap().feet;
    world.get_mut::<Transform>(actor).unwrap().translation = there;
    world.entity_mut(actor).remove::<approach::CombatMove>().insert(turn::Acting);
    refresh(world);
    world.run_system_cached(turn::begin_turns).unwrap();
    world.run_system_cached(swings::open_attacks).unwrap();
    assert_eq!(world.query::<&swings::AimsAt>().iter(world).count(), 1);
    assert_eq!(
        world.get::<Turn>(actor).unwrap().attacks[crate::game::inventory::WeaponSlot::Primary],
        0
    );
}

#[test]
fn final_cloud_pulse_settles_and_credits_the_kill_before_victory() {
    let (mut app, _, member) = app();
    let world = app.world_mut();
    let fight = world.spawn(Battle::default()).id();
    world.get_mut::<Round>(fight).unwrap().phase = RoundPhase::Executing(101);
    world.get_mut::<Round>(fight).unwrap().attacked = true;
    let actor = monster(world, fight, 3.0);
    world.get_mut::<Vitals>(actor).unwrap().enchantments.apply(enchantment(SpellId::FIRESTORM, 1));
    let before = world.get::<Character>(member).unwrap().experience;
    world.run_schedule(FixedUpdate);
    assert!(world.get::<Dead>(actor).is_some());
    assert!(world.get::<Battle>(fight).is_none());
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
    world.get_mut::<Round>(fight).unwrap().phase = RoundPhase::Executing(101);
    let actor = monster(world, fight, 3.0);
    let mut vitals = world.get_mut::<Vitals>(actor).unwrap();
    vitals.enchantments.apply(enchantment(SpellId::FIRESTORM, 1));
    vitals
        .enchantments
        .apply(Enchantment { absorption: 100, ..enchantment(SpellId::GUARDIAN_ANGEL, 1) });
    world.run_schedule(FixedUpdate);
    assert_eq!(world.get::<Vitals>(actor).unwrap().health.current, 20);
    assert!(
        world.get::<Vitals>(actor).unwrap().enchantments.get(SpellId::GUARDIAN_ANGEL).is_none()
    );
}

#[test]
fn projections_change_only_with_their_values_and_refresh_after_removal() {
    fn changes(
        arms: Query<(), Changed<fighters::Arms>>,
        resistance: Query<(), Changed<hits::defences::Resistances>>,
        footing: Query<(), Changed<Footing>>,
    ) -> (usize, usize, usize) {
        (arms.iter().count(), resistance.iter().count(), footing.iter().count())
    }
    let (mut app, party, member) = app();
    let world = app.world_mut();
    let fight = world.spawn(Battle::default()).id();
    let actor = monster(world, fight, 3.0);
    refresh(world);
    assert_eq!(world.run_system_cached(changes).unwrap(), (2, 2, 2));
    refresh(world);
    assert_eq!(world.run_system_cached(changes).unwrap(), (0, 0, 0));
    // Hurt changes fatigue/armour inputs but not this body's weapon or footing.
    world.get_mut::<Vitals>(actor).unwrap().health.current -= 1;
    refresh(world);
    assert_eq!(world.run_system_cached(changes).unwrap(), (0, 0, 0));
    world
        .get_mut::<PartyEffects>(party)
        .unwrap()
        .0
        .apply(enchantment(SpellId::ENCHANTED_BLADE, 10));
    refresh(world);
    assert_eq!(world.run_system_cached(changes).unwrap(), (1, 0, 0));
    world.get_mut::<Position>(party).unwrap().x += 2.0;
    refresh(world);
    assert_eq!(world.run_system_cached(changes).unwrap(), (0, 0, 1));

    world.entity_mut(actor).insert(approach::CombatMove {
        budget: 10.0,
        moved: 0.0,
        goal: approach::MoveGoal::Party,
    });
    refresh(world);
    assert!(world.get::<fighters::ArmourClass>(actor).unwrap().distracted);
    world.entity_mut(actor).remove::<approach::CombatMove>();
    refresh(world);
    assert!(!world.get::<fighters::ArmourClass>(actor).unwrap().distracted);
    world.entity_mut(member).insert(Ordered(Action::Defend));
    refresh(world);
    assert!(world.get::<fighters::ArmourClass>(member).unwrap().braced);
    world.entity_mut(member).remove::<Ordered>();
    refresh(world);
    assert!(!world.get::<fighters::ArmourClass>(member).unwrap().braced);

    let baseline = *world.get::<hits::defences::Resistances>(member).unwrap();
    world.get_mut::<PartyEffects>(party).unwrap().0.apply(enchantment(SpellId::MAGIC_SCREEN, 10));
    refresh(world);
    assert_ne!(*world.get::<hits::defences::Resistances>(member).unwrap(), baseline);
    world.entity_mut(member).remove::<roster::PartyMember>();
    world.run_system_cached(hits::defences::judge_defences).unwrap();
    assert_eq!(*world.get::<hits::defences::Resistances>(member).unwrap(), baseline);
}

#[test]
fn missile_arrival_settles_defeat_before_any_actor_can_start() {
    let (mut app, _, member) = app();
    let world = app.world_mut();
    let fight = world.spawn(Battle::default()).id();
    let actor = monster(world, fight, 3.0);
    world.get_mut::<Round>(fight).unwrap().phase = RoundPhase::Executing(1);
    refresh(world);
    let arms = world.get::<fighters::Arms>(actor).unwrap();
    let turn = Turn {
        phase: 1,
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
    let act = world
        .spawn((
            Deed::new(Act::Fight),
            Uses(definition),
            Strength { power: 1, against: 100, boost: 0 },
        ))
        .id();
    hits::spells::reach(&mut world.commands(), act, &[(member, Some(1.0))]);
    world.flush();
    let before = world.get::<Vitals>(member).unwrap().health.current;
    world.resource_mut::<Time>().advance_by(std::time::Duration::from_millis(500));
    world.run_schedule(FixedUpdate);
    assert_eq!(world.get::<Vitals>(member).unwrap().health.current, before);
    assert_eq!(world.get::<Turn>(actor).unwrap().phase, 1);
    world.run_schedule(FixedUpdate);
    assert!(world.get::<Dead>(member).is_some());
    assert!(world.get::<Battle>(fight).is_none());
    assert!(world.get::<turn::Acting>(actor).is_none());
    assert!(!world.resource::<Messages<CombatNews>>().is_empty());
}

#[test]
fn idle_resolution_does_not_execute_or_mutate_definitions() {
    use hits::effects::Outcome;
    use hits::prototypes::{EffectOf, Prismic};
    let (mut app, _, _) = app();
    let world = app.world_mut();
    let rng = world.resource::<GameRng>().clone();
    let choices = world.query_filtered::<(), (With<EffectOf>, With<Prismic>)>().iter(world).count();
    assert!(choices > 0);
    for _ in 0..3 {
        world.run_schedule(hits::Hits);
        hits::run_hits(world);
        assert_eq!(*world.resource::<GameRng>(), rng);
        assert_eq!(
            world.query_filtered::<(), (With<EffectOf>, With<Prismic>)>().iter(world).count(),
            choices
        );
        assert_eq!(
            world.query_filtered::<(), (With<EffectOf>, With<Outcome>)>().iter(world).count(),
            0
        );
    }
}
