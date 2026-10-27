//! Weapon attacks both ways, as ordinary functions over the fighters'
//! numbers (`docs/research/combat.md` §3-§5, `characters.md` §4.7-§4.8).
//! Fumbles, criticals, knockouts, backstabs, unaware targets and attack
//! effects are not modelled yet; missiles hit by the same roll as melee
//! (a deliberate approximation of retail's scattered aim point).

use enum_map::EnumMap;
use enumset::EnumSet;

use crate::content::characters::{Attribute, BodyPart, Skill};
use crate::content::items::{AttackMode, Dice, EquipClass, ItemCatalog, ItemDefinition};
use crate::content::species::{MonsterAttack, Reach, Species};
use crate::game::characters::{Character, Load, Race, Vitals};
use crate::game::inventory::{Carried, EquipSlot};
use crate::game::rng::GameRng;

/// The to-hit term of a mode.
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

/// The penetration term of a mode.
fn mode_penetration(mode: AttackMode) -> i32 {
    use AttackMode::*;
    match mode {
        Swing | Kick => 0,
        Thrust | Shoot => 10,
        Bash | Throw | Punch => -5,
        Berserk => -30,
        Lash => -10,
    }
}

/// Pick one of the modes (retail `ChooseAttackMode`): draw `w` below 9,
/// then walk the mode bits round from the first, taking the (w+1)-th set
/// one. Not uniform when the count does not divide 9.
pub fn choose_mode(rng: &mut GameRng, modes: EnumSet<AttackMode>) -> AttackMode {
    let wanted = rng.below(9) as usize;
    modes.iter().cycle().nth(wanted).unwrap_or(AttackMode::Swing)
}

/// Stamina spent by one executed swing of an ordinary Attack order.
/// Berserk is a separate accepted order in retail, not merely a weapon mode.
/// Load scaling belongs at the caller once shared party burden is supported.
pub fn swing_stamina(
    rng: &mut GameRng,
    mode: AttackMode,
    weight: Option<u16>,
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
    cost * if diseased { 2 } else { 1 }
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

/// To-hit penalty by the share of stamina spent.
fn fatigue_penalty(c: &Character, vitals: &Vitals) -> i32 {
    match vitals.stamina_spent(c.max_stamina()) {
        0..50 => 0,
        50..70 => 5,
        70..85 => 10,
        85..=94 => 20,
        _ => 40,
    }
}

/// A member's main hand, as the character sheet works it out.
#[derive(Clone, Debug, PartialEq)]
pub struct Hand {
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
    let weapon_skill = weapon.and_then(|d| d.weapon_skill).unwrap_or(Skill::MartialArts);
    let ranged = matches!(weapon_skill, Skill::ModernWeapon | Skill::Bow | Skill::ThrowingAndSling);
    let combat = c.skill(if ranged { Skill::RangedCombat } else { Skill::CloseCombat }) as i32;
    let skill = c.skill(weapon_skill) as i32;
    let physical = c.physical_levels();
    let combined = (2 * (combat + 2 * skill) / 3) * 10 / 20;
    let [str, dex, spd, sen] =
        [Attribute::Strength, Attribute::Dexterity, Attribute::Speed, Attribute::Senses]
            .map(|a| c.attribute(a) as i32);
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
        reach: weapon.map_or(Reach::Touch, |d| d.reach),
        dice: weapon.map_or_else(|| unarmed_dice(c.skill(Skill::MartialArts)), |d| d.damage),
        // An empty hand punches, and kicks too past Martial Arts 4
        // (retail GameplayCode.cpp:696-699).
        modes: weapon.map_or(
            if c.skill(Skill::MartialArts) > 4 {
                AttackMode::Punch | AttackMode::Kick
            } else {
                EnumSet::only(AttackMode::Punch)
            },
            |d| d.attack_modes,
        ),
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
    pub by_location: EnumMap<BodyPart, i32>,
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

pub fn armour(c: &Character, carried: &Carried, catalog: &ItemCatalog) -> Armour {
    let definition = |slot| carried.worn(slot).and_then(|i| catalog.get(i.item));
    let speed = c.attribute(Attribute::Speed);
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
                total += (c.skill(Skill::Shield) as i32 / 25).min(i32::from(d.armor_class_bonus));
            }
        }
    }
    let by_location = EnumMap::from_fn(|part| {
        let worn = definition(armour_slot(part));
        total + worn.map_or(0, |d: &ItemDefinition| i32::from(d.armor_class_bonus))
    });
    Armour { total, by_location }
}

