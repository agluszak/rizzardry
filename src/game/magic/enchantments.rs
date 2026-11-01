//! Retained spell effects. Actor enchantments and party slots have distinct
//! replacement rules in `ConditionsAndEnchantments.cpp` and `W8EffectSlot::Activate`.
use crate::content::characters::Realm;
use crate::content::spells::SpellId;
use crate::game::characters::{Condition, Conditions, Lasting};
use bevy::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Enchantment {
    pub spell: SpellId,
    pub power: u32,
    pub boost: u32,
    pub turns: u32,
    /// Guardian Angel's remaining absorption, rolled once on acceptance.
    pub absorption: u32,
}

impl Enchantment {
    pub fn scaled(&self, amount: u32) -> u32 {
        amount + amount * self.boost / 100
    }

    /// Raise the effect by its boost and round; stronger effects never wrap to zero.
    pub fn adjusted(&self, amount: u32) -> u32 {
        ((self.boost + 100) * amount + 50) / 100
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Enchantments(pub Vec<Enchantment>);

/// Shared effects belong to the party, including members who join after casting.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct PartyEffects(pub Enchantments);

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

/// The offensive slots: clouds and burning that pulse on whoever they hang
/// over each round (retail `AdvanceTimedEffects`), in pulse order.
pub const OFFENSIVE: [SpellId; 5] = [
    SpellId::ACID_BOMB,
    SpellId::TOXIC_CLOUD,
    SpellId::DRAINING_CLOUD,
    SpellId::FIRESTORM,
    SpellId::DEATH_CLOUD,
];

pub fn combat_only(spell: SpellId) -> bool {
    matches!(
        spell,
        SpellId::BLESS
            | SpellId::ACID_BOMB
            | SpellId::ARMORMELT
            | SpellId::ELEMENT_SHIELD
            | SpellId::SOUL_SHIELD
            | SpellId::RING_OF_FIRE
            | SpellId::TOXIC_CLOUD
            | SpellId::DRAINING_CLOUD
            | SpellId::FIRESTORM
            | SpellId::DEATH_CLOUD
    )
}

impl Enchantments {
    pub fn get(&self, spell: SpellId) -> Option<&Enchantment> {
        self.0.iter().find(|e| e.spell == spell && e.turns > 0)
    }

    pub fn has(&self, spell: SpellId) -> bool {
        self.get(spell).is_some()
    }

    /// A character's effect is replaced only by a stronger one.
    pub fn accepts(&self, spell: SpellId, power: u32) -> bool {
        self.admits(spell, power, false)
    }

    /// A monster's effect is also refreshed at equal power (retail
    /// `ApplyMonsterCondition`).
    pub fn monster_accepts(&self, spell: SpellId, power: u32) -> bool {
        self.admits(spell, power, true)
    }

    fn admits(&self, spell: SpellId, power: u32, equal: bool) -> bool {
        !personal(spell)
            || self.get(spell).is_none_or(|e| power > e.power || equal && power == e.power)
    }

    pub fn apply(&mut self, effect: Enchantment) -> bool {
        self.install(effect, false)
    }

    pub fn apply_to_monster(&mut self, effect: Enchantment) -> bool {
        self.install(effect, true)
    }

    fn install(&mut self, effect: Enchantment, equal: bool) -> bool {
        if effect.turns == 0 || !self.admits(effect.spell, effect.power, equal) {
            return false;
        }
        self.0.retain(|e| e.spell != effect.spell);
        self.0.push(effect);
        true
    }

    /// Spend one reactive use and return the strength before spending it.
    pub fn spend_charge(&mut self, spell: SpellId) -> Option<Enchantment> {
        let index = self
            .0
            .iter()
            .position(|effect| effect.spell == spell && effect.turns > 0 && effect.power > 0)?;
        let effect = self.0[index];
        self.0[index].power -= 1;
        if self.0[index].power == 0 {
            self.0.remove(index);
        }
        Some(effect)
    }

    pub fn age(&mut self, ticks: u32) {
        self.0.retain_mut(|e| {
            if e.turns < 9999 {
                e.turns -= ticks.min(e.turns);
            }
            e.turns > 0
        });
    }

    pub fn end_combat(&mut self) {
        self.0.retain(|e| !combat_only(e.spell));
    }

    /// Retail keeps an exactly exhausted angel until the next nonzero hit.
    pub fn absorb(&mut self, amount: u32) -> u32 {
        let Some(e) = self.0.iter_mut().find(|e| e.spell == SpellId::GUARDIAN_ANGEL && e.turns > 0)
        else {
            return amount;
        };
        if amount <= e.absorption {
            e.absorption -= amount;
            return 0;
        }
        let remaining = amount - e.absorption;
        self.0.retain(|e| e.spell != SpellId::GUARDIAN_ANGEL);
        remaining
    }

    /// An enchantment's amount from its power, raised by its boost and rounded.
    fn adjusted(&self, spell: SpellId, amount: impl Fn(u32) -> u32) -> u32 {
        self.get(spell).map_or(0, |e| e.adjusted(amount(e.power)))
    }

    /// An enchantment's amount from its power, raised by its boost.
    fn scaled(&self, spell: SpellId, amount: impl Fn(u32) -> u32) -> u32 {
        self.get(spell).map_or(0, |e| e.scaled(amount(e.power)))
    }

    pub fn armour(&self) -> i32 {
        // RebuildPartyEffectBlock/RebuildMonsterDerivedStats subtract the
        // offensive slot's raw amount; Power Cast affects its duration.
        let melted = self.get(SpellId::ARMORMELT).map_or(0, |e| e.power);
        let raised = self.adjusted(SpellId::BLESS, |_| 2)
            + self.adjusted(SpellId::ARMORPLATE, |power| power.div_ceil(2))
            + self.scaled(SpellId::BODY_OF_STONE, |power| power);
        raised as i32 - melted as i32
    }

    /// Chameleon removes ten sight percentage points per adjusted power.
    /// The byte is read by retail monster sight only outside a fight.
    pub fn concealment(&self) -> u32 {
        self.adjusted(SpellId::CHAMELEON, |power| power)
    }

    pub fn hit_bonus(&self) -> i32 {
        let bonus = self.adjusted(SpellId::BLESS, |_| 2)
            + self.adjusted(SpellId::ENCHANTED_BLADE, |power| power.div_ceil(2));
        bonus as i32
    }

    pub fn resistance(&self, realm: Realm) -> i32 {
        let shield = if matches!(realm, Realm::Mental | Realm::Divine) {
            self.adjusted(SpellId::SOUL_SHIELD, |power| power * 10)
        } else {
            self.adjusted(SpellId::ELEMENT_SHIELD, |power| power * 7)
        };
        (self.adjusted(SpellId::MAGIC_SCREEN, |power| power * 4 + 7) + shield) as i32
    }

    pub fn missile_deflection(&self) -> u32 {
        self.adjusted(SpellId::MISSILE_SHIELD, |power| (power + 5) * 5)
    }

    pub fn deflects(
        &self,
        mode: crate::content::items::AttackMode,
        rng: &mut crate::game::rng::GameRng,
    ) -> bool {
        let chance = self.missile_deflection();
        matches!(
            mode,
            crate::content::items::AttackMode::Shoot | crate::content::items::AttackMode::Throw
        ) && chance > 0
            && rng.below(100) < chance
    }

    /// Slow's turns left once they have cancelled Haste's.
    fn counter_slow(&mut self, turns: u32) -> u32 {
        let Some(haste) = self.0.iter_mut().find(|e| e.spell == SpellId::HASTE) else {
            return turns;
        };
        let cancelled = turns.min(haste.turns);
        haste.turns -= cancelled;
        self.0.retain(|e| e.turns > 0);
        turns - cancelled
    }

    pub fn attribute_bonus(&self, attribute: crate::content::characters::Attribute) -> u32 {
        let haste = if attribute == crate::content::characters::Attribute::Speed {
            self.scaled(SpellId::HASTE, |power| power * 10)
        } else {
            0
        };
        self.scaled(SpellId::SUPERMAN, |power| power * 5) + haste
    }

    pub fn damage_reduction(&self) -> i32 {
        self.scaled(SpellId::BODY_OF_STONE, |power| power * 8).min(100) as i32
    }
}

/// Slow a party member or a monster: its turns first cancel Haste's, and only
/// what is left of them slows; an indefinite slowing ends Haste outright
/// (retail `InflictCharacterCondition`, `InflictMonsterCondition`).
pub fn slow(conditions: &mut Conditions, enchantments: &mut Enchantments, lasting: Lasting) {
    let lasting = match lasting {
        Lasting::Rounds(turns) => match enchantments.counter_slow(turns) {
            0 => return,
            left => Lasting::Rounds(left),
        },
        Lasting::Indefinite => {
            enchantments.0.retain(|e| e.spell != SpellId::HASTE);
            lasting
        }
    };
    conditions.inflict(Condition::Slowed, lasting);
}

/// Haste a party member or a monster: its turns first cancel Slow's, and only
/// what is left of them hastes. Whether it changed anything.
pub fn haste(
    conditions: &mut Conditions,
    enchantments: &mut Enchantments,
    effect: Enchantment,
    monster: bool,
) -> bool {
    let turns = match conditions.lasting_mut(Condition::Slowed) {
        None => effect.turns,
        Some(Lasting::Indefinite) => 0,
        Some(Lasting::Rounds(left)) => {
            let cancelled = effect.turns.min(*left);
            *left -= cancelled;
            if *left == 0 {
                conditions.lift(Condition::Slowed);
            }
            effect.turns - cancelled
        }
    };
    let effect = Enchantment { turns, ..effect };
    turns == 0
        || if monster { enchantments.apply_to_monster(effect) } else { enchantments.apply(effect) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn effect(spell: SpellId, power: u32, turns: u32) -> Enchantment {
        Enchantment { spell, power, turns, boost: 0, absorption: 12 }
    }
    #[test]
    fn armormelt_penalty_stacks_with_protection_and_expires() {
        let mut effects = Enchantments::default();
        effects.apply(Enchantment {
            spell: SpellId::ARMORPLATE,
            power: 4,
            boost: 0,
            turns: 4,
            absorption: 0,
        });
        effects.apply(Enchantment {
            spell: SpellId::ARMORMELT,
            power: 3,
            boost: 100,
            turns: 1,
            absorption: 0,
        });
        assert_eq!(effects.armour(), -1);
        effects.age(1);
        assert_eq!(effects.armour(), 2);
        effects.apply(Enchantment {
            spell: SpellId::ARMORMELT,
            power: 7,
            boost: 0,
            turns: 9,
            absorption: 0,
        });
        effects.end_combat();
        assert_eq!(effects.armour(), 2);
    }

    #[test]
    fn actors_require_stronger_power_but_party_slots_replace() {
        let boosted = Enchantment { boost: 100, ..effect(SpellId::ARMORPLATE, 7, 10) };
        assert_eq!(boosted.adjusted(200), 400);
        let mut effects = Enchantments::default();
        assert!(effects.apply(effect(SpellId::GUARDIAN_ANGEL, 3, 4)));
        assert_eq!(effects.absorb(5), 0);
        assert!(!effects.apply(effect(SpellId::GUARDIAN_ANGEL, 3, 20)));
        assert_eq!(effects.get(SpellId::GUARDIAN_ANGEL).unwrap().absorption, 7);
        assert_eq!(effects.get(SpellId::GUARDIAN_ANGEL).unwrap().turns, 4);
        assert!(effects.apply(effect(SpellId::GUARDIAN_ANGEL, 4, 2)));
        assert_eq!(effects.absorb(14), 2);
        assert!(effects.get(SpellId::GUARDIAN_ANGEL).is_none());
        effects.apply(effect(SpellId::ARMORPLATE, 7, 10));
        effects.apply(effect(SpellId::ARMORPLATE, 1, 2));
        assert_eq!(effects.armour(), 1);
        effects.age(2);
        assert!(effects.0.is_empty());
    }
    #[test]
    fn monsters_refresh_at_equal_power() {
        let mut effects = Enchantments::default();
        assert!(effects.apply_to_monster(effect(SpellId::HASTE, 3, 2)));
        assert!(effects.apply_to_monster(effect(SpellId::HASTE, 3, 9)));
        assert_eq!(effects.get(SpellId::HASTE).unwrap().turns, 9);
        assert!(!effects.apply_to_monster(effect(SpellId::HASTE, 2, 20)));
        assert!(!effects.apply(effect(SpellId::HASTE, 3, 20)));
    }
    #[test]
    fn the_boost_raises_party_and_combat_slots() {
        let mut effects = Enchantments::default();
        let boosted = |spell, power| Enchantment { boost: 50, ..effect(spell, power, 5) };
        effects.apply(boosted(SpellId::ARMORPLATE, 5));
        effects.apply(boosted(SpellId::ENCHANTED_BLADE, 5));
        effects.apply(boosted(SpellId::MISSILE_SHIELD, 1));
        effects.apply(boosted(SpellId::MAGIC_SCREEN, 1));
        assert_eq!(effects.armour(), 5, "(5 + 1) / 2 raised by half");
        assert_eq!(effects.hit_bonus(), 5);
        assert_eq!(effects.missile_deflection(), 45);
        assert_eq!(effects.resistance(Realm::Fire), 17, "11 raised by half, rounded");
    }
    #[test]
    fn stats_and_protections_expire_without_changing_the_profile() {
        use crate::content::characters::Attribute;
        use crate::game::characters::{Character, Gender, Profession, Race};
        let profile = Character::new("Mage", Race::Human, Gender::Male, Profession::Mage);
        let mut effects = Enchantments::default();
        effects.apply(effect(SpellId::HASTE, 3, 5));
        effects.apply(effect(SpellId::SUPERMAN, 2, 2));
        assert_eq!(
            profile.stats(&effects).attribute(Attribute::Speed),
            profile.base_stats().attribute(Attribute::Speed) + 40
        );
        effects.age(2);
        assert_eq!(
            profile.stats(&effects).attribute(Attribute::Speed),
            profile.base_stats().attribute(Attribute::Speed) + 30
        );
        assert_eq!(effects.counter_slow(8), 5);
        assert!(effects.0.is_empty());
        effects.apply(effect(SpellId::BODY_OF_STONE, 5, 2));
        assert_eq!(effects.damage_reduction(), 40);
        effects.apply(effect(SpellId::MAGIC_SCREEN, 4, 10));
        effects.apply(effect(SpellId::ELEMENT_SHIELD, 2, 5));
        assert_eq!(effects.resistance(Realm::Fire), 37);
        assert_eq!(effects.resistance(Realm::Mental), 23);
        effects.end_combat();
        assert_eq!(effects.resistance(Realm::Fire), 23);
        assert!(effects.has(SpellId::BODY_OF_STONE));
    }
}
