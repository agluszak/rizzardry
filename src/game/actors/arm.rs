//! What an actor strikes with (`docs/rules/arms.md`): its arms, each an
//! entity related to it. A monster's are its species' natural attacks,
//! made with it and fixed. A member's are their two hands, and each fact
//! of a hand is a component written by one small system from what the hand
//! holds and the member's training (retail `CalcAttacks`).
//!
//! The systems run over every hand each time bodies settle and write only
//! what changed; a party has a dozen hands.

use bevy::ecs::entity_disabling::Disabled;
use bevy::ecs::query::Allow;
use bevy::prelude::*;
use enum_map::EnumMap;
use enumset::EnumSet;

use super::{Attributes, Skills, Traits};
use crate::content::characters::{Attribute, BodyPart, Skill, Trait};
use crate::content::items::{AttackEffect, AttackEffects, AttackMode, Dice};
use crate::content::species::{AttackWord, MonsterKind, Reach};
use crate::game::characters::{Calling, skill_available};
use crate::game::inventory::WeaponSlot;
use crate::game::items::{
    Burden, Charges, Fit, Items, Launcher, Missile, Strength, Weapon, Weight, partners,
};

/// The sound class of a blow with nothing in hand (retail's class 9).
const FIST_SOUND: u8 = 9;

/// An arm of an actor.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = Arms)]
pub struct ArmOf(pub Entity);

/// What an actor strikes with, in the order its arms were made.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = ArmOf, linked_spawn)]
#[require(Wielding, Tempo, Furthest, GuardReach)]
pub struct Arms(Vec<Entity>);

impl Arms {
    pub fn entities(&self) -> &[Entity] {
        &self.0
    }
}

/// The hand it is; a monster's natural attacks are all its main hand's.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hand(pub WeaponSlot);

/// What a member's hand fights with now: a weapon (with the ammunition the
/// other hand holds for it, if any), its fist, or nothing.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[require(
    Reaches,
    Modes,
    Rolls,
    BlowEffects,
    Slays,
    Rating,
    ToHit,
    DamagePercent,
    Handling,
    AttacksARound,
    ArmInitiative,
    Swings
)]
pub enum Grip {
    /// A shield, a missile, a launcher without its missile, a gun out of
    /// shots, anything else: it does not strike.
    #[default]
    Idle,
    /// Empty, and fighting with Martial Arts.
    Fist,
    Weapon {
        item: Entity,
        ammunition: Option<Entity>,
    },
}

impl Grip {
    pub fn strikes(self) -> bool {
        self != Grip::Idle
    }

    /// The weapon held, which an accepted attack is locked to.
    pub fn weapon(self) -> Option<Entity> {
        match self {
            Grip::Weapon { item, .. } => Some(item),
            _ => None,
        }
    }
}

/// How far it reaches.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reaches(pub Reach);

/// The ways it strikes.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modes(pub EnumSet<AttackMode>);

/// The dice its blows roll; a punch rolls the off hand's (what it holds,
/// else its fist), a kick the main hand's fist.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rolls {
    pub dice: Dice,
    pub punch: Dice,
    pub kick: Dice,
}

impl Rolls {
    pub fn of(&self, mode: AttackMode) -> Dice {
        match mode {
            AttackMode::Punch => self.punch,
            AttackMode::Kick => self.kick,
            _ => self.dice,
        }
    }
}

/// What its blows may do besides damage.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlowEffects(pub AttackEffects);

/// The monster kinds it slays, each a die more.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct Slays(pub Vec<MonsterKind>);

/// How well it strikes: a member's hand rating with five times its to-hit, a
/// monster's attack score. What spells and conditions add is the bearer's
/// own ([`super::AimBonus`]).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rating(pub i32);

/// What its blows' dice are raised by, in percent.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DamagePercent(pub i32);

/// Attacks a round with it.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttacksARound(pub u32);

/// Its initiative, added to its bearer's.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArmInitiative(pub i32);

/// Most swings an attack with it makes, before what its weapon adds (which
/// may be negative).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Swings {
    pub most: u32,
    pub extra: i32,
}

/// The ways of a launcher's missile, from which the mode that pierces is
/// drawn.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loaded(pub EnumSet<AttackMode>);

/// Training that adds a kill chance to its blows: the skill's points.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Critical(pub u32);

/// The skills a blow of it that does damage practises.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trains(pub &'static [Skill]);

/// What swinging it weighs, for the stamina it costs.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Heft(pub u16);

/// Swinging it costs stamina.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Tires;

/// A modern weapon's: fatigue tells on its aim half as much.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Modern;

/// Fatigue tells on its penetration too.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct FatiguePierces;

/// A sword, which a Samurai's Lightning Strike needs.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Sword;

/// It strikes naturally from hiding at arm's length: a Rogue's sword or
/// dagger, a rogue kind's close attack.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Ambushes;

/// A Ninja's throw: it always gets through armour when it strikes.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct SureThrow;

/// How a monster's attack is announced, and whether "at" follows.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Word(pub AttackWord, pub bool);

/// Where its blows land on a member, by its species' chances.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LandsOnParty(pub EnumMap<BodyPart, u8>);

/// How its blows sound (a row of the hit-sound table).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoundClass(pub u8);

