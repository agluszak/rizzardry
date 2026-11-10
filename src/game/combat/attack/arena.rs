//! A fight to try the rules in: fighters made of the components the rules
//! read, with whatever numbers a test gives them.

use avian3d::prelude::Position;
use bevy::prelude::*;
use enum_map::EnumMap;

use super::{Attack, Fighter, FighterReadOnlyItem, Setting, Surprise};
use crate::content::items::{AttackEffects, AttackMode, Dice};
use crate::content::species::Reach;
use crate::game::actors::{Arm, Armour, Attacks, Attributes, Level, Resistances};
use crate::game::characters::Vitals;
use crate::game::difficulty::Difficulty;
use crate::game::inventory::WeaponSlot;
use crate::game::party::body::PartyBody;
use crate::game::party::formation::Formation;
use crate::game::party::roster::{Party, PartyMember};
use crate::game::rng::GameRng;
use crate::game::world::hostility::{Loyalty, Side, Stance};
use crate::game::world::population::{Monster, ScaledBody};

pub const NORMAL: Setting = Setting { difficulty: Difficulty::Normal, running: false };

pub fn arena() -> World {
    let mut world = World::new();
    world.insert_resource(GameRng::seeded(0));
    world.init_resource::<avian3d::collider_tree::ColliderTrees>();
    world
}

/// What every actor carries, in full health, with one claw.
fn actor(stance: Stance) -> impl Bundle {
    (
        Vitals::full(100, 100),
        Side::new(stance),
        Loyalty::None,
        Attributes::default(),
        Level::default(),
        Resistances::default(),
        Armour::default(),
        Attacks { arms: vec![arm(80)], ..default() },
    )
}

/// A monster half a metre across standing at `at`, facing -z.
pub fn monster(world: &mut World, at: Vec3, stance: Stance) -> Entity {
    let body = ScaledBody { scale: 1.0, radius: 0.5, eye_height: 1.0, speed: 1.0 };
    world
        .spawn((
            actor(stance),
            Monster { location: 0, species: 0, body },
            Transform::from_translation(at),
        ))
        .id()
}

/// A party at the origin facing -z, its members seated in order.
pub fn party(world: &mut World, members: usize) -> (Entity, Vec<Entity>) {
    let party = world.spawn((Party, PartyBody::default(), Position::default())).id();
    let mut formation = Formation::default();
    let members: Vec<Entity> = (0..members)
        .map(|_| {
            let member = world.spawn((actor(Stance::Friendly), PartyMember(party))).id();
            formation.seat(member);
            member
        })
        .collect();
    world.entity_mut(party).insert(formation);
    (party, members)
}

/// A close arm of this rating that swings 1d1 once.
pub fn arm(rating: i32) -> Arm {
    Arm {
        slot: WeaponSlot::Primary,
        weapon: None,
        reach: Reach::Touch,
        modes: AttackMode::Swing.into(),
        dice: Dice { count: 1, sides: 1, base: 0 },
        punch: Dice::default(),
        kick: Dice::default(),
        rating,
        percent: 0,
        effects: AttackEffects::default(),
        slays: Vec::new(),
        attacks: 1,
        initiative: 0,
        swings: 1,
        extra_swings: 0,
        ranged: false,
        loaded: None,
        sure_throw: false,
        critical: None,
        practises: &[],
        weight: None,
        tires: false,
        modern: false,
        fatigue_pierces: false,
        sword: false,
        ambushes: false,
        word: None,
        lands_on_party: None::<EnumMap<_, _>>,
    }
}

/// A plain swing with `arm`.
pub fn swing(arm: &Arm) -> Attack<'_> {
    Attack { arm, mode: AttackMode::Swing, berserk: false, surprise: Surprise::default() }
}

/// Two fighters as a rule reads them.
pub fn duel<R>(
    world: &mut World,
    (a, b): (Entity, Entity),
    rule: impl FnOnce(&FighterReadOnlyItem, &FighterReadOnlyItem) -> R,
) -> R {
    let mut state = world.query::<Fighter>();
    let fighters = state.query(world);
    rule(&fighters.get(a).unwrap(), &fighters.get(b).unwrap())
}

/// All but `left` of an actor's hundred stamina is spent.
pub fn tire(world: &mut World, actor: Entity, left: u32) {
    world.get_mut::<Vitals>(actor).unwrap().stamina.current = left;
}

/// Everyone in the arena, and where they stand, as a rule reads them.
pub fn field<R>(
    world: &mut World,
    rule: impl FnOnce(&Query<super::FighterReadOnly>, &crate::game::actors::Places) -> R,
) -> R {
    let mut state = bevy::ecs::system::SystemState::<(
        Query<super::FighterReadOnly>,
        crate::game::actors::Places,
    )>::new(world);
    let (fighters, places) = state.get(world).unwrap();
    rule(&fighters, &places)
}
