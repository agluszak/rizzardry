//! Who is in the party, in order.

use bevy::prelude::*;

use crate::game::characters::{Character, Gender, Profession, Race};
use crate::game::inventory::Carried;

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
        roster
            .0
            .push(commands.spawn((Name::new(name.to_owned()), character, Carried::default())).id());
    }
}