/// Unarmed damage by Martial Arts level / 11 (retail
/// `g_unarmed_damage_dice`).
fn unarmed_dice(martial_arts: u32) -> Dice {
    const DICE: [(i32, i32, i32); 12] = [
        (0, 1, 2),
        (0, 1, 3),
        (0, 2, 2),
        (0, 2, 3),
        (0, 2, 4),
        (0, 3, 3),
        (1, 3, 3),
        (2, 3, 3),
        (0, 3, 5),
        (0, 4, 4),
        (2, 4, 4),
        (4, 4, 4),
    ];
    let (base, count, sides) = DICE[(martial_arts as usize / 11).min(11)];
    Dice { base, count, sides }
}

/// A member has two hands. They are their bearer's even while the bearer
/// is set aside (a companion out of the party), so the rules of a hand read
/// their bearer as it is, set aside or not.
fn grow_hands(added: On<Add<Calling>>, mut commands: Commands) {
    for slot in [WeaponSlot::Primary, WeaponSlot::Secondary] {
        commands.spawn((ArmOf(added.entity), Hand(slot), Grip::default(), Tires));
    }
}

/// What each of a member's hands fights with, from what they hold in each
/// (retail `CalcAttacks`). An empty hand fights with its fist; a ranged
/// weapon needs its missile in the other hand if it is a launcher, and
/// shots if it is a gun. The off hand strikes only beside a main hand of
/// the same kind (both close, or both at range), never when the main hand
/// holds an omnigun, and an empty off hand does not strike beside an armed
/// main hand. The ammunition a hand's weapon has is a missile in the other
/// hand that is not itself a weapon, or what an omnigun in the main hand
/// fires.
#[allow(clippy::type_complexity)]
fn grip(
    mut hands: Query<(&ArmOf, &Hand, &mut Grip)>,
    held: Items,
    items: Query<(
        Entity,
        Fit,
        Option<&Weapon>,
        Option<&Launcher>,
        Option<&Missile>,
        Option<&Charges>,
    )>,
) -> Result {
    for (&ArmOf(bearer), &Hand(slot), mut grip) in &mut hands {
        let get = |slot: WeaponSlot| {
            held.worn(bearer, slot.equip()).map(|item| items.get(item)).transpose()
        };
        let (main, off) = (get(WeaponSlot::Primary)?, get(WeaponSlot::Secondary)?);
        // Close or ranged combat, or neither for what cannot strike.
        let combat = |weapon: Option<Option<&Weapon>>| match weapon {
            None => Some(false),
            Some(weapon) => weapon.map(|weapon| weapon.ranged),
        };
        let omnigun = main.as_ref().is_some_and(|main| main.3.is_some_and(|l| l.fires_weapons));
        let off_idle = combat(main.as_ref().map(|main| main.2))
            != combat(off.as_ref().map(|off| off.2))
            || omnigun
            || (off.is_none() && main.is_some());
        let (mine, other) = match slot {
            WeaponSlot::Primary => (&main, &off),
            WeaponSlot::Secondary => (&off, &main),
        };
        let now = match mine {
            _ if slot == WeaponSlot::Secondary && off_idle => Grip::Idle,
            None => Grip::Fist,
            Some((item, fit, weapon, launcher, _, charges)) => match weapon {
                None => Grip::Idle,
                Some(weapon) => {
                    let fires = launcher.is_some_and(|l| l.fires_weapons);
                    let ammunition = other.as_ref().filter(|other| {
                        other.4.is_some()
                            && (other.2.is_none() || (fires && slot == WeaponSlot::Primary))
                    });
                    let loaded = launcher
                        .is_none_or(|_| ammunition.is_some_and(|other| partners(fit, &other.1)));
                    let shots = !(weapon.shots && charges.is_some_and(|c| c.left == 0));
                    if weapon.ranged && !(loaded && shots) {
                        Grip::Idle
                    } else {
                        Grip::Weapon { item: *item, ammunition: ammunition.map(|other| other.0) }
                    }
                }
            },
        };
        grip.set_if_neq(now);
    }
    Ok(())
}

/// The weapon a hand grips, if any. A grip is set from what the hand holds
/// just before the hand's facts are written, so its weapon is there.
fn weapon_of<'a>(grip: Grip, weapons: &'a Query<&Weapon>) -> Result<Option<&'a Weapon>> {
    Ok(grip.weapon().map(|item| weapons.get(item)).transpose()?)
}

/// The skill a hand fights with: its weapon's, Martial Arts otherwise.
fn skill_of(weapon: Option<&Weapon>) -> Skill {
    weapon.map_or(Skill::MartialArts, |w| w.skill)
}

/// Its blows are ranged combat, not close combat.
fn ranged(weapon: Option<&Weapon>) -> bool {
    weapon.is_some_and(|w| w.ranged)
}

/// What a launcher shoots: the ammunition in the other hand, which the grip
/// took as a missile.
fn shot<'a>(
    grip: Grip,
    launchers: &Query<&Launcher>,
    missiles: &'a Query<&Missile>,
) -> Result<Option<&'a Missile>> {
    Ok(match grip {
        Grip::Weapon { item, ammunition: Some(ammunition) } if launchers.contains(item) => {
            Some(missiles.get(ammunition)?)
        }
        _ => None,
    })
}

