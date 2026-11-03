//! Headless transition tests using the production plugins and settlement schedule.
use super::*;
use crate::content::characters::Realm;
use crate::content::items::{AttackMode, Dice};
use crate::content::level::LevelDefinition;
use crate::content::monster::MonsterBody;
use crate::content::species::{Disposition, MonsterAttack, MonsterCatalog, MonsterWeapon};
use crate::content::spells::{SpellCatalog, SpellDefinition, SpellId};
use crate::game::characters::{Gender, Profession, Race};
use crate::game::hits::definitions::{Definitions, Uses};
use crate::game::hits::{self, Act, Deed, Effect};
use crate::game::magic::enchantments::{Enchantment, PartyEffects};
use crate::game::magic::{self, LearnedSpells, Strength};
use crate::game::party::body::PartyBody;
use crate::game::party::roster;
use crate::game::world::footing;
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
    hits::definitions::define(world);
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
    world.run_schedule(FixedUpdate);
    assert!(world.get::<Vitals>(member).unwrap().is_dead());
    assert!(
        world.resource_mut::<Messages<CombatNews>>().drain().any(|n| matches!(n, CombatNews::Lost))
    );
}

#[test]
fn killing_a_turned_ally_awards_nothing_but_a_revived_enemy_counts() {
    use crate::game::characters::{Condition, Lasting};
    let (mut app, _, _) = app();
    let world = app.world_mut();
    let battle = world.spawn(Battle::default()).id();
    let actor = monster(world, battle, 3.0);
    refresh(world);
    world.get_mut::<Vitals>(actor).unwrap().inflict(Condition::Turncoat, Lasting::Rounds(5));
    hits::refresh_capabilities(world);
    world.get_mut::<Vitals>(actor).unwrap().slay();
    world.run_system_cached(crate::game::characters::settle_deaths).unwrap();
    hits::refresh_capabilities(world);
    world.run_schedule(Settlement);
    assert_eq!(world.get::<Spoils>(battle).unwrap().experience, 0);
    world.get_mut::<Vitals>(actor).unwrap().health.current = 1;
    world.run_system_cached(crate::game::characters::settle_deaths).unwrap();
    hits::refresh_capabilities(world);
    world.get_mut::<Vitals>(actor).unwrap().slay();
    world.run_schedule(Settlement);
    assert_eq!(world.get::<Spoils>(battle).unwrap().kills, 1);
    assert_eq!(world.get::<Spoils>(battle).unwrap().experience, 120);
}

/// A monster's move reads where the party stands this fixed step: bodies
/// are placed before the move is steered, so a party that stepped into
/// reach ends the move on the same step.
#[test]
fn a_move_toward_the_party_sees_where_the_party_stands_this_step() {
    use approach::{CombatMove, MoveGoal};
    use bevy_landmass::AgentTarget3d;
    let (mut app, party, _) = app();
    let world = app.world_mut();
    let fight = world.spawn(Battle::default()).id();
    let actor = monster(world, fight, 10.0);
    refresh(world);
    world.entity_mut(actor).insert((
        CombatMove { budget: 100.0, moved: 0.0, goal: MoveGoal::Party },
        AgentTarget3d::None,
    ));
    world.run_schedule(FixedPreUpdate);
    assert_eq!(*world.get::<AgentTarget3d>(actor).unwrap(), AgentTarget3d::Entity(party));
    // The party steps up to it, within its claws' two metres.
    world.get_mut::<Position>(party).unwrap().0.z = 9.0;
    world.run_schedule(FixedPreUpdate);
    assert!(world.get::<CombatMove>(actor).is_none(), "in reach, the move is over");
    assert_eq!(*world.get::<AgentTarget3d>(actor).unwrap(), AgentTarget3d::None);
}
