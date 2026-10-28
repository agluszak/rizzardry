use super::*;
use crate::game::characters::{Gender, Profession, Race};

#[test]
fn difficulty_changes_both_displayed_and_authoritative_lock_chances() {
    let mut character = Character::new("Fighter", Race::Human, Gender::Male, Profession::Fighter);
    character.attributes[Attribute::Strength] = 85;
    let vitals = Vitals::new(&character);
    assert!(
        force_chance(&character, &vitals, 4, Difficulty::Novice)
            > force_chance(&character, &vitals, 4, Difficulty::Expert)
    );
    assert!(
        trap_chance(Some(25), 8, Difficulty::Novice) > trap_chance(Some(25), 8, Difficulty::Expert)
    );
}

use crate::game::world::triggers::tests::attempt;
use crate::game::world::triggers::tests::*;
use crate::game::world::triggers::{TriggerEffect, activate};

use crate::content::items::{Dice, ItemCatalog, ItemDefinition, ItemId};
use crate::game::data::GameData;
use crate::game::inventory::{ItemIds, ItemStack};

#[test]
fn lock_practice_is_bounded_by_the_device_across_reopening_and_revisit() {
    use super::super::interaction::{leave, pick_tumbler};
    let mut level = chest_level(Some(crate::content::level::Lock::Pickable { difficulty: 2 }), 0);
    let picker = level.members[0];
    let mut character = Character::new(
        "Picker",
        crate::game::characters::Race::Human,
        crate::game::characters::Gender::Male,
        crate::game::characters::Profession::Rogue,
    );
    character.skill_points[crate::content::characters::Skill::LocksAndTraps] = 1;
    level.world.entity_mut(picker).insert(character);
    // Arrange a practice opportunity on the first (no held pins) pick.
    let mut opportunity = GameRng::seeded(1);
    while opportunity.clone().below(5) != 0 {
        opportunity.below(5);
    }
    for remaining in (0..6).rev() {
        level.run("CHEST");
        level.world.insert_resource(opportunity.clone());
        let on = attempt(&level);
        pick_tumbler(&mut level.world, on, picker, 0).unwrap();
        assert_eq!(
            level.world.get::<Guarded>(level.device("CHEST")).unwrap().practice_left,
            remaining
        );
        let on = attempt(&level);
        leave(&mut level.world, on);
        if remaining == 3 {
            let memory = level.remember();
            level.visit(Some(memory));
            assert_eq!(level.world.get::<Guarded>(level.device("CHEST")).unwrap().practice_left, 3);
        }
    }
    let before = level.world.get::<Character>(picker).unwrap().skill_practice;
    level.run("CHEST");
    level.world.insert_resource(opportunity);
    let on = attempt(&level);
    pick_tumbler(&mut level.world, on, picker, 0).unwrap();
    assert_eq!(level.world.get::<Character>(picker).unwrap().skill_practice, before);
    assert_eq!(level.world.get::<Guarded>(level.device("CHEST")).unwrap().practice_left, 0);
}

#[test]
fn a_locked_chest_opens_once_picked_and_stays_unlocked() {
    use super::super::interaction::pick_tumbler;
    let lock = crate::content::level::Lock::Pickable { difficulty: 3 };
    let mut level = chest_level(Some(lock), 5);
    let picker = level.members[0];
    level.world.get_mut::<Character>(picker).unwrap().skill_points
        [crate::content::characters::Skill::LocksAndTraps] = 100;
    // The lock answers instead of the chest: no "Nothing happens".
    let chest = level.device("CHEST");
    assert!(!activate(&mut level.world, chest, Some(1.0)));
    assert!(level.effects().is_empty());
    let Some(Work::Picking(picking)) = work(&level) else { panic!("picking") };
    assert_eq!(picking.tumblers.len(), 3);
    assert_eq!(level.motion("CHEST").target, 0.0);
    // At full skill no tumbler falls; the last one opens the chest.
    for tumbler in 0..3 {
        let on = attempt(&level);
        pick_tumbler(&mut level.world, on, picker, tumbler).unwrap();
    }
    assert!(work(&level).is_none());
    assert!(level.world.get::<Guarded>(chest).is_none());
    assert_eq!(level.motion("CHEST").target, 5.0);
    assert_eq!(level.effects(), [TriggerEffect::Gold(5)]);
    // Overcome for good: kept while the party is away.
    level.settle();
    let memory = level.remember();
    level.visit(Some(memory));
    assert!(level.world.get::<Guarded>(level.device("CHEST")).is_none());
    assert!(level.run("CHEST").0);
    assert!(work(&level).is_none());
}