/// A hand's reach: its weapon's, a fist's touch.
fn reach(mut hands: Query<(&Grip, &mut Reaches)>, weapons: Query<&Weapon>) -> Result {
    for (&grip, mut reaches) in &mut hands {
        let reach = weapon_of(grip, &weapons)?.map_or(Reach::Touch, |w| w.reach);
        reaches.set_if_neq(Reaches(reach));
    }
    Ok(())
}

/// The ways a hand strikes: its weapon's, or a fist's punch, and a kick past
/// Martial Arts 4.
fn modes(
    mut hands: Query<(&ArmOf, &Grip, &mut Modes)>,
    bearers: Query<&Skills, Allow<Disabled>>,
    weapons: Query<&Weapon>,
) -> Result {
    for (&ArmOf(bearer), &grip, mut modes) in &mut hands {
        let martial = bearers.get(bearer)?[Skill::MartialArts];
        let now = match weapon_of(grip, &weapons)? {
            Some(weapon) => weapon.modes,
            None if martial > 4 => AttackMode::Punch | AttackMode::Kick,
            None => EnumSet::only(AttackMode::Punch),
        };
        modes.set_if_neq(Modes(now));
    }
    Ok(())
}

/// The dice of a hand: what a launcher shoots, a weapon's own, or a fist by
/// Martial Arts, two more in the main hand. A punch rolls what the off hand
/// holds as its record gives them (none for a shield), else the off fist's;
/// a kick the main fist's (retail `ResolveCharacterAttackDamage`).
#[allow(clippy::type_complexity)]
fn rolls(
    mut hands: Query<(&ArmOf, &Hand, &Grip, &mut Rolls)>,
    bearers: Query<&Skills, Allow<Disabled>>,
    held: Items,
    (weapons, launchers, missiles): (Query<&Weapon>, Query<&Launcher>, Query<&Missile>),
    strikers: Query<(Option<&Weapon>, Option<&Missile>)>,
) -> Result {
    for (&ArmOf(bearer), &Hand(slot), &grip, mut rolls) in &mut hands {
        let martial = bearers.get(bearer)?[Skill::MartialArts];
        let fist = |slot: WeaponSlot| {
            let mut dice = unarmed_dice(martial);
            if slot == WeaponSlot::Primary {
                dice.base += 2;
            }
            dice
        };
        let dice = match grip {
            Grip::Fist => fist(slot),
            Grip::Weapon { item, .. } => match shot(grip, &launchers, &missiles)? {
                Some(missile) => missile.dice,
                None => weapons.get(item)?.dice,
            },
            Grip::Idle => Dice::default(),
        };
        // What the off hand holds punches as its record gives: a weapon's
        // dice, a missile's, none for anything else.
        let punch = match held.worn(bearer, WeaponSlot::Secondary.equip()) {
            None => fist(WeaponSlot::Secondary),
            Some(item) => match strikers.get(item)? {
                (Some(weapon), _) => weapon.dice,
                (None, Some(missile)) => missile.dice,
                (None, None) => Dice::default(),
            },
        };
        rolls.set_if_neq(Rolls { dice, punch, kick: fist(WeaponSlot::Primary) });
    }
    Ok(())
}

/// What a hand's blows may do besides damage: the weapon's effects with its
/// ammunition's, or a strong fist's knockout chance; and the monster kinds
/// its weapon and ammunition slay.
#[allow(clippy::type_complexity)]
fn effects(
    mut hands: Query<(&ArmOf, &Grip, &mut BlowEffects, &mut Slays)>,
    bearers: Query<&Attributes, Allow<Disabled>>,
    (weapons, launchers, missiles): (Query<&Weapon>, Query<&Launcher>, Query<&Missile>),
) -> Result {
    for (&ArmOf(bearer), &grip, mut effects, mut slays) in &mut hands {
        let weapon = weapon_of(grip, &weapons)?;
        let missile = shot(grip, &launchers, &missiles)?;
        let now = match weapon {
            Some(weapon) => missile.map_or(weapon.effects, |m| weapon.effects.with(&m.effects)),
            None => {
                let mut fist = AttackEffects::default();
                let strength = bearers.get(bearer)?[Attribute::Strength] as i32;
                if strength > 49 {
                    fist.chances[AttackEffect::KnockOut] = ((strength - 50) / 5) as u32;
                }
                fist
            }
        };
        effects.set_if_neq(BlowEffects(now));
        let slain = [weapon.and_then(|w| w.slays), missile.and_then(|m| m.slays)];
        slays.set_if_neq(Slays(slain.into_iter().flatten().collect()));
    }
    Ok(())
}

/// How a member fights with a hand, from the hand and the other one
/// (retail `CalcAttacks`): the combat and weapon skills combined, weighed
/// with the other hand's weapon skill when it strikes with a weapon too and
/// Dual Weapons when both do; what wielding two weapons costs (10 a hand and
/// a quarter of the Dual Weapons lacking); and modern weapons feeling load
/// half as much.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Handling {
    pub skill: i32,
    pub dual_penalty: i32,
}

