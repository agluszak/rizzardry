//! Rules checked against the retail databases. Ignored by default: run
//! them with `RIZZARDRY_RETAIL` set.

use crate::content::items::ItemCatalog;
use crate::game::characters::{Character, Gender, Profession, Race};
use crate::game::inventory::{Carried, EquipSlot, ItemIds, make_item, place_at_creation};
use crate::game::party::roster::starting_items;
use crate::game::rng::GameRng;
use crate::retail::files::RetailFiles;

fn retail_files() -> RetailFiles {
    let root = std::env::var("RIZZARDRY_RETAIL")
        .expect("retail tests need RIZZARDRY_RETAIL set to a Wizardry 8 installation");
    RetailFiles::open(std::path::Path::new(&root)).unwrap()
}

fn retail_catalog(files: &RetailFiles) -> ItemCatalog {
    let strings = files.read("data/strings/stringdata.dat").unwrap().unwrap();
    let strings = crate::retail::strings::read_string_table(&strings).unwrap();
    let items = files.read("data/databases/items.dbs").unwrap().unwrap();
    crate::retail::items::read_item_catalog(&items, &strings).unwrap()
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
