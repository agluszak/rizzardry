//! Retail CharGeneration.cpp level-up rules. Editing owns only a draft;
//! acceptance revalidates the actor and commits training and its consequences.
use super::*;
use crate::content::spells::{SpellCatalog, SpellId, Spellbook};
use crate::game::actors::Skills;
use crate::game::magic::{self, LearnedSpells};
use crate::game::party::roster::PartyRoster;
use enumset::EnumSet;
use std::collections::BTreeSet;

/// Saved picks retain their school when changing profession. Bishop awards
/// are unrestricted, matching skill_costs and magic_bonus_pool in retail.
#[derive(bevy::prelude::Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct SpellPicks {
    books: EnumMap<Spellbook, usize>,
    general: usize,
}
impl SpellPicks {
    pub fn total(&self) -> usize {
        self.books.values().sum::<usize>() + self.general
    }
    pub(super) fn grant(&mut self, profession: Profession, amount: usize) {
        if profession.row().picks_from_any_book {
            self.general += amount;
        } else if let Some(book) = profession.row().books.iter().next() {
            self.books[book] += amount;
        }
    }
    fn available(&self, c: &Character) -> usize {
        if magic::caster_level(&c.calling, c.calling.profession) < 1 {
            return 0;
        }
        self.general
            + c.calling.profession.row().books.iter().map(|book| self.books[book]).sum::<usize>()
    }
    fn spend(&mut self, c: &Character, books: EnumSet<Spellbook>) -> bool {
        let active = c.calling.profession.row().books;
        if let Some(book) = (active & books).iter().find(|&b| self.books[b] > 0) {
            self.books[book] -= 1;
        } else if self.general > 0 {
            self.general -= 1;
        } else if c.calling.profession.row().picks_from_any_book {
            let Some(book) = active.iter().find(|&b| self.books[b] > 0) else {
                return false;
            };
            self.books[book] -= 1;
        } else {
            return false;
        }
        true
    }
}

/// CalcXPGoal adds the current profession's increment to the threshold
/// actually passed; switching profession never reprices previous levels.
pub(super) fn experience_increment(profession: Profession, level: u8) -> u64 {
    let weight = profession.row().experience;
    if level == 1 {
        return weight;
    }
    let mut value = weight << (level.min(10) - 2);
    for _ in 10..level {
        value = value * 12 / 10;
    }
    value
}
pub fn ready(calling: &super::Calling, experience: &super::Experience, v: &Vitals) -> bool {
    // Retail offers a level past 50 and then clamps it away; it is not
    // offered here.
    !v.is_dead()
        && calling.level() < super::MAX_LEVEL
        && experience.earned >= calling.experience_goal(experience)
}

#[derive(Clone, Debug)]
pub struct Advancement {
    member: Entity,
    original: Character,
    known: LearnedSpells,
    /// The member's skills as they stood when the draft began, which
    /// decide the spells they may learn.
    standing: Skills,
    profession: Profession,
    attributes: EnumMap<Attribute, u32>,
    skills: EnumMap<Skill, u32>,
    spells: BTreeSet<SpellId>,
}
/// An [`Advancement`] worked out once ([`Advancement::evaluate`]): a
/// temporary view for one pass over many rows, never kept.
pub struct Evaluation {
    baseline: Character,
    pub preview: Character,
    learned: LearnedSpells,
    attributes_left: u32,
}

