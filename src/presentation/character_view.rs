//! Named read-only view of the components needed by character summaries.
use crate::game::{
    actors::{Armour, Attributes, Resistances},
    characters::Vitals,
    items::{Burden, Items},
};
use bevy::ecs::query::QueryData;
use bevy::prelude::Entity;

#[derive(QueryData)]
pub struct CharacterView {
    pub entity: Entity,
    pub profile: crate::game::characters::PaperReadOnly,
    pub vitals: &'static Vitals,
    pub conditions: &'static crate::game::characters::Conditions,
    pub attributes: &'static Attributes,
    pub skills: &'static crate::game::actors::Skills,
    pub armour: &'static Armour,
    pub resistances: &'static Resistances,
    pub burden: &'static Burden,
    pub nature: &'static crate::game::actors::Nature,
    /// The member's action this round, while in a fight.
    pub ordered: Option<&'static crate::game::combat::Ordered>,
}

impl CharacterViewItem<'_, '_> {
    /// The member's armour as it stands, braced while they defend.
    pub fn armour(&self) -> crate::game::actors::arms::Armour {
        let braced = match self.ordered {
            Some(ordered) if ordered.defends() => self.armour.braced.0,
            _ => 0,
        };
        crate::game::actors::arms::Armour {
            total: self.armour.total + braced,
            by_location: enum_map::EnumMap::from_fn(|part| {
                self.armour.at[enum_map::Enum::into_usize(part)] + braced
            }),
            speed: 0,
            shield: 0,
        }
    }

    /// The member's resistances as they stand, beside what their sheet
    /// alone gives.
    pub fn resistances(
        &self,
        held: &Items,
    ) -> enum_map::EnumMap<
        crate::content::characters::Realm,
        crate::game::characters::resistance::Resistance,
    > {
        let sheet = crate::game::characters::resistance::resistances(
            (self.nature, Some(self.profile.trained())),
            self.attributes,
            self.skills,
            |slot| held.protection(self.entity, slot),
        );
        enum_map::EnumMap::from_fn(|realm| crate::game::characters::resistance::Resistance {
            base: sheet[realm].base,
            total: self.resistances.realms[realm],
        })
    }

    /// The member's load.
    pub fn load(&self) -> crate::game::characters::Load {
        self.burden.load
    }
}
