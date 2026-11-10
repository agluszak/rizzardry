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
use crate::content::spells::{Amount, Boost, Pulse, Pulsing, SpellId, Spread, WhileLaid};
use crate::game::data::Catalogs;

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

    /// What a row's amount comes to at this power and boost.
    fn amount(&self, amount: &Amount) -> i32 {
        let base = amount.per_power * self.power as i32 + amount.plus;
        let base = if amount.halved_up { (base + 1) / 2 } else { base };
        let raised = match amount.boost {
            Boost::Unboosted => base,
            Boost::Rounded => self.adjusted(base as u32) as i32,
            Boost::Scaled => self.scaled(base as u32) as i32,
        };
        amount.cap.map_or(raised, |cap| raised.min(cap))
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

/// It pulses as each round ends, as its row says: the clouds and burning on
/// whoever they hang over, and Ring of Fire about the party.
#[derive(Component, Debug)]
pub struct Pulses(pub Pulse);

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
    /// The bearer is warned of what is about.
    pub warns: bool,
    /// The bearer finds what is hidden as they search.
    pub reveals: bool,
    /// The bearer can breathe as a Dracon does.
    pub breath: bool,
}

/// How a laid spell answers what comes at its bearer, as its rows say.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Answers {
    /// A slowing first spends itself against it.
    pub slowing: bool,
    /// It answers a close blow with its dice.
    pub blows: bool,
}

impl std::ops::AddAssign for Bonus {
    fn add_assign(&mut self, other: Self) {
        self.armour += other.armour;
        self.to_hit += other.to_hit;
        self.missiles += other.missiles;
        self.reduction += other.reduction;
        self.stride += other.stride;
        self.concealment += other.concealment;
        self.warns |= other.warns;
        self.reveals |= other.reveals;
        self.breath |= other.breath;
        for (realm, amount) in other.resistance {
            self.resistance[realm] += amount;
        }
        for (attribute, amount) in other.attributes {
            self.attributes[attribute] += amount;
        }
    }
}

