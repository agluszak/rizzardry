//! Weapon attacks both ways, as ordinary functions over the fighters'
//! numbers (`docs/research/combat.md` §3-§5, `characters.md` §4.7-§4.8).
//! Fumbles, criticals, knockouts, backstabs, unaware targets and attack
//! effects are not modelled yet; missiles hit by the same roll as melee
//! (a deliberate approximation of retail's scattered aim point).

use crate::content::items::{Dice, EquipClass, ItemCatalog, ItemDefinition};
use crate::content::species::{MonsterAttack, Reach, Species};
use crate::game::characters::{
    Character, DEXTERITY, INTELLIGENCE, Load, Race, SENSES, SPEED, STRENGTH,
};
use crate::game::inventory::{Carried, EquipSlot};
use crate::game::rng::GameRng;

/// How a blow is struck, in retail's mode-bit order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Swing,
    Thrust,
    Bash,
    Berserk,
    Throw,
    Punch,
    Kick,
    Lash,
    Shoot,
}

const MODES: [Mode; 9] = [
    Mode::Swing,
    Mode::Thrust,
    Mode::Bash,
    Mode::Berserk,
    Mode::Throw,
    Mode::Punch,
    Mode::Kick,
    Mode::Lash,
    Mode::Shoot,
];
/// To-hit and penetration terms by mode.
const TO_HIT: [i32; 9] = [0, -10, 0, -30, -5, 5, -10, 10, 0];
const PENETRATION: [i32; 9] = [0, 10, -5, -30, 5, -5, 0, -10, 10];

impl Mode {
    fn index(self) -> usize {
        MODES.iter().position(|&m| m == self).unwrap_or(0)
    }

    /// Pick one of the set mode bits (retail `ChooseAttackMode`): draw
    /// `w` below 9, then walk the bits round from the first, taking the
    /// (w+1)-th set one. Not uniform when the count does not divide 9.
    pub fn choose(rng: &mut GameRng, modes: u16) -> Mode {
        let modes = modes & 0x1ff;
        if modes == 0 {
            return Mode::Swing;
        }
        let mut wanted = rng.below(9) + 1;
        let mut bit = 0;
        loop {
            if modes & (1 << bit) != 0 {
                wanted -= 1;
                if wanted == 0 {
                    return MODES[bit];
                }
            }
            bit = (bit + 1) % 9;
        }
    }
}

/// Character hit locations: head, body, leg, hand, foot, with the
/// equipment slot whose armour covers each.
const LOCATION_SLOTS: [EquipSlot; 5] =
    [EquipSlot::Head, EquipSlot::Torso, EquipSlot::Legs, EquipSlot::Hands, EquipSlot::Feet];
const BODY: usize = 1;
/// A monster's body, its fallback location.
const MONSTER_BODY: usize = 3;

/// Where a blow lands, by cumulative percentages against one `below(100)`
/// draw; anything past them hits `fallback`.
fn location(rng: &mut GameRng, chances: &[u8], fallback: usize) -> usize {
    let roll = rng.below(100);
    let mut total = 0;
    for (i, &chance) in chances.iter().enumerate() {
        total += u32::from(chance);
        if roll < total {
            return i;
        }
    }
    fallback
}

/// Senses + speed + dexterity + intelligence, which the attribute term
/// compares.
fn character_ssdi(c: &Character) -> i32 {
    [SENSES, SPEED, DEXTERITY, INTELLIGENCE].iter().map(|&a| c.attribute(a) as i32).sum()
}

/// A species' attributes are strength, intelligence, dexterity, speed,
/// senses.
fn species_ssdi(s: &Species) -> i32 {
    s.attributes[1..].iter().map(|&a| i32::from(a)).sum()
}

/// To-hit penalty by fatigue band.
fn fatigue_penalty(c: &Character) -> i32 {
    [0, 5, 10, 20, 40][c.fatigue_band()]
}

/// A member's main hand, as the character sheet works it out.
#[derive(Clone, Debug, PartialEq)]
pub struct Hand {
    pub reach: Reach,
    pub dice: Dice,
    /// The ways it strikes, a bit per mode; one is chosen per attack.
    pub modes: u16,
    pub attacks: u32,
    pub swings: u32,
    pub attack_rating: i32,
    /// The sheet's "To Hit": weapon, strength, dexterity and senses.
    pub to_hit: i32,
    /// The sheet's "To Damage (%)": strength.
    pub damage_percent: i32,
    /// The weapon's initiative byte (retail `damage_bonus`).
    pub initiative: i32,
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
    Dice { base: base + 2, count, sides }
}

