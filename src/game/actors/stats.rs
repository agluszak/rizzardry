//! What every actor is, a party member or a monster alike
//! (`docs/rules/actors.md`): the components rules read, written when the
//! actor is made and whenever it is refitted.

use bevy::prelude::*;
use enum_map::EnumMap;
use enumset::EnumSet;

use super::Place;
use super::arms::{self, Hands, Held};
use super::spells::{Bonus, Laid, Spells};
use crate::content::characters::{Attribute, BodyPart, Realm, Skill, Trait};
use crate::content::items::{AttackEffects, AttackMode, Dice};
use crate::content::species::{AttackWord, MonsterKind, Reach, Species};
use crate::content::spells::Spellbook;
use crate::game::characters::resistance::resistances;
use crate::game::characters::{Character, Condition, Conditions, Dead, Vitals, scale_by_level};
use crate::game::data::Catalogs;
use crate::game::inventory::WeaponSlot;
use crate::game::items::{Burden, Charges, Item, Items, Wears, Worn};
use crate::game::party::formation::Formation;
use crate::game::party::roster::{PartyMember, PartyRoster};
use crate::game::world::population::Monster;

/// An actor's attributes as they stand now, spells counted, each 1 to 125.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attributes(pub EnumMap<Attribute, u32>);

impl Default for Attributes {
    fn default() -> Self {
        Self(EnumMap::from_fn(|_| 1))
    }
}

impl std::ops::Index<Attribute> for Attributes {
    type Output = u32;
    fn index(&self, attribute: Attribute) -> &u32 {
        &self.0[attribute]
    }
}

/// An actor's skills as they stand now, each 0 to 125 (retail
/// `GetEffectiveSkillLevel`): the points on the sheet, a quarter more (at
/// least one) for the profession's bonus skill, and what its conditions
/// take off. A monster has none.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Skills(EnumMap<Skill, u32>);

impl std::ops::Index<Skill> for Skills {
    type Output = u32;
    fn index(&self, skill: Skill) -> &u32 {
        &self.0[skill]
    }
}

impl Skills {
    fn affected(character: &Character, conditions: &Conditions) -> Self {
        let ailing: i32 = conditions.modifiers().map(|row| row.skills).sum();
        Self(EnumMap::from_fn(|skill| {
            let points = character.skill_points[skill] as i32;
            let bonus = if character.profession.row().bonus_skill == skill {
                (points / 4).max(1)
            } else {
                0
            };
            (points + bonus + ailing).clamp(0, 125) as u32
        }))
    }

    /// A character's, by their sheet alone: before they are anyone's actor.
    pub fn of_sheet(character: &Character) -> Self {
        Self::affected(character, &Conditions::default())
    }
}

impl Attributes {
    /// From base values, with what the spells on the actor add (Superman to
    /// each, Haste to Speed) and what its conditions take off, held to
    /// 1–125 once all are counted (retail `ResetCharacterAttributes`). A
    /// member with the trait that eases a condition gets some of it back,
    /// scaled by their profession level.
    fn affected(
        base: EnumMap<Attribute, u32>,
        spells: &Bonus,
        conditions: &Conditions,
        sheet: Option<&Character>,
    ) -> Self {
        let mut ailing = EnumMap::<Attribute, i32>::default();
        for row in conditions.modifiers() {
            for &(attribute, change) in &row.attributes {
                ailing[attribute] += change;
            }
            if let (Some(eased), Some(sheet)) = (&row.eased_by, sheet)
                && sheet.has_trait(eased.ability)
            {
                ailing[eased.attribute] += sheet.scale_by_profession(eased.back) as i32;
            }
        }
        Self(EnumMap::from_fn(|a| {
            (base[a] as i32 + spells.attributes[a] as i32 + ailing[a]).clamp(1, 125) as u32
        }))
    }

    /// A character's, by their sheet alone: before they are anyone's actor.
    pub fn of_sheet(character: &Character) -> Self {
        Self::affected(character.attributes, &Bonus::default(), &Conditions::default(), None)
    }

    /// These with a point of one lost for good, as disease takes it.
    pub fn weakened(&self, attribute: Attribute) -> Self {
        let mut less = *self;
        less.0[attribute] = (less.0[attribute] - 1).max(1);
        less
    }

    /// Senses + speed + dexterity + intelligence, which weighs for an
    /// actor's blows and against those at it (retail SSDI).
    pub fn agility(&self) -> i32 {
        use Attribute::*;
        [Senses, Speed, Dexterity, Intelligence].map(|a| self[a] as i32).iter().sum()
    }
}

