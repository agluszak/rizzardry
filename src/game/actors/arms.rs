//! A member's arms and armour (`docs/rules/arms.md`): what they strike with
//! and how well they are guarded, from what is in their two hands and on
//! their body. The formulas are the original's (`CalcAttacks`,
//! `CalcArmorClasses`).

use bevy::ecs::query::QueryData;
use bevy::prelude::*;
use enum_map::EnumMap;
use enumset::EnumSet;

use super::{Arm, Attacks, Attributes};
use crate::content::characters::{Attribute, BodyPart, Skill, Trait};
use crate::content::items::{AttackEffect, AttackEffects, AttackMode, Dice};
use crate::content::species::{MonsterKind, Reach};
use crate::game::characters::{Character, Load};
use crate::game::inventory::{EquipSlot, WeaponSlot};
use crate::game::items::{
    Charges, Fit, Launcher, Missile, Protection, Strength, Weapon, Weight, partners,
};

/// What the rules of a hand read of the item in it.
#[derive(QueryData)]
pub struct Held {
    entity: Entity,
    fit: Fit,
    weapon: Option<&'static Weapon>,
    launcher: Option<&'static Launcher>,
    missile: Option<&'static Missile>,
    charges: Option<&'static Charges>,
    weight: &'static Weight,
}

/// What one hand of the set in use does in a fight.
#[derive(Clone, Copy, Debug)]
enum Hand<'a> {
    /// Empty, and fighting with Martial Arts.
    Fist,
    /// A weapon, which strikes by its own components; a launcher with its
    /// missile in the other hand strikes with that too.
    Weapon {
        item: Entity,
        weapon: &'a Weapon,
        launcher: Option<&'a Launcher>,
        /// The ammunition in the other hand, which adds to the rating to
        /// hit; a launcher shoots it.
        missile: Option<&'a Missile>,
        weight: u16,
    },
    /// A shield, a missile, a launcher without its missile, a gun out of
    /// shots, anything else: it does not strike.
    Idle,
}

impl<'a> Hand<'a> {
    fn strikes(&self) -> bool {
        !matches!(self, Hand::Idle)
    }

    /// The skill it fights with: its weapon's, Martial Arts for a fist.
    fn skill(&self) -> Skill {
        match self {
            Hand::Weapon { weapon, .. } => weapon.skill,
            _ => Skill::MartialArts,
        }
    }

    /// Its blows are ranged combat, not close combat.
    fn ranged(&self) -> bool {
        matches!(self, Hand::Weapon { weapon, .. } if weapon.ranged)
    }

    /// What a launcher shoots: the ammunition in the other hand.
    fn shot(&self) -> Option<&'a Missile> {
        match self {
            Hand::Weapon { launcher, missile, .. } => (*launcher).and(*missile),
            _ => None,
        }
    }
}

/// Unarmed damage by Martial Arts level / 11 (retail
/// `g_unarmed_damage_dice`).
fn unarmed_dice(martial_arts: u32) -> Dice {
    const DICE: [(i32, i32, i32); 12] = [
        (0, 1, 2),
        (0, 1, 3),
        (0, 2, 2),
        (0, 2, 3),
        (0, 2, 4),
        (0, 3, 3),
        (1, 3, 3),
        (2, 3, 3),
        (0, 3, 5),
        (0, 4, 4),
        (2, 4, 4),
        (4, 4, 4),
    ];
    let (base, count, sides) = DICE[(martial_arts as usize / 11).min(11)];
    Dice { base, count, sides }
}

/// The two hands of the weapon set in use, and what they make of a member's
/// attributes and skills (retail `CalcAttacks`). An empty hand fights with
/// Martial Arts; the off hand strikes only beside a main hand of the same
/// kind (both close, or both at range), never when the main hand holds an
/// omnigun, and an empty off hand does not strike beside an armed main hand.
pub struct Hands<'a> {
    hands: EnumMap<WeaponSlot, Hand<'a>>,
    /// Both hands hold weapons and both strike.
    dual_wielding: bool,
    /// What a punch from the main hand rolls when the off hand holds
    /// something, whether or not it strikes: the item's own dice, none for
    /// a shield. An empty off hand punches with its fist's.
    held_dice: Option<Dice>,
}