/// The primary hand's numbers (`characters.md` §4.7), without a second
/// weapon or load.
pub fn primary_hand(c: &Character, carried: &Carried, catalog: &ItemCatalog) -> Hand {
    let weapon = carried
        .worn(EquipSlot::Primary)
        .and_then(|i| catalog.get(i.item))
        .filter(|d| d.weapon_skill.is_some());
    // Martial Arts for an empty hand.
    let weapon_skill = weapon.and_then(|d| d.weapon_skill).map_or(14, usize::from);
    let ranged = (7..=9).contains(&weapon_skill);
    let combat = c.skill(if ranged { 17 } else { 16 }) as i32;
    let skill = c.skill(weapon_skill) as i32;
    let physical: i32 = c
        .profession_levels
        .iter()
        .enumerate()
        .map(|(p, &l)| i32::from(l) * [4, 4, 4, 4, 4, 4, 4, 3, 3, 3, 2, 2, 2, 2, 2][p])
        .sum();
    let combined = (2 * (combat + 2 * skill) / 3) * 10 / 20;
    let (str, dex, spd, sen) = (
        c.attribute(STRENGTH) as i32,
        c.attribute(DEXTERITY) as i32,
        c.attribute(SPEED) as i32,
        c.attribute(SENSES) as i32,
    );
    let attack_rating = (dex / 2 + 2 * combined + physical) / 3 + 60;
    let s = ((spd + dex) / 2 + physical + combined) / 3;
    let attacks = 1 + u32::from(s > 49) + u32::from(s > 99);
    let damage_bonus = weapon.map_or(combined / 10, |d| i32::from(d.damage_bonus));
    let swing_score = (spd + 10 * damage_bonus + physical + combined) / 3;
    let swings = 1 + u32::from(swing_score > 66) + u32::from(swing_score > 99);
    let (str_hit, str_damage) =
        if str < 50 { (-(50 - str) / 10, -(50 - str)) } else { ((str - 50) / 10, 2 * str - 100) };
    let sen_hit = if sen < 30 {
        -(30 - sen) / 10
    } else if sen > 70 {
        (sen - 70) / 10
    } else {
        0
    };
    let to_hit = weapon.map_or(0, |d| i32::from(d.hit_bonus)) + str_hit + (dex - 50) / 10 + sen_hit;
    Hand {
        reach: weapon.map_or(Reach::Touch, |d| Reach::from_retail(d.range)),
        dice: weapon.map_or_else(|| unarmed_dice(c.skill(14)), |d| d.damage),
        // An empty hand punches, and kicks too past Martial Arts 4
        // (retail GameplayCode.cpp:696-699).
        modes: weapon.map_or(if c.skill(14) > 4 { 0x60 } else { 0x20 }, |d| d.attack_modes),
        attacks,
        swings,
        attack_rating,
        to_hit,
        damage_percent: str_damage,
        initiative: damage_bonus,
    }
}

/// A member's armour class: the total, and by hit location with that
/// location's armour (`characters.md` §4.8, without skills, spells,
/// conditions, defending or load).
#[derive(Clone, Debug, PartialEq)]
pub struct Armour {
    pub total: i32,
    pub by_location: [i32; 5],
}

pub fn armour(c: &Character, carried: &Carried, catalog: &ItemCatalog) -> Armour {
    let definition = |slot| carried.worn(slot).and_then(|i| catalog.get(i.item));
    let speed = c.attribute(SPEED);
    let mut total =
        i32::from(c.race == Race::Faerie) * 2 + i32::from(speed > 79) + i32::from(speed > 89)
            - i32::from(speed < 20)
            - i32::from(speed < 10);
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
                total += (c.skill(6) as i32 / 25).min(i32::from(d.armor_class_bonus));
            }
        }
    }
    let by_location = LOCATION_SLOTS.map(|slot| {
        total + definition(slot).map_or(0, |d: &ItemDefinition| i32::from(d.armor_class_bonus))
    });
    Armour { total, by_location }
}

/// What one swing did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Swing {
    Missed,
    NoPenetration,
    /// Damage dealt, where.
    Hit {
        damage: u32,
        location: usize,
    },
}

/// One roll decides hit and penetration: 5 or under always hits and
/// penetrates, over 95 always misses.
fn resolve(roll: u32, to_hit: i32, penetration: i32) -> Option<bool> {
    let roll = roll as i32;
    if roll > 95 {
        None
    } else if roll <= 5 {
        Some(true)
    } else if roll > to_hit {
        None
    } else {
        Some(roll <= penetration)
    }
}

/// A member's swing at a monster (retail `ContinueCharacterAttack`), in
/// retail's draw order: the roll, the location, the damage dice.
pub fn character_swing(
    rng: &mut GameRng,
    attacker: &Character,
    hand: &Hand,
    mode: Mode,
    species: &Species,
) -> Swing {
    let mode = mode.index();
    let attributes = (character_ssdi(attacker) - species_ssdi(species)) / 10;
    let base = hand.attack_rating + 5 * hand.to_hit - fatigue_penalty(attacker) + attributes;
    let to_hit = base + TO_HIT[mode] - 5 * (10 - i32::from(species.evasion));
    let roll = rng.below(100) + 1;
    let location = location(rng, &species.hit_locations, MONSTER_BODY);
    let armour_at = 10
        - i32::from(species.armour_by_mode[mode])
        - i32::from(species.armour_by_location[location]);
    let penetration = base + PENETRATION[mode] - 5 * armour_at;
    match resolve(roll, to_hit, penetration) {
        None => Swing::Missed,
        Some(false) => Swing::NoPenetration,
        Some(true) => {
            let rolled = rng.roll(hand.dice) as i32;
            let damage = (((100 + hand.damage_percent) * rolled + 50) / 100).max(1);
            let reduction = i32::from(species.damage_reduction);
            let damage = (((100 - reduction) * damage + 50) / 100).max(0) as u32;
            Swing::Hit { damage, location }
        }
    }
}

