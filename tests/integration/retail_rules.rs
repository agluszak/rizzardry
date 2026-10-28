//! Rules checked against the retail databases. Ignored by default: run
//! them with `RIZZARDRY_RETAIL` set.

use rizzardry::content::items::{AttackMode, ItemCatalog, ItemId};
use rizzardry::game::characters::{Character, Gender, Load, Profession, Race, Vitals};
use rizzardry::game::combat::attack::*;
use rizzardry::game::difficulty::Difficulty;
use rizzardry::game::inventory::{Carried, EquipSlot, ItemIds, make_item, place_at_creation};
use rizzardry::game::party::roster::starting_items;
use rizzardry::game::rng::GameRng;
use rizzardry::game::world::population::{Monster, ScaledBody};
use rizzardry::retail::files::RetailFiles;

fn retail_files() -> RetailFiles {
    let root = std::env::var("RIZZARDRY_RETAIL")
        .expect("retail tests need RIZZARDRY_RETAIL set to a Wizardry 8 installation");
    RetailFiles::open(std::path::Path::new(&root)).unwrap()
}

fn retail_catalog(files: &RetailFiles) -> ItemCatalog {
    let strings = files.read("data/strings/stringdata.dat").unwrap().unwrap();
    let strings = rizzardry::retail::strings::read_string_table(&strings).unwrap();
    let items = files.read("data/databases/items.dbs").unwrap().unwrap();
    rizzardry::retail::items::read_item_catalog(&items, &strings).unwrap()
}

#[test]
#[ignore = "requires a retail installation"]
fn a_fighter_and_a_crab_trade_blows() {
    let files = retail_files();
    let catalog = retail_catalog(&files);
    let species = rizzardry::retail::species::read_species(
        &files.read("data/databases/monsters.dbs").unwrap().unwrap(),
    )
    .unwrap();
    let crab = species.iter().find(|s| s.name.contains("Crab")).unwrap();
    let fighter = Character::new("Ragnar", Race::Human, Gender::Male, Profession::Fighter);
    let (mut ids, mut rng) = (ItemIds::default(), GameRng::seeded(9));
    let mut carried = Carried::default();
    for id in [143, 153, 203, 246, 7] {
        let item = make_item(catalog.get(ItemId(id)).unwrap(), &mut ids, &mut rng, true, true);
        assert!(place_at_creation(&mut carried, item, &catalog, &fighter).is_none());
    }
    let hand = primary_hand(&fighter.base_stats(), &carried, &catalog, Load::None);
    let armour = armour(&fighter.base_stats(), &carried, &catalog, Load::None, false);
    let body = Monster {
        location: 0,
        species: 0,
        hp: 10,
        max_hp: 10,
        stamina: 10,
        max_stamina: 10,
        conditions: Vec::new(),
        enchantments: Default::default(),
        body: ScaledBody { scale: 1.0, radius: 1.0, eye_height: 1.0, speed: 0.0 },
    };
    // Each side's aim lands somewhere a roll of 1-100 sometimes beats and
    // sometimes does not.
    let ours = member_aim(
        &fighter.base_stats(),
        &Vitals::new(&fighter),
        &hand,
        AttackMode::Swing,
        Surprise::default(),
        Some(&Defence::monster(crab, &body, false)),
        Difficulty::Normal,
    );
    assert!((6..=95).contains(&ours), "the fighter sometimes hits and sometimes misses: {ours}");
    let attack = &crab.attacks[0];
    let mode = choose_mode(&mut rng, attack.modes);
    let theirs = monster_aim(
        (crab, &body),
        attack,
        mode,
        Surprise::default(),
        Some(&Defence::member(&fighter.base_stats(), armour, false, false)),
        Difficulty::Normal,
    );
    assert!(theirs > 5, "the crab sometimes hits: {theirs}");
}

#[test]
#[ignore = "requires a retail installation"]
fn starting_equipment_is_worn() {
    let files = retail_files();
    let catalog = retail_catalog(&files);
    let (mut ids, mut rng) = (ItemIds::default(), GameRng::seeded(1));
    let professions = [
        (Race::Human, Profession::Fighter),
        (Race::Dracon, Profession::Valkyrie),
        (Race::Dwarf, Profession::Priest),
        (Race::Hobbit, Profession::Rogue),
        (Race::Elf, Profession::Mage),
        (Race::Felpurr, Profession::Bard),
        (Race::Elf, Profession::Ranger),
        (Race::Faerie, Profession::Ninja),
    ];
    for (race, profession) in professions {
        let gender = if profession == Profession::Valkyrie { Gender::Female } else { Gender::Male };
        let character = Character::new("Test", race, gender, profession);
        let mut carried = Carried::default();
        for id in starting_items(&character) {
            let item = make_item(catalog.get(id).unwrap(), &mut ids, &mut rng, true, true);
            assert!(place_at_creation(&mut carried, item, &catalog, &character).is_none());
        }
        assert!(carried.worn(EquipSlot::Primary).is_some(), "{profession:?} holds a weapon");
    }
}
