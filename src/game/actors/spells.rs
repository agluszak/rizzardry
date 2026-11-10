//! The spells laid on an actor, or on the party for all its members
//! (`docs/rules/actors.md`, "Spells laid").
//!
//! A spell laid is an entity, [`LaidOn`] its bearer. What it is and does is
//! its components: how long it lasts ([`Turns`]), what it adds to its
//! bearer's numbers ([`Bonus`]), what it can still absorb ([`Absorbs`]),
//! whether it outlasts a fight ([`CombatOnly`]). Being laid and being
//! lifted are that entity's coming and going, and each refits the bearer.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use enum_map::EnumMap;

use super::Refit;
use crate::content::characters::{Attribute, Realm};
use crate::content::spells::SpellId;
use crate::game::party::recovery::Elapsed;

/// A spell laid: which, and the power and boost it was cast at. Razor Cloak
/// and Eye for an Eye lose a point of power each time they answer.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Enchantment {
    pub spell: SpellId,
    pub power: u32,
    pub boost: u32,
}

impl Enchantment {
    /// An amount raised by the cast's boost.
    pub fn scaled(&self, amount: u32) -> u32 {
        amount + amount * self.boost / 100
    }

    /// An amount raised by the cast's boost, rounded.
    pub fn adjusted(&self, amount: u32) -> u32 {
        ((self.boost + 100) * amount + 50) / 100
    }
}

/// Who a spell is laid on: an actor, or the party, whose spells hold for
/// every member, those who join after the casting too.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[relationship(relationship_target = Spells)]
pub struct LaidOn(pub Entity);

/// The spells laid on a bearer, which go with it.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = LaidOn, linked_spawn)]
pub struct Spells(Vec<Entity>);

/// The turns a spell has left. One laid without them lasts until lifted.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Turns(pub u32);

/// It lasts only a fight.
#[derive(Component, Debug)]
pub struct CombatOnly;

/// It pulses on whoever it hangs over as each round ends: the clouds and
/// burning, and Ring of Fire about the party.
#[derive(Component, Debug)]
pub struct Pulses;

/// What Guardian Angel can still absorb, rolled as it was laid.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Absorbs(pub u32);

/// What a spell adds to its bearer's numbers while it is laid; refit sums
/// these over the spells on an actor and on its party.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Bonus {
    pub armour: i32,
    pub to_hit: i32,
    pub resistance: EnumMap<Realm, i32>,
    pub attributes: EnumMap<Attribute, u32>,
    /// Percent chance a missile is turned aside.
    pub missiles: u32,
    /// Percent cut off damage.
    pub reduction: i32,
    /// The share of a stride added to a combat move.
    pub stride: f32,
    /// Sight percentage points lost by eyes behind the bearer.
    pub concealment: u32,
}

impl std::ops::AddAssign for Bonus {
    fn add_assign(&mut self, other: Self) {
        self.armour += other.armour;
        self.to_hit += other.to_hit;
        self.missiles += other.missiles;
        self.reduction += other.reduction;
        self.stride += other.stride;
        self.concealment += other.concealment;
        for (realm, amount) in other.resistance {
            self.resistance[realm] += amount;
        }
        for (attribute, amount) in other.attributes {
            self.attributes[attribute] += amount;
        }
    }
}

/// A spell is replaced by the same spell only if at least as strong when it
/// is one of these (retail `ApplyMonsterCondition`; retail replaces a
/// character's only by a stronger one, a difference taken here as none).
pub fn personal(spell: SpellId) -> bool {
    matches!(
        spell,
        SpellId::DRACON_BREATH
            | SpellId::GUARDIAN_ANGEL
            | SpellId::RAZOR_CLOAK
            | SpellId::EYE_FOR_AN_EYE
            | SpellId::HASTE
            | SpellId::SUPERMAN
            | SpellId::BODY_OF_STONE
    )
}

