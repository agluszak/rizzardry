//! Retail CharGeneration.cpp level-up rules. Editing owns only a draft;
//! acceptance revalidates the actor and commits training and its consequences.
use super::*;
use crate::content::spells::{SpellCatalog, SpellId, Spellbook};
use crate::game::inventory::{Carried, PartyInventory};
use crate::game::magic::{self, LearnedSpells, SpellPoints};
use crate::game::party::roster::PartyRoster;
use enumset::EnumSet;
use std::collections::BTreeSet;

/// Saved picks retain their school when changing profession. Bishop awards
/// are unrestricted, matching skill_costs and magic_bonus_pool in retail.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpellPicks {
    books: [u16; 4],
    general: u16,
}
impl SpellPicks {
    pub fn total(&self) -> u32 {
        self.books.iter().map(|&n| u32::from(n)).sum::<u32>() + u32::from(self.general)
    }
    pub(super) fn grant(&mut self, profession: Profession, amount: u16) {
        if profession == Profession::Bishop {
            self.general = self.general.saturating_add(amount);
        } else if let Some(book) = magic::profession_books(profession).iter().next() {
            self.books[book as usize] = self.books[book as usize].saturating_add(amount);
        }
    }
    fn available(&self, c: &Character) -> u32 {
        if magic::caster_level(c, c.profession) < 1 {
            return 0;
        }
        u32::from(self.general)
            + magic::profession_books(c.profession)
                .iter()
                .map(|book| u32::from(self.books[book as usize]))
                .sum::<u32>()
    }
    fn spend(&mut self, c: &Character, books: EnumSet<Spellbook>) -> bool {
        let active = magic::profession_books(c.profession);
        if let Some(book) = (active & books).iter().find(|&b| self.books[b as usize] > 0) {
            self.books[book as usize] -= 1;
        } else if self.general > 0 {
            self.general -= 1;
        } else if c.profession == Profession::Bishop {
            let Some(book) = active.iter().find(|&b| self.books[b as usize] > 0) else {
                return false;
            };
            self.books[book as usize] -= 1;
        } else {
            return false;
        }
        true
    }
}

/// CalcXPGoal adds the current profession's increment to the threshold
/// actually passed; switching profession never reprices previous levels.
pub(super) fn experience_increment(profession: Profession, level: u8) -> u32 {
    let weight = profession_row(profession).experience;
    if level == 1 {
        return weight;
    }
    let mut value = weight << (u32::from(level.min(10)) - 2);
    for _ in 10..level {
        value = (u64::from(value) * 12 / 10).min(u64::from(u32::MAX)) as u32;
    }
    value
}
pub fn ready(c: &Character, v: &Vitals) -> bool {
    !v.is_dead() && c.level < u8::MAX && c.experience >= c.experience_goal()
}

