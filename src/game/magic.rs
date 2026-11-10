//! Spellcasting rules: who can cast what, spell points per realm, whether a
//! cast comes off, and what the modelled spells do to their victims (retail
//! `Magic.cpp`, `Magic Effects.cpp`, `Health Stamina Mana.cpp`).
//!
//! A learned spell and an item's spell share effect resolution but keep
//! their own validation and payment: casting spends spell points, using an
//! item spends a use (see `game::magic::casting`).

pub mod casting;
pub(crate) mod dialogue;
pub(crate) mod pointcasts;
pub mod portals;
pub(crate) mod traps;
pub(crate) mod utilities;

use crate::game::actors::Attributes;
use std::collections::BTreeSet;

use bevy::prelude::*;
use enum_map::EnumMap;
use enumset::EnumSet;

use crate::content::characters::Trait;
use crate::content::characters::{Attribute, Realm, Skill};
use crate::content::spells::{
    MAX_POWER, SpellCatalog, SpellDefinition, SpellId, Spellbook, Targeting, min_caster_level,
};
use crate::game::GameplaySet;
use crate::game::characters::{Character, Profession};
use crate::game::combat::Battle;
use crate::game::data::Catalogs;
use crate::game::items::Items;
use crate::game::rng::GameRng;

/// The spells a character has learned. Creation picks the first ones.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct LearnedSpells(pub BTreeSet<SpellId>);

/// Skill eligibility combines profile rules with the actual learned spell set.
pub fn skill_available(
    character: &Character,
    learned: &LearnedSpells,
    catalog: &SpellCatalog,
    skill: Skill,
) -> bool {
    if skill as usize >= Skill::Wizardry as usize
        && (skill as usize) < Skill::PowerStrike as usize
        && character.traits().contains(Trait::CannotLearn)
    {
        return false;
    }
    if let Some(realm) = skill.realm() {
        if !character
            .profession
            .row()
            .books
            .iter()
            .any(|book| character.skill_available(book_skill(book)))
        {
            return false;
        }
        return learned.0.iter().any(|&id| catalog[id].realm == Some(realm));
    }
    character.skill_available(skill)
}

/// Mana already spent and fractional recovery are history. The current pool is
/// the live maximum minus this deficit: retail's RebuildRealmSpellPointCeilings
/// adds each maximum change to current mana, preserving precisely this value.
/// A loss of capacity can leave signed current mana below zero; it is not a
/// fresh empty pool that a later capacity increase should refill for free.
/// Default means no spending: full mana at the actor's live capacity.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct SpellPoints {
    spent: EnumMap<Realm, i32>,
    pub fraction: EnumMap<Realm, f32>,
}

impl SpellPoints {
    pub fn current(&self, realm: Realm, maximum: &EnumMap<Realm, u32>) -> i32 {
        maximum[realm] as i32 - self.spent[realm]
    }

    pub fn available(&self, realm: Realm, maximum: &EnumMap<Realm, u32>) -> u32 {
        self.current(realm, maximum).max(0) as u32
    }

    pub fn drain_realm(&mut self, realm: Realm, amount: u32, maximum: &EnumMap<Realm, u32>) -> u32 {
        let taken = amount.min(self.available(realm, maximum));
        self.spend(realm, taken);
        taken
    }

    /// Record spending after the caller has checked what is available.
    pub fn spend(&mut self, realm: Realm, amount: u32) {
        self.spent[realm] += amount as i32;
    }