impl<'a> Hands<'a> {
    /// The hands of a member, given what they wear in each slot.
    pub fn of<'s>(worn: impl Fn(EquipSlot) -> Option<HeldItem<'a, 's>>) -> Self {
        let held = EnumMap::from_fn(|slot: WeaponSlot| worn(slot.equip()));
        let mut hands = EnumMap::from_fn(|slot| hand(slot, &held));
        let [main, off] = [WeaponSlot::Primary, WeaponSlot::Secondary].map(|slot| &held[slot]);
        // Close or ranged combat, or neither for what cannot strike.
        let combat = |held: &Option<HeldItem>| match held {
            None => Some(false),
            Some(item) => item.weapon.map(|weapon| weapon.ranged),
        };
        let omnigun =
            main.as_ref().is_some_and(|main| main.launcher.is_some_and(|l| l.fires_weapons));
        if combat(main) != combat(off) || omnigun || (off.is_none() && main.is_some()) {
            hands[WeaponSlot::Secondary] = Hand::Idle;
        }
        let dual_wielding = hands.values().all(|hand| matches!(hand, Hand::Weapon { .. }));
        let held_dice = off.as_ref().map(|item| {
            item.weapon
                .map(|weapon| weapon.dice)
                .or_else(|| item.missile.map(|missile| missile.dice))
                .unwrap_or_default()
        });
        Self { hands, dual_wielding, held_dice }
    }

    /// What a member strikes with: an arm for each hand that strikes, and
    /// how well they press a blow past a guard (retail
    /// `GetCharacterProtectionScore`). The rest of what they fight with is
    /// theirs to add. `to_hit_bonus` is what spells add to the rating.
    pub fn attacks(&self, c: &Character, a: &Attributes, load: Load, to_hit_bonus: i32) -> Attacks {
        let primary = WeaponSlot::Primary;
        Attacks {
            arms: [WeaponSlot::Primary, WeaponSlot::Secondary]
                .into_iter()
                .filter(|&slot| self.hands[slot].strikes())
                .map(|slot| self.arm(c, a, load, to_hit_bonus, slot))
                .collect(),
            press: a[Attribute::Speed] as i32
                + self.to_hit(c, a, load, primary) * 5
                + self.rating(c, a, primary)
                + to_hit_bonus * 5,
            tires_little: self.hands[primary].skill() == Skill::ModernWeapon,
            ..Attacks::default()
        }
    }

    /// The arm of a hand that strikes, as the member's gear and sheet make it.
    fn arm(
        &self,
        c: &Character,
        a: &Attributes,
        load: Load,
        to_hit_bonus: i32,
        slot: WeaponSlot,
    ) -> Arm {
        let hand = &self.hands[slot];
        let skill = hand.skill();
        let reach = match hand {
            Hand::Weapon { weapon, .. } => weapon.reach,
            _ => Reach::Touch,
        };
        Arm {
            slot,
            weapon: match hand {
                Hand::Weapon { item, .. } => Some(*item),
                _ => None,
            },
            reach,
            modes: self.modes(c, slot),
            dice: self.dice(c, slot),
            // A punch rolls the off hand's dice: what is held there as its
            // record gives them, else the fist's (retail
            // `ResolveCharacterAttackDamage`).
            punch: self.held_dice.unwrap_or_else(|| self.dice(c, WeaponSlot::Secondary)),
            kick: self.dice(c, WeaponSlot::Primary),
            rating: self.rating(c, a, slot) + (self.to_hit(c, a, load, slot) + to_hit_bonus) * 5,
            percent: self.damage_percent(a, slot),
            effects: self.effects(a, slot),
            slays: self.slays(slot),
            attacks: self.attacks_a_round(c, a, load, slot),
            initiative: self.initiative(c, slot),
            swings: self.swings(c, a, load, slot),
            extra_swings: match hand {
                Hand::Weapon { weapon, .. } => weapon.extra_swings,
                _ => 0,
            },
            ranged: hand.ranged(),
            loaded: hand.shot().map(|missile| missile.modes),
            sure_throw: self.sure_throw(c, slot),
            critical: self.critical_skill(c, slot).map(|skill| c.skill(skill)),
            practises: match (hand.ranged(), c.has_trait(Trait::ThrownCriticals)) {
                (false, _) => &[Skill::CriticalStrike, Skill::PowerStrike],
                (true, true) => &[Skill::EagleEye, Skill::CriticalStrike],
                (true, false) => &[Skill::EagleEye],
            },
            weight: match hand {
                Hand::Weapon { weight, .. } => Some(*weight),
                _ => None,
            },
            tires: true,
            modern: skill == Skill::ModernWeapon,
            fatigue_pierces: skill != Skill::ModernWeapon,
            sword: skill == Skill::Sword,
            ambushes: c.has_trait(Trait::Backstab)
                && reach <= Reach::Touch
                && matches!(skill, Skill::Sword | Skill::Dagger),
            word: None,
            lands_on_party: None,
        }
    }

    /// The hand's dice: what a launcher shoots, a weapon's own, or a fist by
    /// Martial Arts, two more in the main hand.
    pub fn dice(&self, c: &Character, slot: WeaponSlot) -> Dice {
        match self.hands[slot] {
            Hand::Fist => {
                let mut dice = unarmed_dice(c.skill(Skill::MartialArts));
                if slot == WeaponSlot::Primary {
                    dice.base += 2;
                }
                dice
            }
            Hand::Weapon { weapon, .. } => self.hands[slot].shot().map_or(weapon.dice, |m| m.dice),
            Hand::Idle => Dice::default(),
        }
    }

    /// The ways the hand strikes: its weapon's, or a fist's punch, and a
    /// kick past Martial Arts 4.
    fn modes(&self, c: &Character, slot: WeaponSlot) -> EnumSet<AttackMode> {
        match self.hands[slot] {
            Hand::Weapon { weapon, .. } => weapon.modes,
            _ if c.skill(Skill::MartialArts) > 4 => AttackMode::Punch | AttackMode::Kick,
            _ => EnumSet::only(AttackMode::Punch),
        }
    }

    /// What its blows may do besides damage: the weapon's effects with its
    /// ammunition's, or a strong fist's knockout chance.
    fn effects(&self, a: &Attributes, slot: WeaponSlot) -> AttackEffects {
        match self.hands[slot] {
            Hand::Weapon { weapon, .. } => self.hands[slot]
                .shot()
                .map_or(weapon.effects, |missile| weapon.effects.with(&missile.effects)),
            _ => {
                let mut effects = AttackEffects::default();
                let strength = a[Attribute::Strength] as i32;
                if strength > 49 {
                    effects.chances[AttackEffect::KnockOut] = ((strength - 50) / 5) as u32;
                }
                effects
            }
        }
    }

    /// The monster kinds its weapon and ammunition slay.
    fn slays(&self, slot: WeaponSlot) -> Vec<MonsterKind> {
        let weapon = match self.hands[slot] {
            Hand::Weapon { weapon, .. } => weapon.slays,
            _ => None,
        };
        [weapon, self.hands[slot].shot().and_then(|missile| missile.slays)]
            .into_iter()
            .flatten()
            .collect()
    }

    /// The combat and weapon skills the hand fights with, combined (retail
    /// `CalcAttacks`): weighed with the other hand's weapon skill when it
    /// strikes with a weapon too, and Dual Weapons when both do.
    fn skill(&self, c: &Character, slot: WeaponSlot) -> i32 {
        let hand = &self.hands[slot];
        let combat = if hand.ranged() { Skill::RangedCombat } else { Skill::CloseCombat };
        let mut score = ((c.skill(combat) + c.skill(hand.skill()) * 2) * 2 / 3) as i32;
        let mut divisor = 20;
        if let Hand::Weapon { weapon, .. } = self.hands[slot.other()] {
            score += c.skill(weapon.skill) as i32 >> 1;
            divisor = 25;
        }
        if self.dual_wielding {
            score += c.skill(Skill::DualWeapons) as i32;
            divisor += 10;
        }
        score * 10 / divisor
    }

    /// What wielding two weapons costs the hand: 10 a hand and a quarter of
    /// the Dual Weapons lacking.
    fn dual_penalty(&self, c: &Character, slot: WeaponSlot) -> i32 {
        if !self.dual_wielding {
            return 0;
        }
        let number = match slot {
            WeaponSlot::Primary => 1,
            WeaponSlot::Secondary => 2,
        };
        -10 * number - (100 - c.skill(Skill::DualWeapons) as i32) / 4
    }

    /// Modern weapons feel load half as much.
    fn halved(&self, slot: WeaponSlot, penalty: i32) -> i32 {
        if self.hands[slot].skill() == Skill::ModernWeapon { penalty / 2 } else { penalty }
    }

    /// The hand's attack rating (retail `attack_rating`); none when it
    /// cannot attack.
    fn rating(&self, c: &Character, a: &Attributes, slot: WeaponSlot) -> i32 {
        if !self.hands[slot].strikes() {
            return 0;
        }
        let dexterity = a[Attribute::Dexterity] as i32;
        (self.dual_penalty(c, slot) + dexterity / 2 + self.skill(c, slot) * 2 + c.physical_levels())
            / 3
            + 60
    }

    /// Attacks a round with the hand: one, a second past 50 and a third
    /// past 100 in the main hand, a second past 75 in the off hand; none
    /// when it cannot attack.
    pub fn attacks_a_round(
        &self,
        c: &Character,
        a: &Attributes,
        load: Load,
        slot: WeaponSlot,
    ) -> u32 {
        if !self.hands[slot].strikes() {
            return 0;
        }
        let [dexterity, speed] = [Attribute::Dexterity, Attribute::Speed].map(|x| a[x] as i32);
        let score = ((speed + dexterity) >> 1)
            + self.dual_penalty(c, slot)
            + c.physical_levels()
            + self.halved(slot, -load.attack_penalty())
            + self.skill(c, slot);
        let score = score / 3;
        match slot {
            WeaponSlot::Primary => 1 + u32::from(score > 49) + u32::from(score > 99),
            WeaponSlot::Secondary => 1 + u32::from(score > 74),
        }
    }

    /// The hand's initiative: its weapon's (retail `damage_bonus`), or a
    /// fist's from skill; none when it cannot attack.
    pub fn initiative(&self, c: &Character, slot: WeaponSlot) -> i32 {
        match self.hands[slot] {
            Hand::Idle => 0,
            Hand::Weapon { weapon, .. } => weapon.initiative,
            Hand::Fist => self.skill(c, slot) / 10,
        }
    }

    /// Most swings an attack with the hand makes: one, a second past 66
    /// and a third past 100, before what its weapon adds; a rocket launcher
    /// in the main hand makes one. None when it cannot attack.
    pub fn swings(&self, c: &Character, a: &Attributes, load: Load, slot: WeaponSlot) -> u32 {
        if !self.hands[slot].strikes() {
            return 0;
        }
        if matches!(self.hands[WeaponSlot::Primary], Hand::Weapon { weapon, .. } if weapon.single_swing)
        {
            return 1;
        }
        // Retail reads the initiative its previous pass left, which is this.
        let score = (a[Attribute::Speed] as i32
            + self.initiative(c, slot) * 10
            + self.dual_penalty(c, slot)
            + c.physical_levels()
            + self.halved(slot, -load.attack_penalty())
            + self.skill(c, slot))
            / 3;
        1 + u32::from(score > 66) + u32::from(score > 99)
    }

    /// How strength counts for the hand: not at all for guns and the like
    /// (none), by half for weapons only thrown, lashed or shot, and a
    /// `bonus` by half again in the off hand. The divisor.
    fn strength_divisor(&self, slot: WeaponSlot, bonus: bool) -> Option<i32> {
        let mut divisor = match self.hands[slot] {
            Hand::Weapon { weapon, .. } => match weapon.strength {
                Strength::Full => 1,
                Strength::Half => 2,
                Strength::None => return None,
            },
            _ => 1,
        };
        if bonus && slot == WeaponSlot::Secondary {
            divisor *= 2;
        }
        Some(divisor)
    }

    /// The sheet's "To Hit" (retail `to_hit`): the weapon's and its
    /// ammunition's bonus, strength, dexterity, senses, load, and Eagle Eye
    /// or Power Strike; none when the hand cannot attack.
    pub fn to_hit(&self, c: &Character, a: &Attributes, load: Load, slot: WeaponSlot) -> i32 {
        let hand = &self.hands[slot];
        if !hand.strikes() {
            return 0;
        }
        let [strength, dexterity, senses] =
            [Attribute::Strength, Attribute::Dexterity, Attribute::Senses].map(|x| a[x] as i32);
        let mut to_hit = 0;
        if let Hand::Weapon { weapon, missile, .. } = hand {
            to_hit += weapon.to_hit + missile.map_or(0, |missile| missile.to_hit);
        }
        if let Some(divisor) = self.strength_divisor(slot, strength > 50) {
            if strength < 50 {
                to_hit -= (50 - strength) / (divisor * 10);
            } else if strength > 50 {
                to_hit += (strength - 50) / (divisor * 10);
            }
        }
        to_hit += (dexterity - 50) / 10;
        if senses < 30 {
            to_hit -= (30 - senses) / 10;
        } else if senses > 70 {
            to_hit += (senses - 70) / 10;
        }
        let mut load_penalty = -load.penalty();
        if slot == WeaponSlot::Secondary {
            load_penalty = load_penalty * 3 / 2;
        }
        to_hit += self.halved(slot, load_penalty);
        if hand.ranged() && c.skill_available(Skill::EagleEye) {
            to_hit += c.skill(Skill::EagleEye) as i32 / 20 + 1;
        }
        if !hand.ranged() && c.skill_available(Skill::PowerStrike) {
            to_hit += c.skill(Skill::PowerStrike) as i32 / 20 + 1;
        }
        to_hit
    }

    /// The sheet's "To Damage (%)": a launcher's base, and strength; none
    /// when the hand cannot attack.
    pub fn damage_percent(&self, a: &Attributes, slot: WeaponSlot) -> i32 {
        if !self.hands[slot].strikes() {
            return 0;
        }
        let strength = a[Attribute::Strength] as i32;
        let mut percent = 0;
        if let Hand::Weapon { launcher: Some(launcher), .. } = self.hands[slot] {
            percent += launcher.damage;
        }
        if let Some(divisor) = self.strength_divisor(slot, strength > 50) {
            if strength < 50 {
                percent -= (50 - strength) / divisor;
            } else if strength > 50 {
                percent += (strength * 2 - 100) / divisor;
            }
        }
        percent
    }

    /// The skill whose training adds a kill chance to the hand's blows
    /// (retail `ResolveCharacterAttack`): Critical Strike for close combat,
    /// Ranged Combat for a Ranger's bows and slings, Critical Strike for a
    /// Ninja's throws.
    fn critical_skill(&self, c: &Character, slot: WeaponSlot) -> Option<Skill> {
        let hand = &self.hands[slot];
        let skill = hand.skill();
        let sling = matches!(hand, Hand::Weapon { weapon, .. } if weapon.sling);
        if !hand.ranged() {
            Some(Skill::CriticalStrike)
        } else if matches!(skill, Skill::ModernWeapon | Skill::Bow)
            || (skill == Skill::ThrowingAndSling && sling)
        {
            c.has_trait(Trait::RangedCriticals).then_some(Skill::RangedCombat)
        } else if skill == Skill::ThrowingAndSling && c.has_trait(Trait::ThrownCriticals) {
            Some(Skill::CriticalStrike)
        } else {
            None
        }
    }

    /// A Ninja's thrown weapons always get through armour when they strike
    /// (retail `W8_TRAIT_THROWN_CRITICALS` for ranged thrown weapons other
    /// than slings).
    fn sure_throw(&self, c: &Character, slot: WeaponSlot) -> bool {
        let hand = &self.hands[slot];
        c.has_trait(Trait::ThrownCriticals)
            && hand.ranged()
            && hand.skill() == Skill::ThrowingAndSling
            && !matches!(hand, Hand::Weapon { weapon, .. } if weapon.sling)
    }
}

