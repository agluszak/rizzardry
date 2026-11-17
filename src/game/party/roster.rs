//! Who is in the party, in order.

use bevy::ecs::entity::EntityIndexSet;
use bevy::prelude::*;
use serde::Deserialize;

use crate::content::characters::ExtraItem;
use crate::content::items::ItemId;
use crate::game::characters::{Gender, Profession, Race};
use crate::game::data::Catalogs;
use crate::game::items::{Outfit, make, roll_amount};
use crate::game::rng::GameRng;

/// The party: who adventures together, how they stand and what they share.
/// A campaign creates it when it enters its first level, where it also gets
/// its body ([`super::body::PartyBody`]). A resource, so there is never more
/// than one, on the entity its members and what it holds are related to.
#[derive(Resource, Debug, Default)]
#[require(
    PartyRoster,
    super::formation::Formation,
    crate::game::inventory::PartyInventory,
    crate::game::items::Pool,
    crate::game::items::Hand,
    super::body::PartyMovement,
    crate::game::combat::monster::sight::PartyHides
)]
pub struct Party;

/// Membership of a party. It does not own the character: a member who
/// leaves (an NPC dismissed) keeps its identity and state. A member takes
/// orders in a fight.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
#[relationship(relationship_target = PartyRoster)]
#[require(super::Order)]
#[component(on_insert = member_inserted, on_discard = member_removed)]
pub struct PartyMember(pub Entity);

/// The party's members in slot order, maintained by Bevy from
/// [`PartyMember`]. Each is an entity made by [`crate::game::characters::born`] and the
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
    /// The new character as a draft, ready to join (`draft::Join`).
    pub fn draft(&self, commands: &mut Commands) -> Entity {
        use crate::game::characters::draft::{Conceive, Introduce, Newborn, newborn_bundle};
        let draft = commands.spawn((newborn_bundle(&self.name), Newborn)).id();
        let (race, gender, profession) = (self.race, self.gender, self.profession);
        commands.trigger(Conceive { draft, race, gender, profession });
        let (_, personality, voice) =
            crate::game::characters::creation::identity::defaults(race, gender, profession);
        let identity = crate::game::characters::Identity {
            name: self.name.clone(),
            full_name: self.name.clone(),
            personality,
            voice,
            portrait: self.portrait.clone(),
        };
        commands.trigger(Introduce { draft, identity });
        draft
    }
}

/// A new character's starting items (retail `AddCharacterStartingEquipment`):
/// their race's set if it has its own, otherwise their profession's, then the
/// one extra item some professions add, chosen by their weapon skills.
pub fn starting_items(
    race: crate::content::characters::Race,
    profession: crate::content::characters::Profession,
    skills: &crate::game::actors::Skills,
) -> Vec<ItemId> {
    let row = profession.row();
    let set = race.row().starting_items.as_deref().unwrap_or(&row.starting_items);
    let extra = row.extra_item.map(|extra| match extra {
        ExtraItem::Always(item) => item,
        ExtraItem::ByTrainedSkill { skill, over, then, otherwise } => {
            if skills[skill] > skills[over] { then } else { otherwise }
        }
    });
    set.iter().copied().chain(extra).collect()
}

/// A new member's starting equipment, made at full quantity, identified and
/// worn where it fits, in the set's order.
pub fn outfit(
    commands: &mut Commands,
    member: Entity,
    items: &[ItemId],
    content: &Catalogs,
    rng: &mut GameRng,
) {
    for &id in items {
        let definition = &content.items[id];
        let item = make(commands, definition, roll_amount(definition, rng, true), true);
        commands.trigger(Outfit { member, item });
    }
}

fn member_inserted(
    mut world: bevy::ecs::world::DeferredWorld,
    context: bevy::ecs::lifecycle::HookContext,
) {
    let member = context.entity;
    let party = world.get::<PartyMember>(member).unwrap().0;
    if let Some(mut formation) = world.get_mut::<super::formation::Formation>(party) {
        formation.seat(member);
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
    use crate::content::characters::{Profession, Race, Skill};

    /// A Faerie carries their race's set whatever their profession; a
    /// Fighter's extra weapon follows the better of their axe and sword.
    #[test]
    fn starting_items_are_the_races_or_the_professions_with_an_extra_by_skill() {
        let fighter = crate::game::characters::Calling::begun(Profession::Fighter);
        let calm = crate::game::characters::Conditions::default();
        let mut training = crate::game::characters::Training::default();
        training.0[Skill::Axe] = 9;
        training.0[Skill::Sword] = 3;
        let skills = crate::game::actors::Skills::affected(&training, &fighter, &calm);
        assert_eq!(
            starting_items(Race::Human, Profession::Fighter, &skills),
            [143, 153, 203, 246, 18].map(ItemId),
            "an axe fighter gets the battle axe"
        );
        training.0[Skill::Axe] = 3;
        let skills = crate::game::actors::Skills::affected(&training, &fighter, &calm);
        assert_eq!(starting_items(Race::Human, Profession::Fighter, &skills)[4], ItemId::LONGSWORD);
        let faerie = starting_items(Race::Faerie, Profession::Mage, &skills);
        assert_eq!(faerie, [63, 169, 196, 335, 358, 0].map(ItemId));
    }
}
