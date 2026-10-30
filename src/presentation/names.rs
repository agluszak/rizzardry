//! Display names derived from current actor identity and party knowledge.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::game::characters::Character;
use crate::game::world::identify::{GroupMember, MonsterGroup};
use crate::game::world::population::Monster;

/// Used by screen notices and headless logging; it requires no UI or renderer.
#[derive(SystemParam)]
pub struct ActorNames<'w, 's> {
    characters: Query<'w, 's, &'static Character>,
    monsters: Query<'w, 's, (&'static Monster, Option<&'static GroupMember>)>,
    groups: Query<'w, 's, &'static MonsterGroup>,
    content: If<Res<'w, crate::game::data::Catalogs>>,
}

impl ActorNames<'_, '_> {
    pub fn name(&self, actor: Entity) -> Option<String> {
        if let Ok(profile) = self.characters.get(actor) {
            return Some(profile.name.clone());
        }
        let (monster, membership) = self.monsters.get(actor).ok()?;
        let catalog = &self.content.species;
        let kind = catalog.kind(monster.species)?;
        let known =
            membership.and_then(|m| self.groups.get(m.0).ok()).is_some_and(|g| g.identified);
        Some(kind.shown_name(known, 1).to_owned())
    }

    /// The words a monster goes by (its species' `name_group`).
    pub fn pronouns(&self, actor: Entity) -> Option<u8> {
        let (monster, _) = self.monsters.get(actor).ok()?;
        Some(self.content.species.kind(monster.species)?.pronouns)
    }
}