/// The level an actor's saves, resistance and special attacks weigh.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Level(pub u32);

/// What an actor is to the effects that land on it.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct Resistances {
    /// Percent resistance in each realm.
    pub realms: EnumMap<Realm, i32>,
    /// The conditions it cannot suffer at all (retail
    /// `g_condition_immunities`, by creature kind).
    pub immune: &'static [Condition],
    /// The conditions it shrugs off whenever a save against them is allowed
    /// (a member's traits: an Android's, the Fearless, a Psionic's mind).
    pub wards: Vec<Condition>,
    /// It cannot be slain outright.
    pub death_immune: bool,
    /// Its creature kind (undead, demon, plant, …), which holy and slaying
    /// things single out; a member has none.
    pub kind: Option<MonsterKind>,
    /// How strongly it casts from each book, which a spell turned back on
    /// it or from it grows from.
    pub caster: EnumMap<Spellbook, u32>,
}

impl Resistances {
    /// What a spell's strength from the actor grows from: its highest level
    /// among the spell's books.
    pub fn caster_level(&self, spell: &crate::content::spells::SpellDefinition) -> u32 {
        spell.books.iter().map(|book| self.caster[book]).max().unwrap_or(0)
    }
}

/// An actor's armour against blows: higher is better guarded, five off the
/// aim for each point (retail `GetTargetArmorClass`,
/// `TargetArmorClassAtLocation`, `GetTargetArmorClassModifier`).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Armour {
    /// Against being struck at all.
    pub total: i32,
    /// Where struck, by location.
    pub at: [i32; 7],
    /// What each mode finds besides.
    pub by_mode: EnumMap<AttackMode, i32>,
    /// What a surprise strips from its guard.
    pub exposed: i32,
    /// What bracing (defending) adds everywhere, and to what a surprise
    /// strips: two, and a shield's skill counting for more.
    pub braced: (i32, i32),
    /// Fatigue adds to its guard, as it does to a monster's.
    pub tires: bool,
    /// The percent cut off the damage of blows at it.
    pub reduction: i32,
    /// The percent chance what is shot or thrown at it is turned aside.
    pub missile_shield: u32,
    /// Where blows land on a monster, by its species' chances; a member
    /// has none of its own.
    pub hit_locations: Option<[u8; 7]>,
}

/// One thing an actor strikes with: one of a member's weapon slots as their gear
/// and stats make it now, or one of a monster's species' attacks. A
/// member's two slots are its two arms; a monster's attacks all belong to
/// its primary slot, which shares its attacks a round among them.
#[derive(Clone, Debug, PartialEq)]
pub struct Arm {
    pub slot: WeaponSlot,
    /// The weapon held, which an accepted attack is locked to.
    pub weapon: Option<Entity>,
    pub reach: Reach,
    pub modes: EnumSet<AttackMode>,
    /// The dice its blows roll; a punch rolls the off hand's (what it holds,
    /// else its fist), a kick the main hand's fist.
    pub dice: Dice,
    pub punch: Dice,
    pub kick: Dice,
    /// How well it strikes: a member's slot rating and five times its
    /// to-hit, a monster's attack score and five times its to-hit bonus.
    pub rating: i32,
    /// What its blows' dice are raised by, in percent.
    pub percent: i32,
    pub effects: AttackEffects,
    /// The monster kinds it slays, each a die more.
    pub slays: Vec<MonsterKind>,
    /// Attacks a round with it.
    pub attacks: u32,
    /// Its initiative, added to its body's.
    pub initiative: i32,
    /// Most swings an attack makes, before what the weapon adds.
    pub swings: u32,
    pub extra_swings: i32,
    /// Its blows are ranged combat: they train Eagle Eye and spend what is
    /// shot or thrown.
    pub ranged: bool,
    /// A launcher's: it shoots the ammunition in the other slot, which
    /// pierces by these modes.
    pub loaded: Option<EnumSet<AttackMode>>,
    /// A Ninja's throw, which gets through armour whenever it strikes.
    pub sure_throw: bool,
    /// The wielder's points in the skill whose training adds a kill chance
    /// to its blows.
    pub critical: Option<u32>,
    /// The skills a blow of it that does damage practises.
    pub practises: &'static [Skill],
    /// What swinging it weighs, for the stamina it costs; a natural attack
    /// costs none.
    pub weight: Option<u16>,
    /// Swinging it costs stamina.
    pub tires: bool,
    /// A modern weapon's: fatigue tells on its aim half as much.
    pub modern: bool,
    /// Fatigue tells on its penetration too.
    pub fatigue_pierces: bool,
    /// A sword, which a Samurai's Lightning Strike needs.
    pub sword: bool,
    /// It strikes naturally from hiding at arm's length: a Rogue's sword or
    /// dagger, a rogue kind's close attack.
    pub ambushes: bool,
    /// How a monster's attack is announced, and whether "at" follows.
    pub word: Option<(AttackWord, bool)>,
    /// Where its blows land on a member, by its species' chances.
    pub lands_on_party: Option<EnumMap<BodyPart, u8>>,
    /// How its blows sound (a row of the hit-sound table), if at all.
    pub sound_class: Option<u8>,
}

