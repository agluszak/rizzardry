//! Who is in the party, in order.

use bevy::ecs::entity::EntityIndexSet;
use bevy::prelude::*;
use serde::Deserialize;

use crate::content::characters::ExtraItem;
use crate::content::items::ItemId;
use crate::game::characters::{Character, Gender, Profession, Race};
use crate::game::data::Catalogs;
use crate::game::items::{Outfit, make, roll_amount};
use crate::game::rng::GameRng;

/// The party: who adventures together, how they stand and what they share.
/// A campaign creates it when it enters its first level, where it also gets
/// its body ([`super::body::PartyBody`]).
#[derive(Component, Debug, Default)]
#[require(
    PartyRoster,
    super::formation::Formation,
    crate::game::inventory::PartyInventory,
    crate::game::items::Pool,
    crate::game::items::Hand,
    super::body::PartyMovement
)]
pub struct Party;

/// Membership of a party. It does not own the character: a member who
/// leaves (an NPC dismissed) keeps its identity and state.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
#[relationship(relationship_target = PartyRoster)]
#[component(on_insert = member_inserted, on_discard = member_removed)]
pub struct PartyMember(pub Entity);

/// The party's members in slot order, maintained by Bevy from
/// [`PartyMember`]. Each is an entity with a [`Character`] and the
/// items it wears and packs. Roster order is the order of rules, RNG draws and the UI.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = PartyMember)]
pub struct PartyRoster(EntityIndexSet);

impl std::ops::Deref for PartyRoster {
    type Target = EntityIndexSet;

    fn deref(&self) -> &EntityIndexSet {
        &self.0
    }
}

/// The party, for operations that hold the whole world while it plays.
pub fn the_party(world: &mut World) -> Entity {
    world.query_filtered::<Entity, With<Party>>().single(world).expect("the party")
}

/// A party of `members`, in order, each seated; for tests.
#[cfg(test)]
pub fn test_party(world: &mut World, members: &[Entity]) -> Entity {
    let party = world.spawn(Party).id();
    for &member in members {
        world.entity_mut(member).insert(PartyMember(party));
    }
    party
}

/// Someone joining a new party, as party creation (or a party file) makes
/// them.
#[derive(Clone, Debug, Deserialize)]
pub struct Recruit {
    pub name: String,
    pub race: Race,
    pub gender: Gender,
    pub profession: Profession,
    /// The portrait's file stem, such as `hummf`.
    pub portrait: String,
}

impl Recruit {
    /// The new character, before they join.
    pub fn character(&self) -> Character {
        let mut character = Character::new(&self.name, self.race, self.gender, self.profession);
        character.portrait = self.portrait.clone();
        character
    }
}

/// Commit a new member complete, as one operation: the profile with the
/// player's allocated points, vitals, learned spells with full spell
/// points, starting equipment at full quantity, identified and worn where
/// it fits (retail `AddCharacterStartingEquipment`), order, membership and
/// seat. Items are made in the starting set's order.
pub fn recruit_character(
    commands: &mut Commands,
    party: Entity,
    (character, learned): (Character, crate::game::magic::LearnedSpells),
    content: &Catalogs,
    rng: &mut GameRng,
) -> Entity {
    let starting = starting_items(&character);
    let points = crate::game::magic::SpellPoints::default();
    let vitals = crate::game::characters::Vitals::new(&character);
    let member = commands
        .spawn((
            Name::new(character.name.clone()),
            character,
            vitals,
            learned,
            points,
            PartyMember(party),
        ))
        .id();
    for id in starting {
        let definition = &content.items[id];
        let item = make(commands, definition, roll_amount(definition, rng, true), true);
        commands.trigger(Outfit { member, item });
    }
    member
}

/// A new character's starting items (retail `AddCharacterStartingEquipment`):
/// their race's set if it has its own, otherwise their profession's, then the
/// one extra item some professions add, chosen by their weapon skills.
pub fn starting_items(character: &Character) -> Vec<ItemId> {
    let row = character.profession.row();
    let set = character.race.row().starting_items.as_deref().unwrap_or(&row.starting_items);
    let extra = row.extra_item.map(|extra| match extra {
        ExtraItem::Always(item) => item,
        ExtraItem::ByTrainedSkill { skill, over, then, otherwise } => {
            if character.skill(skill) > character.skill(over) { then } else { otherwise }
        }
    });
    set.iter().copied().chain(extra).collect()
}

fn member_inserted(
    mut world: bevy::ecs::world::DeferredWorld,
    context: bevy::ecs::lifecycle::HookContext,
) {
    let member = context.entity;
    let party = world.get::<PartyMember>(member).unwrap().0;
    let dead = world.get::<crate::game::characters::Dead>(member).is_some();
    if let Some(mut formation) = world.get_mut::<super::formation::Formation>(party) {
        formation.seat(member);
        formation.update_seat(member, dead);
    }
}
fn member_removed(
    mut world: bevy::ecs::world::DeferredWorld,
    context: bevy::ecs::lifecycle::HookContext,
) {
    let party = world.get::<PartyMember>(context.entity).unwrap().0;
    if let Some(mut formation) = world.get_mut::<super::formation::Formation>(party) {
        formation.unseat(context.entity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::{Gender, Profession, Race, Skill};

    /// A Faerie carries their race's set whatever their profession; a
    /// Fighter's extra weapon follows the better of their axe and sword.
    #[test]
    fn starting_items_are_the_races_or_the_professions_with_an_extra_by_skill() {
        let mut fighter = Character::new("F", Race::Human, Gender::Male, Profession::Fighter);
        fighter.skill_points[Skill::Axe] = 9;
        fighter.skill_points[Skill::Sword] = 3;
        assert_eq!(
            starting_items(&fighter),
            [143, 153, 203, 246, 18].map(ItemId),
            "an axe fighter gets the battle axe"
        );
        fighter.skill_points[Skill::Axe] = 3;
        assert_eq!(starting_items(&fighter)[4], ItemId::LONGSWORD);
        let faerie = Character::new("P", Race::Faerie, Gender::Female, Profession::Mage);
        assert_eq!(starting_items(&faerie), [63, 169, 196, 335, 358, 0].map(ItemId));
    }
}
