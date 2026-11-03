//! Acts own their deeds and independently delayed impacts. Once an impact
//! arrives, its immutable definition is evaluated as one immediate operation.
//! Consequences settle before another swing or actor advances; completed acts
//! produce drains, reflections, practice, recasts, and finally reports.

pub mod defences;
pub mod definitions;
pub mod effects;
pub mod reach;
mod resolve;
pub mod specials;
pub mod spells;

use bevy::prelude::*;

use crate::content::items::EquipClass;
use crate::content::spells::SpellId;
use crate::game::characters::Condition;
use crate::game::magic::casting::{MagicNews, Report};

/// What an act did to one victim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Improved {
        victim: Entity,
        skill: crate::content::characters::Skill,
    },
    Purified {
        victim: Entity,
    },
    Enchanted {
        victim: Entity,
        spell: SpellId,
    },
    Healed {
        victim: Entity,
        amount: u32,
    },
    Refreshed {
        victim: Entity,
        amount: u32,
    },
    Replenished {
        victim: Entity,
        amount: u32,
    },
    Damaged {
        victim: Entity,
        amount: u32,
    },
    Inflicted {
        victim: Entity,
        condition: Condition,
    },
    Cured {
        victim: Entity,
        condition: Condition,
    },
    Raised {
        victim: Entity,
    },
    GoldLost {
        amount: u32,
    },
    Destroyed {
        class: EquipClass,
        amount: u32,
    },
    Drained {
        victim: Entity,
        amount: u32,
    },
    /// Stamina a blow drained.
    Tired {
        victim: Entity,
        amount: u32,
    },
    Resisted {
        victim: Entity,
    },
    Reflected {
        victim: Entity,
    },
    Nothing {
        victim: Entity,
    },
}

/// Whether anything an act did counts as affecting a victim, which is what
/// practice needs (retail `SpellAffectedTarget`).
pub fn affected(effects: &[Effect]) -> bool {
    effects.iter().any(|e| !matches!(e, Effect::Nothing { .. } | Effect::Improved { .. }))
}

/// The act is made by this actor; the actor gone, so is the act.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = Acts)]
pub struct By(pub Entity);

/// An actor's acts under way, maintained by Bevy from [`By`].
#[derive(Component, Debug)]
#[relationship_target(relationship = By, linked_spawn)]
pub struct Acts(Vec<Entity>);

/// The impact lands on this body.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = Incoming)]
pub struct Strikes(pub Entity);

/// Impacts on a body, gone with it.
#[derive(Component, Debug)]
#[relationship_target(relationship = Strikes, linked_spawn)]
pub struct Incoming(Vec<Entity>);

/// The impact belongs to this act.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = DeedHits)]
pub struct HitOf(pub Entity);

/// An act's outstanding impacts, gone with it.
#[derive(Component, Debug)]
#[relationship_target(relationship = HitOf, linked_spawn)]
pub struct DeedHits(Vec<Entity>);

/// What one act did, gathered from its effects for its news.
#[derive(Component, Debug)]
pub struct Deed {
    pub act: Act,
    pub effects: Vec<Effect>,
}

/// Which act a deed is, for its news.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// A member's cast, reported with what landed at once.
    Cast(Report),
    /// A missile reaching its victims.
    Missile { caster: Entity, spell: SpellId },
    /// A spell with no caster: a cloud's pulse, a trap.
    Triggered { spell: SpellId },
    /// Something in a fight whose effects the fight reports: a monster's
    /// cast, a breath.
    Fight,
    /// A blow of an attack, which the attack tallies into its own report.
    Swing,
    /// A blow reported on its own: one gone astray, or Razor Cloak's answer.
    Blow { by: Entity, at: Entity },
}

impl Deed {
    pub fn new(act: Act) -> Self {
        Self { act, effects: Vec::new() }
    }
}

/// Where acts land and are reported: [`run_hits`]. It runs wherever acts
/// are made: on the frame for magic outside a fight, on the fixed step
/// within one.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Resolution;

/// The stages that settle effects, in order, then what follows once an
/// act's effects have all landed, then the acts' reports.
#[derive(bevy::ecs::schedule::ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
pub struct Hits;

/// Extension points for the owner of a completed act. Resolution finishes
/// first; continuations consume their work before reporting destroys the deed.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HitPhase {
    Effects,
    Completed,
    Report,
}

