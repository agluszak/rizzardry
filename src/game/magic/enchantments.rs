//! Retained spell effects. Actor enchantments and party slots have distinct
//! replacement rules in `ConditionsAndEnchantments.cpp` and `W8EffectSlot::Activate`.
use crate::content::characters::Realm;
use crate::content::spells::SpellId;
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
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Enchantments(pub Vec<Enchantment>);

/// Shared effects belong to the party, including members who join after casting.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct PartyEffects(pub Enchantments);

pub fn personal(spell: SpellId) -> bool {
    matches!(spell.0, 19 | 21 | 27 | 54 | 56 | 61 | 65)
}

pub fn combat_only(spell: SpellId) -> bool {
    matches!(spell.0, 2 | 48 | 49 | 53 | 59 | 62 | 76 | 80 | 81 | 93)
}

impl Enchantments {
    pub fn get(&self, spell: u8) -> Option<&Enchantment> {
        self.0.iter().find(|e| e.spell.0 == spell && e.turns > 0)
    }

    pub fn accepts(&self, spell: SpellId, power: u8) -> bool {
        !personal(spell) || self.get(spell.0).is_none_or(|e| power > e.power)
    }

    pub fn apply(&mut self, effect: Enchantment) -> bool {
        if effect.turns == 0 || !self.accepts(effect.spell, effect.power) {
            return false;
        }
        self.0.retain(|e| e.spell != effect.spell);
        self.0.push(effect);
        true
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
        let Some(e) = self.0.iter_mut().find(|e| e.spell.0 == 21 && e.turns > 0) else {
            return amount;
        };
        if amount <= e.absorption {
            e.absorption -= amount;
            return 0;
        }
        let remaining = amount - e.absorption;
        self.0.retain(|e| e.spell.0 != 21);
        remaining
    }

    pub fn armour(&self) -> i32 {
        self.get(2).map_or(0, |e| e.scaled(2)) as i32
            + self.get(32).map_or(0, |e| u32::from(e.power).div_ceil(2)) as i32
            + self.get(65).map_or(0, |e| e.scaled(u32::from(e.power))) as i32
    }

    pub fn hit_bonus(&self) -> i32 {
        self.get(2).map_or(0, |e| e.scaled(2)) as i32
            + self.get(20).map_or(0, |e| u32::from(e.power).div_ceil(2)) as i32
    }

    pub fn resistance(&self, realm: Realm) -> i32 {
        let shield = if matches!(realm, Realm::Mental | Realm::Divine) { 59 } else { 53 };
        self.get(40).map_or(0, |e| u32::from(e.power) * 4 + 7) as i32
            + self.get(shield).map_or(0, |e| u32::from(e.power) * if shield == 59 { 10 } else { 7 })
                as i32
    }

    pub fn missile_deflection(&self) -> u32 {
        self.get(26).map_or(0, |e| (u32::from(e.power) + 5) * 5)
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

    pub fn counter_slow(&mut self, turns: u32) -> u32 {
        let Some(haste) = self.0.iter_mut().find(|e| e.spell.0 == 56) else { return turns };
        let remaining = turns.saturating_sub(haste.turns);
        haste.turns = haste.turns.saturating_sub(turns);
        self.0.retain(|e| e.turns > 0);
        remaining
    }

    pub fn attribute_bonus(&self, attribute: crate::content::characters::Attribute) -> u32 {
        self.get(61).map_or(0, |s| s.scaled(u32::from(s.power) * 5))
            + if attribute == crate::content::characters::Attribute::Speed {
                self.get(56).map_or(0, |s| s.scaled(u32::from(s.power) * 10))
            } else {
                0
            }
    }

    pub fn damage_reduction(&self) -> u32 {
        self.get(65).map_or(0, |e| e.scaled(u32::from(e.power) * 8)).min(100)
    }

    pub fn physical_damage(&self, amount: u32) -> u32 {
        let reduction = self.damage_reduction();
        (amount * (100 - reduction) + 50) / 100
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn effect(spell: u8, power: u8, turns: u32) -> Enchantment {
        Enchantment { spell: SpellId(spell), power, turns, boost: 0, absorption: 12 }
    }
    #[test]
    fn actors_require_stronger_power_but_party_slots_replace() {
        let mut effects = Enchantments::default();
        assert!(effects.apply(effect(21, 3, 4)));
        assert_eq!(effects.absorb(5), 0);
        assert!(!effects.apply(effect(21, 3, 20)));
        assert_eq!(effects.get(21).unwrap().absorption, 7);
        assert_eq!(effects.get(21).unwrap().turns, 4);
        assert!(effects.apply(effect(21, 4, 2)));
        assert_eq!(effects.absorb(14), 2);
        assert!(effects.get(21).is_none());
        effects.apply(effect(32, 7, 10));
        effects.apply(effect(32, 1, 2));
        assert_eq!(effects.armour(), 1);
        effects.age(2);
        assert!(effects.0.is_empty());
    }
    #[test]
    fn stats_and_protections_expire_without_changing_the_profile() {
        use crate::content::characters::Attribute;
        use crate::game::characters::{Character, Gender, Profession, Race};
        let profile = Character::new("Mage", Race::Human, Gender::Male, Profession::Mage);
        let mut effects = Enchantments::default();
        effects.apply(effect(56, 3, 5));
        effects.apply(effect(61, 2, 2));
        assert_eq!(
            profile.stats(&effects).attribute(Attribute::Speed),
            profile.attribute(Attribute::Speed) + 40
        );
        effects.age(2);
        assert_eq!(
            profile.stats(&effects).attribute(Attribute::Speed),
            profile.attribute(Attribute::Speed) + 30
        );
        assert_eq!(effects.counter_slow(8), 5);
        assert!(effects.0.is_empty());
        effects.apply(effect(65, 5, 2));
        assert_eq!(effects.physical_damage(10), 6);
        effects.apply(effect(40, 4, 10));
        effects.apply(effect(53, 2, 5));
        assert_eq!(effects.resistance(Realm::Fire), 37);
        assert_eq!(effects.resistance(Realm::Mental), 23);
        effects.end_combat();
        assert_eq!(effects.resistance(Realm::Fire), 23);
        assert!(effects.get(65).is_some());
    }
}
