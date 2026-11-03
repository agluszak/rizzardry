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
        self.spells.contains(&selection_key(id))
    }
    pub fn learnable(&self, draft: &Evaluation, id: SpellId, catalog: &SpellCatalog) -> bool {
        // Retail excludes this draft's magic-skill allocations from spell eligibility.
        !self.known.0.contains(&id)
            && catalog.get(id).is_some_and(|s| magic::can_learn(&draft.baseline, s))
    }
    fn remaining_picks(&self, draft: &Evaluation, catalog: &SpellCatalog) -> Option<SpellPicks> {
        let c = &draft.baseline;
        let mut picks = c.spell_picks.clone();
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
            if !magic::skill_available(&draft.preview, &learned, catalog, s) {
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
            n > 0 && !magic::skill_available(&draft.preview, &draft.learned, catalog, s)
        }) {
            return Err("Refund training in unavailable skills.");
        }
        let picks = self
            .remaining_picks(&draft, catalog)
            .ok_or("The spell selection is no longer valid.")?;
        let Evaluation { mut preview, learned, .. } = draft;
        preview.spell_picks = picks;
        Ok((preview, learned))
    }
}

/// The spell a selection holds it under: retail charges the linked portal pair
/// at Set Portal (75) in spell order.
fn selection_key(id: SpellId) -> SpellId {
    if id == SpellId::RETURN_TO_PORTAL { SpellId::SET_PORTAL } else { id }
}

/// What keeps a member from training: a fight, a journey, a conversation,
/// a travel question.
#[derive(bevy::ecs::query::QueryFilter)]
struct Busy {
    _any: Or<(
        With<crate::game::combat::Battle>,
        With<crate::game::world::level::Destination>,
        With<crate::game::npcs::conversation::Conversation>,
        With<crate::game::world::level::TravelQuestion>,
    )>,
}

/// Whether a member may gain a level now: nothing keeps the party busy,
/// they are still in it, and they are ready ([`ready`]).
fn eligibility(
    busy: &Query<(), Busy>,
    roster: &Query<&PartyRoster>,
    member: Entity,
    (character, vitals): (&Character, &Vitals),
) -> Result<(), &'static str> {
    if !busy.is_empty() {
        return Err("Finish the current encounter or journey first.");
    }
    if !roster.single().is_ok_and(|r| r.contains(&member)) {
        return Err("This character is no longer in the party.");
    }
    if !ready(character, vitals) {
        return Err("This character is not ready to gain a level.");
    }
    Ok(())
}

/// The level-up offer: whether a member may train now, and the draft to
/// work it out. Reads only; [`Progression`] accepts.
#[derive(bevy::ecs::system::SystemParam)]
pub struct LevelOffer<'w, 's> {
    roster: Query<'w, 's, &'static PartyRoster>,
    members: Query<'w, 's, (&'static Character, &'static Vitals, &'static LearnedSpells)>,
    busy: Query<'w, 's, (), Busy>,
}
impl LevelOffer<'_, '_> {
    pub fn eligibility(&self, member: Entity) -> Result<(), &'static str> {
        let (character, vitals, _) =
            self.members.get(member).map_err(|_| "This character is no longer in the party.")?;
        eligibility(&self.busy, &self.roster, member, (character, vitals))
    }
    pub fn begin(&self, member: Entity) -> Result<Advancement, &'static str> {
        self.eligibility(member)?;
        let (character, _, known) = self.members.get(member).unwrap();
        Ok(Advancement::new(member, character, known))
    }
}

/// Accepting a level-up draft: the training and its consequences for the
/// member, their equipment and the party's pool.
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
    busy: Query<'w, 's, (), Busy>,
    content: If<Res<'w, crate::game::data::Catalogs>>,
    ids: ResMut<'w, crate::game::inventory::ItemIds>,
    rng: ResMut<'w, crate::game::rng::GameRng>,
    news: MessageWriter<'w, crate::game::inventory::ItemNews>,
}
impl Progression<'_, '_> {
    fn eligibility(&self, member: Entity) -> Result<(), &'static str> {
        let (character, vitals, ..) =
            self.members.get(member).map_err(|_| "This character is no longer in the party.")?;
        eligibility(&self.busy, &self.roster, member, (character, vitals))
    }
    /// Commit the draft, revalidated against the member as they are now;
    /// nothing changes on refusal. Experience enough for another level
    /// returns its draft straight away.
    pub fn accept(&mut self, draft: &Advancement) -> Result<Option<Advancement>, &'static str> {
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
        v.follow_maxima(&next);
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
        let (character, vitals, known, _) = self.members.get(draft.member).unwrap();
        let next = eligibility(&self.busy, &self.roster, draft.member, (character, vitals))
            .ok()
            .map(|()| Advancement::new(draft.member, character, known));
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    /// The offer reads: a member with the experience for a level is offered
    /// one, unless the party is busy or they have left.
    #[test]
    fn the_offer_follows_readiness_membership_and_what_keeps_the_party_busy() {
        let mut world = World::new();
        let character = Character::new("Up", Race::Human, Gender::Male, Profession::Fighter);
        let goal = character.experience_goal();
        let vitals = Vitals::new(&character);
        let member = world.spawn((character, vitals, LearnedSpells::default())).id();
        let other = world.spawn(()).id();
        crate::game::party::roster::test_party(&mut world, &[member, other]);
        let offer = |world: &mut World| {
            world.run_system_once(move |offer: LevelOffer| offer.eligibility(member)).unwrap()
        };
        assert!(offer(&mut world).is_err(), "not enough experience yet");
        world.get_mut::<Character>(member).unwrap().experience = goal;
        assert_eq!(offer(&mut world), Ok(()));
        let fight = world.spawn(crate::game::combat::Battle::default()).id();
        assert!(offer(&mut world).is_err(), "not during a fight");
        world.despawn(fight);
        assert!(
            world.run_system_once(move |offer: LevelOffer| offer.begin(member)).unwrap().is_ok()
        );
        world.entity_mut(member).remove::<crate::game::party::roster::PartyMember>();
        assert!(offer(&mut world).is_err(), "not after leaving");
    }
}
