//! Who is in the party, in order.

use bevy::prelude::*;

use crate::content::items::{ItemCatalog, ItemId};
use crate::content::characters::Skill;
use crate::game::characters::{Character, Gender, Profession, Race};
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

/// A new character's starting items (retail `g_starting_equipment` and
/// `AddCharacterStartingEquipment`): their profession's set (Faeries have
/// their own), then one extra item for some professions, chosen by their
/// weapon skills.
fn starting_items(character: &Character) -> Vec<ItemId> {
    use Profession::*;
    let set: &[u16] = match character.profession {
        _ if character.race == Race::Faerie => &[63, 169, 196, 335, 358, 0],
        Fighter => &[143, 153, 203, 246],
        Lord => &[35, 1, 158, 204, 247, 210],
        Valkyrie => &[75, 170, 197, 246, 215],
        Ranger => &[110, 125, 157, 188, 247],
        Samurai => &[38, 49, 165, 192, 246],
        Ninja => &[105, 230, 180, 209, 253],
        Monk => &[79, 105, 165, 192, 246],
        Rogue => &[0, 0, 99, 156, 187, 247],
        Gadgeteer => &[132, 156, 187, 246],
        Bard => &[769, 116, 132, 156, 187],
        Priest => &[165, 192, 246, 335],
        Alchemist => &[28, 165, 192, 246, 350],
        Bishop => &[82, 165, 192, 246, 223],
        Psionic => &[0, 165, 192, 246, 347],
        Mage => &[28, 165, 192, 246, 385],
    };
    let skill = |s| character.skill(s);
    let extra = match character.profession {
        Priest if skill(Skill::MaceAndFlail) > skill(Skill::StaffAndWand) => Some(0x16),
        Priest => Some(0x52),
        Fighter if skill(Skill::Axe) > skill(Skill::Sword) => Some(0x12),
        Fighter => Some(7),
        Gadgeteer => Some(599),
        Bard => Some(0x144),
        _ => None,
    };
    set.iter().copied().chain(extra).map(ItemId).collect()
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
            let Some(definition) = catalog.get(id) else { continue };
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
        let catalog = crate::retail::items::retail_catalog(&files);
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
                    make_item(catalog.get(id).unwrap(), &mut ids, &mut rng, true, true);
                place_at_creation(&mut carried, item, &catalog, &character).unwrap();
            }
            let name = |i: &Option<crate::game::inventory::ItemInstance>| {
                i.as_ref().map(|i| catalog.get(i.item).unwrap().name.clone())
            };
            println!("{race:?} {profession:?}");
            for (slot, item) in &carried.equipment {
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