/// What one hand does, from what is in it and in the other hand.
fn hand<'a>(slot: WeaponSlot, held: &EnumMap<WeaponSlot, Option<HeldItem<'a, '_>>>) -> Hand<'a> {
    let Some(item) = &held[slot] else { return Hand::Fist };
    let Some(weapon) = item.weapon else { return Hand::Idle };
    let other = held[slot.other()].as_ref();
    let omnigun = item.launcher.is_some_and(|launcher| launcher.fires_weapons);
    // What the other hand holds as ammunition: a missile that is not itself
    // a weapon, or what an omnigun in the main hand fires.
    let missile = other.and_then(|other| {
        other.missile.filter(|_| other.weapon.is_none() || (omnigun && slot == WeaponSlot::Primary))
    });
    if weapon.ranged {
        // A launcher cannot strike without its missile, nor a gun out of
        // shots; it keeps its combat skill all the same.
        let loaded = item.launcher.is_none_or(|_| {
            missile.is_some() && other.is_some_and(|other| partners(&item.fit, &other.fit))
        });
        let shots = !(weapon.shots && item.charges.is_some_and(|charges| charges.left == 0));
        if !(loaded && shots) {
            return Hand::Idle;
        }
    }
    Hand::Weapon {
        item: item.entity,
        weapon,
        launcher: item.launcher,
        missile,
        weight: item.weight.0,
    }
}

