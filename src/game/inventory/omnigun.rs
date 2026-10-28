//! The Gadgeteer's omnigun, which grows with its owner's Gadgeteer level
//! (retail `UpgradeProfessionClassItem`).

use super::*;
use crate::game::characters::Profession;

/// The omnigun tier a Gadgeteer level earns: the item and its tier above the
/// first (1 to 11), none below level 3.
pub fn tier(level: u8) -> Option<(ItemId, usize)> {
    let tier = match level {
        0..=2 => return None,
        3 | 4 => 1,
        5 | 6 => 2,
        7 | 8 => 3,
        9 | 10 => 4,
        11 | 12 => 5,
        13..=15 => 6,
        16 | 17 => 7,
        18..=20 => 8,
        21 | 22 => 9,
        23..=25 => 10,
        _ => 11,
    };
    Some((ItemId(599 + tier as u16), tier))
}

/// Where the party's omnigun was found.
#[derive(Clone, Copy)]
enum At {
    Member(Entity),
    Hand,
    Pool,
}

/// Raise each Gadgeteer's omnigun to the tier their level earns, telling
/// them so. Retail does this when an advanced character is committed; here
/// it follows any change to a Gadgeteer, which raises nothing until their
/// level does. The omnigun is the first one found on them, worn or carried,
/// else anywhere in the party: the hand, each member in turn, then the pool.
///
/// Deviation, recorded: retail also gives a level 1 Gadgeteer who has lost
/// theirs a new one when the character is committed; character creation
/// already gives one, and there is no recommitting a character here.
pub fn upgrade_omnigun(
    changed: Query<(Entity, Ref<Character>)>,
    mut carried: Query<&mut Carried>,
    party: Single<(&PartyRoster, &mut PartyInventory)>,
    content: crate::game::data::Content,
    mut news: MessageWriter<ItemNews>,
) {
    let Some(catalog) = content.items() else { return };
    let (roster, mut inventory) = party.into_inner();
    let omnigun = |stack: &ItemStack| catalog.get(stack.item).is_some_and(|d| d.kind == OMNIGUN);
    for (member, character) in &changed {
        if !character.is_changed()
            || character.profession != Profession::Gadgeteer
            || !roster.contains(&member)
        {
            continue;
        }
        let Some((item, tier)) = tier(character.profession_levels[Profession::Gadgeteer]) else {
            continue;
        };
        let carries = |owner: Entity| carried.get(owner).is_ok_and(|c| c.stacks().any(omnigun));
        let at = if carries(member) {
            At::Member(member)
        } else if inventory.hand.held.as_ref().is_some_and(omnigun) {
            At::Hand
        } else {
            roster.iter().find(|&owner| carries(owner)).map_or(At::Pool, At::Member)
        };
        let current = match at {
            At::Member(owner) => {
                carried.get(owner).ok().and_then(|c| c.stacks().find(|s| omnigun(s)))
            }
            At::Hand => inventory.hand.held.as_ref(),
            At::Pool => inventory.pool.stacks.iter().find(|s| omnigun(s)),
        };
        if !current.is_some_and(|stack| stack.item < item) {
            continue;
        }
        let raise = |stack: &mut ItemStack| stack.item = item;
        match at {
            At::Member(owner) => {
                carried.get_mut(owner).unwrap().stacks_mut().find(|s| omnigun(s)).map(raise)
            }
            At::Hand => inventory.hand.held.as_mut().map(raise),
            At::Pool => inventory.pool.stacks.iter_mut().find(|s| omnigun(s)).map(raise),
        };
        news.write(ItemNews::Omnigun {
            member: character.name.clone(),
            gender: character.gender,
            tier,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::{Gender, Race};
    use crate::game::data::GameData;
    use bevy::ecs::system::RunSystemOnce;

    /// Items up to the omnigun's last tier, its twelve tiers of its kind.
    fn catalog() -> ItemCatalog {
        ItemCatalog(
            (0..=610)
                .map(|id| ItemDefinition {
                    id: ItemId(id),
                    kind: if id >= 599 { OMNIGUN } else { 0 },
                    ..default()
                })
                .collect(),
        )
    }

    /// A Gadgeteer's omnigun rises with their level, wherever in the party it
    /// is, and is told of once.
    #[test]
    fn the_omnigun_rises_a_tier_with_its_owners_level() {
        let mut world = World::new();
        world.init_resource::<Messages<ItemNews>>();
        let catalog = catalog();
        let mut ids = ItemIds::default();
        let omnigun = ItemStack::new(&catalog.0[599], &mut ids, 1, true);
        let mut catalogs = Assets::<ItemCatalog>::default();
        let items = catalogs.add(catalog);
        world.insert_resource(catalogs);
        world.insert_resource(GameData {
            items,
            loot: default(),
            species: default(),
            spells: default(),
        });
        let mut gadgeteer = Character::new("G", Race::Human, Gender::Female, Profession::Gadgeteer);
        gadgeteer.profession_levels[Profession::Gadgeteer] = 2;
        let member = world.spawn((Vitals::new(&gadgeteer), gadgeteer, Carried::default())).id();
        let party = crate::game::party::roster::test_party(&mut world, &[member]);
        world.get_mut::<PartyInventory>(party).unwrap().pool.stacks.push(omnigun.clone());
        let level = |world: &mut World, level: u8| {
            let mut character = world.get_mut::<Character>(member).unwrap();
            character.profession_levels[Profession::Gadgeteer] = level;
            world.run_system_once(upgrade_omnigun).unwrap();
            let stack = world.get::<PartyInventory>(party).unwrap().pool.stacks[0].clone();
            let news: Vec<_> = world.resource_mut::<Messages<ItemNews>>().drain().collect();
            (stack.item.0, stack.id, news)
        };
        assert_eq!(level(&mut world, 2), (599, omnigun.id, vec![]));
        let news = vec![ItemNews::Omnigun { member: "G".into(), gender: Gender::Female, tier: 1 }];
        assert_eq!(level(&mut world, 3), (600, omnigun.id, news));
        assert_eq!(level(&mut world, 4), (600, omnigun.id, vec![]), "told of once");
        assert_eq!(level(&mut world, 26).0, 610);
        assert_eq!(tier(25), Some((ItemId(609), 10)));
    }
}
