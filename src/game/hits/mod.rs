//! Deeds and what they do to those they reach. Whoever does something to
//! others (a cast, a breath, a cloud's pulse, a trap) releases a [`Deed`] at
//! its victims ([`Release`]); it lands on each as it arrives and, once all
//! of it has landed, settles what follows and tells of it, within the
//! release. Only a deed with missiles still in the air lives on as an
//! entity ([`Flight`]).
//! A weapon's blows land the same way, swing by swing, within their attack.

pub mod defences;
pub mod definitions;
pub mod effects;
pub mod reach;
pub mod specials;
pub mod spells;

use std::collections::VecDeque;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::content::items::EquipClass;
use crate::content::spells::SpellId;
use crate::game::actors::Laying;
use crate::game::characters::{Condition, Practise};
use crate::game::combat::{Battle, CombatNews, Combatants, Round};
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::inventory::PartyInventory;
use crate::game::items::Items;
use crate::game::magic::casting::{MagicNews, Practice, Recast, Report};
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::identify::{GroupMember, GroupMembers};
use crate::game::world::obstacles::WorldObstacles;
use definitions::Definitions;
use effects::{Body, Log, Rules, Strike};
use spells::{Leech, Spell};

/// What a deed did to one victim.
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

/// Whether anything a deed did counts as affecting a victim, which is what
/// practice needs (retail `SpellAffectedTarget`).
pub fn affected(effects: &[Effect]) -> bool {
    effects.iter().any(|e| !matches!(e, Effect::Nothing { .. }))
}

/// A body lost health to something, whatever it was. Presentation can keep
/// this even if healing follows before it draws.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Damaged {
    pub body: Entity,
    pub amount: u32,
    pub died: bool,
}

/// Which deed it is, for its news.
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

/// Something done to others: what lands on each of them, and what follows
/// once all of it has.
#[derive(Clone, Debug)]
pub struct Deed {
    pub act: Act,
    pub strike: Strike,
    /// The spell it is, which may burst or be turned back.
    pub spell: Option<Spell>,
    /// Its damage comes back to its caster's side.
    pub leech: Option<Leech>,
    pub practice: Option<Practice>,
    pub recast: Option<Recast>,
}

impl Deed {
    pub fn new(act: Act, strike: Strike) -> Self {
        Self { act, strike, spell: None, leech: None, practice: None, recast: None }
    }
}

/// A deed let go at its victims, each reached at once or after its
/// missile's flight in seconds.
#[derive(Event, Clone, Debug)]
pub struct Release {
    pub deed: Deed,
    pub victims: Vec<(Entity, Option<f32>)>,
}

/// A deed on its way: who it has yet to reach and when, and what it has
/// done so far.
#[derive(Clone, Debug)]
struct Progress {
    deed: Deed,
    /// Those still to reach; a timer for one a missile flies at.
    arrivals: Vec<(Entity, Option<Timer>)>,
    effects: Vec<Effect>,
    harm: u32,
    /// Its party effects have landed, with the first member reached.
    party_reached: bool,
}

impl Progress {
    fn new(deed: Deed, victims: Vec<(Entity, Option<f32>)>) -> Self {
        let arrivals = victims
            .into_iter()
            .map(|(victim, flight)| {
                (victim, flight.map(|seconds| Timer::from_seconds(seconds, TimerMode::Once)))
            })
            .collect();
        Self { deed, arrivals, effects: Vec::new(), harm: 0, party_reached: false }
    }

    /// Land the deed on those it has reached. Its party effects land once,
    /// with the first member reached. Whether all of it has landed.
    fn advance(
        &mut self,
        bodies: &mut Query<Body>,
        (party, inventory, roster): (Entity, &mut PartyInventory, &PartyRoster),
        rules: &Rules,
        rng: &mut GameRng,
        log: &mut Log,
        laying: &mut Laying,
    ) -> bool {
        let strike = self.deed.strike;
        let (due, waiting) = std::mem::take(&mut self.arrivals)
            .into_iter()
            .partition(|(_, timer)| timer.as_ref().is_none_or(Timer::is_finished));
        self.arrivals = waiting;
        for (victim, _) in due {
            let Ok(mut body) = bodies.get_mut(victim) else { continue };
            let member = body.member;
            effects::land(&strike, &mut body, rules, rng, log, laying);
            if member && !self.party_reached {
                self.party_reached = true;
                let mut members: Vec<_> =
                    bodies.iter_many_unique_mut(roster.iter()).matched().collect();
                effects::land_on_party(
                    &strike,
                    party,
                    &mut *inventory,
                    &mut members,
                    rules,
                    rng,
                    log,
                    laying,
                );
            }
        }
        self.effects.append(&mut log.effects);
        self.harm += std::mem::take(&mut log.harm);
        self.arrivals.is_empty()
    }
}

/// A deed with missiles still in the air.
#[derive(Component, Debug)]
pub struct Flight(Progress);

/// A deed's missiles have reached some of their victims.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct Arrival {
    #[event_target]
    flight: Entity,
}

/// Missiles fly on: each step brings them nearer those they are aimed at,
/// and those that arrive land.
pub fn fly(mut commands: Commands, mut flights: Query<(Entity, &mut Flight)>, time: Res<Time>) {
    for (flight, mut deed) in &mut flights {
        let arrivals = &mut deed.0.arrivals;
        for timer in arrivals.iter_mut().filter_map(|(_, timer)| timer.as_mut()) {
            timer.tick(time.delta());
        }
        if arrivals.iter().any(|(_, timer)| timer.as_ref().is_none_or(Timer::is_finished)) {
            commands.trigger(Arrival { flight });
        }
    }
}

