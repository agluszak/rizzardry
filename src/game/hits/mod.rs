//! Deeds and what they do to those they reach. Whoever does something to
//! others (a cast, a breath, a cloud's pulse, a trap) makes a [`Deed`] aimed
//! at its victims; it lands on each as it arrives and, once all of it has
//! landed, says so ([`Landed`]) and is gone. What follows a deed is its
//! maker's business: whatever the maker put beside the deed is there for
//! the maker's own observer of [`Landed`] to read.
//! A weapon's blows land the same way, swing by swing, within their attack.

pub mod applied;
pub mod defences;
pub mod definitions;
pub mod effects;
pub mod reach;
pub mod specials;

use bevy::prelude::*;

use crate::content::items::EquipClass;
use crate::content::spells::SpellId;
use crate::game::actors::Laying;
use crate::game::characters::{Condition, Practise};
use crate::game::combat::Battle;
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::inventory::PartyInventory;
use crate::game::items::Items;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use effects::{Body, Log, Rules, Strike};

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

/// A body lost health to something, whatever it was. Presentation can keep
/// this even if healing follows before it draws.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Damaged {
    pub body: Entity,
    pub amount: u32,
    pub died: bool,
}

/// Something done to others, on its way to them: what lands on each, who
/// it has yet to reach, and what it has done so far.
#[derive(Component, Clone, Debug)]
pub struct Deed {
    pub strike: Strike,
    /// Those still to reach; a timer for one a missile flies at.
    arrivals: Vec<(Entity, Option<Timer>)>,
    effects: Vec<Effect>,
    harm: u32,
    /// Its party effects have landed, with the first member reached.
    party_reached: bool,
}

impl Deed {
    /// Those it has yet to reach.
    pub fn aimed_at(&self) -> impl Iterator<Item = Entity> + '_ {
        self.arrivals.iter().map(|(victim, _)| *victim)
    }

    /// A deed that reaches its victims at once.
    pub fn at(strike: Strike, victims: impl IntoIterator<Item = Entity>) -> Self {
        Self::flying(strike, victims.into_iter().map(|victim| (victim, None)))
    }

    /// A deed that reaches each victim at once or after its missile's
    /// flight in seconds.
    pub fn flying(
        strike: Strike,
        victims: impl IntoIterator<Item = (Entity, Option<f32>)>,
    ) -> Self {
        let arrivals = victims
            .into_iter()
            .map(|(victim, flight)| {
                (victim, flight.map(|seconds| Timer::from_seconds(seconds, TimerMode::Once)))
            })
            .collect();
        Self { strike, arrivals, effects: Vec::new(), harm: 0, party_reached: false }
    }
}

/// All of a deed has landed: what it did, and the health its plain harm
/// would have taken. The deed is still there to be read, and gone after.
#[derive(EntityEvent, Clone, Debug)]
pub struct Landed {
    #[event_target]
    pub deed: Entity,
    pub effects: Vec<Effect>,
    pub harm: u32,
}

impl Landed {
    /// Whether it did anything to anyone.
    pub fn affected(&self) -> bool {
        self.effects.iter().any(|effect| !matches!(effect, Effect::Nothing { .. }))
    }
}

/// A deed has reached some of its victims.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct Arrival {
    #[event_target]
    deed: Entity,
}

/// A deed is made: it lands on those it reaches at once.
fn release(made: On<Add<Deed>>, mut commands: Commands) {
    commands.trigger(Arrival { deed: made.entity });
}

/// Missiles fly on: each step brings them nearer those they are aimed at.
pub fn fly(mut commands: Commands, mut deeds: Query<(Entity, &mut Deed)>, time: Res<Time>) {
    for (deed, mut flying) in &mut deeds {
        for timer in flying.arrivals.iter_mut().filter_map(|(_, timer)| timer.as_mut()) {
            timer.tick(time.delta());
        }
        if flying.arrivals.iter().any(|(_, timer)| timer.as_ref().is_some_and(Timer::is_finished)) {
            commands.trigger(Arrival { deed });
        }
    }
}

/// A deed lands on those it has reached (retail `FinishSpellEffectTargets`);
/// its party effects land once, with the first member reached. Once nobody
/// is left to reach it has landed, and is gone.
fn arrive(
    arrival: On<Arrival>,
    mut commands: Commands,
    mut deeds: Query<&mut Deed>,
    mut bodies: Query<Body>,
    party: Single<(Entity, &mut PartyInventory, &PartyRoster)>,
    mut laying: Laying,
    held: Items,
    fight: Option<Single<Entity, With<Battle>>>,
    (content, difficulty): (Res<Catalogs>, Res<Difficulty>),
    mut rng: ResMut<GameRng>,
    mut damaged: MessageWriter<Damaged>,
) {
    let Ok(mut deed) = deeds.get_mut(arrival.deed) else { return };
    let (party, mut inventory, roster) = party.into_inner();
    let rules = Rules {
        content: &content,
        difficulty: *difficulty,
        fight: fight.map(|fight| *fight),
        held: &held,
    };
    let mut log = Log::default();
    let strike = deed.strike;
    let (due, waiting) = std::mem::take(&mut deed.arrivals)
        .into_iter()
        .partition(|(_, timer)| timer.as_ref().is_none_or(Timer::is_finished));
    deed.arrivals = waiting;
    for (victim, _) in due {
        let Ok(mut body) = bodies.get_mut(victim) else { continue };
        let member = body.member;
        effects::land(&strike, &mut body, &rules, &mut rng, &mut log, &mut laying);
        if member && !deed.party_reached {
            deed.party_reached = true;
            let mut members: Vec<_> =
                bodies.iter_many_unique_mut(roster.iter()).matched().collect();
            effects::land_on_party(
                &strike,
                party,
                &mut inventory,
                &mut members,
                &rules,
                &mut rng,
                &mut log,
                &mut laying,
            );
        }
    }
    deed.effects.append(&mut log.effects);
    deed.harm += log.harm;
    if deed.arrivals.is_empty() {
        commands.trigger(Landed {
            deed: arrival.deed,
            effects: std::mem::take(&mut deed.effects),
            harm: deed.harm,
        });
        commands.entity(arrival.deed).despawn();
    }
    log.scorch(&mut commands);
    damaged.write_batch(log.damaged);
    for (member, skill, usage) in log.practised {
        commands.trigger(Practise { member, skill, usage });
    }
}

pub struct HitsPlugin;

impl Plugin for HitsPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(release).add_observer(arrive).add_message::<Damaged>();
    }
}