fn with_trap_spell(level: &mut Scenario, spell: crate::content::spells::SpellDefinition) {
    use crate::content::{species::MonsterCatalog, spells::SpellCatalog};
    level.world.init_resource::<Assets<MonsterCatalog>>();
    level.world.init_resource::<Assets<SpellCatalog>>();
    level.world.init_resource::<crate::game::factions::Factions>();
    level.world.init_resource::<Messages<crate::game::magic::casting::MagicNews>>();
    let species =
        level.world.resource_mut::<Assets<MonsterCatalog>>().add(MonsterCatalog::default());
    let mut spells = vec![crate::content::spells::SpellDefinition::default(); 128];
    let id = usize::from(spell.id.0);
    spells[id] = spell;
    let spells = level.world.resource_mut::<Assets<SpellCatalog>>().add(SpellCatalog(spells));
    level.world.resource_mut::<GameData>().species = species;
    level.world.resource_mut::<GameData>().spells = spells;
    for &member in &level.members {
        level.world.entity_mut(member).insert(crate::game::inventory::Carried::default());
    }
}

#[test]
fn a_sprung_trap_pays_its_effect_before_opening_and_retains_the_selected_victim() {
    use super::super::interaction::spring_trap;
    use crate::content::{
        characters::Realm,
        spells::{SpellDefinition, SpellId},
    };
    use crate::game::{
        characters::{Condition, Lasting},
        inventory::Carried,
        magic::casting::MagicNews,
    };
    for destroyed in [false, true] {
        let mut level = chest_level(
            Some(crate::content::level::Lock::Trap { difficulty: 1, kind: Some(10) }),
            0,
        );
        with_trap_spell(
            &mut level,
            SpellDefinition {
                id: SpellId::VAMPIRIC_VORTEX,
                realm: Some(Realm::Divine),
                dice: Some(Dice { count: 0, sides: 0, base: 20 }),
                ..default()
            },
        );
        let first = level.members[0];
        let selected = level.add_member("Selected");
        level.world.entity_mut(first).insert(Carried::default());
        level.world.entity_mut(selected).insert(Carried::default());
        level
            .world
            .get_mut::<Vitals>(selected)
            .unwrap()
            .inflict(Condition::Unconscious, Lasting::Rounds(8));
        let hp = level.world.get::<Vitals>(first).unwrap().hp;
        let selected_hp = level.world.get::<Vitals>(selected).unwrap().hp;
        level.run("CHEST");
        let seed = (0..).find(|&seed| (GameRng::seeded(seed).below(2) == 0) == destroyed).unwrap();
        level.world.insert_resource(GameRng::seeded(seed));
        let on = attempt(&level);
        spring_trap(&mut level.world, on, Some(selected)).unwrap();
        let chest = level.device("CHEST");
        assert_eq!(level.world.get::<Vitals>(first).unwrap().hp, hp);
        assert!(
            level.world.get::<Vitals>(selected).unwrap().hp < selected_hp,
            "unconscious selected member still pays the spell"
        );
        assert_eq!(level.world.get::<Guarded>(chest).is_none(), destroyed);
        assert_eq!(level.motion("CHEST").target > 0.0, destroyed);
        assert!(work(&level).is_none(), "a surviving trap ends the interaction, not a free retry");
        assert!(matches!(
            level
                .world
                .resource_mut::<Messages<MagicNews>>()
                .drain()
                .collect::<Vec<_>>()
                .as_slice(),
            [MagicNews::Triggered { .. }]
        ));
    }
}

