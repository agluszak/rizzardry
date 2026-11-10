//! Casting a learned spell, using an item's spell, or a monster's innate
//! casting (retail `ExecuteCharacterSpellCast`, `UseItem`, `CastItemSpell`,
//! `CastSpellFromSource`, `MonsterCastsSpell`). An accepted cast is an
//! entity ([`Cast`]) made [`By`] its caster; [`resolve_casts`] aims it and
//! judges it without spending, then pays, rolls and releases it as the act
//! that lands ([`crate::game::hits`]): a fizzle, a backfire, or its victims
//! and missiles.
//!
//! Choosing never spends anything. A refused cast spends nothing; one that
//! is rolled pays even when it fizzles or backfires: spell points for a
//! learned spell, one use for an item, nothing for an innate cast. Its act
//! keeps its own copy of the spell and victims, so using the last potion
//! still heals, and a missile in flight still lands after its caster falls.

use bevy::prelude::*;

use super::{Outcome, Strength};
use crate::content::characters::Skill;
use crate::content::items::{ItemCatalog, ItemDefinition, UseType};
use crate::content::spells::{
    MAX_POWER, PowerChoice, SpellDefinition, SpellId, Targeting, Usable, min_caster_level,
};
use crate::game::actors::{Place, Places};
use crate::game::characters::{Character, Condition, Lasting, Pool, Practise, Profession, Vitals};
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::hits::definitions::Definitions;
use crate::game::hits::effects::{Body, BodyItem, BodyReadOnly, BodyReadOnlyItem, Strike};
use crate::game::hits::reach::{Recipients, within};
use crate::game::hits::spells::{Leech, Spell, group_of, leader, owners_strength, targets};
use crate::game::hits::{Act, Deed, Effect, Release};
use crate::game::items::{Items, Spend};
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::hostility::{self, Provocation, Stance};
use crate::game::world::identify::{GroupMember, GroupMembers};
use crate::game::world::level::Level;
use crate::game::world::obstacles::WorldObstacles;

/// The power that asks the caster to choose it: as much as the victim needs
/// (retail "Auto Calibrate"), or, outside a fight, as much and as often as
/// it takes (retail "Auto Recast").
pub const AUTO_POWER: u32 = MAX_POWER + 1;

/// How fast spell missiles fly: retail's `energy_blast.mls` velocity of 30,
/// advanced per two-millisecond step (`Missile.cpp`), comes to 15 m/s.
pub(crate) const MISSILE_SPEED: f32 = 15.0;

/// What pays for a cast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// A learned spell at the chosen power, or [`AUTO_POWER`]; it is
    /// clamped to what the caster can afford when it runs.
    Spell { spell: SpellId, power: u32 },
    /// An item the actor carries or the party's pool holds, identified, not
    /// reserved.
    Item(Entity),
    /// A monster's own casting, which costs no points or uses but tires it:
    /// its spell budget stands in for a caster's skill, its level sets the
    /// strength.
    Innate { spell: SpellId, power: u32, budget: u32, level: u8 },
}

impl Source {
    /// Resolve an item source by its stable stack identity. This is routing,
    /// not acceptance: the cast revalidates eligibility and payment.
    pub fn spell_id(self, held: &Items, catalog: &ItemCatalog) -> Option<SpellId> {
        match self {
            Self::Spell { spell, .. } | Self::Innate { spell, .. } => Some(spell),
            Self::Item(item) => catalog[held.id(item)?].spell.map(|s| s.spell),
        }
    }
}

/// Whom the caster aimed at; group, area, party and self spells widen or
/// replace it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Aim {
    Point(Vec3),
    /// A body, a member's or a monster's.
    Body(Entity),
    /// Whoever stands nearest the caster's heading within the spell's
    /// reach when it is cast (retail's automatic targeting), the nearer of
    /// equals.
    Ahead,
    /// The device attempt under way (its lock or trap), by the attempt.
    Device(Entity),
    /// The selected item, retained across any delayed request.
    Item(Entity),
}

impl Aim {
    /// The body aimed at, if one was.
    pub fn body(self) -> Option<Entity> {
        match self {
            Aim::Body(entity) => Some(entity),
            _ => None,
        }
    }
}

/// An accepted magic action: the source and the aim, nothing resolved yet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MagicChoice {
    pub source: Source,
    pub aim: Aim,
}

/// Why nothing was spent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    CannotAct,
    Silenced,
    NotLearned,
    NoSpellPoints,
    NoItem,
    NoCharges,
    CannotUse,
    /// Its effect is not implemented yet.
    Unsupported,
    /// Not now: only in combat, or only out of it.
    WrongTime,
    NoTarget,
    /// An instrument or gadget that could not be brought to play: its
    /// fatigue is paid, the item is not (retail string 0x1f3). The attempt
    /// is practice.
    PlayedBadly,
}

/// What became of a magic action, for whoever tells the player.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum MagicNews {
    Done(Report),
    /// An indirect trap source has no character caster or payment.
    Triggered {
        spell: SpellId,
        effects: Vec<Effect>,
    },
    /// A missile reached its victims.
    Landed {
        caster: Entity,
        spell: SpellId,
        effects: Vec<Effect>,
    },
    Refused {
        caster: Entity,
        refusal: Refusal,
    },
}

/// A cast was refused, by whoever checked it: the caster is told
/// ([`MagicNews::Refused`]), and a member who was ordered to cast defends
/// instead.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Refused {
    #[event_target]
    pub caster: Entity,
    pub refusal: Refusal,
}

/// The caster is told their cast was refused.
pub fn refuse(refused: On<Refused>, mut news: MessageWriter<MagicNews>) {
    news.write(MagicNews::Refused { caster: refused.caster, refusal: refused.refusal });
}

/// What a cast did.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub caster: Entity,
    pub spell: SpellId,
    pub power: u32,
    /// The item it came from, already spent.
    pub item: Option<String>,
    pub outcome: Outcome,
    /// What landed at once; missiles land later as [`MagicNews::Landed`].
    pub effects: Vec<Effect>,
}

/// A cast's practice, owed once its deed has landed: its caster practises
/// these skills if the cast affected anyone (retail `SpellAffectedTarget`),
/// or whatever came of it once a missile is on its way.
#[derive(Clone, Debug)]
pub struct Practice {
    pub caster: Entity,
    pub skills: Vec<(Skill, u32)>,
    pub sure: bool,
}

/// An auto recast (retail's "repeat" power): once it has landed, its caster
/// casts again while its victims are not whole and stamina and points last.
#[derive(Clone, Debug)]
pub struct Recast {
    pub caster: Entity,
    pub choice: MagicChoice,
    pub victims: Vec<Entity>,
}

impl Recast {
    /// Whether to cast again: a victim is not yet whole in what the spell
    /// restores, and the caster's stamina and points last.
    pub fn wanted(&self, bodies: &Query<BodyReadOnly>, content: &Catalogs) -> bool {
        let Source::Spell { spell, .. } = self.choice.source else { return false };
        let spell = &content.spells[spell];
        let restored = self.victims.iter().all(|&victim| {
            let Ok(body) = bodies.get(victim) else { return true };
            let pool = match spell.id {
                SpellId::STAMINA | SpellId::REST_ALL => body.vitals.stamina,
                _ => body.vitals.health,
            };
            body.vitals.is_dead() || pool.current >= pool.max
        });
        !restored && bodies.get(self.caster).is_ok_and(|caster| can_repeat(&caster, spell, content))
    }
}