/// The clouds and burning that pulse on whoever they hang over each round,
/// in pulse order (retail `AdvanceTimedEffects`).
pub const OFFENSIVE: [SpellId; 5] = [
    SpellId::ACID_BOMB,
    SpellId::TOXIC_CLOUD,
    SpellId::DRAINING_CLOUD,
    SpellId::FIRESTORM,
    SpellId::DEATH_CLOUD,
];

fn combat_only(spell: SpellId) -> bool {
    OFFENSIVE.contains(&spell)
        || matches!(
            spell,
            SpellId::BLESS
                | SpellId::ARMORMELT
                | SpellId::ELEMENT_SHIELD
                | SpellId::SOUL_SHIELD
                | SpellId::RING_OF_FIRE
        )
}

/// What a spell adds while laid, at the power and boost it was cast at
/// (retail `RebuildPartyEffectBlock`, `RebuildMonsterDerivedStats`).
/// Armormelt's loss is by its raw power; the boost only lengthens it.
fn bonus(laid: Enchantment) -> Bonus {
    let power = laid.power;
    let mut bonus = Bonus::default();
    match laid.spell {
        SpellId::BLESS => {
            bonus.armour = laid.adjusted(2) as i32;
            bonus.to_hit = laid.adjusted(2) as i32;
        }
        SpellId::ARMORPLATE => bonus.armour = laid.adjusted(power.div_ceil(2)) as i32,
        SpellId::ARMORMELT => bonus.armour = -(power as i32),
        SpellId::ENCHANTED_BLADE => bonus.to_hit = laid.adjusted(power.div_ceil(2)) as i32,
        SpellId::BODY_OF_STONE => {
            bonus.armour = laid.scaled(power) as i32;
            bonus.reduction = laid.scaled(power * 8).min(100) as i32;
        }
        SpellId::MAGIC_SCREEN => {
            bonus.resistance = EnumMap::from_fn(|_| laid.adjusted(power * 4 + 7) as i32);
        }
        SpellId::SOUL_SHIELD => {
            for realm in [Realm::Mental, Realm::Divine] {
                bonus.resistance[realm] = laid.adjusted(power * 10) as i32;
            }
        }
        SpellId::ELEMENT_SHIELD => {
            for realm in [Realm::Fire, Realm::Water, Realm::Air, Realm::Earth] {
                bonus.resistance[realm] = laid.adjusted(power * 7) as i32;
            }
        }
        SpellId::MISSILE_SHIELD => bonus.missiles = laid.adjusted((power + 5) * 5),
        SpellId::SUPERMAN => bonus.attributes = EnumMap::from_fn(|_| laid.scaled(power * 5)),
        SpellId::HASTE => {
            bonus.attributes[Attribute::Speed] = laid.scaled(power * 10);
            bonus.stride = power as f32 * 0.1;
        }
        SpellId::CHAMELEON => bonus.concealment = laid.adjusted(power),
        _ => {}
    }
    bonus
}

/// The spells laid, to read.
#[derive(SystemParam)]
pub struct Laid<'w, 's> {
    bearers: Query<'w, 's, &'static Spells>,
    spells: Query<'w, 's, (&'static Enchantment, &'static Bonus, Option<&'static Turns>)>,
}

impl Laid<'_, '_> {
    fn on(&self, bearer: Entity) -> impl Iterator<Item = (&Enchantment, &Bonus)> {
        let laid = self.bearers.get(bearer).into_iter().flat_map(|spells| spells.0.iter());
        laid.filter_map(|&spell| self.spells.get(spell).ok()).map(|(laid, bonus, _)| (laid, bonus))
    }

    /// Every spell on a bearer with the turns it has left, for whoever
    /// lists them.
    pub fn listed(&self, bearer: Entity) -> impl Iterator<Item = (Enchantment, Option<u32>)> + '_ {
        let laid = self.bearers.get(bearer).into_iter().flat_map(|spells| spells.0.iter());
        laid.filter_map(|&spell| self.spells.get(spell).ok())
            .map(|(laid, _, turns)| (*laid, turns.map(|turns| turns.0)))
    }

