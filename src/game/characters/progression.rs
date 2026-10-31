//! Retail CharGeneration.cpp level-up rules. Editing owns only a draft;
//! acceptance revalidates the actor and commits training and its consequences.
use super::*;
use crate::content::spells::{SpellCatalog, SpellId, Spellbook};
use crate::game::inventory::{Carried, PartyInventory};
use crate::game::magic::{self, LearnedSpells};
use crate::game::party::roster::PartyRoster;
use enumset::EnumSet;
use std::collections::BTreeSet;

/// Saved picks retain their school when changing profession. Bishop awards
/// are unrestricted, matching skill_costs and magic_bonus_pool in retail.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpellPicks {
    books: EnumMap<Spellbook, usize>,
    general: usize,
}
impl SpellPicks {
    pub fn total(&self) -> usize {
        self.books.values().sum::<usize>() + self.general
    }
    pub(super) fn grant(&mut self, profession: Profession, amount: usize) {
        if profession == Profession::Bishop {
            self.general += amount;
        } else if let Some(book) = magic::profession_books(profession).iter().next() {
            self.books[book] += amount;
        }
    }
    fn available(&self, c: &Character) -> usize {
        if magic::caster_level(c, c.profession) < 1 {
            return 0;
        }
        self.general
            + magic::profession_books(c.profession)
                .iter()
                .map(|book| self.books[book])
                .sum::<usize>()
    }
    fn spend(&mut self, c: &Character, books: EnumSet<Spellbook>) -> bool {
        let active = magic::profession_books(c.profession);
        if let Some(book) = (active & books).iter().find(|&b| self.books[b] > 0) {
            self.books[book] -= 1;
        } else if self.general > 0 {
            self.general -= 1;
        } else if c.profession == Profession::Bishop {
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
    let weight = profession_row(profession).experience;
    if level == 1 {
        return weight;
    }
    let mut value = weight << (level.min(10) - 2);
    for _ in 10..level {
        value = value * 12 / 10;
    }
    value
}
pub fn ready(c: &Character, v: &Vitals) -> bool {
    // Retail offers a level past 50 and then clamps it away; it is not
    // offered here.
    !v.is_dead() && c.level() < super::MAX_LEVEL && c.experience >= c.experience_goal()
}

#[derive(Clone, Debug)]
pub struct Advancement {
    member: Entity,
    original: Character,
    known: LearnedSpells,
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
    fn new(member: Entity, original: &Character, known: &LearnedSpells) -> Self {
        Self {
            member,
            original: original.clone(),
            known: known.clone(),
            profession: original.profession,
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
        if profession == Profession::Valkyrie && self.original.gender != Gender::Female {
            return false;
        }
        let minimums = profession_row(profession).minimums;
        let deficits: Vec<_> =
            minimums.iter().map(|(a, &n)| n - n.min(self.original.attributes[a])).collect();
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
        c.experience_previous_goal = c.experience_goal();
        c.profession = self.profession;
        c.profession_levels[self.profession] += 1;
        for (a, &minimum) in &profession_row(self.profession).minimums {
            c.attributes[a] = c.attributes[a].max(minimum);
        }
        if self.profession != self.original.profession {
            let row = profession_row(self.profession);
            for &s in row.skills.iter().chain(std::iter::once(&row.bonus_skill)) {
                c.skill_points[s] = c.skill_points[s].max(5);
            }
        }
        if magic::caster_level(&c, c.profession) == 1
            && magic::caster_level(&self.original, c.profession) < 1
        {
            for book in magic::profession_books(c.profession) {
                let s = magic::book_skill(book);
                c.skill_points[s] = c.skill_points[s].max(5);
            }
        }
        if magic::caster_level(&c, c.profession) > 0 {
            c.spell_picks.grant(c.profession, 1);
        }
        c
    }
    pub fn preview(&self) -> Character {
        self.allocate(self.baseline())
    }
    fn allocate(&self, mut c: Character) -> Character {
        for (a, &n) in &self.attributes {
            c.attributes[a] += n;
        }
        for (s, &n) in &self.skills {
            c.skill_points[s] += n;
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
        6 - preview.attributes.iter().map(|(a, &n)| n - self.original.attributes[a]).sum::<u32>()
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
            && c.attributes[a] < 100
            && c.attributes[a] - self.original.attributes[a] < 3
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
            if s as usize >= Skill::PowerStrike as usize && !c.skill_available(s) {
                self.skills[s] = 0;
            }
        }
        true
    }
    pub fn learned(&self) -> LearnedSpells {
        let mut known = self.known.clone();
        known.0.extend(self.spells.iter().copied());
        if self.spells.contains(&SpellId::RETURN_TO_PORTAL)
            || self.spells.contains(&SpellId::SET_PORTAL)
        {
            known.0.extend([SpellId::RETURN_TO_PORTAL, SpellId::SET_PORTAL]);
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
            && c.skill_points[s] < 75
            && magic::skill_available(c, &draft.learned, catalog, s)
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
        self.spells.contains(&id)
            || (matches!(
                id,
                crate::content::spells::SpellId::RETURN_TO_PORTAL
                    | crate::content::spells::SpellId::SET_PORTAL
            ) && self.spells.iter().any(|s| {
                matches!(
                    *s,
                    crate::content::spells::SpellId::RETURN_TO_PORTAL
                        | crate::content::spells::SpellId::SET_PORTAL
                )
            }))
    }
    pub fn can_learn(&self, id: SpellId, catalog: &SpellCatalog) -> bool {
        self.learnable(&self.evaluate(), id, catalog)
    }
    pub fn learnable(&self, draft: &Evaluation, id: SpellId, catalog: &SpellCatalog) -> bool {
        // Retail excludes this draft's magic-skill allocations from spell eligibility.
        !self.known.0.contains(&id)
            && catalog.get(id).is_some_and(|s| magic::can_learn(&draft.baseline, s))
    }
    fn remaining_picks(&self, catalog: &SpellCatalog) -> Option<SpellPicks> {
        let c = self.baseline();
        let mut picks = c.spell_picks.clone();
        for &id in &self.spells {
            if !self.can_learn(id, catalog) || !picks.spend(&c, catalog.get(id)?.books) {
                return None;
            }
        }
        Some(picks)
    }
    pub fn spell_remaining(&self, catalog: &SpellCatalog) -> usize {
        self.remaining_picks(catalog).map_or(0, |p| p.available(&self.baseline()))
    }
    pub fn toggle_spell(&mut self, id: SpellId, catalog: &SpellCatalog) -> bool {
        // Retail charges the linked portal pair at Set Portal (75) in spell order.
        let id = if id == SpellId::RETURN_TO_PORTAL { SpellId::SET_PORTAL } else { id };
        if self.selected_spell(id) {
            self.spells.remove(&id);
            if matches!(
                id,
                crate::content::spells::SpellId::RETURN_TO_PORTAL
                    | crate::content::spells::SpellId::SET_PORTAL
            ) {
                self.spells.remove(&SpellId::RETURN_TO_PORTAL);
                self.spells.remove(&SpellId::SET_PORTAL);
            }
        } else {
            if !self.can_learn(id, catalog) {
                return false;
            }
            self.spells.insert(id);
            if self.remaining_picks(catalog).is_none() {
                self.spells.remove(&id);
                return false;
            }
        }
        let c = self.preview();
        let learned = self.learned();
        for s in EnumSet::<Skill>::all() {
            if !magic::skill_available(&c, &learned, catalog, s) {
                self.skills[s] = 0;
            }
        }
        true
    }
    fn finish(&self, catalog: &SpellCatalog) -> Result<(Character, LearnedSpells), &'static str> {
        if !self.eligible_profession(self.profession) {
            return Err("This profession is unavailable.");
        }
        if EnumSet::<Attribute>::all().iter().any(|a| self.can_adjust_attribute(a, true)) {
            return Err("Allocate the remaining attribute points.");
        }
        if EnumSet::<Skill>::all().iter().any(|s| self.can_adjust_skill(s, true, catalog)) {
            return Err("Allocate the remaining skill points.");
        }
        let mut c = self.preview();
        let learned = self.learned();
        if self
            .skills
            .iter()
            .any(|(s, &n)| n > 0 && !magic::skill_available(&c, &learned, catalog, s))
        {
            return Err("Refund training in unavailable skills.");
        }
        c.spell_picks =
            self.remaining_picks(catalog).ok_or("The spell selection is no longer valid.")?;
        Ok((c, self.learned()))
    }
}

#[derive(bevy::ecs::system::SystemParam)]
pub struct Progression<'w, 's> {
    roster: Query<'w, 's, &'static PartyRoster>,
    members: Query<
        'w,
        's,
        (
            &'static mut Character,
            &'static mut Vitals,
            &'static mut LearnedSpells,
            &'static mut Carried,
        ),
    >,
    inventory: Query<'w, 's, &'static mut PartyInventory>,
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
    ids: ResMut<'w, crate::game::inventory::ItemIds>,
    rng: ResMut<'w, crate::game::rng::GameRng>,
    news: MessageWriter<'w, crate::game::inventory::ItemNews>,
}
impl Progression<'_, '_> {
    pub fn eligibility(&self, member: Entity) -> Result<(), &'static str> {
        if !self.blocked.is_empty() {
            return Err("Finish the current encounter or journey first.");
        }
        if !self.roster.single().is_ok_and(|r| r.contains(&member)) {
            return Err("This character is no longer in the party.");
        }
        let (c, v, ..) = self.members.get(member).unwrap();
        if !ready(c, v) {
            return Err("This character is not ready to gain a level.");
        }
        Ok(())
    }
    pub fn begin(&self, member: Entity) -> Result<Advancement, &'static str> {
        self.eligibility(member)?;
        let (c, _, known, ..) = self.members.get(member).unwrap();
        Ok(Advancement::new(member, c, known))
    }
    pub fn accept(&mut self, draft: &Advancement) -> Result<(), &'static str> {
        self.eligibility(draft.member)?;
        let spells = &self.content.spells;
        let items = &self.content.items;
        let (mut c, mut v, mut known, mut carried) = self.members.get_mut(draft.member).unwrap();
        if *c != draft.original || *known != draft.known {
            return Err("This character's training changed. Reopen level-up.");
        }
        let (next, learned) = draft.finish(spells)?;
        // Stage only inventory work that can fail. Full destinations preserve all owners.
        let mut inventory =
            self.inventory.single_mut().map_err(|_| "The party inventory is unavailable.")?;
        let mut equipment = carried.clone();
        let mut pool = inventory.pool.clone();
        for slot in EnumSet::<crate::game::inventory::EquipSlot>::all() {
            if let Some(worn) = &equipment.equipment[slot] {
                let definition = &items[worn.stack.item];
                if !next.can_use(definition) {
                    let stack = equipment.equipment[slot].take().unwrap().stack;
                    if equipment
                        .receive(stack, items)
                        .and_then(|s| pool.receive(s, items))
                        .is_some()
                    {
                        return Err("Make room for equipment this profession cannot wear.");
                    }
                }
            }
        }
        // Preserve deficits, conditions and recovery fractions; advancement is not a full heal.
        // The spells on the member count for both maxima.
        let (old, new) = (c.stats(&v.enchantments), next.stats(&v.enchantments));
        let (hp, stamina) = (
            resized(v.hp, old.max_hp(), new.max_hp()),
            resized(v.stamina, old.max_stamina(), new.max_stamina()),
        );
        (v.hp, v.stamina) = (hp, stamina);
        *c = next;
        *known = learned;
        *carried = equipment;
        inventory.pool = pool;
        // A Gadgeteer's omnigun follows the committed level.
        let character = c.clone();
        let roster = self.roster.single().map_err(|_| "The party is unavailable.")?;
        let mut owners = self.members.transmute_lens::<&mut Carried>();
        let told = crate::game::inventory::omnigun::advance(
            draft.member,
            &character,
            roster,
            &mut owners.query(),
            &mut inventory,
            items,
            (&mut self.ids, &mut self.rng),
        );
        self.news.write_batch(told);
        Ok(())
    }
}