#[test]
fn trap_losses_use_existing_purse_items_and_all_party_spell_points() {
    use super::super::interaction::spring_trap;
    use crate::content::{
        characters::Realm,
        items::{EquipClass, QuantityKind},
        spells::{SpellDefinition, SpellId},
    };
    use crate::game::{
        inventory::{Carried, EquipSlot, Equipped},
        magic::{LearnedSpells, SpellPoints},
    };
    for (kind, spell) in [(9, 124), (11, 126), (13, 127)] {
        let mut level = chest_level(
            Some(crate::content::level::Lock::Trap { difficulty: 1, kind: Some(kind) }),
            0,
        );
        with_trap_spell(
            &mut level,
            SpellDefinition {
                id: SpellId(spell),
                realm: Some(Realm::Divine),
                dice: Some(Dice { count: 1, sides: 10, base: 0 }),
                ..default()
            },
        );
        let selected = level.members[0];
        let other = level.add_member("Other");
        level.world.entity_mut(other).insert(Carried::default());
        for member in [selected, other] {
            let mut points = SpellPoints::default();
            points.current[Realm::Divine] = 10;
            points.fraction[Realm::Divine] = 0.5;
            level.world.entity_mut(member).insert((LearnedSpells::default(), points));
        }
        let potion = ItemDefinition {
            id: ItemId(0),
            class: EquipClass::Potion,
            quantity: QuantityKind::Stack,
            ..default()
        };
        let scroll = ItemDefinition {
            id: ItemId(1),
            class: EquipClass::Scroll,
            quantity: QuantityKind::Stack,
            ..default()
        };
        let handle = level.world.resource::<GameData>().items.clone();
        level
            .world
            .resource_mut::<Assets<ItemCatalog>>()
            .insert(&handle, ItemCatalog(vec![potion.clone(), scroll.clone()]))
            .unwrap();
        let mut ids = ItemIds::default();
        let potions = ItemStack::new(&potion, &mut ids, 30, true);
        let scrolls = ItemStack::new(&scroll, &mut ids, 30, true);
        let (potion_id, scroll_id) = (potions.id, scrolls.id);
        level.inventory().pool.stacks.push(potions);
        level.inventory().gold = 1000;
        level.world.get_mut::<Carried>(selected).unwrap().equipment[EquipSlot::AlternatePrimary] =
            Some(Equipped { stack: scrolls, curse_released: true });
        level.run("CHEST");
        level.world.insert_resource(GameRng::seeded(3));
        if spell == 127 {
            let points = level.world.entity_mut(other).take::<SpellPoints>().unwrap();
            let before_rng = level.world.resource::<GameRng>().clone();
            let on = attempt(&level);
            assert_eq!(
                spring_trap(&mut level.world, on, Some(selected)),
                Err(super::super::interaction::Unable::MissingContent)
            );
            assert_eq!(*level.world.resource::<GameRng>(), before_rng);
            assert!(level.world.get::<Guarded>(level.device("CHEST")).is_some());
            assert_eq!(level.inventory().gold, 1000);
            level.world.entity_mut(other).insert(points);
        }
        let on = attempt(&level);
        spring_trap(&mut level.world, on, Some(selected)).unwrap();
        match spell {
            124 => assert!((600..1000).contains(&level.inventory().gold)),
            126 => {
                let carried = level.world.get::<Carried>(selected).unwrap();
                let scrolls = carried.equipment[EquipSlot::AlternatePrimary].as_ref().unwrap();
                assert_eq!(scrolls.stack.id, scroll_id);
                assert!(scrolls.curse_released && scrolls.stack.count() < 30);
                let inventory = level.inventory();
                assert_eq!(inventory.pool.stacks[0].id, potion_id);
                assert!(inventory.pool.stacks[0].count() < 30);
            }
            127 => {
                for member in [selected, other] {
                    let points = level.world.get::<SpellPoints>(member).unwrap();
                    assert_eq!(
                        points.current[Realm::Divine],
                        0,
                        "even the unselected living member loses mana"
                    );
                    assert_eq!(points.fraction[Realm::Divine], 0.5);
                }
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn device_casts_revalidate_targets_and_pay_from_the_selected_item_owner() {
    use super::super::interaction::{Tumbler, cast_device, prepare_device_magic};
    use crate::content::{
        characters::{Realm, Skill},
        items::{EquipClass, QuantityKind, UseType},
        spells::{ItemSpell, SpellDefinition, SpellId, Spellbook, Targeting, Usable},
    };
    use crate::game::{
        inventory::{Carried, EquipSlot, Equipped},
        magic::{
            LearnedSpells, SpellPoints,
            casting::{Refusal, Source},
        },
    };
    let mut level = chest_level(Some(crate::content::level::Lock::Pickable { difficulty: 8 }), 0);
    let spell = SpellDefinition {
        id: SpellId::KNOCK_KNOCK,
        realm: Some(Realm::Earth),
        cost: 1,
        targeting: Targeting::LockOrTrap,
        usable: Usable::OnLockOrTrap,
        books: Spellbook::Wizardry.into(),
        ..default()
    };
    with_trap_spell(&mut level, spell);
    let actor = level.members[0];
    let mut profile = Character::new("Caster", Race::Human, Gender::Male, Profession::Mage);
    for skill in [Skill::Wizardry, Skill::EarthMagic, Skill::Artifacts] {
        profile.skill_points[skill] = 100;
    }
    let mut points = SpellPoints::default();
    points.current[Realm::Earth] = 20;
    level.world.entity_mut(actor).insert((
        Vitals::new(&profile),
        profile,
        LearnedSpells([SpellId::KNOCK_KNOCK].into()),
        points,
    ));
    let pool_tool = ItemDefinition {
        id: ItemId(0),
        class: EquipClass::MiscMagic,
        use_type: UseType::Charged,
        quantity: QuantityKind::Charges,
        professions: enumset::EnumSet::all(),
        races: enumset::EnumSet::all(),
        genders: enumset::EnumSet::all(),
        spell: Some(ItemSpell { spell: SpellId::KNOCK_KNOCK, power: 1 }),
        ..default()
    };
    let worn_tool = ItemDefinition {
        id: ItemId(1),
        must_equip_to_use: true,
        armor_class_bonus: 1,
        ..pool_tool.clone()
    };
    let handle = level.world.resource::<GameData>().items.clone();
    level
        .world
        .resource_mut::<Assets<ItemCatalog>>()
        .insert(&handle, ItemCatalog(vec![pool_tool.clone(), worn_tool.clone()]))
        .unwrap();
    let mut ids = ItemIds::default();
    let pooled = ItemStack::new(&pool_tool, &mut ids, 1, true);
    let worn = ItemStack::new(&worn_tool, &mut ids, 1, true);
    let (pool_id, worn_id) = (pooled.id, worn.id);
    level.inventory().pool.stacks.push(pooled);
    level.world.get_mut::<Carried>(actor).unwrap().backpack[0] = Some(worn);
    level.run("CHEST");
    let other = level.world.spawn_empty().id();
    let rng = level.world.resource::<GameRng>().clone();
    assert_eq!(
        cast_device(
            &mut level.world,
            actor,
            other,
            Source::Spell { spell: SpellId::KNOCK_KNOCK, power: 1 }
        ),
        Err(Refusal::NoTarget)
    );
    assert_eq!(*level.world.resource::<GameRng>(), rng);
    level.world.get_mut::<Vitals>(actor).unwrap().inflict(
        crate::game::characters::Condition::Webbed,
        crate::game::characters::Lasting::Rounds(3),
    );
    let on = attempt(&level);
    assert_eq!(
        cast_device(
            &mut level.world,
            actor,
            on,
            Source::Spell { spell: SpellId::KNOCK_KNOCK, power: 1 }
        ),
        Err(Refusal::CannotAct)
    );
    assert_eq!(*level.world.resource::<GameRng>(), rng);
    assert_eq!(level.world.get::<SpellPoints>(actor).unwrap().current[Realm::Earth], 20);
    level.world.get_mut::<Vitals>(actor).unwrap().lift(crate::game::characters::Condition::Webbed);
    let on = attempt(&level);
    assert_eq!(
        cast_device(&mut level.world, actor, on, Source::Item(worn_id)),
        Err(Refusal::CannotUse)
    );
    assert_eq!(*level.world.resource::<GameRng>(), rng);
    let on = attempt(&level);
    cast_device(
        &mut level.world,
        actor,
        on,
        Source::Spell { spell: SpellId::KNOCK_KNOCK, power: 1 },
    )
    .unwrap();
    assert_eq!(level.world.get::<SpellPoints>(actor).unwrap().current[Realm::Earth], 19);
    let on = attempt(&level);
    cast_device(&mut level.world, actor, on, Source::PoolItem(pool_id)).unwrap();
    assert!(level.inventory().pool.stacks.is_empty(), "a plain exhausted charged tool is consumed");
    assert!(level.world.get::<Carried>(actor).unwrap().find_stack(worn_id).is_some());
    let on = attempt(&level);
    assert_eq!(
        cast_device(&mut level.world, actor, on, Source::PoolItem(pool_id)),
        Err(Refusal::NoItem)
    );
    let mut carried = level.world.get_mut::<Carried>(actor).unwrap();
    let worn = carried.backpack[0].take().unwrap();
    carried.equipment[EquipSlot::AlternatePrimary] = Some(Equipped::new(worn));
    let on = attempt(&level);
    assert_eq!(
        cast_device(&mut level.world, actor, on, Source::Item(worn_id)),
        Err(Refusal::CannotUse)
    );
    let mut carried = level.world.get_mut::<Carried>(actor).unwrap();
    carried.equipment[EquipSlot::Primary] = carried.equipment[EquipSlot::AlternatePrimary].take();
    let on = attempt(&level);
    cast_device(&mut level.world, actor, on, Source::Item(worn_id)).unwrap();
    assert_eq!(
        level.world.get::<Carried>(actor).unwrap().find_stack(worn_id).unwrap().remaining_uses(),
        Some(0),
        "a residual AC effect keeps the exhausted item"
    );
    // Cancelling a newly opened selector drops only this actor's magic holds.
    let on = attempt(&level);
    let mut interaction =
        level.world.get_mut::<super::super::interaction::DeviceInteraction>(on).unwrap();
    let Work::Picking(picking) = &mut interaction.work else { panic!("picking") };
    picking.tumblers[0] = Tumbler::Sealed(actor);
    picking.tumblers[1] = Tumbler::Held(actor);
    prepare_device_magic(&mut level.world, actor, on).unwrap();
    let Some(Work::Picking(picking)) = work(&level) else { panic!("picking") };
    assert_eq!(picking.tumblers[0], Tumbler::Down);
    assert_eq!(picking.tumblers[1], Tumbler::Held(actor));
}

#[test]
fn divine_trap_replaces_its_session_bonus_and_uses_shared_spell_payment() {
    use super::super::interaction::{cast_device, leave};
    use crate::content::{
        characters::{Realm, Skill},
        spells::{SpellCatalog, SpellDefinition, SpellId, Spellbook, Targeting, Usable},
    };
    use crate::game::magic::{LearnedSpells, SpellPoints, casting::Source};
    let mut level =
        chest_level(Some(crate::content::level::Lock::Trap { difficulty: 7, kind: Some(10) }), 0);
    with_trap_spell(
        &mut level,
        SpellDefinition { id: SpellId::VAMPIRIC_VORTEX, realm: Some(Realm::Divine), ..default() },
    );
    let handle = level.world.resource::<GameData>().spells.clone();
    level.world.resource_mut::<Assets<SpellCatalog>>().get_mut(&handle).unwrap().0[18] =
        SpellDefinition {
            id: SpellId::DIVINE_TRAP,
            realm: Some(Realm::Mental),
            cost: 1,
            books: Spellbook::Wizardry.into(),
            targeting: Targeting::LockOrTrap,
            usable: Usable::OnLockOrTrap,
            ..default()
        };
    let actor = level.members[0];
    let mut profile = Character::new("Caster", Race::Human, Gender::Male, Profession::Mage);
    profile.skill_points[Skill::Wizardry] = 100;
    profile.skill_points[Skill::MentalMagic] = 100;
    let mut points = SpellPoints::default();
    points.current[Realm::Mental] = 10;
    level.world.entity_mut(actor).insert((
        Vitals::new(&profile),
        profile,
        LearnedSpells([SpellId::DIVINE_TRAP].into()),
        points,
    ));
    level.run("CHEST");
    let device = level.device("CHEST");
    let on = attempt(&level);
    cast_device(
        &mut level.world,
        actor,
        on,
        Source::Spell { spell: SpellId::DIVINE_TRAP, power: 2 },
    )
    .unwrap();
    let Some(Work::Disarming(trap)) = work(&level) else { panic!("disarming") };
    assert_eq!(trap.divine_power, 2);
    let stronger = trap.inspection_chance(Some(25), crate::game::difficulty::Difficulty::Normal);
    let on = attempt(&level);
    cast_device(
        &mut level.world,
        actor,
        on,
        Source::Spell { spell: SpellId::DIVINE_TRAP, power: 1 },
    )
    .unwrap();
    let Some(Work::Disarming(trap)) = work(&level) else { panic!("disarming") };
    assert_eq!(trap.divine_power, 1);
    assert!(
        trap.inspection_chance(Some(25), crate::game::difficulty::Difficulty::Normal) < stronger
    );
    assert_eq!(level.world.get::<SpellPoints>(actor).unwrap().current[Realm::Mental], 7);
    let on = attempt(&level);
    leave(&mut level.world, on);
    level.run("CHEST");
    let Some(Work::Disarming(trap)) = work(&level) else { panic!("disarming") };
    assert_eq!(trap.divine_power, 0);
    assert!(level.world.get::<Guarded>(device).is_some());
}

#[test]
fn an_unsupported_trap_keeps_its_type_and_refuses_every_discharge_path() {
    use super::super::interaction::{
        LockNews, TRAP_COMPONENTS, Unable, disarm_component, inspect_trap, leave, spring_trap,
    };
    let lock = crate::content::level::Lock::Trap { difficulty: 5, kind: Some(0) };
    let mut level = chest_level(Some(lock), 0);
    let member = level.members[0];
    level.world.get_mut::<Character>(member).unwrap().skill_points
        [crate::content::characters::Skill::LocksAndTraps] = 50;
    assert!(!level.run("CHEST").0);
    let Some(Work::Disarming(disarming)) = work(&level) else { panic!("disarming") };
    let trap = disarming.trap;
    let chest = level.device("CHEST");
    assert_eq!(level.world.get::<Guarded>(chest).unwrap().trap, Some(trap));
    // Leaving and returning faces the same trap.
    let on = attempt(&level);
    leave(&mut level.world, on);
    let memory = level.remember();
    level.visit(Some(memory));
    let chest = level.device("CHEST");
    assert!(!level.run("CHEST").0);
    assert_eq!(level.world.get::<Guarded>(chest).unwrap().trap, Some(trap));
    let wrong = TRAP_COMPONENTS[usize::from(trap)].iter().position(|&p| !p).unwrap();
    let before_rng = level.world.resource::<GameRng>().clone();
    let on = attempt(&level);
    assert_eq!(disarm_component(&mut level.world, on, member, wrong), Err(Unable::UnsupportedTrap));
    let on = attempt(&level);
    assert_eq!(inspect_trap(&mut level.world, on, member), Err(Unable::UnsupportedTrap));
    let on = attempt(&level);
    assert_eq!(spring_trap(&mut level.world, on, Some(member)), Err(Unable::UnsupportedTrap));
    assert_eq!(*level.world.resource::<GameRng>(), before_rng);
    assert!(matches!(work(&level), Some(Work::Disarming(_))));
    assert_eq!(level.world.get::<Guarded>(chest).unwrap().trap, Some(trap));
    assert_eq!(level.motion("CHEST").target, 0.0);
    let news: Vec<_> = level.world.resource_mut::<Messages<LockNews>>().drain().collect();
    assert_eq!(news.iter().filter(|n| matches!(n, LockNews::UnsupportedTrap)).count(), 3);
}