    pub fn get(&self, bearer: Entity, spell: SpellId) -> Option<Enchantment> {
        self.on(bearer).map(|(laid, _)| *laid).find(|laid| laid.spell == spell)
    }

    pub fn has(&self, bearer: Entity, spell: SpellId) -> bool {
        self.get(bearer, spell).is_some()
    }

    /// What the spells on a bearer add together.
    pub fn bonus(&self, bearer: Entity) -> Bonus {
        self.on(bearer).fold(Bonus::default(), |mut sum, (_, bonus)| {
            sum += *bonus;
            sum
        })
    }
}

/// The spells laid, to lay, spend and lift.
#[derive(SystemParam)]
pub struct Laying<'w, 's> {
    commands: Commands<'w, 's>,
    bearers: Query<'w, 's, &'static Spells>,
    spells: Query<
        'w,
        's,
        (&'static mut Enchantment, Option<&'static mut Turns>, Option<&'static mut Absorbs>),
    >,
}

impl Laying<'_, '_> {
    fn find(&self, bearer: Entity, spell: SpellId) -> Option<Entity> {
        let laid = self.bearers.get(bearer).ok()?;
        laid.0
            .iter()
            .copied()
            .find(|&entity| self.spells.get(entity).is_ok_and(|(laid, ..)| laid.spell == spell))
    }

    pub fn get(&self, bearer: Entity, spell: SpellId) -> Option<Enchantment> {
        self.find(bearer, spell).and_then(|entity| Some(*self.spells.get(entity).ok()?.0))
    }

    /// Lay a spell on a bearer for `turns`, in place of the same spell
    /// already there; a [`personal`] one only if at least as strong.
    /// `absorbs` is what Guardian Angel takes before it is spent. Whether
    /// it was laid.
    pub fn lay(&mut self, bearer: Entity, laid: Enchantment, turns: u32, absorbs: u32) -> bool {
        let old = self.find(bearer, laid.spell);
        let weaker = personal(laid.spell)
            && old.is_some_and(|old| {
                self.spells.get(old).is_ok_and(|(old, ..)| laid.power < old.power)
            });
        if turns == 0 || weaker {
            return false;
        }
        if let Some(old) = old {
            self.commands.entity(old).despawn();
        }
        let mut spell = self.commands.spawn((laid, bonus(laid), Turns(turns), LaidOn(bearer)));
        if combat_only(laid.spell) {
            spell.insert(CombatOnly);
        }
        if OFFENSIVE.contains(&laid.spell) || laid.spell == SpellId::RING_OF_FIRE {
            spell.insert(Pulses);
        }
        if laid.spell == SpellId::GUARDIAN_ANGEL {
            spell.insert(Absorbs(absorbs));
        }
        true
    }

    /// Lift a spell from a bearer.
    pub fn lift(&mut self, bearer: Entity, spell: SpellId) -> bool {
        let found = self.find(bearer, spell);
        if let Some(entity) = found {
            self.commands.entity(entity).despawn();
        }
        found.is_some()
    }

    /// Lift every spell on a bearer that `keep` does not keep.
    pub fn lift_unless(&mut self, bearer: Entity, keep: impl Fn(SpellId) -> bool) {
        let laid: Vec<Entity> = self.bearers.get(bearer).map_or(vec![], |laid| laid.0.clone());
        for entity in laid {
            if self.spells.get(entity).is_ok_and(|(laid, ..)| !keep(laid.spell)) {
                self.commands.entity(entity).despawn();
            }
        }
    }

    /// Take `turns` off a spell on a bearer; with none left it is lifted.
    /// The turns it had.
    pub fn shorten(&mut self, bearer: Entity, spell: SpellId, turns: u32) -> u32 {
        let Some(entity) = self.find(bearer, spell) else { return 0 };
        let Ok((_, Some(mut left), _)) = self.spells.get_mut(entity) else { return 0 };
        let had = left.0;
        left.0 -= turns.min(had);
        if left.0 == 0 {
            self.commands.entity(entity).despawn();
        }
        had
    }

