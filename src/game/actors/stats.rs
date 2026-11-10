//! What every actor is, a party member or a monster alike
//! (`docs/rules/actors.md`): the components rules read, written when the
//! actor is made and whenever it is refitted.

use bevy::prelude::*;
use enum_map::EnumMap;
use enumset::EnumSet;

use super::Place;
use super::arms::{self, Hands, Held};
use super::spells::{Bonus, Laid};
use crate::content::characters::{Attribute, BodyPart, Realm, Skill, Trait};
use crate::content::items::{AttackEffects, AttackMode, Dice};
use crate::content::species::{AttackWord, MonsterKind, Reach, Species};
use crate::content::spells::Spellbook;
use crate::game::characters::resistance::resistances;
use crate::game::characters::{Character, Condition, Vitals, scale_by_level};
use crate::game::data::Catalogs;
use crate::game::inventory::WeaponSlot;
use crate::game::items::{Burden, Items, Reweigh};
use crate::game::party::formation::Formation;
use crate::game::party::roster::PartyMember;
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

impl Attributes {
    /// From base values, with what the spells on the actor add (Superman to
    /// each, Haste to Speed).
    fn with_spells(base: EnumMap<Attribute, u32>, spells: &Bonus) -> Self {
        Self(EnumMap::from_fn(|a| (base[a].clamp(1, 125) + spells.attributes[a]).min(125)))
    }

    /// A character's, by their sheet alone: before they are anyone's actor.
    pub fn of_sheet(character: &Character) -> Self {
        Self::with_spells(character.attributes, &Bonus::default())
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
    /// The skill whose training adds a kill chance to its blows.
    pub critical_skill: Option<Skill>,
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

/// An actor's sources changed (`docs/rules/actors.md`): it was made, joined
/// or left the party, or its gear, sheet, load or spells changed. Its
/// components are written anew.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Refit {
    pub entity: Entity,
}

/// An actor is refitted as it is made, and as it joins or leaves the party.
pub(super) fn refit_made<T: Component>(made: On<Add<T>>, mut commands: Commands) {
    commands.trigger(Refit { entity: made.entity });
}

pub(super) fn refit_joined(joined: On<Insert<PartyMember>>, mut commands: Commands) {
    commands.trigger(Refit { entity: joined.entity });
    commands.trigger(Reweigh);
}

pub(super) fn refit_left(left: On<Remove<PartyMember>>, mut commands: Commands) {
    commands.trigger(Refit { entity: left.entity });
    commands.trigger(Reweigh);
}

/// Write an actor's components from its sources: a monster's species, or a
/// member's sheet and what they wear, with the spells on it and, for a
/// member, on the party. Fired at a party, it refits every member.
#[allow(clippy::type_complexity)]
pub(super) fn refit(
    event: On<Refit>,
    mut actors: Query<(
        &mut Vitals,
        AnyOf<((&Character, Option<&Burden>), &Monster)>,
        Option<&PartyMember>,
        (&mut Attributes, &mut Level, &mut Resistances, &mut Armour, &mut Attacks),
    )>,
    rosters: Query<&crate::game::party::roster::PartyRoster>,
    (items, hands): (Items, Query<Held>),
    laid: Laid,
    content: Option<Res<Catalogs>>,
    mut commands: Commands,
) {
    let Some(content) = content else { return };
    if let Ok(roster) = rosters.get(event.entity) {
        roster.iter().for_each(|entity| commands.trigger(Refit { entity }));
        return;
    }
    let Ok((mut vitals, (sheet, monster), member, written)) = actors.get_mut(event.entity) else {
        return;
    };
    // A member counts the spells on them and on their party together.
    let mut spells = laid.bonus(event.entity);
    if let Some(member) = member {
        spells += laid.bonus(member.0);
    }
    let (attributes, level, resistances, armour, attacks) = match (sheet, monster) {
        (Some((character, burden)), _) => {
            let load = burden.map(|burden| burden.load).unwrap_or_default();
            let made = of_member(character, (&items, &hands, event.entity), load, &spells);
            let (health, stamina) = (character.max_hp(&made.0), character.max_stamina(&made.0));
            if (vitals.health.max, vitals.stamina.max) != (health, stamina) {
                vitals.refit_maxima(health, stamina);
            }
            made
        }
        (None, Some(monster)) => of_species(&content.species[monster.species], &spells),
        (None, None) => return,
    };
    let (mut a, mut l, mut r, mut ar, mut at) = written;
    // What a member can carry follows their strength.
    if a.set_if_neq(attributes) && member.is_some() {
        commands.trigger(Reweigh);
    }
    l.set_if_neq(level);
    r.set_if_neq(resistances);
    ar.set_if_neq(armour);
    at.set_if_neq(attacks);
}

/// A monster, from its species record and the spells on it.
fn of_species(kind: &Species, spells: &Bonus) -> (Attributes, Level, Resistances, Armour, Attacks) {
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
    let attributes = Attributes::with_spells(base, spells);
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
    let laid = spells.armour;
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
    let hit_bonus = spells.to_hit;
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
        stride: Some(kind.combat_move_range * crate::game::combat::approach::METRES_PER_MOVE_RANGE),
        hasted: spells.stride,
    };
    (attributes, Level(u32::from(kind.level)), resistances, armour, attacks)
}

/// A character, from their sheet and what they wear under this load, with
/// the spells on them and, for a party member, the party's.
fn of_member(
    character: &Character,
    (held, hands, member): (&Items, &Query<Held>, Entity),
    load: crate::game::characters::Load,
    spells: &Bonus,
) -> (Attributes, Level, Resistances, Armour, Attacks) {
    let protection = |slot| held.protection(member, slot);
    let attributes = Attributes::with_spells(character.attributes, spells);
    let a = &attributes;
    let sheet = resistances(character, a, protection);
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
    let laid = spells.armour;
    let standing = arms::armour(character, a, protection, load, false);
    let bracing = arms::armour(character, a, protection, load, true);
    let exposed = |armour: &arms::Armour| armour.speed.max(0) + armour.shield.max(0);
    let armour = Armour {
        total: standing.total + laid,
        at: std::array::from_fn(|place| {
            standing.by_location.values().nth(place).copied().unwrap_or(standing.total) + laid
        }),
        by_mode: EnumMap::default(),
        exposed: exposed(&standing),
        braced: (bracing.total - standing.total, exposed(&bracing) - exposed(&standing)),
        tires: false,
        reduction: (character.damage_reduction(a) + spells.reduction).min(100),
        missile_shield: spells.missiles,
        hit_locations: None,
    };
    let hands = Hands::of(|slot| held.worn(member, slot).and_then(|item| hands.get(item).ok()));
    let attacks = Attacks {
        initiative: character.initiative(a, load),
        blind_fighting: character
            .has_trait(Trait::EffectiveWhileBlind)
            .then(|| character.profession_levels[character.profession]),
        keen_sight: 0,
        backstab: character.scale_by_profession(100.0) as i32,
        stride: None,
        hasted: spells.stride,
        ..hands.attacks(character, a, load, spells.to_hit)
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
        critical_skill: None,
        weight: None,
        tires: false,
        modern: swung.tires_little(),
        fatigue_pierces: swung.tires_little(),
        sword: false,
        ambushes: kind.kind == MonsterKind::Rogue && swung.reach <= Reach::Touch,
        word: Some((swung.word, swung.named())),
        lands_on_party: Some(kind.hits_on_party),
    }
}
