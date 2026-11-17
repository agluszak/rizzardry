//! Spellcasting (`docs/rules/casting.md`): the spells a character has learned,
//! the spell points of each realm, and the rules of a cast, each a function of
//! the facts it is about.
//!
//! - [`may`]: who may cast what, from what, and when (2)
//! - [`power`]: the power of a cast and the strength it goes off at (3)
//! - [`roll`]: how likely a cast is to fail, and what a roll becomes (4)
//! - [`reach`]: whom a cast reaches, and what a backfire turns it on (5)
//! - [`cost`]: what a cast costs and trains (7)
//! - [`workings`]: what a utility spell does once it comes off (8)
//!
//! What lands is [`crate::game::hits`]. A cast is one chain, for a learned
//! spell and an item's, in a fight and out of it: [`casting`] reads the facts,
//! asks the rules in the order of the page and makes happen what they decide;
//! a utility spell then does its own thing, one observer to a kind.

pub mod aim;
pub mod casting;
pub mod cost;
pub(crate) mod dialogue;
pub mod landed;
pub mod may;
pub mod occasion;
pub(crate) mod pointcasts;
pub mod portals;
pub mod power;
pub mod reach;
pub mod roll;
pub(crate) mod traps;
pub(crate) mod utilities;
pub mod workings;

use std::collections::BTreeMap;

use bevy::prelude::*;
use enum_map::EnumMap;
use enumset::EnumSet;

use crate::content::characters::Trait;
use crate::content::characters::{Realm, Skill};
use crate::content::spells::{SpellCatalog, SpellDefinition, SpellId, Spellbook, min_caster_level};
use crate::game::GameplaySet;
use crate::game::characters::{Calling, Profession, Training};
use crate::game::combat::Battle;
use crate::game::data::Catalogs;
use crate::game::items::Items;
use crate::game::rng::GameRng;

/// The spells an actor has learned, each with what it is to the learner,
/// read from the spell as it is learned. Creation picks the first ones.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct LearnedSpells(BTreeMap<SpellId, Learned>);

/// A spell learned: its realm, and what it costs at power 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Learned {
    pub realm: Option<Realm>,
    pub cost: u32,
}

impl LearnedSpells {
    /// These spells, learned.
    pub fn of(ids: impl IntoIterator<Item = SpellId>, spells: &SpellCatalog) -> Self {
        let mut learned = Self::default();
        for id in ids {
            learned.learn(&spells[id]);
        }
        learned
    }

    pub fn learn(&mut self, spell: &SpellDefinition) {
        self.0.insert(spell.id, Learned { realm: spell.realm, cost: spell.cost });
    }

    pub fn knows(&self, id: SpellId) -> bool {
        self.0.contains_key(&id)
    }

    pub fn ids(&self) -> impl Iterator<Item = SpellId> + '_ {
        self.0.keys().copied()
    }

    /// Those of a realm.
    pub fn in_realm(&self, realm: Realm) -> impl Iterator<Item = &Learned> + Clone + '_ {
        self.0.values().filter(move |learned| learned.realm == Some(realm))
    }

    /// What either has learned.
    pub fn with(mut self, other: &Self) -> Self {
        self.0.extend(other.0.iter().map(|(&id, &learned)| (id, learned)));
        self
    }
}

/// Skill eligibility combines profile rules with the actual learned spell set.
pub fn skill_available(
    (calling, abilities, traits): (&Calling, &crate::game::actors::Abilities, EnumSet<Trait>),
    learned: &LearnedSpells,
    skill: Skill,
) -> bool {
    let can_use = |skill| crate::game::characters::skill_available(calling, abilities, skill);
    if skill as usize >= Skill::Wizardry as usize
        && (skill as usize) < Skill::PowerStrike as usize
        && traits.contains(Trait::CannotLearn)
    {
        return false;
    }
    if let Some(realm) = skill.realm() {
        if !calling.profession.row().books.iter().any(|book| can_use(book_skill(book))) {
            return false;
        }
        return learned.in_realm(realm).next().is_some();
    }
    can_use(skill)
}

/// An actor's spell points, realm by realm: the most each can hold, a
/// figure written by [`crate::game::actors::figures::spell_points`] from the
/// spells learned, the level, Piety and the magic skills; and what is spent
/// of it. What is left is the most less what is spent, so when the most moves
/// what is left moves with it (retail `RebuildRealmSpellPointCeilings`); a
/// loss of the most can leave a realm below nothing, and its return does not
/// refill it for free. A fraction of a point recovered waits for the next.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct SpellPoints {
    most: EnumMap<Realm, u32>,
    spent: EnumMap<Realm, i32>,
    pub fraction: EnumMap<Realm, f32>,
}

impl SpellPoints {
    /// The most a realm can hold.
    pub fn most(&self, realm: Realm) -> u32 {
        self.most[realm]
    }

    pub fn current(&self, realm: Realm) -> i32 {
        self.most[realm] as i32 - self.spent[realm]
    }

    pub fn available(&self, realm: Realm) -> u32 {
        self.current(realm).max(0) as u32
    }

