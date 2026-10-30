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
    pub carried: &'static Carried,
    pub burden: &'static Burden,
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
    ) -> crate::game::combat::attack::Armour {
        crate::game::combat::attack::enchanted_armour(
            &self.profile.stats(&self.vitals.enchantments),
            self.carried,
            catalog,
            self.load(),
            battle.is_some_and(|battle| battle.defends(actor)),
            (&self.vitals.enchantments, party.filter(|_| self.member.is_some()).map(|p| &p.0)),
        )
    }

    /// The main hand's numbers, as the sheet shows them.
    pub fn hand(
        &self,
        catalog: &crate::content::items::ItemCatalog,
    ) -> crate::game::combat::attack::Hand {
        crate::game::combat::attack::primary_hand(
            &self.profile.stats(&self.vitals.enchantments),
            self.carried,
            catalog,
            self.load(),
        )
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
        match party.filter(|_| self.member.is_some()) {
            Some(party) => member_resistances(&stats, self.carried, catalog, &party.0),
            None => resistances(&stats, self.carried, catalog),
        }
    }

    /// The member's load.
    pub fn load(&self) -> crate::game::characters::Load {
        self.burden.load
    }
}
