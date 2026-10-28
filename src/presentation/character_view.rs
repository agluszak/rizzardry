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
    pub hand: Option<&'static crate::game::combat::attack::Hand>,
    pub member: Option<&'static crate::game::party::roster::PartyMember>,
}

impl CharacterViewItem<'_, '_> {
    /// Derive the sheet from live equipment/stats, spells and the retained
    /// round action. The party's protections count for its members only.
    pub fn armour(
        &self,
        actor: bevy::prelude::Entity,
        catalog: Option<&crate::content::items::ItemCatalog>,
        battle: Option<&crate::game::combat::Battle>,
        party: Option<&crate::game::magic::enchantments::PartyEffects>,
    ) -> Option<crate::game::combat::attack::Armour> {
        Some(crate::game::combat::attack::enchanted_armour(
            &self.profile.stats(&self.vitals.enchantments),
            self.carried?,
            catalog?,
            self.load(),
            battle.is_some_and(|battle| battle.defends(actor)),
            (&self.vitals.enchantments, party.filter(|_| self.member.is_some()).map(|p| &p.0)),
        ))
    }

    /// The member's load, none for those who carry nothing.
    pub fn load(&self) -> crate::game::characters::Load {
        self.burden.map_or_else(Default::default, |burden| burden.load)
    }
}