    /// Drain randomly chosen realms, at most fifty draws (retail
    /// `DrainCharacterSpellPoints`). Fractional recovery is not spent.
    pub fn drain(&mut self, amount: u32, maximum: &EnumMap<Realm, u32>, rng: &mut GameRng) -> u32 {
        let mut remaining = amount;
        for _ in 0..50 {
            if remaining == 0 {
                break;
            }
            let realm = Realm::try_from(rng.below(6) as u8).unwrap();
            let taken = self.drain_realm(realm, remaining, maximum);
            remaining -= taken;
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

    /// The highest power the realm's points pay for, 0 if none.
    pub fn affordable_power(&self, spell: &SpellDefinition, maximum: &EnumMap<Realm, u32>) -> u32 {
        let Some(realm) = spell.realm else { return 0 };
        if spell.cost == 0 {
            return MAX_POWER;
        }
        (self.available(realm, maximum) / spell.cost).min(MAX_POWER)
    }

    /// Whether any realm is short of its maximum.
    pub fn short(&self) -> bool {
        self.spent.values().any(|&spent| spent > 0)
    }

    /// Full pools need no write unless a previous operation left a fraction
    /// for the aging boundary to clear.
    pub(crate) fn needs_recovery(&self) -> bool {
        self.short() || self.fraction.values().any(|&fraction| fraction != 0.0)
    }

    /// Recover over `ticks` aging ticks at `(0.65·max + 20)/240` per tick
    /// times `scale` (retail aging, `GameplayTime.cpp`).
    pub fn recover(&mut self, ticks: u32, scale: f32, maximum: &EnumMap<Realm, u32>) {
        for (realm, &max) in maximum {
            if self.spent[realm] <= 0 {
                self.fraction[realm] = 0.0;
                continue;
            }
            self.fraction[realm] += ticks as f32 * (0.65 * max as f32 + 20.0) / 240.0 * scale;
            let whole = self.fraction[realm].trunc();
            self.fraction[realm] -= whole;
            self.spent[realm] = (self.spent[realm] - whole as i32).max(0);
            if self.spent[realm] == 0 {
                self.fraction[realm] = 0.0;
            }
        }
    }
}

/// Select an available target by angle to the heading, then distance.
pub fn target_ahead(
    eye: Vec3,
    forward: Vec3,
    reach: f32,
    candidates: impl IntoIterator<Item = (Entity, Vec3, f32)>,
) -> Option<Entity> {
    candidates
        .into_iter()
        .filter(|(_, _, distance)| *distance <= reach)
        .map(|(entity, at, distance)| {
            (entity, (at - eye).with_y(0.0).angle_between(forward.with_y(0.0)), distance)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1).then(a.2.total_cmp(&b.2)))
        .map(|(entity, ..)| entity)
}

/// The share of spell points recovered while health recovers at
/// `health_scale`: never under half (retail `spellScale`).
pub fn recovery_scale(health_scale: f32) -> f32 {
    health_scale.max(0.5)
}

/// Retail `GetProfessionCasterLevel`; -1 for a profession that cannot cast.
pub fn caster_level(character: &Character, profession: Profession) -> i32 {
    profession_caster_level(profession, character.profession_levels[profession])
}

/// A profession's caster level after `levels` of it; -1 if it cannot cast.
fn profession_caster_level(profession: Profession, levels: u8) -> i32 {
    profession.row().caster_offset.map_or(-1, |offset| i32::from(levels) + offset)
}

/// Retail `GetTotalCasterLevel` over `books`: the current profession's caster
/// level plus every other held profession sharing a book.
pub fn total_caster_level(character: &Character, books: EnumSet<Spellbook>) -> i32 {
    let mut total = caster_level(character, character.profession);
    for (profession, &levels) in &character.profession_levels {
        if levels != 0
            && profession != character.profession
            && !(profession.row().books & books).is_empty()
        {
            total += caster_level(character, profession).max(0);
        }
    }
    total
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
fn character_books(character: &Character) -> EnumSet<Spellbook> {
    character
        .profession_levels
        .iter()
        .filter(|&(_, &levels)| levels > 0)
        .map(|(profession, _)| profession.row().books)
        .fold(character.profession.row().books, |all, books| all | books)
}

/// The book skill a spell is cast by: the best of the character's (retail
/// `GetBestSpellbookSkillForSpell`).
pub(crate) fn best_book(character: &Character, spell: &SpellDefinition) -> Skill {
    let books = spell.books & character_books(character);
    let books = if books.is_empty() { spell.books } else { books };
    books.iter().map(book_skill).max_by_key(|&s| character.skill(s)).unwrap_or(Skill::Wizardry)
}

/// Retail `GetSpellCastingSkillLevel`: the book counts once, the realm four
/// times.
fn casting_skill(character: &Character, spell: &SpellDefinition) -> u32 {
    let realm = spell.realm.map_or(0, |r| character.skill(r.skill()));
    (character.skill(best_book(character, spell)) + realm * 4) / 5
}

/// Whether the character may learn a spell now (retail `CanLearnSpell`): it is
/// in one of their books and its level is within both their caster level and
/// their realm and book training.
pub fn can_learn(character: &Character, spell: &SpellDefinition) -> bool {
    let books = character.profession.row().books;
    if spell.id.0 == 0 || spell.id.0 >= SpellId::LEARNABLE || (spell.books & books).is_empty() {
        return false;
    }
    if character.traits().contains(Trait::CannotLearn)
        || caster_level(character, character.profession) <= 0
        || spell.realm.is_none()
    {
        return false;
    }
    let caster = total_caster_level(character, spell.books);
    let by_level = (1..=7).filter(|&l| min_caster_level(l) <= caster).max().unwrap_or(0);
    let realm_points = spell.realm.map_or(0, |r| character.skill_points[r.skill()]);
    let book = [Spellbook::Wizardry, Spellbook::Divinity, Spellbook::Alchemy, Spellbook::Psionics]
        .into_iter()
        .filter(|book| spell.books.contains(*book))
        .max_by_key(|book| character.skill_points[book_skill(*book)]);
    let best = book.map_or(0, |book| character.skill(book_skill(book)));
    let by_skill = (realm_points / 10 + best) / 15 + 1;
    spell.level <= by_level.min(by_skill)
}

/// How many first spells a new character picks: two for the pure casters,
/// none for the others (retail `CharGeneration.cpp`).
pub fn starting_picks(profession: Profession) -> usize {
    if profession_caster_level(profession, 1) > 0 { 2 } else { 0 }
}

impl Character {
    /// Spell points per realm (retail `RecalculateRealmSpellPoints`): none in
    /// a realm without a learned spell, and never under its dearest learned
    /// spell.
    pub fn max_points(
        &self,
        attributes: &Attributes,
        learned: &LearnedSpells,
        spells: &SpellCatalog,
    ) -> EnumMap<Realm, u32> {
        let character = self;
        let mut books: Vec<u32> =
            [Skill::Wizardry, Skill::Divinity, Skill::Alchemy, Skill::Psionics]
                .map(|s| character.skill(s))
                .to_vec();
        books.sort_unstable_by(|a, b| b.cmp(a));
        let weight = (books[0] as f32
            + (books[1] / 2) as f32
            + (books[2] / 4) as f32
            + (books[3] / 8) as f32)
            .min(125.0);
        let piety = attributes[Attribute::Piety] as f32;
        EnumMap::from_fn(|realm| {
            let known: Vec<_> = learned
                .0
                .iter()
                .filter_map(|&id| spells.get(id))
                .filter(|s| s.realm == Some(realm))
                .collect();
            if known.is_empty() {
                return 0;
            }
            let skill = character.skill(realm.skill()) as f32;
            let levels = (known.len() + usize::from(character.level()) + 1) as f32;
            let points = ((weight + 3.0 * skill + piety) * 0.02 * levels + 0.5) as u32;
            points.max(known.iter().map(|s| s.cost).max().unwrap_or(0))
        })
    }
}

/// The skill threshold a spell's difficulty band needs at power 7 (retail
/// `g_combat_effect_slot_spells_and_cast_success`).
const CAST_THRESHOLDS: [u32; 17] =
    [30, 40, 50, 58, 64, 70, 76, 81, 86, 90, 94, 97, 100, 102, 105, 107, 110];

/// Retail `GetSpellFailureChance`: the skill a spell's difficulty band
/// needs at `power`, and up to 70% failure for falling short of it.
pub(crate) fn spell_failure(skill: u32, spell: &SpellDefinition, power: u32) -> u32 {
    let band = (spell.cost / 2 + spell.level).min(16) as usize;
    let need = CAST_THRESHOLDS[band] * power / 7;
    if skill < need { (need - skill) * 70 / need } else { 0 }
}

/// The percent chance a learned cast fails (retail `ExecuteCharacterSpellCast`),
/// before combat pace: the casting skill's shortfall, and the caster levels the
/// spell's level and power call for. It is not held to 100: a chance past it
/// still counts for a third of itself in the chance to backfire.
pub fn fail_chance(character: &Character, spell: &SpellDefinition, power: u32) -> u32 {
    let mut chance = spell_failure(casting_skill(character, spell), spell, power);
    let caster = total_caster_level(character, spell.books);
    let short = min_caster_level(spell.level) - caster - 1 + power as i32;
    if short > 0 {
        chance += spell.level * short as u32;
    }
    chance
}

/// Whether a silence stops this caster casting this learned spell (retail
/// `IsSpellBlockedForCharacter`, `Magic.cpp:138-146`): it stops all but an
/// Alchemy spell in the hands of someone with the Alchemy skill.
pub fn silence_stops(character: &Character, spell: &SpellDefinition) -> bool {
    !(spell.books.contains(Spellbook::Alchemy) && character.skill(Skill::Alchemy) > 0)
}

/// Retail's Power Cast bonus: `(level >> 2) + 1` while the skill is available.
fn power_cast(character: &Character) -> u32 {
    if character.skill_available(Skill::PowerCast) {
        (character.skill(Skill::PowerCast) >> 2) + 1
    } else {
        0
    }
}

/// The strength a cast's victims save against, and the percent its rolls are
/// raised by (retail `CastSpellFromSource`).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Strength {
    pub power: u32,
    pub against: u32,
    pub boost: u32,
}

impl Strength {
    /// A character's cast: by their total caster level, raised by Power Cast.
    pub fn of(character: &Character, spell: &SpellDefinition, power: u32) -> Self {
        let caster = total_caster_level(character, spell.books).max(0) as u32;
        Strength::at(caster, spell, power, power_cast(character))
    }

    /// Retail `GetSpellDifficulty`: `figure/2 + power + (cost/2 + level)/2`,
    /// three less against all enemies, less the death spells' floors, then
    /// raised by half the boost.
    pub fn at(figure: u32, spell: &SpellDefinition, power: u32, boost: u32) -> Self {
        let mut against = figure / 2 + power + (spell.cost / 2 + spell.level) / 2;
        let penalty =
            spell.facts.death_floor + if spell.targeting == Targeting::AllEnemies { 3 } else { 0 };
        against = against.max(penalty) - penalty;
        against += against * (boost / 2) / 100;
        Strength { power, against, boost }
    }
}

/// Whether a cast came off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Cast,
    Fizzled,
    /// It turned on the caster.
    Backfired,
}

/// Roll a cast against its failure chance (retail `CastSpellFromSource`):
/// the same roll decides failure and backfire. Ordinary failure chances below
/// 6% cannot backfire; quiet (thrown) uses backfire whenever they fail and can.
/// Underwater Fire first overrides failure to 100%, then quiet halves it;
/// either case forbids backfire and keeps the ordinary payment transaction.
pub fn roll(
    rng: &mut GameRng,
    chance: u32,
    spell: &SpellDefinition,
    quiet: bool,
    underwater: bool,
) -> Outcome {
    let suppressed = underwater && spell.realm == Some(Realm::Fire);
    let chance = if suppressed { if quiet { 50 } else { 100 } } else { chance };
    if chance == 0 {
        return Outcome::Cast;
    }
    let rolled = rng.below(100);
    if rolled >= chance {
        return Outcome::Cast;
    }
    let backfire = if suppressed || !spell.can_backfire() || (!quiet && chance < 6) {
        0
    } else if quiet {
        chance
    } else {
        chance / 3
    };
    if rolled < backfire { Outcome::Backfired } else { Outcome::Fizzled }
}

/// Retail fatigue for a learned cast: the spell's level plus its power.
fn cast_fatigue(spell: &SpellDefinition, power: u32) -> u32 {
    spell.level + power
}

pub struct MagicPlugin;

impl Plugin for MagicPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<MagicRequest>()
            .add_message::<casting::MagicNews>()
            .add_observer(casting::cast)
            .add_observer(casting::refuse)
            .add_observer(utilities::identify)
            .add_observer(dialogue::cast)
            .add_observer(portals::cast)
            .add_observer(pointcasts::cast)
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

/// Magic that is not a deed on bodies, accepted: a utility spell (portals,
/// dialogue, point casts, Identify) or a cast at a device. The observer for
/// its kind carries it out; the others let it be.
#[derive(EntityEvent, Clone, Copy, Debug, PartialEq)]
pub struct Utility {
    #[event_target]
    pub caster: Entity,
    pub spell: SpellId,
    pub choice: casting::MagicChoice,
}

/// Accept magic requests (retail `ChooseAction`): in combat only record the
/// choice; outside it, cast now.
pub fn accept_magic(
    mut requests: MessageReader<MagicRequest>,
    mut commands: Commands,
    battle: Option<Single<&Battle>>,
    camp: Option<Res<crate::game::camp::Camp>>,
    interaction: Query<(), With<crate::game::world::interaction::DeviceInteraction>>,
    mut orders: Query<&mut crate::game::party::Order>,
    held: Items,
    content: Res<Catalogs>,
) {
    for &MagicRequest { caster, choice } in requests.read() {
        let spell = choice.source.spell_id(&held, &content.items);
        // A point cast in a fight waits for its turn, as any other order.
        let utility = (spell.is_some_and(utilities::spell)
            && (battle.is_none() || !spell.is_some_and(pointcasts::spell)))
            || matches!(choice.aim, casting::Aim::Device(_));
        if let (true, Some(spell)) = (utility, spell) {
            commands.trigger(Utility { caster, spell, choice });
            continue;
        }
        let refusal = match spell {
            None => Some(casting::Refusal::NoItem),
            Some(_) if !interaction.is_empty() => Some(casting::Refusal::WrongTime),
            Some(_) => None,
        };
        if let Some(refusal) = refusal {
            commands.trigger(casting::Refused { caster, refusal });
            continue;
        }
        if battle.is_some() {
            orders.get_mut(caster).unwrap().set_if_neq(crate::game::party::Order::Magic(choice));
            continue;
        }
        // Outside a fight no monster is a target: the hostile spells are
        // combat spells.
        let context =
            if camp.is_some() { casting::UseContext::Camp } else { casting::UseContext::World };
        commands.trigger(casting::Cast { caster, spell: spell.unwrap(), context, choice });
    }
}

#[cfg(test)]
fn test_maximum(world: &World, member: Entity) -> EnumMap<Realm, u32> {
    let profile = world.get::<Character>(member).unwrap();
    let learned = world.get::<LearnedSpells>(member).unwrap();
    let spells = &world.resource::<Catalogs>().spells;
    let attributes =
        world.get::<Attributes>(member).copied().unwrap_or(Attributes::of_sheet(profile));
    profile.max_points(&attributes, learned, spells)
}

#[cfg(test)]
pub(crate) fn test_set_points(world: &mut World, member: Entity, realm: Realm, current: i32) {
    let maximum = test_maximum(world, member);
    world.get_mut::<SpellPoints>(member).unwrap().spent[realm] = maximum[realm] as i32 - current;
}

#[cfg(test)]
pub(crate) fn test_points(world: &World, member: Entity, realm: Realm) -> i32 {
    world.get::<SpellPoints>(member).unwrap().current(realm, &test_maximum(world, member))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::content::items::Dice;
    use crate::content::items::ItemCatalog;
    use crate::content::species::MonsterCatalog;
    use crate::content::spells::{PowerChoice, Reach, Usable};
    use crate::game::characters::{Gender, Race};

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

    /// The out-of-combat use path with real requests: a potion stack and a
    /// charged wand, each casting its own spell at its own power.
    #[test]
    fn underwater_fire_cannot_backfire_even_for_a_quiet_hostile_spell() {
        let spell = SpellDefinition {
            id: SpellId::ENERGY_BLAST,
            targeting: Targeting::Enemy,
            realm: Some(Realm::Fire),
            ..default()
        };
        assert!(spell.can_backfire());
        let mut rng = GameRng::seeded(1);
        for _ in 0..20 {
            assert_eq!(roll(&mut rng, 0, &spell, false, true), Outcome::Fizzled);
            assert_ne!(roll(&mut rng, 100, &spell, true, true), Outcome::Backfired);
        }
    }

    /// A chance past a hundred counts for a third of itself in the chance to
    /// backfire: 300 is 100, so every failure backfires (held to 100 first,
    /// it was 33).
    #[test]
    fn a_failure_chance_past_a_hundred_always_backfires() {
        let spell = SpellDefinition {
            id: SpellId::ENERGY_BLAST,
            targeting: Targeting::Enemy,
            realm: Some(Realm::Fire),
            ..default()
        };
        let mut rng = GameRng::seeded(2);
        for _ in 0..100 {
            assert_eq!(roll(&mut rng, 300, &spell, false, false), Outcome::Backfired);
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
        use casting::{Aim, MagicChoice, MagicNews, Refusal, Source};
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_resource::<crate::game::factions::Factions>()
            .init_resource::<crate::game::time::GameClock>()
            .add_message::<crate::game::factions::FactionNews>()
            .init_resource::<crate::game::difficulty::Difficulty>()
            .init_resource::<avian3d::collider_tree::ColliderTrees>()
            .insert_resource(GameRng::seeded(3))
            .add_message::<MagicRequest>()
            .add_message::<casting::MagicNews>()
            .add_message::<crate::game::combat::CombatNews>()
            .add_plugins((crate::game::hits::HitsPlugin, crate::game::items::plugin))
            .add_observer(casting::cast)
            .add_observer(casting::refuse)
            .add_systems(Update, accept_magic);
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
        let profile = Character::new("Drinker", Race::Human, Gender::Male, Profession::Fighter);
        let mut vitals = Vitals::new(&profile);
        vitals.health.current = 1;
        let member = world.spawn((profile, vitals)).id();
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
            news.extend(app.world_mut().resource_mut::<Messages<MagicNews>>().drain());
            news.last().cloned()
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

    #[test]
    fn a_priest_picks_two_first_level_divine_spells_and_gains_their_points() {
        let priest = Character::new("Cleric", Race::Human, Gender::Male, Profession::Priest);
        let fighter = Character::new("Brute", Race::Human, Gender::Male, Profession::Fighter);
        let heal = heal_wounds();
        assert_eq!((starting_picks(priest.profession), starting_picks(fighter.profession)), (2, 0));
        assert!(can_learn(&priest, &heal) && !can_learn(&fighter, &heal));
        let higher = SpellDefinition { level: 2, ..heal_wounds() };
        assert!(!can_learn(&priest, &higher), "level 1 casters learn level 1 spells only");
        let catalog =
            SpellCatalog((0..=6).map(|i| if i == 6 { heal_wounds() } else { default() }).collect());
        let learned = LearnedSpells([SpellId::HEAL_WOUNDS].into());
        let points = priest.max_points(&Attributes::of_sheet(&priest), &learned, &catalog);
        assert!(points[Realm::Divine] >= 3, "never under the dearest learned spell");
        assert_eq!(points[Realm::Fire], 0, "no spell, no points");
    }

    #[test]
    fn mana_restoration_levels_deficits_and_wastes_excess_without_resetting_recovery() {
        let mut maximum = EnumMap::default();
        maximum[Realm::Fire] = 10;
        maximum[Realm::Water] = 5;
        maximum[Realm::Air] = 3;
        let mut points = SpellPoints { spent: maximum.map(|_, max| max as i32), ..default() };
        points.fraction[Realm::Fire] = 0.75;
        assert_eq!(points.restore_evenly(8), 8);
        // Five go to Fire; then the equal deficits share, in realm order.
        assert_eq!(
            (points.current(Realm::Fire, &maximum), points.current(Realm::Water, &maximum)),
            (7, 1)
        );
        assert_eq!(points.current(Realm::Air, &maximum), 0);
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
        use casting::{Aim, MagicChoice, MagicNews, Source};

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
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, AssetPlugin::default()))
                .init_resource::<crate::game::factions::Factions>()
                .init_resource::<crate::game::time::GameClock>()
                .add_message::<crate::game::factions::FactionNews>()
                .init_resource::<crate::game::difficulty::Difficulty>()
                .init_resource::<avian3d::collider_tree::ColliderTrees>()
                .insert_resource(GameRng::seeded(3))
                .add_message::<MagicRequest>()
                .add_message::<MagicNews>()
                .add_message::<crate::game::combat::CombatNews>()
                .add_plugins((crate::game::hits::HitsPlugin, crate::game::items::plugin))
                .add_observer(casting::cast)
                .add_observer(casting::refuse)
                .add_observer(crate::game::actors::spells::age)
                .add_systems(Update, accept_magic);
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
            let mut user = Character::new("User", Race::Human, Gender::Male, Profession::Priest);
            user.skill_points[Skill::ThrowingAndSling] = 100;
            let user = world.spawn((Vitals::new(&user), user)).id();
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
            let ally = Character::new("Ally", Race::Human, Gender::Male, Profession::Priest);
            let mut vitals = Vitals::new(&ally);
            vitals.health.current = 1;
            vitals.stamina.current = 1;
            vitals.afflict(Condition::Asleep, Lasting::Rounds(8));
            vitals.afflict(Condition::Unconscious, Lasting::Rounds(8));
            vitals.poison(Lasting::Rounds(20), 10);
            let ally = world
                .spawn((
                    ally,
                    vitals,
                    LearnedSpells([SpellId::HEAL_WOUNDS].into()),
                    SpellPoints::default(),
                ))
                .id();
            test_set_points(world, ally, Realm::Divine, 0);
            crate::game::party::roster::test_party(world, &[user, ally]);
            world.spawn(crate::game::world::level::Level::test(default()));
            world.spawn((
                crate::game::party::body::PartyBody::default(),
                avian3d::prelude::Position::default(),
            ));
            let user_before = world.get::<Vitals>(user).unwrap().clone();
            world.write_message(MagicRequest {
                caster: user,
                choice: MagicChoice { source: Source::Item(stack_id), aim: Aim::Body(ally) },
            });
            app.update();
            let world = app.world_mut();
            let news: Vec<_> = world.resource_mut::<Messages<MagicNews>>().drain().collect();
            assert!(matches!(news.as_slice(), [MagicNews::Done(_)]), "{id:?}: {news:?}");
            assert_eq!(world.get::<Count>(stack_id).unwrap().of, 1);
            let user_after = world.get::<Vitals>(user).unwrap();
            assert_eq!(user_after.health.current, user_before.health.current);
            assert_eq!(user_after.conditions, user_before.conditions);
            let fatigue = user_before.stamina.current - user_after.stamina.current;
            if id == SpellId::SMELLING_SALTS {
                assert_eq!(fatigue, 10);
            } else {
                assert!((1..=2).contains(&fatigue), "using an item still pays its normal fatigue");
            }
            let vitals = world.get::<Vitals>(ally).unwrap();
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
                    assert_eq!(vitals.hurt(through), 0);
                    assert_eq!(vitals.stamina.current, stamina, "absorbed damage does not fatigue");
                    let through = absorb(world, 2);
                    let mut vitals = world.get_mut::<Vitals>(ally).unwrap();
                    assert_eq!(vitals.hurt(through), 1);
                    assert!(vitals.is_dead());
                }
                SpellId::ARMORPLATE => {
                    use bevy::ecs::system::RunSystemOnce;
                    let party = crate::game::party::roster::the_party(world);
                    let (on_ally, on_party) = world
                        .run_system_once(move |laid: crate::game::actors::Laid| {
                            (laid.listed(ally).count(), laid.bonus(party).armour)
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
                    assert!(
                        !vitals.conditions.has(Condition::Asleep)
                            && !vitals.conditions.has(Condition::Unconscious)
                    )
                }
                // Power four takes sixteen rounds off, and the poison weakens
                // by the same share, as aging would (retail
                // `TickCharacterCondition`).
                SpellId::CURE_POISON => {
                    assert_eq!(
                        vitals.conditions.lasting(Condition::Poisoned),
                        Some(Lasting::Rounds(4))
                    );
                    assert_eq!(vitals.poison, 2);
                }
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn lowered_capacity_keeps_debt_until_the_capacity_returns() {
        let mut high = EnumMap::default();
        high[Realm::Divine] = 20;
        let mut low = high;
        low[Realm::Divine] = 10;
        let mut points = SpellPoints::default();
        points.spend(Realm::Divine, 19);
        points.fraction[Realm::Divine] = 0.5;
        assert_eq!(points.current(Realm::Divine, &low), -9);
        assert_eq!(points.available(Realm::Divine, &low), 0);
        assert_eq!(points.drain_realm(Realm::Divine, 5, &low), 0);
        let spell = SpellDefinition { realm: Some(Realm::Divine), cost: 1, ..default() };
        assert_eq!(points.affordable_power(&spell, &low), 0);
        assert_eq!(points.current(Realm::Divine, &high), 1);
    }

    #[test]
    fn points_recover_by_retail_rate_and_never_beyond_the_maximum() {
        let mut maximum = EnumMap::default();
        maximum[Realm::Divine] = 20;
        let mut points = SpellPoints { spent: maximum.map(|_, max| max as i32), ..default() };
        // (0.65·20 + 20)/240 = 0.1375 a tick at half rate: 0.06875.
        points.recover(100, recovery_scale(0.0), &maximum);
        assert_eq!(points.current(Realm::Divine, &maximum), 6);
        points.recover(10_000, 1.0, &maximum);
        assert_eq!(points.current(Realm::Divine, &maximum), 20);
        points.spent[Realm::Divine] = -5;
        points.fraction[Realm::Divine] = 0.5;
        points.recover(1, 1.0, &maximum);
        assert_eq!(points.current(Realm::Divine, &maximum), 25, "retail skips full realms");
    }

    #[test]
    fn idle_recovery_keeps_surplus() {
        let mut maximum = EnumMap::default();
        maximum[Realm::Divine] = 10;
        let mut points = SpellPoints::default();
        points.spent[Realm::Divine] = -5;
        points.fraction[Realm::Divine] = 0.5;
        points.recover(1, 0.5, &maximum);
        assert_eq!(points.current(Realm::Divine, &maximum), 15);
        assert!(!points.needs_recovery());
        points.spend(Realm::Divine, 6);
        points.recover(100, 0.5, &maximum);
        assert_eq!(points.current(Realm::Divine, &maximum), 10);
    }

    #[test]
    fn affordable_power_is_points_over_cost_within_seven() {
        let mut points = SpellPoints::default();
        let mut maximum = EnumMap::default();
        maximum[Realm::Divine] = 100;
        points.spent[Realm::Divine] = 90;
        assert_eq!(points.affordable_power(&heal_wounds(), &maximum), 3);
        points.spent[Realm::Divine] = 0;
        assert_eq!(points.affordable_power(&heal_wounds(), &maximum), 7);
    }

    #[test]
    fn healing_cannot_backfire_and_an_untrained_caster_fails_more_at_higher_power() {
        let priest = Character::new("Cleric", Race::Human, Gender::Male, Profession::Priest);
        let heal = heal_wounds();
        assert!(fail_chance(&priest, &heal, 7) > fail_chance(&priest, &heal, 1));
        let mut rng = GameRng::seeded(1);
        for _ in 0..200 {
            assert_ne!(roll(&mut rng, 100, &heal, false, false), Outcome::Backfired);
        }
    }
}
