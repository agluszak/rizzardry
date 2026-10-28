//! Named read-only view of the components needed by character summaries.
use crate::game::{
    characters::{Character, Vitals},
    inventory::{Burden, Carried},
};
use bevy::ecs::query::QueryData;
use bevy::prelude::Has;

#[derive(QueryData)]
pub struct CharacterView {
    pub profile: &'static Character,
    pub vitals: &'static Vitals,
    pub carried: Option<&'static Carried>,
    pub burden: Option<&'static Burden>,
    armour: Option<&'static crate::game::combat::attack::Armour>,
    defends: Has<crate::game::combat::Defends>,
    pub hand: Option<&'static crate::game::combat::attack::Hand>,
}

impl CharacterViewItem<'_, '_> {
    /// The member's armour as it stands, with the defensive action while
    /// they defend.
    pub fn armour(&self) -> Option<crate::game::combat::attack::Armour> {
        self.armour.map(|armour| armour.defending(self.defends))
    }

    /// The member's load, none for those who carry nothing.
    pub fn load(&self) -> crate::game::characters::Load {
        self.burden.map_or_else(Default::default, |burden| burden.load)
    }
}
