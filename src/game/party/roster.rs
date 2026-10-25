//! Who is in the party, in order.

use bevy::prelude::*;

use crate::content::items::{ItemCatalog, ItemId};
use crate::game::characters::{
    AXE, Character, Gender, MACE_AND_FLAIL, Profession, Race, STAFF_AND_WAND, SWORD,
};
use crate::game::data::GameData;
use crate::game::inventory::{Carried, ItemIds, make_item, place_at_creation};
use crate::game::rng::GameRng;

/// The party's members in slot order. Each is an entity with a
/// [`Character`] and what it [`Carried`].
#[derive(Resource, Debug, Default)]
pub struct PartyRoster(pub Vec<Entity>);

/// A party to play with until party creation exists.
pub fn spawn_test_party(mut commands: Commands, mut roster: ResMut<PartyRoster>) {
    let members = [
        ("Ragnar", Race::Human, Gender::Male, Profession::Fighter, "hummf"),
        ("Saphra", Race::Dracon, Gender::Female, Profession::Valkyrie, "dracff"),
        ("Borin", Race::Dwarf, Gender::Male, Profession::Priest, "dwarfmn"),
        ("Pippa", Race::Hobbit, Gender::Female, Profession::Rogue, "hobfn"),
        ("Elwen", Race::Elf, Gender::Female, Profession::Mage, "elffm"),
        ("Purrl", Race::Felpurr, Gender::Male, Profession::Bard, "felpmf"),
    ];
    for (name, race, gender, profession, portrait) in members {
        let mut character = Character::new(name, race, gender, profession);
        character.portrait = portrait.to_owned();
        roster.0.push(commands.spawn((Name::new(name.to_owned()), character, NeedsEquipment)).id());
    }
}

/// A new member still to receive their starting equipment.
#[derive(Component, Debug)]
pub struct NeedsEquipment;

/// Each profession's six starting items (Faeries share their own set),
/// retail `g_starting_equipment`.
const STARTING_SETS: [[Option<u16>; 6]; 16] = {
    const N: Option<u16> = None;
    const fn s(i: u16) -> Option<u16> {
        Some(i)
    }
    [
        [s(143), s(153), s(203), s(246), N, N],
        [s(35), s(1), s(158), s(204), s(247), s(210)],
        [s(75), s(170), s(197), s(246), s(215), N],
        [s(110), s(125), s(157), s(188), s(247), N],
        [s(38), s(49), s(165), s(192), s(246), N],
        [s(105), s(230), s(180), s(209), s(253), N],
        [s(79), s(105), s(165), s(192), s(246), N],
        [s(0), s(0), s(99), s(156), s(187), s(247)],
        [s(132), s(156), s(187), s(246), N, N],
        [s(769), s(116), s(132), s(156), s(187), N],
        [s(165), s(192), s(246), s(335), N, N],
        [s(28), s(165), s(192), s(246), s(350), N],
        [s(82), s(165), s(192), s(246), s(223), N],
        [s(0), s(165), s(192), s(246), s(347), N],
        [s(28), s(165), s(192), s(246), s(385), N],
        [s(63), s(169), s(196), s(335), s(358), s(0)],
    ]
};

/// A new character's starting items (retail
/// `AddCharacterStartingEquipment`): their set, then one extra item for
/// some professions, chosen by their weapon skills.
fn starting_items(character: &Character) -> Vec<u16> {
    let set = if character.race == Race::Faerie { 15 } else { character.profession as usize };
    let mut items: Vec<u16> = STARTING_SETS[set].iter().flatten().copied().collect();
    let skill = |s| character.skill(s);
    items.extend(match character.profession {
        Profession::Priest if skill(MACE_AND_FLAIL) > skill(STAFF_AND_WAND) => Some(0x16),
        Profession::Priest => Some(0x52),
        Profession::Fighter if skill(AXE) > skill(SWORD) => Some(0x12),
        Profession::Fighter => Some(7),
        Profession::Gadgeteer => Some(599),
        Profession::Bard => Some(0x144),
        _ => None,
    });
    items
}

/// Hand new members their starting equipment once the items have loaded:
/// at full quantity, identified, worn where it fits.
pub fn give_starting_equipment(
    mut commands: Commands,
    data: Option<Res<GameData>>,
    catalogs: Res<Assets<ItemCatalog>>,
    mut ids: ResMut<ItemIds>,
    mut rng: ResMut<GameRng>,
    members: Query<(Entity, &Character), With<NeedsEquipment>>,
) {
    let Some(catalog) = data.and_then(|d| catalogs.get(&d.items)) else { return };
    for (entity, character) in &members {
        let mut carried = Carried::default();
        for id in starting_items(character) {
            let Some(definition) = catalog.get(ItemId(id)) else { continue };
            let item = make_item(definition, &mut ids, &mut rng, true, true);
            if place_at_creation(&mut carried, item, catalog, character).is_err() {
                warn!("{}: no room for {}", character.name, definition.name);
            }
        }
        commands.entity(entity).remove::<NeedsEquipment>().insert(carried);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::characters::Gender;
    use crate::game::inventory::EquipSlot;

    #[test]
    #[ignore = "requires a retail installation"]
    fn starting_equipment_is_worn() {
        let root =
            std::env::var("RIZZARDRY_RETAIL").expect("set RIZZARDRY_RETAIL to run this test");
        let files = crate::retail::files::RetailFiles::open(root).unwrap();
        let catalog = crate::retail::items::read_item_catalog(
            &files.read("data/databases/items.dbs").unwrap().unwrap(),
        )
        .unwrap();
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
            let gender =
                if profession == Profession::Valkyrie { Gender::Female } else { Gender::Male };
            let character = Character::new("Test", race, gender, profession);
            let mut carried = Carried::default();
            for id in starting_items(&character) {
                let item =
                    make_item(catalog.get(ItemId(id)).unwrap(), &mut ids, &mut rng, true, true);
                place_at_creation(&mut carried, item, &catalog, &character).unwrap();
            }
            let name = |i: &Option<crate::game::inventory::ItemInstance>| {
                i.as_ref().map(|i| catalog.get(i.item).unwrap().name.clone())
            };
            println!("{race:?} {profession:?}");
            for (slot, item) in EquipSlot::ALL.iter().zip(&carried.equipment) {
                if let Some(name) = name(item) {
                    println!("  {slot:?}: {name}");
                }
            }
            for item in carried.backpack.iter().filter(|i| i.is_some()) {
                println!("  pack: {}", name(item).unwrap());
            }
            assert!(carried.worn(EquipSlot::Primary).is_some(), "{profession:?} holds a weapon");
        }
    }
}