    /// The points of every realm together, and the most they could be.
    pub fn total(&self) -> (u32, u32) {
        let have = self.most.iter().map(|(realm, _)| self.available(realm)).sum();
        (have, self.most.values().sum())
    }

    pub fn drain_realm(&mut self, realm: Realm, amount: u32) -> u32 {
        let taken = amount.min(self.available(realm));
        self.spend(realm, taken);
        taken
    }

    /// Record spending after the caller has checked what is available.
    pub fn spend(&mut self, realm: Realm, amount: u32) {
        self.spent[realm] += amount as i32;
    }

    /// Drain randomly chosen realms, at most fifty draws (retail
    /// `DrainCharacterSpellPoints`). Fractional recovery is not spent.
    pub fn drain(&mut self, amount: u32, rng: &mut GameRng) -> u32 {
        let mut remaining = amount;
        for _ in 0..50 {
            if remaining == 0 {
                break;
            }
            let realm = Realm::try_from(rng.below(6) as u8).unwrap();
            remaining -= self.drain_realm(realm, remaining);
        }
        amount - remaining
    }

    /// Restore the largest realm deficits first (retail
    /// `RestoreCharacterSpellPointsEvenly`). Ties keep the initial deficit order
    /// (initial ties use realm order). Excess is wasted; recovery fractions stay.
    pub fn restore_evenly(&mut self, amount: u32) -> u32 {
        let mut deficits: Vec<_> =
            self.spent.iter().map(|(realm, &spent)| (realm, spent.max(0))).collect();
        deficits.sort_by_key(|&(_, missing)| std::cmp::Reverse(missing));
        let mut restored = 0;
        for _ in 0..amount {
            let largest = deficits.iter().map(|&(_, missing)| missing).max().unwrap_or(0);
            if largest == 0 {
                break;
            }
            let (realm, missing) =
                deficits.iter_mut().find(|(_, missing)| *missing == largest).unwrap();
            self.spent[*realm] -= 1;
            *missing -= 1;
            restored += 1;
        }
        restored
    }

    /// Each realm gains `amount` points, or loses them when negative, as
    /// what is worn gives or takes a minute (retail `GameTurnsPassedChar`).
    pub fn adjust(&mut self, amount: i32) {
        for realm in EnumSet::<Realm>::all() {
            if amount < 0 {
                self.drain_realm(realm, (-amount) as u32);
            } else if self.spent[realm] > 0 {
                self.spent[realm] = (self.spent[realm] - amount).max(0);
            }
        }
    }

    /// Whether any realm is short of its most.
    pub fn short(&self) -> bool {
        self.spent.values().any(|&spent| spent > 0)
    }

    /// Full pools need no write unless a previous operation left a fraction
    /// for the aging boundary to clear.
    pub(crate) fn needs_recovery(&self) -> bool {
        self.short() || self.fraction.values().any(|&fraction| fraction != 0.0)
    }

    /// Recover over `ticks` aging ticks at `(0.65·most + 20)/240` per tick
    /// times `scale` (retail aging, `GameplayTime.cpp`).
    pub fn recover(&mut self, ticks: u32, scale: f32) {
        for (realm, &most) in &self.most {
            if self.spent[realm] <= 0 {
                self.fraction[realm] = 0.0;
                continue;
            }
            self.fraction[realm] += ticks as f32 * (0.65 * most as f32 + 20.0) / 240.0 * scale;
            let whole = self.fraction[realm].trunc();
            self.fraction[realm] -= whole;
            self.spent[realm] = (self.spent[realm] - whole as i32).max(0);
            if self.spent[realm] == 0 {
                self.fraction[realm] = 0.0;
            }
        }
    }

    /// The most each realm can hold is this now; what is spent stays spent.
    pub(crate) fn set_most(&mut self, most: EnumMap<Realm, u32>) {
        self.most = most;
    }
}

/// The share of spell points recovered while health recovers at
/// `health_scale`: never under half (retail `spellScale`).
pub fn recovery_scale(health_scale: f32) -> f32 {
    health_scale.max(0.5)
}

/// Retail `GetProfessionCasterLevel`; -1 for a profession that cannot cast.
pub fn caster_level(calling: &Calling, profession: Profession) -> i32 {
    profession_caster_level(profession, calling.levels[profession])
}

/// A profession's caster level after `levels` of it; -1 if it cannot cast.
fn profession_caster_level(profession: Profession, levels: u8) -> i32 {
    profession.row().caster_offset.map_or(-1, |offset| i32::from(levels) + offset)
}

pub(crate) fn book_skill(book: Spellbook) -> Skill {
    match book {
        Spellbook::Wizardry => Skill::Wizardry,
        Spellbook::Divinity => Skill::Divinity,
        Spellbook::Alchemy => Skill::Alchemy,
        Spellbook::Psionics => Skill::Psionics,
    }
}

/// The books the character has ever cast from.
fn character_books(character: &Calling) -> EnumSet<Spellbook> {
    character
        .levels
        .iter()
        .filter(|&(_, &levels)| levels > 0)
        .map(|(profession, _)| profession.row().books)
        .fold(character.profession.row().books, |all, books| all | books)
}

