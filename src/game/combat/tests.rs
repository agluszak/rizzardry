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
        .init_resource::<crate::game::inventory::ItemIds>()
        .init_resource::<avian3d::collider_tree::ColliderTrees>()
        .init_resource::<Assets<LevelDefinition>>()
        .init_resource::<Assets<MonsterBody>>()
        .init_resource::<Assets<WalkerMotionConfig>>()
        .add_message::<crate::game::factions::FactionNews>()
        .add_message::<Identified>()
        .add_message::<crate::game::party::recovery::Elapsed>()
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
    world.write_message(Release {
        deed: Deed::new(Act::Fight, Strike { definition, strength, blow: None }),
        victims: vec![(member, None), (second, None)],
    });
    refresh(world);
    world.run_system_cached(hits::resolve).unwrap();
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
    world.get_mut::<Round>(fight).unwrap().phase = RoundPhase::Executing(101);
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
    world.get_mut::<Round>(fight).unwrap().phase = RoundPhase::Executing(101);
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
    world.get_mut::<Round>(fight).unwrap().phase = RoundPhase::Executing(1);
    refresh(world);
    let arms = world.get::<crate::game::actors::Attacks>(actor).unwrap();
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
    let strength = Strength { power: 1, against: 100, boost: 0 };
    world.write_message(Release {
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
        let amount = world.get_mut::<Vitals>(body).unwrap().slay();
        world.write_message(crate::game::hits::Damaged { body, amount, died: true });
        world.run_system_cached(crate::game::characters::settle_deaths).unwrap();
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
    use crate::game::magic::casting::{
        self, Aim, Cast, MagicChoice, MagicNews, Source, UseContext,
    };
    use crate::game::magic::{Outcome, SpellPoints};
    use crate::game::world::hostility::Provocation;
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
        crate::game::inventory::Carried::default(),
    ));
    let fight = world.spawn(Battle::default()).id();
    world.get_mut::<Round>(fight).unwrap().phase = RoundPhase::Executing(1);
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
    world.write_message(Cast { caster: member, spell: SpellId::ENERGY_BLAST, context, choice });
    world.run_system_cached(casting::cast).unwrap();
    assert_eq!(magic::test_points(world, member, Realm::Fire), points - 1);
    let provoked: Vec<_> = world.resource_mut::<Messages<Provocation>>().drain().collect();
    assert_eq!(provoked, vec![Provocation(foe)]);
    let news: Vec<_> = world.resource_mut::<Messages<MagicNews>>().drain().collect();
    assert!(
        matches!(news.as_slice(), [MagicNews::Done(report)] if report.outcome == Outcome::Cast),
        "{news:?}"
    );
    world.run_system_cached(hits::resolve).unwrap();
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