fn handling(
    mut hands: Query<(&ArmOf, &Hand, &Grip, &mut Handling)>,
    grips: Query<(&Hand, &Grip)>,
    bearers: Query<(&Skills, &Arms), Allow<Disabled>>,
    weapons: Query<&Weapon>,
) -> Result {
    for (&ArmOf(bearer), &Hand(slot), &grip, mut handling) in &mut hands {
        let (skills, arms) = bearers.get(bearer)?;
        let other = other_hand(arms, slot, &grips)?;
        let dual = grip.weapon().is_some() && other.weapon().is_some();
        let weapon = weapon_of(grip, &weapons)?;
        let combat = if ranged(weapon) { Skill::RangedCombat } else { Skill::CloseCombat };
        let mut score = ((skills[combat] + skills[skill_of(weapon)] * 2) * 2 / 3) as i32;
        let mut divisor = 20;
        if let Some(theirs) = weapon_of(other, &weapons)? {
            score += skills[theirs.skill] as i32 >> 1;
            divisor = 25;
        }
        if dual {
            score += skills[Skill::DualWeapons] as i32;
            divisor += 10;
        }
        let number = if slot == WeaponSlot::Primary { 1 } else { 2 };
        let dual_penalty =
            if dual { -10 * number - (100 - skills[Skill::DualWeapons] as i32) / 4 } else { 0 };
        handling.set_if_neq(Handling { skill: score * 10 / divisor, dual_penalty });
    }
    Ok(())
}

/// What a member's other hand grips. A member has both hands.
fn other_hand(arms: &Arms, slot: WeaponSlot, grips: &Query<(&Hand, &Grip)>) -> Result<Grip> {
    for &arm in arms.entities() {
        let (hand, &grip) = grips.get(arm)?;
        if hand.0 == slot.other() {
            return Ok(grip);
        }
    }
    Err("a member has two hands".into())
}

/// How strength counts for a hand: not at all for guns and the like (none),
/// by half for weapons only thrown, lashed or shot, and a `bonus` by half
/// again in the off hand. The divisor.
fn strength_divisor(weapon: Option<&Weapon>, slot: WeaponSlot, bonus: bool) -> Option<i32> {
    let mut divisor = match weapon.map(|weapon| weapon.strength) {
        Some(Strength::None) => return None,
        Some(Strength::Half) => 2,
        _ => 1,
    };
    if bonus && slot == WeaponSlot::Secondary {
        divisor *= 2;
    }
    Some(divisor)
}

/// A load's penalty, halved for a modern weapon.
fn felt(modern: bool, penalty: i32) -> i32 {
    if modern { penalty / 2 } else { penalty }
}

/// The sheet's "To Hit" for a hand (retail `to_hit`): the weapon's and its
/// ammunition's bonus, strength, dexterity, senses, load, and Eagle Eye or
/// Power Strike; none when it cannot attack.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ToHit(pub i32);

#[allow(clippy::type_complexity)]
fn to_hit(
    mut hands: Query<(&ArmOf, &Hand, &Grip, &mut ToHit)>,
    bearers: Query<
        (&Attributes, &Skills, &Calling, &crate::game::actors::Abilities, &Burden),
        Allow<Disabled>,
    >,
    (weapons, missiles): (Query<&Weapon>, Query<&Missile>),
) -> Result {
    for (&ArmOf(bearer), &Hand(slot), &grip, mut written) in &mut hands {
        let (a, skills, calling, abilities, burden) = bearers.get(bearer)?;
        if !grip.strikes() {
            written.set_if_neq(ToHit(0));
            continue;
        }
        let weapon = weapon_of(grip, &weapons)?;
        let [strength, dexterity, senses] =
            [Attribute::Strength, Attribute::Dexterity, Attribute::Senses].map(|x| a[x] as i32);
        let mut to_hit = 0;
        if let Grip::Weapon { ammunition, .. } = grip {
            let missile = ammunition.map(|item| missiles.get(item)).transpose()?;
            to_hit += weapon.map_or(0, |w| w.to_hit) + missile.map_or(0, |m| m.to_hit);
        }
        if let Some(divisor) = strength_divisor(weapon, slot, strength > 50) {
            if strength < 50 {
                to_hit -= (50 - strength) / (divisor * 10);
            } else if strength > 50 {
                to_hit += (strength - 50) / (divisor * 10);
            }
        }
        to_hit += (dexterity - 50) / 10;
        if senses < 30 {
            to_hit -= (30 - senses) / 10;
        } else if senses > 70 {
            to_hit += (senses - 70) / 10;
        }
        let mut load = -burden.load.penalty();
        if slot == WeaponSlot::Secondary {
            load = load * 3 / 2;
        }
        to_hit += felt(skill_of(weapon) == Skill::ModernWeapon, load);
        let ranged = ranged(weapon);
        let can = |skill| skill_available(calling, abilities, skill);
        if ranged && can(Skill::EagleEye) {
            to_hit += skills[Skill::EagleEye] as i32 / 20 + 1;
        }
        if !ranged && can(Skill::PowerStrike) {
            to_hit += skills[Skill::PowerStrike] as i32 / 20 + 1;
        }
        written.set_if_neq(ToHit(to_hit));
    }
    Ok(())
}