    /// A slowing for `lasting` first spends itself on the bearer's Haste:
    /// only what is left of it slows, and one without end ends Haste
    /// outright (retail `InflictCharacterCondition`,
    /// `InflictMonsterCondition`). What is left of the slowing, if any.
    pub fn slow(
        &mut self,
        bearer: Entity,
        lasting: crate::game::characters::Lasting,
    ) -> Option<crate::game::characters::Lasting> {
        use crate::game::characters::Lasting;
        match lasting {
            Lasting::Indefinite => {
                self.lift(bearer, SpellId::HASTE);
                Some(lasting)
            }
            Lasting::Rounds(turns) => {
                let spent = self.shorten(bearer, SpellId::HASTE, turns).min(turns);
                (turns > spent).then_some(Lasting::Rounds(turns - spent))
            }
        }
    }

    /// Purify Air shortens each cloud hanging over a bearer by `strength`
    /// less the cloud's own level.
    pub fn purify(&mut self, bearer: Entity, strength: u32, content: &crate::game::data::Catalogs) {
        for cloud in OFFENSIVE {
            let level = content.spells[cloud].level;
            self.shorten(bearer, cloud, strength.max(level) - level);
        }
    }

    /// Anti-Magic ends the spells on a bearer that would outlast a fight.
    pub fn dispel(&mut self, bearer: Entity) {
        self.lift_unless(bearer, combat_only);
    }

    /// Guardian Angel takes what it can of `amount` of damage to a bearer;
    /// what gets through is returned. An angel exactly spent stays until a
    /// blow gets past it, then lifts (as retail keeps it).
    pub fn absorb(&mut self, bearer: Entity, amount: u32) -> u32 {
        let Some(entity) = self.find(bearer, SpellId::GUARDIAN_ANGEL) else { return amount };
        let Ok((.., Some(mut angel))) = self.spells.get_mut(entity) else { return amount };
        if amount <= angel.0 {
            angel.0 -= amount;
            return 0;
        }
        let through = amount - angel.0;
        angel.0 = 0;
        self.commands.entity(entity).despawn();
        through
    }