/// Whether an auto cast's caster can go again: stamina past the spell's
/// level and points for it at power 1.
fn can_repeat(caster: &BodyReadOnlyItem, spell: &SpellDefinition, content: &Catalogs) -> bool {
    let (Some(character), Some((learned, pool))) = (caster.character, caster.spells) else {
        return false;
    };
    let maximum = character.max_points(caster.attributes, learned, &content.spells);
    caster.vitals.stamina.current > spell.level
        && pool.available(spell.realm.unwrap(), &maximum) >= spell.cost
}

/// Mutually exclusive execution contexts, shared by menus and commitment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseContext {
    World,
    Combat,
    Camp,
    Conversation,
    Lock,
    Trap,
}

/// Whether the game can resolve a spell: its effects are defined, or it is
/// one of the device spells the device interaction resolves.
pub fn resolvable(spell: SpellId, definitions: &Definitions) -> bool {
    definitions.spell(spell).is_some()
        || super::utilities::spell(spell)
        || matches!(spell, SpellId::DIVINE_TRAP | SpellId::KNOCK_KNOCK)
}

pub fn usable_now(spell: &SpellDefinition, context: UseContext) -> bool {
    match context {
        UseContext::Conversation => super::dialogue::spell(spell.id),
        UseContext::Lock => spell.id == SpellId::KNOCK_KNOCK,
        UseContext::Trap => matches!(spell.id, SpellId::DIVINE_TRAP | SpellId::KNOCK_KNOCK),
        _ => match spell.usable {
            Usable::Always => true,
            Usable::InCombat => context == UseContext::Combat,
            Usable::OutOfCombat => context != UseContext::Combat,
            Usable::Camped => context == UseContext::Camp,
            Usable::OnLockOrTrap => false,
        },
    }
}

/// Whether `target` lies within a cone's arcs along `aim`, both directions
/// from its apex (retail `TargetInRangeAndArcs`, which tests heading and
/// elevation independently against a 45° half arc). Range is the caller's.
pub(crate) fn in_cone(aim: Vec3, target: Vec3) -> bool {
    let heading = |v: Vec3| v.x.atan2(v.z);
    let elevation = |v: Vec3| v.y.atan2(v.x.hypot(v.z));
    let within = |a: f32, b: f32| {
        let difference = a - b;
        difference.sin().atan2(difference.cos()).abs() < std::f32::consts::FRAC_PI_4
    };
    within(heading(aim), heading(target)) && within(elevation(aim), elevation(target))
}

/// Whether a body at `from` reaches one at `to` with a spell of `reach`:
/// within it, body to body, along a clear line between their eyes (retail
/// `CharacterActionReachesTarget`, `CanMonsterSeeMonster`'s line).
pub fn reaches(from: &Place, to: &Place, reach: f32, obstacles: &WorldObstacles) -> bool {
    from.gap(to) <= reach && obstacles.clear(from.eye, to.eye)
}

/// Potions and food: usable by anyone who holds them.
fn consumed_by_anyone(definition: &ItemDefinition) -> bool {
    matches!(definition.use_type, UseType::Potion | UseType::Food)
}

/// How likely using an item's spell fails, and whether it is a quiet
/// (thrown) use, every failure of which can backfire (retail
/// `GetItemUseDifficulty`, `CastItemSpell`, `CastSpellFromSource`).
fn item_failure(
    character: &Character,
    definition: &ItemDefinition,
    spell: &SpellDefinition,
    power: u32,
    identified: bool,
    (difficulty, moment): (Difficulty, u32),
) -> (u32, bool) {
    let Some(skill) = definition.use_type.skill() else { return (0, false) };
    let level = character.skill(skill);
    let mut failure = super::spell_failure(level, spell, power);
    match definition.use_type {
        UseType::Thrown => failure = failure * 8 / 10,
        UseType::Charged | UseType::Scroll => failure = failure * 9 / 10,
        UseType::Instrument | UseType::Gadget => {
            let shortfall = spell.level * 15;
            if level < shortfall {
                failure += (shortfall - level) / 3;
            }
            let profession = if definition.use_type == UseType::Instrument {
                Profession::Bard
            } else {
                Profession::Gadgeteer
            };
            let adjust = min_caster_level(spell.level)
                - i32::from(character.profession_levels[profession])
                - 1
                + power as i32;
            if adjust > 0 {
                failure += spell.level * adjust as u32;
            }
        }
        _ => {}
    }
    let mut failure = difficulty.scale_by_pace(failure, moment);
    if !identified {
        failure += 30;
    }
    let quiet = definition.use_type == UseType::Thrown;
    if quiet {
        failure /= 2;
    }
    (failure, quiet)
}

/// The strength an item's spell is cast at: by the user's level, or their
/// Bard or Gadgeteer levels for instruments and gadgets (retail
/// `CastItemSpell`).
fn item_strength(
    character: &Character,
    definition: &ItemDefinition,
    spell: &SpellDefinition,
    power: u32,
) -> Strength {
    let figure = match definition.use_type {
        UseType::Instrument => u32::from(character.profession_levels[Profession::Bard]),
        UseType::Gadget => u32::from(character.profession_levels[Profession::Gadgeteer]),
        _ => u32::from(character.level()),
    };
    Strength::at(figure, spell, power, 0)
}

/// The power an auto cast takes (retail `ChooseSpellPowerLevelForTarget`,
/// `Magic.cpp:1707-1880`), for the spells it knows and 1 for any other. A
/// member lacking health or stamina is healed by the lowest power whose
/// average roll covers it, stopping short of a failure chance over 10%
/// (Heal Wounds; Stamina; Restoration, which looks to stamina only once health
/// is full); a monster is healed at 1. A cure takes the cheapest power to
/// outlast the longest of the conditions it lifts: Cure Lesser Conditions
/// Nauseated, Afraid, Asleep and Blind; Cure Paralysis Paralyzed; Cure Poison
/// Poisoned; Cure Disease Diseased; Sane Mind Insane, Turncoat and Asleep;
/// Remove Curse Hexed. `failure` is the chance a cast at a power fails.
fn auto_power(
    spell: &SpellDefinition,
    victim: Option<(&Vitals, bool)>,
    affordable: u32,
    failure: &dyn Fn(u32) -> u32,
) -> u32 {
    let Some((vitals, member)) = victim else { return 1 };
    let cured: &[Condition] = match spell.id {
        SpellId::HEAL_WOUNDS | SpellId::STAMINA | SpellId::RESTORATION => &[],
        SpellId::CURE_LESSER_COND => {
            &[Condition::Nauseated, Condition::Afraid, Condition::Asleep, Condition::Blind]
        }
        SpellId::CURE_PARALYSIS => &[Condition::Paralyzed],
        SpellId::CURE_POISON => &[Condition::Poisoned],
        SpellId::CURE_DISEASE => &[Condition::Diseased],
        SpellId::SANE_MIND => &[Condition::Insane, Condition::Turncoat, Condition::Asleep],
        SpellId::REMOVE_CURSE => &[Condition::Hexed],
        _ => return 1,
    };
    let power = if cured.is_empty() {
        let lacks = |pool: Pool| pool.max.max(pool.current) - pool.current;
        let missing = match spell.id {
            SpellId::STAMINA => lacks(vitals.stamina),
            SpellId::RESTORATION if lacks(vitals.health) == 0 => lacks(vitals.stamina),
            _ => lacks(vitals.health),
        };
        // The average roll, cut down to a whole number.
        let average = |p: u32| {
            spell.dice.map_or(0, |d| (2 * d.base + d.count * p as i32 * (d.sides + 1)) / 2)
        };
        if !member || missing == 0 {
            1
        } else {
            (1..=MAX_POWER)
                .find_map(|p| {
                    if failure(p) > 10 {
                        Some((p - 1).max(1))
                    } else {
                        ((missing as i32) < average(p)).then_some(p)
                    }
                })
                // Retail's eight: as much as the points pay for.
                .unwrap_or(MAX_POWER + 1)
        }
    } else {
        let mut turns = None;
        for &(condition, lasting) in vitals.conditions.iter() {
            if cured.contains(&condition) {
                match lasting {
                    // Nothing outlasts it: the least power.
                    Lasting::Indefinite => return 1,
                    Lasting::Rounds(t) => turns = turns.max(Some(t)),
                }
            }
        }
        match turns {
            None => 1,
            Some(turns) => (1..=affordable.max(1))
                .map(|p| {
                    // What one cast really delivers: the square of the power,
                    // less the share the failure chance takes.
                    let square = p.pow(2);
                    let per = (square - (failure(p) * square / 100).min(square)).max(1);
                    (spell.cost * turns.div_ceil(per) * p, p)
                })
                .min()
                .map_or(1, |(_, p)| p),
        }
    };
    power.min(affordable)
}