/// The book skill a spell is cast by: the best of the character's (retail
/// `GetBestSpellbookSkillForSpell`).
pub(crate) fn best_book(
    (calling, training): (&Calling, &Training),
    spell: &SpellDefinition,
) -> Skill {
    let books = spell.books & character_books(calling);
    let books = if books.is_empty() { spell.books } else { books };
    books.iter().map(book_skill).max_by_key(|&s| training.0[s]).unwrap_or(Skill::Wizardry)
}

/// Whether the character may learn a spell now (retail `CanLearnSpell`): it is
/// in one of their books and its level is within both their caster level and
/// their realm and book training. `(followed, caster)` are their caster
/// levels: of the profession followed now, and in all for the spell's books
/// ([`crate::game::actors::CasterLevels`]).
pub fn can_learn(
    (character, traits, training): (&Calling, EnumSet<Trait>, &Training),
    skills: &crate::game::actors::Skills,
    (followed, caster): (i32, i32),
    spell: &SpellDefinition,
) -> bool {
    let books = character.profession.row().books;
    if spell.id.0 == 0 || spell.id.0 >= SpellId::LEARNABLE || (spell.books & books).is_empty() {
        return false;
    }
    if traits.contains(Trait::CannotLearn) || followed <= 0 || spell.realm.is_none() {
        return false;
    }
    let by_level = (1..=7).filter(|&l| min_caster_level(l) <= caster).max().unwrap_or(0);
    let realm_points = spell.realm.map_or(0, |r| training.0[r.skill()]);
    let book = [Spellbook::Wizardry, Spellbook::Divinity, Spellbook::Alchemy, Spellbook::Psionics]
        .into_iter()
        .filter(|book| spell.books.contains(*book))
        .max_by_key(|book| training.0[book_skill(*book)]);
    let best = book.map_or(0, |book| skills[book_skill(book)]);
    let by_skill = (realm_points / 10 + best) / 15 + 1;
    spell.level <= by_level.min(by_skill)
}

/// How many first spells a new character picks: two for the pure casters,
/// none for the others (retail `CharGeneration.cpp`).
pub fn starting_picks(profession: Profession) -> usize {
    if profession_caster_level(profession, 1) > 0 { 2 } else { 0 }
}

/// Spell points per realm at a level (retail `RecalculateRealmSpellPoints`):
/// `(books + 3 * realm skill + Piety) * 0.02 * (spells known in the realm +
/// level + 1)`, rounded, where `books` is the best magic skill, half the
/// next, a quarter and an eighth of the others, at most 125; none in a realm
/// without a learned spell, and never under its dearest learned spell.
pub fn max_points(
    level: u32,
    piety: u32,
    skill: impl Fn(Skill) -> u32,
    learned: &LearnedSpells,
) -> EnumMap<Realm, u32> {
    let mut books = [Skill::Wizardry, Skill::Divinity, Skill::Alchemy, Skill::Psionics].map(&skill);
    books.sort_unstable_by(|a, b| b.cmp(a));
    let weight =
        (books[0] as f32 + (books[1] / 2) as f32 + (books[2] / 4) as f32 + (books[3] / 8) as f32)
            .min(125.0);
    EnumMap::from_fn(|realm| {
        let known = learned.in_realm(realm);
        let (count, dearest) = known.fold((0, 0), |(n, most), s| (n + 1, s.cost.max(most)));
        if count == 0 {
            return 0;
        }
        let levels = (count + level + 1) as f32;
        let points = ((weight + 3.0 * skill(realm.skill()) as f32 + piety as f32) * 0.02 * levels
            + 0.5) as u32;
        points.max(dearest)
    })
}

/// The strength a cast's victims save against, and the percent its rolls are
/// raised by (retail `CastSpellFromSource`).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Strength {
    pub power: u32,
    pub against: u32,
    pub boost: u32,
}

/// Whether a cast came off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Cast,
    Fizzled,
    /// It turned on the caster.
    Backfired,
}

pub struct MagicPlugin;

impl Plugin for MagicPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((occasion::plugin, casting::plugin))
            .add_message::<MagicRequest>()
            .add_message::<casting::MagicNews>()
            .add_observer(pointcasts::answer)
            .add_systems(
                Update,
                (accept_magic, pointcasts::expire_lures, pointcasts::follow_lures)
                    .chain()
                    .after(GameplaySet::Bodies)
                    .before(GameplaySet::Combat)
                    .run_if(crate::game::world::level::level_active),
            );
    }
}

/// A member asks to cast or use an item. In a fight it becomes their order
/// for the next round; otherwise it happens at once.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct MagicRequest {
    pub caster: Entity,
    pub choice: casting::MagicChoice,
}

/// Accept magic requests (retail `ChooseAction`): in combat only record the
/// choice, except for a utility spell, which is cast at once, unless it is cast
/// at a spot, which waits for its turn as any other order; outside combat, cast
/// now.
pub fn accept_magic(
    mut requests: MessageReader<MagicRequest>,
    mut commands: Commands,
    battle: Option<Res<Battle>>,
    mut orders: Query<&mut crate::game::party::Order>,
    held: Items,
    content: Res<Catalogs>,
) {
    for &MagicRequest { caster, choice } in requests.read() {
        let Some(spell) = choice.source.spell_id(&held) else {
            commands.trigger(casting::Refused { caster, refusal: may::Refusal::NoItem });
            continue;
        };
        let working = content.spells[spell].facts.working;
        let at_once = battle.is_none()
            || working.is_some_and(|working| !working.at_a_spot())
            || matches!(choice.aim, reach::Aim::Device(_));
        if at_once {
            commands.trigger(casting::Cast { caster, spell, choice, again: false });
        } else {
            orders.get_mut(caster).unwrap().set_if_neq(crate::game::party::Order::Magic(choice));
        }
    }
}

