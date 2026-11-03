//! What a body is to the effects that land on it: the level its saves and
//! resistance weigh, and its resistance in each realm. The same components
//! on a member and a monster, each produced from its own sources (a
//! member's sheet, gear and party; a monster's species and spells) at the
//! start of each pass of the hit stages; the save and resistance systems
//! read them without asking which it is.

use bevy::prelude::*;
use enum_map::EnumMap;

use crate::content::characters::Realm;
use crate::game::characters::resistance::{member_resistances, resistances};
use crate::game::characters::{Character, Vitals};
use crate::game::data::Catalogs;
use crate::game::inventory::Carried;
use crate::game::magic::enchantments::PartyEffects;
use crate::game::party::roster::PartyMember;
use crate::game::world::population::{self, Monster};

/// The level a body's saves and resistance weigh against an effect's
/// strength: a member's, a monster's species'.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SaveLevel(pub u32);

/// A body's resistance in each realm: a member's from their stats, what
/// they wear and, in the party, its spells; a monster's from its species
/// and spells.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Resistances(pub EnumMap<Realm, i32>);

/// Characters, by their sheet and gear, and a member by the party's spells.
pub fn judge_characters(
    mut commands: Commands,
    characters: Query<(Entity, &Character, &Vitals, &Carried, Has<PartyMember>)>,
    party: Single<&PartyEffects>,
    content: If<Res<Catalogs>>,
) {
    for (entity, character, vitals, carried, member) in &characters {
        let stats = character.stats(&vitals.enchantments);
        let resistances = match member {
            true => member_resistances(&stats, carried, &content.items, &party.0),
            false => resistances(&stats, carried, &content.items),
        };
        commands.entity(entity).insert((
            SaveLevel(u32::from(character.level())),
            Resistances(EnumMap::from_fn(|realm| resistances[realm].total)),
        ));
    }
}

/// Monsters, by their species and spells.
pub fn judge_monsters(
    mut commands: Commands,
    monsters: Query<(Entity, &Monster, &Vitals)>,
    content: If<Res<Catalogs>>,
) {
    for (entity, monster, vitals) in &monsters {
        let kind = &content.species[monster.species];
        commands.entity(entity).insert((
            SaveLevel(u32::from(kind.level)),
            Resistances(EnumMap::from_fn(|realm| population::resistance(kind, vitals, realm))),
        ));
    }
}
