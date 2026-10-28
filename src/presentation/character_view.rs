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
}

impl CharacterViewItem<'_, '_> {
    /// Derive the sheet from live equipment/stats and the retained round action.
    pub fn armour(
        &self,
        actor: bevy::prelude::Entity,
        catalog: Option<&crate::content::items::ItemCatalog>,
        battle: Option<&crate::game::combat::Battle>,
    ) -> Option<crate::game::combat::attack::Armour> {
        Some(crate::game::combat::attack::armour(
            self.profile,
            self.carried?,
            catalog?,
            self.load(),
            battle.is_some_and(|battle| battle.defends(actor)),
        ))
    }

    /// The member's load, none for those who carry nothing.
    pub fn load(&self) -> crate::game::characters::Load {
        self.burden.map_or_else(Default::default, |burden| burden.load)
    }
}