#[cfg(test)]
pub(crate) fn test_set_points(world: &mut World, member: Entity, realm: Realm, current: i32) {
    let mut points = world.get_mut::<SpellPoints>(member).unwrap();
    points.spent[realm] = points.most[realm] as i32 - current;
}

#[cfg(test)]
pub(crate) fn test_points(world: &World, member: Entity, realm: Realm) -> i32 {
    world.get::<SpellPoints>(member).unwrap().current(realm)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::content::characters::Skill;
    use crate::content::items::Dice;
    use crate::content::items::ItemCatalog;
    use crate::content::species::MonsterCatalog;
    use crate::content::spells::{PowerChoice, Reach, Targeting, Usable};
    use crate::game::characters::{Gender, Race, Training, born};

    /// Magic as the game has it.
    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_resource::<crate::game::factions::Factions>()
            .init_resource::<crate::game::time::GameClock>()
            .add_message::<crate::game::factions::FactionNews>()
            .init_resource::<crate::game::difficulty::Difficulty>()
            .init_resource::<avian3d::collider_tree::ColliderTrees>()
            .insert_resource(GameRng::seeded(3))
            .add_message::<crate::game::combat::CombatNews>()
            .add_plugins((
                crate::game::hits::HitsPlugin,
                crate::game::items::plugin,
                crate::game::actors::plugin,
                MagicPlugin,
            ));
        app
    }

    pub(crate) fn heal_wounds() -> SpellDefinition {
        SpellDefinition {
            id: SpellId::HEAL_WOUNDS,
            name: "Heal Wounds".into(),
            realm: Some(Realm::Divine),
            level: 1,
            cost: 3,
            books: Spellbook::Divinity | Spellbook::Alchemy | Spellbook::Psionics,
            power: PowerChoice::ChosenOrRepeat,
            reach: Reach::Long,
            targeting: Targeting::Ally,
            usable: Usable::Always,
            dice: Some(Dice { base: 0, count: 4, sides: 4 }),
            ..default()
        }
    }

    #[test]
    fn using_items_spends_a_dose_or_a_charge_and_the_last_dose_still_heals() {
        use crate::content::items::{
            EquipClass, ItemDefinition, ItemId, LootTables, QuantityKind, UseType,
        };
        use crate::content::spells::ItemSpell;
        use crate::game::characters::Vitals;
        use crate::game::inventory::EquipSlot;
        use crate::game::items::{Charges, Count, Place, Put, make};
        use bevy::ecs::system::RunSystemOnce;
        use casting::{MagicChoice, MagicNews, Source};
        use may::Refusal;
        use reach::Aim;
        let mut app = app();
        let potion = ItemDefinition {
            id: ItemId(0),
            name: "Potion of Light Heal".into(),
            class: EquipClass::Potion,
            use_type: UseType::Potion,
            quantity: QuantityKind::Stack,
            stack_limit: 5,
            spell: Some(ItemSpell { spell: SpellId::HEAL_WOUNDS, power: 1 }),
            ..default()
        };
        let wand = ItemDefinition {
            id: ItemId(1),
            name: "Healing Wand".into(),
            armor_class_bonus: 1,
            class: EquipClass::MiscMagic,
            use_type: UseType::Charged,
            quantity: QuantityKind::Charges,
            professions: enumset::EnumSet::all(),
            races: enumset::EnumSet::all(),
            genders: enumset::EnumSet::all(),
            spell: Some(ItemSpell { spell: SpellId::HEAL_WOUNDS, power: 1 }),
            ..default()
        };
        let world = app.world_mut();
        let items = ItemCatalog(vec![potion, wand]);
        let species = MonsterCatalog::default();
        let loot = LootTables::default();
        let spells =
            SpellCatalog((0..=6).map(|i| if i == 6 { heal_wounds() } else { default() }).collect());
        world.insert_resource(crate::game::data::Catalogs {
            items,
            loot,
            species,
            spells,
            ..default()
        });
        let member =
            world.spawn(born("Drinker", Race::Human, Gender::Male, Profession::Fighter)).id();
        world.get_mut::<Vitals>(member).unwrap().health.current = 1;
        let made = |world: &mut World, id: ItemId, amount: u16, at: Place| {
            let item = world
                .run_system_once(
                    move |mut commands: Commands, content: Res<crate::game::data::Catalogs>| {
                        make(&mut commands, &content.items[id], amount, true)
                    },
                )
                .unwrap();
            world.flush();
            world.trigger(Put { item, at });
            world.flush();
            item
        };
        let potion_id = made(world, ItemId(0), 2, Place::Pack { member, slot: 0 });
        let wand_id = made(world, ItemId(1), 1, Place::Slot { member, slot: EquipSlot::Misc1 });
        crate::game::party::roster::test_party(world, &[member]);
        world.spawn(crate::game::world::level::Level::test(default()));
        world.spawn((
            crate::game::party::body::PartyBody::default(),
            avian3d::prelude::Position::default(),
        ));
        let mut news = Vec::new();
        let mut request = |app: &mut App, source| {
            let choice = MagicChoice { source, aim: Aim::Body(member) };
            app.world_mut().write_message(MagicRequest { caster: member, choice });
            app.update();
            let told: Vec<_> =
                app.world_mut().resource_mut::<Messages<MagicNews>>().drain().collect();
            let first = told.first().cloned();
            news.extend(told);
            first
        };
        let hp = |app: &App| app.world().get::<Vitals>(member).unwrap().health.current;
        let doses = |app: &App| app.world().get::<Count>(potion_id).map(|count| count.of);
        assert!(matches!(request(&mut app, Source::Item(potion_id)), Some(MagicNews::Done(_))));
        assert_eq!(doses(&app), Some(1));
        let before = hp(&app);
        assert!(before > 1, "4d4 heals at power 1");
        // Hurt again, and drink the last one: it is gone, and it heals.
        app.world_mut().get_mut::<Vitals>(member).unwrap().health.current = 1;
        assert!(matches!(request(&mut app, Source::Item(potion_id)), Some(MagicNews::Done(_))));
        assert_eq!(doses(&app), None, "the last dose is used up");
        assert!(hp(&app) > 1, "the effect outlives the potion");
        assert!(matches!(
            request(&mut app, Source::Item(potion_id)),
            Some(MagicNews::Refused { refusal: Refusal::NoItem, .. })
        ));
        // A charged item keeps its body when emptied, and refuses then.
        assert!(matches!(request(&mut app, Source::Item(wand_id)), Some(MagicNews::Done(_))));
        assert_eq!(app.world().get::<Charges>(wand_id).unwrap().left, 0);
        assert!(matches!(
            request(&mut app, Source::Item(wand_id)),
            Some(MagicNews::Refused { refusal: Refusal::NoCharges, .. })
        ));
    }

    /// A utility spell is cast like any other up to the point where it lands: it
    /// is paid for in points and stamina (cost 2 at power 1 is 2 points, and a
    /// level 1 spell at power 1 tires by 1 + 1 = 2), and then does its own
    /// thing: Identify Item reveals what the party holds; Set Portal saves
    /// where the party stands on its caster.
    #[test]
    fn a_utility_spell_is_paid_for_like_any_cast_and_then_does_its_own_thing() {
        use crate::content::items::{ItemDefinition, ItemId, LootTables, QuantityKind};
        use crate::game::characters::Vitals;
        use crate::game::items::{Known, Place, Put, make};
        use bevy::ecs::system::RunSystemOnce;
        use casting::{MagicChoice, MagicNews, Source};
        use reach::Aim;
        let mut app = app();
        let scroll = ItemDefinition {
            id: ItemId(0),
            name: "Scroll".into(),
            quantity: QuantityKind::Stack,
            stack_limit: 5,
            identify_difficulty: 3,
            ..default()
        };
        let world = app.world_mut();
        let mut definitions = vec![SpellDefinition::default(); 117];
        definitions[usize::from(SpellId::IDENTIFY_ITEM.0)] = SpellDefinition {
            id: SpellId::IDENTIFY_ITEM,
            realm: Some(Realm::Divine),
            level: 1,
            cost: 2,
            books: Spellbook::Divinity.into(),
            targeting: Targeting::Item,
            ..default()
        };
        definitions[usize::from(SpellId::SET_PORTAL.0)] = SpellDefinition {
            id: SpellId::SET_PORTAL,
            realm: Some(Realm::Divine),
            level: 1,
            cost: 2,
            books: Spellbook::Divinity.into(),
            targeting: Targeting::Caster,
            ..default()
        };
        world.insert_resource(crate::game::data::Catalogs {
            items: ItemCatalog(vec![scroll]),
            loot: LootTables::default(),
            species: MonsterCatalog::default(),
            spells: SpellCatalog::new(definitions),
            ..default()
        });
        let learned = LearnedSpells::of(
            [SpellId::IDENTIFY_ITEM, SpellId::SET_PORTAL],
            &world.resource::<crate::game::data::Catalogs>().spells,
        );
        let member = world
            .spawn(born("Seer", Race::Human, Gender::Male, Profession::Priest))
            .insert((learned, SpellPoints::default()))
            .id();
        let mut training = world.get_mut::<Training>(member).unwrap();
        training.0[Skill::Divinity] = 100;
        training.0[Skill::DivineMagic] = 100;
        let item = world
            .run_system_once(|mut commands: Commands, content: Res<crate::game::data::Catalogs>| {
                make(&mut commands, &content.items[ItemId(0)], 1, false)
            })
            .unwrap();
        world.flush();
        world.trigger(Put { item, at: Place::Pack { member, slot: 0 } });
        world.flush();
        crate::game::party::roster::test_party(world, &[member]);
        world.spawn(crate::game::world::level::Level::test(default()));
        world.spawn((
            crate::game::party::body::PartyBody::default(),
            avian3d::prelude::Position::default(),
        ));
        world.run_schedule(crate::game::actors::Settle);
        let before = test_points(world, member, Realm::Divine);
        let stamina = world.get::<Vitals>(member).unwrap().stamina.current;
        assert!(!world.get::<Known>(item).unwrap().identified);
        let source = Source::Spell { spell: SpellId::IDENTIFY_ITEM, power: 1 };
        world.write_message(MagicRequest {
            caster: member,
            choice: MagicChoice { source, aim: Aim::Item(item) },
        });
        app.update();
        let world = app.world_mut();
        let news: Vec<_> = world.resource_mut::<Messages<MagicNews>>().drain().collect();
        assert!(
            matches!(news.as_slice(), [MagicNews::Done(report)] if report.outcome == Outcome::Cast),
            "{news:?}"
        );
        assert!(
            world.get::<Known>(item).unwrap().identified,
            "3 a power reveals a difficulty of 3"
        );
        assert_eq!(test_points(world, member, Realm::Divine), before - 2);
        assert_eq!(world.get::<Vitals>(member).unwrap().stamina.current, stamina - 2);
        // An item wholly known is no target; the refusal costs nothing.
        let after = test_points(world, member, Realm::Divine);
        world.write_message(MagicRequest {
            caster: member,
            choice: MagicChoice { source, aim: Aim::Item(item) },
        });
        app.update();
        let world = app.world_mut();
        let news: Vec<_> = world.resource_mut::<Messages<MagicNews>>().drain().collect();
        assert!(
            matches!(news.as_slice(), [MagicNews::Refused { refusal: may::Refusal::NoTarget, .. }]),
            "{news:?}"
        );
        assert_eq!(test_points(world, member, Realm::Divine), after);
        // Set Portal is cast at nothing, and its caster carries the portal.
        assert!(world.get::<portals::Portal>(member).is_none());
        let source = Source::Spell { spell: SpellId::SET_PORTAL, power: 1 };
        world.write_message(MagicRequest {
            caster: member,
            choice: MagicChoice { source, aim: Aim::Body(member) },
        });
        app.update();
        let world = app.world_mut();
        let news: Vec<_> = world.resource_mut::<Messages<MagicNews>>().drain().collect();
        assert!(
            matches!(news.as_slice(), [MagicNews::Done(report)] if report.outcome == Outcome::Cast),
            "{news:?}"
        );
        assert!(world.get::<portals::Portal>(member).is_some());
    }

    #[test]
    fn a_priest_picks_two_first_level_divine_spells_and_gains_their_points() {
        use crate::game::actors::{Attributes, Skills, Traits};
        use crate::game::characters::Calling;
        let mut app = App::new();
        app.add_plugins(crate::game::actors::plugin).init_resource::<GameRng>();
        let world = app.world_mut();
        let priest =
            world.spawn(born("Cleric", Race::Human, Gender::Male, Profession::Priest)).id();
        let fighter =
            world.spawn(born("Brute", Race::Human, Gender::Male, Profession::Fighter)).id();
        world.run_schedule(crate::game::actors::Settle);
        let world = app.world();
        let calling = |body| world.get::<Calling>(body).unwrap();
        let learns = |body, spell: &SpellDefinition| {
            let (traits, training) =
                (world.get::<Traits>(body).unwrap(), world.get::<Training>(body).unwrap());
            let levels = world.get::<crate::game::actors::CasterLevels>(body).unwrap();
            can_learn(
                (calling(body), traits.0, training),
                world.get::<Skills>(body).unwrap(),
                (levels.followed, levels.caster_level(spell)),
                spell,
            )
        };
        let heal = heal_wounds();
        assert_eq!(
            (
                starting_picks(calling(priest).profession),
                starting_picks(calling(fighter).profession)
            ),
            (2, 0)
        );
        assert!(learns(priest, &heal) && !learns(fighter, &heal));
        let higher = SpellDefinition { level: 2, ..heal_wounds() };
        assert!(!learns(priest, &higher), "level 1 casters learn level 1 spells only");
        let mut learned = LearnedSpells::default();
        learned.learn(&heal_wounds());
        let skills = world.get::<Skills>(priest).unwrap();
        let piety =
            world.get::<Attributes>(priest).unwrap()[crate::content::characters::Attribute::Piety];
        let points = max_points(1, piety, |skill| skills[skill], &learned);
        assert!(points[Realm::Divine] >= 3, "never under the dearest learned spell");
        assert_eq!(points[Realm::Fire], 0, "no spell, no points");
    }

    /// A pool whose realms can hold these, all of it spent.
    fn spent(most: &[(Realm, u32)]) -> SpellPoints {
        let mut points = SpellPoints::default();
        for &(realm, amount) in most {
            points.most[realm] = amount;
            points.spent[realm] = amount as i32;
        }
        points
    }

    #[test]
    fn mana_restoration_levels_deficits_and_wastes_excess_without_resetting_recovery() {
        let mut points = spent(&[(Realm::Fire, 10), (Realm::Water, 5), (Realm::Air, 3)]);
        points.fraction[Realm::Fire] = 0.75;
        assert_eq!(points.restore_evenly(8), 8);
        // Five go to Fire; then the equal deficits share, in realm order.
        assert_eq!((points.current(Realm::Fire), points.current(Realm::Water)), (7, 1));
        assert_eq!(points.current(Realm::Air), 0);
        assert_eq!(points.restore_evenly(100), 10);
        assert!(!points.short());
        assert_eq!(points.restore_evenly(5), 0);
    }

    #[test]
    fn restorative_and_enchantment_items_pay_once_and_keep_their_target() {
        use crate::content::items::{
            Dice, EquipClass, ItemDefinition, ItemId, LootTables, QuantityKind, UseType,
        };
        use crate::content::spells::ItemSpell;
        use crate::game::characters::{Condition, Lasting, Vitals};
        use crate::game::items::{Count, Place, Put, make};
        use bevy::ecs::system::RunSystemOnce;
        use casting::{MagicChoice, MagicNews, Source};
        use reach::Aim;

        for id in [
            SpellId::GUARDIAN_ANGEL,
            SpellId::ARMORPLATE,
            SpellId::HASTE,
            SpellId::SUPERMAN,
            SpellId::BODY_OF_STONE,
            SpellId::RESTORE_HEALTH,
            SpellId::RESTORE_MAGIC,
            SpellId::SMELLING_SALTS,
            SpellId::CURE_POISON,
        ] {
            let mut app = app();
            let definition = ItemDefinition {
                id: ItemId(0),
                class: EquipClass::Potion,
                use_type: if id == SpellId::SMELLING_SALTS {
                    UseType::Thrown
                } else {
                    UseType::Potion
                },
                quantity: QuantityKind::Stack,
                stack_limit: 5,
                professions: EnumSet::all(),
                races: EnumSet::all(),
                genders: EnumSet::all(),
                spell: Some(ItemSpell { spell: id, power: 4 }),
                ..default()
            };
            let world = app.world_mut();
            let items = ItemCatalog(vec![definition]);
            let species = MonsterCatalog::default();
            let loot = LootTables::default();
            let mut definitions = vec![SpellDefinition::default(); 117];
            definitions[usize::from(SpellId::HEAL_WOUNDS.0)] = heal_wounds();
            definitions[usize::from(id.0)] = SpellDefinition {
                id,
                realm: Some(Realm::Divine),
                targeting: Targeting::Ally,
                dice: Some(Dice { base: 0, count: 1, sides: 1 }),
                ..default()
            };
            let spells = SpellCatalog::new(definitions);
            world.insert_resource(crate::game::data::Catalogs {
                items,
                loot,
                species,
                spells,
                ..default()
            });
            let user =
                world.spawn(born("User", Race::Human, Gender::Male, Profession::Priest)).id();
            world.get_mut::<Training>(user).unwrap().0[Skill::ThrowingAndSling] = 100;
            let stack_id = world
                .run_system_once(
                    |mut commands: Commands, content: Res<crate::game::data::Catalogs>| {
                        make(&mut commands, &content.items[ItemId(0)], 2, true)
                    },
                )
                .unwrap();
            world.flush();
            world.trigger(Put { item: stack_id, at: Place::Pack { member: user, slot: 0 } });
            world.flush();
            let mut ailing = crate::game::characters::Conditions::default();
            ailing.afflict(Condition::Asleep, Lasting::Rounds(8));
            ailing.afflict(Condition::Unconscious, Lasting::Rounds(8));
            ailing.poison(Lasting::Rounds(20), 10);
            let learned = LearnedSpells::of(
                [SpellId::HEAL_WOUNDS],
                &world.resource::<crate::game::data::Catalogs>().spells,
            );
            let ally = world
                .spawn(born("Ally", Race::Human, Gender::Male, Profession::Priest))
                .insert((ailing, learned, SpellPoints::default()))
                .id();
            let mut vitals = world.get_mut::<Vitals>(ally).unwrap();
            vitals.health.current = 1;
            vitals.stamina.current = 1;
            test_set_points(world, ally, Realm::Divine, 0);
            crate::game::party::roster::test_party(world, &[user, ally]);
            world.spawn(crate::game::world::level::Level::test(default()));
            world.spawn((
                crate::game::party::body::PartyBody::default(),
                avian3d::prelude::Position::default(),
            ));
            let user_before = world.get::<Vitals>(user).unwrap().clone();
            let ailed_before =
                world.get::<crate::game::characters::Conditions>(user).unwrap().clone();
            world.write_message(MagicRequest {
                caster: user,
                choice: MagicChoice { source: Source::Item(stack_id), aim: Aim::Body(ally) },
            });
            app.update();
            let world = app.world_mut();
            let news: Vec<_> = world.resource_mut::<Messages<MagicNews>>().drain().collect();
            assert!(matches!(news.as_slice(), [MagicNews::Done(_), ..]), "{id:?}: {news:?}");
            assert_eq!(world.get::<Count>(stack_id).unwrap().of, 1);
            let user_after = world.get::<Vitals>(user).unwrap();
            assert_eq!(user_after.health.current, user_before.health.current);
            assert_eq!(
                world.get::<crate::game::characters::Conditions>(user).unwrap(),
                &ailed_before
            );
            let fatigue = user_before.stamina.current - user_after.stamina.current;
            if id == SpellId::SMELLING_SALTS {
                assert_eq!(fatigue, 10);
            } else {
                assert!((1..=2).contains(&fatigue), "using an item still pays its normal fatigue");
            }
            let vitals = world.get::<Vitals>(ally).unwrap();
            let ailing = world.get::<crate::game::characters::Conditions>(ally).unwrap();
            match id {
                SpellId::GUARDIAN_ANGEL => {
                    use crate::game::actors::{Laid, Laying};
                    use bevy::ecs::system::RunSystemOnce;
                    let laid = world
                        .run_system_once(move |laid: Laid| {
                            (laid.listed(user).count(), laid.get(ally, id))
                        })
                        .unwrap();
                    assert_eq!(laid.0, 0);
                    assert_eq!(laid.1.map(|angel| angel.spell), Some(id));
                    let absorb = |world: &mut World, amount| {
                        let through = world
                            .run_system_once(move |mut laying: Laying| laying.absorb(ally, amount))
                            .unwrap();
                        world.flush();
                        through
                    };
                    let stamina = world.get::<Vitals>(ally).unwrap().stamina.current;
                    let through = absorb(world, 4);
                    let mut vitals = world.get_mut::<Vitals>(ally).unwrap();
                    assert_eq!(vitals.hurt(through, None), 0);
                    assert_eq!(vitals.stamina.current, stamina, "absorbed damage does not fatigue");
                    let through = absorb(world, 2);
                    let mut vitals = world.get_mut::<Vitals>(ally).unwrap();
                    assert_eq!(vitals.hurt(through, None), 1);
                    assert!(vitals.is_dead());
                }
                SpellId::ARMORPLATE => {
                    use bevy::ecs::system::RunSystemOnce;
                    let party = crate::game::party::roster::the_party(world);
                    let (on_ally, on_party) = world
                        .run_system_once(move |laid: crate::game::actors::Laid| {
                            (laid.listed(ally).count(), laid.bonus(party).penetration)
                        })
                        .unwrap();
                    assert_eq!((on_ally, on_party), (0, 2));
                }
                SpellId::HASTE | SpellId::SUPERMAN | SpellId::BODY_OF_STONE => {
                    use bevy::ecs::system::RunSystemOnce;
                    let turns = world
                        .run_system_once(move |laid: crate::game::actors::Laid| {
                            laid.listed(ally).find(|(laid, _)| laid.spell == id).unwrap().1.unwrap()
                        })
                        .unwrap();
                    world.trigger(crate::game::actors::spells::Aged { ticks: turns });
                    world.flush();
                    let left = world
                        .run_system_once(move |laid: crate::game::actors::Laid| laid.get(ally, id))
                        .unwrap();
                    assert!(left.is_none());
                }
                SpellId::RESTORE_HEALTH => {
                    assert_eq!(vitals.health.current, 5, "power four heals 4d1");
                    assert_eq!(
                        vitals.stamina.current, 9,
                        "stamina rolls twice the dice, independently"
                    );
                }
                SpellId::RESTORE_MAGIC => {
                    assert!(test_points(world, ally, Realm::Divine) > 0);
                    assert_eq!((vitals.health.current, vitals.stamina.current), (1, 1));
                }
                SpellId::SMELLING_SALTS => {
                    assert!(!ailing.has(Condition::Asleep) && !ailing.has(Condition::Unconscious))
                }
                // Power four takes sixteen rounds off, and the poison weakens
                // by the same share, as aging would (retail
                // `TickCharacterCondition`).
                SpellId::CURE_POISON => {
                    assert_eq!(ailing.lasting(Condition::Poisoned), Some(Lasting::Rounds(4)));
                    assert_eq!(ailing.poison, 2);
                }
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn lowered_capacity_keeps_debt_until_the_capacity_returns() {
        let mut points = spent(&[(Realm::Divine, 20)]);
        points.spent[Realm::Divine] = 19;
        let mut low = EnumMap::default();
        low[Realm::Divine] = 10;
        points.set_most(low);
        assert_eq!(points.current(Realm::Divine), -9);
        assert_eq!(points.drain_realm(Realm::Divine, 5), 0);
        assert_eq!(power::affordable(points.available(Realm::Divine), 1), 0);
        low[Realm::Divine] = 20;
        points.set_most(low);
        assert_eq!(points.current(Realm::Divine), 1);
    }

    #[test]
    fn points_recover_by_retail_rate_and_never_beyond_the_maximum() {
        let mut points = spent(&[(Realm::Divine, 20)]);
        // (0.65·20 + 20)/240 = 0.1375 a tick at half rate: 0.06875.
        points.recover(100, recovery_scale(0.0));
        assert_eq!(points.current(Realm::Divine), 6);
        points.recover(10_000, 1.0);
        assert_eq!(points.current(Realm::Divine), 20);
        points.spent[Realm::Divine] = -5;
        points.fraction[Realm::Divine] = 0.5;
        points.recover(1, 1.0);
        assert_eq!(points.current(Realm::Divine), 25, "retail skips full realms");
        assert!(!points.needs_recovery());
    }
}