/// Shared menu/commit activation rule; alternate hands are not worn.
pub fn can_activate(character: &Character, item: &ItemDefinition, worn: bool) -> bool {
    !matches!(item.use_type, UseType::None | UseType::Spellbook)
        && (!item.requires_worn() || worn)
        && (character.can_use(item) || consumed_by_anyone(item))
}

/// An item about to be used: what it is, and where it is kept.
#[derive(Clone, Copy)]
struct ItemUse<'d> {
    item: Entity,
    definition: &'d ItemDefinition,
    identified: bool,
    /// Worn in a slot in use.
    worn: bool,
    /// No charge is left in it.
    spent: bool,
}

/// The item a source uses, if it is one and is still there: carried by the
/// caster, or in the party's pool.
fn item_use<'d>(
    source: Source,
    caster: Entity,
    held: &Items,
    catalog: &'d ItemCatalog,
) -> Option<ItemUse<'d>> {
    let Source::Item(item) = source else { return None };
    if held.bearer(item) != Some(caster) && !held.pooled(item) {
        return None;
    }
    Some(ItemUse {
        item,
        definition: &catalog[held.id(item)?],
        identified: held.known(item)?.identified,
        worn: held.slot(item).is_some_and(|slot| slot.is_active()),
        spent: held.charges(item) == Some(0),
    })
}

/// An accepted cast: the spell, when it is cast, and what the caster
/// chose. [`cast`] carries it out or refuses it.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Cast {
    #[event_target]
    pub caster: Entity,
    pub spell: SpellId,
    pub context: UseContext,
    pub choice: MagicChoice,
}

/// What an attempt is: how likely it fails, whether it is a quiet (thrown)
/// use, and the strength it goes off at.
#[derive(Clone, Copy, Debug)]
struct Attempt {
    failure: u32,
    quiet: bool,
    strength: Strength,
}

/// What a backfire does instead (retail `PrepareSpellTarget`): a spell
/// aimed at one body turns on the caster, a wider one on everyone of the
/// caster's side in the fight within its reach, a blast on those within
/// its radius of the caster. It comes from whom it was aimed at, or where
/// it was to land, and is as strong as they are.
#[derive(Debug)]
struct Backfire {
    owner: Entity,
    from: Vec3,
    aim: Aim,
    victims: Vec<Entity>,
}

/// A cast aimed and judged, nothing spent yet.
#[derive(Debug)]
struct Aimed {
    victims: Vec<Entity>,
    /// The point a blast lands on or a cone fans toward; `None` for a cast
    /// aimed at bodies only.
    target: Option<Vec3>,
    power: u32,
    attempt: Attempt,
    /// An instrument's or gadget's toll on its player, paid however it
    /// played.
    instrument: Option<u32>,
    backfire: Backfire,
}

/// How far a cast got before anything is spent.
#[derive(Debug)]
enum Plan {
    Refused(Refusal),
    /// An instrument or gadget that could not be brought to play: the
    /// player is tired by `fatigue` and practises `skill`.
    PlayedBadly {
        fatigue: u32,
        skill: Option<Skill>,
    },
    Aimed(Aimed),
}

/// Who stands where as a cast is aimed.
struct Stage<'a, 'w, 's> {
    bodies: &'a Query<'w, 's, BodyReadOnly>,
    roster: &'a PartyRoster,
    combatants: &'a [Entity],
    memberships: &'a Query<'w, 's, &'static GroupMember>,
    groups: &'a Query<'w, 's, &'static GroupMembers>,
    obstacles: &'a WorldObstacles<'w, 's>,
    places: &'a Places<'w, 's>,
}

/// Whether the cast can be made at all (retail `ExecuteCharacterSpellCast`,
/// `UseItem`): by someone able to fight, in the party unless innate, not
/// silenced for a voiced cast, of a learned spell with points for it or an
/// item still there that its user may activate with a charge left, of a
/// spell the game resolves, at a moment it may be cast.
fn check(
    cast: &Cast,
    caster: Option<&BodyReadOnlyItem>,
    held: &Items,
    content: &Catalogs,
    definitions: &Definitions,
) -> Result<(), Refusal> {
    let Cast { spell, context, choice, .. } = *cast;
    let definition = &content.spells[spell];
    let innate = matches!(choice.source, Source::Innate { .. });
    let Some(caster) = caster.filter(|body| body.vitals.can_fight() && (innate || body.member))
    else {
        return Err(Refusal::CannotAct);
    };
    // In a fight only what a member carries is within reach, not the
    // party's pool (retail `CanPartySlotUseRecordedItem`).
    if context == UseContext::Combat
        && let Source::Item(item) = choice.source
        && held.pooled(item)
    {
        return Err(Refusal::CannotUse);
    }
    let character = caster.character;
    if !innate
        && let Some(character) = character
        && caster.vitals.conditions.has(Condition::Silenced)
    {
        // A learned spell is voiced, and so is an instrument's tune.
        let stopped = match item_use(choice.source, caster.entity, held, &content.items) {
            None => super::silence_stops(character, definition),
            Some(item) => item.definition.use_type == UseType::Instrument,
        };
        if stopped {
            return Err(Refusal::Silenced);
        }
    }
    if let Source::Spell { spell, power } = choice.source {
        if power == 0 {
            return Err(Refusal::CannotUse);
        }
        if !caster.spells.is_some_and(|(learned, _)| learned.0.contains(&spell)) {
            return Err(Refusal::NotLearned);
        }
        if affordable(caster, definition, content) == 0 {
            return Err(Refusal::NoSpellPoints);
        }
    }
    if let Some(character) = character
        && let Some(item) = item_use(choice.source, caster.entity, held, &content.items)
    {
        let Some(uses) = item.definition.spell else { return Err(Refusal::CannotUse) };
        let powerless = uses.power == 0
            && !matches!(item.definition.use_type, UseType::Instrument | UseType::Gadget);
        if powerless || !can_activate(character, item.definition, item.worn) {
            return Err(Refusal::CannotUse);
        }
        if item.spent {
            return Err(Refusal::NoCharges);
        }
    } else if character.is_some() && matches!(choice.source, Source::Item(_)) {
        return Err(Refusal::NoItem);
    }
    if definitions.spell(spell).is_none() {
        return Err(Refusal::Unsupported);
    }
    let repeats = choice.source == Source::Spell { spell, power: AUTO_POWER }
        && definition.power == PowerChoice::ChosenOrRepeat;
    if !innate && (!usable_now(definition, context) || (repeats && context == UseContext::Combat)) {
        return Err(Refusal::WrongTime);
    }
    Ok(())
}

