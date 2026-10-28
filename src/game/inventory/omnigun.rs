//! The Gadgeteer's omnigun, which grows with its owner's Gadgeteer level as
//! their advancement is committed (retail `UpgradeProfessionClassItem`).

use super::*;
use crate::content::characters::Profession;
use crate::content::items::ItemKind;

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

/// Give or raise a Gadgeteer's omnigun for the level their advancement has
/// just committed (retail `UpgradeProfessionClassItem`, from
/// `CommitCharacter`), returning what to tell. A level 1 Gadgeteer without
/// one of their own is given one, to them or else the party, unless neither
/// has room. From level 3 the first omnigun found (theirs, worn or carried,
/// else the hand's, each member's in turn, then the pool's) rises to the
/// tier their level earns, never down.
pub fn advance(
    member: Entity,
    character: &Character,
    roster: &PartyRoster,
    carried: &mut Query<&mut Carried>,
    party: &mut PartyInventory,
    catalog: &ItemCatalog,
    (ids, rng): (&mut ItemIds, &mut GameRng),
) -> Vec<ItemNews> {
    if character.profession != Profession::Gadgeteer {
        return Vec::new();
    }
    let omnigun =
        |stack: &ItemStack| catalog.get(stack.item).is_some_and(|d| d.kind == ItemKind::OMNIGUN);
    let carries = |carried: &Query<&mut Carried>, owner: Entity| {
        carried.get(owner).is_ok_and(|c| c.stacks().any(omnigun))
    };
    let told =
        |tier| ItemNews::Omnigun { member: character.name.clone(), gender: character.gender, tier };
    let level = character.profession_levels[Profession::Gadgeteer];
    if level == 1 {
        let Some(definition) = catalog.get(ItemId(599)) else { return Vec::new() };
        if carries(carried, member) {
            return Vec::new();
        }
        let mut given = make_item(definition, ids, rng, false, true);
        given.curse_known = true;
        let item = given.display_name(definition, false);
        let rest = match carried.get_mut(member) {
            Ok(mut own) => own.receive(given, catalog),
            Err(_) => Some(given),
        };
        let taker = match rest.map(|rest| party.pool.receive(rest, catalog)) {
            None => Some(character.name.clone()),
            Some(None) => None,
            Some(Some(_)) => return Vec::new(),
        };
        return vec![ItemNews::Takes { member: taker, item }, told(0)];
    }
    let Some((item, tier)) = tier(level) else { return Vec::new() };
    let at = if carries(carried, member) {
        At::Member(member)
    } else if party.hand.held.as_ref().is_some_and(omnigun) {
        At::Hand
    } else {
        roster.iter().find(|&owner| carries(carried, owner)).map_or(At::Pool, At::Member)
    };
    let current = match at {
        At::Member(owner) => carried.get(owner).ok().and_then(|c| c.stacks().find(|s| omnigun(s))),
        At::Hand => party.hand.held.as_ref(),
        At::Pool => party.pool.stacks.iter().find(|s| omnigun(s)),
    };
    if !current.is_some_and(|stack| stack.item < item) {
        return Vec::new();
    }
    let raise = |stack: &mut ItemStack| stack.item = item;
    match at {
        At::Member(owner) => {
            carried.get_mut(owner).unwrap().stacks_mut().find(|s| omnigun(s)).map(raise)
        }
        At::Hand => party.hand.held.as_mut().map(raise),
        At::Pool => party.pool.stacks.iter_mut().find(|s| omnigun(s)).map(raise),
    };
    vec![told(tier)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::{Gender, Race};
    use bevy::ecs::system::RunSystemOnce;

    /// Items up to the omnigun's last tier, its twelve tiers of its kind.
    fn catalog() -> ItemCatalog {
        ItemCatalog(
            (0..=610)
                .map(|id| ItemDefinition {
                    id: ItemId(id),
                    kind: if id >= 599 { ItemKind::OMNIGUN } else { ItemKind::default() },
                    ..default()
                })
                .collect(),
        )
    }

    /// Committing a Gadgeteer's level gives a level 1 one their omnigun and
    /// raises it a tier from level 3, wherever in the party it is, telling
    /// each once.
    #[test]
    fn committed_levels_give_and_raise_the_omnigun() {
        let mut world = World::new();
        world.init_resource::<ItemIds>();
        world.init_resource::<GameRng>();
        let catalog = std::sync::Arc::new(catalog());
        let mut gadgeteer = Character::new("G", Race::Human, Gender::Female, Profession::Gadgeteer);
        gadgeteer.profession_levels[Profession::Gadgeteer] = 1;
        let member = world.spawn((Vitals::new(&gadgeteer), Carried::default())).id();
        let party = crate::game::party::roster::test_party(&mut world, &[member]);
        let mut commit = |world: &mut World, level: u8| {
            gadgeteer.profession_levels[Profession::Gadgeteer] = level;
            let (character, catalog) = (gadgeteer.clone(), catalog.clone());
            world
                .run_system_once(
                    move |roster: Single<&PartyRoster>,
                          mut carried: Query<&mut Carried>,
                          mut party: Single<&mut PartyInventory>,
                          (mut ids, mut rng): (ResMut<ItemIds>, ResMut<GameRng>)| {
                        advance(
                            member,
                            &character,
                            &roster,
                            &mut carried,
                            &mut party,
                            &catalog,
                            (&mut ids, &mut rng),
                        )
                    },
                )
                .unwrap()
        };
        let told = |tier| ItemNews::Omnigun { member: "G".into(), gender: Gender::Female, tier };
        let given = commit(&mut world, 1);
        assert_eq!(given.last(), Some(&told(0)));
        assert_eq!(commit(&mut world, 1), vec![], "one is enough");
        // Handed to the pool, it still rises.
        let gun = world.get_mut::<Carried>(member).unwrap().backpack[0].take().unwrap();
        world.get_mut::<PartyInventory>(party).unwrap().pool.stacks.push(gun.clone());
        assert_eq!(commit(&mut world, 2), vec![]);
        assert_eq!(commit(&mut world, 3), vec![told(1)]);
        assert_eq!(commit(&mut world, 4), vec![], "told of once");
        assert_eq!(commit(&mut world, 26), vec![told(11)]);
        let pool = &world.get::<PartyInventory>(party).unwrap().pool.stacks;
        assert_eq!((pool[0].item, pool[0].id), (ItemId(610), gun.id), "the same gun, risen");
    }
}
