//! Rules checked against the retail databases. Ignored by default: run
//! them with `RIZZARDRY_RETAIL` set.

use crate::content::items::ItemCatalog;
use crate::game::characters::{Character, Gender, Profession, Race};
use crate::game::inventory::EquipSlot;
use crate::game::items::{Items, Outfit, make, roll_amount};
use crate::game::party::roster::starting_items;
use crate::game::rng::GameRng;
use crate::retail::files::RetailFiles;
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;

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
    let mut rng = GameRng::seeded(1);
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
        let mut app = App::new();
        app.add_plugins(crate::game::items::plugin);
        let world = app.world_mut();
        let vitals = crate::game::characters::Vitals::new(&character);
        let member = world.spawn((character.clone(), vitals)).id();
        for id in starting_items(&character) {
            let definition = catalog.get(id).unwrap().clone();
            let amount = roll_amount(&definition, &mut rng, true);
            let item = world
                .run_system_once(move |mut commands: Commands| {
                    make(&mut commands, &definition, amount, true)
                })
                .unwrap();
            world.flush();
            world.trigger(Outfit { member, item });
            world.flush();
        }
        let wields = world
            .run_system_once(move |items: Items| items.worn(member, EquipSlot::Primary).is_some())
            .unwrap();
        assert!(wields, "{profession:?} holds a weapon");
    }
}
