//! Named read-only view of the components needed by character summaries.
use crate::game::{
    characters::{Character, Vitals},
    inventory::{Burden, Carried},
};
use bevy::ecs::query::QueryData;

#[derive(QueryData)]
pub struct CharacterView {
    pub profile: &'static Character,
    pub vitals: &'static Vitals,
    pub carried: Option<&'static Carried>,
    pub burden: Option<&'static Burden>,
    pub member: Option<&'static crate::game::party::roster::PartyMember>,
}

impl CharacterViewItem<'_, '_> {
    /// Derive the sheet from live equipment/stats, spells and the retained
    /// round action. The party's protections count for its members only.
    pub fn armour(
        &self,
        actor: bevy::prelude::Entity,
        catalog: &crate::content::items::ItemCatalog,
        battle: Option<&crate::game::combat::Battle>,
        party: Option<&crate::game::magic::enchantments::PartyEffects>,
    ) -> Option<crate::game::combat::attack::Armour> {
        Some(crate::game::combat::attack::enchanted_armour(
            &self.profile.stats(&self.vitals.enchantments),
            self.carried?,
            catalog,
            self.load(),
            battle.is_some_and(|battle| battle.defends(actor)),
            (&self.vitals.enchantments, party.filter(|_| self.member.is_some()).map(|p| &p.0)),
        ))
    }

    /// The main hand's numbers, as the sheet shows them; none for those who
    /// carry nothing.
    pub fn hand(
        &self,
        catalog: &crate::content::items::ItemCatalog,
    ) -> Option<crate::game::combat::attack::Hand> {
        Some(crate::game::combat::attack::primary_hand(
            &self.profile.stats(&self.vitals.enchantments),
            self.carried?,
            catalog,
            self.load(),
        ))
    }

    /// The member's resistances, the party's protections included for its
    /// members.
    pub fn resistances(
        &self,
        catalog: &crate::content::items::ItemCatalog,
        party: Option<&crate::game::magic::enchantments::PartyEffects>,
    ) -> enum_map::EnumMap<
        crate::content::characters::Realm,
        crate::game::characters::resistance::Resistance,
    > {
        use crate::game::characters::resistance::{member_resistances, resistances};
        let stats = self.profile.stats(&self.vitals.enchantments);
        let empty = Carried::default();
        let carried = self.carried.unwrap_or(&empty);
        match party.filter(|_| self.member.is_some()) {
            Some(party) => member_resistances(&stats, carried, catalog, &party.0),
            None => resistances(&stats, carried, catalog),
        }
    }

    /// The member's load, none for those who carry nothing.
    pub fn load(&self) -> crate::game::characters::Load {
        self.burden.map_or_else(Default::default, |burden| burden.load)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::{Attribute, Gender, Profession, Race, Realm};
    use crate::content::items::{ItemCatalog, ItemDefinition, ItemId};
    use crate::game::inventory::{EquipSlot, Equipped, ItemIds, ItemStack};
    use crate::game::magic::enchantments::{Enchantment, PartyEffects};
    use bevy::prelude::*;

    /// What a member wears and is made of counts at once, in the hand they
    /// strike with and the resistances a spell meets: nothing has to be
    /// refreshed in between.
    #[test]
    fn equipment_attributes_and_party_spells_count_as_soon_as_they_change() {
        let ring = ItemDefinition {
            id: ItemId(0),
            class: crate::content::items::EquipClass::Misc,
            resistance_bonus: enum_map::enum_map! { Realm::Fire => 30, _ => 0 },
            professions: enumset::EnumSet::all(),
            races: enumset::EnumSet::all(),
            genders: enumset::EnumSet::all(),
            ..Default::default()
        };
        let catalog = ItemCatalog(vec![ring.clone()]);
        let mut world = World::new();
        let mut character = Character::new("A", Race::Human, Gender::Male, Profession::Fighter);
        character.attributes[Attribute::Strength] = 50;
        let member = world
            .spawn((Vitals::new(&character), character, Carried::default(), Burden::default()))
            .id();
        crate::game::party::roster::test_party(&mut world, &[member]);
        let party = crate::game::party::roster::the_party(&mut world);
        let read = |world: &mut World| {
            let mut views = world.query::<CharacterView>();
            let mut effects = world.query::<&PartyEffects>();
            let party = effects.iter(world).next();
            let view = views.get(world, member).unwrap();
            (
                view.hand(&catalog).unwrap().to_hit,
                view.resistances(&catalog, party)[Realm::Fire].total,
            )
        };
        let (to_hit, fire) = read(&mut world);
        let worn = ItemStack::new(&ring, &mut ItemIds::default(), 1, true);
        world.get_mut::<Carried>(member).unwrap().equipment[EquipSlot::Misc1] =
            Some(Equipped::new(worn));
        world.get_mut::<Character>(member).unwrap().attributes[Attribute::Strength] += 20;
        let (stronger, warded) = read(&mut world);
        assert_eq!(stronger, to_hit + 2, "strength counts in the next swing");
        assert_eq!(warded, fire + 30, "the ring counts against the next spell");
        world.get_mut::<PartyEffects>(party).unwrap().0.apply(Enchantment {
            spell: crate::content::spells::SpellId::MAGIC_SCREEN,
            power: 1,
            boost: 0,
            turns: 5,
            absorption: 0,
        });
        assert_eq!(read(&mut world).1, warded + 11, "and so does the party's Magic Screen");
    }
}