/// A hand's rating (retail `attack_rating`, with five times its to-hit):
/// its dexterity, skill, dual penalty and physical levels. None when it
/// cannot attack.
fn rating(
    mut hands: Query<(&ArmOf, &Grip, &Handling, &ToHit, &mut Rating)>,
    bearers: Query<(&Attributes, &Calling), Allow<Disabled>>,
) -> Result {
    for (&ArmOf(bearer), &grip, handling, to_hit, mut rating) in &mut hands {
        let (a, calling) = bearers.get(bearer)?;
        if !grip.strikes() {
            rating.set_if_neq(Rating(0));
            continue;
        }
        let dexterity = a[Attribute::Dexterity] as i32;
        let base = (handling.dual_penalty
            + dexterity / 2
            + handling.skill * 2
            + calling.physical_levels())
            / 3
            + 60;
        rating.set_if_neq(Rating(base + to_hit.0 * 5));
    }
    Ok(())
}

/// The sheet's "To Damage (%)": a launcher's base, and strength; none when
/// the hand cannot attack.
fn damage_percent(
    mut hands: Query<(&ArmOf, &Hand, &Grip, &mut DamagePercent)>,
    bearers: Query<&Attributes, Allow<Disabled>>,
    (weapons, launchers): (Query<&Weapon>, Query<Option<&Launcher>>),
) -> Result {
    for (&ArmOf(bearer), &Hand(slot), &grip, mut percent) in &mut hands {
        let strength = bearers.get(bearer)?[Attribute::Strength] as i32;
        let now = match grip {
            Grip::Idle => 0,
            _ => {
                let weapon = weapon_of(grip, &weapons)?;
                let launcher = grip.weapon().map(|item| launchers.get(item)).transpose()?;
                let mut now = launcher.flatten().map_or(0, |launcher| launcher.damage);
                if let Some(divisor) = strength_divisor(weapon, slot, strength > 50) {
                    if strength < 50 {
                        now -= (50 - strength) / divisor;
                    } else if strength > 50 {
                        now += (strength * 2 - 100) / divisor;
                    }
                }
                now
            }
        };
        percent.set_if_neq(DamagePercent(now));
    }
    Ok(())
}

/// Attacks a round with a hand: one, a second past 50 and a third past 100
/// in the main hand, a second past 75 in the off hand; the hand's
/// initiative, its weapon's or a fist's from skill; and most swings an
/// attack with it makes, one, a second past 66 and a third past 100, before
/// what its weapon adds, one with a rocket launcher in the main hand. None
/// when it cannot attack.
#[allow(clippy::type_complexity)]
fn tempo(
    mut hands: Query<(
        &ArmOf,
        &Hand,
        &Grip,
        &Handling,
        (&mut AttacksARound, &mut ArmInitiative, &mut Swings),
    )>,
    grips: Query<(&Hand, &Grip)>,
    bearers: Query<(&Attributes, &Calling, &Burden, &Arms), Allow<Disabled>>,
    weapons: Query<&Weapon>,
) -> Result {
    for (&ArmOf(bearer), &Hand(slot), &grip, handling, (mut attacks, mut initiative, mut swings)) in
        &mut hands
    {
        let (a, calling, burden, arms) = bearers.get(bearer)?;
        let weapon = weapon_of(grip, &weapons)?;
        if !grip.strikes() {
            attacks.set_if_neq(AttacksARound(0));
            initiative.set_if_neq(ArmInitiative(0));
            swings.set_if_neq(Swings::default());
            continue;
        }
        let [dexterity, speed] = [Attribute::Dexterity, Attribute::Speed].map(|x| a[x] as i32);
        let modern = weapon.is_some_and(|w| w.skill == Skill::ModernWeapon);
        let load = felt(modern, -burden.load.attack_penalty());
        let shared = handling.dual_penalty + calling.physical_levels() + load + handling.skill;
        let score = (((speed + dexterity) >> 1) + shared) / 3;
        attacks.set_if_neq(AttacksARound(match slot {
            WeaponSlot::Primary => 1 + u32::from(score > 49) + u32::from(score > 99),
            WeaponSlot::Secondary => 1 + u32::from(score > 74),
        }));
        let own = weapon.map_or(handling.skill / 10, |w| w.initiative);
        initiative.set_if_neq(ArmInitiative(own));
        let main = if slot == WeaponSlot::Primary { grip } else { other_hand(arms, slot, &grips)? };
        let main_single = weapon_of(main, &weapons)?.is_some_and(|w| w.single_swing);
        // Retail reads the initiative its previous pass left, which is this.
        let score = (speed + own * 10 + shared) / 3;
        let most = if main_single { 1 } else { 1 + u32::from(score > 66) + u32::from(score > 99) };
        swings.set_if_neq(Swings { most, extra: weapon.map_or(0, |w| w.extra_swings) });
    }
    Ok(())
}

