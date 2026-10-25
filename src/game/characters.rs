//! Party members: who they are, their attributes and skills, and the numbers
//! derived from them. Tables are retail's, hard-coded in its executable (see
//! `docs/research/characters.md`). Spells, conditions, resistances, armour
//! class and attacks are not modelled yet.

use bevy::prelude::*;

use crate::content::items::ItemDefinition;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum Race {
    Human,
    Elf,
    Dwarf,
    Gnome,
    Hobbit,
    Faerie,
    Lizardman,
    Dracon,
    Felpurr,
    Rawulf,
    Mook,
    Trynnie,
    TRang,
    Umpani,
    Rapax,
    Android,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum Profession {
    Fighter,
    Lord,
    Valkyrie,
    Ranger,
    Samurai,
    Ninja,
    Monk,
    Rogue,
    Gadgeteer,
    Bard,
    Priest,
    Alchemist,
    Bishop,
    Psionic,
    Mage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum Gender {
    Male,
    Female,
}

/// Strength, Intelligence, Piety, Vitality, Dexterity, Speed, Senses.
pub const ATTRIBUTES: usize = 7;
pub const STRENGTH: usize = 0;
pub const PIETY: usize = 2;
pub const VITALITY: usize = 3;
pub const SPEED: usize = 5;
pub const SENSES: usize = 6;
pub const SKILLS: usize = 41;
/// The expert skill Snake Speed, which adds to initiative.
const SNAKE_SPEED: usize = 39;

/// Base attributes of the playable races, in [`ATTRIBUTES`] order.
const RACE_ATTRIBUTES: [[u8; ATTRIBUTES]; 11] = [
    [45, 45, 45, 45, 45, 45, 45],
    [35, 50, 50, 35, 50, 45, 40],
    [55, 30, 50, 60, 35, 35, 35],
    [35, 50, 40, 50, 50, 35, 45],
    [40, 40, 30, 45, 55, 50, 50],
    [25, 55, 35, 30, 50, 60, 45],
    [60, 25, 25, 70, 40, 50, 30],
    [55, 35, 30, 60, 50, 40, 30],
    [40, 40, 30, 35, 50, 60, 50],
    [40, 30, 55, 50, 40, 40, 50],
    [50, 50, 25, 50, 35, 35, 55],
];

/// Per profession: attribute minimums (0 = none), HP factor, experience
/// weight, bonus skill and profession skills.
struct ProfessionRow {
    minimums: [u8; ATTRIBUTES],
    hp_factor: f32,
    experience: u32,
    bonus_skill: usize,
    skills: &'static [usize],
}

const PROFESSIONS: [ProfessionRow; 15] = [
    ProfessionRow {
        minimums: [55, 0, 0, 50, 50, 0, 0],
        hp_factor: 9.0,
        experience: 1000,
        bonus_skill: 16,
        skills: &[17, 0, 1, 6],
    },
    ProfessionRow {
        minimums: [55, 0, 55, 55, 50, 50, 0],
        hp_factor: 7.5,
        experience: 1400,
        bonus_skill: 18,
        skills: &[16, 0, 4],
    },
    ProfessionRow {
        minimums: [50, 0, 55, 55, 50, 50, 0],
        hp_factor: 7.5,
        experience: 1400,
        bonus_skill: 2,
        skills: &[16, 21, 1],
    },
    ProfessionRow {
        minimums: [50, 50, 0, 50, 55, 0, 55],
        hp_factor: 6.5,
        experience: 1400,
        bonus_skill: 17,
        skills: &[15, 8, 21],
    },
    ProfessionRow {
        minimums: [50, 55, 0, 50, 55, 55, 0],
        hp_factor: 7.0,
        experience: 1400,
        bonus_skill: 0,
        skills: &[16, 18, 19],
    },
    ProfessionRow {
        minimums: [50, 50, 0, 50, 55, 55, 50],
        hp_factor: 6.5,
        experience: 1600,
        bonus_skill: 19,
        skills: &[16, 14, 9, 11],
    },
    ProfessionRow {
        minimums: [0, 50, 50, 0, 50, 55, 55],
        hp_factor: 6.5,
        experience: 1400,
        bonus_skill: 14,
        skills: &[16, 19, 5, 11],
    },
    ProfessionRow {
        minimums: [0, 0, 0, 0, 55, 50, 50],
        hp_factor: 6.0,
        experience: 1000,
        bonus_skill: 10,
        skills: &[4, 18, 13, 11],
    },
    ProfessionRow {
        minimums: [45, 55, 0, 0, 60, 0, 55],
        hp_factor: 5.5,
        experience: 1000,
        bonus_skill: 7,
        skills: &[17, 23, 10],
    },
    ProfessionRow {
        minimums: [45, 50, 0, 0, 55, 0, 55],
        hp_factor: 6.0,
        experience: 1000,
        bonus_skill: 22,
        skills: &[12, 21, 20],
    },
    ProfessionRow {
        minimums: [0, 0, 60, 55, 0, 0, 0],
        hp_factor: 5.0,
        experience: 1200,
        bonus_skill: 25,
        skills: &[3, 5, 22],
    },
    ProfessionRow {
        minimums: [0, 55, 0, 0, 60, 0, 0],
        hp_factor: 4.5,
        experience: 1200,
        bonus_skill: 26,
        skills: &[21, 9],
    },
    ProfessionRow {
        minimums: [0, 55, 55, 0, 55, 0, 55],
        hp_factor: 4.0,
        experience: 1600,
        bonus_skill: 20,
        skills: &[26, 24, 25, 27],
    },
    ProfessionRow {
        minimums: [0, 55, 0, 0, 0, 0, 60],
        hp_factor: 3.5,
        experience: 1200,
        bonus_skill: 27,
        skills: &[22, 21, 32],
    },
    ProfessionRow {
        minimums: [0, 60, 0, 0, 55, 0, 0],
        hp_factor: 3.0,
        experience: 1200,
        bonus_skill: 24,
        skills: &[28, 29, 30, 32],
    },
];

/// The two attributes each skill's base level averages (one for spellbooks
/// and expert skills), by skill id.
const GOVERNING: [(usize, usize); SKILLS] = [
    (0, 4),
    (0, 4),
    (0, 4),
    (0, 4),
    (4, 5),
    (0, 4),
    (0, 4),
    (4, 5),
    (4, 0),
    (4, 0),
    (4, 1),
    (4, 1),
    (4, 1),
    (4, 5),
    (4, 5),
    (6, 1),
    (6, 1),
    (6, 1),
    (4, 6),
    (6, 5),
    (1, 6),
    (6, 1),
    (1, 6),
    (1, 4),
    (1, 1),
    (2, 2),
    (4, 1),
    (6, 1),
    (1, 2),
    (1, 2),
    (1, 2),
    (1, 2),
    (1, 2),
    (1, 2),
    (0, 0),
    (1, 1),
    (2, 2),
    (3, 3),
    (4, 4),
    (5, 5),
    (6, 6),
];

/// One adventurer.
#[derive(Component, Clone, Debug, PartialEq, Reflect)]
pub struct Character {
    pub name: String,
    pub race: Race,
    pub gender: Gender,
    pub profession: Profession,
    pub original_profession: Profession,
    pub level: u8,
    pub profession_levels: [u8; 15],
    pub experience: u32,
    pub attributes: [u8; ATTRIBUTES],
    /// Points invested per skill.
    pub skill_points: [u8; SKILLS],
    pub hp: u32,
    pub stamina: u32,
}

impl Character {
    /// A minimal valid level 1 character (retail's NPC recipe): each
    /// attribute at the race's base or the profession's minimum, whichever
    /// is higher; the profession's skills seeded from their base levels;
    /// full health and stamina. Only the eleven playable races have bases.
    pub fn new(name: &str, race: Race, gender: Gender, profession: Profession) -> Self {
        let row = &PROFESSIONS[profession as usize];
        let base = RACE_ATTRIBUTES.get(race as usize).copied().unwrap_or([45; ATTRIBUTES]);
        let attributes = std::array::from_fn(|a| base[a].max(row.minimums[a]));
        let mut profession_levels = [0; 15];
        profession_levels[profession as usize] = 1;
        let mut character = Character {
            name: name.to_owned(),
            race,
            gender,
            profession,
            original_profession: profession,
            level: 1,
            profession_levels,
            experience: 0,
            attributes,
            skill_points: [0; SKILLS],
            hp: 0,
            stamina: 0,
        };
        let share = 60 / (1 + row.skills.len() as u32);
        for &skill in row.skills.iter().chain(std::iter::once(&row.bonus_skill)) {
            character.skill_points[skill] = (character.skill_base(skill) * share / 100) as u8;
        }
        character.hp = character.max_hp();
        character.stamina = character.max_stamina();
        character
    }

    /// Effective attribute: no modifiers are modelled yet, so the base,
    /// within retail's 1..=125.
    pub fn attribute(&self, attribute: usize) -> u32 {
        u32::from(self.attributes[attribute]).clamp(1, 125)
    }

    /// The average of a skill's governing attributes' bases.
    fn skill_base(&self, skill: usize) -> u32 {
        let (a, b) = GOVERNING[skill];
        (u32::from(self.attributes[a]) + u32::from(self.attributes[b])) / 2
    }

    /// Effective skill level: the points, plus a quarter (at least one) for
    /// the profession's bonus skill.
    pub fn skill(&self, skill: usize) -> u32 {
        let points = u32::from(self.skill_points[skill]);
        let bonus = if PROFESSIONS[self.profession as usize].bonus_skill == skill && points > 0 {
            (points * 25 / 100).max(1)
        } else {
            0
        };
        (points + bonus).min(125)
    }

    /// `round(Σ (0.6 + 0.008·VIT) × factor × levels)`, counting one extra
    /// level for the original profession.
    pub fn max_hp(&self) -> u32 {
        let vitality = 0.6 + 0.008 * self.attribute(VITALITY) as f32;
        let total: f32 = PROFESSIONS
            .iter()
            .enumerate()
            .map(|(p, row)| {
                let mut levels = u32::from(self.profession_levels[p]);
                if p == self.original_profession as usize && levels > 0 {
                    levels += 1;
                }
                vitality * row.hp_factor * levels as f32
            })
            .sum();
        (total.round() as u32).max(1)
    }

    /// `round((STR + PIE + VIT) / 3 × (0.1·level + 2))`.
    pub fn max_stamina(&self) -> u32 {
        let sum =
            (self.attribute(STRENGTH) + self.attribute(PIETY) + self.attribute(VITALITY)) as f32;
        (sum / 3.0 * (0.1 * f32::from(self.level) + 2.0)).round() as u32
    }

    /// Tenths of a pound: `(VIT + 2·STR) × 12`, × 8 for a Faerie.
    pub fn capacity(&self) -> u32 {
        let factor = if self.race == Race::Faerie { 8 } else { 12 };
        (self.attribute(VITALITY) + 2 * self.attribute(STRENGTH)) * factor
    }

    /// `(level + 1)/2 + SEN/5 + SPD/5 − 10`, plus Snake Speed; load is not
    /// counted yet.
    pub fn initiative(&self) -> i32 {
        let mut initiative = (i32::from(self.level) + 1) / 2
            + self.attribute(SENSES) as i32 / 5
            + self.attribute(SPEED) as i32 / 5
            - 10;
        if self.skill_points[SNAKE_SPEED] > 0 {
            initiative += self.skill(SNAKE_SPEED) as i32 / 10 + 1;
        }
        initiative
    }

    /// Experience needed for the next level (retail goals): the weight at
    /// level 1, doubling increments to level 10, then 20% more each level.
    pub fn experience_goal(&self) -> u32 {
        let weight = PROFESSIONS[self.profession as usize].experience;
        let mut goal = weight;
        let mut step = weight;
        for level in 2..=u32::from(self.level) {
            step = if level <= 10 { weight << (level - 2) } else { step * 12 / 10 };
            goal += step;
        }
        goal
    }

    /// Whether the character may use an item (retail `CanCharacterUseItem`):
    /// profession, race and gender, and every attribute and skill minimum.
    /// Spellbooks and casting items' extra tests need spells, not modelled.
    pub fn can_use(&self, item: &ItemDefinition) -> bool {
        let gender_bit = match self.gender {
            Gender::Male => 1,
            Gender::Female => 2,
        };
        item.profession_mask & (1 << self.profession as u16) != 0
            && item.race_mask & (1 << self.race as u32) != 0
            && (item.gender_mask == 3 || item.gender_mask & gender_bit != 0)
            && item.attribute_requirements.iter().all(|r| {
                usize::from(r.id) >= ATTRIBUTES
                    || self.attribute(usize::from(r.id)) >= u32::from(r.minimum)
            })
            && item.skill_requirements.iter().all(|r| {
                usize::from(r.id) >= SKILLS || self.skill(usize::from(r.id)) >= u32::from(r.minimum)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::items::{Dice, EquipClass, ItemId, QuantityKind, Requirement};

    #[test]
    fn a_human_fighter_has_the_worked_example_numbers() {
        let fighter = Character::new("Ragnar", Race::Human, Gender::Male, Profession::Fighter);
        assert_eq!(fighter.attributes, [55, 45, 45, 50, 50, 45, 45]);
        assert_eq!(fighter.max_hp(), 18);
        assert_eq!(fighter.max_stamina(), 105);
        assert_eq!(fighter.capacity(), 1920);
        assert_eq!(fighter.initiative(), 9);
        assert_eq!((fighter.hp, fighter.stamina), (18, 105));
        assert_eq!(fighter.experience_goal(), 1000);
        // Profession and bonus skills are seeded; others are not.
        assert!(fighter.skill(16) > fighter.skill_points[16].into());
        assert!(fighter.skill_points[0] > 0 && fighter.skill_points[10] == 0);
    }

    #[test]
    fn experience_goals_double_then_grow_by_a_fifth() {
        let mut mage = Character::new("Mira", Race::Elf, Gender::Female, Profession::Mage);
        mage.level = 3;
        assert_eq!(mage.experience_goal(), 1200 + 1200 + 2400);
        mage.level = 11;
        // Level 1 is one weight; levels 2 to 10 add 1, 2, ..., 256 weights.
        let at_ten = 1200 * (1 + 511);
        assert_eq!(mage.experience_goal(), at_ten + 256 * 1200 * 12 / 10);
    }

    #[test]
    fn items_check_profession_race_gender_and_minimums() {
        let fighter = Character::new("Ragnar", Race::Human, Gender::Male, Profession::Fighter);
        let mut bow = ItemDefinition {
            id: ItemId(0),
            name: "Strong Bow".into(),
            kind: 0,
            class: EquipClass::RangedWeapon,
            auto_identify: false,
            crucial: false,
            two_handed: false,
            off_hand: false,
            merchants_refuse: false,
            container: false,
            equip_to_use: false,
            never_depletes: false,
            weapon_skill: Some(8),
            range: 0,
            damage: Dice::default(),
            damage_bonus: 0,
            hit_bonus: 0,
            armor_class_bonus: 0,
            quantity: QuantityKind::None,
            initial_quantity: Dice::default(),
            maximum_quantity: 0,
            profession_mask: 0x7fff,
            race_mask: 0xffff,
            gender_mask: 3,
            attribute_requirements: vec![Requirement { id: 0, minimum: 85 }],
            skill_requirements: vec![],
            value: 0,
            weight: 0,
            cursed: false,
        };
        assert!(!fighter.can_use(&bow), "too weak");
        bow.attribute_requirements.clear();
        assert!(fighter.can_use(&bow));
        bow.gender_mask = 2;
        assert!(!fighter.can_use(&bow), "female only");
        bow.gender_mask = 3;
        bow.profession_mask = 1 << Profession::Mage as u16;
        assert!(!fighter.can_use(&bow));
    }
}
