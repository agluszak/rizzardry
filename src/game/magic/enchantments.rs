//! Retained spell effects. Actor enchantments and party slots have distinct
//! replacement rules in `ConditionsAndEnchantments.cpp` and `W8EffectSlot::Activate`.
use crate::content::characters::Realm;
use crate::content::spells::SpellId;
use crate::game::characters::{Condition, Conditions, Lasting};
use bevy::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Enchantment {
    pub spell: SpellId,
    pub power: u8,
    pub boost: u32,
    pub turns: u32,
    /// Guardian Angel's remaining absorption, rolled once on acceptance.
    pub absorption: u32,
}

impl Enchantment {
    pub fn scaled(&self, amount: u32) -> u32 {
        amount + amount * self.boost / 100
    }

    /// A party or combat slot's figure raised by the boost, rounded and kept
    /// to a byte (retail `AdjustByteByPercent`).
    pub fn adjusted(&self, amount: u32) -> u32 {
        (((self.boost + 100) * u32::from(amount as u8) + 50) / 100) as u8 as u32
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Enchantments(pub Vec<Enchantment>);

/// Shared effects belong to the party, including members who join after casting.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct PartyEffects(pub Enchantments);

impl std::ops::Deref for PartyEffects {
    type Target = Enchantments;
    fn deref(&self) -> &Enchantments {
        &self.0
    }
}

impl std::ops::DerefMut for PartyEffects {
    fn deref_mut(&mut self) -> &mut Enchantments {
        &mut self.0
    }
}

pub fn personal(spell: SpellId) -> bool {
    matches!(
        spell,
        crate::content::spells::SpellId::DRACON_BREATH
            | crate::content::spells::SpellId::GUARDIAN_ANGEL
            | crate::content::spells::SpellId::RAZOR_CLOAK
            | crate::content::spells::SpellId::EYE_FOR_AN_EYE
            | crate::content::spells::SpellId::HASTE
            | crate::content::spells::SpellId::SUPERMAN
            | crate::content::spells::SpellId::BODY_OF_STONE
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
        crate::content::spells::SpellId::BLESS
            | crate::content::spells::SpellId::ACID_BOMB
            | crate::content::spells::SpellId::ARMORMELT
            | crate::content::spells::SpellId::ELEMENT_SHIELD
            | crate::content::spells::SpellId::SOUL_SHIELD
            | crate::content::spells::SpellId::RING_OF_FIRE
            | crate::content::spells::SpellId::TOXIC_CLOUD
            | crate::content::spells::SpellId::DRAINING_CLOUD
            | crate::content::spells::SpellId::FIRESTORM
            | crate::content::spells::SpellId::DEATH_CLOUD
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
    pub fn accepts(&self, spell: SpellId, power: u8) -> bool {
        self.admits(spell, power, false)
    }

    /// A monster's effect is also refreshed at equal power (retail
    /// `ApplyMonsterCondition`).
    pub fn monster_accepts(&self, spell: SpellId, power: u8) -> bool {
        self.admits(spell, power, true)
    }

    fn admits(&self, spell: SpellId, power: u8, equal: bool) -> bool {
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
                e.turns = e.turns.saturating_sub(ticks);
            }
            e.turns > 0
        });
    }

    pub fn end_combat(&mut self) {
        self.0.retain(|e| !combat_only(e.spell));
    }

    /// Retail keeps an exactly exhausted angel until the next nonzero hit.
    pub fn absorb(&mut self, amount: u32) -> u32 {
        let Some(e) = self
            .0
            .iter_mut()
            .find(|e| e.spell == crate::content::spells::SpellId::GUARDIAN_ANGEL && e.turns > 0)
        else {
            return amount;
        };
        if amount <= e.absorption {
            e.absorption -= amount;
            return 0;
        }
        let remaining = amount - e.absorption;
        self.0.retain(|e| e.spell != crate::content::spells::SpellId::GUARDIAN_ANGEL);
        remaining
    }

    pub fn armour(&self) -> i32 {
        // RebuildPartyEffectBlock/RebuildMonsterDerivedStats subtract the
        // offensive slot's raw amount; Power Cast affects its duration.
        let melted = self.get(SpellId::ARMORMELT).map_or(0, |effect| i32::from(effect.power));
        self.get(crate::content::spells::SpellId::BLESS).map_or(0, |e| e.adjusted(2)) as i32
            + self
                .get(crate::content::spells::SpellId::ARMORPLATE)
                .map_or(0, |e| e.adjusted(u32::from(e.power).div_ceil(2))) as i32
            + self
                .get(crate::content::spells::SpellId::BODY_OF_STONE)
                .map_or(0, |e| e.scaled(u32::from(e.power))) as i32
            - melted
    }

    /// Chameleon removes ten sight percentage points per adjusted power.
    /// The byte is read by retail monster sight only outside a fight.
    pub fn concealment(&self) -> u32 {
        self.get(SpellId::CHAMELEON).map_or(0, |effect| effect.adjusted(u32::from(effect.power)))
    }

