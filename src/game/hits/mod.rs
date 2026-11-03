//! What lands on bodies. An act (a cast, a missile's impact, a breath, a
//! cloud's pulse, a trap, a blow) is an entity with a [`Deed`] recording
//! what it did; each effect it has on a body is an entity that [`Strikes`]
//! that body and is a [`HitOf`] the act ([`effects`]). The [`Hits`] stages
//! settle the effects on the bodies' components, then what follows once an
//! act's effects have all landed, and [`report_deeds`] turns each finished
//! act into news.

pub mod body;
pub mod effects;
pub mod prototypes;
pub mod reach;
pub mod specials;
pub mod spells;

use bevy::prelude::*;

use crate::content::characters::Skill;
use crate::content::items::EquipClass;
use crate::content::spells::SpellId;
use crate::game::characters::Condition;
use crate::game::magic::casting::{MagicNews, Report};
use crate::game::world::damage::DamageOutcome;

pub use body::{Body, BodyItem, BodyReadOnlyItem};

/// What a spell did to one victim.
#[derive(Clone, Copy, Debug, PartialEq)]
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
        outcome: DamageOutcome,
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

/// Whether anything a spell did counts as affecting a victim, which is what
/// practice needs (retail `SpellAffectedTarget`).
pub fn affected(effects: &[Effect]) -> bool {
    effects.iter().any(|e| !matches!(e, Effect::Nothing { .. }))
}

/// The hit lands on this body.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = Incoming)]
pub struct Strikes(pub Entity);

/// The hits landing on a body, gone with it.
#[derive(Component, Debug)]
#[relationship_target(relationship = Strikes, linked_spawn)]
pub struct Incoming(Vec<Entity>);

/// The hit belongs to this deed.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = DeedHits)]
pub struct HitOf(pub Entity);

/// A deed's hits still to land.
#[derive(Component, Debug)]
#[relationship_target(relationship = HitOf, linked_spawn)]
pub struct DeedHits(Vec<Entity>);

/// The deed is part of a larger one, which takes what it did into its own
/// report: a blow of an attack.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = Parts)]
pub struct PartOf(pub Entity);

/// The deeds still part of a larger one, gone with it.
#[derive(Component, Debug)]
#[relationship_target(relationship = PartOf, linked_spawn)]
pub struct Parts(Vec<Entity>);

/// What one act did, gathered from its hits for its news.
#[derive(Component, Debug)]
pub struct Deed {
    pub act: Act,
    pub effects: Vec<Effect>,
    /// Skills its hits made victims practise, by whom.
    pub improved: Vec<(Entity, Skill)>,
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
}

impl Deed {
    pub fn new(act: Act) -> Self {
        Self { act, effects: Vec::new(), improved: Vec::new() }
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

/// Run the hit stages until no effect is left to settle but missiles still
/// on their way: an effect waiting on the one before it, or what a landed
/// act set off (a reflection, a burst), is settled within the same step.
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
        if passes > 64 {
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
        use prototypes::{Definitions, define, instantiate, pick, roll};
        app.add_systems(
            PreUpdate,
            define
                .run_if(resource_exists::<crate::game::data::Catalogs>)
                .run_if(not(resource_exists::<Definitions>)),
        )
        .add_systems(
            Hits,
            (
                (
                    instantiate,
                    pick,
                    roll,
                    ready,
                    miss_the_dead,
                    roll_chances,
                    immunities,
                    saves,
                    resist,
                )
                    .chain(),
                (
                    harm,
                    sap,
                    afflict,
                    slay,
                    tire,
                    drain_magic,
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
                    lose_gold,
                    dispel,
                )
                    .chain(),
                release_followers,
                (spells::drain_life, spells::burst_blood, spells::reflect_spells).chain(),
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

/// Turn each deed whose hits have all landed into news, crediting a
/// fight's kills, and let it go; one part of a larger deed is left to it.
pub fn report_deeds(
    mut commands: Commands,
    deeds: Query<(Entity, &Deed), (Without<DeedHits>, Without<PartOf>)>,
    members: Query<(), With<crate::game::characters::Character>>,
    mut battle: Option<Single<&mut crate::game::combat::Battle>>,
    mut magic_news: MessageWriter<MagicNews>,
    mut combat_news: MessageWriter<crate::game::combat::CombatNews>,
) {
    for (entity, deed) in &deeds {
        commands.entity(entity).despawn();
        if let Some(battle) = battle.as_mut() {
            battle.credit_kills(&deed.effects, &members, &mut combat_news);
            // A cloud's pulse keeps the fight going as an attack would.
            if matches!(deed.act, Act::Triggered { .. }) {
                battle.attacked(affected(&deed.effects));
            }
        }
        let (effects, improved) = (deed.effects.clone(), deed.improved.clone());
        match deed.act.clone() {
            Act::Cast(mut report) => {
                report.effects.extend(effects);
                report.improved.extend(improved);
                magic_news.write(MagicNews::Done(report));
            }
            Act::Missile { caster, spell } => {
                magic_news.write(MagicNews::Landed { caster, spell, effects, improved });
            }
            Act::Triggered { spell } => {
                magic_news.write(MagicNews::Triggered { spell, effects, improved });
            }
            Act::Fight => {
                if !effects.is_empty() {
                    combat_news.write(crate::game::combat::CombatNews::Effects(effects));
                }
            }
        }
    }
}
