//! What acting in a fight costs one that tires by what it does
//! (`docs/rules/monsters.md`, "Effort"). One that tires by what it swings
//! pays for each swing instead ([`super::attack::swing_stamina`]).

use bevy::prelude::*;

use crate::game::characters::{Condition, Conditions, Vitals};
use crate::game::rng::GameRng;

/// One that tires by what it does in a fight, not by what it swings: a
/// monster, marked as it is made.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Labours;

/// Something done in a fight that takes effort.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Deserialize)]
pub enum Effort {
    /// Closing in, hurt.
    Charge,
    /// Running for its life.
    Run,
    /// Walking up to strike.
    Approach,
    /// Backing away.
    Return,
    /// Holding back, or drawn by a lure.
    Wait,
    /// One attack, whatever its swings.
    Attack,
    /// One attack in a fury.
    Fury,
}

/// A row of `assets/tables/efforts.ron`.
#[derive(Clone, Copy, Debug, serde::Deserialize)]
struct Cost {
    below: u32,
    add: u32,
}

static COSTS: std::sync::LazyLock<std::collections::HashMap<Effort, Cost>> =
    std::sync::LazyLock::new(|| {
        ron::from_str(include_str!("../../../assets/tables/efforts.ron"))
            .expect("assets/tables/efforts.ron is a table of efforts")
    });

/// An actor did something in a fight that took effort.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Exerted {
    #[event_target]
    pub actor: Entity,
    pub effort: Effort,
}

/// One that tires by what it does pays for it, twice over when diseased.
pub(super) fn tire(
    exerted: On<Exerted>,
    mut actors: Query<(&mut Vitals, &Conditions), With<Labours>>,
    mut rng: ResMut<GameRng>,
) {
    let Ok((mut vitals, conditions)) = actors.get_mut(exerted.actor) else { return };
    let Cost { below, add } = COSTS[&exerted.effort];
    let cost = rng.below(below) + add;
    vitals.tire(if conditions.has(Condition::Diseased) { cost * 2 } else { cost });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Retail `MonsterActionFatigueCost`: an attack costs a monster two to
    /// four stamina, twice that diseased; a member's costs are their
    /// swings'.
    #[test]
    fn a_monsters_attack_tires_it_and_twice_as_much_diseased() {
        let mut world = World::new();
        world.insert_resource(GameRng::default());
        world.add_observer(tire);
        let monster = world.spawn((Vitals::full(10, 100), Labours)).id();
        let sick = world.spawn((Vitals::full(10, 100), Labours)).id();
        world
            .get_mut::<Conditions>(sick)
            .unwrap()
            .afflict(Condition::Diseased, crate::game::characters::Lasting::Indefinite);
        let member = world.spawn(Vitals::full(10, 100)).id();
        for actor in [monster, sick, member] {
            world.trigger(Exerted { actor, effort: Effort::Attack });
        }
        let spent = |actor| 100 - world.get::<Vitals>(actor).unwrap().stamina.current;
        assert!((2..=4).contains(&spent(monster)), "{}", spent(monster));
        assert!((4..=8).contains(&spent(sick)) && spent(sick) % 2 == 0, "{}", spent(sick));
        assert_eq!(spent(member), 0);
    }
}
