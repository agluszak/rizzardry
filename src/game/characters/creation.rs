//! New-character drafts. No live actor exists until the party is accepted.
use super::*;
use enumset::EnumSet;

pub const PLAYABLE_RACES: [Race; 11] = [
    Race::Human,
    Race::Elf,
    Race::Dwarf,
    Race::Gnome,
    Race::Hobbit,
    Race::Faerie,
    Race::Lizardman,
    Race::Dracon,
    Race::Felpurr,
    Race::Rawulf,
    Race::Mook,
];

#[derive(Clone, Debug)]
pub struct CharacterDraft {
    pub name: String,
    pub portrait: String,
    race: Race,
    gender: Gender,
    profession: Profession,
    attributes: EnumMap<Attribute, u8>,
    skills: EnumMap<Skill, u8>,
}

impl Default for CharacterDraft {
    fn default() -> Self {
        Self {
            name: String::new(),
            portrait: "hummf".into(),
            race: Race::Human,
            gender: Gender::Male,
            profession: Profession::Fighter,
            attributes: EnumMap::default(),
            skills: EnumMap::default(),
        }
    }
}

impl CharacterDraft {
    pub fn race(&self) -> Race {
        self.race
    }
    pub fn gender(&self) -> Gender {
        self.gender
    }
    pub fn profession(&self) -> Profession {
        self.profession
    }

    /// Changing the recipe refunds allocations. The caller can preview eligibility
    /// without altering the draft, and rejection leaves it intact.
    pub fn choose(
        &mut self,
        race: Race,
        gender: Gender,
        profession: Profession,
    ) -> Result<(), &'static str> {
        Self::budget(race, gender, profession)?;
        self.race = race;
        self.gender = gender;
        self.profession = profession;
        self.attributes = EnumMap::default();
        self.skills = EnumMap::default();
        Ok(())
    }

    pub fn budget(race: Race, gender: Gender, profession: Profession) -> Result<u8, &'static str> {
        let base = race_attributes(race).ok_or("Choose a playable race.")?;
        if profession == Profession::Valkyrie && gender != Gender::Female {
            return Err("Valkyries must be female.");
        }
        let required: u16 = profession_row(profession)
            .minimums
            .iter()
            .map(|(a, &minimum)| u16::from(minimum.saturating_sub(base[a])))
            .sum();
        60u16
            .checked_sub(required)
            .map(|n| n as u8)
            .ok_or("This race cannot meet the profession's requirements.")
    }

    pub fn attribute_remaining(&self) -> u8 {
        Self::budget(self.race, self.gender, self.profession).unwrap()
            - self.attributes.values().copied().sum::<u8>()
    }
    pub fn skill_remaining(&self) -> u8 {
        15 - self.skills.values().copied().sum::<u8>()
    }

    pub fn adjust_attribute(&mut self, attribute: Attribute, increase: bool) -> bool {
        let allocated = self.attributes[attribute];
        if increase {
            // Retail recomputes this ceiling after paying profession minimums.
            let budget = Self::budget(self.race, self.gender, self.profession).unwrap();
            let cap = budget.div_ceil(3).max(3).min(budget);
            if self.attribute_remaining() == 0
                || allocated >= cap
                || self.preview().attributes[attribute] >= 100
            {
                return false;
            }
            self.attributes[attribute] += 1;
        } else {
            if allocated == 0 {
                return false;
            }
            self.attributes[attribute] -= 1;
        }
        true
    }

    pub fn adjust_skill(&mut self, skill: Skill, increase: bool) -> bool {
        if increase {
            if self.skill_remaining() == 0
                || self.skills[skill] >= 5
                || !self.skill_available(skill)
                || self.preview().skill_points[skill] >= 75
            {
                return false;
            }
            self.skills[skill] += 1;
        } else {
            if self.skills[skill] == 0 {
                return false;
            }
            self.skills[skill] -= 1;
        }
        true
    }

    pub fn skill_available(&self, skill: Skill) -> bool {
        let index = skill as usize;
        if index >= Skill::FireMagic as usize {
            // No learned spells in this creation flow. Expert skills require 100
            // in their governing attribute, unreachable with creation's budgets.
            return false;
        }
        if AVAILABILITY[index] & (1 << self.profession as usize) == 0 {
            return false;
        }
        if index >= Skill::Wizardry as usize {
            return matches!(
                self.profession,
                Profession::Mage
                    | Profession::Priest
                    | Profession::Alchemist
                    | Profession::Psionic
                    | Profession::Bishop
            );
        }
        true
    }

    pub fn preview(&self) -> Character {
        let mut character =
            Character::new(self.name.trim(), self.race, self.gender, self.profession);
        character.portrait.clone_from(&self.portrait);
        for a in EnumSet::<Attribute>::all() {
            character.attributes[a] += self.attributes[a];
        }
        let row = profession_row(self.profession);
        let share = 60 / (1 + row.skills.len() as u32);
        character.skill_points = EnumMap::default();
        for &skill in row.skills.iter().chain(std::iter::once(&row.bonus_skill)) {
            character.skill_points[skill] = (character.skill_base(skill) * share / 100) as u8;
        }
        for skill in EnumSet::<Skill>::all() {
            character.skill_points[skill] += self.skills[skill];
        }
        character
    }

    pub fn finish(&self) -> Result<Character, &'static str> {
        Self::budget(self.race, self.gender, self.profession)?;
        if self.name.trim().is_empty()
            || self.name.chars().count() > 24
            || self.name.chars().any(char::is_control)
        {
            return Err("Enter a name of 1–24 characters.");
        }
        if self.portrait.is_empty() || !self.portrait.bytes().all(|c| c.is_ascii_alphanumeric()) {
            return Err("Choose a portrait.");
        }
        Ok(self.preview())
    }
}