/// The highest power of a spell the caster's points pay for now.
fn affordable(caster: &BodyReadOnlyItem, spell: &SpellDefinition, content: &Catalogs) -> u32 {
    let (Some(character), Some((learned, pool))) = (caster.character, caster.spells) else {
        return 0;
    };
    let maximum = character.max_points(caster.attributes, learned, &content.spells);
    pool.affordable_power(spell, &maximum)
}

impl Stage<'_, '_, '_> {
    /// A body spells may be aimed at.
    fn target(&self, body: Entity) -> Option<BodyReadOnlyItem<'_, '_>> {
        self.bodies.get(body).ok().filter(|body| !body.untargetable)
    }

    /// Everyone a spell may take, in the order the fight takes bodies.
    fn participants(&self) -> impl Iterator<Item = Entity> + '_ {
        crate::game::combat::participants(self.roster, self.combatants)
    }

    /// The living foes a caster's spell reaches, in the order the fight
    /// takes bodies: the party's members, then the fight's monsters.
    fn foes_reached(&self, caster: Entity, reach: f32) -> Vec<Entity> {
        let Some(from) = self.target(caster) else { return vec![] };
        let mine = from.standing();
        self.participants()
            .filter(|&other| {
                other != caster
                    && self.target(other).is_some_and(|to| {
                        !to.vitals.is_dead()
                            && hostility::between(mine, to.standing()) == Stance::Hostile
                            && reaches(
                                &self.places.of(from.entity),
                                &self.places.of(to.entity),
                                reach,
                                self.obstacles,
                            )
                    })
            })
            .collect()
    }

    /// The foe an aim takes among those reached: the one aimed at, or for
    /// an automatic aim the one nearest the caster's heading, the nearer of
    /// equals.
    fn aimed_foe(&self, aim: Aim, foes: &[Entity], from: &Place) -> Option<Entity> {
        match aim {
            Aim::Ahead => super::target_ahead(
                from.eye,
                from.forward(),
                f32::INFINITY,
                foes.iter().map(|&foe| {
                    let at = self.places.of(foe);
                    (foe, at.feet, from.gap(&at))
                }),
            ),
            aim => aim.body().filter(|aimed| foes.contains(aimed)),
        }
    }

    /// Whom the cast reaches and where it is aimed, by the spell's
    /// targeting (retail `PopulateSpellTargetMarkers`): its caster, the
    /// friend aimed at if on the caster's side and in reach, every member,
    /// the foe aimed at or nearest ahead, that foe's group, every foe in
    /// reach, or everyone in a cone toward the foe. A blast's victims wait
    /// for its power.
    fn aim_at(
        &self,
        caster: Entity,
        aim: Aim,
        targeting: Targeting,
        spell: &SpellDefinition,
    ) -> Result<(Vec<Entity>, Option<Vec3>), Refusal> {
        let reach_of = spell.reach.metres();
        let feet = |body: Entity| self.places.of(body).feet;
        let foe = || {
            let foes = self.foes_reached(caster, reach_of);
            let from = self.places.of(caster);
            let target = self.aimed_foe(aim, &foes, &from).ok_or(Refusal::NoTarget)?;
            Ok((foes, from, target))
        };
        match targeting {
            Targeting::Caster => Ok((vec![caster], None)),
            Targeting::Ally => {
                let friend = aim.body().and_then(|aimed| {
                    let (from, to) = (self.target(caster)?, self.target(aimed)?);
                    let friendly = aimed == caster
                        || hostility::between(from.standing(), to.standing()) == Stance::Friendly;
                    (friendly
                        && reaches(
                            &self.places.of(from.entity),
                            &self.places.of(to.entity),
                            reach_of,
                            self.obstacles,
                        ))
                    .then_some(aimed)
                });
                friend.map(|friend| (vec![friend], None)).ok_or(Refusal::NoTarget)
            }
            Targeting::Party => Ok((self.roster.iter().collect(), None)),
            Targeting::Enemy => foe().map(|(_, _, target)| (vec![target], Some(feet(target)))),
            Targeting::Radius => foe().map(|(_, _, target)| (vec![], Some(feet(target)))),
            // The aimed foe's whole group, of its side, however far or out of
            // sight (retail `PopulateSpellTargetMarkers`, `Magic.cpp:3714-3733`).
            Targeting::EnemyGroup => {
                let (_, _, first) = foe()?;
                let side = self.target(first).map(|first| first.stance());
                let mates = group_of(first, self.roster, self.memberships, self.groups)
                    .into_iter()
                    .filter(|&mate| {
                        mate != first
                            && self.target(mate).is_some_and(|mate| {
                                !mate.vitals.is_dead() && Some(mate.stance()) == side
                            })
                    });
                Ok((std::iter::once(first).chain(mates).collect(), Some(feet(first))))
            }
            Targeting::AllEnemies => {
                let victims = self.foes_reached(caster, reach_of);
                if victims.is_empty() { Err(Refusal::NoTarget) } else { Ok((victims, None)) }
            }
            Targeting::Cone => {
                let (_, from, target) = foe()?;
                let toward = feet(target) - from.eye;
                let victims = self
                    .participants()
                    .filter(|&other| other != caster)
                    .filter(|&other| {
                        self.target(other).is_some_and(|to| {
                            !to.vitals.is_dead()
                                && reaches(
                                    &from,
                                    &self.places.of(to.entity),
                                    reach_of,
                                    self.obstacles,
                                )
                                && in_cone(toward, self.places.of(to.entity).feet - from.eye)
                        })
                    })
                    .collect();
                Ok((victims, Some(from.eye + toward)))
            }
            Targeting::Point | Targeting::Item | Targeting::LockOrTrap => Err(Refusal::Unsupported),
        }
    }

    /// Those a blast takes within `radius` of `centre`.
    fn blasted(&self, centre: Vec3, radius: f32, recipients: Recipients) -> Vec<Entity> {
        within(
            centre,
            centre,
            radius,
            recipients,
            targets(self.bodies, self.places),
            (self.roster, self.combatants),
            self.obstacles,
        )
    }
}

