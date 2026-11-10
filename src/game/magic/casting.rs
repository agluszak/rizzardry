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
use crate::game::characters::{Character, Condition, Lasting, Profession, Vitals};
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::hits::definitions::{Definitions, EffectKind};
use crate::game::hits::effects::{Body, BodyItem, BodyReadOnly, BodyReadOnlyItem, Strike};
use crate::game::hits::reach::{Recipients, within};
use crate::game::hits::spells::{Leech, Spell, group_of, leader, owners_strength, targets};
use crate::game::hits::{Act, Deed, Effect, Release};
use crate::game::inventory::{Carried, PartyInventory, StackId};
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
    /// A stack the actor carries, identified, not reserved.
    Item(StackId),
    /// A shared-pool stack; payment never silently changes owners.
    PoolItem(StackId),
    /// A monster's own casting, which costs nothing: its spell budget
    /// stands in for a caster's skill, its level sets the strength.
    Innate { spell: SpellId, power: u32, budget: u32, level: u8 },
}

impl Source {
    /// Resolve an item source by its stable stack identity. This is routing,
    /// not acceptance: the cast revalidates eligibility and payment.
    pub fn spell_id(
        self,
        carried: &Carried,
        inventory: &PartyInventory,
        items: &ItemCatalog,
    ) -> Option<SpellId> {
        match self {
            Self::Spell { spell, .. } | Self::Innate { spell, .. } => Some(spell),
            Self::Item(id) => items[carried.find_stack(id)?.item].spell.map(|s| s.spell),
            Self::PoolItem(id) => items[inventory.pool.stacks.iter().find(|s| s.id == id)?.item]
                .spell
                .map(|s| s.spell),
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
    /// The selected stack, retained across any delayed request.
    Item(StackId),
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
    /// is practice, and `improved` is the skill it raised, if any.
    PlayedBadly {
        improved: Option<Skill>,
    },
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
    difficulty: Difficulty,
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
    // Choices are made between rounds, so none is late in one.
    let mut failure = difficulty.scale_by_pace(failure, 0);
    if !identified {
        failure += 30;
    }
    let quiet = definition.use_type == UseType::Thrown;
    if quiet {
        failure /= 2;
    }
    (failure.min(100), quiet)
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

/// The power an auto cast takes (retail `ChooseSpellPowerLevelForTarget`):
/// for healing, the lowest whose average covers what the victim lacks,
/// stopping short of a failure chance over 10%; for cures, the cheapest to
/// outlast the condition; otherwise 1.
fn auto_power(
    character: &Character,
    spell: &SpellDefinition,
    victim: Option<&Vitals>,
    affordable: u32,
    restores: bool,
) -> u32 {
    let fail = |p| super::fail_chance(character, spell, p);
    let Some(vitals) = victim else { return 1 };
    let power = if restores {
        let pool = if spell.id == SpellId::STAMINA { vitals.stamina } else { vitals.health };
        let missing = (pool.max as f32 - pool.current as f32).max(0.0);
        let average = |p: u32| {
            spell.dice.map_or(0.0, |d| {
                d.count as f32 * p as f32 * (d.sides as f32 + 1.0) / 2.0 + d.base as f32
            })
        };
        if missing == 0.0 {
            1
        } else {
            (1..=MAX_POWER)
                .find_map(|p| {
                    if fail(p) > 10 {
                        Some((p - 1).max(1))
                    } else {
                        (missing < average(p)).then_some(p)
                    }
                })
                .unwrap_or(MAX_POWER)
        }
    } else {
        let turns = vitals
            .conditions
            .iter()
            .filter_map(|&(_, lasting)| match lasting {
                Lasting::Rounds(t) => Some(t),
                Lasting::Indefinite => None,
            })
            .max();
        match turns {
            None => 1,
            Some(turns) => (1..=affordable.max(1))
                .map(|p| {
                    let square = p.pow(2);
                    let per = (square - fail(p) * square / 100).max(1);
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

/// An item about to be used: its stack, what it is, and where it is kept.
#[derive(Clone, Copy)]
struct ItemUse<'d> {
    id: StackId,
    definition: &'d ItemDefinition,
    identified: bool,
    from_pool: bool,
    /// No charge is left in it.
    spent: bool,
}

/// The item a source uses, if it is one and is still there.
fn item_use<'d>(
    source: Source,
    carried: &Carried,
    inventory: &PartyInventory,
    items: &'d ItemCatalog,
) -> Option<ItemUse<'d>> {
    let (id, from_pool) = match source {
        Source::Spell { .. } | Source::Innate { .. } => return None,
        Source::Item(id) => (id, false),
        Source::PoolItem(id) => (id, true),
    };
    let stack = if from_pool {
        inventory.pool.stacks.iter().find(|s| s.id == id)
    } else {
        carried.find_stack(id)
    }?;
    Some(ItemUse {
        id,
        definition: &items[stack.item],
        identified: stack.identified,
        from_pool,
        spent: stack.remaining_uses() == Some(0),
    })
}

/// An accepted cast: the spell, when it is cast, and what the caster
/// chose. [`cast`] carries it out or refuses it.
#[derive(Message, Clone, Copy, Debug)]
pub struct Cast {
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
    inventory: &PartyInventory,
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
    let character = caster.character.zip(caster.carried);
    if !innate && let Some((_, carried)) = character {
        let item = item_use(choice.source, carried, inventory, &content.items);
        let voiced = item.is_none_or(|item| item.definition.use_type == UseType::Instrument);
        if voiced && caster.vitals.conditions.has(Condition::Silenced) {
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
    if let Some((character, carried)) = character
        && let Some(item) = item_use(choice.source, carried, inventory, &content.items)
    {
        let worn =
            !item.from_pool && carried.active_equipment().any(|(_, e)| e.stack.id == item.id);
        let Some(uses) = item.definition.spell else { return Err(Refusal::CannotUse) };
        let powerless = uses.power == 0
            && !matches!(item.definition.use_type, UseType::Instrument | UseType::Gadget);
        if powerless || !can_activate(character, item.definition, worn) {
            return Err(Refusal::CannotUse);
        }
        if item.spent {
            return Err(Refusal::NoCharges);
        }
    } else if character.is_some() && matches!(choice.source, Source::Item(_) | Source::PoolItem(_))
    {
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
            Targeting::EnemyGroup => {
                let (foes, _, first) = foe()?;
                let mates = group_of(first, self.roster, self.memberships, self.groups)
                    .into_iter()
                    .filter(|mate| *mate != first && foes.contains(mate));
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
/// manages, luck allowing (retail `UseItem`); 0 when played badly.
fn played_power(
    character: &Character,
    item: &ItemDefinition,
    spell: &SpellDefinition,
    rng: &mut GameRng,
) -> u32 {
    let mut power = MAX_POWER;
    while power > 0 && item_failure(character, item, spell, power, true, Difficulty::Normal).0 > 15
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
    inventory: &PartyInventory,
    (content, definitions, difficulty): (&Catalogs, &Definitions, Difficulty),
    rng: &mut GameRng,
) -> Plan {
    let Cast { caster, choice: MagicChoice { source, aim }, .. } = *cast;
    let spell = &content.spells[cast.spell];
    let body = stage.bodies.get(caster).ok();
    if let Err(refusal) = check(cast, body.as_ref(), inventory, content, definitions) {
        return Plan::Refused(refusal);
    }
    let body = body.unwrap();
    let item =
        body.carried.and_then(|carried| item_use(source, carried, inventory, &content.items));
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
                let restores = definitions.spell(spell.id).is_some_and(|id| {
                    definitions
                        .get(id)
                        .effects
                        .iter()
                        .any(|e| matches!(e.kind, EffectKind::Heal | EffectKind::Refresh))
                });
                auto_power(
                    body.character.unwrap(),
                    spell,
                    victim.map(|victim| victim.vitals),
                    affordable,
                    restores,
                )
            } else {
                power.min(affordable)
            }
        }
        (Source::Item(_) | Source::PoolItem(_), None) => {
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
        _ => attempt(body.character.unwrap(), item, spell, power, difficulty),
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

/// Carry out every accepted cast (retail `CastSpellFromSource`): aim and
/// judge it without spending; refuse it with a word to the player, or
/// provoke whom a member aims at, roll it, pay for it whatever the roll,
/// tire the caster, and release it as a deed ([`crate::game::hits`]). A
/// fizzle leaves only its report, and an auto cast tries again while
/// stamina and points last; a backfire turns on the caster's side; a cast
/// that went off takes its victims, its missiles landing as they arrive.
/// In a fight it counts as an attack.
pub fn cast(
    mut casts: ResMut<Messages<Cast>>,
    mut bodies: Query<Body>,
    (memberships, groups): (Query<&'static GroupMember>, Query<&'static GroupMembers>),
    mut fight: Option<
        Single<(&mut crate::game::combat::Round, Option<&crate::game::combat::Combatants>)>,
    >,
    party: Single<(&PartyRoster, &mut PartyInventory)>,
    obstacles: WorldObstacles,
    places: Places,
    states: Query<&crate::game::world::triggers::LevelState, With<Level>>,
    (content, definitions, difficulty): (Res<Catalogs>, Res<Definitions>, Res<Difficulty>),
    mut rng: ResMut<GameRng>,
    (mut news, mut provocations, mut released): (
        MessageWriter<MagicNews>,
        MessageWriter<Provocation>,
        MessageWriter<Release>,
    ),
) {
    let (roster, mut inventory) = party.into_inner();
    let underwater = states.single().is_ok_and(|state| state.camera_sway);
    // An auto cast that fizzles goes again at once, provoking nobody anew.
    // Casts are taken, not read: whichever schedule carries casts out now
    // (the frame's outside a fight, the fixed step's within one) does so once.
    let mut pending: Vec<(Cast, bool)> = casts.drain().map(|cast| (cast, false)).collect();
    pending.reverse();
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
            &inventory,
            (&content, &definitions, *difficulty),
            &mut rng,
        );
        let aimed = match plan {
            Plan::Refused(refusal) => {
                news.write(MagicNews::Refused { caster, refusal });
                continue;
            }
            Plan::PlayedBadly { fatigue, skill } => {
                let mut body = bodies.get_mut(caster).unwrap();
                body.vitals.tire(fatigue);
                let improved = skill.filter(|&skill| {
                    body.character.as_deref_mut().unwrap().practise(skill, 1, &mut rng)
                });
                let refusal = Refusal::PlayedBadly { improved };
                news.write(MagicNews::Refused { caster, refusal });
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
        if let Some(fatigue) = instrument {
            body.vitals.tire(fatigue);
        }
        // A party member's spell aimed at enemies provokes those it is cast
        // at, whatever the roll (retail `MakeTargetGroupHostile` as the
        // action starts); a retry provokes nobody again.
        if !retry && body.member && definition.aimed_at_enemies() {
            provocations.write_batch(victims.iter().map(|&victim| Provocation(victim)));
        }
        let innate = matches!(source, Source::Innate { .. });
        let item = body
            .carried
            .as_deref()
            .and_then(|carried| item_use(source, carried, &inventory, &content.items));
        let report = Report {
            caster,
            spell,
            power,
            item: item.map(|item| item.definition.name.clone()),
            outcome,
            effects: Vec::new(),
        };
        // Paid whatever the roll: points for a learned spell, a use for an
        // item, nothing for an innate cast; then the caster is tired, more
        // under load (retail `FatigueCharacter`).
        match (source, item) {
            (Source::Spell { .. }, _) => {
                let realm = definition.realm.expect("a cast spell has a realm");
                body.spells.as_mut().unwrap().1.spend(realm, definition.cost_at(power));
            }
            (_, Some(item)) if item.from_pool => {
                inventory.pool.consume_use(item.id, item.definition)
            }
            (_, Some(item)) => {
                let keeps = item.definition.keeps_empty();
                body.carried.as_mut().unwrap().consume_use(item.id, item.definition, keeps);
            }
            _ => {}
        }
        if !innate {
            let fatigue = match item.map(|item| item.definition.use_type) {
                None => super::cast_fatigue(definition, power),
                Some(UseType::Thrown) => 10,
                Some(UseType::Instrument | UseType::Gadget) => 0,
                Some(_) => {
                    let base = rng.below(2) + 1;
                    if body.vitals.conditions.has(Condition::Diseased) { base * 2 } else { base }
                }
            };
            let slowed = body.vitals.conditions.has(Condition::Slowed);
            let hasted = body.attacks.hasted > 0.0;
            let load = body.burden.map(|burden| burden.load).unwrap_or_default();
            body.vitals.tire(load.scale_fatigue(fatigue, slowed, hasted));
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
        released.write(Release {
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

/// A utility or device cast's source, revalidated, rolled and paid at once
/// by its caster: retail counts these as having done something even when
/// they fizzle, so the caster practises whatever the roll. `underwater`
/// quenches Fire.
pub(crate) fn pay(
    caster: &mut BodyItem,
    inventory: &mut PartyInventory,
    (source, context): (Source, UseContext),
    (content, difficulty, underwater): (&Catalogs, Difficulty, bool),
    rng: &mut GameRng,
    news: &mut MessageWriter<crate::game::combat::CombatNews>,
) -> Result<(Report, Strength), Refusal> {
    let (spells, items) = (&content.spells, &content.items);
    let (Some(character), Some(carried), true) =
        (caster.character.as_deref_mut(), caster.carried.as_deref_mut(), caster.member)
    else {
        return Err(Refusal::CannotAct);
    };
    let vitals = &mut *caster.vitals;
    if !vitals.can_fight() {
        return Err(Refusal::CannotAct);
    }
    let spell = source.spell_id(carried, inventory, items).ok_or(Refusal::NoItem)?;
    let spell = &spells[spell];
    if !usable_now(spell, context) {
        return Err(Refusal::WrongTime);
    }
    let item = item_use(source, carried, inventory, items);
    let power = match (source, item) {
        (Source::Spell { power, .. }, _) => {
            if vitals.conditions.has(Condition::Silenced) {
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
    let Attempt { failure, quiet, strength } = attempt(character, item, spell, power, difficulty);
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
        Some(item) if item.from_pool => inventory.pool.consume_use(item.id, item.definition),
        Some(item) => carried.consume_use(item.id, item.definition, item.definition.keeps_empty()),
    }
    vitals.tire(super::cast_fatigue(spell, power));
    for (skill, usage) in skills {
        if character.practise(skill, usage, rng) {
            let improved =
                crate::game::combat::CombatNews::Improved { member: caster.entity, skill };
            news.write(improved);
        }
    }
    Ok((report, strength))
}

/// A sourced attempt at `power`: its failure chance, whether it is quiet (a
/// thrown item), and the strength it casts with, from a learned spell or an
/// item.
fn attempt(
    character: &Character,
    item: Option<ItemUse>,
    spell: &SpellDefinition,
    power: u32,
    difficulty: Difficulty,
) -> Attempt {
    match item {
        Some(item) => {
            let (failure, quiet) =
                item_failure(character, item.definition, spell, power, item.identified, difficulty);
            Attempt {
                failure,
                quiet,
                strength: item_strength(character, item.definition, spell, power),
            }
        }
        None => Attempt {
            failure: difficulty.scale_by_pace(super::fail_chance(character, spell, power), 0),
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
    character: &Character,
    carried: &Carried,
    items: &crate::content::items::ItemCatalog,
    definitions: &Definitions,
) -> Vec<(StackId, SpellId)> {
    carried
        .stacks()
        .filter_map(|stack| {
            let worn =
                carried.active_equipment().any(|(_, equipped)| equipped.stack.id == stack.id);
            Some((stack.id, usable_stack(character, stack, items, definitions, worn)?))
        })
        .collect()
}

/// The spell a stack casts for `character`, if they can use it now: one
/// they may activate (worn, if it must be), with a charge left, whose
/// effect the game resolves.
pub fn usable_stack(
    character: &Character,
    stack: &crate::game::inventory::ItemStack,
    items: &crate::content::items::ItemCatalog,
    definitions: &Definitions,
    worn: bool,
) -> Option<SpellId> {
    let definition = &items[stack.item];
    let cast = definition.spell?;
    (can_activate(character, definition, worn)
        && stack.remaining_uses() != Some(0)
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