/// A member's total and per-location armour, without skills or conditions.
/// Calculated from current inputs, never stored on the member.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Armour {
    pub total: i32,
    pub by_location: EnumMap<BodyPart, i32>,
    /// What speed adds, which a blow from behind gets past.
    pub speed: i32,
    /// What a shield and the skill to use it add, likewise.
    pub shield: i32,
}

impl Armour {
    /// Hit-location-weighted average displayed on the character portrait.
    pub fn average(&self) -> i32 {
        let weight = |part| match part {
            BodyPart::Head => 15,
            BodyPart::Body => 40,
            BodyPart::Leg => 30,
            BodyPart::Hand => 10,
            BodyPart::Foot => 5,
        };
        let total: i32 = self.by_location.iter().map(|(part, value)| value * weight(part)).sum();
        (total as f32 / 100.0).round() as i32
    }
}

/// The equipment slot whose armour covers a character's hit location.
fn armour_slot(part: BodyPart) -> EquipSlot {
    match part {
        BodyPart::Head => EquipSlot::Head,
        BodyPart::Body => EquipSlot::Torso,
        BodyPart::Leg => EquipSlot::Legs,
        BodyPart::Hand => EquipSlot::Hands,
        BodyPart::Foot => EquipSlot::Feet,
    }
}

