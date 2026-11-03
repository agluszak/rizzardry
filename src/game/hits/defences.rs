//! What a body is to the effects that land on it: the level its saves and
//! resistance weigh, its senses, its resistance in each realm and how it
//! casts. The same components on a member and a monster, judged by one
//! system from each body's own sources (a member's sheet, gear and party; a
//! monster's species and spells) at the start of each pass of the hit
//! stages; the save, resistance and waking rules read them without asking
//! which it is. The rules themselves follow.

use bevy::prelude::*;
use enum_map::EnumMap;

use crate::content::characters::Realm;
use crate::content::species::MonsterKind;
use crate::content::spells::Spellbook;
use crate::game::characters::resistance::{member_resistances, resistances};
use crate::game::characters::{Character, Condition, Conditions, Lasting, Vitals};
use crate::game::data::Catalogs;
use crate::game::inventory::Carried;
use crate::game::magic::Strength;
use crate::game::magic::enchantments::{Enchantments, PartyEffects};
use crate::game::party::roster::PartyMember;
use crate::game::rng::GameRng;
use crate::game::world::population::{self, Monster};

/// The level a body's saves and resistance weigh against an effect's
/// strength: a member's, a monster's species'.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SaveLevel(pub u32);

/// How alert a body is, which a blow may wake it by: a member's Senses, a
/// monster's species'.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Senses(pub u32);

/// A body's resistance in each realm: a member's from their stats, what
/// they wear and, in the party, its spells; a monster's from its species
/// and spells.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Resistances(pub EnumMap<Realm, i32>);

/// A monster's kind (undead, demon, plant, …), which holy and slaying
/// things single out.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kind(pub MonsterKind);

/// What a body shrugs off before any save: the conditions its kind cannot
/// suffer, and whether it cannot be slain outright (retail
/// `g_condition_immunities`, `death_immune`). A member's own immunities
/// are traits, weighed after the save.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Immunities {
    pub conditions: &'static [Condition],
    pub death: bool,
}

/// How strongly a body casts from each book, which a spell turned back on
/// it or from it grows from: a member's caster level in the book, a
/// monster's species' level.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CasterLevels(pub EnumMap<Spellbook, u32>);

impl CasterLevels {
    /// What a spell's strength from the body grows from: its highest level
    /// among the spell's books.
    pub fn figure(&self, spell: &crate::content::spells::SpellDefinition) -> u32 {
        spell.books.iter().map(|book| self.0[book]).max().unwrap_or(0)
    }
}

/// A character's defences, by their sheet and gear, and a member's by the
/// party's spells too.
fn character_defences(
    (character, carried): (&Character, &Carried),
    vitals: &Vitals,
    member: bool,
    party: &Enchantments,
    items: &crate::content::items::ItemCatalog,
) -> impl Bundle {
    let stats = character.stats(&vitals.enchantments);
    let resistances = match member {
        true => member_resistances(&stats, carried, items, party),
        false => resistances(&stats, carried, items),
    };
    (
        SaveLevel(u32::from(character.level())),
        Senses(stats.attribute(crate::content::characters::Attribute::Senses)),
        Resistances(EnumMap::from_fn(|realm| resistances[realm].total)),
        CasterLevels(EnumMap::from_fn(|book| {
            crate::game::magic::total_caster_level(character, enumset::EnumSet::only(book)).max(0)
                as u32
        })),
        Immunities::default(),
    )
}

/// A monster's defences, by its species and spells.
fn species_defences(kind: &crate::content::species::Species, vitals: &Vitals) -> impl Bundle {
    (
        SaveLevel(u32::from(kind.level)),
        Senses(u32::from(kind.attributes.senses)),
        Resistances(EnumMap::from_fn(|realm| population::resistance(kind, vitals, realm))),
        CasterLevels(EnumMap::from_fn(|_| u32::from(kind.level))),
        Immunities { conditions: immune(kind.kind), death: kind.death_immune },
        Kind(kind.kind),
    )
}