/// What one swing did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Swing {
    Missed,
    NoPenetration,
    Hit(u32),
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
    vitals: &Vitals,
    hand: &Hand,
    mode: AttackMode,
    species: &Species,
) -> Swing {
    let attributes = (character_ssdi(attacker) - species_ssdi(species)) / 10;
    let base =
        hand.attack_rating + 5 * hand.to_hit - fatigue_penalty(attacker, vitals) + attributes;
    let to_hit = base + mode_to_hit(mode) - 5 * (10 - i32::from(species.evasion));
    let roll = rng.below(100) + 1;
    let location = location(rng, species.hit_locations.into_iter().enumerate(), MONSTER_BODY);
    let armour_at = 10
        - i32::from(species.armour_by_mode[mode])
        - i32::from(species.armour_by_location[location]);
    let penetration = base + mode_penetration(mode) - 5 * armour_at;
    match resolve(roll, to_hit, penetration) {
        None => Swing::Missed,
        Some(false) => Swing::NoPenetration,
        Some(true) => {
            let rolled = rng.roll(hand.dice) as i32;
            let damage = (((100 + hand.damage_percent) * rolled + 50) / 100).max(1);
            let reduction = i32::from(species.damage_reduction);
            let damage = (((100 - reduction) * damage + 50) / 100).max(0) as u32;
            Swing::Hit(damage)
        }
    }
}

/// A monster's swing at a member (retail `ContinueMonsterAttack`).
pub fn monster_swing(
    rng: &mut GameRng,
    species: &Species,
    attack: &MonsterAttack,
    mode: AttackMode,
    target: &Character,
    armour: &Armour,
) -> Swing {
    let attributes = (species_ssdi(species) - character_ssdi(target)) / 10;
    let base = i32::from(attack.score) + attributes;
    let to_hit = base + mode_to_hit(mode) - 5 * armour.total;
    let roll = rng.below(100) + 1;
    let location = location(rng, species.hits_on_party, BodyPart::Body);
    let penetration = base + mode_penetration(mode) - 5 * armour.by_location[location];
    match resolve(roll, to_hit, penetration) {
        None => Swing::Missed,
        Some(false) => Swing::NoPenetration,
        Some(true) => Swing::Hit(rng.roll(attack.damage).max(1)),
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
    fn swing_costs_follow_the_attack_and_disease_doubles_them() {
        for (mode, weight, range) in [
            (AttackMode::Punch, Some(800), 2..=4),
            (AttackMode::Kick, Some(800), 3..=5),
            (AttackMode::Swing, None, 3..=6),
            (AttackMode::Swing, Some(0), 2..=2),
            (AttackMode::Swing, Some(80), 4..=6),
        ] {
            for seed in 0..16 {
                let cost = swing_stamina(&mut GameRng::seeded(seed), mode, weight, false);
                assert!(range.contains(&cost));
                assert_eq!(swing_stamina(&mut GameRng::seeded(seed), mode, weight, true), 2 * cost);
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
        let files = crate::retail::files::retail_files();
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
        let tally = |swings: Vec<Swing>| {
            let hits: Vec<u32> = swings
                .iter()
                .filter_map(|s| match s {
                    Swing::Hit(damage) => Some(*damage),
                    _ => None,
                })
                .collect();
            (hits.len(), hits.iter().sum::<u32>() as f32 / hits.len().max(1) as f32)
        };
        let ours = tally(
            (0..1000)
                .map(|_| {
                    character_swing(
                        &mut rng,
                        &fighter,
                        &Vitals::new(&fighter),
                        &hand,
                        AttackMode::Swing,
                        crab,
                    )
                })
                .collect(),
        );
        let attack = &crab.attacks[0];
        let theirs = tally(
            (0..1000)
                .map(|_| {
                    let mode = choose_mode(&mut rng, attack.modes);
                    monster_swing(&mut rng, crab, attack, mode, &fighter, &armour)
                })
                .collect(),
        );
        assert!(ours.0 > 100 && ours.0 < 1000, "the fighter sometimes hits and sometimes misses");
        assert!(theirs.0 > 50 && theirs.0 < 1000);
    }
}