impl Advancement {
    fn new(member: Entity, original: &Character, known: &LearnedSpells, standing: &Skills) -> Self {
        Self {
            member,
            standing: *standing,
            original: original.clone(),
            known: known.clone(),
            profession: original.calling.profession,
            attributes: default(),
            skills: default(),
            spells: default(),
        }
    }
    pub fn member(&self) -> Entity {
        self.member
    }
    pub fn profession(&self) -> Profession {
        self.profession
    }
    pub fn eligible_profession(&self, profession: Profession) -> bool {
        if profession.row().only_gender.is_some_and(|only| only != self.original.birth.gender) {
            return false;
        }
        let minimums = profession.row().minimums;
        let deficits: Vec<_> =
            minimums.iter().map(|(a, &n)| n - n.min(self.original.abilities.0[a])).collect();
        deficits.iter().all(|&n| n <= 3) && deficits.iter().sum::<u32>() <= 6
    }
    pub fn choose_profession(&mut self, profession: Profession) -> bool {
        if !self.eligible_profession(profession) {
            return false;
        }
        self.profession = profession;
        self.attributes = default();
        self.skills = default();
        self.spells.clear();
        true
    }
    fn baseline(&self) -> Character {
        let mut c = self.original.clone();
        c.experience.last_goal = c.calling.experience_goal(&c.experience);
        c.calling.profession = self.profession;
        c.calling.levels[self.profession] += 1;
        for (a, &minimum) in &self.profession.row().minimums {
            c.abilities.0[a] = c.abilities.0[a].max(minimum);
        }
        if self.profession != self.original.calling.profession {
            let row = self.profession.row();
            for &s in row.skills.iter().chain(std::iter::once(&row.bonus_skill)) {
                c.training.0[s] = c.training.0[s].max(5);
            }
        }
        if magic::caster_level(&c.calling, c.calling.profession) == 1
            && magic::caster_level(&self.original.calling, c.calling.profession) < 1
        {
            for book in c.calling.profession.row().books {
                let s = magic::book_skill(book);
                c.training.0[s] = c.training.0[s].max(5);
            }
        }
        if magic::caster_level(&c.calling, c.calling.profession) > 0 {
            c.picks.grant(c.calling.profession, 1);
        }
        c
    }
    pub fn preview(&self) -> Character {
        self.allocate(self.baseline())
    }
    fn allocate(&self, mut c: Character) -> Character {
        for (a, &n) in &self.attributes {
            c.abilities.0[a] += n;
        }
        for (s, &n) in &self.skills {
            c.training.0[s] += n;
        }
        c
    }
    /// The draft worked out once (its baseline, its preview, what it would
    /// know and the attribute points left), for checking many adjustments
    /// against without rebuilding them for each.
    pub fn evaluate(&self) -> Evaluation {
        let baseline = self.baseline();
        let preview = self.allocate(baseline.clone());
        Evaluation {
            attributes_left: self.attributes_left(&preview),
            learned: self.learned(),
            baseline,
            preview,
        }
    }
    fn attributes_left(&self, preview: &Character) -> u32 {
        6 - preview.abilities.0.iter().map(|(a, &n)| n - self.original.abilities.0[a]).sum::<u32>()
    }
    pub fn attribute_remaining(&self) -> u32 {
        self.attributes_left(&self.preview())
    }
    pub fn skill_remaining(&self) -> u32 {
        9 - self.skills.values().sum::<u32>()
    }
    pub fn allocated_attribute(&self, a: Attribute) -> u32 {
        self.attributes[a]
    }
    pub fn allocated_skill(&self, s: Skill) -> u32 {
        self.skills[s]
    }
    pub fn can_adjust_attribute(&self, a: Attribute, up: bool) -> bool {
        self.attribute_adjustable(&self.evaluate(), a, up)
    }
    pub fn attribute_adjustable(&self, draft: &Evaluation, a: Attribute, up: bool) -> bool {
        if !up {
            return self.attributes[a] > 0;
        }
        let c = &draft.preview;
        draft.attributes_left > 0
            && c.abilities.0[a] < 100
            && c.abilities.0[a] - self.original.abilities.0[a] < 3
    }
    pub fn adjust_attribute(&mut self, a: Attribute, up: bool) -> bool {
        if !self.can_adjust_attribute(a, up) {
            return false;
        }
        if up {
            self.attributes[a] += 1;
        } else {
            self.attributes[a] -= 1;
        }
        // An attribute lowered below 100 closes its expert skill, and what
        // was put into it comes back (retail `AdjustAllocatedAttribute`,
        // `RecomputeSkillLimits`, `ClampSkillsToBudget`).
        let c = self.preview();
        for s in EnumSet::<Skill>::all() {
            if s as usize >= Skill::PowerStrike as usize && !c.trained().can_use(s) {
                self.skills[s] = 0;
            }
        }
        true
    }
    pub fn learned(&self) -> LearnedSpells {
        let mut known = self.known.clone();
        known.0.extend(self.spells.iter().copied());
        if self.spells.contains(&SpellId::SET_PORTAL) {
            known.0.extend([SpellId::RETURN_TO_PORTAL]);
        }
        known
    }
    pub fn can_adjust_skill(&self, s: Skill, up: bool, catalog: &SpellCatalog) -> bool {
        self.skill_adjustable(&self.evaluate(), s, up, catalog)
    }
    pub fn skill_adjustable(
        &self,
        draft: &Evaluation,
        s: Skill,
        up: bool,
        catalog: &SpellCatalog,
    ) -> bool {
        if !up {
            return self.skills[s] > 0;
        }
        let c = &draft.preview;
        self.skill_remaining() > 0
            && self.skills[s] < 3
            && c.training.0[s] < 75
            && magic::skill_available(c.trained(), &draft.learned, catalog, s)
    }
    pub fn adjust_skill(&mut self, s: Skill, up: bool, catalog: &SpellCatalog) -> bool {
        if !self.can_adjust_skill(s, up, catalog) {
            return false;
        }
        if up {
            self.skills[s] += 1;
        } else {
            self.skills[s] -= 1;
        }
        true
    }
    pub fn selected_spell(&self, id: SpellId) -> bool {
        self.spells.contains(&selection_key(id))
    }
    pub fn learnable(&self, draft: &Evaluation, id: SpellId, catalog: &SpellCatalog) -> bool {
        // Retail excludes this draft's magic-skill allocations from spell eligibility.
        !self.known.0.contains(&id)
            && catalog.get(id).is_some_and(|s| {
                magic::can_learn(
                    (draft.baseline.trained(), &draft.baseline.training),
                    &self.standing,
                    s,
                )
            })
    }
    fn remaining_picks(&self, draft: &Evaluation, catalog: &SpellCatalog) -> Option<SpellPicks> {
        let c = &draft.baseline;
        let mut picks = c.picks.clone();
        for &id in &self.spells {
            if !self.learnable(draft, id, catalog) || !picks.spend(c, catalog.get(id)?.books) {
                return None;
            }
        }
        Some(picks)
    }
    pub fn spell_remaining(&self, catalog: &SpellCatalog) -> usize {
        let draft = self.evaluate();
        self.remaining_picks(&draft, catalog).map_or(0, |p| p.available(&draft.baseline))
    }
    pub fn toggle_spell(&mut self, id: SpellId, catalog: &SpellCatalog) -> bool {
        let id = selection_key(id);
        // The spell selection does not enter the baseline or the preview.
        let draft = self.evaluate();
        if !self.spells.remove(&id) {
            if !self.learnable(&draft, id, catalog) {
                return false;
            }
            self.spells.insert(id);
            if self.remaining_picks(&draft, catalog).is_none() {
                self.spells.remove(&id);
                return false;
            }
        }
        let learned = self.learned();
        for s in EnumSet::<Skill>::all() {
            if !magic::skill_available(draft.preview.trained(), &learned, catalog, s) {
                self.skills[s] = 0;
            }
        }
        true
    }
    fn finish(&self, catalog: &SpellCatalog) -> Result<(Character, LearnedSpells), &'static str> {
        if !self.eligible_profession(self.profession) {
            return Err("This profession is unavailable.");
        }
        let draft = self.evaluate();
        if EnumSet::<Attribute>::all().iter().any(|a| self.attribute_adjustable(&draft, a, true)) {
            return Err("Allocate the remaining attribute points.");
        }
        if EnumSet::<Skill>::all().iter().any(|s| self.skill_adjustable(&draft, s, true, catalog)) {
            return Err("Allocate the remaining skill points.");
        }
        if self.skills.iter().any(|(s, &n)| {
            n > 0 && !magic::skill_available(draft.preview.trained(), &draft.learned, catalog, s)
        }) {
            return Err("Refund training in unavailable skills.");
        }
        let picks = self
            .remaining_picks(&draft, catalog)
            .ok_or("The spell selection is no longer valid.")?;
        let Evaluation { mut preview, learned, .. } = draft;
        preview.picks = picks;
        Ok((preview, learned))
    }
}