/// The skill whose training adds a kill chance to a hand's blows (retail
/// `ResolveCharacterAttack`), with its points: Critical Strike for close
/// combat, Ranged Combat for a Ranger's bows and slings, Critical Strike for
/// a Ninja's throws.
fn critical(
    hands: Query<(Entity, &ArmOf, &Grip, Option<&Critical>)>,
    bearers: Query<(&Traits, &Skills), Allow<Disabled>>,
    weapons: Query<&Weapon>,
    mut commands: Commands,
) -> Result {
    for (hand, &ArmOf(bearer), &grip, had) in &hands {
        let (traits, skills) = bearers.get(bearer)?;
        let weapon = weapon_of(grip, &weapons)?;
        let skill = weapon.map_or(Skill::MartialArts, |w| w.skill);
        let sling = weapon.is_some_and(|w| w.sling);
        let critical = if !weapon.is_some_and(|w| w.ranged) {
            Some(Skill::CriticalStrike)
        } else if matches!(skill, Skill::ModernWeapon | Skill::Bow)
            || (skill == Skill::ThrowingAndSling && sling)
        {
            traits.has(Trait::RangedCriticals).then_some(Skill::RangedCombat)
        } else if skill == Skill::ThrowingAndSling && traits.has(Trait::ThrownCriticals) {
            Some(Skill::CriticalStrike)
        } else {
            None
        };
        let now = critical.map(|critical| Critical(skills[critical]));
        if now != had.copied() {
            match now {
                Some(critical) => commands.entity(hand).insert(critical),
                None => commands.entity(hand).remove::<Critical>(),
            };
        }
    }
    Ok(())
}

/// What a member's hand is, besides its numbers, as it changes hands or
/// the member's traits change: what it loads, a Ninja's sure throw, the
/// skills it trains, its weight, its sound (a fist's when empty), and
/// whether it is modern, a sword, or ambushes (a Rogue's sword or dagger at
/// touch).
#[allow(clippy::type_complexity)]
fn kind_of_hand(
    hands: Query<(Entity, &ArmOf, Ref<Grip>)>,
    bearers: Query<Ref<Traits>, Allow<Disabled>>,
    (weapons, launchers, missiles, weights): (
        Query<&Weapon>,
        Query<&Launcher>,
        Query<&Missile>,
        Query<&Weight>,
    ),
    mut commands: Commands,
) -> Result {
    for (hand, &ArmOf(bearer), grip) in &hands {
        let traits = bearers.get(bearer)?;
        if !grip.is_changed() && !traits.is_changed() {
            continue;
        }
        let grip = *grip;
        let mut hand = commands.entity(hand);
        hand.remove::<(Loaded, SureThrow, Heft, SoundClass, Modern, FatiguePierces, Sword, Ambushes)>();
        let weapon = weapon_of(grip, &weapons)?;
        let skill = skill_of(weapon);
        let is_ranged = ranged(weapon);
        let sling = weapon.is_some_and(|w| w.sling);
        if let Some(missile) = shot(grip, &launchers, &missiles)? {
            hand.insert(Loaded(missile.modes));
        }
        if traits.has(Trait::ThrownCriticals)
            && is_ranged
            && skill == Skill::ThrowingAndSling
            && !sling
        {
            hand.insert(SureThrow);
        }
        hand.insert(Trains(match (is_ranged, traits.has(Trait::ThrownCriticals)) {
            (false, _) => &[Skill::CriticalStrike, Skill::PowerStrike],
            (true, true) => &[Skill::EagleEye, Skill::CriticalStrike],
            (true, false) => &[Skill::EagleEye],
        }));
        if let Some(item) = grip.weapon() {
            hand.insert(Heft(weights.get(item)?.0));
        }
        if let Some(sound) = weapon.map_or(Some(FIST_SOUND), |w| w.sound_class) {
            hand.insert(SoundClass(sound));
        }
        if skill == Skill::ModernWeapon {
            hand.insert(Modern);
        } else {
            hand.insert(FatiguePierces);
        }
        if skill == Skill::Sword {
            hand.insert(Sword);
        }
        let reach = weapon.map_or(Reach::Touch, |w| w.reach);
        if traits.has(Trait::Backstab)
            && reach <= Reach::Touch
            && matches!(skill, Skill::Sword | Skill::Dagger)
        {
            hand.insert(Ambushes);
        }
    }
    Ok(())
}

/// The weapon each of an actor's hands strikes with, which its attacks are
/// accepted with.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Wielding(pub EnumMap<WeaponSlot, Option<Entity>>);

/// Each hand's attacks a round and initiative: the most of its arms'.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tempo {
    pub attacks: EnumMap<WeaponSlot, u32>,
    pub initiative: EnumMap<WeaponSlot, i32>,
}

impl Tempo {
    /// The hand an attack opens with: the one with the most attacks, then
    /// the bigger initiative, the main hand on a tie (retail
    /// `ChooseCharacterAttackHand`).
    pub fn opening(&self) -> Option<WeaponSlot> {
        [WeaponSlot::Primary, WeaponSlot::Secondary]
            .into_iter()
            .filter(|&slot| self.attacks[slot] > 0)
            .reduce(|best, slot| {
                let rank = |slot: WeaponSlot| (self.attacks[slot], self.initiative[slot]);
                if rank(slot) > rank(best) { slot } else { best }
            })
    }
}

/// How far the furthest of an actor's arms that strike reaches; none when
/// it has none.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Furthest(pub Option<Reach>);

impl Furthest {
    /// Whether it reaches someone `gap` metres off.
    pub fn reaches(self, gap: f32) -> bool {
        self.0.is_some_and(|reach| gap <= reach.metres())
    }
}