impl Arm {
    /// The dice a blow of `mode` rolls: a punch the off hand's, a kick the
    /// main hand's fist, anything else the arm's own.
    pub fn rolls(&self, mode: AttackMode) -> Dice {
        match mode {
            AttackMode::Punch => self.punch,
            AttackMode::Kick => self.kick,
            _ => self.dice,
        }
    }
}

/// What an actor strikes with, and how it fights.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Attacks {
    pub arms: Vec<Arm>,
    /// When it acts in a round, before its weapon's own initiative.
    pub initiative: i32,
    /// How well it presses a blow past a guard, or guards against one,
    /// before fatigue (retail `GetCharacterProtectionScore`,
    /// `GetMonsterProtectionScore`); fatigue tells half as much on one whose
    /// first weapon `tires_little`.
    pub press: i32,
    pub tires_little: bool,
    /// Effective while blind (a member with the trait, a Monk among
    /// monsters): its aim past 10 scales by this level instead of being
    /// held to 10, and its action is not lost to blindness.
    pub blind_fighting: Option<u8>,
    /// What its kind adds to its Senses when it looks about.
    pub keen_sight: u32,
    /// The percent chance its natural strikes roll extra dice (retail
    /// `RollBackstabExtraDice`).
    pub backstab: i32,
    /// The level of one whose blows knock out more often, by which the
    /// chance grows: a member with the trait's, in their profession.
    pub knock_out: Option<u8>,
    /// How far it may walk in one combat move, in metres; the party walks
    /// as one, so a member has none.
    pub stride: Option<f32>,
    /// The share of its stride Haste adds to a move: a tenth a power.
    pub hasted: f32,
}

impl Attacks {
    /// The furthest any of its arms reaches.
    pub fn reach(&self) -> Reach {
        self.arms.iter().map(|arm| arm.reach).max().unwrap_or(Reach::Touch)
    }

    /// The arms of a slot.
    pub fn of(&self, slot: WeaponSlot) -> impl Iterator<Item = &Arm> {
        self.arms.iter().filter(move |arm| arm.slot == slot)
    }

    /// The attacks a round the slot has: its arms' most.
    pub fn attacks(&self, slot: WeaponSlot) -> u32 {
        self.of(slot).map(|arm| arm.attacks).max().unwrap_or(0)
    }

    /// The slot's initiative: its arms' most.
    pub fn slot_initiative(&self, slot: WeaponSlot) -> i32 {
        self.of(slot).map(|arm| arm.initiative).max().unwrap_or(0)
    }

    /// The slot an attack opens with: the one with the most attacks, then
    /// the bigger initiative (retail `ChooseCharacterAttackHand`).
    pub fn opening(&self) -> Option<WeaponSlot> {
        [WeaponSlot::Primary, WeaponSlot::Secondary]
            .into_iter()
            .filter(|&slot| self.attacks(slot) > 0)
            .reduce(|best, slot| {
                let rank = |slot| (self.attacks(slot), self.slot_initiative(slot));
                // The main hand keeps it on a tie.
                if rank(slot) > rank(best) { slot } else { best }
            })
    }

    /// The weapons its slots hold.
    pub fn weapons(&self) -> EnumMap<WeaponSlot, Option<Entity>> {
        EnumMap::from_fn(|slot| self.of(slot).find_map(|arm| arm.weapon))
    }
}