/// Drain immediate impacts and the finite follow-ups they release. Each spell
/// completion is consumed once; reflected spells cannot reflect again and
/// explosions do not burst. Only missile flight may survive this boundary.
pub fn run_hits(world: &mut World) {
    if world.query_filtered::<(), With<Deed>>().iter(world).next().is_none() {
        return;
    }
    world.run_schedule(Hits);
    while world
        .query_filtered::<(), (With<HitOf>, Without<effects::InFlight>)>()
        .iter(world)
        .next()
        .is_some()
    {
        world.run_schedule(Hits);
    }
}

pub struct HitsPlugin;

impl Plugin for HitsPlugin {
    fn build(&self, app: &mut App) {
        use definitions::{Definitions, define};
        use resolve::party_reaches;
        app.add_systems(
            Capabilities,
            (
                crate::game::world::hostility::judge_sides,
                crate::game::world::footing::place_bodies,
                defences::judge_defences,
            )
                .chain(),
        )
        .add_systems(
            AfterEffect,
            (
                crate::game::characters::sync_member_maxima,
                crate::game::characters::cheat_death,
                crate::game::characters::settle_deaths,
            )
                .chain(),
        )
        .configure_sets(Hits, (HitPhase::Effects, HitPhase::Completed, HitPhase::Report).chain())
        .add_systems(
            PreUpdate,
            define
                .run_if(resource_exists::<crate::game::data::Catalogs>)
                .run_if(not(resource_exists::<Definitions>)),
        )
        .add_systems(
            Hits,
            (
                refresh_capabilities,
                spells::strength_of_owners,
                party_reaches,
                resolve::impacts,
                refresh_capabilities,
                spells::drain_back,
                spells::burst_blood,
                spells::reflect_spells,
            )
                .chain()
                .in_set(HitPhase::Effects),
        )
        .add_systems(Hits, report_deeds.in_set(HitPhase::Report))
        .add_systems(
            Update,
            run_hits
                .in_set(Resolution)
                .run_if(not(any_with_component::<crate::game::combat::Battle>)),
        )
        // Combat owns fixed-step settlement, including missile flight.
        .configure_sets(Update, Resolution.after(crate::game::GameplaySet::Combat));
    }
}

/// Turn each deed whose reaches are all done into news, and let it go; a
/// swing is left to its attack.
pub fn report_deeds(
    mut commands: Commands,
    deeds: Query<(Entity, &Deed), Without<DeedHits>>,
    mut battle: Option<Single<&mut crate::game::combat::Round>>,
    mut magic_news: MessageWriter<MagicNews>,
    mut combat_news: MessageWriter<crate::game::combat::CombatNews>,
) {
    for (entity, deed) in &deeds {
        if deed.act == Act::Swing {
            continue;
        }
        commands.entity(entity).despawn();
        // A cloud's pulse keeps the fight going as an attack would.
        if let Some(round) = battle.as_mut()
            && matches!(deed.act, Act::Triggered { .. })
        {
            round.attacked(affected(&deed.effects));
        }
        let effects = deed.effects.clone();
        match deed.act.clone() {
            Act::Cast(mut report) => {
                report.effects.extend(effects);
                magic_news.write(MagicNews::Done(report));
            }
            Act::Missile { caster, spell } => {
                magic_news.write(MagicNews::Landed { caster, spell, effects });
            }
            Act::Triggered { spell } => {
                magic_news.write(MagicNews::Triggered { spell, effects });
            }
            Act::Fight => {
                if !effects.is_empty() {
                    combat_news.write(crate::game::combat::CombatNews::Effects(effects));
                }
            }
            Act::Blow { by, at } => {
                let damage = effects
                    .iter()
                    .map(|effect| match effect {
                        Effect::Damaged { amount, .. } => *amount,
                        _ => 0,
                    })
                    .sum();
                combat_news.write(crate::game::combat::CombatNews::Result {
                    attacker: by,
                    target: at,
                    hits: 1,
                    damage,
                });
                let rest: Vec<Effect> = effects
                    .into_iter()
                    .filter(|effect| !matches!(effect, Effect::Damaged { .. }))
                    .collect();
                if !rest.is_empty() {
                    combat_news.write(crate::game::combat::CombatNews::Effects(rest));
                }
            }
            Act::Swing => unreachable!(),
        }
    }
}

/// Projections are live capabilities, refreshed at consequence boundaries.
#[derive(bevy::ecs::schedule::ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Capabilities;
#[derive(bevy::ecs::schedule::ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
struct AfterEffect;
pub(crate) fn refresh_capabilities(world: &mut World) {
    world.run_schedule(Capabilities);
}
