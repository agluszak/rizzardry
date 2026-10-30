//! Utility targets are validated before the shared cast/payment operation.
use super::casting::{Aim, MagicChoice, MagicNews, Refusal, Source, UseContext};
use crate::content::spells::SpellId;
use crate::game::data::Catalogs;
use crate::game::inventory::{Carried, ItemStack, PartyInventory, StackId};
use crate::game::party::roster::{PartyRoster, the_party};
use bevy::prelude::*;

pub(crate) fn spell(id: SpellId) -> bool {
    super::portals::spell(id)
        || super::dialogue::spell(id)
        || super::pointcasts::spell(id)
        || id == SpellId::IDENTIFY_ITEM
}

pub(crate) fn source_spell(
    world: &mut World,
    caster: Entity,
    source: Source,
) -> Result<SpellId, Refusal> {
    let party = the_party(world);
    let inventory = world.get::<PartyInventory>(party).unwrap();
    let items = &world.resource::<Catalogs>().items;
    source.spell_id(world.get::<Carried>(caster).unwrap(), inventory, items).ok_or(Refusal::NoItem)
}

/// Search only the current roster and party inventory. Stable stack IDs allow
/// a target to move between those owners without retargeting a different item.
fn target(world: &World, party: Entity, id: StackId) -> Option<&ItemStack> {
    world.get::<PartyInventory>(party).unwrap().find_stack(id).or_else(|| {
        world
            .get::<PartyRoster>(party)
            .unwrap()
            .iter()
            .find_map(|member| world.get::<Carried>(member).unwrap().find_stack(id))
    })
}

/// Whether an identify at `strength` reveals an item this hard (retail
/// `ApplyIdentifyAttempt`: power × 3, raised by Power Cast).
fn identifies(strength: super::Strength, difficulty: u8) -> bool {
    let score = u32::from(strength.power) * 3;
    score + score * strength.boost / 100 >= u32::from(difficulty)
}