/// How far an actor guards a fellow from: its first arm's reach (retail
/// reads the first attack); none when it has no arm.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GuardReach(pub Option<Reach>);

/// What an actor's hands wield follows their grips.
/// A monster's natural attacks grip nothing.
fn wielding(
    mut bearers: Query<(&Arms, &mut Wielding)>,
    hands: Query<(&Hand, Option<&Grip>)>,
) -> Result {
    for (arms, mut wielding) in &mut bearers {
        let mut now = EnumMap::default();
        for &arm in arms.entities() {
            if let (&Hand(slot), Some(grip)) = hands.get(arm)? {
                now[slot] = grip.weapon();
            }
        }
        wielding.set_if_neq(Wielding(now));
    }
    Ok(())
}

/// An actor's tempo follows its arms'.
fn tempo_of_hands(
    mut bearers: Query<(&Arms, &mut Tempo)>,
    arms: Query<(&Hand, &AttacksARound, &ArmInitiative)>,
) -> Result {
    for (own, mut tempo) in &mut bearers {
        let mut now = Tempo::default();
        for &arm in own.entities() {
            let (&Hand(slot), attacks, initiative) = arms.get(arm)?;
            now.attacks[slot] = now.attacks[slot].max(attacks.0);
            now.initiative[slot] = now.initiative[slot].max(initiative.0);
        }
        tempo.set_if_neq(now);
    }
    Ok(())
}

/// How far an actor reaches, and guards from, follows its arms that strike.
fn furthest(
    mut bearers: Query<(&Arms, &mut Furthest, &mut GuardReach)>,
    arms: Query<(&Reaches, &AttacksARound)>,
) -> Result {
    for (own, mut furthest, mut guard) in &mut bearers {
        let mut striking = Vec::new();
        for &arm in own.entities() {
            let (reaches, attacks) = arms.get(arm)?;
            if attacks.0 > 0 {
                striking.push(reaches.0);
            }
        }
        furthest.set_if_neq(Furthest(striking.iter().copied().max()));
        guard.set_if_neq(GuardReach(striking.first().copied()));
    }
    Ok(())
}