/// Whether an actor reaches another with `reach` (retail `CloseFormationGap`,
/// `FrontRankScreens`): a member and someone outside the party by the
/// formation, which shortens touch and short reach by the occupied rows
/// between; two members unless the center screens a touch blow; anyone
/// else by the gap between their bodies.
pub fn reaches(
    formation: &Formation,
    (a, from): (Entity, &Place),
    (b, to): (Entity, &Place),
    reach: Reach,
) -> bool {
    let seated = |body| formation.row_of(body).is_some();
    let through_formation = |member, seat: &Place, other: &Place| {
        let side = seat.quadrant(other.feet);
        formation.reach(member, side, reach).is_some_and(|r| seat.gap(other) <= r.metres())
    };
    match (seated(a), seated(b)) {
        (true, true) => a == b || reach > Reach::Touch || !formation.screens(a, b),
        (true, false) => through_formation(a, from, to),
        (false, true) => through_formation(b, to, from),
        (false, false) => from.gap(to) <= reach.metres(),
    }
}

/// An actor whose figures no longer follow their sources
/// (`docs/rules/actors.md`): it was made, joined or left a party, died or
/// was raised, or its sheet, what ails it, its gear, load or spells changed.
/// The small systems below see such a change and mark the actor; nothing
/// else says so. [`refit`] writes the figures anew and takes the mark off.
#[derive(Component, Clone, Copy, Debug, Default)]
pub(super) struct Stale;

/// An actor's components have been written anew: said once a refit is done,
/// for what follows from its figures (a member's seat, what they can still
/// wear).
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Refitted {
    pub entity: Entity,
}

/// What an actor has of its own that its figures are made from has changed.
#[allow(clippy::type_complexity)]
pub(super) fn notice_changed_sources(
    actors: Query<
        Entity,
        (
            With<Vitals>,
            Or<(
                Changed<Character>,
                Changed<Monster>,
                Changed<Conditions>,
                Changed<Burden>,
                Changed<PartyMember>,
                Changed<Wears>,
                Changed<Spells>,
                Changed<Dead>,
            )>,
        ),
    >,
    mut commands: Commands,
) {
    for actor in &actors {
        commands.entity(actor).insert(Stale);
    }
}

/// The spells on a party are in every member's figures.
pub(super) fn notice_party_spells(
    parties: Query<&PartyRoster, Changed<Spells>>,
    mut commands: Commands,
) {
    for member in parties.iter().flat_map(|roster| roster.iter()) {
        commands.entity(member).try_insert(Stale);
    }
}

/// A worn thing that is itself no longer what it was (out of shots, grown
/// into a better one) changes its wearer's figures.
pub(super) fn notice_changed_gear(
    gear: Query<&Worn, Or<(Changed<Item>, Changed<Charges>)>>,
    mut commands: Commands,
) {
    for worn in &gear {
        commands.entity(worn.by).try_insert(Stale);
    }
}

/// A source going altogether (the last thing worn, life among the dead, a
/// place in the party) changes the figures as its coming did.
pub(super) fn stale_without<T: Component>(gone: On<Remove<T>>, mut commands: Commands) {
    commands.entity(gone.entity).try_insert(Stale);
}

/// The last spell going from a bearer; from a party, from every member.
pub(super) fn stale_without_spells(
    gone: On<Remove<Spells>>,
    rosters: Query<&PartyRoster>,
    mut commands: Commands,
) {
    match rosters.get(gone.entity) {
        Ok(roster) => roster.iter().for_each(|member| {
            commands.entity(member).try_insert(Stale);
        }),
        Err(_) => {
            commands.entity(gone.entity).try_insert(Stale);
        }
    }
}

