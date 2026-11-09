//! What a body in a fight is to the blows struck at it and by it: the same
//! components on a party member and on a monster, so the rules of a fight
//! read them without asking which it is. One system judges every body from
//! its own sources (a member's sheet, gear and party; a monster's species
//! and spells) as each fixed step of a fight begins, and nothing else
//! writes them. They are valid only in a fight: they go when the fight
//! ends, or when a monster leaves it. Where bodies stand is their
//! [`Footing`](crate::game::world::footing::Footing).

use bevy::prelude::*;
use enum_map::EnumMap;
use enumset::EnumSet;

use super::approach::CombatMove;
use super::attack::{self, Grip};
use super::turn::Ordered;
use super::{Battle, Combatant, PartyPace};
use crate::content::characters::{Attribute, BodyPart, Skill, Trait};
use crate::content::items::{AttackEffects, AttackMode, Dice};
use crate::content::species::{AttackWord, MonsterKind, Reach, Species};
use crate::game::characters::{Character, Vitals, scale_by_level};
use crate::game::data::Catalogs;
use crate::game::inventory::{Burden, Carried, StackId, WeaponSlot};
use crate::game::magic::enchantments::PartyEffects;
use crate::game::party::formation::Formation;
use crate::game::world::footing::Footing;
use crate::game::world::population::Monster;

/// What a body in a fight is, apart from what it strikes with and its
/// armour.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Fighter {
    /// When it acts in a round, before its weapon's own initiative.
    pub initiative: i32,
    /// How far it may walk in one combat move, in metres: a monster's
    /// species' move range (retail `combat_move_range` × 20). The party
    /// walks as one, so a member has none.
    pub stride: Option<f32>,
    /// Its strength, which breaks webs.
    pub brawn: u32,
    /// Its intelligence, which weighs its choices.
    pub wit: u32,
    /// Senses + speed + dexterity + intelligence (retail SSDI), which
    /// weighs for its blows and against those at it.
    pub agility: i32,
    /// The percent cut off the damage of blows at it.
    pub reduction: i32,
    /// How well it presses a blow past a guard, or guards against one
    /// (retail `GetCharacterProtectionScore`, `GetMonsterProtectionScore`).
    pub protection: i32,
    /// The percent chance a Missile Shield turns aside what is shot or
    /// thrown at it: a monster's own, a member's party's.
    pub missile_shield: u32,
    /// Effective while blind (a member with the trait, a Monk among
    /// monsters): its aim past 10 scales by this level instead of being
    /// held to 10, and its action is not lost to blindness.
    pub blind_fighting: Option<u8>,
    /// The percent chance its natural strikes roll extra dice (retail
    /// `RollBackstabExtraDice`): by a member's profession, a monster's
    /// level.
    pub backstab: i32,
    /// Where blows land on a monster, by its species' chances; a member
    /// has none of its own.
    pub hit_locations: Option<[u8; 7]>,
}

/// A body's armour class against blows (retail `GetTargetArmorClass`,
/// `TargetArmorClassAtLocation`, `GetTargetArmorClassModifier`).
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct ArmourClass {
    /// Against being struck at all.
    pub total: i32,
    /// Where struck, by location.
    pub at: [i32; 7],
    /// What each mode finds besides.
    pub by_mode: EnumMap<AttackMode, i32>,
    /// What a surprise strips from its guard.
    pub exposed: i32,
    /// Fatigue adds to its guard, as it does to a monster's.
    pub tires: bool,
    /// On the move, its guard down to blows that close in.
    pub distracted: bool,
    /// Braced, defending.
    pub braced: bool,
}

impl ArmourClass {
    /// What its guard comes to against a blow: fatigue adds a tenth of
    /// itself, a surprise strips what it exposes, and four go to a blow
    /// that closes in on it on the move.
    pub fn guard(&self, mode: AttackMode, surprise: attack::Surprise, vitals: &Vitals) -> i32 {
        let mut guard = if self.tires { attack::fatigue(vitals) / 10 } else { 0 };
        if surprise.any() {
            guard -= self.exposed;
        }
        if self.distracted && !attack::keeps_distance(mode) {
            guard -= 4;
        }
        guard
    }

