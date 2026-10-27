//! New-character drafts. No live actor exists until the party is accepted.
use super::*;
use crate::content::spells::{SpellDefinition, SpellId};
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
    /// The first spells picked (retail state 2 until the character is made).
    spells: std::collections::BTreeSet<SpellId>,
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
            spells: default(),
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
        self.spells.clear();
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
            let preview = self.preview();
            if self.skill_remaining() == 0
                || self.skills[skill] >= 5
                || !preview.skill_available(skill)
                || preview.skill_points[skill] >= 75
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

    pub fn preview(&self) -> Character {
        let mut character =
            Character::new(self.name.trim(), self.race, self.gender, self.profession);
        character.portrait.clone_from(&self.portrait);
        for a in EnumSet::<Attribute>::all() {
            character.attributes[a] += self.attributes[a];
        }
        character.seed_starting_skills();
        for skill in EnumSet::<Skill>::all() {
            character.skill_points[skill] += self.skills[skill];
        }
        character
    }

    pub fn spells(&self) -> &std::collections::BTreeSet<SpellId> {
        &self.spells
    }

    /// Pick or unpick a first spell (retail `CharGeneration.cpp`): the
    /// profession's casters pick two they could learn now.
    pub fn toggle_spell(&mut self, spell: &SpellDefinition) -> bool {
        if self.spells.remove(&spell.id) {
            return true;
        }
        let profile = self.preview();
        if self.spells.len() >= crate::game::magic::starting_picks(&profile)
            || !crate::game::magic::can_learn(&profile, spell)
        {
            return false;
        }
        self.spells.insert(spell.id)
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
        let preview = self.preview();
        if self.spells.len() != crate::game::magic::starting_picks(&preview) {
            return Err("Choose your first spells.");
        }
        Ok(preview)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creation_spends_the_pool_after_requirements_and_refunds_a_changed_recipe() {
        let mut draft = CharacterDraft { name: "  Kestrel  ".into(), ..default() };
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
        let mut draft = CharacterDraft { name: "Mira".into(), ..default() };
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

    /// A pure caster must pick their two first spells from their own book;
    /// changing profession clears the picks.
    #[test]
    fn a_mage_picks_two_first_spells_from_their_book() {
        use crate::content::spells::{SpellDefinition, Spellbook};
        let spell = |id, books| SpellDefinition {
            id: SpellId(id),
            level: 1,
            books,
            realm: Some(crate::content::characters::Realm::Fire),
            ..default()
        };
        let (blast, frost, missile, heal) = (
            spell(5, Spellbook::Wizardry.into()),
            spell(4, Spellbook::Wizardry.into()),
            spell(25, Spellbook::Wizardry.into()),
            spell(6, Spellbook::Divinity.into()),
        );
        let mut draft = CharacterDraft { name: "Merlin".into(), ..default() };
        draft.choose(Race::Human, Gender::Male, Profession::Mage).unwrap();
        assert_eq!(draft.finish().unwrap_err(), "Choose your first spells.");
        assert!(!draft.toggle_spell(&heal), "not in the Mage's book");
        assert!(draft.toggle_spell(&blast) && draft.toggle_spell(&frost));
        assert!(!draft.toggle_spell(&missile), "two picks only");
        assert!(draft.finish().is_ok());
        assert!(draft.toggle_spell(&frost), "a pick can be taken back");
        draft.choose(Race::Human, Gender::Male, Profession::Fighter).unwrap();
        assert!(draft.spells().is_empty());
        assert!(draft.finish().is_ok(), "fighters pick none");
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
