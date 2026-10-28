//! Rules checked against the retail databases. Ignored by default: run
//! them with `RIZZARDRY_RETAIL` set.

use rizzardry::content::items::{AttackMode, ItemCatalog, ItemId};
use rizzardry::game::characters::{Character, Gender, Load, Profession, Race, Vitals};
use rizzardry::game::combat::attack::*;
use rizzardry::game::inventory::{Carried, EquipSlot, ItemIds, make_item, place_at_creation};
use rizzardry::game::party::roster::starting_items;
use rizzardry::game::rng::GameRng;
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
    let hand = primary_hand(&fighter, &carried, &catalog, Load::None);
    let armour = armour(&fighter, &carried, &catalog, Load::None, false);
    let tally = |swings: Vec<Swing>| {
        let hits: Vec<u32> = swings
            .iter()
            .filter_map(|s| match s {
                Swing::Hit(damage) => Some(*damage),
                _ => None,
            })
            .collect();
        (hits.len(), hits.iter().sum::<u32>() as f32 / hits.len().max(1) as f32)
    };
    let ours = tally(
        (0..1000)
            .map(|_| {
                character_swing(
                    &mut rng,
                    &fighter,
                    &Vitals::new(&fighter),
                    &hand,
                    AttackMode::Swing,
                    crab,
                )
            })
            .collect(),
    );
    let attack = &crab.attacks[0];
    let theirs = tally(
        (0..1000)
            .map(|_| {
                let mode = choose_mode(&mut rng, attack.modes);
                monster_swing(&mut rng, crab, attack, mode, &fighter, &armour)
            })
            .collect(),
    );
    assert!(ours.0 > 100 && ours.0 < 1000, "the fighter sometimes hits and sometimes misses");
    assert!(theirs.0 > 50 && theirs.0 < 1000);
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