/// Every body's defences, from its own sources.
#[allow(clippy::type_complexity)]
pub fn judge_defences(
    mut commands: Commands,
    bodies: Query<(Entity, &Vitals, AnyOf<((&Character, &Carried), &Monster)>, Has<PartyMember>)>,
    party: Single<&PartyEffects>,
    content: If<Res<Catalogs>>,
) {
    for (entity, vitals, (character, monster), member) in &bodies {
        match (character, monster) {
            (Some(character), _) => {
                let defences =
                    character_defences(character, vitals, member, &party.0, &content.items);
                commands.entity(entity).insert(defences);
            }
            (None, Some(monster)) => {
                let defences = species_defences(&content.species[monster.species], vitals);
                commands.entity(entity).insert(defences);
            }
            (None, None) => {}
        }
    }
}

/// Retail's marker for a condition without end (`W8_CONDITION_INDEFINITE`),
/// which resistance cuts like any other duration.
pub const INDEFINITE: u32 = 9999;

/// Each condition's base save (retail `g_condition_resist_base`), by retail
/// number (Drained is 1, Dead 18).
pub const CONDITION_SAVE: [i32; 20] =
    [0, 1, 2, 3, 4, 4, 5, 6, 7, 8, 9, 10, 12, 14, 16, 20, 18, 22, 24, 22];
pub const DEATH_SAVE: i32 = CONDITION_SAVE[18];

/// A condition's base save.
pub fn condition_save(condition: Condition) -> i32 {
    CONDITION_SAVE[condition as usize + 1]
}

/// The conditions a monster of `kind` shrugs off (retail
/// `g_condition_immunities`).
pub fn immune(kind: MonsterKind) -> &'static [Condition] {
    use Condition::*;
    match kind {
        MonsterKind::Undead => {
            &[Diseased, Poisoned, Unconscious, Irritated, Afraid, Insane, Asleep, Nauseated]
        }
        MonsterKind::Android => &[Diseased, Poisoned, Nauseated, Irritated, Afraid, Asleep],
        MonsterKind::Plant => &[Insane, Afraid, Nauseated],
        _ => &[],
    }
}

/// `(level − strength)·3 + base + resistance`, the chance a victim saves.
pub fn condition_chance(level: u32, resistance: i32, against: u32, base: i32) -> i32 {
    (level as i32 - against as i32) * 3 + base + resistance
}

/// A saving throw at `chance` percent, held within 5..95 (retail
/// `TargetResistsCondition`).
pub fn save_roll(rng: &mut GameRng, chance: i32) -> bool {
    (rng.below(100) as i32) < chance.clamp(5, 95)
}

/// Retail `ReduceMagnitudeByResistance`: resistance and the victim's level
/// over the attack's strength take a random share off.
pub fn resist(rng: &mut GameRng, amount: u32, resistance: i32, level: u32, against: u32) -> u32 {
    let mut c = resistance + (level as i32 - against as i32) * 3;
    if c <= 0 {
        return amount;
    }
    c += 1;
    let half = c / 2;
    let share =
        (rng.below((c - half) as u32) as i32 + half + rng.below(half as u32) as i32).min(100);
    amount - amount * share as u32 / 100
}

/// The rounds a cure takes off a timed condition: `power²`, boosted.
pub fn cure_rounds(strength: Strength) -> u32 {
    let per = strength.power.pow(2);
    per + per * strength.boost / 100
}

/// Retail `TryCureConditionOnTargets`: a timed condition loses `power²`
/// rounds (and goes if that is all of them); a lasting one goes with chance
/// `power·15`%. A forced cure lifts them all.
pub fn cure(
    rng: &mut GameRng,
    conditions: &mut Conditions,
    cured: &[Condition],
    strength: Strength,
    forced: bool,
) -> Vec<Condition> {
    let per = cure_rounds(strength);
    let mut lifted = Vec::new();
    for &condition in cured {
        let Some(lasting) = conditions.lasting_mut(condition) else { continue };
        let gone = match lasting {
            _ if forced => true,
            Lasting::Rounds(left) if *left <= per => true,
            Lasting::Rounds(left) => {
                *left -= per;
                false
            }
            Lasting::Indefinite => rng.below(100) < strength.power * 15,
        };
        if gone {
            conditions.lift(condition);
            lifted.push(condition);
        }
    }
    lifted
}
