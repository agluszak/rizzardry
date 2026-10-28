//! Damage and mandatory monster-death consequences, independent of the cause.

use super::navigation::{WalkerMotion, links::Crossing};
use crate::game::world::ActorPose;
use crate::game::world::level::LevelOwned;
use avian3d::prelude::LinearVelocity;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_landmass::{AgentTarget3d, Velocity3d};
use bevy_tnua::{TnuaController, TnuaMotor};

use super::items::drop_loot;
use super::level::Level;
use super::navigation::{Walker, feet};
use super::population::Monster;
use crate::game::characters::{Condition, Lasting};
use crate::game::inventory::{ItemIds, PartyInventory, make_item};
use crate::game::rng::GameRng;

/// What [`MonsterDamage`] lets others read of a monster.
pub type MonsterQuery<'w, 's> =
    Query<'w, 's, (&'static mut Monster, ActorPose, Option<&'static Walker>, &'static LevelOwned)>;

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

/// Monsters in play, changed only through its operations. [`apply`] is
/// synchronous over the actual target and its owning level: it creates loot
/// and transfers gold before returning a death outcome. Calling it again on
/// the corpse produces no second reward or drop.
///
/// [`apply`]: MonsterDamage::apply
#[derive(SystemParam)]
pub struct MonsterDamage<'w, 's> {
    monsters: MonsterQuery<'w, 's>,
    motion: Query<
        'w,
        's,
        (
            Option<&'static mut AgentTarget3d>,
            Option<&'static mut TnuaController<WalkerMotion>>,
            Option<&'static mut TnuaMotor>,
            Option<&'static mut LinearVelocity>,
            Option<&'static mut Velocity3d>,
        ),
    >,
    levels: Query<'w, 's, (), With<Level>>,
    commands: Commands<'w, 's>,
    content: crate::game::data::Content<'w>,
    ids: ResMut<'w, ItemIds>,
    factions: ResMut<'w, crate::game::factions::Factions>,
}

impl<'w, 's> MonsterDamage<'w, 's> {
    pub fn ready(&self) -> bool {
        self.content.items().is_some()
            && self.content.species().is_some()
            && self.content.loot().is_some()
    }

    /// The monsters, to look at.
    pub fn monsters(&self) -> &MonsterQuery<'w, 's> {
        &self.monsters
    }

    /// Current live actors intersecting a radius, in authored location order.
    pub fn nearby(&mut self, centre: Vec3, radius: f32) -> Vec<Entity> {
        let mut lens = self
            .monsters
            .transmute_lens::<(Entity, &super::population::Monster, ActorPose, &LevelOwned)>();
        let query = lens.query();
        let mut found: Vec<_> = query
            .iter()
            .filter(|(_, body, at, owner)| {
                self.levels.contains(owner.0)
                    && !body.is_dead()
                    && at.translation().distance(centre) - body.body.radius <= radius
            })
            .map(|(entity, body, _, _)| (body.location, entity))
            .collect();
        found.sort_unstable();
        found.into_iter().map(|(_, entity)| entity).collect()
    }

    /// A monster's conditions, to inflict, cure or age.
    pub fn conditions_mut(
        &mut self,
        monster: Entity,
    ) -> Option<Mut<'_, Vec<(Condition, Lasting)>>> {
        let (monster, ..) = self.monsters.get_mut(monster).ok()?;
        Some(monster.map_unchanged(|monster| &mut monster.conditions))
    }

    /// Tire a monster (retail `FatigueMonster`); spent, it drops
    /// unconscious. Returns the stamina taken.
    pub fn fatigue(&mut self, monster: Entity, amount: u32) -> u32 {
        let Ok((mut monster, ..)) = self.monsters.get_mut(monster) else { return 0 };
        if monster.is_dead() || monster.stamina == 0 {
            return 0;
        }
        let taken = amount.min(monster.stamina);
        monster.stamina -= taken;
        if monster.stamina == 0 {
            crate::game::characters::inflict(
                &mut monster.conditions,
                Condition::Unconscious,
                Lasting::Indefinite,
            );
        }
        taken
    }

    /// Spell points are live monster history; the species supplies only the
    /// initial budget. Draining never rerolls or reconstructs that budget.
    pub fn drain_magic(&mut self, entity: Entity, amount: u32) -> u32 {
        let Ok((mut monster, ..)) = self.monsters.get_mut(entity) else { return 0 };
        let lost = monster.mana.min(amount);
        monster.mana -= lost;
        lost
    }

    pub fn enchantments_mut(
        &mut self,
        entity: Entity,
    ) -> Option<Mut<'_, crate::game::magic::enchantments::Enchantments>> {
        let (monster, ..) = self.monsters.get_mut(entity).ok()?;
        Some(monster.map_unchanged(|m| &mut m.enchantments))
    }

    pub fn active(&self, target: Entity) -> bool {
        self.monsters.get(target).is_ok_and(|(_, _, _, owner)| self.levels.contains(owner.0))
    }

    /// Hurt a monster; a kill drops its loot and pays its gold into the
    /// party's purse.
    pub fn apply(
        &mut self,
        target: Entity,
        amount: u32,
        party_level: u32,
        purse: &mut PartyInventory,
        rng: &mut GameRng,
    ) -> Result<DamageOutcome, DamageRefusal> {
        let (mut monster, at, walker, owner) =
            self.monsters.get_mut(target).map_err(|_| DamageRefusal::MissingTarget)?;
        let level = owner.0;
        if !self.levels.contains(level) {
            return Err(DamageRefusal::InactiveLevel);
        }
        let (Some(catalog), Some(species), Some(tables)) =
            (self.content.items(), self.content.species(), self.content.loot())
        else {
            return Err(DamageRefusal::MissingContent);
        };
        let kind = species.kind(monster.species).ok_or(DamageRefusal::MissingContent)?;
        if monster.is_dead() || amount == 0 {
            return Ok(DamageOutcome::default());
        }
        // A Guardian Angel's absorption counts before the NPC-death refusal,
        // and is spent only once the hit is allowed.
        let mut enchantments = monster.enchantments.clone();
        let amount = enchantments.absorb(amount);
        if amount >= monster.hp && kind.npc.is_some() {
            return Err(DamageRefusal::UnsupportedNpcDeath);
        }
        monster.enchantments = enchantments;
        if amount == 0 {
            return Ok(DamageOutcome::default());
        }
        let position = feet(at.translation(), walker);
        let lost = amount.min(monster.hp);
        monster.hp -= lost;
        self.commands.entity(target).insert(super::population::Provoked);
        if !monster.is_dead() {
            crate::game::characters::wake_on_hit(
                &mut monster.conditions,
                u32::from(kind.attributes.senses),
                rng,
            );
            return Ok(DamageOutcome { damage: lost, ..default() });
        }
        monster.enchantments.0.clear();
        // Stop autonomous work, not the body's geometry or vertical physics.
        // Structural removal is applied with this operation's loot commands,
        // before any following movement consumer can run.
        let (targeting, controller, motor, velocity, navigation_velocity) =
            self.motion.get_mut(target).unwrap();
        if let Some(mut targeting) = targeting {
            *targeting = AgentTarget3d::None;
        }
        if let Some(mut controller) = controller {
            controller.basis = default();
        }
        if let Some(mut motor) = motor {
            *motor = default();
        }
        if let Some(mut velocity) = velocity {
            velocity.0 = Vec3::Y * velocity.y;
        }
        if let Some(mut velocity) = navigation_velocity {
            velocity.velocity = Vec3::ZERO;
        }
        self.commands.entity(target).remove_with_requires::<Crossing>();
        // The kill counts against its faction (retail `RecordFactionOffense`).
        self.factions.record_offence(kind.faction, kind.npc.is_some());
        let (found, gold) =
            crate::game::loot::monster_loot(kind, tables, catalog, party_level, rng);
        if gold > 0 {
            purse.gold += gold;
        }
        let items = found
            .into_iter()
            .filter_map(|id| catalog.get(id))
            .map(|definition| make_item(definition, &mut self.ids, rng, false, false))
            .collect();
        drop_loot(&mut self.commands, level, position, items, catalog);
        Ok(DamageOutcome { damage: lost, died: true, experience: kind.experience })
    }
}
