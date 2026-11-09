//! What a body is to the effects that land on it: the level its saves and
//! resistance weigh, its senses, its resistance in each realm and how it
//! casts. The same component on a member and a monster, judged by one
//! system from each body's own sources (a member's sheet, gear and party; a
//! monster's species and spells) whenever those change; the save,
//! resistance and waking rules read it without asking which it is. The
//! rules themselves follow.

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

/// What a body is to the effects that land on it, a member's or a
/// monster's alike.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Defences {
    /// The level its saves and resistance weigh against an effect's
    /// strength: a member's, a monster's species'.
    pub level: u32,
    /// How alert it is, which a blow may wake it by.
    pub senses: u32,
    /// Its resistance in each realm: a member's from their stats, what
    /// they wear and, in the party, its spells; a monster's from its
    /// species and spells.
    pub resistances: EnumMap<Realm, i32>,
    /// How strongly it casts from each book, which a spell turned back on
    /// it or from it grows from: a member's caster level in the book, a
    /// monster's species' level.
    pub caster: EnumMap<Spellbook, u32>,
    /// A monster's kind (undead, demon, plant, …), which holy and slaying
    /// things single out.
    pub kind: Option<MonsterKind>,
    /// The conditions its kind cannot suffer, before any save (retail
    /// `g_condition_immunities`). A member's own immunities are traits,
    /// weighed after the save.
    pub immune: &'static [Condition],
    /// It cannot be slain outright (retail `death_immune`).
    pub death_immune: bool,
}

impl Defences {
    /// What a spell's strength from the body grows from: its highest level
    /// among the spell's books.
    pub fn caster_level(&self, spell: &crate::content::spells::SpellDefinition) -> u32 {
        spell.books.iter().map(|book| self.caster[book]).max().unwrap_or(0)
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
) -> Defences {
    let stats = character.stats(&vitals.enchantments);
    let resistances = match member {
        true => member_resistances(&stats, carried, items, party),
        false => resistances(&stats, carried, items),
    };
    Defences {
        level: u32::from(character.level()),
        senses: stats.attribute(crate::content::characters::Attribute::Senses),
        resistances: EnumMap::from_fn(|realm| resistances[realm].total),
        caster: EnumMap::from_fn(|book| {
            crate::game::magic::total_caster_level(character, enumset::EnumSet::only(book)).max(0)
                as u32
        }),
        ..default()
    }
}

/// A monster's defences, by its species and spells.
fn species_defences(kind: &crate::content::species::Species, vitals: &Vitals) -> Defences {
    Defences {
        level: u32::from(kind.level),
        senses: u32::from(kind.attributes.senses),
        resistances: EnumMap::from_fn(|realm| population::resistance(kind, vitals, realm)),
        caster: EnumMap::from_fn(|_| u32::from(kind.level)),
        kind: Some(kind.kind),
        immune: immune(kind.kind),
        death_immune: kind.death_immune,
    }
}

/// Keep every body's defences to their sources: its sheet, gear and spells,
/// and the party's spells for a member. Leaving the party is a change too:
/// a former member stops borrowing the party's resistance.
#[allow(clippy::type_complexity)]
pub fn judge_defences(
    mut bodies: Query<(
        Entity,
        &mut Defences,
        Ref<Vitals>,
        AnyOf<((Ref<Character>, Ref<Carried>), Ref<Monster>)>,
        Option<Ref<PartyMember>>,
    )>,
    party: Single<Ref<PartyEffects>>,
    content: If<Res<Catalogs>>,
    mut departed: RemovedComponents<PartyMember>,
) {
    let departed: bevy::ecs::entity::EntityHashSet = departed.read().collect();
    for (entity, mut defences, vitals, (character, monster), member) in &mut bodies {
        let changed = defences.is_added()
            || vitals.is_changed()
            || content.is_changed()
            || member.as_ref().is_some_and(|m| m.is_changed() || party.is_changed())
            || departed.contains(&entity);
        let judged = match (character, monster) {
            (Some((character, carried)), _)
                if changed || character.is_changed() || carried.is_changed() =>
            {
                character_defences(
                    (&character, &carried),
                    &vitals,
                    member.is_some(),
                    &party.0,
                    &content.items,
                )
            }
            (None, Some(monster)) if changed || monster.is_changed() => {
                species_defences(&content.species[monster.species], &vitals)
            }
            _ => continue,
        };
        defences.set_if_neq(judged);
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
