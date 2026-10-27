//! Named read-only view of the components needed by character summaries.
use crate::game::{
    characters::{Character, Vitals},
    inventory::Carried,
};
use bevy::{ecs::query::QueryData, prelude::*};

#[derive(QueryData)]
pub struct CharacterView {
    pub profile: Ref<'static, Character>,
    pub vitals: Ref<'static, Vitals>,
    pub carried: Option<Ref<'static, Carried>>,
}
impl CharacterViewItem<'_, '_> {
    /// Optional-component removals are supplied by the caller's removal reader.
    pub fn is_changed(&self) -> bool {
        self.profile.is_changed()
            || self.vitals.is_changed()
            || self.carried.as_ref().is_some_and(|c| c.is_changed())
    }
}