    /// Razor Cloak or Eye for an Eye on a bearer answers: it loses a point
    /// of power and lifts at none. What it was as it answered.
    pub fn spend(&mut self, bearer: Entity, spell: SpellId) -> Option<Enchantment> {
        let entity = self.find(bearer, spell)?;
        let mut laid = self.spells.get_mut(entity).ok()?.0;
        let before = *laid;
        if before.power == 0 {
            return None;
        }
        laid.power -= 1;
        if laid.power == 0 {
            self.commands.entity(entity).despawn();
        }
        Some(before)
    }
}

/// A spell coming or going refits its bearer; one on a party, every member
/// ([`Refit`] at a party forwards itself).
fn refit_bearer(spell: Entity, bearers: &Query<&LaidOn>, commands: &mut Commands) {
    if let Ok(&LaidOn(entity)) = bearers.get(spell) {
        commands.trigger(Refit { entity });
    }
}

pub(super) fn laid(laid: On<Insert<LaidOn>>, bearers: Query<&LaidOn>, mut commands: Commands) {
    refit_bearer(laid.entity, &bearers, &mut commands);
}

pub(super) fn lifted(lifted: On<Discard<LaidOn>>, bearers: Query<&LaidOn>, mut commands: Commands) {
    refit_bearer(lifted.entity, &bearers, &mut commands);
}

/// Every spell laid lasts less by the aging ticks that passed; one with no
/// turns left is lifted.
pub fn age(
    mut elapsed: MessageReader<Elapsed>,
    mut spells: Query<(Entity, &mut Turns)>,
    mut commands: Commands,
) {
    let ticks: u32 = elapsed.read().map(|elapsed| elapsed.ticks.aging).sum();
    if ticks == 0 {
        return;
    }
    for (spell, mut turns) in &mut spells {
        turns.0 -= ticks.min(turns.0);
        if turns.0 == 0 {
            commands.entity(spell).despawn();
        }
    }
}

/// What lasts only a fight is lifted when it ends.
pub(super) fn end_fight(
    _: On<Remove<crate::game::combat::Battle>>,
    spells: Query<Entity, With<CombatOnly>>,
    mut commands: Commands,
) {
    for spell in &spells {
        commands.entity(spell).despawn();
    }
}

/// The dead keep no spells.
pub(super) fn lift_from_the_dead(
    died: On<Add<crate::game::characters::Dead>>,
    mut commands: Commands,
) {
    commands.entity(died.entity).despawn_related::<Spells>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn laid(spell: SpellId, power: u32, boost: u32) -> Enchantment {
        Enchantment { spell, power, boost }
    }

    /// Armormelt takes its raw power off what Bless and Armorplate add.
    #[test]
    fn armormelt_takes_from_what_protections_add() {
        let mut sum = bonus(laid(SpellId::BLESS, 1, 0));
        sum += bonus(laid(SpellId::ARMORPLATE, 4, 0));
        sum += bonus(laid(SpellId::ARMORMELT, 3, 50));
        assert_eq!((sum.armour, sum.to_hit), (2 + 2 - 3, 2));
    }

    /// The boost raises what a spell adds: Haste's Speed, Body of Stone's cut.
    #[test]
    fn the_boost_raises_what_a_spell_adds() {
        assert_eq!(bonus(laid(SpellId::HASTE, 2, 50)).attributes[Attribute::Speed], 30);
        assert_eq!(bonus(laid(SpellId::BODY_OF_STONE, 2, 0)).reduction, 16);
    }

    /// Soul Shield guards the mind and the soul; Element Shield the rest.
    #[test]
    fn shields_guard_their_own_realms() {
        let soul = bonus(laid(SpellId::SOUL_SHIELD, 2, 0)).resistance;
        let element = bonus(laid(SpellId::ELEMENT_SHIELD, 2, 0)).resistance;
        assert_eq!((soul[Realm::Mental], soul[Realm::Fire]), (20, 0));
        assert_eq!((element[Realm::Mental], element[Realm::Fire]), (0, 14));
    }

    /// A personal spell is replaced only by one as strong; an angel exactly
    /// spent stays until a blow gets past it; a spell with no turns left
    /// lifts and its bearer has it no more.
    #[test]
    fn laying_spending_and_ageing_through_the_world() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.init_resource::<Messages<Elapsed>>();
        let bearer = world.spawn_empty().id();
        let lay = |world: &mut World, spell, power, turns| {
            let laid = world
                .run_system_once(move |mut laying: Laying| {
                    laying.lay(bearer, laid(spell, power, 0), turns, 12)
                })
                .unwrap();
            world.flush();
            laid
        };
        assert!(lay(&mut world, SpellId::HASTE, 3, 5));
        assert!(!lay(&mut world, SpellId::HASTE, 2, 5), "weaker");
        assert!(lay(&mut world, SpellId::BLESS, 3, 2));
        assert!(lay(&mut world, SpellId::BLESS, 1, 2), "not personal");
        assert!(lay(&mut world, SpellId::GUARDIAN_ANGEL, 1, 5));
        let absorb = |world: &mut World, amount| {
            let through = world
                .run_system_once(move |mut laying: Laying| laying.absorb(bearer, amount))
                .unwrap();
            world.flush();
            through
        };
        assert_eq!(absorb(&mut world, 12), 0);
        assert_eq!(absorb(&mut world, 3), 3);
        assert_eq!(absorb(&mut world, 3), 3, "the angel is gone");
        let ticks = crate::game::time::Ticks { stamina: 0, aging: 2 };
        let pace = crate::game::party::recovery::Pace::Still;
        world.write_message(Elapsed { ticks, pace });
        world.run_system_once(age).unwrap();
        world.flush();
        let on = world
            .run_system_once(move |laid: Laid| {
                laid.listed(bearer).map(|(laid, _)| laid.spell).collect::<Vec<_>>()
            })
            .unwrap();
        assert_eq!(on, [SpellId::HASTE]);
    }
}
