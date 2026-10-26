//! Damage and mandatory monster-death consequences, independent of the cause.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::items::drop_loot;
use super::level::{Departing, Level};
use super::navigation::{Walker, feet};
use super::population::Monster;
use crate::content::items::{ItemCatalog, LootTables};
use crate::content::species::MonsterCatalog;
use crate::game::data::GameData;
use crate::game::inventory::{ItemIds, PartyInventory, make_item};
use crate::game::rng::GameRng;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DamageOutcome {
    pub damage: u32,
    pub died: bool,
    /// A battle session or another credited cause decides how to award this.
    pub experience: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageRefusal {
    MissingTarget,
    InactiveLevel,
    MissingContent,
    /// NPC death uses its own retained-character and loot rules, not monster tables.
    UnsupportedNpcDeath,
}

/// A synchronous operation over the actual target and its owning level. It
/// creates loot and transfers gold before returning a death outcome. Calling
/// it again on the corpse produces no second reward or drop.
#[derive(SystemParam)]
pub struct MonsterDamage<'w, 's> {
    pub monsters: Query<
        'w,
        's,
        (&'static mut Monster, &'static GlobalTransform, Option<&'static Walker>, &'static ChildOf),
    >,
    levels: Query<'w, 's, (), (With<Level>, Without<Departing>)>,
    commands: Commands<'w, 's>,
    data: Option<Res<'w, GameData>>,
    catalogs: Res<'w, Assets<ItemCatalog>>,
    species: Res<'w, Assets<MonsterCatalog>>,
    tables: Res<'w, Assets<LootTables>>,
    inventory: ResMut<'w, PartyInventory>,
    ids: ResMut<'w, ItemIds>,
}

impl MonsterDamage<'_, '_> {
    pub fn ready(&self) -> bool {
        self.data.as_ref().is_some_and(|d| {
            self.catalogs.contains(&d.items)
                && self.species.contains(&d.species)
                && self.tables.contains(&d.loot)
        })
    }

    pub fn active(&self, target: Entity) -> bool {
        self.monsters.get(target).is_ok_and(|(_, _, _, owner)| self.levels.contains(owner.parent()))
    }

    pub fn apply(
        &mut self,
        target: Entity,
        amount: u32,
        party_level: u32,
        rng: &mut GameRng,
    ) -> Result<DamageOutcome, DamageRefusal> {
        let (monster, at, walker, owner) =
            self.monsters.get(target).map_err(|_| DamageRefusal::MissingTarget)?;
        let level = owner.parent();
        if !self.levels.contains(level) {
            return Err(DamageRefusal::InactiveLevel);
        }
        let data = self.data.as_ref().ok_or(DamageRefusal::MissingContent)?;
        let (Some(catalog), Some(species), Some(tables)) = (
            self.catalogs.get(&data.items),
            self.species.get(&data.species),
            self.tables.get(&data.loot),
        ) else {
            return Err(DamageRefusal::MissingContent);
        };
        let kind = species.0.get(monster.species as usize).ok_or(DamageRefusal::MissingContent)?;
        if monster.is_dead() || amount == 0 {
            return Ok(DamageOutcome::default());
        }
        if amount >= monster.hp && kind.npc {
            return Err(DamageRefusal::UnsupportedNpcDeath);
        }
        let position = feet(at.translation(), walker);
        let lost = amount.min(monster.hp);
        let (mut monster, ..) = self.monsters.get_mut(target).unwrap();
        monster.hp -= lost;
        if !monster.is_dead() {
            return Ok(DamageOutcome { damage: lost, ..default() });
        }
        let (found, gold) =
            crate::game::loot::monster_loot(kind, tables, catalog, party_level, rng);
        if gold > 0 {
            self.inventory.gold += gold;
        }
        let items = found
            .into_iter()
            .filter_map(|id| catalog.get(id))
            .map(|definition| make_item(definition, &mut self.ids, rng, false, false))
            .collect();
        drop_loot(&mut self.commands, level, position, items, catalog, &mut self.ids, rng);
        Ok(DamageOutcome { damage: lost, died: true, experience: kind.experience })
    }
}