/// The power an instrument or gadget is played at: as high as its player
/// manages, luck allowing (retail `UseItem`, which weighs the item's
/// difficulty unscaled by the round); 0 when played badly.
fn played_power(
    character: &Character,
    item: &ItemDefinition,
    spell: &SpellDefinition,
    rng: &mut GameRng,
) -> u32 {
    let mut power = MAX_POWER;
    while power > 0
        && item_failure(character, item, spell, power, true, (Difficulty::Normal, 0)).0 > 15
    {
        power -= 1;
    }
    while power > 0 && power * 7 > rng.below(100) {
        power -= 1;
    }
    power
}

/// Aim and judge the cast without spending anything: the checks that
/// refuse it, whom it reaches, the power it goes off at, its blast's
/// victims, what it is as an attempt, and what its backfire would do.
fn plan(
    cast: &Cast,
    stage: &Stage,
    held: &Items,
    (content, definitions, pace): (&Catalogs, &Definitions, (Difficulty, u32)),
    rng: &mut GameRng,
) -> Plan {
    let Cast { caster, choice: MagicChoice { source, aim }, .. } = *cast;
    let spell = &content.spells[cast.spell];
    let body = stage.bodies.get(caster).ok();
    if let Err(refusal) = check(cast, body.as_ref(), held, content, definitions) {
        return Plan::Refused(refusal);
    }
    let body = body.unwrap();
    let item = item_use(source, caster, held, &content.items);
    // A potion or food used in a fight is taken by its user (retail
    // `UseItem` aims potions at their user).
    let forced = cast.context == UseContext::Combat
        && item.is_some_and(|item| consumed_by_anyone(item.definition));
    let targeting = if forced { Targeting::Caster } else { spell.targeting };
    let (mut victims, target) = match stage.aim_at(caster, aim, targeting, spell) {
        Ok(aimed) => aimed,
        Err(refusal) => return Plan::Refused(refusal),
    };
    let instrument = item
        .filter(|item| matches!(item.definition.use_type, UseType::Instrument | UseType::Gadget));
    let toll = instrument.map(|_| spell.level * 8 + 20);
    let power = match (source, instrument) {
        (_, Some(item)) => {
            let power = played_power(body.character.unwrap(), item.definition, spell, rng);
            if power == 0 {
                return Plan::PlayedBadly {
                    fatigue: toll.unwrap(),
                    skill: item.definition.use_type.skill(),
                };
            }
            power
        }
        (Source::Innate { power, .. }, _) => power,
        (Source::Spell { power, .. }, _) => {
            let affordable = affordable(&body, spell, content);
            if power == AUTO_POWER {
                let victim = aim.body().and_then(|body| stage.bodies.get(body).ok());
                let character = body.character.unwrap();
                let failure = |power| {
                    pace.0.scale_by_pace(super::fail_chance(character, spell, power), pace.1)
                };
                auto_power(
                    spell,
                    victim.as_ref().map(|victim| (victim.vitals, victim.member)),
                    affordable,
                    &failure,
                )
            } else {
                power.min(affordable)
            }
        }
        (Source::Item(_), None) => {
            item.and_then(|item| item.definition.spell).map_or(0, |uses| uses.power)
        }
    };
    // A blast takes whoever stands within its radius of where it lands,
    // but those on its caster's side.
    let radius = spell.radius + spell.radius_per_power * power as f32;
    let side = body.stance();
    if let (Targeting::Radius, Some(centre)) = (targeting, target) {
        victims = stage.blasted(centre, radius, Recipients::Opponents(side));
    }
    let attempt = match source {
        Source::Innate { budget, level, .. } => Attempt {
            failure: super::spell_failure(budget, spell, power),
            quiet: false,
            strength: Strength::at(u32::from(level), spell, power, 0),
        },
        _ => attempt(body.character.unwrap(), item, spell, power, pace, aim.body() == Some(caster)),
    };
    let stand = stage.places.of(body.entity);
    let eye_of = |body: Entity| {
        stage.bodies.get(body).map_or(stand.eye, |body| stage.places.of(body.entity).eye)
    };
    let (owner, from) = match (targeting, aim) {
        (Targeting::Enemy | Targeting::Ally, Aim::Body(entity)) => (entity, eye_of(entity)),
        (Targeting::EnemyGroup, Aim::Body(entity)) => {
            let leader = leader(entity, stage.combatants, stage.memberships, stage.groups);
            (leader, eye_of(leader))
        }
        (Targeting::Radius | Targeting::Cone, _) => (caster, target.unwrap_or(stand.eye)),
        _ => (caster, stand.eye),
    };
    let turned_on = match targeting {
        Targeting::Caster | Targeting::Ally | Targeting::Enemy => vec![caster],
        Targeting::Radius => {
            stage.blasted(stand.feet, radius, Recipients::Opponents(side).reversed())
        }
        // Turned back, it takes the caster's whole party or group, however
        // far or out of sight (retail `RedirectBackfiredSpellTarget`).
        Targeting::EnemyGroup => group_of(caster, stage.roster, stage.memberships, stage.groups)
            .into_iter()
            .filter(|&other| stage.target(other).is_some_and(|to| !to.vitals.is_dead()))
            .collect(),
        _ => stage
            .participants()
            .filter(|&other| {
                stage.target(other).is_some_and(|to| {
                    !to.vitals.is_dead()
                        && to.stance() == side
                        && (other == caster
                            || reaches(
                                &stand,
                                &stage.places.of(to.entity),
                                spell.reach.metres(),
                                stage.obstacles,
                            ))
                })
            })
            .collect(),
    };
    let backfire_aim = match (targeting, target) {
        (Targeting::Radius | Targeting::Cone, Some(at)) => Aim::Point(at),
        _ => aim,
    };
    Plan::Aimed(Aimed {
        victims,
        target,
        power,
        attempt,
        instrument: toll,
        backfire: Backfire { owner, from, aim: backfire_aim, victims: turned_on },
    })
}

