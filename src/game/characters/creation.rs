//! New-character drafts. No live actor exists until the party is accepted.

pub mod identity;
use super::*;
use crate::content::spells::{SpellCatalog, SpellId, Spellbook};
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
    pub nickname: String,
    pub personality: crate::content::characters::Personality,
    pub voice: crate::content::characters::Voice,
    pub portrait: String,
    race: Race,
    gender: Gender,
    profession: Profession,
    attributes: EnumMap<Attribute, u32>,
    skills: EnumMap<Skill, u32>,
    learned_spells: EnumMap<Realm, Vec<SpellId>>,
    last_spell: Option<SpellId>,
}

impl Default for CharacterDraft {
    fn default() -> Self {
        Self {
            name: String::new(),
            nickname: String::new(),
            personality: Default::default(),
            voice: Default::default(),
            portrait: "hummf".into(),
            race: Race::Human,
            gender: Gender::Male,
            profession: Profession::Fighter,
            attributes: EnumMap::default(),
            skills: EnumMap::default(),
            learned_spells: EnumMap::default(),
            last_spell: None,
        }
    }
}

/// The most characters a new party starts with.
pub const PARTY_LIMIT: usize = 6;
/// The longest full name and nickname, in characters.
pub const NAME_LIMIT: usize = 39;
pub const NICKNAME_LIMIT: usize = 9;

/// The gender a profession gives a character whose gender is not chosen
/// yet: Valkyries are women.
pub fn default_gender(profession: Profession, gender: Gender, chosen: bool) -> Gender {
    if profession == Profession::Valkyrie && !chosen { Gender::Female } else { gender }
}

/// Whether the finished draft may join the candidates: it must have a
/// nickname no other candidate has.
pub fn admit<'a>(
    draft: &CharacterDraft,
    mut candidates: impl Iterator<Item = &'a CharacterDraft>,
) -> Result<(), &'static str> {
    if draft.nickname.trim().is_empty() {
        return Err("Enter a nickname.");
    }
    draft.finish()?;
    if candidates.any(|candidate| candidate.display_name() == draft.display_name()) {
        return Err("A character with that nickname already exists.");
    }
    Ok(())
}

impl CharacterDraft {
    pub fn display_name(&self) -> &str {
        if self.nickname.trim().is_empty() { self.name.trim() } else { self.nickname.trim() }
    }
    pub fn race(&self) -> Race {
        self.race
    }
    pub fn gender(&self) -> Gender {
        self.gender
    }
    pub fn profession(&self) -> Profession {
        self.profession
    }

