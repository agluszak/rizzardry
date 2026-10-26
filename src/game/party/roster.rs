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

/// The party's members in slot order. Each is an entity with a
/// [`Character`] and what it [`Carried`].
#[derive(Resource, Debug, Default)]
pub struct PartyRoster(pub EntityIndexSet);

impl PartyRoster {
    /// Remove membership without transferring its formation seat to whoever
    /// follows in roster order. The character entity may remain as an NPC.
    pub fn remove(&mut self, member: Entity, formation: &mut super::formation::Formation) -> bool {
        let removed = self.0.shift_remove(&member);
        if removed {
            formation.unseat(member);
        }
        removed
    }
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

/// Add a new member at the end of the party; they receive their starting
/// equipment once the items have loaded.
pub fn recruit(
    commands: &mut Commands,
    roster: &mut PartyRoster,
    formation: &mut super::formation::Formation,
    recruit: &Recruit,
) -> Entity {
    let mut character =
        Character::new(&recruit.name, recruit.race, recruit.gender, recruit.profession);
    character.portrait = recruit.portrait.clone();
    let vitals = crate::game::characters::Vitals::new(&character);
    let member = commands
        .spawn((
            Name::new(recruit.name.clone()),
            character,
            vitals,
            super::Order::Attack,
            NeedsEquipment,
        ))
        .id();
    roster.0.insert(member);
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
    members: Query<&Character, With<NeedsEquipment>>,
    roster: Res<PartyRoster>,
) {
    let Some(catalog) = data.and_then(|d| catalogs.get(&d.items)) else { return };
    for &entity in &roster.0 {
        let Ok(character) = members.get(entity) else { continue };
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
                let item = make_item(catalog.get(id).unwrap(), &mut ids, &mut rng, true, true);
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
    #[test]
    fn recruitment_and_removal_settle_formation_without_inheriting_anothers_order() {
        use crate::game::party::{Order, formation::Formation};
        use bevy::ecs::system::SystemState;
        let mut world = World::new();
        world.init_resource::<PartyRoster>();
        world.init_resource::<Formation>();
        let definition = Recruit {
            name: "First".into(),
            race: Race::Human,
            gender: Gender::Male,
            profession: Profession::Fighter,
            portrait: String::new(),
        };
        let mut operations =
            SystemState::<(Commands, ResMut<PartyRoster>, ResMut<Formation>)>::new(&mut world);
        let (mut commands, mut roster, mut formation) = operations.get_mut(&mut world).unwrap();
        let first = recruit(&mut commands, &mut roster, &mut formation, &definition);
        let second = recruit(
            &mut commands,
            &mut roster,
            &mut formation,
            &Recruit { name: "Second".into(), ..definition.clone() },
        );
        operations.apply(&mut world);
        let place = world.resource::<Formation>().place_of(second);
        world.entity_mut(first).insert(Order::Defend);
        let (mut commands, mut roster, mut formation) = operations.get_mut(&mut world).unwrap();
        assert!(roster.remove(first, &mut formation));
        let replacement = recruit(
            &mut commands,
            &mut roster,
            &mut formation,
            &Recruit { name: "Replacement".into(), ..definition },
        );
        operations.apply(&mut world);
        assert_eq!(
            world.resource::<PartyRoster>().0.iter().copied().collect::<Vec<_>>(),
            [second, replacement]
        );
        assert!(world.resource::<Formation>().place_of(first).is_none());
        assert_eq!(world.resource::<Formation>().place_of(second), place);
        assert!(world.resource::<Formation>().place_of(replacement).is_some());
        assert_eq!(world.get::<Order>(replacement), Some(&Order::Attack));
        assert_eq!(
            world.get::<Order>(first),
            Some(&Order::Defend),
            "the retained actor owns its preference"
        );
        assert!(world.get::<Character>(first).is_some());
    }
}
