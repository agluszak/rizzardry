//! Who is in the party, in order.

use bevy::ecs::entity::EntityIndexSet;
use bevy::prelude::*;
use serde::Deserialize;

use crate::content::characters::Skill;
use crate::content::items::{ItemCatalog, ItemId};
use crate::game::characters::{Character, Gender, Profession, Race};
use crate::game::data::GameData;
use crate::game::inventory::{Carried, ItemIds, make_item, place_at_creation};
use crate::game::rng::GameRng;

/// The party: who adventures together, how they stand and what they share.
/// A campaign creates it when it enters its first level, where it also gets
/// its body ([`super::body::PartyBody`]).
#[derive(Component, Debug, Default)]
#[require(
    super::formation::Formation,
    crate::game::inventory::PartyInventory,
    super::body::PartyMovement
)]
pub struct Party;

/// Membership of a party. It does not own the character: a member who
/// leaves (an NPC dismissed) keeps its identity and state.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
#[relationship(relationship_target = PartyRoster)]
pub struct PartyMember(pub Entity);

/// The party's members in slot order, maintained by Bevy from
/// [`PartyMember`]. Each is an entity with a [`Character`] and what it
/// [`Carried`]. Roster order is the order of rules, RNG draws and the UI.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = PartyMember)]
pub struct PartyRoster(EntityIndexSet);

impl PartyRoster {
    /// The members' average level (retail `GetAveragePartyLevel`), which
    /// loot and stock scale by; 1 for an empty party, where retail divides
    /// by zero.
    pub fn average_level(&self, level_of: impl Fn(Entity) -> Option<u8>) -> u32 {
        let (sum, count) = self
            .iter()
            .filter_map(|member| level_of(member))
            .fold((0, 0), |(sum, count), level| (sum + u32::from(level), count + 1));
        sum.checked_div(count).unwrap_or(1)
    }
}

impl std::ops::Deref for PartyRoster {
    type Target = EntityIndexSet;

    fn deref(&self) -> &EntityIndexSet {
        &self.0
    }
}

/// The party, for operations that hold the whole world.
pub fn the_party(world: &mut World) -> Option<Entity> {
    world.query_filtered::<Entity, With<Party>>().single(world).ok()
}

/// A party of `members`, in order, each seated; for tests.
#[cfg(test)]
pub fn test_party(world: &mut World, members: &[Entity]) -> Entity {
    let party = world.spawn(Party).id();
    for &member in members {
        world.entity_mut(member).insert(PartyMember(party));
        world.get_mut::<super::formation::Formation>(party).unwrap().seat(member);
    }
    party
}

/// Add `member` to the end of the party and seat them; for tests.
#[cfg(test)]
pub fn test_join(world: &mut World, member: Entity) {
    let party = the_party(world).unwrap();
    world.entity_mut(member).insert(PartyMember(party));
    world.get_mut::<super::formation::Formation>(party).unwrap().seat(member);
}

/// Put the party's members in `order`; for tests.
#[cfg(test)]
pub fn test_reorder(world: &mut World, order: &[Entity]) {
    let party = the_party(world).unwrap();
    for &member in order {
        world.entity_mut(member).remove::<PartyMember>().insert(PartyMember(party));
    }
}

/// End a membership without transferring its formation seat to whoever
/// follows in roster order. The character entity remains.
pub fn leave(commands: &mut Commands, member: Entity, formation: &mut super::formation::Formation) {
    commands.entity(member).remove::<PartyMember>();
    formation.unseat(member);
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

/// Commit the completed profile, including the player's allocated points.
pub fn recruit_character(
    commands: &mut Commands,
    party: Entity,
    formation: &mut super::formation::Formation,
    character: Character,
) -> Entity {
    let vitals = crate::game::characters::Vitals::new(&character);
    let member = commands
        .spawn((
            Name::new(character.name.clone()),
            character,
            vitals,
            super::Order::Attack,
            crate::game::magic::LearnedSpells::default(),
            NeedsEquipment,
            PartyMember(party),
        ))
        .id();
    formation.seat(member);
    member
}

/// A new member still to receive their starting equipment.
#[derive(Component, Debug)]
pub struct NeedsEquipment;

/// A new character's starting items (retail `g_starting_equipment` and
/// `AddCharacterStartingEquipment`): their profession's set (Faeries have
/// their own), then one extra item for some professions, chosen by their
/// weapon skills.
pub fn starting_items(character: &Character) -> Vec<ItemId> {
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
    roster: Single<&PartyRoster>,
) {
    let Some(catalog) = data.and_then(|d| catalogs.get(&d.items)) else { return };
    for (entity, character) in members.iter_many(roster.iter()).matched() {
        let mut carried = Carried::default();
        for id in starting_items(character) {
            let Some(definition) = catalog.get(id) else { continue };
            let item = make_item(definition, &mut ids, &mut rng, true, true);
            if place_at_creation(&mut carried, item, catalog, character).is_some() {
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

    #[test]
    fn recruitment_and_removal_settle_formation_without_inheriting_anothers_order() {
        use crate::game::party::{Order, formation::Formation};
        use bevy::ecs::system::SystemState;
        let mut world = World::new();
        let party = world.spawn(Party).id();
        let definition = Recruit {
            name: "First".into(),
            race: Race::Human,
            gender: Gender::Male,
            profession: Profession::Fighter,
            portrait: String::new(),
        };
        let mut operations = SystemState::<(Commands, Single<&mut Formation>)>::new(&mut world);
        let (mut commands, mut formation) = operations.get_mut(&mut world).unwrap();
        let first = recruit_character(&mut commands, party, &mut formation, definition.character());
        let second = recruit_character(
            &mut commands,
            party,
            &mut formation,
            Recruit { name: "Second".into(), ..definition.clone() }.character(),
        );
        operations.apply(&mut world);
        let place = world.get::<Formation>(party).unwrap().place_of(second);
        world.entity_mut(first).insert(Order::Defend);
        let (mut commands, mut formation) = operations.get_mut(&mut world).unwrap();
        leave(&mut commands, first, &mut formation);
        let replacement = recruit_character(
            &mut commands,
            party,
            &mut formation,
            Recruit { name: "Replacement".into(), ..definition }.character(),
        );
        operations.apply(&mut world);
        assert_eq!(
            world.get::<PartyRoster>(party).unwrap().iter().collect::<Vec<_>>(),
            [second, replacement]
        );
        let formation = world.get::<Formation>(party).unwrap();
        assert!(formation.place_of(first).is_none());
        assert_eq!(formation.place_of(second), place);
        assert!(formation.place_of(replacement).is_some());
        assert_eq!(world.get::<Order>(replacement), Some(&Order::Attack));
        assert_eq!(
            world.get::<Order>(first),
            Some(&Order::Defend),
            "the retained actor owns its preference"
        );
        assert!(world.get::<Character>(first).is_some());
    }
}