    /// Changing the recipe refunds allocations and gives the draft the
    /// recipe's default portrait, personality and voice. The caller can
    /// preview eligibility without altering the draft, and rejection leaves
    /// it intact.
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
        self.learned_spells = EnumMap::default();
        self.last_spell = None;
        identity::initialize(self);
        Ok(())
    }

    pub fn budget(race: Race, gender: Gender, profession: Profession) -> Result<u32, &'static str> {
        let base = race_attributes(race).ok_or("Choose a playable race.")?;
        if profession == Profession::Valkyrie && gender != Gender::Female {
            return Err("Valkyries must be female.");
        }
        let required: u32 = profession_row(profession)
            .minimums
            .iter()
            .map(|(a, &minimum)| minimum - minimum.min(base[a]))
            .sum();
        if required > 60 {
            return Err("This race cannot meet the profession's requirements.");
        }
        Ok(60 - required)
    }

    pub fn attribute_remaining(&self) -> u32 {
        Self::budget(self.race, self.gender, self.profession).unwrap()
            - self.attributes.values().copied().sum::<u32>()
    }
    /// The books first spells come from: the profession's, for those who
    /// pick any (retail `FinalizeSpellPointPool`: none below caster level 1,
    /// so hybrids learn theirs later).
    pub fn spell_books(&self) -> EnumSet<Spellbook> {
        if self.spell_points_total() == 0 {
            return EnumSet::empty();
        }
        crate::game::magic::profession_books(self.profession)
    }

    pub fn spell_points_total(&self) -> usize {
        crate::game::magic::starting_picks(self.profession)
    }

    pub fn spell_remaining(&self) -> usize {
        self.spell_points_total() - self.learned_spells.values().map(Vec::len).sum::<usize>()
    }

    /// The accepted selection, shared with the live LearnedSpells component.
    pub fn spells(&self) -> std::collections::BTreeSet<SpellId> {
        self.learned_spells.values().flatten().copied().collect()
    }

    pub fn knows_spell(&self, id: SpellId) -> bool {
        self.learned_spells.values().any(|spells| spells.contains(&id))
    }

    pub fn can_learn_spell(&self, catalog: &SpellCatalog, id: SpellId) -> bool {
        id.0 > 0
            && id.0 < 0x72
            && catalog.get(id).is_some_and(|spell| {
                spell.realm.is_some()
                    && spell.level == 1
                    && !(spell.books & self.spell_books()).is_empty()
            })
    }

    pub fn toggle_spell(&mut self, catalog: &SpellCatalog, id: SpellId) -> bool {
        if !self.can_learn_spell(catalog, id) {
            return false;
        }
        let realm = catalog.get(id).unwrap().realm.unwrap();
        if let Some(index) = self.learned_spells[realm].iter().position(|&spell| spell == id) {
            self.learned_spells[realm].remove(index);
        } else {
            if self.spell_remaining() == 0 {
                // Retail replaces the earliest selected spell other than the most recent.
                let replaced = self
                    .learned_spells
                    .values()
                    .flatten()
                    .copied()
                    .filter(|spell| Some(*spell) != self.last_spell)
                    .min_by_key(|spell| spell.0);
                if let Some(replaced) = replaced {
                    for spells in self.learned_spells.values_mut() {
                        spells.retain(|&spell| spell != replaced);
                    }
                } else {
                    return false;
                }
            }
            self.learned_spells[realm].push(id);
            self.last_spell = Some(id);
        }
        // Allocations in a realm cease to be valid when its last spell is removed.
        for realm in EnumSet::<Realm>::all() {
            if self.learned_spells[realm].is_empty() {
                self.skills[realm.skill()] = 0;
            }
        }
        true
    }

    pub fn reset_spells(&mut self) {
        self.learned_spells = EnumMap::default();
        self.last_spell = None;
        for realm in EnumSet::<Realm>::all() {
            self.skills[realm.skill()] = 0;
        }
    }

    pub fn skill_remaining(&self) -> u32 {
        15 - self.skills.values().copied().sum::<u32>()
    }

    /// Points explicitly assigned on the creation screen, after minimums.
    pub fn allocated_attribute(&self, attribute: Attribute) -> u32 {
        self.attributes[attribute]
    }

    pub fn allocated_skill(&self, skill: Skill) -> u32 {
        self.skills[skill]
    }

    /// Retail's per-attribute ceiling after profession requirements are paid.
    pub fn attribute_step_limit(&self) -> u32 {
        let budget = Self::budget(self.race, self.gender, self.profession).unwrap();
        budget.div_ceil(3).max(3).min(budget)
    }

    pub fn can_adjust_attribute(&self, attribute: Attribute, increase: bool) -> bool {
        let allocated = self.attributes[attribute];
        if increase {
            self.attribute_remaining() > 0
                && allocated < self.attribute_step_limit()
                && self.preview().attributes[attribute] < 100
        } else {
            allocated > 0
        }
    }

    pub fn reset_attributes(&mut self) {
        self.attributes = EnumMap::default();
    }

    pub fn reset_skills(&mut self) {
        self.skills = EnumMap::default();
    }

    pub fn adjust_attribute(&mut self, attribute: Attribute, increase: bool) -> bool {
        if !self.can_adjust_attribute(attribute, increase) {
            return false;
        }
        if increase {
            self.attributes[attribute] += 1;
        } else {
            self.attributes[attribute] -= 1;
        }
        true
    }

    pub fn can_adjust_skill(&self, skill: Skill, increase: bool) -> bool {
        if increase {
            self.skill_remaining() > 0
                && self.skills[skill] < 5
                && self.skill_available(skill)
                && self.preview().skill_points[skill] < 75
        } else {
            self.skills[skill] > 0
        }
    }

    pub fn adjust_skill(&mut self, skill: Skill, increase: bool) -> bool {
        if !self.can_adjust_skill(skill, increase) {
            return false;
        }
        if increase {
            self.skills[skill] += 1;
        } else {
            self.skills[skill] -= 1;
        }
        true
    }

    /// Whether the drafted character may train a skill: the character's own
    /// rule ([`Character::skill_available`]), a realm skill by the spells
    /// picked in its realm.
    pub fn skill_available(&self, skill: Skill) -> bool {
        match skill.realm() {
            Some(realm) => !self.learned_spells[realm].is_empty(),
            None => self.preview().skill_available(skill),
        }
    }

    pub fn preview(&self) -> Character {
        let mut character =
            Character::new(self.display_name(), self.race, self.gender, self.profession);
        character.full_name = self.name.trim().to_owned();
        character.personality = self.personality;
        character.voice = self.voice;
        character.spell_picks.grant(self.profession, self.spell_remaining());
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

    pub fn finish(&self) -> Result<Character, &'static str> {
        Self::budget(self.race, self.gender, self.profession)?;
        if self.name.trim().is_empty()
            || self.name.chars().count() > NAME_LIMIT
            || self.name.chars().any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))
        {
            return Err("Enter a valid full name of 1–39 characters.");
        }
        if self.nickname.chars().count() > NICKNAME_LIMIT
            || self.nickname.chars().any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))
        {
            return Err("Enter a valid nickname of at most nine characters.");
        }
        if self.portrait.is_empty() || !self.portrait.bytes().all(|c| c.is_ascii_alphanumeric()) {
            return Err("Choose a portrait.");
        }
        if EnumSet::<Attribute>::all()
            .iter()
            .any(|attribute| self.can_adjust_attribute(attribute, true))
        {
            return Err("Allocate the remaining attribute points.");
        }
        if EnumSet::<Skill>::all().iter().any(|skill| self.can_adjust_skill(skill, true)) {
            return Err("Allocate the remaining skill points.");
        }
        // Spell picks may deliberately be retained for later advancement.
        Ok(self.preview())
    }
}
