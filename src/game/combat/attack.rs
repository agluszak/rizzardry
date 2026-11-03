//! The rules of weapon attacks, as ordinary functions over fighters'
//! components (retail `Combat Attack.cpp`, `CalcAttacks`). The swing systems
//! ([`super::swings`]) add their terms up; the character sheet reads the
//! same rules.
//!
//! A missile's flight is not simulated: where a shot lands is retail's
//! scatter around its aim point, and it strikes its target when that point
//! falls within the target's body. Retail flies the shot and can hit
//! whatever else it meets on the way.

use bevy::math::Vec3Swizzles;
use bevy::prelude::{Component, Vec3};
use enum_map::{EnumMap, enum_map};
use enumset::EnumSet;

use crate::content::characters::{Attribute, BodyPart, Skill, Trait};
use crate::content::items::{
    AttackEffect, AttackEffects, AttackMode, Dice, EquipClass, ItemCatalog, ItemDefinition,
    ItemKind, QuantityKind,
};
use crate::content::species::{MonsterKind, Reach, Species};
#[cfg(test)]
use crate::game::characters::Character;
use crate::game::characters::{CharacterStats, Load, Race, Vitals};
use crate::game::inventory::{Carried, EquipSlot, ItemStack, WeaponSlot};
use crate::game::magic::enchantments::Enchantments;
use crate::game::rng::GameRng;

/// The to-hit term of a mode (retail `AttackModeMod` for the aim).
pub fn mode_to_hit(mode: AttackMode) -> i32 {
    use AttackMode::*;
    match mode {
        Swing | Bash | Shoot => 0,
        Thrust | Kick => -10,
        Berserk => -30,
        Throw => -5,
        Punch => 5,
        Lash => 10,
    }
}

/// The penetration term of a mode (retail `AttackModeMod` for armour).
pub fn mode_penetration(mode: AttackMode) -> i32 {
    use AttackMode::*;
    match mode {
        Swing | Kick => 0,
        Thrust | Shoot => 10,
        Bash | Punch => -5,
        Berserk => -30,
        Throw => 5,
        Lash => -10,
    }
}

/// Throws, lashes and shots: modes that neither find a gap in a distracted
/// guard nor roll extra dice for it.
pub fn keeps_distance(mode: AttackMode) -> bool {
    matches!(mode, AttackMode::Throw | AttackMode::Lash | AttackMode::Shoot)
}

/// Pick one of the modes (retail `ChooseAttackMode`): draw `w` below 9,
/// then walk the mode bits round from the first, taking the (w+1)-th set
/// one. Not uniform when the count does not divide 9.
pub fn choose_mode(rng: &mut GameRng, modes: EnumSet<AttackMode>) -> AttackMode {
    let wanted = rng.below(9) as usize;
    modes.iter().cycle().nth(wanted).unwrap_or(AttackMode::Swing)
}

/// Stamina spent by one swing (retail `CharacterActionFatigueCost`): punches
/// and kicks cost a flat roll, weapons by their weight; a Berserk order
/// doubles it, and so does disease. Load scaling belongs to the caller.
pub fn swing_stamina(
    rng: &mut GameRng,
    mode: AttackMode,
    weight: Option<u16>,
    berserk: bool,
    diseased: bool,
) -> u32 {
    let cost = match mode {
        AttackMode::Punch => rng.below(3) + 2,
        AttackMode::Kick => rng.below(3) + 3,
        _ => match weight {
            None => rng.below(4) + 3,
            Some(weight) => {
                let band = u32::from(weight) / 40 + 1;
                rng.below(band) + 1 + band
            }
        },
    };
    cost * if berserk { 2 } else { 1 } * if diseased { 2 } else { 1 }
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

/// A monster's body, where a blow lands past its chances.
pub const MONSTER_BODY: usize = 3;
/// Where blows from the party land on one of its own (retail
/// `gubLocalACPercent`): head, body, legs, hands, feet.
pub const MEMBER_LOCATIONS: [u8; 5] = [15, 40, 30, 10, 5];

/// Where a blow lands, by cumulative percentages against one `below(100)`
/// draw; anything past them hits `fallback`.
pub fn locate(rng: &mut GameRng, chances: &[u8], fallback: usize) -> Location {
    let roll = rng.below(100);
    let mut total = 0;
    for (location, &chance) in chances.iter().enumerate() {
        total += u32::from(chance);
        if roll < total {
            return Location(location);
        }
    }
    Location(fallback)
}

/// A member's agility as blows weigh it: senses + speed + dexterity +
/// intelligence (retail SSDI).
pub fn member_agility(c: &CharacterStats<'_>) -> i32 {
    use Attribute::*;
    [Senses, Speed, Dexterity, Intelligence].map(|a| c.attribute(a) as i32).iter().sum()
}

/// A monster's agility as blows weigh it, its spells raising each
/// attribute within 1 and 125.
pub fn monster_agility(s: &Species, effects: &Enchantments) -> i32 {
    let a = s.attributes;
    [
        (Attribute::Senses, a.senses),
        (Attribute::Speed, a.speed),
        (Attribute::Dexterity, a.dexterity),
        (Attribute::Intelligence, a.intelligence),
    ]
    .map(|(attribute, base)| {
        (u32::from(base) + effects.attribute_bonus(attribute)).clamp(1, 125) as i32
    })
    .iter()
    .sum()
}

/// Aim and armour lost to fatigue by the share of stamina spent (retail
/// `FatigueArmorPenalty`).
pub fn fatigue(vitals: &Vitals) -> i32 {
    match vitals.stamina.spent_percent() {
        0..50 => 0,
        50..70 => 5,
        70..85 => 10,
        85..=94 => 20,
        _ => 40,
    }
}

/// What a weapon slot holds (retail `SetHandType`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Holds {
    #[default]
    Nothing,
    Weapon,
    Shield,
    Ammunition,
    /// Anything else, which cannot strike.
    Other,
}

