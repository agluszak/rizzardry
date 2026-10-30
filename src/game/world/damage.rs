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
use crate::game::characters::Condition;
use crate::game::inventory::{ItemIds, PartyInventory, make_item};
use crate::game::rng::GameRng;

/// What [`MonsterDamage`] lets others read of a monster.
pub type MonsterQuery<'w, 's> = Query<
    'w,
    's,
    (&'static mut Monster, ActorPose, Option<&'static Walker>, &'static LevelOwned, Entity),
>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DamageOutcome {
    pub damage: u32,
    pub died: bool,
    /// A battle session or another credited cause decides how to award this.
    pub experience: u32,
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
    membership: Query<'w, 's, &'static super::identify::GroupMember>,
    summoned: Query<'w, 's, (), With<crate::game::magic::pointcasts::SummonedBy>>,
    lures: Query<'w, 's, &'static crate::game::magic::pointcasts::LureInfluence>,
    combatants: Query<'w, 's, (), With<crate::game::combat::Combatant>>,
    groups: Query<'w, 's, &'static mut super::identify::MonsterGroup>,
    group_members: Query<'w, 's, &'static super::identify::GroupMembers>,
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
    /// The NPCs whose world roles are among the monsters.
    npcs: (
        Query<'w, 's, &'static crate::game::npcs::NpcBody>,
        Query<'w, 's, &'static mut crate::game::npcs::Npc>,
    ),
    commands: Commands<'w, 's>,
    content: If<Res<'w, crate::game::data::Catalogs>>,
    ids: ResMut<'w, ItemIds>,
    factions: ResMut<'w, crate::game::factions::Factions>,
    faction_news: MessageWriter<'w, crate::game::factions::FactionNews>,
}

impl<'w, 's> MonsterDamage<'w, 's> {
    pub fn group(&self, entity: Entity) -> Vec<Entity> {
        use bevy::ecs::relationship::RelationshipTarget;
        self.membership
            .get(entity)
            .ok()
            .and_then(|member| self.group_members.get(member.0).ok())
            .map_or_else(|| vec![entity], |members| members.iter().collect())
    }
    pub fn leader(&self, entity: Entity) -> Entity {
        self.group(entity)
            .into_iter()
            .min_by_key(|&member| {
                self.monsters.get(member).map_or(i32::MAX, |(body, ..)| body.location)
            })
            .unwrap_or(entity)
    }
    /// A monster's own stance now ([`super::hostility::monster_own`]): its
    /// group's, swapped by its own Turncoat.
    pub fn stance(&self, entity: Entity, now: u64) -> super::hostility::Stance {
        let turncoat =
            self.monsters.get(entity).is_ok_and(|(m, ..)| m.conditions.has(Condition::Turncoat));
        super::hostility::own_stance(self.group_stance(entity, now), turncoat)
    }

    /// How a monster's group stands now ([`super::hostility::stance`]); one
    /// outside any group by its species' default.
    pub fn group_stance(&self, entity: Entity, now: u64) -> super::hostility::Stance {
        use super::hostility::{self, Stance};
        let Ok((monster, ..)) = self.monsters.get(entity) else { return Stance::Neutral };
        let kind = &self.content.species[monster.species];
        let set = self
            .membership
            .get(entity)
            .ok()
            .and_then(|member| self.groups.get(member.0).ok())
            .and_then(|group| group.set);
        hostility::stance(set, kind, &self.factions, now, self.combatants.contains(entity))
    }

    /// Commit hostility before damage/conditions inspect the target's side.
    /// The later notification can start a fight even when this hit kills it.
    pub fn provoke(&mut self, entity: Entity, now: u64) -> bool {
        use super::hostility::{self, SetStance, Stance};
        let Ok((body, ..)) = self.monsters.get(entity) else { return false };
        if body.is_dead() || body.conditions.has(Condition::Turncoat) {
            return false;
        }
        let kind = &self.content.species[body.species];
        if let Ok(member) = self.membership.get(entity)
            && let Ok(mut group) = self.groups.get_mut(member.0)
            && hostility::stance(group.set, kind, &self.factions, now, true) != Stance::Hostile
        {
            group.set = Some(SetStance { stance: Stance::Hostile, at: now });
        }
        true
    }

    pub fn restore_health(&mut self, entity: Entity, amount: u32) -> u32 {
        let Ok((mut body, ..)) = self.monsters.get_mut(entity) else { return 0 };
        if body.is_dead() {
            return 0;
        }
        let before = body.hp;
        body.hp = body.hp.saturating_add(amount).min(body.max_hp);
        body.hp.saturating_sub(before)
    }
    /// Restoring stamina wakes exhaustion, but leaves a timed knockout alone.
    pub fn restore_stamina(&mut self, entity: Entity, amount: u32) -> u32 {
        let Ok((mut body, ..)) = self.monsters.get_mut(entity) else { return 0 };
        if body.is_dead() {
            return 0;
        }
        let body = &mut *body;
        crate::game::characters::rest(
            &mut body.stamina,
            &mut body.conditions,
            amount,
            body.max_stamina,
        )
    }
    pub fn restore_magic(&mut self, entity: Entity, amount: u32) {
        if let Ok((mut body, ..)) = self.monsters.get_mut(entity) {
            body.spell_points =
                body.spell_points.saturating_add(amount.min(i32::MAX as u32) as i32);
        }
    }
    /// The monsters, to look at.
    pub fn monsters(&self) -> &MonsterQuery<'w, 's> {
        &self.monsters
    }

    /// A monster's conditions, to inflict, cure or age.
    pub fn conditions_mut(
        &mut self,
        monster: Entity,
    ) -> Option<Mut<'_, crate::game::characters::Conditions>> {
        Some(self.monster_mut(monster)?.map_unchanged(|monster| &mut monster.conditions))
    }

    /// A monster's body, for what changes its conditions and enchantments
    /// together.
    pub fn monster_mut(&mut self, monster: Entity) -> Option<Mut<'_, Monster>> {
        Some(self.monsters.get_mut(monster).ok()?.0)
    }

    /// Tire a monster (retail `FatigueMonster`); spent, it drops
    /// unconscious. Returns the stamina taken.
    pub fn fatigue(&mut self, monster: Entity, amount: u32) -> u32 {
        let Ok((mut monster, ..)) = self.monsters.get_mut(monster) else { return 0 };
        if monster.is_dead() {
            return 0;
        }
        let monster = &mut *monster;
        crate::game::characters::tire(&mut monster.stamina, &mut monster.conditions, amount)
    }

    /// Drain the current effective spell budget; only the change belongs to
    /// the actor. Casting and draining read the same species-plus-history rule.
    pub fn drain_magic(&mut self, entity: Entity, amount: u32) -> u32 {
        let Ok((mut monster, ..)) = self.monsters.get_mut(entity) else { return 0 };
        let kind = &self.content.species[monster.species];
        let lost = monster.spell_budget(kind).min(amount);
        monster.spell_points = monster.spell_points.saturating_sub(lost as i32);
        lost
    }

    pub fn enchantments_mut(
        &mut self,
        entity: Entity,
    ) -> Option<Mut<'_, crate::game::magic::enchantments::Enchantments>> {
        Some(self.monster_mut(entity)?.map_unchanged(|m| &mut m.enchantments))
    }

    pub fn active(&self, target: Entity) -> bool {
        self.monsters.get(target).is_ok_and(|(_, _, _, owner, _)| self.levels.contains(owner.0))
    }

    /// Whether magic may take a monster as a target or victim: in play,
    /// alive, and of a species not kept out of targeting (retail's live
    /// targetable monsters, `CollectConeMonsterTargets`,
    /// `CollectMonstersWithinRadius`).
    pub fn targetable(&self, target: Entity) -> bool {
        self.active(target)
            && self.monsters.get(target).is_ok_and(|(body, ..)| {
                !body.is_dead() && !self.content.species[body.species].untargetable
            })
    }

    /// Hurt a monster at game time `now`; a kill drops its loot, pays its
    /// gold into the party's purse and counts against its faction then.
    pub fn apply(
        &mut self,
        target: Entity,
        amount: u32,
        (party_level, now): (u32, u64),
        purse: &mut PartyInventory,
        rng: &mut GameRng,
    ) -> DamageOutcome {
        let (mut monster, at, walker, owner, _) = self.monsters.get_mut(target).unwrap();
        let level = owner.0;
        let catalogs = &self.content;
        let (catalog, tables) = (&catalogs.items, &catalogs.loot);
        let kind = &catalogs.species[monster.species];
        let (ids, factions, faction_news) =
            (&mut *self.ids, &mut *self.factions, &mut self.faction_news);
        if monster.is_dead() || amount == 0 {
            return DamageOutcome::default();
        }
        let amount = monster.enchantments.absorb(amount);
        if amount == 0 {
            return DamageOutcome::default();
        }
        let position = feet(at.translation(), walker);
        let lost = amount.min(monster.hp);
        monster.hp -= lost;
        if lost > 0
            && let Ok(influence) = self.lures.get(target)
        {
            let influence = crate::game::magic::pointcasts::LureInfluence {
                response: crate::game::magic::pointcasts::LureResponse::Resisted,
                ..*influence
            };
            self.commands.entity(target).insert(influence);
            if !self.combatants.contains(target)
                && let Ok((Some(mut goal), ..)) = self.motion.get_mut(target)
            {
                *goal = AgentTarget3d::None;
            }
        }
        if !monster.is_dead() {
            monster.conditions.wake_on_hit(u32::from(kind.attributes.senses), rng);
            return DamageOutcome { damage: lost, ..default() };
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
        if self.summoned.contains(target) {
            return DamageOutcome { damage: lost, died: true, experience: 0 };
        }
        // The kill counts against its faction (retail `RecordFactionOffense`).
        if let Some(news) = factions.record_offence(kind.faction, kind.npc.is_some(), now) {
            faction_news.write(news);
        }
        let npc = self.npcs.0.get(target).ok().and_then(|body| self.npcs.1.get_mut(body.0).ok());
        let (found, gold) = match npc {
            Some(mut npc) => {
                npc.dead = true;
                npc.possessions.give_up(catalog, rng)
            }
            None => crate::game::loot::monster_loot(kind, tables, catalog, party_level, rng),
        };
        if gold > 0 {
            purse.gold += gold;
        }
        let items =
            found.into_iter().map(|id| make_item(&catalog[id], ids, rng, false, false)).collect();
        drop_loot(&mut self.commands, level, position, items, catalog);
        DamageOutcome { damage: lost, died: true, experience: kind.experience }
    }
}