/// A character's armour (retail `CalcArmorClasses`), given what protects in
/// each slot they wear, and whether it is a shield. Defending adds 2
/// everywhere and lets Shield skill count for more.
pub fn armour<'a>(
    c: &Character,
    a: &Attributes,
    worn: impl Fn(EquipSlot) -> Option<(&'a Protection, bool)>,
    load: Load,
    defending: bool,
) -> Armour {
    let speed = a[Attribute::Speed];
    let speed = i32::from(speed > 79) + i32::from(speed > 89)
        - i32::from(speed < 20)
        - i32::from(speed < 10);
    let mut total =
        c.race.row().armour + speed - load.armour_penalty() + if defending { 2 } else { 0 };
    let mut shield = 0;
    // Shields and magic items worn in the misc, cloak and weapon slots.
    for slot in [
        EquipSlot::Misc1,
        EquipSlot::Misc2,
        EquipSlot::Cloak,
        EquipSlot::Primary,
        EquipSlot::Secondary,
    ] {
        if let Some((protection, is_shield)) = worn(slot) {
            total += protection.armour;
            if is_shield {
                let bonus = protection.armour;
                let skill = c.skill(Skill::Shield) as i32;
                let skill = if defending {
                    (skill / 15).min(bonus * 3 / 2)
                } else {
                    (skill / 25).min(bonus)
                };
                total += skill;
                shield += bonus + skill;
            }
        }
    }
    let by_location = EnumMap::from_fn(|part| {
        total + worn(armour_slot(part)).map_or(0, |(protection, _)| protection.armour)
    });
    Armour { total, by_location, speed, shield }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::Gender;
    use crate::content::items::ItemKind;
    use crate::content::items::{EquipClass, ItemDefinition, ItemId};
    use crate::game::characters::{Profession, Race};
    use crate::game::items::{Items, Worn, make};
    use bevy::ecs::system::RunSystemOnce;

    fn weapon(id: u16, class: EquipClass, skill: Skill) -> ItemDefinition {
        ItemDefinition {
            id: ItemId(id),
            class,
            weapon_skill: Some(skill),
            damage: Dice { base: 0, count: 1, sides: 6 },
            attack_modes: EnumSet::only(AttackMode::Swing),
            professions: EnumSet::all(),
            races: EnumSet::all(),
            genders: EnumSet::all(),
            ..ItemDefinition::default()
        }
    }

    /// A member wearing these in their hands, any launcher holding with any
    /// partner, and what `through` makes of their hands and what they wear.
    fn wearing<T: Send + 'static>(
        worn: &[(EquipSlot, &ItemDefinition)],
        through: impl Fn(&Items, &Query<Held>, Entity) -> T + Send + Sync + 'static,
    ) -> T {
        let mut world = World::new();
        let member = world.spawn_empty().id();
        for &(slot, definition) in worn {
            let definition = definition.clone();
            let item = world
                .run_system_once(move |mut commands: Commands| {
                    make(&mut commands, &definition, 1, true)
                })
                .unwrap();
            world.flush();
            world.entity_mut(item).insert(Worn { by: member, slot });
        }
        world
            .run_system_once(move |items: Items, held: Query<Held>| through(&items, &held, member))
            .unwrap()
    }

    fn in_hands<T: Send + 'static>(
        worn: &[(EquipSlot, &ItemDefinition)],
        through: impl Fn(&Hands) -> T + Send + Sync + 'static,
    ) -> T {
        wearing(worn, move |items, held, member| {
            through(&Hands::of(|slot| {
                items.worn(member, slot).and_then(|item| held.get(item).ok())
            }))
        })
    }

    #[test]
    fn defending_adds_two_everywhere_and_lets_shield_skill_count_for_more() {
        let shield = ItemDefinition {
            class: EquipClass::Shield,
            armor_class_bonus: 4,
            ..ItemDefinition::default()
        };
        let mut bearer = Character::new("A", Race::Human, Gender::Male, Profession::Mage);
        let armours = |bearer: Character, defending| {
            wearing(&[(EquipSlot::Secondary, &shield)], move |items, _, member| {
                armour(
                    &bearer,
                    &Attributes::of_sheet(&bearer),
                    |slot| items.protection(member, slot),
                    Load::None,
                    defending,
                )
            })
        };
        let gain = |bearer: &Character| {
            let standing = armours(bearer.clone(), false);
            let defending = armours(bearer.clone(), true);
            for (part, armour) in &defending.by_location {
                assert_eq!(armour - standing.by_location[part], defending.total - standing.total);
            }
            defending.total - standing.total
        };
        // Shield 60: 60 / 25 = 2 standing, 60 / 15 = 4 defending.
        bearer.skill_points[Skill::Shield] = 60;
        assert_eq!(gain(&bearer), 2 + (4 - 2));
        // Shield 125: capped at the shield's 4 standing, at 6 (1.5×) defending.
        bearer.skill_points[Skill::Shield] = 125;
        assert_eq!(gain(&bearer), 2 + (6 - 4));
        assert_eq!(armours(bearer, false).shield, 4 + 4);
    }

    #[test]
    fn a_punch_rolls_the_dice_of_whatever_the_off_hand_holds() {
        let shield = ItemDefinition {
            id: ItemId(1),
            class: EquipClass::Shield,
            ..ItemDefinition::default()
        };
        let mut arrows = weapon(3, EquipClass::Ammunition, Skill::Bow);
        arrows.kind = ItemKind::ARROW;
        arrows.damage = Dice { base: 0, count: 1, sides: 8 };
        let mut knife = weapon(4, EquipClass::ThrownWeapon, Skill::ThrowingAndSling);
        knife.damage = Dice { base: 1, count: 2, sides: 3 };
        let monk = Character::new("A", Race::Human, Gender::Male, Profession::Fighter);
        let attributes = Attributes::of_sheet(&monk);
        let punch = |worn: &[(EquipSlot, &ItemDefinition)]| {
            let monk = monk.clone();
            in_hands(worn, move |hands| {
                let arm = hands.arm(&monk, &attributes, Load::None, 0, WeaponSlot::Primary);
                (arm.modes, arm.punch, arm.kick)
            })
        };
        // Empty hands: both fists roll Martial Arts 0's 1d2, the main hand's
        // two more on the base.
        let (modes, bare, kick) = punch(&[]);
        assert_eq!(modes, EnumSet::only(AttackMode::Punch));
        assert_eq!(bare, Dice { base: 0, count: 1, sides: 2 });
        assert_eq!(kick, Dice { base: 2, count: 1, sides: 2 });
        // A shield's record has no dice, and does not strike; arrows and a
        // thrown knife do not strike beside a fist either, yet a punch rolls
        // their record's dice (retail reads `Hand[1]`'s item).
        assert_eq!(punch(&[(EquipSlot::Secondary, &shield)]).1, Dice::default());
        assert_eq!(punch(&[(EquipSlot::Secondary, &arrows)]).1, arrows.damage);
        assert_eq!(punch(&[(EquipSlot::Secondary, &knife)]).1, knife.damage);
    }

    #[test]
    fn two_weapons_each_attack_with_their_dual_wielding_penalty() {
        let sword = weapon(0, EquipClass::ShortWeapon, Skill::Sword);
        let dagger = weapon(1, EquipClass::ShortWeapon, Skill::Dagger);
        let mut fighter = Character::new("A", Race::Human, Gender::Male, Profession::Fighter);
        fighter.skill_points[Skill::DualWeapons] = 40;
        let attributes = Attributes::of_sheet(&fighter);
        let striking = |worn: &[(EquipSlot, &ItemDefinition)]| {
            let fighter = fighter.clone();
            in_hands(worn, move |hands| {
                let (c, a) = (&fighter, &attributes);
                let slots = [WeaponSlot::Primary, WeaponSlot::Secondary];
                let rating = slots.map(|slot| hands.rating(c, a, slot));
                (
                    slots.map(|slot| hands.hands[slot].strikes()),
                    rating,
                    hands.attacks(c, a, Load::None, 0),
                )
            })
        };
        // An empty secondary slot beside a weapon does not punch.
        let (alone, ..) = striking(&[(EquipSlot::Primary, &sword)]);
        assert_eq!(alone, [true, false]);
        let (both, rating, attacks) =
            striking(&[(EquipSlot::Primary, &sword), (EquipSlot::Secondary, &dagger)]);
        assert_eq!(both, [true, true]);
        assert!(attacks.arms[1].weapon.is_some());
        assert_eq!(attacks.arms[1].modes, dagger.attack_modes);
        // Each hand pays −10 per hand number and a quarter of the Dual
        // Weapons it lacks: (−10 − 15)/3 and (−20 − 15)/3 off the rating.
        assert!(rating[0] > rating[1]);
        assert_eq!(attacks.opening(), Some(WeaponSlot::Primary));
    }

    #[test]
    fn a_shield_or_a_mismatched_weapon_keeps_the_off_hand_out() {
        let sword = weapon(0, EquipClass::ShortWeapon, Skill::Sword);
        let shield = ItemDefinition {
            id: ItemId(1),
            class: EquipClass::Shield,
            ..ItemDefinition::default()
        };
        let mut bow = weapon(2, EquipClass::RangedWeapon, Skill::Bow);
        bow.kind = ItemKind::LONGBOW;
        bow.reach = Reach::Long;
        bow.damage = Dice { base: 2, count: 1, sides: 4 };
        let mut arrows = weapon(3, EquipClass::Ammunition, Skill::Bow);
        arrows.kind = ItemKind::ARROW;
        arrows.damage = Dice { base: 0, count: 1, sides: 8 };
        arrows.effects.chances[AttackEffect::Poison] = 10;
        let ranger = Character::new("A", Race::Human, Gender::Male, Profession::Ranger);
        let attributes = Attributes::of_sheet(&ranger);
        let strikes = |worn: &[(EquipSlot, &ItemDefinition)]| {
            in_hands(worn, |hands| {
                [WeaponSlot::Primary, WeaponSlot::Secondary].map(|slot| hands.hands[slot].strikes())
            })
        };
        assert_eq!(
            strikes(&[(EquipSlot::Primary, &sword), (EquipSlot::Secondary, &shield)]),
            [true, false]
        );
        // A bow without arrows cannot shoot; with them it shoots theirs.
        assert_eq!(strikes(&[(EquipSlot::Primary, &bow)]), [false, false]);
        let (strikes, ranged, loaded, dice, poison, percent) = in_hands(
            &[(EquipSlot::Primary, &bow), (EquipSlot::Secondary, &arrows)],
            move |hands| {
                let primary = WeaponSlot::Primary;
                let arm = hands.arm(&ranger, &attributes, Load::None, 0, primary);
                (
                    [primary, WeaponSlot::Secondary].map(|slot| hands.hands[slot].strikes()),
                    arm.ranged,
                    arm.loaded,
                    hands.dice(&ranger, primary),
                    hands.effects(&attributes, primary).chances[AttackEffect::Poison],
                    hands.damage_percent(&attributes, primary),
                )
            },
        );
        assert_eq!(strikes, [true, false]);
        assert!(ranged);
        // The bow shoots the arrows, which pierce by their own modes.
        assert_eq!(loaded, Some(arrows.attack_modes));
        assert_eq!(dice, arrows.damage);
        assert_eq!(poison, 10);
        // The bow's own base is a damage bonus: +20%, with strength's.
        let strength = Attributes::of_sheet(&Character::new(
            "A",
            Race::Human,
            Gender::Male,
            Profession::Ranger,
        ))[Attribute::Strength] as i32;
        let from_strength = match strength {
            ..50 => -(50 - strength),
            50 => 0,
            _ => 2 * strength - 100,
        };
        assert_eq!(percent, 20 + from_strength);
    }
}