/// The weapons that need ammunition from the other slot (retail
/// `ItemHasSingledOutGenericName`): longbow, crossbow, sling, greatbow and
/// omnigun.
fn is_launcher(kind: ItemKind) -> bool {
    matches!(
        kind,
        ItemKind::LONGBOW
            | ItemKind::CROSSBOW
            | ItemKind::SLING
            | ItemKind::GREATBOW
            | ItemKind::OMNIGUN
    )
}
/// Throwing weapons and guns that strength does not help.
const NO_STRENGTH: [ItemKind; 5] = [
    ItemKind::POWER_GLOVE,
    ItemKind::PHASER,
    ItemKind::MUSKET,
    ItemKind::OMNIGUN,
    ItemKind::ROCKET_LAUNCHER,
];

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

/// How a member holds their weapon set, decoded from what they wear (retail
/// `CalcAttacks`): what each weapon slot holds, whether it fights at range
/// or close, and which slots can attack. An empty slot fights with Martial
/// Arts; the secondary slot attacks only beside a primary of the same kind
/// (close or ranged), and never with a shield or ammunition. Its methods
/// are the rules that read it with the member's attributes and skills.
pub struct Grip<'a> {
    worn: EnumMap<WeaponSlot, Option<(&'a ItemStack, &'a ItemDefinition)>>,
    holds: EnumMap<WeaponSlot, Holds>,
    /// Ranged or close combat, or neither for a slot that cannot strike.
    ranged: EnumMap<WeaponSlot, Option<bool>>,
    in_play: EnumMap<WeaponSlot, bool>,
    dual_wielding: bool,
}

impl<'a> Grip<'a> {
    pub fn of(carried: &'a Carried, items: &'a ItemCatalog) -> Self {
        use WeaponSlot::*;
        let worn: EnumMap<WeaponSlot, Option<(&ItemStack, &ItemDefinition)>> =
            enum_map! { slot => carried.worn(slot.equip()).map(|s| (s, &items[s.item])) };
        let definition = |slot: WeaponSlot| worn[slot].map(|(_, d)| d);
        let holds = enum_map! { slot => match definition(slot) {
            None => Holds::Nothing,
            Some(d) => match d.class {
                EquipClass::ShortWeapon | EquipClass::ExtendedWeapon | EquipClass::RangedWeapon => {
                    Holds::Weapon
                }
                EquipClass::ThrownWeapon
                    if slot == Secondary
                        && definition(Primary).is_some_and(|p| p.kind == ItemKind::OMNIGUN) =>
                {
                    Holds::Ammunition
                }
                EquipClass::ThrownWeapon => Holds::Weapon,
                EquipClass::Ammunition => Holds::Ammunition,
                EquipClass::Shield => Holds::Shield,
                _ => Holds::Other,
            },
        }};
        let ranged = enum_map! { slot => {
            match definition(slot).map_or(Some(Skill::MartialArts), |d| d.weapon_skill) {
                Some(skill)
                    if skill as usize <= Skill::Shield as usize || skill == Skill::MartialArts =>
                {
                    Some(false)
                }
                Some(Skill::ModernWeapon | Skill::Bow | Skill::ThrowingAndSling) => Some(true),
                _ => None,
            }
        }};
        // A ranged slot cannot attack with a launcher lacking its
        // ammunition, or a gun out of shots; it keeps its combat skill all
        // the same.
        let loaded = |slot: WeaponSlot| {
            let Some((stack, d)) = worn[slot] else { return true };
            let other = definition(slot.other());
            (!is_launcher(d.kind) || other.is_some_and(|o| crate::game::inventory::partners(d, o)))
                && !(d.quantity == QuantityKind::Shots && stack.remaining_uses() == Some(0))
        };
        let mut in_play = enum_map! { slot => match ranged[slot] {
            None => false,
            Some(false) => true,
            Some(true) => loaded(slot),
        }};
        if ranged[Primary] != ranged[Secondary]
            || matches!(holds[Secondary], Holds::Shield | Holds::Ammunition)
            || definition(Primary).is_some_and(|d| d.kind == ItemKind::OMNIGUN)
            || (holds[Secondary] == Holds::Nothing && holds[Primary] != Holds::Nothing)
        {
            in_play[Secondary] = false;
        }
        let dual_wielding = holds.values().all(|&holds| holds == Holds::Weapon)
            && in_play.values().all(|&in_play| in_play);
        Self { worn, holds, ranged, in_play, dual_wielding }
    }

