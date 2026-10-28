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
    attributes: EnumMap<Attribute, u8>,
    skills: EnumMap<Skill, u8>,
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
        self.learned_spells = EnumMap::default();
        self.last_spell = None;
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
        let character = Character::new("", self.race, self.gender, self.profession);
        crate::game::magic::starting_picks(&character)
    }

    pub fn spell_remaining(&self) -> usize {
        self.spell_points_total().saturating_sub(self.learned_spells.values().map(Vec::len).sum())
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
                let skill = Skill::try_from(Skill::FireMagic as u8 + realm as u8).unwrap();
                self.skills[skill] = 0;
            }
        }
        true
    }

    pub fn reset_spells(&mut self) {
        self.learned_spells = EnumMap::default();
        self.last_spell = None;
        for realm in EnumSet::<Realm>::all() {
            let skill = Skill::try_from(Skill::FireMagic as u8 + realm as u8).unwrap();
            self.skills[skill] = 0;
        }
    }

    pub fn skill_remaining(&self) -> u8 {
        15 - self.skills.values().copied().sum::<u8>()
    }

    /// Points explicitly assigned on the creation screen, after minimums.
    pub fn allocated_attribute(&self, attribute: Attribute) -> u8 {
        self.attributes[attribute]
    }

    pub fn allocated_skill(&self, skill: Skill) -> u8 {
        self.skills[skill]
    }

    /// Retail's per-attribute ceiling after profession requirements are paid.
    pub fn attribute_step_limit(&self) -> u8 {
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

    pub fn skill_available(&self, skill: Skill) -> bool {
        let index = skill as usize;
        if index >= Skill::PowerStrike as usize {
            return false;
        }
        if let Some(realm) = skill.realm() {
            return !self.learned_spells[realm].is_empty();
        }
        if SKILL_AVAILABILITY[index] & (1 << self.profession as usize) == 0 {
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
            Character::new(self.display_name(), self.race, self.gender, self.profession);
        character.full_name = self.name.trim().to_owned();
        character.personality = self.personality;
        character.voice = self.voice;
        character.spell_picks.grant(self.profession, self.spell_remaining() as u16);
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

#[cfg(test)]
pub(crate) fn complete_allocations(draft: &mut CharacterDraft) {
    while let Some(attribute) =
        EnumSet::<Attribute>::all().iter().find(|&a| draft.can_adjust_attribute(a, true))
    {
        draft.adjust_attribute(attribute, true);
    }
    while let Some(skill) =
        EnumSet::<Skill>::all().iter().find(|&s| draft.can_adjust_skill(s, true))
    {
        draft.adjust_skill(skill, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learned_spells_survive_creation_and_make_their_realm_available() {
        use crate::content::spells::SpellDefinition;
        let catalog = SpellCatalog(vec![
            SpellDefinition::default(),
            SpellDefinition {
                id: SpellId::ACID_SPLASH,
                name: "Flame".into(),
                level: 1,
                cost: 1,
                realm: Some(Realm::Fire),
                books: Spellbook::Wizardry.into(),
                ..default()
            },
            SpellDefinition {
                id: SpellId::BLESS,
                name: "Frost".into(),
                level: 1,
                cost: 1,
                realm: Some(Realm::Water),
                books: Spellbook::Wizardry.into(),
                ..default()
            },
            SpellDefinition {
                id: SpellId::CHARM,
                name: "Prayer".into(),
                level: 1,
                cost: 1,
                realm: Some(Realm::Divine),
                books: Spellbook::Divinity.into(),
                ..default()
            },
        ]);
        let mut draft = CharacterDraft::default();
        draft.choose(Race::Human, Gender::Male, Profession::Mage).unwrap();
        assert!(!draft.toggle_spell(&catalog, SpellId::CHARM));
        assert!(draft.toggle_spell(&catalog, SpellId::ACID_SPLASH));
        assert!(draft.skill_available(Skill::FireMagic));
        assert_eq!(draft.preview().spell_picks.total(), 1);
        assert!(draft.adjust_skill(Skill::FireMagic, true));
        assert!(draft.toggle_spell(&catalog, SpellId::BLESS));
        draft.name = "A spellcaster".into();
        complete_allocations(&mut draft);
        let character = draft.finish().unwrap();
        assert!(crate::game::magic::skill_available(
            &character,
            Some(&crate::game::magic::LearnedSpells(draft.spells())),
            Some(&catalog),
            Skill::FireMagic
        ));
        assert!(draft.spells().contains(&SpellId::BLESS));
        assert!(draft.toggle_spell(&catalog, SpellId::ACID_SPLASH));
        assert!(!draft.skill_available(Skill::FireMagic));
        assert_eq!(draft.allocated_skill(Skill::FireMagic), 0);
    }

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
        complete_allocations(&mut draft);
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
        complete_allocations(&mut draft);
        draft.reset_skills();
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
    fn finish_rejects_unallocated_points_but_allows_retained_spell_picks() {
        let mut draft = CharacterDraft { name: "Mage".into(), ..default() };
        draft.choose(Race::Human, Gender::Male, Profession::Mage).unwrap();
        assert!(draft.finish().is_err());
        complete_allocations(&mut draft);
        assert_eq!(draft.finish().unwrap().spell_picks.total(), 2);
        draft.reset_skills();
        assert!(draft.finish().is_err());
    }

    #[test]
    fn unfinished_identity_cannot_be_committed() {
        let mut draft = CharacterDraft::default();
        assert!(draft.finish().is_err());
        draft.name = "\n".into();
        assert!(draft.finish().is_err());
        draft.name = "Éowyn".into();
        draft.choose(Race::Human, Gender::Female, Profession::Valkyrie).unwrap();
        complete_allocations(&mut draft);
        assert!(draft.finish().is_ok());
    }
}