/// Write a stale actor's components from its sources: a monster's species,
/// or a member's sheet and what they wear, with the spells on it and, for a
/// member, on the party.
#[allow(clippy::type_complexity)]
pub(super) fn refit(
    mut actors: Query<
        (
            Entity,
            (&mut Vitals, &Conditions),
            AnyOf<((&Character, Option<&Burden>), &Monster)>,
            Option<&PartyMember>,
            (&mut Attributes, &mut Skills, &mut Level, &mut Resistances, &mut Armour, &mut Attacks),
        ),
        With<Stale>,
    >,
    (items, hands): (Items, Query<Held>),
    laid: Laid,
    content: Option<Res<Catalogs>>,
    mut commands: Commands,
) {
    let Some(content) = content else { return };
    for (entity, (mut vitals, conditions), (sheet, monster), member, written) in &mut actors {
        // A member counts the spells on them and on their party together.
        let mut spells = laid.bonus(entity);
        if let Some(member) = member {
            spells += laid.bonus(member.0);
        }
        let skills = sheet.map(|(character, _)| Skills::affected(character, conditions));
        let skills = skills.unwrap_or_default();
        let (attributes, level, resistances, armour, attacks) = match (sheet, monster) {
            (Some((character, burden)), _) => {
                let load = burden.map(|burden| burden.load).unwrap_or_default();
                let made = of_member(
                    character,
                    &skills,
                    (&items, &hands, entity),
                    load,
                    &spells,
                    (&vitals, conditions),
                );
                let (health, stamina) = (character.max_hp(&made.0), character.max_stamina(&made.0));
                if (vitals.health.max, vitals.stamina.max) != (health, stamina) {
                    vitals.refit_maxima(health, stamina);
                }
                made
            }
            (None, Some(monster)) => {
                of_species(&content.species[monster.species], &spells, conditions)
            }
            (None, None) => continue,
        };
        let (mut a, mut s, mut l, mut r, mut ar, mut at) = written;
        s.set_if_neq(skills);
        a.set_if_neq(attributes);
        l.set_if_neq(level);
        r.set_if_neq(resistances);
        ar.set_if_neq(armour);
        at.set_if_neq(attacks);
        commands.entity(entity).remove::<Stale>();
        commands.trigger(Refitted { entity });
    }
}

/// A monster, from its species record and the spells on it.
fn of_species(
    kind: &Species,
    spells: &Bonus,
    conditions: &Conditions,
) -> (Attributes, Level, Resistances, Armour, Attacks) {
    let base = EnumMap::from_fn(|attribute| {
        u32::from(match attribute {
            Attribute::Strength => kind.attributes.strength,
            Attribute::Intelligence => kind.attributes.intelligence,
            Attribute::Dexterity => kind.attributes.dexterity,
            Attribute::Speed => kind.attributes.speed,
            Attribute::Senses => kind.attributes.senses,
            Attribute::Piety | Attribute::Vitality => 1,
        })
    });
    let attributes = Attributes::affected(base, spells, conditions, None);
    let resistances = Resistances {
        realms: EnumMap::from_fn(|realm| {
            (i32::from(kind.resistances[realm]) + spells.resistance[realm]).min(100)
        }),
        immune: crate::game::hits::defences::immune(kind.kind),
        wards: Vec::new(),
        death_immune: kind.death_immune,
        kind: Some(kind.kind),
        caster: EnumMap::from_fn(|_| u32::from(kind.level)),
    };
    let laid = spells.armour + conditions.armour();
    let armour = Armour {
        total: 10 - i32::from(kind.evasion) + laid,
        at: std::array::from_fn(|place| 10 + laid - i32::from(kind.armour_by_location[place])),
        by_mode: EnumMap::from_fn(|mode| i32::from(kind.armour_by_mode[mode])),
        exposed: 2 * i32::from(kind.vulnerable_from_behind),
        braced: (0, 0),
        tires: true,
        reduction: (i32::from(kind.damage_reduction) + spells.reduction).min(100),
        missile_shield: spells.missiles,
        hit_locations: Some(kind.hit_locations),
    };
    let hit_bonus = spells.to_hit + conditions.to_hit();
    let first = kind.attacks.first();
    let attacks = Attacks {
        arms: kind.attacks.iter().map(|swung| monster_arm(kind, swung, hit_bonus)).collect(),
        initiative: i32::from(kind.initiative),
        press: attributes[Attribute::Speed] as i32
            + first.map_or(0, |a| i32::from(a.score))
            + hit_bonus * 5,
        tires_little: first.is_some_and(crate::content::species::MonsterAttack::tires_little),
        blind_fighting: (kind.kind == MonsterKind::Monk).then_some(kind.level),
        keen_sight: if kind.level_sight { (u32::from(kind.level) * 5).min(125) } else { 0 },
        backstab: scale_by_level(kind.level, 100.0) as i32,
        knock_out: None,
        stride: Some(
            kind.combat_move_range * crate::game::combat::monster::close::METRES_PER_MOVE_RANGE,
        ),
        hasted: spells.stride,
    };
    (attributes, Level(u32::from(kind.level)), resistances, armour, attacks)
}

