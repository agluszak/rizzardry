//! Named read-only view of the components needed by character summaries.
use crate::game::{
    actors::{Armour, Attributes, Resistances},
    characters::{Character, Vitals},
    inventory::{Burden, Carried},
};
use bevy::ecs::query::QueryData;

#[derive(QueryData)]
pub struct CharacterView {
    pub profile: &'static Character,
    pub vitals: &'static Vitals,
    pub attributes: &'static Attributes,
    pub armour: &'static Armour,
    pub resistances: &'static Resistances,
    pub carried: &'static Carried,
    pub burden: &'static Burden,
    /// The member's action this round, while in a fight.
    pub ordered: Option<&'static crate::game::combat::Ordered>,
}

impl CharacterViewItem<'_, '_> {
    /// The member's armour as it stands, braced while they defend.
    pub fn armour(&self) -> crate::game::combat::attack::Armour {
        let braced = match self.ordered {
            Some(ordered) if ordered.defends() => self.armour.braced.0,
            _ => 0,
        };
        crate::game::combat::attack::Armour {
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
        catalog: &crate::content::items::ItemCatalog,
    ) -> enum_map::EnumMap<
        crate::content::characters::Realm,
        crate::game::characters::resistance::Resistance,
    > {
        let sheet = crate::game::characters::resistance::resistances(
            self.profile,
            self.attributes,
            self.carried,
            catalog,
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