/// Carry out an accepted cast (retail `CastSpellFromSource`): aim and
/// judge it without spending; refuse it with a word to the player, or
/// provoke whom a member aims at, roll it, pay for it whatever the roll,
/// tire the caster, and release it as a deed ([`crate::game::hits`]). A
/// fizzle leaves only its report, and an auto cast tries again while
/// stamina and points last; a backfire turns on the caster's side; a cast
/// that went off takes its victims, its missiles landing as they arrive.
/// In a fight it counts as an attack.
pub fn cast(
    cast: On<Cast>,
    mut bodies: Query<Body>,
    (memberships, groups): (Query<&'static GroupMember>, Query<&'static GroupMembers>),
    mut fight: Option<
        Single<(&mut crate::game::combat::Round, Option<&crate::game::combat::Combatants>)>,
    >,
    roster: Single<&PartyRoster>,
    held: Items,
    obstacles: WorldObstacles,
    places: Places,
    states: Query<&crate::game::world::triggers::LevelState, With<Level>>,
    (content, definitions, difficulty): (Res<Catalogs>, Res<Definitions>, Res<Difficulty>),
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<MagicNews>,
    mut commands: Commands,
) {
    let roster = *roster;
    let underwater = states.single().is_ok_and(|state| state.camera_sway);
    // The caster's moment in the round, which the failure chance reads.
    let moment = fight.as_ref().map_or(0, |fight| fight.0.moment());
    // An auto cast that fizzles goes again at once, provoking nobody anew.
    let mut pending = vec![(*cast, false)];
    while let Some((cast, retry)) = pending.pop() {
        let Cast { caster, spell, context, choice } = cast;
        let source = choice.source;
        let definition = &content.spells[spell];
        let readonly = bodies.as_readonly();
        let plan = plan(
            &cast,
            &Stage {
                bodies: &readonly,
                roster,
                combatants: fight
                    .as_ref()
                    .and_then(|fight| fight.1)
                    .map_or(&[][..], |fight| fight.entities()),
                memberships: &memberships,
                groups: &groups,
                obstacles: &obstacles,
                places: &places,
            },
            &held,
            (&content, &definitions, (*difficulty, moment)),
            &mut rng,
        );
        let aimed = match plan {
            Plan::Refused(refusal) => {
                commands.trigger(Refused { caster, refusal });
                continue;
            }
            Plan::PlayedBadly { fatigue, skill } => {
                tire_by_load(&mut bodies.get_mut(caster).unwrap(), fatigue);
                if let Some(skill) = skill {
                    commands.trigger(Practise { member: caster, skill, usage: 1 });
                }
                commands.trigger(Refused { caster, refusal: Refusal::PlayedBadly });
                continue;
            }
            Plan::Aimed(aimed) => aimed,
        };
        let Aimed { victims, target, power, attempt, instrument, backfire } = aimed;
        let outcome = super::roll(&mut rng, attempt.failure, definition, attempt.quiet, underwater);
        let backfired = outcome == Outcome::Backfired;
        // What a backfire is as strong as, judged before anything changes.
        let strength = if backfired {
            owners_strength(
                Some(backfire.owner),
                definition,
                (attempt.strength.power, attempt.strength.boost),
                &readonly,
            )
        } else {
            attempt.strength
        };
        let mut body = bodies.get_mut(caster).unwrap();
        // A party member's spell aimed at enemies provokes those it is cast
        // at, whatever the roll (retail `MakeTargetGroupHostile` as the
        // action starts); a retry provokes nobody again.
        if !retry && body.member && definition.aimed_at_enemies() {
            // They are hostile before the cast lands.
            for &victim in &victims {
                commands.trigger(Provocation { monster: victim });
            }
        }
        let innate = matches!(source, Source::Innate { .. });
        let item = item_use(source, caster, &held, &content.items);
        let report = Report {
            caster,
            spell,
            power,
            item: item.map(|item| item.definition.name.clone()),
            outcome,
            effects: Vec::new(),
        };
        // Paid whatever the roll: points for a learned spell, a use for an
        // item, nothing for an innate cast; then the caster is tired: a
        // member more under load, a monster by the cast alone.
        match (source, item) {
            (Source::Spell { .. }, _) => {
                let realm = definition.realm.expect("a cast spell has a realm");
                body.spells.as_mut().unwrap().1.spend(realm, definition.cost_at(power));
            }
            (_, Some(item)) => commands.trigger(Spend { item: item.item, amount: 1 }),
            _ => {}
        }
        if innate {
            // The spell's level and its power (retail `MonsterCastsSpell`,
            // `SpellCastFatigueCost`, then `FatigueMonster`).
            body.vitals.tire(super::cast_fatigue(definition, power));
        } else {
            let fatigue = match item.map(|item| item.definition.use_type) {
                None => super::cast_fatigue(definition, power),
                Some(UseType::Thrown) => 10,
                Some(UseType::Instrument | UseType::Gadget) => instrument.unwrap_or_default(),
                Some(_) => {
                    let base = rng.below(2) + 1;
                    if body.vitals.conditions.has(Condition::Diseased) { base * 2 } else { base }
                }
            };
            tire_by_load(&mut body, fatigue);
        }
        let auto = source == Source::Spell { spell, power: AUTO_POWER };
        if outcome == Outcome::Fizzled {
            news.write(MagicNews::Done(report));
            let again =
                bodies.get(caster).is_ok_and(|body| can_repeat(&body, definition, &content));
            if auto && again {
                pending.push((cast, true));
            }
            continue;
        }
        let stand = places.of(body.entity);
        let (owner, spell_of, victims) = if backfired {
            let Backfire { owner, from, aim, victims } = backfire;
            (owner, Spell { owner: Some(owner), from, aim, ..Spell::of(spell, caster) }, victims)
        } else {
            let aim = match (definition.targeting, target) {
                (Targeting::Radius | Targeting::Cone, Some(at)) => Aim::Point(at),
                _ => choice.aim,
            };
            let from = stand.eye;
            (caster, Spell { owner: Some(caster), from, aim, ..Spell::of(spell, caster) }, victims)
        };
        if context == UseContext::Combat
            && let Some(fight) = fight.as_mut()
        {
            fight.0.attacked(true);
        }
        let uses = if backfired { definitions.backfire(spell) } else { definitions.spell(spell) };
        let uses = uses.expect("a checked spell is defined");
        let flies = definition.missile.is_some() && !backfired && !victims.is_empty();
        let practice = body.character.as_deref().filter(|_| !innate).map(|character| Practice {
            caster,
            skills: practice(character, definition, power, item.map(|item| item.definition)),
            sure: flies,
        });
        // A missile cast is reported as it leaves; its impacts when they land.
        let act = if flies {
            news.write(MagicNews::Done(report));
            Act::Missile { caster, spell }
        } else {
            Act::Cast(report)
        };
        let flight = |victim: Entity| {
            let to = match (definition.targeting, target) {
                (Targeting::Radius, Some(at)) => at,
                _ => bodies.get(victim).map_or(stand.eye, |body| places.of(body.entity).feet),
            };
            flies.then(|| to.distance(stand.eye) / MISSILE_SPEED)
        };
        commands.trigger(Release {
            victims: victims.iter().map(|&victim| (victim, flight(victim))).collect(),
            deed: Deed {
                act,
                strike: Strike { definition: uses, strength, blow: None },
                leech: definitions
                    .get(uses)
                    .leeches
                    .map(|lifesteal| Leech { caster: owner, lifesteal }),
                practice,
                recast: (auto && definition.power == PowerChoice::ChosenOrRepeat).then(|| Recast {
                    caster,
                    choice,
                    victims: victims.clone(),
                }),
                spell: Some(Spell { backfired, victims, ..spell_of }),
            },
        });
    }
}

/// Tire a member by `fatigue`, more under load, less when slowed, more when
/// hasted (retail `FatigueCharacter` with its load scaling).
fn tire_by_load(body: &mut BodyItem, fatigue: u32) {
    let slowed = body.vitals.conditions.has(Condition::Slowed);
    let hasted = body.attacks.hasted > 0.0;
    let load = body.burden.map(|burden| burden.load).unwrap_or_default();
    body.vitals.tire(load.scale_fatigue(fatigue, slowed, hasted));
}

