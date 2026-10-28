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
}

impl CharacterViewItem<'_, '_> {
    /// The member's load, none for those who carry nothing.
    pub fn load(&self) -> crate::game::characters::Load {
        self.burden.map_or_else(Default::default, |burden| burden.load)
    }
}