pub(super) fn plugin(app: &mut App) {
    app.add_observer(grow_hands).add_systems(
        super::Settle,
        (
            grip,
            (reach, modes, rolls, effects, handling, damage_percent),
            (to_hit, (rating, tempo, kind_of_hand, critical)).chain(),
            (wielding, tempo_of_hands, furthest),
        )
            .chain()
            .in_set(super::Written)
            .after(super::figures::skills)
            .before(super::figures::mending),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::{Gender, Profession, Race};
    use crate::content::items::{EquipClass, ItemDefinition, ItemId, ItemKind};
    use crate::game::actors::Settle;
    use crate::game::characters::{Character, Vitals};
    use crate::game::inventory::EquipSlot;
    use crate::game::items::{Worn, make};
    use bevy::ecs::system::RunSystemOnce;

    fn weapon(id: u16, class: EquipClass, skill: Skill) -> ItemDefinition {
        ItemDefinition {
            id: ItemId(id),
            class,
            weapon_skill: Some(skill),
            damage: Dice { base: 0, count: 1, sides: 6 },
            attack_modes: EnumSet::only(AttackMode::Swing),
            professions: EnumSet::all(),
            races: EnumSet::all(),
            genders: EnumSet::all(),
            ..ItemDefinition::default()
        }
    }

    fn shield() -> ItemDefinition {
        ItemDefinition { id: ItemId(1), class: EquipClass::Shield, ..ItemDefinition::default() }
    }

    fn arrows() -> ItemDefinition {
        let mut arrows = weapon(3, EquipClass::Ammunition, Skill::Bow);
        arrows.kind = ItemKind::ARROW;
        arrows.damage = Dice { base: 0, count: 1, sides: 8 };
        arrows
    }

    /// A member of this profession, wearing these, once their body has
    /// settled; and their two hands, main first.
    fn member(
        profession: Profession,
        worn: &[(EquipSlot, &ItemDefinition)],
        train: impl FnOnce(&mut Character),
    ) -> (App, [Entity; 2]) {
        let mut app = App::new();
        app.add_plugins(crate::game::actors::plugin).init_resource::<crate::game::rng::GameRng>();
        let mut character = Character::new("A", Race::Human, Gender::Male, profession);
        train(&mut character);
        let vitals = Vitals::new(&character);
        let world = app.world_mut();
        let member =
            world.spawn((crate::game::actors::armour::born(Race::Human), character, vitals)).id();
        for &(slot, definition) in worn {
            let definition = definition.clone();
            let item = world
                .run_system_once(move |mut commands: Commands| {
                    make(&mut commands, &definition, 1, true)
                })
                .unwrap();
            world.flush();
            world.entity_mut(item).insert(Worn { by: member, slot });
        }
        world.run_schedule(Settle);
        world.run_schedule(Settle);
        let hands = world.get::<Arms>(member).unwrap().entities().to_vec();
        let slot = |hand: Entity| world.get::<Hand>(hand).unwrap().0;
        let main = *hands.iter().find(|&&hand| slot(hand) == WeaponSlot::Primary).unwrap();
        let off = *hands.iter().find(|&&hand| slot(hand) == WeaponSlot::Secondary).unwrap();
        (app, [main, off])
    }

    fn get<T: Component + Clone>(app: &App, entity: Entity) -> T {
        app.world().get::<T>(entity).unwrap().clone()
    }

    /// Retail `ResolveCharacterAttackDamage`: empty hands roll Martial Arts
    /// 0's 1d2, the main hand's two more on the base; a punch rolls what the
    /// off hand holds as its record gives it (retail reads `Hand[1]`'s item),
    /// a shield's none.
    #[test]
    fn a_punch_rolls_the_dice_of_whatever_the_off_hand_holds() {
        let fighter = Profession::Fighter;
        let (app, [main, _]) = member(fighter, &[], |_| {});
        assert_eq!(get::<Modes>(&app, main).0, EnumSet::only(AttackMode::Punch));
        let rolls = get::<Rolls>(&app, main);
        assert_eq!(rolls.punch, Dice { base: 0, count: 1, sides: 2 });
        assert_eq!(rolls.kick, Dice { base: 2, count: 1, sides: 2 });
        let (app, [main, _]) = member(fighter, &[(EquipSlot::Secondary, &shield())], |_| {});
        assert_eq!(get::<Rolls>(&app, main).punch, Dice::default());
        let arrows = arrows();
        let (app, [main, _]) = member(fighter, &[(EquipSlot::Secondary, &arrows)], |_| {});
        assert_eq!(get::<Rolls>(&app, main).punch, arrows.damage);
    }

    /// Retail `CalcAttacks`: an empty off hand beside a weapon does not
    /// punch; two weapons both strike, each paying 10 per hand number and a
    /// quarter of the Dual Weapons it lacks off its rating, and the main hand
    /// opens.
    #[test]
    fn two_weapons_each_attack_with_their_dual_wielding_penalty() {
        let sword = weapon(0, EquipClass::ShortWeapon, Skill::Sword);
        let dagger = weapon(2, EquipClass::ShortWeapon, Skill::Dagger);
        let train = |c: &mut Character| c.training.0[Skill::DualWeapons] = 40;
        let fighter = Profession::Fighter;
        let (app, [_, off]) = member(fighter, &[(EquipSlot::Primary, &sword)], train);
        assert_eq!(get::<Grip>(&app, off), Grip::Idle);
        let both = [(EquipSlot::Primary, &sword), (EquipSlot::Secondary, &dagger)];
        let (app, [main, off]) = member(fighter, &both, train);
        assert!(get::<Grip>(&app, main).weapon().is_some());
        assert!(get::<Grip>(&app, off).weapon().is_some());
        assert_eq!(get::<Modes>(&app, off).0, dagger.attack_modes);
        let handling = [main, off].map(|hand| get::<Handling>(&app, hand).dual_penalty);
        assert_eq!(handling, [-10 - 15, -20 - 15]);
        assert!(get::<Rating>(&app, main).0 > get::<Rating>(&app, off).0);
    }

    /// A shield keeps the off hand out; a bow without arrows cannot shoot,
    /// and with them shoots theirs: their dice, their effects with its own,
    /// their modes to pierce by, and its own base as a damage bonus (+20%)
    /// besides strength's.
    #[test]
    fn a_shield_or_a_mismatched_weapon_keeps_the_off_hand_out() {
        let sword = weapon(0, EquipClass::ShortWeapon, Skill::Sword);
        let ranger = Profession::Ranger;
        let shielded = [(EquipSlot::Primary, &sword), (EquipSlot::Secondary, &shield())];
        let (app, [main, off]) = member(ranger, &shielded, |_| {});
        assert!(get::<Grip>(&app, main).strikes() && !get::<Grip>(&app, off).strikes());
        let mut bow = weapon(2, EquipClass::RangedWeapon, Skill::Bow);
        bow.kind = ItemKind::LONGBOW;
        bow.reach = Reach::Long;
        bow.damage = Dice { base: 2, count: 1, sides: 4 };
        let (app, [main, _]) = member(ranger, &[(EquipSlot::Primary, &bow)], |_| {});
        assert_eq!(get::<Grip>(&app, main), Grip::Idle);
        let mut arrows = arrows();
        arrows.effects.chances[AttackEffect::Poison] = 10;
        let loaded = [(EquipSlot::Primary, &bow), (EquipSlot::Secondary, &arrows)];
        let (app, [main, off]) = member(ranger, &loaded, |_| {});
        assert!(get::<Grip>(&app, main).strikes() && !get::<Grip>(&app, off).strikes());
        assert_eq!(get::<Loaded>(&app, main).0, arrows.attack_modes);
        assert_eq!(get::<Rolls>(&app, main).dice, arrows.damage);
        assert_eq!(get::<BlowEffects>(&app, main).0.chances[AttackEffect::Poison], 10);
        let strength = Character::new("A", Race::Human, Gender::Male, ranger).abilities.0
            [Attribute::Strength] as i32;
        let from_strength = match strength {
            ..50 => -(50 - strength),
            50 => 0,
            _ => 2 * strength - 100,
        };
        assert_eq!(get::<DamagePercent>(&app, main).0, 20 + from_strength);
    }
}