/// A utility or device cast's source, revalidated, rolled and paid at once
/// by its caster: retail counts these as having done something even when
/// they fizzle, so the caster practises whatever the roll. `underwater`
/// quenches Fire; `moment` is the caster's in the round of a fight (0 outside
/// one).
pub(crate) fn pay(
    caster: &mut BodyItem,
    held: &Items,
    (source, context): (Source, UseContext),
    (content, difficulty, underwater): (&Catalogs, Difficulty, bool),
    moment: u32,
    rng: &mut GameRng,
    commands: &mut Commands,
) -> Result<(Report, Strength), Refusal> {
    let (spells, items) = (&content.spells, &content.items);
    let (Some(character), true) = (caster.character.as_deref_mut(), caster.member) else {
        return Err(Refusal::CannotAct);
    };
    let vitals = &mut *caster.vitals;
    if !vitals.can_fight() {
        return Err(Refusal::CannotAct);
    }
    let spell = source.spell_id(held, items).ok_or(Refusal::NoItem)?;
    let spell = &spells[spell];
    if !usable_now(spell, context) {
        return Err(Refusal::WrongTime);
    }
    let item = item_use(source, caster.entity, held, items);
    let power = match (source, item) {
        (Source::Spell { power, .. }, _) => {
            if vitals.conditions.has(Condition::Silenced) && super::silence_stops(character, spell)
            {
                return Err(Refusal::Silenced);
            }
            let Some((learned, pool)) =
                caster.spells.as_ref().filter(|s| s.0.0.contains(&spell.id))
            else {
                return Err(Refusal::NotLearned);
            };
            let maximum = character.max_points(caster.attributes, learned, spells);
            power.min(MAX_POWER).min(pool.affordable_power(spell, &maximum))
        }
        (Source::Innate { .. }, _) | (_, None) => return Err(Refusal::NoItem),
        (_, Some(item)) => item.definition.spell.map_or(0, |cast| cast.power),
    };
    if power == 0 {
        return Err(Refusal::NoSpellPoints);
    }
    let Attempt { failure, quiet, strength } =
        attempt(character, item, spell, power, (difficulty, moment), false);
    let report = Report {
        caster: caster.entity,
        spell: spell.id,
        power,
        item: item.map(|item| item.definition.name.clone()),
        outcome: super::roll(rng, failure, spell, quiet, underwater),
        effects: Vec::new(),
    };
    let skills = practice(character, spell, power, item.map(|item| item.definition));
    match item {
        None => {
            let realm = spell.realm.expect("a cast spell has a realm");
            caster.spells.as_mut().unwrap().1.spend(realm, spell.cost_at(power));
        }
        Some(item) => commands.trigger(Spend { item: item.item, amount: 1 }),
    }
    vitals.tire(super::cast_fatigue(spell, power));
    for (skill, usage) in skills {
        commands.trigger(Practise { member: caster.entity, skill, usage });
    }
    Ok((report, strength))
}

/// A sourced attempt at `power`: its failure chance, whether it is quiet (a
/// thrown item), and the strength it casts with, from a learned spell or an
/// item. `pace` is the difficulty and the caster's moment in the round;
/// `on_oneself` is a learned spell aimed at its caster.
fn attempt(
    character: &Character,
    item: Option<ItemUse>,
    spell: &SpellDefinition,
    power: u32,
    pace: (Difficulty, u32),
    on_oneself: bool,
) -> Attempt {
    match item {
        Some(item) => {
            let (failure, quiet) =
                item_failure(character, item.definition, spell, power, item.identified, pace);
            Attempt {
                failure,
                quiet,
                strength: item_strength(character, item.definition, spell, power),
            }
        }
        // Sane Mind is hard to cast on oneself: 50 more in the failure chance,
        // after the round's pace (retail `ExecuteCharacterSpellCast`,
        // `Magic.cpp:2656-2658`).
        None => Attempt {
            failure: pace.0.scale_by_pace(super::fail_chance(character, spell, power), pace.1)
                + if spell.id == SpellId::SANE_MIND && on_oneself { 50 } else { 0 },
            quiet: false,
            strength: Strength::of(character, spell, power),
        },
    }
}

/// Practice from a cast that did something (retail `Magic.cpp:2682-2692`,
/// `PC Item.cpp:3792-3796`): a learned spell trains its book, its realm and
/// Power Cast; an item its use's skill.
fn practice(
    character: &Character,
    spell: &SpellDefinition,
    power: u32,
    item: Option<&ItemDefinition>,
) -> Vec<(Skill, u32)> {
    match item {
        None => {
            let index = spell.level + power;
            let mut practice = vec![(super::best_book(character, spell), (index + 2) >> 2)];
            if let Some(realm) = spell.realm {
                practice.push((realm.skill(), index));
            }
            if character.skill_available(Skill::PowerCast) {
                practice.push((Skill::PowerCast, (index + 2) >> 2));
            }
            practice
        }
        Some(definition) => definition
            .use_type
            .skill()
            .map(|skill| vec![(skill, spell.level + power / 2)])
            .unwrap_or_default(),
    }
}

/// The spells and items an actor could use: what menus offer. Execution
/// validates again.
pub fn usable_items(
    member: Entity,
    character: &Character,
    held: &Items,
    catalog: &ItemCatalog,
    definitions: &Definitions,
) -> Vec<(Entity, SpellId)> {
    held.carried(member)
        .filter_map(|item| {
            let worn = held.slot(item).is_some_and(|slot| slot.is_active());
            Some((item, usable_item(character, item, held, catalog, definitions, worn)?))
        })
        .collect()
}

/// The spell an item casts for `character`, if they can use it now: one
/// they may activate (worn, if it must be), with a charge left, whose
/// effect the game resolves.
pub fn usable_item(
    character: &Character,
    item: Entity,
    held: &Items,
    catalog: &ItemCatalog,
    definitions: &Definitions,
    worn: bool,
) -> Option<SpellId> {
    let definition = &catalog[held.id(item)?];
    let cast = definition.spell?;
    (can_activate(character, definition, worn)
        && held.charges(item) != Some(0)
        && resolvable(cast.spell, definitions))
    .then_some(cast.spell)
}