// Retail g_profession_skill_availability, ordinary and spellbook skills.
// Bit order is Profession's authored order; realm availability needs learned spells.
const AVAILABILITY: [u16; 28] = [
    0x03bf, 0x000f, 0x006f, 0x146f, 0x6bbf, 0x7fff, 0x1f9f, 0x018f, 0x03bf, 0x7fff, 0x03a0, 0x00e0,
    0x0200, 0x02a0, 0x0060, 0x0008, 0x7fff, 0x7fff, 0x03ff, 0x0070, 0x7fff, 0x7fff, 0x7fff, 0x0100,
    0x5010, 0x1406, 0x1828, 0x3040,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creation_spends_the_pool_after_requirements_and_refunds_a_changed_recipe() {
        let mut draft = CharacterDraft::default();
        draft.name = "  Kestrel  ".into();
        assert_eq!(draft.attribute_remaining(), 40); // Human Fighter pays 20 in minimums.
        for _ in 0..14 {
            assert!(draft.adjust_attribute(Attribute::Strength, true));
        }
        assert!(!draft.adjust_attribute(Attribute::Strength, true));
        for _ in 0..14 {
            assert!(draft.adjust_attribute(Attribute::Vitality, true));
        }
        for _ in 0..12 {
            assert!(draft.adjust_attribute(Attribute::Dexterity, true));
        }
        assert_eq!(draft.attribute_remaining(), 0);
        assert!(!draft.adjust_attribute(Attribute::Speed, true));
        let character = draft.finish().unwrap();
        assert_eq!(character.name, "Kestrel");
        assert_eq!(character.attributes[Attribute::Strength], 69);
        assert_eq!(character.attributes.values().map(|&v| u32::from(v)).sum::<u32>(), 7 * 45 + 60);
        draft.choose(Race::Elf, Gender::Female, Profession::Mage).unwrap();
        assert_eq!(
            draft.attribute_remaining(),
            CharacterDraft::budget(Race::Elf, Gender::Female, Profession::Mage).unwrap()
        );
        assert_eq!(draft.skill_remaining(), 15);
    }

    #[test]
    fn training_respects_profession_and_per_skill_limits_without_losing_allocations() {
        let mut draft = CharacterDraft::default();
        draft.name = "Mira".into();
        let initial = draft.preview();
        assert!(!draft.adjust_skill(Skill::Wizardry, true));
        for skill in [Skill::Sword, Skill::CloseCombat, Skill::Shield] {
            for _ in 0..5 {
                assert!(draft.adjust_skill(skill, true));
            }
            assert!(!draft.adjust_skill(skill, true));
        }
        assert!(!draft.adjust_skill(Skill::Bow, true));
        assert!(draft.adjust_skill(Skill::Shield, false));
        assert!(draft.adjust_skill(Skill::Bow, true));
        let trained = draft.finish().unwrap();
        assert_eq!(trained.skill_points[Skill::Sword], initial.skill_points[Skill::Sword] + 5);
        assert_eq!(trained.skill_points[Skill::Shield], initial.skill_points[Skill::Shield] + 4);
        assert_eq!(trained.skill_points[Skill::Bow], initial.skill_points[Skill::Bow] + 1);
        let before = draft.preview();
        assert!(draft.choose(Race::Human, Gender::Male, Profession::Valkyrie).is_err());
        assert_eq!(draft.preview(), before, "rejected identity choices do not erase training");
    }

    #[test]
    fn unfinished_identity_cannot_be_committed() {
        let mut draft = CharacterDraft::default();
        assert!(draft.finish().is_err());
        draft.name = "\n".into();
        assert!(draft.finish().is_err());
        draft.name = "Éowyn".into();
        draft.choose(Race::Human, Gender::Female, Profession::Valkyrie).unwrap();
        assert!(draft.finish().is_ok());
    }
}