    pub fn hit_bonus(&self) -> i32 {
        self.get(crate::content::spells::SpellId::BLESS).map_or(0, |e| e.adjusted(2)) as i32
            + self
                .get(crate::content::spells::SpellId::ENCHANTED_BLADE)
                .map_or(0, |e| e.adjusted(u32::from(e.power).div_ceil(2))) as i32
    }

    pub fn resistance(&self, realm: Realm) -> i32 {
        let shield = if matches!(realm, Realm::Mental | Realm::Divine) {
            crate::content::spells::SpellId::SOUL_SHIELD
        } else {
            crate::content::spells::SpellId::ELEMENT_SHIELD
        };
        self.get(crate::content::spells::SpellId::MAGIC_SCREEN)
            .map_or(0, |e| e.adjusted(u32::from(e.power) * 4 + 7)) as i32
            + self.get(shield).map_or(0, |e| {
                e.adjusted(
                    u32::from(e.power)
                        * if shield == crate::content::spells::SpellId::SOUL_SHIELD {
                            10
                        } else {
                            7
                        },
                )
            }) as i32
    }

    pub fn missile_deflection(&self) -> u32 {
        self.get(crate::content::spells::SpellId::MISSILE_SHIELD)
            .map_or(0, |e| e.adjusted((u32::from(e.power) + 5) * 5))
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
        let remaining = turns.saturating_sub(haste.turns);
        haste.turns = haste.turns.saturating_sub(turns);
        self.0.retain(|e| e.turns > 0);
        remaining
    }

    pub fn attribute_bonus(&self, attribute: crate::content::characters::Attribute) -> u32 {
        self.get(crate::content::spells::SpellId::SUPERMAN)
            .map_or(0, |s| s.scaled(u32::from(s.power) * 5))
            + if attribute == crate::content::characters::Attribute::Speed {
                self.get(crate::content::spells::SpellId::HASTE)
                    .map_or(0, |s| s.scaled(u32::from(s.power) * 10))
            } else {
                0
            }
    }

    pub fn damage_reduction(&self) -> u32 {
        self.get(crate::content::spells::SpellId::BODY_OF_STONE)
            .map_or(0, |e| e.scaled(u32::from(e.power) * 8))
            .min(100)
    }
}

/// Slow a party member or a monster: its turns first cancel Haste's, and only
/// what is left of them slows; an indefinite slowing ends Haste outright
/// (retail `InflictCharacterCondition`, `InflictMonsterCondition`).
pub fn slow(conditions: &mut Conditions, enchantments: &mut Enchantments, lasting: Lasting) {
    let lasting = match lasting {
        Lasting::Rounds(turns) => match enchantments.counter_slow(u32::from(turns)) {
            0 => return,
            left => Lasting::Rounds(left as u16),
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
            let remaining = effect.turns.saturating_sub(u32::from(*left));
            *left = u32::from(*left).saturating_sub(effect.turns) as u16;
            if *left == 0 {
                conditions.lift(Condition::Slowed);
            }
            remaining
        }
    };
    let effect = Enchantment { turns, ..effect };
    turns == 0
        || if monster { enchantments.apply_to_monster(effect) } else { enchantments.apply(effect) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn effect(spell: SpellId, power: u8, turns: u32) -> Enchantment {
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
        let mut effects = Enchantments::default();
        assert!(effects.apply(effect(crate::content::spells::SpellId::GUARDIAN_ANGEL, 3, 4)));
        assert_eq!(effects.absorb(5), 0);
        assert!(!effects.apply(effect(crate::content::spells::SpellId::GUARDIAN_ANGEL, 3, 20)));
        assert_eq!(
            effects.get(crate::content::spells::SpellId::GUARDIAN_ANGEL).unwrap().absorption,
            7
        );
        assert_eq!(effects.get(crate::content::spells::SpellId::GUARDIAN_ANGEL).unwrap().turns, 4);
        assert!(effects.apply(effect(crate::content::spells::SpellId::GUARDIAN_ANGEL, 4, 2)));
        assert_eq!(effects.absorb(14), 2);
        assert!(effects.get(crate::content::spells::SpellId::GUARDIAN_ANGEL).is_none());
        effects.apply(effect(crate::content::spells::SpellId::ARMORPLATE, 7, 10));
        effects.apply(effect(crate::content::spells::SpellId::ARMORPLATE, 1, 2));
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
        effects.apply(effect(crate::content::spells::SpellId::HASTE, 3, 5));
        effects.apply(effect(crate::content::spells::SpellId::SUPERMAN, 2, 2));
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
        effects.apply(effect(crate::content::spells::SpellId::BODY_OF_STONE, 5, 2));
        assert_eq!(effects.damage_reduction(), 40);
        effects.apply(effect(crate::content::spells::SpellId::MAGIC_SCREEN, 4, 10));
        effects.apply(effect(crate::content::spells::SpellId::ELEMENT_SHIELD, 2, 5));
        assert_eq!(effects.resistance(Realm::Fire), 37);
        assert_eq!(effects.resistance(Realm::Mental), 23);
        effects.end_combat();
        assert_eq!(effects.resistance(Realm::Fire), 23);
        assert!(effects.has(crate::content::spells::SpellId::BODY_OF_STONE));
    }
}
