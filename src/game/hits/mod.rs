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
use crate::game::characters::Condition;
use crate::game::party::roster::PartyMember;
use definitions::EffectSpec;
use effects::Strike;

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
    /// Its rows for those it reaches, and for the party as a whole.
    rows: Vec<EffectSpec>,
    party_rows: Vec<EffectSpec>,
    /// Those still to reach; a timer for one a missile flies at.
    arrivals: Vec<(Entity, Option<Timer>)>,
    effects: Vec<Effect>,
    harm: u32,
    /// How the row last weighed on the victim it is landing on came out.
    outcome: Option<applied::Outcome>,
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
        Self {
            rows: applied::rows(&strike.effects, strike.blow, false).collect(),
            party_rows: applied::rows(&strike.effects, strike.blow, true).collect(),
            strike,
            arrivals,
            effects: Vec::new(),
            harm: 0,
            outcome: None,
            party_reached: false,
        }
    }

    /// A row landed, and did this.
    fn landed(&mut self, effect: Effect) {
        self.outcome = Some(applied::Outcome::Landed);
        self.effects.push(effect);
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

/// A deed lands on those it has reached (retail `FinishSpellEffectTargets`),
/// each in turn; its party effects land once, with the first member
/// reached. Once nobody is left to reach it has landed, and is gone.
fn arrive(
    arrival: On<Arrival>,
    mut commands: Commands,
    mut deeds: Query<&mut Deed>,
    members: Query<(), With<PartyMember>>,
) {
    let Ok(mut deed) = deeds.get_mut(arrival.deed) else { return };
    let (due, waiting): (Vec<_>, _) = std::mem::take(&mut deed.arrivals)
        .into_iter()
        .partition(|(_, timer)| timer.as_ref().is_none_or(Timer::is_finished));
    deed.arrivals = waiting;
    for (victim, _) in due {
        commands.trigger(effects::Lands { deed: arrival.deed, victim, row: 0 });
        if members.contains(victim) && !deed.party_reached {
            deed.party_reached = true;
            commands.trigger(effects::LandsOnParty { deed: arrival.deed });
        }
    }
    if deed.arrivals.is_empty() {
        commands.trigger(Reached { deed: arrival.deed });
    }
}

/// A deed has reached everyone it was aimed at: it has landed, and is gone.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct Reached {
    #[event_target]
    deed: Entity,
}

fn reached(reached: On<Reached>, mut deeds: Query<&mut Deed>, mut commands: Commands) {
    let Ok(mut deed) = deeds.get_mut(reached.deed) else { return };
    commands.trigger(Landed {
        deed: reached.deed,
        effects: std::mem::take(&mut deed.effects),
        harm: deed.harm,
    });
    commands.entity(reached.deed).despawn();
}

pub struct HitsPlugin;

impl Plugin for HitsPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(release)
            .add_observer(arrive)
            .add_observer(reached)
            .add_message::<Damaged>();
        effects::plugin(app);
    }
}