    /// Its armour against a blow's aim; its guard counts once.
    pub fn against_aim(
        &self,
        mode: AttackMode,
        surprise: attack::Surprise,
        vitals: &Vitals,
    ) -> i32 {
        self.total + self.guard(mode, surprise, vitals)
    }

    /// Its armour where a blow struck; its guard counts twice.
    pub fn at(
        &self,
        mode: AttackMode,
        surprise: attack::Surprise,
        vitals: &Vitals,
        location: usize,
    ) -> i32 {
        self.at[location] - self.by_mode[mode] + 2 * self.guard(mode, surprise, vitals)
    }
}

/// What a body strikes with: one of a member's weapon slots as their gear
/// and stats make it now, or one of a monster's species' attacks. A
/// member's two slots are its two arms; a monster's attacks all belong to
/// its primary slot, which shares its attacks a round among them.
#[derive(Clone, Debug, PartialEq)]
pub struct Arm {
    pub slot: WeaponSlot,
    /// The weapon held, which an accepted attack is locked to.
    pub weapon: Option<StackId>,
    pub reach: Reach,
    pub modes: EnumSet<AttackMode>,
    /// The dice its blows roll; a punch strikes with the secondary slot's
    /// fist, a kick with the primary's.
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

/// What a body strikes with this step.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Arms(pub Vec<Arm>);

impl Arms {
    /// The furthest any of its arms reaches.
    pub fn reach(&self) -> Reach {
        self.0.iter().map(|arm| arm.reach).max().unwrap_or(Reach::Touch)
    }

    /// The arms of a slot.
    pub fn of(&self, slot: WeaponSlot) -> impl Iterator<Item = &Arm> {
        self.0.iter().filter(move |arm| arm.slot == slot)
    }

    /// The attacks a round the slot has: its arms' most.
    pub fn attacks(&self, slot: WeaponSlot) -> u32 {
        self.of(slot).map(|arm| arm.attacks).max().unwrap_or(0)
    }

    /// The slot's initiative: its arms' most.
    pub fn initiative(&self, slot: WeaponSlot) -> i32 {
        self.of(slot).map(|arm| arm.initiative).max().unwrap_or(0)
    }

    /// The slot an attack opens with (retail `ChooseCharacterAttackHand`):
    /// the one with the most attacks, then the bigger initiative.
    pub fn opening(&self) -> Option<WeaponSlot> {
        [WeaponSlot::Primary, WeaponSlot::Secondary]
            .into_iter()
            .filter(|&slot| self.attacks(slot) > 0)
            .max_by_key(|&slot| (self.attacks(slot), self.initiative(slot)))
    }

