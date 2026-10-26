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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::items::{Dice, ItemDefinition, ItemId};
    use crate::content::species::{Drop, Species, Treasure};
    use crate::game::world::items::WorldItem;
    use crate::game::world::population::ScaledBody;
    use bevy::ecs::system::RunSystemOnce;

    fn scene() -> (World, Entity, Entity) {
        let mut world = World::new();
        let mut items = Assets::<ItemCatalog>::default();
        let items_handle = items.add(ItemCatalog(vec![ItemDefinition {
            id: ItemId(0),
            name: "drop".into(),
            ..default()
        }]));
        let mut species = Assets::<MonsterCatalog>::default();
        let species_handle = species.add(MonsterCatalog(vec![Species {
            experience: 25,
            gold: Dice { base: 7, ..default() },
            treasure: vec![Treasure {
                drop: Drop::Item(ItemId(0)),
                chance: 100,
                dice: Dice { base: 2, ..default() },
            }],
            ..default()
        }]));
        let mut tables = Assets::<LootTables>::default();
        let loot = tables.add(LootTables::default());
        world.insert_resource(GameData { items: items_handle, species: species_handle, loot });
        world.insert_resource(items);
        world.insert_resource(species);
        world.insert_resource(tables);
        world.init_resource::<PartyInventory>();
        world.init_resource::<ItemIds>();
        world.insert_resource(GameRng::seeded(17));
        let level = world
            .spawn(Level {
                path: "damage-test".into(),
                geometry: default(),
                navmeshes: vec![],
                sky: None,
                script: default(),
                messages: None,
            })
            .id();
        let monster = world
            .spawn((
                Monster {
                    location: 1,
                    species: 0,
                    hp: 10,
                    max_hp: 10,
                    stamina: 0,
                    max_stamina: 0,
                    conditions: vec![],
                    body: ScaledBody { scale: 1.0, radius: 0.5, eye_height: 1.0, speed: 0.0 },
                },
                GlobalTransform::from_translation(Vec3::new(3.0, 4.0, 5.0)),
                ChildOf(level),
            ))
            .id();
        (world, level, monster)
    }

    fn hit(world: &mut World, target: Entity, amount: u32) -> Result<DamageOutcome, DamageRefusal> {
        world
            .run_system_once(move |mut damage: MonsterDamage, mut rng: ResMut<GameRng>| {
                damage.apply(target, amount, 1, &mut rng)
            })
            .unwrap()
    }

    #[test]
    fn damage_without_a_battle_settles_death_and_preserves_all_owned_drops_once() {
        let (mut world, level, monster) = scene();
        let before = world.resource::<GameRng>().clone();
        assert_eq!(hit(&mut world, monster, 3), Ok(DamageOutcome { damage: 3, ..default() }));
        assert_eq!(world.get::<Monster>(monster).unwrap().hp, 7);
        assert_eq!(*world.resource::<GameRng>(), before);
        assert_eq!(
            hit(&mut world, monster, 99),
            Ok(DamageOutcome { damage: 7, died: true, experience: 25 })
        );
        assert_eq!(world.resource::<PartyInventory>().gold, 7);
        // No bag definition: both owned items still appear, under the actual
        // target's level rather than whichever level a caller has selected.
        let drops: Vec<_> = world
            .query::<(&WorldItem, &ChildOf, &Transform)>()
            .iter(&world)
            .map(|(item, owner, at)| (item.item.id, owner.parent(), at.translation))
            .collect();
        assert_eq!(drops.len(), 2);
        assert_ne!(drops[0].0, drops[1].0);
        assert!(drops.iter().all(|(_, owner, at)| *owner == level && at.x == 3.0 && at.z == 5.0));
        let after = world.resource::<GameRng>().clone();
        assert_eq!(hit(&mut world, monster, 100), Ok(DamageOutcome::default()));
        assert_eq!(*world.resource::<GameRng>(), after);
        assert_eq!(world.resource::<PartyInventory>().gold, 7);
        assert_eq!(world.query::<&WorldItem>().iter(&world).count(), 2);
    }

    #[test]
    fn invalid_or_unsupported_damage_cannot_partially_mutate_the_target_or_rng() {
        let (mut world, level, monster) = scene();
        let before = world.resource::<GameRng>().clone();
        world.entity_mut(level).insert(Departing);
        assert_eq!(hit(&mut world, monster, 10), Err(DamageRefusal::InactiveLevel));
        world.entity_mut(level).remove::<Departing>();
        let species = world.resource::<GameData>().species.clone();
        world.resource_mut::<Assets<MonsterCatalog>>().get_mut(&species).unwrap().0[0].npc = true;
        assert_eq!(hit(&mut world, monster, 10), Err(DamageRefusal::UnsupportedNpcDeath));
        world.remove_resource::<GameData>();
        assert_eq!(hit(&mut world, monster, 1), Err(DamageRefusal::MissingContent));
        assert_eq!(world.get::<Monster>(monster).unwrap().hp, 10);
        assert_eq!(*world.resource::<GameRng>(), before);
        assert_eq!(world.resource::<PartyInventory>().gold, 0);
        assert_eq!(world.query::<&WorldItem>().iter(&world).count(), 0);
    }
}