/// What deeds land on and what they set off.
#[derive(SystemParam)]
struct Landing<'w, 's> {
    commands: Commands<'w, 's>,
    bodies: Query<'w, 's, Body>,
    party: Single<'w, 's, (Entity, &'static mut PartyInventory, &'static PartyRoster)>,
    laying: Laying<'w, 's>,
    held: Items<'w, 's>,
    fight: Option<
        Single<'w, 's, (Entity, &'static mut Round, Option<&'static Combatants>), With<Battle>>,
    >,
    memberships: Query<'w, 's, &'static GroupMember>,
    groups: Query<'w, 's, &'static GroupMembers>,
    obstacles: WorldObstacles<'w, 's>,
    places: crate::game::actors::Places<'w, 's>,
    content: Res<'w, Catalogs>,
    definitions: Res<'w, Definitions>,
    difficulty: Res<'w, Difficulty>,
    rng: ResMut<'w, GameRng>,
    damaged: MessageWriter<'w, Damaged>,
    requests: MessageWriter<'w, crate::game::magic::MagicRequest>,
    magic_news: MessageWriter<'w, MagicNews>,
    combat_news: MessageWriter<'w, CombatNews>,
}

/// A deed is let go (retail `FinishSpellEffectTargets`): it lands on those
/// it reaches at once, and a missile in the air waits for its arrival.
fn release(release: On<Release>, mut landing: Landing) {
    let Release { deed, victims } = release.event().clone();
    landing.land(Progress::new(deed, victims));
}

/// Missiles reach their victims: the deed lands on them.
fn arrive(arrival: On<Arrival>, mut flights: Query<&mut Flight>, mut landing: Landing) {
    let Ok(flight) = flights.get_mut(arrival.flight) else { return };
    landing.commands.entity(arrival.flight).despawn();
    landing.land(flight.0.clone());
}

impl Landing<'_, '_> {
    /// Land a deed on those it has reached, and settle what a finished deed
    /// sets off in turn (a drain's share, Boiling Blood's bursts, Eye for an
    /// Eye), until only missiles in the air remain. Each deed's
    /// consequences are settled before the next lands.
    fn land(&mut self, progress: Progress) {
        let Landing {
            commands,
            bodies,
            party,
            laying,
            held,
            fight,
            memberships,
            groups,
            obstacles,
            places,
            content,
            definitions,
            difficulty,
            rng,
            damaged,
            requests,
            magic_news,
            combat_news,
        } = self;
        let (party, inventory, roster) = &mut **party;
        let (party, roster) = (*party, *roster);
        let rules = Rules {
            content,
            definitions,
            difficulty: **difficulty,
            fight: fight.as_ref().map(|fight| fight.0),
            held,
        };
        let mut log = Log::default();
        let mut queue = VecDeque::from([progress]);
        while let Some(mut progress) = queue.pop_front() {
            let landed = (party, &mut **inventory, roster);
            if !progress.advance(bodies, landed, &rules, rng, &mut log, laying) {
                commands.spawn(Flight(progress));
                continue;
            }
            let Progress { deed, mut effects, harm, .. } = progress;
            if let Some(leech) = deed.leech {
                effects.extend(spells::drain_back(leech, harm, roster, bodies, content));
            }
            if let Some(spell) = &deed.spell {
                let around = spells::Surroundings {
                    roster,
                    combatants: fight
                        .as_ref()
                        .and_then(|fight| fight.2)
                        .map_or(&[][..], Combatants::entities),
                    memberships,
                    groups,
                    obstacles,
                    places,
                    content,
                    definitions,
                };
                let strength = deed.strike.strength;
                for (burst, victims) in
                    spells::bursts(spell, strength, &bodies.as_readonly(), &around)
                {
                    queue.push_back(Progress::new(burst, victims));
                }
                if let Some((charged, turned, victims)) =
                    spells::reflection(spell, strength, &bodies.as_readonly(), &around, laying)
                {
                    effects.push(Effect::Reflected { victim: charged });
                    queue.push_back(Progress::new(turned, victims));
                }
            }
            if let Some(practice) = &deed.practice
                && (practice.sure || affected(&effects))
            {
                for &(skill, usage) in &practice.skills {
                    commands.trigger(Practise { member: practice.caster, skill, usage });
                }
            }
            if let Some(recast) = &deed.recast
                && recast.wanted(&bodies.as_readonly(), content)
            {
                requests.write(crate::game::magic::MagicRequest {
                    caster: recast.caster,
                    choice: recast.choice,
                });
            }
            match deed.act {
                Act::Cast(mut report) => {
                    report.effects.extend(effects);
                    magic_news.write(MagicNews::Done(report));
                }
                Act::Missile { caster, spell } => {
                    magic_news.write(MagicNews::Landed { caster, spell, effects });
                }
                Act::Triggered { spell } => {
                    // A cloud's pulse keeps the fight going as an attack would.
                    if let Some(fight) = fight.as_mut() {
                        fight.1.attacked(affected(&effects));
                    }
                    magic_news.write(MagicNews::Triggered { spell, effects });
                }
                Act::Fight => {
                    if !effects.is_empty() {
                        combat_news.write(CombatNews::Effects(effects));
                    }
                }
            }
        }
        log.scorch(commands);
        damaged.write_batch(log.damaged);
        for raised in log.raised {
            commands.entity(raised).remove::<crate::game::characters::Dead>();
        }
        for (member, skill, usage) in log.practised {
            commands.trigger(Practise { member, skill, usage });
        }
    }
}

pub struct HitsPlugin;

impl Plugin for HitsPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(release).add_observer(arrive).add_message::<Damaged>().add_systems(
            PreUpdate,
            definitions::define
                .run_if(resource_exists::<Catalogs>)
                .run_if(not(resource_exists::<Definitions>)),
        );
    }
}