    /// The weapon worn in a slot.
    pub fn weapon(&self, slot: WeaponSlot) -> Option<&'a ItemDefinition> {
        self.worn[slot].map(|(_, d)| d)
    }

    /// The stack worn in a slot, by its identity.
    pub fn weapon_id(&self, slot: WeaponSlot) -> Option<crate::game::inventory::StackId> {
        self.worn[slot].map(|(stack, _)| stack.id)
    }

    /// The slot can attack.
    pub fn in_play(&self, slot: WeaponSlot) -> bool {
        self.in_play[slot]
    }

    /// The skill the slot fights with: its weapon's, Martial Arts when empty.
    pub fn weapon_skill(&self, slot: WeaponSlot) -> Skill {
        match self.weapon(slot) {
            Some(d) => d.weapon_skill.unwrap_or(Skill::MartialArts),
            None => Skill::MartialArts,
        }
    }

    /// Its blows are ranged combat, not close combat.
    pub fn ranged(&self, slot: WeaponSlot) -> bool {
        self.ranged[slot] == Some(true)
    }

    /// How far it strikes: a weapon's or ammunition's reach, otherwise
    /// arm's length.
    pub fn reach(&self, slot: WeaponSlot) -> Reach {
        match (self.holds[slot], self.weapon(slot)) {
            (Holds::Weapon | Holds::Ammunition, Some(d)) => d.reach,
            _ => Reach::Touch,
        }
    }

    /// It shoots ammunition from the other slot.
    pub fn paired(&self, slot: WeaponSlot) -> bool {
        self.weapon(slot).is_some_and(|d| is_launcher(d.kind)) && self.in_play[slot]
    }

    /// What a launcher shoots: the ammunition in the other slot.
    pub fn ammunition(&self, slot: WeaponSlot) -> Option<&'a ItemDefinition> {
        if self.paired(slot) { self.weapon(slot.other()) } else { None }
    }

    /// The slot's dice: what a launcher shoots, a weapon's own, or a fist
    /// by Martial Arts, two more in the primary slot.
    pub fn dice(&self, c: &CharacterStats<'_>, slot: WeaponSlot) -> Dice {
        match self.weapon(slot) {
            Some(d) => self.ammunition(slot).unwrap_or(d).damage,
            None => {
                let mut dice = unarmed_dice(c.skill(Skill::MartialArts));
                if slot == WeaponSlot::Primary {
                    dice.base += 2;
                }
                dice
            }
        }
    }

    /// The ways the slot strikes: its weapon's, or a fist's punch, and a
    /// kick past Martial Arts 4.
    pub fn modes(&self, c: &CharacterStats<'_>, slot: WeaponSlot) -> EnumSet<AttackMode> {
        match self.weapon(slot) {
            Some(d) => d.attack_modes,
            None if c.skill(Skill::MartialArts) > 4 => AttackMode::Punch | AttackMode::Kick,
            None => EnumSet::only(AttackMode::Punch),
        }
    }

    /// What its blows may do besides damage: the weapon's effects with its
    /// ammunition's, or a strong fist's knockout chance.
    pub fn effects(&self, c: &CharacterStats<'_>, slot: WeaponSlot) -> AttackEffects {
        match self.weapon(slot) {
            Some(d) => self.ammunition(slot).map_or(d.effects, |a| d.effects.with(&a.effects)),
            None => {
                let mut effects = AttackEffects::default();
                let strength = c.attribute(Attribute::Strength) as i32;
                if strength > 49 {
                    effects.chances[AttackEffect::KnockOut] = ((strength - 50) / 5) as u32;
                }
                effects
            }
        }
    }

    /// The monster kinds its weapon and ammunition slay.
    pub fn slays(&self, slot: WeaponSlot) -> Vec<MonsterKind> {
        [self.weapon(slot), self.ammunition(slot)]
            .into_iter()
            .flatten()
            .filter_map(|d| d.slays)
            .collect()
    }

    /// The combat and weapon skills the slot fights with, combined (retail
    /// `CalcAttacks`): weighed with the other slot's weapon skill when it
    /// attacks too, and Dual Weapons when both hold weapons.
    fn skill(&self, c: &CharacterStats<'_>, slot: WeaponSlot) -> i32 {
        let combat = if self.ranged(slot) { Skill::RangedCombat } else { Skill::CloseCombat };
        let mut score = ((c.skill(combat) + c.skill(self.weapon_skill(slot)) * 2) * 2 / 3) as i32;
        let mut divisor = 20;
        let other = slot.other();
        if self.holds[other] == Holds::Weapon && self.in_play[other] {
            score += c.skill(self.weapon_skill(other)) as i32 >> 1;
            divisor = 25;
        }
        if self.dual_wielding {
            score += c.skill(Skill::DualWeapons) as i32;
            divisor += 10;
        }
        score * 10 / divisor
    }

    /// What wielding two weapons costs the slot: 10 a slot and a quarter of
    /// the Dual Weapons lacking.
    fn dual_penalty(&self, c: &CharacterStats<'_>, slot: WeaponSlot) -> i32 {
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
        if self.weapon_skill(slot) == Skill::ModernWeapon { penalty / 2 } else { penalty }
    }

    /// The slot's attack rating (retail `attack_rating`); none when it
    /// cannot attack.
    pub fn rating(&self, c: &CharacterStats<'_>, slot: WeaponSlot) -> i32 {
        if !self.in_play[slot] {
            return 0;
        }
        let dexterity = c.attribute(Attribute::Dexterity) as i32;
        (self.dual_penalty(c, slot) + dexterity / 2 + self.skill(c, slot) * 2 + c.physical_levels())
            / 3
            + 60
    }

    /// Attacks a round with the slot: one, a second past 50 and a third
    /// past 100 in the primary, a second past 75 in the secondary; none when
    /// it cannot attack.
    pub fn attacks(&self, c: &CharacterStats<'_>, load: Load, slot: WeaponSlot) -> u32 {
        if !self.in_play[slot] {
            return 0;
        }
        let [dexterity, speed] =
            [Attribute::Dexterity, Attribute::Speed].map(|a| c.attribute(a) as i32);
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

    /// The slot's initiative: its weapon's (retail `damage_bonus`), or a
    /// fist's from skill; none when it cannot attack.
    pub fn initiative(&self, c: &CharacterStats<'_>, slot: WeaponSlot) -> i32 {
        if !self.in_play[slot] {
            return 0;
        }
        self.weapon(slot).map_or(self.skill(c, slot) / 10, |d| d.damage_bonus)
    }

    /// Most swings an attack with the slot makes: one, a second past 66
    /// and a third past 100, before what its weapon adds; a rocket launcher
    /// in the primary slot makes one. None when it cannot attack.
    pub fn swings(&self, c: &CharacterStats<'_>, load: Load, slot: WeaponSlot) -> u32 {
        if !self.in_play[slot] {
            return 0;
        }
        if self.weapon(WeaponSlot::Primary).is_some_and(|d| d.kind == ItemKind::ROCKET_LAUNCHER) {
            return 1;
        }
        // Retail reads the initiative its previous pass left, which is this.
        let score = (c.attribute(Attribute::Speed) as i32
            + self.initiative(c, slot) * 10
            + self.dual_penalty(c, slot)
            + c.physical_levels()
            + self.halved(slot, -load.attack_penalty())
            + self.skill(c, slot))
            / 3;
        1 + u32::from(score > 66) + u32::from(score > 99)
    }

    /// Swings the weapon adds to each attack.
    pub fn extra_swings(&self, slot: WeaponSlot) -> i32 {
        self.weapon(slot).map_or(0, |d| d.extra_swings)
    }

    /// How strength counts for the slot: not at all for guns and the like
    /// (none), by half for weapons only thrown, lashed or shot, and a
    /// `bonus` by half again in the secondary slot. The divisor.
    fn strength_divisor(&self, slot: WeaponSlot, bonus: bool) -> Option<i32> {
        let mut divisor = 1;
        if let Some(d) = self.weapon(slot)
            && d.attack_modes.is_subset(AttackMode::Throw | AttackMode::Lash | AttackMode::Shoot)
        {
            if NO_STRENGTH.contains(&d.kind) {
                return None;
            }
            divisor = 2;
        }
        if bonus && slot == WeaponSlot::Secondary {
            divisor *= 2;
        }
        Some(divisor)
    }

    /// The sheet's "To Hit" (retail `to_hit`): the weapon's and its
    /// ammunition's bonus, strength, dexterity, senses, load, and Eagle Eye
    /// or Power Strike; none when the slot cannot attack.
    pub fn to_hit(&self, c: &CharacterStats<'_>, load: Load, slot: WeaponSlot) -> i32 {
        if !self.in_play[slot] {
            return 0;
        }
        let [strength, dexterity, senses] =
            [Attribute::Strength, Attribute::Dexterity, Attribute::Senses]
                .map(|a| c.attribute(a) as i32);
        let mut to_hit = 0;
        if let Some(d) = self.weapon(slot) {
            to_hit += d.hit_bonus;
            if self.holds[slot.other()] == Holds::Ammunition
                && let Some(o) = self.weapon(slot.other())
            {
                to_hit += o.hit_bonus;
            }
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
        if self.ranged(slot) && c.skill_available(Skill::EagleEye) {
            to_hit += c.skill(Skill::EagleEye) as i32 / 20 + 1;
        }
        if !self.ranged(slot) && c.skill_available(Skill::PowerStrike) {
            to_hit += c.skill(Skill::PowerStrike) as i32 / 20 + 1;
        }
        to_hit
    }

    /// The sheet's "To Damage (%)": a launcher's base, and strength; none
    /// when the slot cannot attack.
    pub fn damage_percent(&self, c: &CharacterStats<'_>, slot: WeaponSlot) -> i32 {
        if !self.in_play[slot] {
            return 0;
        }
        let strength = c.attribute(Attribute::Strength) as i32;
        let mut percent = 0;
        if let Some(d) = self.weapon(slot)
            && is_launcher(d.kind)
        {
            percent += d.damage.base * 10;
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

    /// The slot an attack opens with (retail `ChooseCharacterAttackHand`):
    /// the one with the most attacks, then the bigger initiative.
    pub fn opening(&self, c: &CharacterStats<'_>, load: Load) -> Option<WeaponSlot> {
        let mut best: Option<WeaponSlot> = None;
        for slot in [WeaponSlot::Primary, WeaponSlot::Secondary] {
            let attacks = self.attacks(c, load, slot);
            let better = match best {
                None => attacks > 0,
                Some(best) => {
                    let theirs = self.attacks(c, load, best);
                    theirs < attacks
                        || (attacks == theirs
                            && self.initiative(c, slot) > self.initiative(c, best))
                }
            };
            if self.in_play[slot] && better {
                best = Some(slot);
            }
        }
        best
    }

    /// A Berserk order needs the trait, a slot that can attack and a
    /// primary for close quarters (retail `CanCharacterBerserk`).
    pub fn can_berserk(&self, c: &CharacterStats<'_>) -> bool {
        use WeaponSlot::*;
        c.has_trait(Trait::Berserk)
            && (self.in_play[Primary] || self.in_play[Secondary])
            && (!self.in_play[Primary] || self.reach(Primary) <= Reach::Short)
    }

    /// The skill whose training adds a kill chance to the slot's blows
    /// (retail `ResolveCharacterAttack`): Critical Strike for close combat,
    /// Ranged Combat for a Ranger's bows and slings, Critical Strike for a
    /// Ninja's throws.
    pub fn critical_skill(&self, c: &CharacterStats<'_>, slot: WeaponSlot) -> Option<Skill> {
        let skill = self.weapon_skill(slot);
        let sling = self.weapon(slot).is_some_and(|d| d.kind == ItemKind::SLING);
        if !self.ranged(slot) {
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
    pub fn sure_throw(&self, c: &CharacterStats<'_>, slot: WeaponSlot) -> bool {
        c.has_trait(Trait::ThrownCriticals)
            && self.ranged(slot)
            && self.weapon_skill(slot) == Skill::ThrowingAndSling
            && self.weapon(slot).is_none_or(|d| d.kind != ItemKind::SLING)
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

/// A member's armour with the spells on them and, for a party member, the
/// party's protections.
pub fn enchanted_armour(
    c: &CharacterStats<'_>,
    carried: &Carried,
    catalog: &ItemCatalog,
    load: Load,
    defending: bool,
    personal: &Enchantments,
    party: Option<&Enchantments>,
) -> Armour {
    let mut armour = armour(c, carried, catalog, load, defending);
    let bonus = personal.armour() + party.map_or(0, |party| party.armour());
    armour.total += bonus;
    for (_, value) in armour.by_location.iter_mut() {
        *value += bonus;
    }
    armour
}

/// A character's armour (retail `CalcArmorClasses`). Defending adds 2
/// everywhere and lets Shield skill count for more.
pub fn armour(
    c: &CharacterStats<'_>,
    carried: &Carried,
    catalog: &ItemCatalog,
    load: Load,
    defending: bool,
) -> Armour {
    let definition = |slot| carried.worn(slot).map(|i| &catalog[i.item]);
    let speed = c.attribute(Attribute::Speed);
    let speed = i32::from(speed > 79) + i32::from(speed > 89)
        - i32::from(speed < 20)
        - i32::from(speed < 10);
    let mut total = i32::from(c.race == Race::Faerie) * 2 + speed - load.armour_penalty()
        + if defending { 2 } else { 0 };
    let mut shield = 0;
    // Shields and magic items worn in the misc, cloak and weapon slots.
    for slot in [
        EquipSlot::Misc1,
        EquipSlot::Misc2,
        EquipSlot::Cloak,
        EquipSlot::Primary,
        EquipSlot::Secondary,
    ] {
        if let Some(d) = definition(slot) {
            total += d.armor_class_bonus;
            if d.class == EquipClass::Shield {
                let bonus = d.armor_class_bonus;
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
        let worn = definition(armour_slot(part));
        total + worn.map_or(0, |d: &ItemDefinition| d.armor_class_bonus)
    });
    Armour { total, by_location, speed, shield }
}

/// How an attack catches its target (retail `unaware` and `natural_attack`).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Surprise {
    /// Struck from behind, unseen.
    pub unaware: bool,
    /// A rogue's backstab, or a monster's natural ambush.
    pub natural: bool,
}

impl Surprise {
    pub fn any(self) -> bool {
        self.unaware || self.natural
    }
}

/// Where a blow lands: one of a monster's seven locations, or a member's
/// body part.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Location(pub usize);

/// A monster's damage reduction: its species' and its spells', at most all.
pub fn monster_reduction(species: &Species, vitals: &Vitals) -> i32 {
    (i32::from(species.damage_reduction) + vitals.enchantments.damage_reduction()).min(100)
}

/// Damage after a reduction in percent (retail `ApplyDamageReduction`).
pub fn reduce(damage: i32, reduction: i32) -> u32 {
    let damage = if reduction != 0 { ((100 - reduction) * damage + 50) / 100 } else { damage };
    damage.max(0) as u32
}

/// The most one blow that went astray takes off its victim: a fifth of its
/// health, at least one (retail `CapAttackDamageByTargetHealth`).
pub fn fumble_cap(damage: u32, health: u32) -> u32 {
    damage.min((health / 5).max(1))
}

/// The chance a swing goes astray onto someone else in reach (retail
/// `GetCombatFumbleChance`): none from an aim of 100 or more, otherwise
/// cubic in what it falls short by and growing with who could be struck.
pub fn fumble_chance(bystanders: usize, aim: i32) -> u32 {
    if aim >= 100 {
        return 0;
    }
    let chance = (f64::from(100 - aim).powi(3) * 1e-5 + 0.5) as u32;
    (bystanders as u32 * chance * 10 / 100).min(100)
}

/// One swing's to-hit roll and what decides it (retail: 5 or under always
/// lands, over 95 always misses, a fumble lands elsewhere).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aim {
    Missed,
    /// The swing reaches its target: `roll` against the aim and
    /// penetration, which a roll of 5 or under gets through.
    Reaches {
        roll: u32,
        sure: bool,
    },
    /// It goes astray: the bystander it strikes, by index, with a fresh
    /// roll for penetration.
    Fumbled {
        bystander: usize,
        roll: u32,
    },
}

/// Roll a melee swing (retail `ResolveCharacterAttack`,
/// `ContinueMonsterAttack`): the roll, then a fumble onto one of
/// `bystanders` when it falls within the fumble chance.
pub fn swing(rng: &mut GameRng, bystanders: usize, fumble: u32) -> Aim {
    let roll = rng.below(100) + 1;
    let sure = roll <= 5;
    if !sure && bystanders > 0 && roll > 100 - fumble {
        let bystander = rng.below(bystanders as u32) as usize;
        return Aim::Fumbled { bystander, roll: rng.below(100) + 1 };
    }
    if roll > 95 { Aim::Missed } else { Aim::Reaches { roll, sure } }
}

/// Extra dice a backstab rolls (retail `RollBackstabExtraDice`), up to
/// three with a high enough chance.
pub fn backstab_dice(rng: &mut GameRng, chance: i32) -> u32 {
    let roll = rng.below(100) as i32;
    [15, 75, 95].iter().take_while(|&&over| roll + over < chance).count() as u32
}

/// Roll `dice` `count` times (retail `RollDice` with a roll count).
pub fn roll_dice(rng: &mut GameRng, dice: Dice, count: u32) -> u32 {
    (0..count).map(|_| rng.roll(dice)).sum()
}

/// The effects' strength (retail effect power level): the attacker's level
/// and as much again up to 15.
pub fn effect_power(level: u32) -> u32 {
    level + if level > 14 { 15 } else { level }
}

/// The kill chance training adds to a blow: a point per 25 of `skill` and
/// a chance at one more.
pub fn critical_chance(rng: &mut GameRng, skill: u32) -> u32 {
    let mut bonus = skill / 25;
    if !skill.is_multiple_of(25) && rng.below(25) < skill % 25 {
        bonus += 1;
    }
    bonus
}

/// How well a member presses a blow past a guard (retail
/// `GetCharacterProtectionScore`): Speed, less fatigue (halved with a
/// modern weapon), with the primary slot's to-hit and rating when it can
/// attack, and the member's own to-hit bonus.
pub fn member_protection(c: &CharacterStats<'_>, vitals: &Vitals, grip: &Grip, load: Load) -> i32 {
    let mut fatigue = fatigue(vitals);
    if grip.weapon_skill(WeaponSlot::Primary) == Skill::ModernWeapon {
        fatigue >>= 1;
    }
    let armed = grip.to_hit(c, load, WeaponSlot::Primary) * 5 + grip.rating(c, WeaponSlot::Primary);
    c.attribute(Attribute::Speed) as i32 - fatigue + armed + vitals.enchantments.hit_bonus() * 5
}

/// How well a monster guards, or presses past a guard (retail
/// `GetMonsterProtectionScore`): Speed, less fatigue (halved for the
/// weapons that tire little), with its first attack's score and its to-hit
/// bonus.
pub fn monster_protection(species: &Species, vitals: &Vitals) -> i32 {
    let first = species.attacks.first();
    let mut fatigue = fatigue(vitals);
    if first.is_some_and(crate::content::species::MonsterAttack::tires_little) {
        fatigue >>= 1;
    }
    i32::from(species.attributes.speed) - fatigue
        + first.map_or(0, |a| i32::from(a.score))
        + vitals.enchantments.hit_bonus() * 5
}

/// The world units in a metre (retail's 500).
const UNITS_PER_METRE: f32 = 500.0;

/// Where a missile aimed from `from` at `to` actually heads (retail
/// `ScatterMissileAimPoint`, in metres): a random direction across the line
/// of flight, at a distance accuracy shrinks. A blind shooter scatters
/// coarsely.
pub fn scatter(rng: &mut GameRng, from: Vec3, to: Vec3, accuracy: i32, blind: bool) -> Vec3 {
    let d = to - from;
    // Two directions across the line of flight, as retail builds them.
    let across = Vec3::new(d.y * -d.x, d.z * d.z - d.x * -d.x, -(d.z * d.y));
    let other = Vec3::new(
        across.y * d.z - across.z * d.y,
        across.z * d.x - d.z * across.x,
        across.x * d.y - across.y * d.x,
    );
    let angle = (rng.below(360) as f32).to_radians();
    let direction =
        across.normalize_or_zero() * angle.cos() + other.normalize_or_zero() * angle.sin();
    let distance = d.length();
    let units = f64::from(distance * UNITS_PER_METRE);
    if units < 1.0 {
        return to;
    }
    let distance = f64::from(distance);
    let (factor, widest) = if blind { ((1.0 / distance).min(1.75), 1.75) } else { (0.08, 0.2) };
    let closest = factor * distance;
    let spread = (units * 0.002) as i32;
    let effective = (100 - spread) * accuracy / 100;
    let mut offset = if effective <= 0 {
        widest * distance
    } else if effective >= 300 {
        closest
    } else {
        closest + (widest - factor) * f64::from(300 - effective) / 300.0 * distance
    };
    let roll = rng.below(100) as i32 + 1;
    if effective.min(0x5f) >= roll {
        offset = f64::from(roll) / f64::from(effective) * closest;
    } else if effective >= 1 || roll + effective >= 1 {
        let weight = if effective < 1 {
            f64::from(100 - (roll + effective)) * 0.01
        } else {
            f64::from(roll - effective) / f64::from(roll)
        };
        offset = (offset - closest) * weight + closest;
    }
    to + direction.normalize_or_zero() * offset as f32
}

/// Whether a missile heading for `aimed` strikes a body centred at `at`:
/// within its radius across, and its half-height up or down.
pub fn strikes(at: Vec3, aimed: Vec3, radius: f32, half_height: f32) -> bool {
    let off = aimed - at;
    off.xz().length() <= radius && off.y.abs() <= half_height
}

/// When an actor acts in the round, 1 (first) to 100: a d4 plus its
/// initiative, highest first.
pub fn phase(rng: &mut GameRng, initiative: i32) -> u32 {
    let v = rng.below(4) as i32 + initiative;
    (90 - v.clamp(-10, 89)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::characters::{Gender, Profession};
    use crate::game::inventory::{Equipped, ItemIds, ItemStack};

    fn wield(carried: &mut Carried, slot: EquipSlot, definition: &ItemDefinition, count: u16) {
        let stack = ItemStack::new(definition, &mut ItemIds::default(), count, true);
        carried.equipment[slot] = Some(Equipped::new(stack));
    }

    fn weapon(id: u16, class: EquipClass, skill: Skill) -> ItemDefinition {
        ItemDefinition {
            id: crate::content::items::ItemId(id),
            class,
            weapon_skill: Some(skill),
            damage: Dice { base: 0, count: 1, sides: 6 },
            attack_modes: EnumSet::only(AttackMode::Swing),
            ..ItemDefinition::default()
        }
    }

    #[test]
    fn swing_costs_follow_the_attack_and_disease_and_berserk_double_them() {
        for (mode, weight, range) in [
            (AttackMode::Punch, Some(800), 2..=4),
            (AttackMode::Kick, Some(800), 3..=5),
            (AttackMode::Swing, None, 3..=6),
            (AttackMode::Swing, Some(0), 2..=2),
            (AttackMode::Swing, Some(80), 4..=6),
        ] {
            for seed in 0..16 {
                let cost = swing_stamina(&mut GameRng::seeded(seed), mode, weight, false, false);
                assert!(range.contains(&cost));
                let rng = || GameRng::seeded(seed);
                assert_eq!(swing_stamina(&mut rng(), mode, weight, false, true), 2 * cost);
                assert_eq!(swing_stamina(&mut rng(), mode, weight, true, true), 4 * cost);
            }
        }
    }

    #[test]
    fn defending_adds_two_everywhere_and_lets_shield_skill_count_for_more() {
        let shield = ItemDefinition {
            class: EquipClass::Shield,
            armor_class_bonus: 4,
            ..ItemDefinition::default()
        };
        let catalog = ItemCatalog(vec![shield.clone()]);
        let mut carried = Carried::default();
        wield(&mut carried, EquipSlot::Secondary, &shield, 1);
        let mut bearer = Character::new("A", Race::Human, Gender::Male, Profession::Mage);
        let gain = |bearer: &Character| {
            let standing = armour(&bearer.base_stats(), &carried, &catalog, Load::None, false);
            let defending = armour(&bearer.base_stats(), &carried, &catalog, Load::None, true);
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
        assert_eq!(
            armour(&bearer.base_stats(), &carried, &catalog, Load::None, false).shield,
            4 + 4
        );
    }

    #[test]
    fn early_phases_go_to_high_initiative() {
        let mut rng = GameRng::seeded(1);
        for _ in 0..20 {
            assert!(phase(&mut rng, 200) == 1);
            assert!(phase(&mut rng, -50) == 100);
            assert!((87..=90).contains(&phase(&mut rng, 0)));
        }
    }

    #[test]
    fn two_weapons_each_attack_with_their_dual_wielding_penalty() {
        let sword = weapon(0, EquipClass::ShortWeapon, Skill::Sword);
        let dagger = weapon(1, EquipClass::ShortWeapon, Skill::Dagger);
        let catalog = ItemCatalog(vec![sword.clone(), dagger.clone()]);
        let mut fighter = Character::new("A", Race::Human, Gender::Male, Profession::Fighter);
        fighter.skill_points[Skill::DualWeapons] = 40;
        let mut carried = Carried::default();
        use WeaponSlot::*;
        let stats = fighter.base_stats();
        wield(&mut carried, EquipSlot::Primary, &sword, 1);
        let alone = Grip::of(&carried, &catalog);
        // An empty secondary slot beside a weapon does not punch.
        assert!(alone.in_play(Primary) && !alone.in_play(Secondary));
        wield(&mut carried, EquipSlot::Secondary, &dagger, 1);
        let both = Grip::of(&carried, &catalog);
        assert!(both.in_play(Primary) && both.in_play(Secondary));
        assert_eq!(both.weapon_skill(Secondary), Skill::Dagger);
        // Each slot pays −10 per slot number and a quarter of the Dual
        // Weapons it lacks: (−10 − 15)/3 and (−20 − 15)/3 off the rating.
        assert!(both.rating(&stats, Primary) > both.rating(&stats, Secondary));
        assert_eq!(both.opening(&stats, Load::None), Some(Primary));
    }

    #[test]
    fn a_shield_or_a_mismatched_weapon_keeps_the_off_hand_out() {
        let sword = weapon(0, EquipClass::ShortWeapon, Skill::Sword);
        let shield = ItemDefinition {
            id: crate::content::items::ItemId(1),
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
        let catalog = ItemCatalog(vec![sword.clone(), shield.clone(), bow.clone(), arrows.clone()]);
        use WeaponSlot::*;
        let ranger = Character::new("A", Race::Human, Gender::Male, Profession::Ranger);
        let stats = ranger.base_stats();
        let mut carried = Carried::default();
        wield(&mut carried, EquipSlot::Primary, &sword, 1);
        wield(&mut carried, EquipSlot::Secondary, &shield, 1);
        assert!(!Grip::of(&carried, &catalog).in_play(Secondary));
        // A bow without arrows cannot shoot; with them it shoots theirs.
        let mut carried = Carried::default();
        wield(&mut carried, EquipSlot::Primary, &bow, 1);
        assert!(!Grip::of(&carried, &catalog).in_play(Primary));
        wield(&mut carried, EquipSlot::Secondary, &arrows, 20);
        let grip = Grip::of(&carried, &catalog);
        assert!(grip.in_play(Primary) && grip.paired(Primary) && grip.ranged(Primary));
        assert!(!grip.in_play(Secondary));
        // The bow shoots the arrows, which pierce by their own modes.
        assert_eq!(grip.ammunition(Primary).map(|a| a.attack_modes), Some(arrows.attack_modes));
        assert_eq!(grip.dice(&stats, Primary), arrows.damage);
        assert_eq!(grip.effects(&stats, Primary).chances[AttackEffect::Poison], 10);
        // The bow's own base is a damage bonus: +20%, with strength's.
        let strength = ranger.base_stats().attribute(Attribute::Strength) as i32;
        let from_strength = match strength {
            ..50 => -(50 - strength),
            50 => 0,
            _ => 2 * strength - 100,
        };
        assert_eq!(grip.damage_percent(&stats, Primary), 20 + from_strength);
    }

    #[test]
    fn steadier_aim_scatters_closer_and_blind_poor_shots_scatter_wide() {
        let from = Vec3::ZERO;
        let to = Vec3::new(0.0, 0.0, -10.0);
        let spread = |accuracy, blind| -> f32 {
            (0..200)
                .map(|seed| {
                    let aimed = scatter(&mut GameRng::seeded(seed), from, to, accuracy, blind);
                    // Always across the line of flight.
                    assert!((aimed - to).z.abs() < 1e-3);
                    aimed.distance(to)
                })
                .sum::<f32>()
                / 200.0
        };
        let (poor, good, blind) = (spread(20, false), spread(150, false), spread(20, true));
        assert!(good < poor, "{good} < {poor}");
        assert!(poor < blind, "{poor} < {blind}");
        // At ten metres the tightest scatter is within 8% of the distance.
        assert!(spread(400, false) <= 0.8);
    }
}