/// Whether a learned spell can be cast in `context` with `affordable`
/// power within reach: as the cast itself checks, points for it, an effect
/// the game resolves, the moment for it and no silence.
pub fn castable(
    spell: &SpellDefinition,
    affordable: u32,
    context: UseContext,
    silenced: bool,
    definitions: &Definitions,
) -> bool {
    affordable > 0 && resolvable(spell.id, definitions) && usable_now(spell, context) && !silenced
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::Realm;
    use crate::content::spells::Spellbook;
    use crate::game::characters::{Gender, Race};

    /// A first-level priest with no skill points: a Divinity of 1 by the
    /// bonus skill, a casting skill of (1 + 4 * 0) / 5 = 0.
    fn untrained_priest() -> Character {
        let mut priest = Character::new("Cleric", Race::Human, Gender::Male, Profession::Priest);
        priest.skill_points = default();
        priest
    }

    fn spell(id: SpellId) -> SpellDefinition {
        SpellDefinition {
            id,
            level: 7,
            cost: 14,
            realm: Some(Realm::Divine),
            books: Spellbook::Divinity.into(),
            ..default()
        }
    }

    /// Band 14 / 2 + 7 = 14 asks 105 at power 7: 70% for a skill of 0. A
    /// level 7 spell asks caster level 18; the priest has 1: 18 - 1 - 1 + 7
    /// = 23 short, 7 * 23 = 161 more. 231 in all, not held to 100.
    #[test]
    fn a_failure_chance_is_not_held_to_a_hundred_and_grows_late_in_a_round() {
        let (priest, spell) = (untrained_priest(), spell(SpellId::HEAL_ALL));
        let failure =
            |pace: (Difficulty, u32)| attempt(&priest, None, &spell, 7, pace, false).failure;
        assert_eq!(failure((Difficulty::Normal, 0)), 231);
        // Normal pace is 60: nothing is added before it, and at it the
        // factor is (60 + 50 - 60) * 2 / 100 = 1.
        assert_eq!(failure((Difficulty::Normal, 59)), 231);
        assert_eq!(failure((Difficulty::Normal, 60)), 231);
        // At 90: (90 + 50 - 60) = 80; 80 * 231 * 2 / 100 = 369.
        assert_eq!(failure((Difficulty::Normal, 90)), 369);
        // Novice's pace is 80; at 100: (100 + 50 - 80) = 70; 70 * 231 * 2 / 100 = 323.
        assert_eq!(failure((Difficulty::Novice, 100)), 323);
    }

    /// An item's chance is scaled by the round the same way: 70% for a
    /// skill of 0, nine tenths of it for a charged item (63), then at
    /// moment 100 on Normal (100 + 50 - 60) * 63 * 2 / 100 = 113, and 30 more
    /// unidentified.
    #[test]
    fn an_items_failure_chance_grows_late_in_a_round() {
        let (priest, spell) = (untrained_priest(), spell(SpellId::HEAL_ALL));
        let wand = ItemDefinition { use_type: UseType::Charged, ..default() };
        let failure = |identified| {
            item_failure(&priest, &wand, &spell, 7, identified, (Difficulty::Normal, 100)).0
        };
        assert_eq!(failure(true), 113);
        assert_eq!(failure(false), 143);
    }

    #[test]
    fn sane_mind_is_harder_to_cast_on_oneself_by_fifty() {
        let (priest, sane) = (untrained_priest(), spell(SpellId::SANE_MIND));
        let pace = (Difficulty::Normal, 90);
        // 231 at moment 90 is 369 (above); 50 more after the pace, aimed
        // at oneself.
        assert_eq!(attempt(&priest, None, &sane, 7, pace, false).failure, 369);
        assert_eq!(attempt(&priest, None, &sane, 7, pace, true).failure, 419);
        let other = spell(SpellId::CURE_POISON);
        assert_eq!(attempt(&priest, None, &other, 7, pace, true).failure, 369);
    }

    fn cure_poison() -> SpellDefinition {
        SpellDefinition { cost: 3, ..spell(SpellId::CURE_POISON) }
    }

    /// Cost 3, a poison of 20 rounds, no failure: power p delivers p * p a
    /// cast, so it takes ceil(20 / p^2) casts of 3 * p points:
    /// 60, 30, 27, 24, 15, 18, 21 for powers 1 to 7. Power 5 is cheapest.
    #[test]
    fn a_cure_takes_the_cheapest_power_to_outlast_the_poison() {
        let mut poisoned = Vitals::full(10, 10);
        poisoned.afflict(Condition::Poisoned, Lasting::Rounds(20));
        let sure = |_| 0;
        assert_eq!(auto_power(&cure_poison(), Some((&poisoned, true)), 7, &sure), 5);
        // Held to what the points pay for: 4 costs 24 of 60, 30, 27, 24.
        assert_eq!(auto_power(&cure_poison(), Some((&poisoned, true)), 4, &sure), 4);
        // A failure chance of 50% halves what a cast delivers: p * p - p * p / 2.
        // Powers 1 to 7 deliver 1, 2, 5, 8, 13, 18, 25 (24 / 2 = 12.5 cut to 12 off 25).
        let half = |_| 50;
        // Casts: 20, 10, 4, 3, 2, 2, 1; points 60, 60, 36, 36, 30, 36, 21.
        assert_eq!(auto_power(&cure_poison(), Some((&poisoned, true)), 7, &half), 7);
    }

    /// Only the conditions a cure lifts count, and one without end cannot be
    /// outlasted.
    #[test]
    fn a_cure_looks_only_at_the_conditions_it_lifts_and_gives_up_on_one_without_end() {
        let lesser = SpellDefinition { cost: 3, ..spell(SpellId::CURE_LESSER_COND) };
        let sure = |_| 0;
        let mut ill = Vitals::full(10, 10);
        // Irritated is not among the five it is chosen for.
        ill.afflict(Condition::Irritated, Lasting::Rounds(60));
        assert_eq!(auto_power(&lesser, Some((&ill, true)), 7, &sure), 1);
        // Nauseated for 20 rounds wants power 5 (above); Asleep without end
        // outweighs it, and the least power is taken.
        ill.afflict(Condition::Nauseated, Lasting::Rounds(20));
        assert_eq!(auto_power(&lesser, Some((&ill, true)), 7, &sure), 5);
        ill.afflict(Condition::Asleep, Lasting::Indefinite);
        assert_eq!(auto_power(&lesser, Some((&ill, true)), 7, &sure), 1);
    }

    fn dice(count: i32, sides: i32) -> Option<crate::content::items::Dice> {
        Some(crate::content::items::Dice { base: 0, count, sides })
    }

    /// 1d4 averages 2.5, cut to 2: a victim lacking 2 is not covered at power
    /// 1 (2 < 2 is false) but is at power 2 (5).
    #[test]
    fn a_heal_is_chosen_by_the_average_roll_cut_to_a_whole_number() {
        let heal = SpellDefinition { dice: dice(1, 4), ..spell(SpellId::HEAL_WOUNDS) };
        let mut hurt = Vitals::full(10, 10);
        hurt.health.current = 8;
        let sure = |_| 0;
        assert_eq!(auto_power(&heal, Some((&hurt, true)), 7, &sure), 2);
        // A monster is healed at 1, and nobody lacking nothing at more.
        assert_eq!(auto_power(&heal, Some((&hurt, false)), 7, &sure), 1);
        assert_eq!(auto_power(&heal, Some((&Vitals::full(10, 10), true)), 7, &sure), 1);
        // Where no power covers it, retail's 8 is held to what the points pay for.
        hurt.health.current = 0;
        let small = SpellDefinition { dice: dice(1, 1), ..heal };
        assert_eq!(auto_power(&small, Some((&hurt, true)), 5, &sure), 5);
        // Over 10% failing from power 3 on stops it at 2.
        let unsure = |power| if power >= 3 { 20 } else { 0 };
        assert_eq!(auto_power(&small, Some((&hurt, true)), 7, &unsure), 2);
    }

    /// Restoration looks to stamina only once health is full.
    #[test]
    fn restoration_heals_health_first_and_then_stamina() {
        let restoration = SpellDefinition { dice: dice(1, 1), ..spell(SpellId::RESTORATION) };
        let sure = |_| 0;
        // 1d1 averages p. A health of 10 lacking 6 wants power 7; stamina
        // lacking 2 alone wants 3.
        let mut hurt = Vitals::full(10, 10);
        hurt.health.current = 4;
        hurt.stamina.current = 8;
        assert_eq!(auto_power(&restoration, Some((&hurt, true)), 7, &sure), 7);
        hurt.health.current = 10;
        assert_eq!(auto_power(&restoration, Some((&hurt, true)), 7, &sure), 3);
    }
}