/// A character, from their sheet and what they wear under this load, with
/// the spells on them and, for a party member, the party's.
fn of_member(
    character: &Character,
    skills: &Skills,
    (held, hands, member): (&Items, &Query<Held>, Entity),
    load: crate::game::characters::Load,
    spells: &Bonus,
    (vitals, conditions): (&Vitals, &Conditions),
) -> (Attributes, Level, Resistances, Armour, Attacks) {
    let protection = |slot| held.protection(member, slot);
    let attributes =
        Attributes::affected(character.attributes, spells, conditions, Some(character));
    let a = &attributes;
    let sheet = resistances(character, a, skills, protection);
    let mut wards = Vec::new();
    if character.has_trait(Trait::AndroidConditionImmunity) {
        use Condition::*;
        wards.extend([Afraid, Diseased, Irritated, Nauseated, Poisoned, Asleep]);
    }
    if character.has_trait(Trait::Fearless) {
        wards.push(Condition::Afraid);
    }
    if character.has_trait(Trait::MentalConditionImmunity) {
        wards.extend([Condition::Insane, Condition::Turncoat]);
    }
    let resistances = Resistances {
        realms: EnumMap::from_fn(|realm| (sheet[realm].total + spells.resistance[realm]).min(100)),
        immune: &[],
        wards,
        death_immune: false,
        kind: None,
        caster: EnumMap::from_fn(|book| {
            crate::game::magic::total_caster_level(character, EnumSet::only(book)).max(0) as u32
        }),
    };
    // The dead and the missing guard themselves with nothing: only what
    // they wear still counts (retail `CalcArmorClasses`).
    let lively = !vitals.is_dead() && conditions.below(Condition::Missing);
    let guard = |defending| {
        let mut armour = if lively {
            arms::armour(character, skills, a, protection, load, defending)
        } else {
            arms::worn_armour(protection)
        };
        let laid = if lively { spells.armour + conditions.armour() } else { 0 };
        // One kept from their place is guarded no better than -5 overall,
        // before what covers each place is added.
        let whole = armour.total + laid;
        let held = if conditions.unseats(character.traits()) { whole.min(-5) } else { whole };
        armour.by_location.values_mut().for_each(|at| *at += held - armour.total);
        armour.total = held;
        armour
    };
    let (standing, bracing) = (guard(false), guard(true));
    let exposed = |armour: &arms::Armour| armour.speed.max(0) + armour.shield.max(0);
    let armour = Armour {
        total: standing.total,
        at: std::array::from_fn(|place| {
            standing.by_location.values().nth(place).copied().unwrap_or(standing.total)
        }),
        by_mode: EnumMap::default(),
        exposed: exposed(&standing),
        braced: (bracing.total - standing.total, exposed(&bracing) - exposed(&standing)),
        tires: false,
        reduction: (character.damage_reduction(a, skills) + spells.reduction).min(100),
        missile_shield: spells.missiles,
        hit_locations: None,
    };
    let hands = Hands::of(|slot| held.worn(member, slot).and_then(|item| hands.get(item).ok()));
    let attacks = Attacks {
        initiative: character.initiative(a, load, skills),
        blind_fighting: character
            .has_trait(Trait::EffectiveWhileBlind)
            .then(|| character.profession_levels[character.profession]),
        keen_sight: 0,
        backstab: character.scale_by_profession(100.0) as i32,
        knock_out: character
            .has_trait(Trait::Knockout)
            .then(|| character.profession_levels[character.profession]),
        stride: None,
        hasted: spells.stride,
        ..hands.attacks(character, skills, a, load, spells.to_hit + conditions.to_hit())
    };
    (attributes, Level(u32::from(character.level())), resistances, armour, attacks)
}

/// A monster's arm: one of its species' attacks.
fn monster_arm(
    kind: &Species,
    swung: &crate::content::species::MonsterAttack,
    hit_bonus: i32,
) -> Arm {
    Arm {
        slot: WeaponSlot::Primary,
        weapon: None,
        reach: swung.reach,
        modes: swung.modes,
        dice: swung.damage,
        punch: swung.damage,
        kick: swung.damage,
        rating: i32::from(swung.score) + 5 * hit_bonus,
        percent: 0,
        effects: swung.effects,
        slays: Vec::new(),
        attacks: u32::from(kind.attacks_per_round),
        initiative: 0,
        swings: u32::from(kind.swings_per_round.max(1)),
        extra_swings: 0,
        ranged: false,
        loaded: None,
        sure_throw: false,
        critical: None,
        practises: &[],
        weight: None,
        tires: false,
        modern: swung.tires_little(),
        fatigue_pierces: swung.tires_little(),
        sword: false,
        ambushes: kind.kind == MonsterKind::Rogue && swung.reach <= Reach::Touch,
        word: Some((swung.word, swung.named())),
        lands_on_party: Some(kind.hits_on_party),
        sound_class: swung.sound_class,
    }
}