/// A monster's swing at a member (retail `ContinueMonsterAttack`).
pub fn monster_swing(
    rng: &mut GameRng,
    species: &Species,
    attack: &MonsterAttack,
    mode: Mode,
    target: &Character,
    armour: &Armour,
) -> Swing {
    let mode = mode.index();
    let attributes = (species_ssdi(species) - character_ssdi(target)) / 10;
    let base = i32::from(attack.score) + attributes;
    let to_hit = base + TO_HIT[mode] - 5 * armour.total;
    let roll = rng.below(100) + 1;
    let location = location(rng, &species.hits_on_party, BODY);
    let penetration = base + PENETRATION[mode] - 5 * armour.by_location[location];
    match resolve(roll, to_hit, penetration) {
        None => Swing::Missed,
        Some(false) => Swing::NoPenetration,
        Some(true) => Swing::Hit { damage: rng.roll(attack.damage).max(1), location },
    }
}

/// When an actor acts in the round, 1 (first) to 100: a d4 plus its
/// initiative, highest first.
pub fn phase(rng: &mut GameRng, initiative: i32) -> u8 {
    let v = rng.below(4) as i32 + initiative;
    (90 - v.clamp(-10, 89)) as u8
}

/// The character's initiative before the roll, weapon included.
pub fn character_initiative(c: &Character, hand: &Hand) -> i32 {
    c.initiative(Load::None) + hand.initiative
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_are_chosen_from_the_set_bits_round_robin() {
        let mut rng = GameRng::seeded(3);
        for _ in 0..50 {
            assert_eq!(Mode::choose(&mut rng, 1 << 7), Mode::Lash);
            assert!(matches!(Mode::choose(&mut rng, 0b11), Mode::Swing | Mode::Thrust));
        }
    }

    #[test]
    fn one_roll_decides_hit_and_penetration() {
        assert_eq!(resolve(3, -50, -50), Some(true), "5 or under always lands");
        assert_eq!(resolve(96, 200, 200), None, "over 95 always misses");
        assert_eq!(resolve(60, 50, 80), None);
        assert_eq!(resolve(40, 50, 30), Some(false));
        assert_eq!(resolve(40, 50, 45), Some(true));
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
}

#[cfg(test)]
mod retail_tests {
    use super::*;
    use crate::content::items::ItemId;
    use crate::game::characters::{Gender, Profession};
    use crate::game::inventory::{ItemIds, make_item, place_at_creation};

    #[test]
    #[ignore = "requires a retail installation"]
    fn a_fighter_and_a_crab_trade_blows() {
        let root =
            std::env::var("RIZZARDRY_RETAIL").expect("set RIZZARDRY_RETAIL to run this test");
        let files = crate::retail::files::RetailFiles::open(root).unwrap();
        let catalog = crate::retail::items::retail_catalog(&files);
        let species = crate::retail::species::read_species(
            &files.read("data/databases/monsters.dbs").unwrap().unwrap(),
        )
        .unwrap();
        let crab = species.iter().find(|s| s.name.contains("Crab")).unwrap();
        let fighter = Character::new("Ragnar", Race::Human, Gender::Male, Profession::Fighter);
        let (mut ids, mut rng) = (ItemIds::default(), GameRng::seeded(9));
        let mut carried = Carried::default();
        for id in [143, 153, 203, 246, 7] {
            let item = make_item(catalog.get(ItemId(id)).unwrap(), &mut ids, &mut rng, true, true);
            place_at_creation(&mut carried, item, &catalog, &fighter).unwrap();
        }
        let hand = primary_hand(&fighter, &carried, &catalog);
        let armour = armour(&fighter, &carried, &catalog);
        println!("{} {:?}", crab.name, crab.attacks);
        println!("{hand:?}\n{armour:?}");
        let tally = |swings: Vec<Swing>| {
            let hits: Vec<u32> = swings
                .iter()
                .filter_map(|s| match s {
                    Swing::Hit { damage, .. } => Some(*damage),
                    _ => None,
                })
                .collect();
            (hits.len(), hits.iter().sum::<u32>() as f32 / hits.len().max(1) as f32)
        };
        let ours = tally(
            (0..1000)
                .map(|_| character_swing(&mut rng, &fighter, &hand, Mode::Swing, crab))
                .collect(),
        );
        let attack = &crab.attacks[0];
        let theirs = tally(
            (0..1000)
                .map(|_| {
                    let mode = Mode::choose(&mut rng, attack.modes);
                    monster_swing(&mut rng, crab, attack, mode, &fighter, &armour)
                })
                .collect(),
        );
        println!("fighter lands {ours:?} of 1000, crab lands {theirs:?}");
        assert!(ours.0 > 100 && ours.0 < 1000, "the fighter sometimes hits and sometimes misses");
        assert!(theirs.0 > 50 && theirs.0 < 1000);
    }
}