    /// The weapons its slots hold.
    pub fn weapons(&self) -> EnumMap<WeaponSlot, Option<StackId>> {
        EnumMap::from_fn(|slot| self.of(slot).find_map(|arm| arm.weapon))
    }
}

/// Whether a body reaches another with `reach` (retail `CloseFormationGap`,
/// `FrontRankScreens`): a member and someone outside the party by the
/// formation, which shortens touch and short reach by the occupied rows
/// between; two members unless the center screens a touch blow; anyone
/// else by the gap between their bodies.
pub fn reaches(
    formation: &Formation,
    (a, from): (Entity, &Footing),
    (b, to): (Entity, &Footing),
    reach: Reach,
) -> bool {
    let seated = |body| formation.row_of(body).is_some();
    let through_formation = |member, seat: &Footing, other: &Footing| {
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

/// A member's arm in a slot, as their gear and stats make it.
fn member_arm(
    grip: &Grip,
    stats: &crate::game::characters::CharacterStats<'_>,
    character: &Character,
    load: crate::game::characters::Load,
    to_hit_bonus: i32,
    slot: WeaponSlot,
) -> Arm {
    let skill = grip.weapon_skill(slot);
    let reach = grip.reach(slot);
    Arm {
        slot,
        weapon: grip.weapon_id(slot),
        reach,
        modes: grip.modes(stats, slot),
        dice: grip.dice(stats, slot),
        punch: grip.dice(stats, WeaponSlot::Secondary),
        kick: grip.dice(stats, WeaponSlot::Primary),
        rating: grip.rating(stats, slot) + (grip.to_hit(stats, load, slot) + to_hit_bonus) * 5,
        percent: grip.damage_percent(stats, slot),
        effects: grip.effects(stats, slot),
        slays: grip.slays(slot),
        attacks: grip.attacks(stats, load, slot),
        initiative: grip.initiative(stats, slot),
        swings: grip.swings(stats, load, slot),
        extra_swings: grip.extra_swings(slot),
        ranged: grip.ranged(slot),
        loaded: grip.ammunition(slot).map(|ammunition| ammunition.attack_modes),
        sure_throw: grip.sure_throw(stats, slot),
        critical_skill: grip.critical_skill(stats, slot),
        weight: grip.weapon(slot).map(|weapon| weapon.weight),
        tires: true,
        modern: skill == Skill::ModernWeapon,
        fatigue_pierces: skill != Skill::ModernWeapon,
        sword: skill == Skill::Sword,
        ambushes: character.has_trait(Trait::Backstab)
            && reach <= Reach::Touch
            && matches!(skill, Skill::Sword | Skill::Dagger),
        word: None,
        lands_on_party: None,
    }
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

/// What a member is to blows, by their sheet, gear and party: what they
/// wear and the spells on them and the party, braced when they defend and
/// distracted while the party runs.
fn member_fighter(
    (character, vitals, carried, burden): (&Character, &Vitals, &Carried, &Burden),
    braced: bool,
    running: bool,
    party: &PartyEffects,
    items: &crate::content::items::ItemCatalog,
) -> (Fighter, Arms, ArmourClass) {
    let stats = character.stats(&vitals.enchantments);
    let load = burden.load;
    let armour = attack::enchanted_armour(
        &stats,
        carried,
        items,
        load,
        braced,
        &vitals.enchantments,
        Some(&party.0),
    );
    let grip = Grip::of(carried, items);
    let blind_fighter = character.has_trait(Trait::EffectiveWhileBlind);
    let to_hit_bonus = party.0.hit_bonus();
    let arms = [WeaponSlot::Primary, WeaponSlot::Secondary]
        .into_iter()
        .filter(|&slot| grip.in_play(slot))
        .map(|slot| member_arm(&grip, &stats, character, load, to_hit_bonus, slot))
        .collect();
    let fighter = Fighter {
        initiative: stats.initiative(load),
        stride: None,
        brawn: stats.attribute(Attribute::Strength),
        wit: stats.attribute(Attribute::Intelligence),
        agility: attack::member_agility(&stats),
        reduction: stats.damage_reduction(),
        protection: attack::member_protection(&stats, vitals, &grip, load),
        missile_shield: party.0.missile_deflection(),
        blind_fighting: blind_fighter.then(|| character.profession_levels[character.profession]),
        backstab: character.scale_by_profession(100.0) as i32,
        hit_locations: None,
    };
    let armour = ArmourClass {
        total: armour.total,
        at: std::array::from_fn(|location| {
            armour.by_location.values().nth(location).copied().unwrap_or(armour.total)
        }),
        by_mode: EnumMap::default(),
        exposed: armour.speed.max(0) + armour.shield.max(0),
        tires: false,
        distracted: running,
        braced,
    };
    (fighter, Arms(arms), armour)
}

/// What a monster is to blows, by its species, fatigue and spells: its
/// guard drops while it advances.
fn monster_fighter(
    kind: &Species,
    vitals: &Vitals,
    advancing: bool,
) -> (Fighter, Arms, ArmourClass) {
    let spells = vitals.enchantments.armour();
    let hit_bonus = vitals.enchantments.hit_bonus();
    let arms = kind.attacks.iter().map(|swung| monster_arm(kind, swung, hit_bonus)).collect();
    let fighter = Fighter {
        initiative: i32::from(kind.initiative),
        stride: Some(kind.combat_move_range * super::approach::METRES_PER_MOVE_RANGE),
        brawn: u32::from(kind.attributes.strength),
        wit: u32::from(kind.attributes.intelligence),
        agility: attack::monster_agility(kind, &vitals.enchantments),
        reduction: attack::monster_reduction(kind, vitals),
        protection: attack::monster_protection(kind, vitals),
        missile_shield: vitals.enchantments.missile_deflection(),
        blind_fighting: (kind.kind == MonsterKind::Monk).then_some(kind.level),
        backstab: scale_by_level(kind.level, 100.0) as i32,
        hit_locations: Some(kind.hit_locations),
    };
    let armour = ArmourClass {
        total: 10 - i32::from(kind.evasion) + spells,
        at: std::array::from_fn(|location| {
            10 + spells - i32::from(kind.armour_by_location[location])
        }),
        by_mode: EnumMap::from_fn(|mode| i32::from(kind.armour_by_mode[mode])),
        exposed: 2 * i32::from(kind.vulnerable_from_behind),
        tires: true,
        distracted: advancing,
        braced: false,
    };
    (fighter, Arms(arms), armour)
}

/// Keep every body in the fight judged from its sources: its sheet, gear,
/// spells and party, or its species and spells; braced while it defends,
/// distracted while it moves. One whose sources have not changed since it
/// was last judged is left alone.
#[allow(clippy::type_complexity)]
pub(super) fn judge_fighters(
    mut commands: Commands,
    bodies: Query<
        (
            Entity,
            Ref<Vitals>,
            AnyOf<((Ref<Character>, Ref<Carried>, Ref<Burden>), Ref<Monster>)>,
            (Option<&Ordered>, Has<CombatMove>),
            Option<&ArmourClass>,
        ),
        With<Combatant>,
    >,
    party: Single<Ref<PartyEffects>>,
    round: Single<&super::Round, With<Battle>>,
    content: Res<Catalogs>,
) {
    let running = round.pace == Some(PartyPace::Run);
    for (entity, vitals, (sheet, monster), (ordered, advancing), previous) in &bodies {
        let changed = previous.is_none() || vitals.is_changed() || content.is_changed();
        let judged = match (sheet, monster) {
            (Some((character, carried, burden)), _) => {
                let braced = ordered.is_some_and(Ordered::defends);
                if !(changed
                    || character.is_changed()
                    || carried.is_changed()
                    || burden.is_changed()
                    || party.is_changed()
                    || previous.is_some_and(|armour| {
                        armour.braced != braced || armour.distracted != running
                    }))
                {
                    continue;
                }
                member_fighter(
                    (&character, &vitals, &carried, &burden),
                    braced,
                    running,
                    &party,
                    &content.items,
                )
            }
            (None, Some(monster)) => {
                if !(changed
                    || monster.is_changed()
                    || previous.is_some_and(|armour| armour.distracted != advancing))
                {
                    continue;
                }
                monster_fighter(&content.species[monster.species], &vitals, advancing)
            }
            (None, None) => continue,
        };
        commands.entity(entity).insert(judged);
    }
}

/// An actor leaving the fight is no longer judged.
pub(super) fn leave(left: On<Remove<Combatant>>, mut commands: Commands) {
    commands.entity(left.entity).try_remove::<(Fighter, Arms, ArmourClass)>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::combat::attack::Surprise;

    #[test]
    fn surprise_strips_the_exposed_guard_and_the_distracted_drop_theirs() {
        let armour = ArmourClass {
            total: 8,
            at: [8; 7],
            by_mode: EnumMap::default(),
            exposed: 5,
            tires: false,
            distracted: false,
            braced: false,
        };
        let vitals = Vitals::full(10, 10);
        let swing = AttackMode::Swing;
        assert_eq!(armour.against_aim(swing, Surprise::default(), &vitals), 8);
        let behind = Surprise { unaware: true, natural: false };
        assert_eq!(armour.against_aim(swing, behind, &vitals), 8 - 5);
        let running = ArmourClass { distracted: true, ..armour };
        assert_eq!(running.against_aim(swing, Surprise::default(), &vitals), 8 - 4);
        // Shots and throws do not find the gap; where struck the guard
        // counts twice.
        assert_eq!(running.against_aim(AttackMode::Shoot, Surprise::default(), &vitals), 8);
        assert_eq!(running.at(swing, behind, &vitals, 1), 8 - 2 * (5 + 4));
    }
}
