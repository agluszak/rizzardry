//! What lands on bodies. An act (a cast, a missile's impact, a breath, a
//! cloud's pulse, a trap, a blow) is an entity with a [`Deed`] recording what
//! it did. It reaches each body it lands on ([`Reach`]), and the reach works
//! through what the act's definition does to that body one effect at a time
//! ([`effects`]): each effect is an entity that [`Strikes`] the body and is a
//! [`HitOf`] the act, settled by the [`Hits`] stages. Once an act's reaches
//! are done, what follows (a drain's share, a burst, a reflection) is made
//! and [`report_deeds`] turns the act into news.

pub mod defences;
pub mod effects;
pub mod prototypes;
pub mod reach;
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
    effects.iter().any(|e| !matches!(e, Effect::Nothing { .. }))
}

/// The act is made by this actor; the actor gone, so is the act.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = Acts)]
pub struct By(pub Entity);

/// An actor's acts under way, maintained by Bevy from [`By`].
#[derive(Component, Debug)]
#[relationship_target(relationship = By, linked_spawn)]
pub struct Acts(Vec<Entity>);

/// The reach or effect lands on this body.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = Incoming)]
pub struct Strikes(pub Entity);

/// The reaches and effects landing on a body, gone with it.
#[derive(Component, Debug)]
#[relationship_target(relationship = Strikes, linked_spawn)]
pub struct Incoming(Vec<Entity>);

/// The reach or effect belongs to this act.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = DeedHits)]
pub struct HitOf(pub Entity);

/// An act's reaches and effects still to land, gone with it.
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

/// Run the hit stages until no reach is left to work but missiles still on
/// their way: each effect, and what a landed act set off (a reflection, a
/// burst), is settled within the same step.
pub fn run_hits(world: &mut World) {
    world.run_schedule(Hits);
    let mut passes = 1;
    while world
        .query_filtered::<(), (With<HitOf>, Without<effects::InFlight>)>()
        .iter(world)
        .next()
        .is_some()
    {
        passes += 1;
        if passes > 256 {
            error!("effects left unsettled after {passes} passes");
            break;
        }
        world.run_schedule(Hits);
    }
}

pub struct HitsPlugin;

impl Plugin for HitsPlugin {
    fn build(&self, app: &mut App) {
        use effects::*;
        use prototypes::{Definitions, advance, define, pick, roll};
        app.add_systems(
            PreUpdate,
            define
                .run_if(resource_exists::<crate::game::data::Catalogs>)
                .run_if(not(resource_exists::<Definitions>)),
        )
        .add_systems(
            Hits,
            (
                (defences::judge_characters, defences::judge_monsters)
                    .run_if(any_with_component::<HitOf>),
                spells::strength_of_owners,
                advance,
                (
                    pick,
                    roll,
                    miss_the_dead,
                    roll_chances,
                    immunities,
                    saves,
                    iron_will,
                    resist,
                    shrug,
                )
                    .chain(),
                (
                    harm,
                    sap,
                    afflict,
                    slay,
                    tire,
                    drain_pool,
                    drain_budget,
                    drain_realm,
                    heal,
                    refresh,
                    replenish,
                    cure,
                    renew,
                    raise,
                    purify,
                    enchant,
                    burn,
                )
                    .chain(),
                (enchant_party, purify_party, drain_party, burn_pool, lose_gold, dispel).chain(),
                (
                    notify_hurt,
                    break_lures,
                    count_drains,
                    wake,
                    crate::game::characters::sync_member_maxima,
                    crate::game::characters::cheat_death,
                    crate::game::characters::settle_deaths,
                )
                    .chain(),
                (
                    spells::drain_back,
                    spells::burst_blood,
                    spells::reflect_spells,
                    crate::game::magic::casting::practise_casts,
                    crate::game::magic::casting::recast,
                )
                    .chain(),
                report_deeds,
            )
                .chain(),
        )
        .add_systems(Update, run_hits.in_set(Resolution))
        .add_systems(FixedUpdate, (fly, run_hits).chain().in_set(Resolution))
        // Outside a fight acts land on the frame, once magic is accepted; in
        // one, on the step, once the round's actors and clouds have made
        // theirs and before its deaths and defeat are settled.
        .configure_sets(Update, Resolution.after(crate::game::GameplaySet::Combat))
        .configure_sets(
            FixedUpdate,
            Resolution
                .after(crate::game::combat::end_round)
                .before(crate::game::combat::summon::summon),
        );
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