/// What a spell adds while laid, at the power and boost it was cast at, from
/// its row in `assets/tables/spells_laid.ron`.
fn bonus(laid: Enchantment, row: &WhileLaid) -> Bonus {
    let amount = |amount: &Option<Amount>| amount.map_or(0, |amount| laid.amount(&amount));
    fn spread<T: PartialEq>(laid: Enchantment, spread: &Option<Spread<T>>, kind: T) -> i32 {
        spread
            .as_ref()
            .filter(|spread| spread.covers(kind))
            .map_or(0, |spread| laid.amount(&spread.amount))
    }
    Bonus {
        armour: amount(&row.armour),
        to_hit: amount(&row.to_hit),
        resistance: EnumMap::from_fn(|realm| spread(laid, &row.resistance, realm)),
        attributes: EnumMap::from_fn(|attribute| spread(laid, &row.attributes, attribute) as u32),
        missiles: amount(&row.missiles) as u32,
        reduction: amount(&row.reduction),
        stride: laid.power as f32 * row.stride_per_power,
        concealment: amount(&row.concealment) as u32,
        warns: row.warns,
        reveals: row.reveals,
        breath: row.breath,
    }
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
    content: Res<'w, Catalogs>,
    bearers: Query<'w, 's, &'static Spells>,
    spells: Query<
        'w,
        's,
        (&'static mut Enchantment, Option<&'static mut Turns>, Option<&'static mut Absorbs>),
    >,
    answers: Query<'w, 's, &'static Answers>,
}

impl Laying<'_, '_> {
    /// The spell on a bearer that answers in this way.
    fn answering(&self, bearer: Entity, how: fn(&Answers) -> bool) -> Option<Entity> {
        let laid = self.bearers.get(bearer).ok()?;
        laid.0.iter().copied().find(|&entity| self.answers.get(entity).is_ok_and(how))
    }

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
    /// already there; a personal one only if at least as strong.
    /// `absorbs` is what Guardian Angel takes before it is spent. Whether
    /// it was laid.
    pub fn lay(&mut self, bearer: Entity, laid: Enchantment, turns: u32, absorbs: u32) -> bool {
        let old = self.find(bearer, laid.spell);
        let facts = &self.content.spells[laid.spell].facts;
        let weaker = facts.personal
            && old.is_some_and(|old| {
                self.spells.get(old).is_ok_and(|(old, ..)| laid.power < old.power)
            });
        if turns == 0 || weaker {
            return false;
        }
        if let Some(old) = old {
            self.commands.entity(old).despawn();
        }
        let mut spell =
            self.commands.spawn((laid, bonus(laid, &facts.laid), Turns(turns), LaidOn(bearer)));
        if facts.only_in_a_fight {
            spell.insert(CombatOnly);
        }
        if let Some(pulse) = facts.pulse {
            spell.insert(Pulses(pulse));
        }
        let row = self.content.spell_effects.get(laid.spell);
        if row.is_some_and(|row| row.absorbs) {
            spell.insert(Absorbs(absorbs));
        }
        let answers = Answers {
            slowing: row.is_some_and(|row| row.answers_slowing),
            blows: facts.laid.answers_blows,
        };
        if answers.slowing || answers.blows {
            spell.insert(answers);
        }
        true
    }

    /// Take `turns` off a spell laid; with none left it is lifted. The turns
    /// it had.
    fn shorten(&mut self, entity: Entity, turns: u32) -> u32 {
        let Ok((_, Some(mut left), _)) = self.spells.get_mut(entity) else { return 0 };
        let had = left.0;
        left.0 -= turns.min(had);
        if left.0 == 0 {
            self.commands.entity(entity).despawn();
        }
        had
    }

    /// A slowing for `lasting` first spends itself on a spell of the bearer's
    /// that answers one (Haste): only what is left of it slows, and one
    /// without end ends that spell outright (retail `InflictCharacterCondition`,
    /// `InflictMonsterCondition`). What is left of the slowing, if any.
    pub fn slow(
        &mut self,
        bearer: Entity,
        lasting: crate::game::characters::Lasting,
    ) -> Option<crate::game::characters::Lasting> {
        use crate::game::characters::Lasting;
        let Some(haste) = self.answering(bearer, |answers| answers.slowing) else {
            return Some(lasting);
        };
        match lasting {
            Lasting::Indefinite => {
                self.commands.entity(haste).despawn();
                Some(lasting)
            }
            Lasting::Rounds(turns) => {
                let spent = self.shorten(haste, turns).min(turns);
                (turns > spent).then_some(Lasting::Rounds(turns - spent))
            }
        }
    }

    /// Purify Air shortens each cloud hanging over a bearer by `strength`
    /// less the cloud's own level.
    pub fn purify(&mut self, bearer: Entity, strength: u32) {
        let hanging: Vec<Entity> = self.bearers.get(bearer).map_or(vec![], |laid| laid.0.clone());
        for entity in hanging {
            let Ok((laid, ..)) = self.spells.get(entity) else { continue };
            let spell = laid.spell;
            let cloud = &self.content.spells[spell];
            let hangs = cloud.facts.pulse.is_some_and(|pulse| pulse.on == Pulsing::Bearer);
            let by = strength.max(cloud.level) - cloud.level;
            if hangs {
                self.shorten(entity, by);
            }
        }
    }

    /// Anti-Magic ends the spells on a bearer that would outlast a fight.
    pub fn dispel(&mut self, bearer: Entity) {
        let laid: Vec<Entity> = self.bearers.get(bearer).map_or(vec![], |laid| laid.0.clone());
        for entity in laid {
            if self
                .spells
                .get(entity)
                .is_ok_and(|(laid, ..)| !self.content.spells[laid.spell].facts.only_in_a_fight)
            {
                self.commands.entity(entity).despawn();
            }
        }
    }

    /// A spell that absorbs (Guardian Angel) takes what it can of `amount`
    /// of damage to a bearer; what gets through is returned. One exactly
    /// spent stays until a blow gets past it, then lifts (as retail keeps
    /// it).
    pub fn absorb(&mut self, bearer: Entity, amount: u32) -> u32 {
        let absorbing = self.bearers.get(bearer).ok().and_then(|laid| {
            laid.0.iter().copied().find(|&e| self.spells.get(e).is_ok_and(|s| s.2.is_some()))
        });
        let Some(entity) = absorbing else { return amount };
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

    /// A spell on a bearer that answers close blows (Razor Cloak) answers
    /// one: what it was as it answered.
    pub fn answer_blow(&mut self, bearer: Entity) -> Option<Enchantment> {
        let entity = self.answering(bearer, |answers| answers.blows)?;
        self.spend_point(entity)
    }

    /// A spell on a bearer answers (Eye for an Eye): what it was as it
    /// answered.
    pub fn spend(&mut self, bearer: Entity, spell: SpellId) -> Option<Enchantment> {
        let entity = self.find(bearer, spell)?;
        self.spend_point(entity)
    }

    /// A spell that answers loses a point of power and lifts at none.
    fn spend_point(&mut self, entity: Entity) -> Option<Enchantment> {
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

/// Time has worn on the spells laid, by so many aging ticks. After the
/// time that passed ([`Elapsed`](crate::game::party::recovery::Elapsed)): a round's last pulses land between.
#[derive(Event, Clone, Copy, Debug)]
pub struct Aged {
    pub ticks: u32,
}

/// Every spell laid lasts less by the aging ticks that passed; one with no
/// turns left is lifted.
pub fn age(aged: On<Aged>, mut spells: Query<(Entity, &mut Turns)>, mut commands: Commands) {
    for (spell, mut turns) in &mut spells {
        turns.wear(spell, aged.ticks, &mut commands);
    }
}

impl Turns {
    /// The spell lasts less by so many aging ticks; with no turns left it is
    /// lifted.
    pub(super) fn wear(&mut self, spell: Entity, ticks: u32, commands: &mut Commands) {
        self.0 -= ticks.min(self.0);
        if self.0 == 0 {
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
    use crate::content::spells::SpellCatalog;

    /// The test catalog: every spell has a record, with the facts the game's
    /// table gives it.
    pub(crate) fn catalog() -> Catalogs {
        let spells = (0..=255)
            .map(|id| crate::content::spells::SpellDefinition {
                id: SpellId(id),
                realm: Some(Realm::Divine),
                ..default()
            })
            .collect();
        Catalogs { spells: SpellCatalog::new(spells), ..default() }
    }

    /// What a spell adds, laid at this power and boost.
    fn bonus_at(spell: SpellId, power: u32, boost: u32) -> Bonus {
        bonus(laid(spell, power, boost), &catalog().spells[spell].facts.laid)
    }

    fn laid(spell: SpellId, power: u32, boost: u32) -> Enchantment {
        Enchantment { spell, power, boost }
    }

    /// The rows give each amount as the rule works it out.
    #[test]
    fn what_a_row_adds_is_worked_out_from_the_cast() {
        // Armorplate: half the power, rounded up, raised by the boost, rounded.
        assert_eq!(bonus_at(SpellId::ARMORPLATE, 3, 0).armour, 2);
        assert_eq!(bonus_at(SpellId::ARMORPLATE, 3, 50).armour, 3);
        // Magic Screen guards every realm by four a power and seven.
        let screen = bonus_at(SpellId::MAGIC_SCREEN, 2, 0).resistance;
        assert!(screen.values().all(|&amount| amount == 15));
        assert_eq!(bonus_at(SpellId::MISSILE_SHIELD, 1, 0).missiles, 30);
        // Superman raises every attribute five a power, the boost rounding down.
        let superman = bonus_at(SpellId::SUPERMAN, 3, 50).attributes;
        assert!(superman.values().all(|&amount| amount == 22));
        assert_eq!(bonus_at(SpellId::CHAMELEON, 3, 0).concealment, 3);
        // Haste adds a tenth of a stride a power.
        assert_eq!(bonus_at(SpellId::HASTE, 4, 0).stride, 4.0 * 0.1);
        assert_eq!(bonus_at(SpellId::ENCHANTED_BLADE, 5, 0).to_hit, 3);
        // Body of Stone's cut is held to all of the damage.
        assert_eq!(bonus_at(SpellId::BODY_OF_STONE, 13, 0).reduction, 100);
        assert_eq!(bonus_at(SpellId::BODY_OF_STONE, 13, 0).armour, 13);
        // A spell with no row adds nothing.
        assert_eq!(bonus_at(SpellId::FIREBALL, 7, 100), Bonus::default());
    }

    /// Armormelt takes its raw power off what Bless and Armorplate add.
    #[test]
    fn armormelt_takes_from_what_protections_add() {
        let mut sum = bonus_at(SpellId::BLESS, 1, 0);
        sum += bonus_at(SpellId::ARMORPLATE, 4, 0);
        sum += bonus_at(SpellId::ARMORMELT, 3, 50);
        assert_eq!((sum.armour, sum.to_hit), (2 + 2 - 3, 2));
    }

    /// The boost raises what a spell adds: Haste's Speed, Body of Stone's cut.
    #[test]
    fn the_boost_raises_what_a_spell_adds() {
        assert_eq!(bonus_at(SpellId::HASTE, 2, 50).attributes[Attribute::Speed], 30);
        assert_eq!(bonus_at(SpellId::BODY_OF_STONE, 2, 0).reduction, 16);
    }

    /// A personal spell is replaced only by one as strong; an angel exactly
    /// spent stays until a blow gets past it; a spell with no turns left
    /// lifts and its bearer has it no more.
    #[test]
    fn laying_spending_and_ageing_through_the_world() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.insert_resource(catalog());
        world.add_observer(age);
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
        world.trigger(Aged { ticks: ticks.aging });
        world.flush();
        let on = world
            .run_system_once(move |laid: Laid| {
                laid.listed(bearer).map(|(laid, _)| laid.spell).collect::<Vec<_>>()
            })
            .unwrap();
        assert_eq!(on, [SpellId::HASTE]);
    }
}
