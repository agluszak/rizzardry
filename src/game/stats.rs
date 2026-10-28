//! Members' derived stats: what follows from their profile, belongings and
//! load (their load itself, armour, main hand, resistances and spell-point
//! maximum), kept on them and refreshed at one point in each update,
//! [`GameplaySet::Stats`]. Everything after it in the update reads them
//! current; a change made later shows from the next update's refresh.
//! Transient modifiers, such as defending, are applied where they are used.

use bevy::prelude::*;

use crate::game::GameplaySet;
use crate::game::characters::Character;
use crate::game::characters::resistance::{Resistances, resistances};
use crate::game::combat::Battle;
use crate::game::combat::attack::{Armour, Hand, armour, primary_hand};
use crate::game::inventory::{Burden, Carried, distribute_burden};
use crate::game::magic::sync_spell_points;
use crate::game::world::level::level_active;

pub(crate) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        (
            // A fight keeps the loads it began with (retail
            // `RedistributePartyEncumbrance` defers while in combat).
            distribute_burden.run_if(not(any_with_component::<Battle>)),
            (sync_stats, sync_spell_points),
        )
            .chain()
            .in_set(GameplaySet::Stats)
            .run_if(level_active),
    );
}

/// Keep each member's armour, main hand and resistances current, recomputing
/// them only when something they are made of changes: the character, what
/// they wear or their load.
pub fn sync_stats(
    mut members: Query<(
        (Ref<Character>, Ref<Carried>, Ref<Burden>),
        (&mut Armour, &mut Hand, &mut Resistances),
    )>,
    content: crate::game::data::Content,
    mut had_catalog: Local<bool>,
) {
    let Some(catalog) = content.items() else { return };
    // Everyone's stats are settled once the item catalog has loaded.
    let loaded = !std::mem::replace(&mut *had_catalog, true);
    for ((character, carried, burden), (mut stored_armour, mut hand, mut stored_resistances)) in
        &mut members
    {
        let worn = loaded || character.is_changed() || carried.is_changed();
        if worn {
            stored_resistances.set_if_neq(Resistances(resistances(&character, &carried, catalog)));
        }
        if worn || burden.is_changed() {
            let load = burden.load;
            hand.set_if_neq(primary_hand(&character, &carried, catalog, load));
            stored_armour.set_if_neq(armour(&character, &carried, catalog, load));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::{Gender, Profession, Race, Realm};
    use crate::content::items::{EquipClass, ItemCatalog, ItemDefinition, ItemId};
    use crate::game::characters::Vitals;
    use crate::game::data::GameData;
    use crate::game::inventory::{EquipSlot, Equipped, ItemIds, ItemStack};
    use enumset::EnumSet;

    #[derive(Resource, Default)]
    struct Seen(Vec<i32>);

    /// Equipment changed before the boundary (as the inventory's clicks
    /// change it) counts for what reads it after, spells and rounds alike,
    /// within the same update.
    #[test]
    fn equipment_changed_before_the_boundary_counts_in_the_same_update() {
        let ring = ItemDefinition {
            id: ItemId(0),
            class: EquipClass::Misc,
            resistance_bonus: enum_map::enum_map! { Realm::Fire => 30, _ => 0 },
            professions: EnumSet::all(),
            races: EnumSet::all(),
            genders: EnumSet::all(),
            ..Default::default()
        };
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<ItemCatalog>>()
            .configure_sets(Update, (GameplaySet::Stats, GameplaySet::Combat).chain())
            .init_resource::<Seen>()
            .add_plugins(plugin);
        let items = app
            .world_mut()
            .resource_mut::<Assets<ItemCatalog>>()
            .add(ItemCatalog(vec![ring.clone()]));
        app.insert_resource(GameData {
            items,
            loot: default(),
            species: default(),
            spells: default(),
        });
        app.world_mut().spawn(crate::game::world::level::Level::test(default()));
        let character = Character::new("A", Race::Human, Gender::Male, Profession::Fighter);
        let member =
            app.world_mut().spawn((Vitals::new(&character), character, Carried::default())).id();
        crate::game::party::roster::test_party(app.world_mut(), &[member]);
        // Where spell resolution reads resistances.
        app.add_systems(
            Update,
            (move |members: Query<&Resistances>, mut seen: ResMut<Seen>| {
                seen.0.push(members.get(member).unwrap().0[Realm::Fire].total);
            })
            .after(GameplaySet::Stats)
            .before(GameplaySet::Combat),
        );
        app.update();
        let ring = ItemStack::new(&ring, &mut ItemIds::default(), 1, true);
        app.world_mut().get_mut::<Carried>(member).unwrap().equipment[EquipSlot::Misc1] =
            Some(Equipped::new(ring));
        app.update();
        let seen = &app.world().resource::<Seen>().0;
        assert_eq!(seen[1], seen[0] + 30, "the ring counts in the update it is worn");
    }
}