/// The spell a selection holds it under: retail charges the linked portal pair
/// at Set Portal (75) in spell order.
fn selection_key(id: SpellId) -> SpellId {
    if id == SpellId::RETURN_TO_PORTAL { SpellId::SET_PORTAL } else { id }
}

#[derive(bevy::ecs::system::SystemParam)]
pub struct Progression<'w, 's> {
    roster: Query<'w, 's, (Entity, &'static PartyRoster)>,
    members:
        Query<'w, 's, (super::Paper, &'static Vitals, &'static mut LearnedSpells, &'static Skills)>,
    blocked: Query<
        'w,
        's,
        (),
        Or<(
            With<crate::game::combat::Battle>,
            With<crate::game::world::level::Destination>,
            With<crate::game::npcs::conversation::Conversation>,
            With<crate::game::world::level::TravelQuestion>,
        )>,
    >,
    content: If<Res<'w, crate::game::data::Catalogs>>,
    commands: Commands<'w, 's>,
}
impl Progression<'_, '_> {
    pub fn eligibility(&self, member: Entity) -> Result<(), &'static str> {
        if !self.blocked.is_empty() {
            return Err("Finish the current encounter or journey first.");
        }
        if !self.roster.single().is_ok_and(|(_, r)| r.contains(&member)) {
            return Err("This character is no longer in the party.");
        }
        let (paper, v, ..) = self.members.get(member).unwrap();
        let sheet = paper.copy();
        if !ready(&sheet.calling, &sheet.experience, v) {
            return Err("This character is not ready to gain a level.");
        }
        Ok(())
    }
    pub fn begin(&self, member: Entity) -> Result<Advancement, &'static str> {
        self.eligibility(member)?;
        let (paper, _, known, skills) = self.members.get(member).unwrap();
        Ok(Advancement::new(member, &paper.copy(), known, skills))
    }
    pub fn accept(&mut self, draft: &Advancement) -> Result<(), &'static str> {
        self.eligibility(draft.member)?;
        let spells = &self.content.spells;
        let (mut paper, _, mut known, _) = self.members.get_mut(draft.member).unwrap();
        if paper.copy() != draft.original || *known != draft.known {
            return Err("This character's training changed. Reopen level-up.");
        }
        let (next, learned) = draft.finish(spells)?;
        let (party, _) = self.roster.single().map_err(|_| "The party is unavailable.")?;
        let member = draft.member;
        // Deficits, conditions and recovery fractions stay; advancement is
        // not a full heal. The maxima follow as figures do.
        paper.write(next);
        *known = learned;
        // A Gadgeteer's omnigun follows the committed level.
        self.commands.trigger(crate::game::inventory::omnigun::Advanced { party, member });
        Ok(())
    }
}