#[derive(Clone, Debug)]
pub struct Advancement {
    member: Entity,
    original: Character,
    known: LearnedSpells,
    profession: Profession,
    attributes: EnumMap<Attribute, u8>,
    skills: EnumMap<Skill, u8>,
    spells: BTreeSet<SpellId>,
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
    pub fn profession(&self) -> Profession {
        self.profession
    }
    pub fn eligible_profession(&self, profession: Profession) -> bool {
        if profession == Profession::Valkyrie && self.original.gender != Gender::Female {
            return false;
        }
        let minimums = profession_row(profession).minimums;
        let deficits: Vec<_> =
            minimums.iter().map(|(a, &n)| n.saturating_sub(self.original.attributes[a])).collect();
        deficits.iter().all(|&n| n <= 3) && deficits.iter().map(|&n| u16::from(n)).sum::<u16>() <= 6
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
        c.level += 1;
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
        let mut c = self.baseline();
        for (a, &n) in &self.attributes {
            c.attributes[a] += n;
        }
        for (s, &n) in &self.skills {
            c.skill_points[s] += n;
        }
        c
    }
    pub fn attribute_remaining(&self) -> u8 {
        let c = self.preview();
        6 - c.attributes.iter().map(|(a, &n)| n - self.original.attributes[a]).sum::<u8>()
    }
    pub fn skill_remaining(&self) -> u8 {
        9 - self.skills.values().sum::<u8>()
    }
    pub fn allocated_attribute(&self, a: Attribute) -> u8 {
        self.attributes[a]
    }
    pub fn allocated_skill(&self, s: Skill) -> u8 {
        self.skills[s]
    }
    pub fn can_adjust_attribute(&self, a: Attribute, up: bool) -> bool {
        if !up {
            return self.attributes[a] > 0;
        }
        let c = self.preview();
        self.attribute_remaining() > 0
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
        if !up {
            return self.skills[s] > 0;
        }
        let c = self.preview();
        self.skill_remaining() > 0
            && self.skills[s] < 3
            && c.skill_points[s] < 75
            && magic::skill_available(&c, Some(&self.learned()), Some(catalog), s)
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
        // Retail excludes this draft's magic-skill allocations from spell eligibility.
        !self.known.0.contains(&id)
            && catalog.get(id).is_some_and(|s| magic::can_learn(&self.baseline(), s))
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
    pub fn spell_remaining(&self, catalog: &SpellCatalog) -> u32 {
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
            if !magic::skill_available(&c, Some(&learned), Some(catalog), s) {
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
            .any(|(s, &n)| n > 0 && !magic::skill_available(&c, Some(&learned), Some(catalog), s))
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
            &'static mut SpellPoints,
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
    content: crate::game::data::Content<'w>,
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
        let (c, v, ..) = self.members.get(member).map_err(|_| "Character data is unavailable.")?;
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
        let spells = self.content.spells().ok_or("Spell definitions are not ready.")?;
        let items = self.content.items().ok_or("Item definitions are not ready.")?;
        let (mut c, mut v, mut known, mut points, mut carried) =
            self.members.get_mut(draft.member).unwrap();
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
                let definition = items
                    .get(worn.stack.item)
                    .ok_or("An equipped item's definition is missing.")?;
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
        v.hp = resized(v.hp, c.max_hp(), next.max_hp());
        v.stamina = resized(v.stamina, c.max_stamina(), next.max_stamina());
        let old_max = magic::max_points(&c, &known, spells);
        let new_max = magic::max_points(&next, &learned, spells);
        for realm in EnumSet::<Realm>::all() {
            points.current[realm] = resized(points.current[realm], old_max[realm], new_max[realm]);
        }
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
fn resized(current: u32, old: u32, new: u32) -> u32 {
    current.saturating_add(new.saturating_sub(old)).saturating_sub(old.saturating_sub(new)).min(new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{
        items::{ItemCatalog, ItemDefinition, ItemId, ItemKind},
        spells::SpellDefinition,
    };
    use crate::game::{
        data::GameData,
        inventory::{EquipSlot, Equipped, ItemIds, ItemStack, POOL_LIMIT},
        party::roster::{self, PartyMember},
    };
    use bevy::ecs::system::SystemState;

    fn complete(draft: &mut Advancement, spells: &SpellCatalog) {
        while let Some(a) =
            EnumSet::<Attribute>::all().iter().find(|&a| draft.can_adjust_attribute(a, true))
        {
            draft.adjust_attribute(a, true);
        }
        while let Some(s) =
            EnumSet::<Skill>::all().iter().find(|&s| draft.can_adjust_skill(s, true, spells))
        {
            draft.adjust_skill(s, true, spells);
        }
    }
    fn character(profession: Profession) -> Character {
        let mut c = Character::new("Tester", Race::Human, Gender::Female, profession);
        c.experience = c.experience_goal();
        c
    }
    #[test]
    fn a_profession_change_spends_only_the_new_level_and_keeps_past_xp_prices() {
        let mut original = character(Profession::Fighter);
        original.attributes = EnumMap::from_fn(|_| 80);
        original.skill_practice[Skill::Sword] = 7;
        let mut draft = Advancement::new(Entity::PLACEHOLDER, &original, &default());
        assert!(draft.choose_profession(Profession::Mage));
        assert_eq!(draft.attribute_remaining(), 6);
        assert_eq!(draft.skill_remaining(), 9);
        for _ in 0..3 {
            assert!(draft.adjust_attribute(Attribute::Intelligence, true));
        }
        assert!(!draft.adjust_attribute(Attribute::Intelligence, true));
        complete(&mut draft, &default());
        let (next, _) = draft.finish(&default()).unwrap();
        assert_eq!(
            (
                next.level,
                next.profession_levels[Profession::Fighter],
                next.profession_levels[Profession::Mage]
            ),
            (2, 1, 1)
        );
        assert_eq!(next.original_profession, Profession::Fighter);
        assert_eq!(next.experience, 1000);
        assert_eq!(next.experience_previous_goal, 1000);
        assert_eq!(next.experience_goal(), 2200);
        assert_eq!(next.skill_practice[Skill::Sword], 7);
        assert_eq!(next.spell_picks.total(), 1);
        assert_eq!(original.level, 1, "editing/cancel never changes the source");
    }
    #[test]
    fn profession_minimums_use_the_same_six_points_and_three_point_limit() {
        let mut c = character(Profession::Fighter);
        c.attributes = EnumMap::from_fn(|_| 80);
        c.attributes[Attribute::Intelligence] = 57; // Mage needs 60.
        let mut draft = Advancement::new(Entity::PLACEHOLDER, &c, &default());
        assert!(draft.choose_profession(Profession::Mage));
        assert_eq!(draft.attribute_remaining(), 3);
        assert_eq!(draft.preview().attributes[Attribute::Intelligence], 60);
        assert!(!draft.can_adjust_attribute(Attribute::Intelligence, true));
        assert!(draft.choose_profession(Profession::Fighter));
        assert_eq!(draft.attribute_remaining(), 6);
        c.attributes[Attribute::Intelligence] = 56;
        let draft = Advancement::new(Entity::PLACEHOLDER, &c, &default());
        assert!(!draft.eligible_profession(Profession::Mage));
    }
    #[test]
    fn hybrid_unlock_and_saved_school_picks_follow_profession_history() {
        let mut c = character(Profession::Samurai);
        c.level = 4;
        c.profession_levels[Profession::Samurai] = 4;
        let draft = Advancement::new(Entity::PLACEHOLDER, &c, &default());
        assert_eq!(draft.preview().skill_points[Skill::Wizardry], 5);
        assert_eq!(draft.spell_remaining(&default()), 1);
        let mut c = character(Profession::Mage);
        c.attributes = EnumMap::from_fn(|_| 80);
        c.spell_picks.grant(Profession::Mage, 2);
        let mut draft = Advancement::new(Entity::PLACEHOLDER, &c, &default());
        assert!(draft.choose_profession(Profession::Priest));
        assert_eq!(draft.spell_remaining(&default()), 1, "old Wizardry picks cannot pay Divinity");
        complete(&mut draft, &default());
        let (mut priest, _) = draft.finish(&default()).unwrap();
        assert_eq!(priest.spell_picks.total(), 3);
        priest.experience = priest.experience_goal();
        let mut returning = Advancement::new(Entity::PLACEHOLDER, &priest, &default());
        assert!(returning.choose_profession(Profession::Mage));
        assert_eq!(returning.spell_remaining(&default()), 3);
    }
    #[test]
    fn portal_pair_costs_one_pick_and_draft_skill_points_cannot_unlock_spells() {
        let mut c = character(Profession::Mage);
        c.level = 10;
        c.attributes = EnumMap::from_fn(|_| 80);
        c.profession_levels[Profession::Mage] = 10;
        c.skill_points[Skill::Wizardry] = 60;
        let mut spells = SpellCatalog(vec![SpellDefinition::default(); 76]);
        for id in [73, 75] {
            spells.0[id] = SpellDefinition {
                id: SpellId(id as u8),
                realm: Some(Realm::Mental),
                level: 1,
                books: Spellbook::Wizardry.into(),
                ..default()
            };
        }
        let mut draft = Advancement::new(Entity::PLACEHOLDER, &c, &default());
        assert!(draft.toggle_spell(SpellId::SET_PORTAL, &spells));
        assert_eq!(draft.spell_remaining(&spells), 0);
        assert!(draft.learned().0.contains(&SpellId::RETURN_TO_PORTAL));
        assert!(magic::skill_available(
            &draft.preview(),
            Some(&draft.learned()),
            Some(&spells),
            Skill::MentalMagic
        ));
        complete(&mut draft, &spells);
        let (trained, learned) = draft.finish(&spells).unwrap();
        let mut next = Advancement::new(Entity::PLACEHOLDER, &trained, &learned);
        assert!(next.choose_profession(Profession::Fighter));
        assert!(!magic::skill_available(
            &next.preview(),
            Some(&learned),
            Some(&spells),
            Skill::MentalMagic
        ));
        assert!(draft.toggle_spell(SpellId::RETURN_TO_PORTAL, &spells));
        assert_eq!(draft.spell_remaining(&spells), 1);
        assert!(draft.learned().0.is_empty());
        c.skill_points[Skill::Wizardry] = 10; // 12 effective, below rank 2.
        spells.0[1] = SpellDefinition {
            id: SpellId::ACID_SPLASH,
            realm: Some(Realm::Fire),
            level: 2,
            books: Spellbook::Wizardry.into(),
            ..default()
        };
        let mut draft = Advancement::new(Entity::PLACEHOLDER, &c, &default());
        for _ in 0..3 {
            assert!(draft.adjust_skill(Skill::Wizardry, true, &spells));
        }
        assert!(magic::can_learn(&draft.preview(), &spells.0[1]));
        assert!(!draft.can_learn(SpellId::ACID_SPLASH, &spells));
    }
    fn world(c: Character, catalog: ItemCatalog) -> (World, Entity, Entity) {
        let mut world = World::new();
        world.init_resource::<ItemIds>();
        world.init_resource::<crate::game::rng::GameRng>();
        world.init_resource::<Messages<crate::game::inventory::ItemNews>>();
        let mut items = Assets::<ItemCatalog>::default();
        let items_handle = items.add(catalog);
        let mut spells = Assets::<SpellCatalog>::default();
        let spells_handle = spells.add(SpellCatalog::default());
        world.insert_resource(items);
        world.insert_resource(spells);
        world.insert_resource(GameData {
            items: items_handle,
            spells: spells_handle,
            loot: default(),
            species: default(),
        });
        let member = world
            .spawn((
                Vitals::new(&c),
                c,
                LearnedSpells::default(),
                SpellPoints::default(),
                Carried::default(),
            ))
            .id();
        let party = roster::test_party(&mut world, &[member]);
        (world, member, party)
    }
    fn begin(world: &mut World, member: Entity) -> Advancement {
        SystemState::<Progression>::new(world).get_mut(world).unwrap().begin(member).unwrap()
    }
    fn accept(world: &mut World, draft: &Advancement) -> Result<(), &'static str> {
        SystemState::<Progression>::new(world).get_mut(world).unwrap().accept(draft)
    }
    /// Committing a change to Gadgeteer brings the welcome omnigun (retail
    /// `CommitCharacter`, `UpgradeProfessionClassItem`).
    #[test]
    fn becoming_a_gadgeteer_brings_the_omnigun() {
        let mut original = character(Profession::Fighter);
        original.attributes = EnumMap::from_fn(|_| 80);
        let catalog = ItemCatalog(
            (0..=599)
                .map(|id| ItemDefinition {
                    id: ItemId(id),
                    kind: if id == 599 { ItemKind::OMNIGUN } else { ItemKind::default() },
                    ..default()
                })
                .collect(),
        );
        let (mut w, member, _) = world(original, catalog);
        let mut draft = begin(&mut w, member);
        assert!(draft.choose_profession(Profession::Gadgeteer));
        complete(&mut draft, &default());
        accept(&mut w, &draft).unwrap();
        assert!(w.get::<Carried>(member).unwrap().stacks().any(|s| s.item == ItemId(599)));
        let news: Vec<_> =
            w.resource_mut::<Messages<crate::game::inventory::ItemNews>>().drain().collect();
        assert!(matches!(
            news.last(),
            Some(crate::game::inventory::ItemNews::Omnigun { tier: 0, .. })
        ));
    }

    #[test]
    fn acceptance_revalidates_actor_and_preserves_live_damage_and_conditions() {
        let (mut w, member, _) = world(character(Profession::Fighter), default());
        w.get_mut::<Character>(member).unwrap().experience -= 1;
        assert!(
            SystemState::<Progression>::new(&mut w).get_mut(&mut w).unwrap().begin(member).is_err()
        );
        w.get_mut::<Character>(member).unwrap().experience += 1;
        let mut draft = begin(&mut w, member);
        complete(&mut draft, &default());
        let original = w.get::<Character>(member).unwrap().clone();
        let hp = w.get::<Vitals>(member).unwrap().hp;
        w.get_mut::<Vitals>(member).unwrap().hp = 0;
        assert!(accept(&mut w, &draft).is_err(), "death after opening prevents advancement");
        assert_eq!(*w.get::<Character>(member).unwrap(), original);
        w.get_mut::<Vitals>(member).unwrap().hp = hp;
        {
            let mut v = w.get_mut::<Vitals>(member).unwrap();
            v.hp -= 2;
            v.stamina -= 5;
            v.inflict(Condition::Poisoned, Lasting::Rounds(10));
        }
        let entering = w
            .spawn(crate::game::world::level::Destination::new(
                crate::game::world::level::Level::test(default()),
                default(),
                crate::game::world::level::Entry::Travel,
            ))
            .id();
        assert!(accept(&mut w, &draft).is_err());
        assert_eq!(w.get::<Character>(member).unwrap().level, 1);
        w.despawn(entering);
        accept(&mut w, &draft).unwrap();
        let c = w.get::<Character>(member).unwrap();
        let v = w.get::<Vitals>(member).unwrap();
        assert_eq!(v.hp, c.max_hp() - 2);
        assert_eq!(v.stamina, c.max_stamina() - 5);
        assert_eq!(v.conditions, [(Condition::Poisoned, Lasting::Rounds(10))]);
        assert_eq!(c.experience, original.experience);
        assert!(accept(&mut w, &draft).is_err(), "duplicate acceptance cannot gain another level");
        w.get_mut::<Character>(member).unwrap().experience = 10000;
        let mut stale = begin(&mut w, member);
        complete(&mut stale, &default());
        w.get_mut::<Character>(member).unwrap().skill_points[Skill::Sword] += 1;
        assert!(accept(&mut w, &stale).is_err());
        w.entity_mut(member).remove::<PartyMember>();
        assert!(accept(&mut w, &stale).is_err(), "a retained profile is not a party member");
    }
    #[test]
    fn profession_change_moves_unusable_equipment_or_refuses_without_partial_changes() {
        let mut c = character(Profession::Fighter);
        c.attributes = EnumMap::from_fn(|_| 80);
        let item = ItemDefinition {
            id: ItemId(0),
            professions: Profession::Fighter.into(),
            races: EnumSet::all(),
            genders: EnumSet::all(),
            ..default()
        };
        let mut ids = ItemIds::default();
        let stack = ItemStack::new(&item, &mut ids, 1, true);
        let backpack = std::array::from_fn(|_| Some(ItemStack::new(&item, &mut ids, 1, true)));
        let full_pool = (0..POOL_LIMIT).map(|_| ItemStack::new(&item, &mut ids, 1, true)).collect();
        let (mut w, member, party) = world(c, ItemCatalog(vec![item]));
        w.get_mut::<Carried>(member).unwrap().equipment[EquipSlot::Primary] =
            Some(Equipped::new(stack.clone()));
        w.get_mut::<Carried>(member).unwrap().backpack = backpack;
        w.get_mut::<PartyInventory>(party).unwrap().pool.stacks = full_pool;
        let before = w.get::<Carried>(member).unwrap().clone();
        let mut draft = begin(&mut w, member);
        assert!(draft.choose_profession(Profession::Mage));
        complete(&mut draft, &default());
        assert!(accept(&mut w, &draft).is_err());
        assert_eq!(*w.get::<Carried>(member).unwrap(), before);
        assert_eq!(w.get::<Character>(member).unwrap().level, 1);
        w.get_mut::<Carried>(member).unwrap().backpack[0] = None;
        accept(&mut w, &draft).unwrap();
        let carried = w.get::<Carried>(member).unwrap();
        assert!(carried.equipment[EquipSlot::Primary].is_none());
        assert_eq!(carried.backpack[0], Some(stack));
    }
    #[test]
    fn learning_at_advancement_preserves_mana_deficits_and_fractional_recovery() {
        let c = character(Profession::Priest);
        let (mut w, member, _) = world(c, default());
        let spell = |id| SpellDefinition {
            id: SpellId(id),
            realm: Some(Realm::Divine),
            level: 1,
            cost: 3,
            books: Spellbook::Divinity.into(),
            ..default()
        };
        let catalog = SpellCatalog(vec![SpellDefinition::default(), spell(1), spell(2)]);
        let known = LearnedSpells([SpellId::ACID_SPLASH].into());
        let old = magic::max_points(w.get::<Character>(member).unwrap(), &known, &catalog);
        let mut mana = SpellPoints::full(old);
        mana.current[Realm::Divine] -= 2;
        mana.fraction[Realm::Divine] = 0.25;
        w.entity_mut(member).insert((known, mana));
        let handle = w.resource::<GameData>().spells.clone();
        w.resource_mut::<Assets<SpellCatalog>>().insert(handle.id(), catalog).unwrap();
        let mut draft = begin(&mut w, member);
        let catalog = w.resource::<Assets<SpellCatalog>>().get(&handle).unwrap();
        assert!(draft.toggle_spell(SpellId::BLESS, catalog));
        complete(&mut draft, catalog);
        accept(&mut w, &draft).unwrap();
        let c = w.get::<Character>(member).unwrap();
        let learned = w.get::<LearnedSpells>(member).unwrap();
        let maximum = magic::max_points(
            c,
            learned,
            w.resource::<Assets<SpellCatalog>>().get(&handle).unwrap(),
        );
        let mana = w.get::<SpellPoints>(member).unwrap();
        assert_eq!(mana.current[Realm::Divine], maximum[Realm::Divine] - 2);
        assert_eq!(mana.fraction[Realm::Divine], 0.25);
        assert_eq!(learned.0, [SpellId::ACID_SPLASH, SpellId::BLESS].into());
        assert_eq!(c.spell_picks.total(), 0);
    }
}