pub(crate) fn cast(world: &mut World, caster: Entity, choice: MagicChoice) -> Result<(), Refusal> {
    let id = source_spell(world, caster, choice.source)?;
    if super::pointcasts::spell(id) {
        return super::pointcasts::cast(world, caster, choice);
    }
    if super::dialogue::spell(id) {
        return super::dialogue::cast(world, caster, choice);
    }
    if super::portals::spell(id) {
        return super::portals::cast(world, caster, choice.source);
    }
    if id != SpellId::IDENTIFY_ITEM {
        return Err(Refusal::Unsupported);
    }
    if !world.run_system_cached(crate::game::freedom::unoccupied).unwrap_or(false)
        || world.query::<&crate::game::combat::Battle>().iter(world).next().is_some()
    {
        return Err(Refusal::WrongTime);
    }
    let Aim::Item(target_id) = choice.aim else { return Err(Refusal::NoTarget) };
    let party = the_party(world);
    let stack = target(world, party, target_id).ok_or(Refusal::NoTarget)?;
    let definition = &world.resource::<Catalogs>().items[stack.item];
    let difficulty = definition.identify_difficulty;
    // The source cannot consume its own target and then silently identify a
    // replacement. A fully known stack likewise needs no paid attempt.
    if matches!(choice.source, Source::Item(s) | Source::PoolItem(s) if s == target_id)
        || stack.identified && stack.spell_known && stack.curse_known
    {
        return Err(Refusal::NoTarget);
    }
    let source = if let Source::Spell { spell, power: super::casting::AUTO_POWER } = choice.source {
        let spells = &world.resource::<Catalogs>().spells;
        let definition = &spells[spell];
        let profile = world.get::<crate::game::characters::Character>(caster).unwrap();
        let power = (1..=crate::content::spells::MAX_POWER)
            .find(|&power| identifies(super::Strength::of(profile, definition, power), difficulty))
            .unwrap_or(crate::content::spells::MAX_POWER);
        Source::Spell { spell, power }
    } else {
        choice.source
    };
    let (report, strength) = super::sources::cast_source(world, caster, source, UseContext::World)?;
    // No item roll or extra RNG draw; success reveals identity, spell and
    // binding.
    if report.outcome == super::Outcome::Cast && identifies(strength, difficulty) {
        let reveal = |stack: &mut ItemStack| {
            stack.identified = true;
            stack.spell_known = true;
            stack.curse_known = true;
        };
        let mut inventory = world.get_mut::<PartyInventory>(party).unwrap();
        if let Some(stack) = inventory.find_stack_mut(target_id) {
            reveal(stack);
        } else {
            let owner = world
                .get::<PartyRoster>(party)
                .unwrap()
                .iter()
                .find(|&member| {
                    world.get::<Carried>(member).unwrap().find_stack(target_id).is_some()
                })
                .unwrap();
            reveal(world.get_mut::<Carried>(owner).unwrap().find_stack_mut(target_id).unwrap());
        }
    }
    world.write_message(MagicNews::Done(report));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{
        characters::{Realm, Skill},
        items::{ItemCatalog, ItemDefinition, ItemId},
        species::MonsterCatalog,
        spells::{SpellCatalog, SpellDefinition, Spellbook, Usable},
    };
    use crate::game::{
        characters::{Character, Gender, Profession, Race, Vitals},
        inventory::ItemIds,
        magic::{LearnedSpells, MagicRequest, SpellPoints, portals::Portal},
        party::body::PartyBody,
        rng::GameRng,
        world::{scripts::LevelScript, triggers::tests::Scenario},
    };
    use avian3d::prelude::Position;

    fn scenario() -> Scenario {
        let mut scene = Scenario::new(
            vec![crate::game::world::triggers::tests::trigger(
                "ARRIVAL",
                crate::content::level::TriggerAction::Nothing,
            )],
            &[],
            LevelScript::None,
        );
        let world = &mut scene.world;
        world.init_resource::<crate::game::factions::Factions>();
        world.init_resource::<Messages<crate::game::factions::FactionNews>>();
        world.init_resource::<ItemIds>();
        world.init_resource::<Messages<MagicRequest>>();
        world.init_resource::<Messages<MagicNews>>();
        let species = MonsterCatalog::default();
        let mut definitions = vec![SpellDefinition::default(); 76];
        let ids = [SpellId::SET_PORTAL, SpellId::RETURN_TO_PORTAL, SpellId::IDENTIFY_ITEM];
        for id in ids {
            definitions[id.0 as usize] = SpellDefinition {
                id,
                realm: Some(Realm::Fire),
                cost: 1,
                books: Spellbook::Wizardry.into(),
                usable: Usable::OutOfCombat,
                ..default()
            };
        }
        let spells = SpellCatalog(definitions);
        let mut data = world.resource_mut::<crate::game::data::Catalogs>();
        data.spells = spells;
        data.species = species;
        let mut profile = Character::new("Caster", Race::Human, Gender::Male, Profession::Mage);

        profile.profession_levels[Profession::Mage] = 30;
        profile.skill_points[Skill::Wizardry] = 100;
        profile.skill_points[Skill::FireMagic] = 100;
        let vitals = Vitals::new(&profile);
        world.entity_mut(scene.members[0]).insert((
            profile,
            vitals,
            Carried::default(),
            LearnedSpells(ids.into_iter().collect()),
            SpellPoints::default(),
        ));
        crate::game::magic::test_set_points(world, scene.members[0], Realm::Fire, 100);
        scene
    }

    /// Casting outside a fight needs no monster, loot or faction services;
    /// scenarios that cast without them keep it so.
    fn request(scene: &mut Scenario, spell: SpellId, aim: Aim) -> MagicNews {
        let caster = scene.members[0];
        scene.world.write_message(MagicRequest {
            caster,
            choice: MagicChoice { source: Source::Spell { spell, power: 1 }, aim },
        });
        scene.world.run_system_cached(super::super::accept_magic).unwrap();
        scene.world.resource_mut::<Messages<MagicNews>>().drain().last().unwrap()
    }

    #[test]
    fn utility_casting_while_camped_keeps_the_world_input_gate_closed() {
        let mut scene = scenario();
        let caster = scene.members[0];
        scene.world.insert_resource(crate::game::camp::Camp::UntilAble);
        assert!(!scene.world.run_system_cached(crate::game::freedom::party_free).unwrap());
        assert!(
            matches!(request(&mut scene, SpellId::SET_PORTAL, Aim::Member(caster)), MagicNews::Done(report) if report.outcome == super::super::Outcome::Cast)
        );
        assert!(scene.world.get::<Portal>(caster).is_some());
        assert!(!scene.world.run_system_cached(crate::game::freedom::party_free).unwrap());
    }

    #[test]
    fn underwater_fire_fizzles_with_normal_payment_for_ordinary_and_utility_casts() {
        use crate::content::{items::Dice, spells::Targeting};
        let mut scene = scenario();
        let caster = scene.members[0];
        scene.world.resource_mut::<crate::game::data::Catalogs>().spells.0
            [usize::from(SpellId::HEAL_WOUNDS.0)] = SpellDefinition {
            id: SpellId::HEAL_WOUNDS,
            realm: Some(Realm::Fire),
            cost: 1,
            level: 1,
            books: Spellbook::Wizardry.into(),
            targeting: Targeting::Ally,
            dice: Some(Dice { base: 5, count: 0, sides: 0 }),
            ..default()
        };
        scene.world.get_mut::<LearnedSpells>(caster).unwrap().0.insert(SpellId::HEAL_WOUNDS);
        scene.world.get_mut::<Vitals>(caster).unwrap().hp = 1;
        assert!(
            matches!(request(&mut scene, SpellId::HEAL_WOUNDS, Aim::Member(caster)), MagicNews::Done(report) if report.outcome == super::super::Outcome::Cast)
        );
        assert_eq!(scene.world.get::<Vitals>(caster).unwrap().hp, 6);
        let mana = crate::game::magic::test_points(&scene.world, caster, Realm::Fire);
        let stamina = scene.world.get::<Vitals>(caster).unwrap().stamina;
        scene
            .world
            .get_mut::<crate::game::world::triggers::LevelState>(scene.root)
            .unwrap()
            .camera_sway = true;
        assert!(
            matches!(request(&mut scene, SpellId::HEAL_WOUNDS, Aim::Member(caster)), MagicNews::Done(report) if report.outcome == super::super::Outcome::Fizzled)
        );
        assert_eq!(scene.world.get::<Vitals>(caster).unwrap().hp, 6);
        assert_eq!(crate::game::magic::test_points(&scene.world, caster, Realm::Fire), mana - 1);
        assert_eq!(scene.world.get::<Vitals>(caster).unwrap().stamina, stamina - 2);
        assert!(
            matches!(request(&mut scene, SpellId::SET_PORTAL, Aim::Member(caster)), MagicNews::Done(report) if report.outcome == super::super::Outcome::Fizzled)
        );
        assert!(scene.world.get::<Portal>(caster).is_none());
        assert_eq!(crate::game::magic::test_points(&scene.world, caster, Realm::Fire), mana - 2);
    }

    #[test]
    fn quiet_fire_items_keep_the_half_failure_adjustment_and_pay_their_use_underwater() {
        use super::super::Outcome;
        use crate::content::{
            items::{EquipClass, QuantityKind, UseType},
            spells::ItemSpell,
        };
        for fizzles in [false, true] {
            let seed =
                (0..100).find(|&seed| (GameRng::seeded(seed).below(100) < 50) == fizzles).unwrap();
            let mut scene = scenario();
            // A utility source pays without the party's formation.
            let party = the_party(&mut scene.world);
            scene.world.entity_mut(party).remove::<crate::game::party::formation::Formation>();
            let caster = scene.members[0];
            scene
                .world
                .get_mut::<crate::game::world::triggers::LevelState>(scene.root)
                .unwrap()
                .camera_sway = true;
            let item = ItemDefinition {
                id: ItemId(0),
                class: EquipClass::Bomb,
                quantity: QuantityKind::Stack,
                use_type: UseType::Thrown,
                professions: enumset::EnumSet::all(),
                races: enumset::EnumSet::all(),
                genders: enumset::EnumSet::all(),
                spell: Some(ItemSpell { spell: SpellId::SET_PORTAL, power: 1 }),
                ..default()
            };
            let stack = ItemStack::new(&item, &mut ItemIds::default(), 2, true);
            let id = stack.id;
            let handle = ItemCatalog(vec![item]);
            scene.world.resource_mut::<crate::game::data::Catalogs>().items = handle;
            scene.inventory().pool.stacks.push(stack);
            scene.world.insert_resource(GameRng::seeded(seed));
            let (report, _) = super::super::sources::cast_source(
                &mut scene.world,
                caster,
                Source::PoolItem(id),
                UseContext::World,
            )
            .unwrap();
            assert_eq!(report.outcome, if fizzles { Outcome::Fizzled } else { Outcome::Cast });
            assert_eq!(scene.inventory().pool.stacks[0].count(), 1);
            assert_eq!(crate::game::magic::test_points(&scene.world, caster, Realm::Fire), 100);
        }
    }

    #[test]
    fn portal_requests_keep_caster_ownership_and_refuse_missing_destinations_without_payment() {
        let mut scene = scenario();
        let caster = scene.members[0];
        let party = the_party(&mut scene.world);
        let before = scene.world.get::<SpellPoints>(caster).unwrap().clone();
        let rng = scene.world.resource::<GameRng>().clone();
        assert!(matches!(
            request(&mut scene, SpellId::RETURN_TO_PORTAL, Aim::Member(caster)),
            MagicNews::Refused { refusal: Refusal::NoTarget, .. }
        ));
        assert_eq!(scene.world.get::<SpellPoints>(caster).unwrap(), &before);
        assert_eq!(scene.world.resource::<GameRng>().clone().below(1000), rng.clone().below(1000));
        scene.world.get_mut::<PartyBody>(party).unwrap().pitch = 0.25;
        assert!(matches!(request(&mut scene, SpellId::SET_PORTAL, Aim::Member(caster)),
            MagicNews::Done(report) if report.outcome == super::super::Outcome::Cast));
        let saved = scene.world.get::<Portal>(caster).unwrap().clone();
        let other = scene.add_member("Other");
        assert!(scene.world.get::<Portal>(other).is_none());
        scene.world.get_mut::<Position>(party).unwrap().0 = Vec3::splat(20.0);
        scene.world.get_mut::<PartyBody>(party).unwrap().pitch = -0.5;
        assert!(matches!(request(&mut scene, SpellId::RETURN_TO_PORTAL, Aim::Member(other)),
            MagicNews::Done(report) if report.outcome == super::super::Outcome::Cast));
        assert_eq!(PartyBody::eye(scene.world.get::<Position>(party).unwrap()), saved.eye);
        assert_eq!(scene.world.get::<PartyBody>(party).unwrap().pitch, 0.25);
        assert_eq!(crate::game::magic::test_points(&scene.world, caster, Realm::Fire), 98);
        let memory = scene.remember();
        scene.visit(Some(memory));
        assert_eq!(scene.world.get::<Portal>(caster).unwrap().eye, saved.eye);
    }

    #[test]
    fn identify_targets_the_selected_stack_and_reveals_all_item_knowledge() {
        let mut scene = scenario();
        let caster = scene.members[0];
        let definition = ItemDefinition { id: ItemId(0), identify_difficulty: 3, ..default() };
        let target = ItemStack::new(&definition, &mut ItemIds::default(), 1, false);
        let target_id = target.id;
        let items = ItemCatalog(vec![definition]);
        scene.world.resource_mut::<crate::game::data::Catalogs>().items = items;
        scene.inventory().hand.held = Some(target);
        scene.world.write_message(MagicRequest {
            caster,
            choice: MagicChoice {
                source: Source::Spell {
                    spell: SpellId::IDENTIFY_ITEM,
                    power: super::super::casting::AUTO_POWER,
                },
                aim: Aim::Item(target_id),
            },
        });
        // The same target may move before the deferred request is consumed.
        let target = scene.inventory().hand.held.take().unwrap();
        scene.world.get_mut::<Carried>(caster).unwrap().backpack[0] = Some(target);
        scene.world.run_system_cached(super::super::accept_magic).unwrap();
        let news = scene.world.resource_mut::<Messages<MagicNews>>().drain().last().unwrap();
        assert!(matches!(news, MagicNews::Done(report)
            if report.outcome == super::super::Outcome::Cast && report.power == 1));
        let stack = scene.world.get::<Carried>(caster).unwrap().find_stack(target_id).unwrap();
        assert!(stack.identified && stack.spell_known && stack.curse_known);
        let before = scene.world.get::<SpellPoints>(caster).unwrap().clone();
        assert!(matches!(
            request(&mut scene, SpellId::IDENTIFY_ITEM, Aim::Item(target_id)),
            MagicNews::Refused { refusal: Refusal::NoTarget, .. }
        ));
        assert_eq!(scene.world.get::<SpellPoints>(caster).unwrap(), &before);
    }
}
