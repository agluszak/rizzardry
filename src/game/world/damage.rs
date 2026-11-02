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
use crate::game::characters::{Condition, Vitals};
use crate::game::inventory::{ItemIds, PartyInventory, make_item};
use crate::game::rng::GameRng;

/// What [`MonsterDamage`] lets others read of a monster.
pub type MonsterQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Monster,
        ActorPose,
        Option<&'static Walker>,
        &'static LevelOwned,
        Entity,
        &'static mut Vitals,
    ),
    Without<crate::game::characters::Character>,
>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DamageOutcome {
    pub damage: u32,
    pub died: bool,
    /// A battle session or another credited cause decides how to award this.
    pub experience: u32,
}

/// Monsters in play, changed only through its operations. Hurting a corpse
/// does nothing.
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
    commands: Commands<'w, 's>,
    content: If<Res<'w, crate::game::data::Catalogs>>,
    factions: Res<'w, crate::game::factions::Factions>,
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
        let turncoat = self
            .monsters
            .get(entity)
            .is_ok_and(|(.., vitals)| vitals.conditions.has(Condition::Turncoat));
        super::hostility::own_stance(self.group_stance(entity, now), turncoat)
    }

    /// How a monster's group stands now ([`super::hostility::stance`]); one
    /// outside any group by its species' default.
    pub fn group_stance(&self, entity: Entity, now: u64) -> super::hostility::Stance {
        use super::hostility;
        let (monster, ..) = self.monsters.get(entity).unwrap();
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
        let (body, .., vitals) = self.monsters.get(entity).unwrap();
        if vitals.is_dead() || vitals.conditions.has(Condition::Turncoat) {
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

    pub fn restore_magic(&mut self, entity: Entity, amount: u32) {
        let (mut body, ..) = self.monsters.get_mut(entity).unwrap();
        body.spell_points += amount as i32;
    }
    /// The monsters, to look at.
    pub fn monsters(&self) -> &MonsterQuery<'w, 's> {
        &self.monsters
    }

    /// A monster's body, which magic and blows change.
    pub fn vitals_mut(&mut self, monster: Entity) -> Mut<'_, Vitals> {
        self.monsters.get_mut(monster).unwrap().5
    }

    /// Drain the current effective spell budget; only the change belongs to
    /// the actor. Casting and draining read the same species-plus-history rule.
    pub fn drain_magic(&mut self, entity: Entity, amount: u32) -> u32 {
        let (mut monster, ..) = self.monsters.get_mut(entity).unwrap();
        let kind = &self.content.species[monster.species];
        let lost = monster.spell_budget(kind).min(amount);
        monster.spell_points -= lost as i32;
        lost
    }

    pub fn active(&self, target: Entity) -> bool {
        self.monsters.get(target).is_ok_and(|(_, _, _, owner, ..)| self.levels.contains(owner.0))
    }

    /// Whether magic may take a monster as a target or victim: in play,
    /// alive, and of a species not kept out of targeting (retail's live
    /// targetable monsters, `CollectConeMonsterTargets`,
    /// `CollectMonstersWithinRadius`).
    pub fn targetable(&self, target: Entity) -> bool {
        self.active(target)
            && self.monsters.get(target).is_ok_and(|(body, .., vitals)| {
                !vitals.is_dead() && !self.content.species[body.species].untargetable
            })
    }

    /// Hurt a monster. What its death brings about follows when it is
    /// marked [`Dead`](crate::game::characters::Dead) ([`monster_died`]).
    pub fn apply(&mut self, target: Entity, amount: u32, rng: &mut GameRng) -> DamageOutcome {
        let (monster, _, _, _, _, mut vitals) = self.monsters.get_mut(target).unwrap();
        let kind = &self.content.species[monster.species];
        if vitals.is_dead() || amount == 0 {
            return DamageOutcome::default();
        }
        let amount = vitals.enchantments.absorb(amount);
        if amount == 0 {
            return DamageOutcome::default();
        }
        let lost = amount.min(vitals.health.current);
        vitals.health.current -= lost;
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
        if !vitals.is_dead() {
            vitals.conditions.wake_on_hit(u32::from(kind.attributes.senses), rng);
            return DamageOutcome { damage: lost, ..default() };
        }
        vitals.enchantments.0.clear();
        let summoned = self.summoned.contains(target);
        DamageOutcome {
            damage: lost,
            died: true,
            experience: if summoned { 0 } else { kind.experience },
        }
    }
}

/// A monster has died: its autonomous work stops, and unless it was
/// summoned its death counts against its faction (retail
/// `RecordFactionOffense`), an NPC's body leaves the NPC dead for good, and
/// it drops its loot and pays its gold into the party's purse.
#[allow(clippy::too_many_arguments)]
pub fn monster_died(
    death: On<Add<crate::game::characters::Dead>>,
    mut commands: Commands,
    monsters: Query<(
        &Monster,
        ActorPose,
        Option<&Walker>,
        &LevelOwned,
        Has<crate::game::magic::pointcasts::SummonedBy>,
    )>,
    mut motion: Query<(
        Option<&mut AgentTarget3d>,
        Option<&mut TnuaController<WalkerMotion>>,
        Option<&mut TnuaMotor>,
        Option<&mut LinearVelocity>,
        Option<&mut Velocity3d>,
    )>,
    (bodies, mut npcs): (
        Query<&crate::game::npcs::NpcBody>,
        Query<(&mut crate::game::npcs::Npc, &crate::game::npcs::ActorId)>,
    ),
    (mut purse, roster, characters): (
        Single<&mut PartyInventory>,
        Single<&crate::game::party::roster::PartyRoster>,
        Query<&crate::game::characters::Character>,
    ),
    content: If<Res<crate::game::data::Catalogs>>,
    (mut ids, mut rng, mut factions, clock): (
        ResMut<ItemIds>,
        ResMut<GameRng>,
        ResMut<crate::game::factions::Factions>,
        Res<crate::game::time::GameClock>,
    ),
    mut faction_news: MessageWriter<crate::game::factions::FactionNews>,
) {
    let target = death.entity;
    let Ok((monster, at, walker, owner, summoned)) = monsters.get(target) else { return };
    // Stop autonomous work, not the body's geometry or vertical physics.
    let (targeting, controller, motor, velocity, navigation_velocity) =
        motion.get_mut(target).unwrap();
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
    commands.entity(target).remove_with_requires::<Crossing>();
    if summoned {
        return;
    }
    let (catalog, tables) = (&content.items, &content.loot);
    let kind = &content.species[monster.species];
    if let Some(news) = factions.record_offence(kind.faction, kind.npc.is_some(), clock.ms) {
        faction_news.write(news);
    }
    let rng = &mut *rng;
    let npc = bodies.get(target).ok().and_then(|body| npcs.get_mut(body.0).ok());
    let (found, gold) = match npc {
        Some((mut npc, id)) => {
            npc.dead = true;
            crate::game::loot::npc_loot(&content.npcs[id.npc()], tables, catalog, rng)
        }
        None => {
            let party_level = crate::game::party::average_level(
                roster.iter().map(|member| characters.get(member).unwrap().level()),
            );
            crate::game::loot::monster_loot(kind, tables, catalog, party_level, rng)
        }
    };
    purse.gold += gold;
    let items =
        found.into_iter().map(|id| make_item(&catalog[id], &mut ids, rng, false, false)).collect();
    let position = feet(at.translation(), walker);
    drop_loot(&mut commands, owner.0, position, items, catalog);
}
