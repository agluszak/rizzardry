//! Weapon attacks in both directions, as ordinary functions over fighter
//! stats (retail `Combat Attack.cpp`, `CalcAttacks`).
//!
//! A missile's flight is not simulated: where a shot lands is retail's
//! scatter around its aim point, and it strikes its target when that point
//! falls within the target's body. Retail flies the shot and can hit
//! whatever else it meets on the way.

use bevy::math::Vec3Swizzles;
use bevy::prelude::{Component, Vec3};
use enum_map::EnumMap;
use enumset::EnumSet;

use crate::content::characters::{Attribute, BodyPart, Skill, Trait};
use crate::content::items::{
    AttackEffect, AttackEffects, AttackMode, Dice, EquipClass, ItemCatalog, ItemDefinition,
    QuantityKind,
};
use crate::content::species::{MonsterAttack, Reach, Species};
use crate::game::characters::{Character, Condition, Load, Race, Vitals};
use crate::game::difficulty::{Difficulty, Side};
use crate::game::inventory::{Carried, EquipSlot};
use crate::game::rng::GameRng;
use crate::game::world::population::Monster;

/// The to-hit term of a mode (retail `AttackModeMod` for the aim).
fn mode_to_hit(mode: AttackMode) -> i32 {
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
fn mode_penetration(mode: AttackMode) -> i32 {
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
fn keeps_distance(mode: AttackMode) -> bool {
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

/// A monster's body, its fallback location.
const MONSTER_BODY: usize = 3;
/// Where blows from the party land on one of its own (retail
/// `gubLocalACPercent`): head, body, legs, hands, feet.
const MEMBER_LOCATIONS: [u8; 5] = [15, 40, 30, 10, 5];

/// Where a blow lands, by cumulative percentages against one `below(100)`
/// draw; anything past them hits `fallback`.
fn location<L: Copy>(
    rng: &mut GameRng,
    chances: impl IntoIterator<Item = (L, u8)>,
    fallback: L,
) -> L {
    let roll = rng.below(100);
    let mut total = 0;
    for (location, chance) in chances {
        total += u32::from(chance);
        if roll < total {
            return location;
        }
    }
    fallback
}

/// Senses + speed + dexterity + intelligence, which the attribute term
/// compares.
fn character_ssdi(c: &Character) -> i32 {
    use Attribute::*;
    [Senses, Speed, Dexterity, Intelligence].map(|a| c.attribute(a) as i32).iter().sum()
}

fn species_ssdi(s: &Species) -> i32 {
    let a = s.attributes;
    [a.senses, a.speed, a.dexterity, a.intelligence].map(i32::from).iter().sum()
}

/// Aim and armour lost to fatigue by the share of stamina spent (retail
/// `FatigueArmorPenalty`).
fn fatigue_penalty(spent: u32) -> i32 {
    match spent {
        0..50 => 0,
        50..70 => 5,
        70..85 => 10,
        85..=94 => 20,
        _ => 40,
    }
}

fn character_fatigue(c: &Character, vitals: &Vitals) -> i32 {
    fatigue_penalty(vitals.stamina_spent(c.max_stamina()))
}

fn monster_fatigue(monster: &Monster) -> i32 {
    fatigue_penalty(100 - monster.stamina * 100 / monster.max_stamina.max(1))
}

/// What a hand holds (retail `SetHandType`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Holds {
    #[default]
    Nothing,
    Weapon,
    Shield,
    Ammunition,
    /// Anything else, which cannot strike.
    Other,
}

/// The weapons that need ammunition from the other hand (retail
/// `ItemHasSingledOutGenericName`): longbow, crossbow, sling, greatbow and
/// omnigun.
fn is_launcher(kind: u16) -> bool {
    matches!(kind, 0x0b | 0x0c | 0x0d | 0x25 | 0x83)
}
const SLING: u16 = 0x0d;
/// The omnigun: what it fires sits in the off hand as ammunition.
const OMNIGUN: u16 = 0x83;
/// A weapon swung only once an attack.
const SINGLE_SWING: u16 = 0x90;
/// Throwing weapons and guns that strength does not help.
const NO_STRENGTH: [u16; 5] = [0x68, 0x6e, 0x72, OMNIGUN, SINGLE_SWING];

/// One hand's attack, as the character sheet works it out (retail
/// `W8HandAttack`).
#[derive(Clone, Debug, PartialEq)]
pub struct Hand {
    pub holds: Holds,
    /// The hand can attack.
    pub in_play: bool,
    pub weapon_skill: Skill,
    /// Its blows are ranged combat, not close combat.
    pub ranged: bool,
    pub reach: Reach,
    pub dice: Dice,
    /// The ways it strikes; one is chosen per attack.
    pub modes: EnumSet<AttackMode>,
    pub attacks: u32,
    pub swings: u32,
    pub attack_rating: i32,
    /// The sheet's "To Hit": weapon, strength, dexterity and senses.
    pub to_hit: i32,
    /// The sheet's "To Damage (%)": strength.
    pub damage_percent: i32,
    /// The weapon's initiative byte (retail `damage_bonus`).
    pub initiative: i32,
    /// What its blows may do besides damage: a fist's knockout chance, or
    /// the weapon's effects with its ammunition's.
    pub effects: AttackEffects,
    /// The monster kinds its weapon and ammunition slay.
    pub slays: [Option<u8>; 2],
    /// Swings the weapon adds to each attack.
    pub extra_swings: i32,
    /// It shoots ammunition from the other hand.
    pub paired: bool,
    /// What swinging it weighs, for its stamina.
    pub weight: Option<u16>,
    /// The weapon's retail kind.
    pub kind: Option<u16>,
}

/// A member's two hands (retail `CalcAttacks`).
#[derive(Clone, Debug, PartialEq)]
pub struct Hands {
    pub hands: [Hand; 2],
    pub dual_wielding: bool,
}

impl Hands {
    /// The hand an attack opens with (retail `ChooseCharacterAttackHand`):
    /// the one with the most attacks, then the bigger initiative.
    pub fn choose(&self) -> Option<usize> {
        let mut best = None;
        for (index, hand) in self.hands.iter().enumerate() {
            let better = match best {
                None => hand.attacks > 0,
                Some(b) => {
                    let b: &Hand = &self.hands[b];
                    b.attacks < hand.attacks
                        || (hand.attacks == b.attacks && hand.initiative > b.initiative)
                }
            };
            if hand.in_play && better {
                best = Some(index);
            }
        }
        best
    }
}

/// Unarmed damage by Martial Arts level / 11 (retail
/// `g_unarmed_damage_dice`), with 2 more in the main hand.
fn unarmed_dice(martial_arts: u32) -> Dice {
    const DICE: [(i16, u8, u8); 12] = [
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

/// Both hands' numbers (retail `CalcAttacks`). An empty hand fights with
/// Martial Arts; an off hand attacks only beside a main hand of the same
/// kind (close or ranged), and never with a shield or ammunition.
pub fn hands(c: &Character, carried: &Carried, catalog: &ItemCatalog, load: Load) -> Hands {
    const SLOTS: [EquipSlot; 2] = [EquipSlot::Primary, EquipSlot::Secondary];
    let worn = SLOTS.map(|slot| carried.worn(slot).and_then(|s| Some((s, catalog.get(s.item)?))));
    let definition = |hand: usize| worn[hand].map(|(_, d)| d);
    let holds = |hand: usize| match definition(hand) {
        None => Holds::Nothing,
        Some(d) => match d.class {
            EquipClass::ShortWeapon | EquipClass::ExtendedWeapon | EquipClass::RangedWeapon => {
                Holds::Weapon
            }
            EquipClass::ThrownWeapon
                if hand == 1 && definition(0).is_some_and(|p| p.kind == OMNIGUN) =>
            {
                Holds::Ammunition
            }
            EquipClass::ThrownWeapon => Holds::Weapon,
            EquipClass::Ammunition => Holds::Ammunition,
            EquipClass::Shield => Holds::Shield,
            _ => Holds::Other,
        },
    };
    let holds = [holds(0), holds(1)];
    let weapon_skill =
        |hand: usize| definition(hand).map_or(Some(Skill::MartialArts), |d| d.weapon_skill);
    // Which combat skill each hand fights with, and whether it can at all.
    let combat = |hand: usize| -> Option<bool> {
        let skill = weapon_skill(hand)?;
        if skill as usize <= Skill::Shield as usize || skill == Skill::MartialArts {
            return Some(false);
        }
        if !matches!(skill, Skill::ModernWeapon | Skill::Bow | Skill::ThrowingAndSling) {
            return None;
        }
        let (stack, d) = worn[hand]?;
        let other = definition(1 - hand);
        if is_launcher(d.kind)
            && !other.is_some_and(|o| d.shoots.is_some() && d.shoots == o.ammunition)
        {
            return None;
        }
        if d.quantity == QuantityKind::Shots && stack.remaining_uses() == Some(0) {
            return None;
        }
        Some(true)
    };
    let ranged = [combat(0), combat(1)];
    let mut in_play = ranged.map(|r| r.is_some());
    if ranged[0] != ranged[1]
        || matches!(holds[1], Holds::Shield | Holds::Ammunition)
        || definition(0).is_some_and(|d| d.kind == OMNIGUN)
        || (holds[1] == Holds::Nothing && holds[0] != Holds::Nothing)
    {
        in_play[1] = false;
    }
    let dual_wielding = holds == [Holds::Weapon, Holds::Weapon] && in_play[0] && in_play[1];

    let physical = c.physical_levels();
    let dual = c.skill(Skill::DualWeapons) as i32;
    let [strength, dexterity, speed, senses] =
        [Attribute::Strength, Attribute::Dexterity, Attribute::Speed, Attribute::Senses]
            .map(|a| c.attribute(a) as i32);
    let hand = |index: usize| -> Hand {
        let skill = weapon_skill(index).unwrap_or(Skill::MartialArts);
        let ranged = ranged[index] == Some(true);
        let d = definition(index);
        // A launcher takes its damage, effects and slaying from what it shoots.
        let paired = d.is_some_and(|d| is_launcher(d.kind)) && in_play[index];
        let ammunition = if paired { definition(1 - index) } else { None };
        let mut hand = Hand {
            holds: holds[index],
            in_play: in_play[index],
            weapon_skill: skill,
            ranged,
            reach: if matches!(holds[index], Holds::Weapon | Holds::Ammunition) {
                d.map_or(Reach::Touch, |d| d.reach)
            } else {
                Reach::Touch
            },
            dice: Dice::default(),
            modes: EnumSet::empty(),
            attacks: 0,
            swings: 0,
            attack_rating: 0,
            to_hit: 0,
            damage_percent: 0,
            initiative: 0,
            effects: AttackEffects::default(),
            slays: [d.and_then(|d| d.slays), ammunition.and_then(|a| a.slays)],
            extra_swings: d.map_or(0, |d| d.extra_swings),
            paired,
            weight: d.map(|d| d.weight),
            kind: d.map(|d| d.kind),
        };
        // What it strikes with, which a punch, a kick or the shot a launcher
        // looses reads from a hand that cannot attack itself.
        match d {
            Some(d) => {
                hand.dice = ammunition.unwrap_or(d).damage;
                hand.modes = d.attack_modes;
                hand.effects = ammunition.map_or(d.effects, |a| d.effects.with(&a.effects));
            }
            None => {
                let martial_arts = c.skill(skill);
                // An empty hand punches, and kicks too past Martial Arts 4.
                hand.modes = if martial_arts > 4 {
                    AttackMode::Punch | AttackMode::Kick
                } else {
                    EnumSet::only(AttackMode::Punch)
                };
                hand.dice = unarmed_dice(martial_arts);
                if index == 0 {
                    hand.dice.base += 2;
                }
                if strength > 49 {
                    hand.effects.chances[AttackEffect::KnockOut] = ((strength - 50) / 5) as u8;
                }
            }
        }
        if !hand.in_play {
            return hand;
        }
        let other = 1 - index;
        let combat_skill = if ranged { Skill::RangedCombat } else { Skill::CloseCombat };
        let mut score = (c.skill(combat_skill) as i32 + c.skill(skill) as i32 * 2) * 2 / 3;
        let mut divisor = 20;
        if holds[other] == Holds::Weapon && in_play[other] {
            score += c.skill(weapon_skill(other).unwrap_or(Skill::MartialArts)) as i32 >> 1;
            divisor = 25;
        }
        if dual_wielding {
            score += dual;
            divisor += 10;
        }
        let combined = score * 10 / divisor;
        // Modern weapons feel load half as much.
        let modern = skill == Skill::ModernWeapon;
        let halved = |penalty: i32| if modern { penalty / 2 } else { penalty };
        let load_penalty = halved(-load.attack_penalty());
        let dual_penalty =
            if dual_wielding { -10 * (index as i32 + 1) - (100 - dual) / 4 } else { 0 };
        hand.attack_rating = (dual_penalty + dexterity / 2 + combined * 2 + physical) / 3 + 60;
        let score = ((speed + dexterity) >> 1) + dual_penalty + physical + load_penalty + combined;
        let score = score / 3;
        hand.attacks = match index {
            0 => 1 + u32::from(score > 49) + u32::from(score > 99),
            _ => 1 + u32::from(score > 74),
        };
        hand.initiative = d.map_or(combined / 10, |d| i32::from(d.damage_bonus));
        // Retail reads the initiative its previous pass left, which is this.
        let score =
            (speed + hand.initiative * 10 + dual_penalty + physical + load_penalty + combined) / 3;
        hand.swings = 1 + u32::from(score > 66) + u32::from(score > 99);
        if definition(0).is_some_and(|d| d.kind == SINGLE_SWING) {
            hand.swings = 1;
        }
        let mut strength_divisor = 1;
        let mut strength_helps = true;
        if let Some(d) = d {
            hand.to_hit = i32::from(d.hit_bonus);
            if holds[other] == Holds::Ammunition
                && let Some(o) = definition(other)
            {
                hand.to_hit += i32::from(o.hit_bonus);
            }
            if is_launcher(d.kind) {
                hand.damage_percent += i32::from(d.damage.base) * 10;
            }
            // Weapons only thrown, lashed or shot.
            if d.attack_modes.is_subset(AttackMode::Throw | AttackMode::Lash | AttackMode::Shoot) {
                if NO_STRENGTH.contains(&d.kind) {
                    strength_helps = false;
                } else {
                    strength_divisor = 2;
                }
            }
        }
        if strength_helps {
            if strength < 50 {
                hand.to_hit -= (50 - strength) / (strength_divisor * 10);
                hand.damage_percent -= (50 - strength) / strength_divisor;
            } else if strength > 50 {
                strength_divisor *= index as i32 + 1;
                hand.to_hit += (strength - 50) / (strength_divisor * 10);
                hand.damage_percent += (strength * 2 - 100) / strength_divisor;
            }
        }
        hand.to_hit += (dexterity - 50) / 10;
        if senses < 30 {
            hand.to_hit -= (30 - senses) / 10;
        } else if senses > 70 {
            hand.to_hit += (senses - 70) / 10;
        }
        let mut load_penalty = -load.penalty();
        if index == 1 {
            load_penalty = load_penalty * 3 / 2;
        }
        hand.to_hit += halved(load_penalty);
        if ranged && c.skill_available(Skill::EagleEye) {
            hand.to_hit += c.skill(Skill::EagleEye) as i32 / 20 + 1;
        }
        if !ranged && c.skill_available(Skill::PowerStrike) {
            hand.to_hit += c.skill(Skill::PowerStrike) as i32 / 20 + 1;
        }
        hand
    };
    Hands { hands: [hand(0), hand(1)], dual_wielding }
}

/// The main hand's numbers, as the character sheet shows them.
pub fn primary_hand(c: &Character, carried: &Carried, catalog: &ItemCatalog, load: Load) -> Hand {
    let [primary, _] = hands(c, carried, catalog, load).hands;
    primary
}

/// A member's total and per-location armour, without skills, spells or
/// conditions. Members carry theirs, kept current by `sync_armour`.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
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

/// A character's armour (retail `CalcArmorClasses`). Defending adds 2
/// everywhere and lets Shield skill count for more.
pub fn armour(
    c: &Character,
    carried: &Carried,
    catalog: &ItemCatalog,
    load: Load,
    defending: bool,
) -> Armour {
    let definition = |slot| carried.worn(slot).and_then(|i| catalog.get(i.item));
    let speed = c.attribute(Attribute::Speed);
    let speed = i32::from(speed > 79) + i32::from(speed > 89)
        - i32::from(speed < 20)
        - i32::from(speed < 10);
    let mut total = i32::from(c.race == Race::Faerie) * 2 + speed - load.armour_penalty()
        + if defending { 2 } else { 0 };
    let mut shield = 0;
    // Shields and magic items worn in the misc, cloak and hand slots.
    for slot in [
        EquipSlot::Misc1,
        EquipSlot::Misc2,
        EquipSlot::Cloak,
        EquipSlot::Primary,
        EquipSlot::Secondary,
    ] {
        if let Some(d) = definition(slot) {
            total += i32::from(d.armor_class_bonus);
            if d.class == EquipClass::Shield {
                let bonus = i32::from(d.armor_class_bonus);
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
        total + worn.map_or(0, |d: &ItemDefinition| i32::from(d.armor_class_bonus))
    });
    Armour { total, by_location, speed, shield }
}

/// How a blow catches its target (retail `unaware` and `natural_attack`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Surprise {
    /// Struck from behind, unseen.
    pub unaware: bool,
    /// A rogue's backstab, or a monster's natural ambush.
    pub natural: bool,
}

impl Surprise {
    fn any(self) -> bool {
        self.unaware || self.natural
    }
}

/// Where a blow lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Location {
    Member(BodyPart),
    /// One of a monster's seven locations.
    Monster(usize),
}

/// Whoever a blow is aimed at, as its rolls see them.
#[derive(Clone, Copy, Debug)]
pub struct Defence<'a> {
    defender: Defender<'a>,
    /// Senses + speed + dexterity + intelligence.
    ssdi: i32,
    /// On the move this round: the party running, a monster advancing.
    moving: bool,
    /// Braced: a member defending.
    defending: bool,
}

#[derive(Clone, Copy, Debug)]
enum Defender<'a> {
    Member { armour: Armour, reduction: i32, max_hp: u32 },
    Monster { species: &'a Species, fatigue: i32, max_hp: u32 },
}

impl<'a> Defence<'a> {
    pub fn member(c: &Character, armour: Armour, running: bool, defending: bool) -> Self {
        Defence {
            defender: Defender::Member {
                armour,
                reduction: c.damage_reduction(),
                max_hp: c.max_hp(),
            },
            ssdi: character_ssdi(c),
            moving: running,
            defending,
        }
    }

    pub fn monster(species: &'a Species, monster: &Monster, advancing: bool) -> Self {
        Defence {
            defender: Defender::Monster {
                species,
                fatigue: monster_fatigue(monster),
                max_hp: monster.max_hp,
            },
            ssdi: species_ssdi(species),
            moving: advancing,
            defending: false,
        }
    }

    /// The situational adjustment against one mode (retail
    /// `GetTargetArmorClassModifier`): a monster's fatigue, and the guard a
    /// surprised target or one distracted by moving loses.
    fn modifier(&self, mode: AttackMode, surprise: Surprise) -> i32 {
        let mut modifier = match self.defender {
            Defender::Monster { species, fatigue, .. } => {
                fatigue / 10 - 2 * i32::from(surprise.any() && species.vulnerable_from_behind)
            }
            Defender::Member { armour, .. } if surprise.any() => {
                -armour.speed.max(0) - armour.shield.max(0)
            }
            Defender::Member { .. } => 0,
        };
        if self.moving && !keeps_distance(mode) {
            modifier -= 4;
        }
        modifier
    }

    /// Armour against one mode before the location (retail
    /// `GetTargetArmorClass`): a monster's evasion, a member's total.
    fn armour_class(&self, mode: AttackMode, surprise: Surprise) -> i32 {
        let base = match self.defender {
            Defender::Monster { species, .. } => 10 - i32::from(species.evasion),
            Defender::Member { armour, .. } => armour.total,
        };
        base + self.modifier(mode, surprise)
    }

    /// Armour at the location struck (retail `TargetArmorClassAtLocation`),
    /// which counts the situational adjustment twice.
    fn armour_at(&self, mode: AttackMode, surprise: Surprise, at: Location) -> i32 {
        let modifier = self.modifier(mode, surprise);
        let base = self.armour_class(mode, surprise);
        match (self.defender, at) {
            (Defender::Monster { species, .. }, Location::Monster(location)) => {
                modifier
                    - i32::from(species.armour_by_mode[mode])
                    - i32::from(species.armour_by_location[location])
                    + i32::from(species.evasion)
                    + base
            }
            (Defender::Member { armour, .. }, Location::Member(part)) => {
                modifier + armour.by_location[part] - armour.total + base
            }
            _ => base + modifier,
        }
    }

    /// Where a blow from the party, or from a monster aiming by its own
    /// chances, lands on this target.
    pub fn locate(&self, rng: &mut GameRng, from: Option<&Species>) -> Location {
        match (self.defender, from) {
            (Defender::Monster { species, .. }, _) => Location::Monster(location(
                rng,
                species.hit_locations.into_iter().enumerate(),
                MONSTER_BODY,
            )),
            (Defender::Member { .. }, Some(attacker)) => {
                Location::Member(location(rng, attacker.hits_on_party, BodyPart::Body))
            }
            (Defender::Member { .. }, None) => Location::Member(location(
                rng,
                [BodyPart::Head, BodyPart::Body, BodyPart::Leg, BodyPart::Hand, BodyPart::Foot]
                    .into_iter()
                    .zip(MEMBER_LOCATIONS),
                BodyPart::Body,
            )),
        }
    }

    /// Damage after the target's own reduction (retail
    /// `ApplyDamageReduction`, `ApplyCharacterDamageReduction`).
    fn reduce(&self, damage: i32) -> u32 {
        let reduction = match self.defender {
            Defender::Monster { species, .. } => i32::from(species.damage_reduction),
            Defender::Member { reduction, .. } => reduction,
        };
        let damage = if reduction != 0 { ((100 - reduction) * damage + 50) / 100 } else { damage };
        damage.max(0) as u32
    }

    /// The most one blow that went astray takes off it: a fifth of its
    /// health, at least one (retail `CapAttackDamageByTargetHealth`).
    pub fn fumble_cap(&self, damage: u32) -> u32 {
        let health = match self.defender {
            Defender::Member { max_hp, .. } | Defender::Monster { max_hp, .. } => max_hp,
        };
        damage.min((health / 5).max(1))
    }

    /// The species, when the target is a monster.
    pub fn species(&self) -> Option<&'a Species> {
        match self.defender {
            Defender::Monster { species, .. } => Some(species),
            Defender::Member { .. } => None,
        }
    }
}

/// A member's aim with one hand (retail `GetTargetAttackAttributes`):
/// against `target`'s defences, or with none for the odds of a fumble.
#[allow(clippy::too_many_arguments)]
pub fn member_aim(
    c: &Character,
    vitals: &Vitals,
    hand: &Hand,
    mode: AttackMode,
    surprise: Surprise,
    target: Option<&Defence>,
    difficulty: Difficulty,
) -> i32 {
    let mut fatigue = character_fatigue(c, vitals);
    if hand.weapon_skill == Skill::ModernWeapon {
        fatigue >>= 1;
    }
    let mut score = hand.attack_rating
        + hand.to_hit * 5
        + mode_to_hit(mode)
        + 10 * i32::from(surprise.unaware)
        + 10 * i32::from(surprise.natural)
        - fatigue;
    if let Some(target) = target {
        score -= target.armour_class(mode, surprise) * 5;
        score += (character_ssdi(c) - target.ssdi) / 10;
    }
    if vitals.has(Condition::Blind) && score > 10 {
        score = if c.has_trait(Trait::EffectiveWhileBlind) {
            c.scale_by_profession((score - 10) as f32) as i32 + 10
        } else {
            10
        };
    }
    difficulty.for_character(score, vitals.has(Condition::Turncoat))
}

/// A member's chance to get through armour where the blow landed (retail
/// `GetTargetHitAttackAttributes`).
pub fn member_penetration(
    c: &Character,
    vitals: &Vitals,
    hand: &Hand,
    mode: AttackMode,
    surprise: Surprise,
    (target, at): (&Defence, Location),
    difficulty: Difficulty,
) -> i32 {
    let mut score = hand.attack_rating + hand.to_hit * 5 + mode_penetration(mode);
    if hand.weapon_skill != Skill::ModernWeapon {
        score -= character_fatigue(c, vitals);
    }
    score -= target.armour_at(mode, surprise, at) * 5;
    score += (character_ssdi(c) - target.ssdi) / 10;
    difficulty.for_character(score, vitals.has(Condition::Turncoat))
}

/// A monster's aim with one attack (retail `GetMonsterAttackScore`).
pub fn monster_aim(
    (species, monster): (&Species, &Monster),
    attack: &MonsterAttack,
    mode: AttackMode,
    surprise: Surprise,
    target: Option<&Defence>,
    difficulty: Difficulty,
) -> i32 {
    let mut fatigue = monster_fatigue(monster);
    if attack.tires_little() {
        fatigue >>= 1;
    }
    let mut score = i32::from(attack.score)
        + mode_to_hit(mode)
        + 10 * i32::from(surprise.unaware)
        + 10 * i32::from(surprise.natural)
        - fatigue;
    if let Some(target) = target {
        score -= target.armour_class(mode, surprise) * 5;
        score += (species_ssdi(species) - target.ssdi) / 10;
    }
    if monster.conditions.iter().any(|&(c, _)| c == Condition::Blind) && score > 10 {
        score = 10;
    }
    difficulty.for_monster(score, Side::Hostile)
}

/// A monster's chance to get through armour (retail `ContinueMonsterAttack`).
/// Retail takes fatigue off only the attacks that tire little, in full.
pub fn monster_penetration(
    (species, monster): (&Species, &Monster),
    attack: &MonsterAttack,
    mode: AttackMode,
    surprise: Surprise,
    (target, at): (&Defence, Location),
    difficulty: Difficulty,
) -> i32 {
    let mut score = i32::from(attack.score) + mode_penetration(mode);
    if attack.tires_little() {
        score -= monster_fatigue(monster);
    }
    score -= target.armour_at(mode, surprise, at) * 5;
    score += (species_ssdi(species) - target.ssdi) / 10;
    difficulty.for_monster(score, Side::Hostile)
}

/// The chance a swing goes astray onto someone else in reach (retail
/// `GetCombatFumbleChance`): none from an aim of 100 or more, otherwise
/// cubic in what it falls short by and growing with who could be struck.
pub fn fumble_chance(bystanders: usize, aim: i32) -> u32 {
    if aim >= 100 {
        return 0;
    }
    let chance = (f64::from(100 - aim).powi(3) * 1e-5 + 0.5) as i64;
    (bystanders as i64 * chance * 10 / 100).clamp(0, 100) as u32
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
fn backstab_dice(rng: &mut GameRng, chance: i32) -> u32 {
    let roll = rng.below(100) as i32;
    [15, 75, 95].iter().take_while(|&&over| roll + over < chance).count() as u32
}

/// How many times a landed blow's dice are rolled (retail
/// `ResolveCharacterAttackDamage`, `ResolveMonsterAttackDamage`): twice
/// against a moving or unaware target, more for a backstab of
/// `backstab` percent and for a Berserk blow, one less against a braced
/// target that saw it coming, and one more for each slaying weapon.
pub fn dice_count(
    rng: &mut GameRng,
    mode: AttackMode,
    target: &Defence,
    surprise: Surprise,
    backstab: i32,
    slaying: u32,
) -> u32 {
    let mut count = 1;
    if !keeps_distance(mode) {
        if target.moving || surprise.unaware {
            count = 2;
        }
        if surprise.natural {
            count += backstab_dice(rng, backstab);
        }
    }
    if mode == AttackMode::Berserk {
        count += 1;
    }
    if count > 1 && target.defending && !surprise.unaware {
        count -= 1;
    }
    count + slaying
}

/// Roll `dice` `count` times (retail `RollDice` with a roll count).
pub fn roll_dice(rng: &mut GameRng, dice: Dice, count: u32) -> u32 {
    (0..count).map(|_| rng.roll(dice)).sum()
}

/// A member's damage from rolled dice (retail `ResolveCharacterAttackDamage`):
/// raised by the hand's damage percent, at least 1, then cut by the
/// target's reduction.
pub fn member_damage(hand: &Hand, rolled: u32, target: &Defence) -> u32 {
    let damage = (((100 + hand.damage_percent) * rolled as i32 + 50) / 100).max(1);
    target.reduce(damage)
}

/// A monster's damage from rolled dice, at least 1, cut by the target's
/// reduction (retail `ResolveMonsterAttackDamage`).
pub fn monster_damage(rolled: u32, target: &Defence) -> u32 {
    target.reduce((rolled as i32).max(1))
}

/// The effects' strength (retail effect power level): the attacker's level
/// and as much again up to 15.
pub fn effect_power(level: u32) -> u32 {
    level + if level > 14 { 15 } else { level }
}

/// The kill chance a member's training adds to a blow (retail
/// `ResolveCharacterAttack`): Critical Strike for close combat, Ranged
/// Combat for a Ranger's bows and slings, Critical Strike for a Ninja's
/// throws; a point per 25 skill and a chance at one more.
pub fn critical_chance(rng: &mut GameRng, c: &Character, hand: &Hand) -> u8 {
    let skill = if !hand.ranged {
        c.skill(Skill::CriticalStrike)
    } else if matches!(hand.weapon_skill, Skill::ModernWeapon | Skill::Bow)
        || (hand.weapon_skill == Skill::ThrowingAndSling && hand.kind == Some(SLING))
    {
        if c.has_trait(Trait::RangedCriticals) { c.skill(Skill::RangedCombat) } else { 0 }
    } else if hand.weapon_skill == Skill::ThrowingAndSling && c.has_trait(Trait::ThrownCriticals) {
        c.skill(Skill::CriticalStrike)
    } else {
        0
    };
    if skill == 0 {
        return 0;
    }
    let mut bonus = skill / 25;
    if skill % 25 != 0 && rng.below(25) < skill % 25 {
        bonus += 1;
    }
    bonus as u8
}

/// A Ninja's thrown weapons always get through armour when they strike
/// (retail `W8_TRAIT_THROWN_CRITICALS` for ranged thrown weapons other than
/// slings).
pub fn sure_penetration(c: &Character, hand: &Hand) -> bool {
    c.has_trait(Trait::ThrownCriticals)
        && hand.ranged
        && hand.weapon_skill == Skill::ThrowingAndSling
        && hand.kind != Some(SLING)
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
pub fn phase(rng: &mut GameRng, initiative: i32) -> u8 {
    let v = rng.below(4) as i32 + initiative;
    (90 - v.clamp(-10, 89)) as u8
}

/// The character's initiative before the roll, weapon included.
pub fn character_initiative(c: &Character, hand: &Hand, load: Load) -> i32 {
    c.initiative(load) + hand.initiative
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
    fn modes_are_chosen_from_the_set_bits_round_robin() {
        // Each draw below 9 walks the set bits round: with two modes the
        // first takes the even draws (5 in 9), with three each takes 3 in 9.
        let (mut rng, mut twin) = (GameRng::seeded(3), GameRng::seeded(3));
        for _ in 0..50 {
            let expected = match twin.below(9) % 2 {
                0 => AttackMode::Swing,
                _ => AttackMode::Shoot,
            };
            assert_eq!(choose_mode(&mut rng, AttackMode::Shoot | AttackMode::Swing), expected);
            let expected = [AttackMode::Thrust, AttackMode::Kick, AttackMode::Lash]
                [twin.below(9) as usize % 3];
            let modes = AttackMode::Lash | AttackMode::Thrust | AttackMode::Kick;
            assert_eq!(choose_mode(&mut rng, modes), expected);
        }
        assert_eq!(choose_mode(&mut rng, EnumSet::empty()), AttackMode::Swing);
    }

    #[test]
    fn a_roll_of_five_lands_over_ninety_five_misses_and_fumbles_go_astray() {
        for seed in 0..200 {
            let roll = GameRng::seeded(seed).below(100) + 1;
            match swing(&mut GameRng::seeded(seed), 3, 30) {
                Aim::Reaches { roll: r, sure } => {
                    assert_eq!((r, sure), (roll, roll <= 5));
                    assert!(roll <= 70);
                }
                // At a 30% fumble chance a roll over 95 goes astray first.
                Aim::Missed => panic!("{roll} should have gone astray"),
                Aim::Fumbled { bystander, .. } => assert!(roll > 70 && bystander < 3),
            }
            // With nobody else in reach, over 95 simply misses.
            let alone = swing(&mut GameRng::seeded(seed), 0, 30);
            assert_eq!(alone == Aim::Missed, roll > 95);
        }
        assert_eq!(fumble_chance(1, 100), 0);
        // 50 short: round(1.25) = 1, then 1 · 3 bystanders · 10 / 100 = 0.
        assert_eq!(fumble_chance(3, 50), 0);
        // 100 short: 10 a bystander, at a tenth of a percent each.
        assert_eq!(fumble_chance(4, 0), 4);
        assert_eq!(fumble_chance(20, -100), 100);
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
            let standing = armour(bearer, &carried, &catalog, Load::None, false);
            let defending = armour(bearer, &carried, &catalog, Load::None, true);
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
        assert_eq!(armour(&bearer, &carried, &catalog, Load::None, false).shield, 4 + 4);
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
        wield(&mut carried, EquipSlot::Primary, &sword, 1);
        let alone = hands(&fighter, &carried, &catalog, Load::None);
        assert!(alone.hands[0].in_play && !alone.dual_wielding);
        // An empty off hand beside a weapon does not punch.
        assert!(!alone.hands[1].in_play);
        wield(&mut carried, EquipSlot::Secondary, &dagger, 1);
        let both = hands(&fighter, &carried, &catalog, Load::None);
        assert!(both.dual_wielding && both.hands[1].in_play);
        assert_eq!(both.hands[1].weapon_skill, Skill::Dagger);
        // Each hand pays −10 per hand number and a quarter of the Dual
        // Weapons it lacks: (−10 − 15)/3 and (−20 − 15)/3 off the rating.
        let rating = |h: &Hand| h.attack_rating;
        assert!(rating(&both.hands[0]) > rating(&both.hands[1]));
        assert_eq!(both.choose(), Some(0));
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
        bow.kind = 0x0b;
        bow.shoots = Some(crate::content::items::Ammunition::Arrow);
        bow.reach = Reach::Long;
        bow.damage = Dice { base: 2, count: 1, sides: 4 };
        let mut arrows = weapon(3, EquipClass::Ammunition, Skill::Bow);
        arrows.ammunition = Some(crate::content::items::Ammunition::Arrow);
        arrows.damage = Dice { base: 0, count: 1, sides: 8 };
        arrows.effects.chances[AttackEffect::Poison] = 10;
        let catalog = ItemCatalog(vec![sword.clone(), shield.clone(), bow.clone(), arrows.clone()]);
        let ranger = Character::new("A", Race::Human, Gender::Male, Profession::Ranger);
        let mut carried = Carried::default();
        wield(&mut carried, EquipSlot::Primary, &sword, 1);
        wield(&mut carried, EquipSlot::Secondary, &shield, 1);
        assert!(!hands(&ranger, &carried, &catalog, Load::None).hands[1].in_play);
        // A bow without arrows cannot shoot; with them it shoots theirs.
        let mut carried = Carried::default();
        wield(&mut carried, EquipSlot::Primary, &bow, 1);
        assert!(!hands(&ranger, &carried, &catalog, Load::None).hands[0].in_play);
        wield(&mut carried, EquipSlot::Secondary, &arrows, 20);
        let [bow_hand, arrow_hand] = hands(&ranger, &carried, &catalog, Load::None).hands;
        assert!(bow_hand.in_play && bow_hand.paired && bow_hand.ranged);
        assert!(!arrow_hand.in_play);
        // The bow pierces by the arrows' own modes.
        assert_eq!(arrow_hand.modes, arrows.attack_modes);
        assert_eq!(bow_hand.dice, arrows.damage);
        assert_eq!(bow_hand.effects.chances[AttackEffect::Poison], 10);
        // The bow's own base is a damage bonus: +20%, with strength's.
        let strength = ranger.attribute(Attribute::Strength) as i32;
        let from_strength = match strength {
            ..50 => -(50 - strength),
            50 => 0,
            _ => 2 * strength - 100,
        };
        assert_eq!(bow_hand.damage_percent, 20 + from_strength);
    }

    #[test]
    fn surprise_strips_speed_and_shield_and_moving_targets_drop_their_guard() {
        let armour = Armour { total: 8, speed: 2, shield: 3, ..Armour::default() };
        let defender = Character::new("D", Race::Human, Gender::Male, Profession::Fighter);
        let standing = Defence::member(&defender, armour, false, false);
        let swing = AttackMode::Swing;
        assert_eq!(standing.armour_class(swing, Surprise::default()), 8);
        let behind = Surprise { unaware: true, natural: false };
        assert_eq!(standing.armour_class(swing, behind), 8 - 2 - 3);
        let running = Defence::member(&defender, armour, true, false);
        assert_eq!(running.armour_class(swing, Surprise::default()), 8 - 4);
        // Shots and throws do not find the gap.
        assert_eq!(running.armour_class(AttackMode::Shoot, Surprise::default()), 8);
    }

    #[test]
    fn extra_dice_come_from_surprise_berserk_and_slaying_and_bracing_takes_one() {
        let armour = Armour::default();
        let defender = Character::new("D", Race::Human, Gender::Male, Profession::Fighter);
        let rng = &mut GameRng::seeded(0);
        let still = Defence::member(&defender, armour, false, false);
        let swing = AttackMode::Swing;
        assert_eq!(dice_count(rng, swing, &still, Surprise::default(), 0, 0), 1);
        let behind = Surprise { unaware: true, natural: false };
        assert_eq!(dice_count(rng, swing, &still, behind, 0, 0), 2);
        assert_eq!(dice_count(rng, AttackMode::Berserk, &still, Surprise::default(), 0, 1), 3);
        let braced = Defence::member(&defender, armour, true, true);
        assert_eq!(dice_count(rng, swing, &braced, Surprise::default(), 0, 0), 1);
        assert_eq!(dice_count(rng, swing, &braced, behind, 0, 0), 2);
        // A backstab of 100 adds one for rolls below 85, two below 25 and
        // three below 5.
        let backstab = Surprise { unaware: false, natural: true };
        for seed in 0..50 {
            let roll = GameRng::seeded(seed).below(100);
            let n = dice_count(&mut GameRng::seeded(seed), swing, &still, backstab, 100, 0);
            assert_eq!(n, 1 + [85, 25, 5].iter().filter(|&&under| roll < under).count() as u32);
        }
        // Shots never roll the surprise dice.
        assert_eq!(dice_count(rng, AttackMode::Shoot, &still, behind, 0, 0), 1);
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
