//! What a body in a fight is to the blows struck at it and by it: the same
//! components on a party member and on a monster, so the swing systems read
//! them without asking which it is. Each component is produced from its
//! body's own sources (a member's sheet, gear and party; a monster's species,
//! pose and spells) by one system per kind, at the start of every fixed step
//! of a fight and of every swing pass, and nothing else writes it. They are
//! valid only in a fight: they go when the fight ends, or when a monster
//! leaves it.

use avian3d::prelude::Position;
use bevy::prelude::*;
use enum_map::EnumMap;

use super::approach::CombatMove;
use super::attack::{self, Grip};
use super::round::PartyTurn;
use super::{Battle, Combatant, PartyPace};
use crate::content::characters::Trait;
use crate::content::items::AttackMode;
use crate::content::species::Reach;
use crate::game::characters::{Character, Vitals, scale_by_level};
use crate::game::data::Catalogs;
use crate::game::inventory::{Burden, Carried};
use crate::game::magic::enchantments::PartyEffects;
use crate::game::party::body::{self as party_body, PartyBody};
use crate::game::party::formation::{self, Formation, Row};
use crate::game::party::roster::PartyRoster;
use crate::game::world::ActorPose;
use crate::game::world::navigation::{Walker, feet};
use crate::game::world::population::Monster;

/// Where a body stands and how big it is: a monster at its pose, a member
/// in the party's body, which they share.
#[derive(Component, Clone, Copy, Debug)]
pub struct Footing {
    pub feet: Vec3,
    /// Where it looks from.
    pub eye: Vec3,
    /// Where a blow or a missile strikes it.
    pub centre: Vec3,
    pub radius: f32,
    pub half_height: f32,
    /// Which way it faces.
    pub yaw: f32,
}

impl Footing {
    pub fn forward(&self) -> Vec3 {
        party_body::heading(self.yaw)
    }

    /// How far it is from another body, body to body.
    pub fn gap(&self, other: &Footing) -> f32 {
        (self.feet.distance(other.feet) - self.radius - other.radius).max(0.0)
    }

    /// The side of it something at `at` is on.
    pub fn quadrant(&self, at: Vec3) -> Row {
        formation::quadrant(self.eye, self.yaw, at)
    }
}

/// Senses + speed + dexterity + intelligence (retail SSDI), which weighs
/// for its blows and against those at it.
#[derive(Component, Clone, Copy, Debug)]
pub struct Agility(pub i32);

/// The percent cut off the damage of blows at it.
#[derive(Component, Clone, Copy, Debug)]
pub struct Reduction(pub i32);

/// How well it presses a blow past a guard, or guards against one (retail
/// `GetCharacterProtectionScore`, `GetMonsterProtectionScore`).
#[derive(Component, Clone, Copy, Debug)]
pub struct Protection(pub i32);

/// The percent chance a Missile Shield turns aside what is shot or thrown
/// at it: a monster's own, a member's party's.
#[derive(Component, Clone, Copy, Debug)]
pub struct MissileShield(pub u32);

/// How a blinded body still aims: at 10 at most, or for one effective
/// while blind, by its profession's level past that.
#[derive(Component, Clone, Copy, Debug)]
pub struct BlindFighting(pub Option<u8>);

/// The percent chance its natural strikes roll extra dice (retail
/// `RollBackstabExtraDice`): by a member's profession, a monster's level.
#[derive(Component, Clone, Copy, Debug)]
pub struct Backstab(pub i32);

/// Where blows land on a monster, by its species' chances; a member has
/// none of its own.
#[derive(Component, Clone, Copy, Debug)]
pub struct HitLocations(pub [u8; 7]);

/// A body's armour class against blows (retail `GetTargetArmorClass`,
/// `TargetArmorClassAtLocation`, `GetTargetArmorClassModifier`).
#[derive(Component, Clone, Copy, Debug)]
pub struct ArmourClass {
    /// Against being struck at all.
    pub total: i32,
    /// Where struck, by location.
    pub at: [i32; 7],
    /// What each mode finds besides.
    pub by_mode: EnumMap<AttackMode, i32>,
    /// What a surprise strips from its guard.
    pub exposed: i32,
    /// What fatigue adds to a monster's guard.
    pub tired: i32,
    /// On the move, its guard down to blows that close in.
    pub distracted: bool,
    /// Braced, defending.
    pub braced: bool,
}

impl ArmourClass {
    /// What its guard comes to against a blow: a surprise strips what it
    /// exposes, and four go to a blow that closes in on it on the move.
    pub fn guard(&self, mode: AttackMode, surprise: attack::Surprise) -> i32 {
        let mut guard = self.tired;
        if surprise.any() {
            guard -= self.exposed;
        }
        if self.distracted && !attack::keeps_distance(mode) {
            guard -= 4;
        }
        guard
    }

    /// Its armour against a blow's aim; its guard counts once.
    pub fn against_aim(&self, mode: AttackMode, surprise: attack::Surprise) -> i32 {
        self.total + self.guard(mode, surprise)
    }

    /// Its armour where a blow struck; its guard counts twice.
    pub fn at(&self, mode: AttackMode, surprise: attack::Surprise, location: usize) -> i32 {
        self.at[location] - self.by_mode[mode] + 2 * self.guard(mode, surprise)
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

/// The members stand in the party's body.
pub(super) fn place_members(
    mut commands: Commands,
    body: Single<(&Position, &PartyBody)>,
    roster: Single<&PartyRoster>,
) {
    let (position, body) = *body;
    let eye = PartyBody::eye(position);
    let footing = Footing {
        feet: PartyBody::feet_below(eye),
        eye,
        centre: PartyBody::centre(eye),
        radius: party_body::RADIUS,
        half_height: party_body::RADIUS,
        yaw: body.yaw,
    };
    for member in roster.iter() {
        commands.entity(member).insert(footing);
    }
}

/// The monsters in the fight stand at their poses.
pub(super) fn place_monsters(
    mut commands: Commands,
    monsters: Query<(Entity, &Monster, ActorPose, Option<&Walker>), With<Combatant>>,
) {
    for (entity, monster, pose, walker) in &monsters {
        let feet = feet(pose.translation(), walker);
        let half_height = monster.body.eye_height * 0.5;
        commands.entity(entity).insert(Footing {
            feet,
            eye: monster.eye(feet),
            centre: feet + Vec3::Y * half_height,
            radius: monster.body.radius,
            half_height,
            yaw: party_body::yaw_toward(pose.forward()),
        });
    }
}

/// A member, by their sheet, gear and party: what they wear and the spells
/// on them and the party, braced when they defend and distracted while the
/// party runs.
pub(super) fn judge_members(
    mut commands: Commands,
    members: Query<(&Character, &Vitals, &Carried, &Burden, Option<&PartyTurn>)>,
    roster: Single<&PartyRoster>,
    party: Single<&PartyEffects>,
    battle: Single<&Battle>,
    content: If<Res<Catalogs>>,
) {
    let running = battle.pace == Some(PartyPace::Run);
    for member in roster.iter() {
        let Ok((character, vitals, carried, burden, turn)) = members.get(member) else { continue };
        let stats = character.stats(&vitals.enchantments);
        let braced = turn.is_some_and(PartyTurn::defends);
        let load = burden.load;
        let armour = attack::enchanted_armour(
            &stats,
            carried,
            &content.items,
            load,
            braced,
            &vitals.enchantments,
            Some(&party.0),
        );
        let grip = Grip::of(carried, &content.items);
        let blind_fighter = character.has_trait(Trait::EffectiveWhileBlind);
        commands.entity(member).insert((
            Agility(attack::member_agility(&stats)),
            Reduction(stats.damage_reduction()),
            Protection(attack::member_protection(&stats, vitals, &grip, load)),
            MissileShield(party.0.missile_deflection()),
            BlindFighting(blind_fighter.then(|| character.profession_levels[character.profession])),
            Backstab(character.scale_by_profession(100.0) as i32),
            ArmourClass {
                total: armour.total,
                at: std::array::from_fn(|location| {
                    armour.by_location.values().nth(location).copied().unwrap_or(armour.total)
                }),
                by_mode: EnumMap::default(),
                exposed: armour.speed.max(0) + armour.shield.max(0),
                tired: 0,
                distracted: running,
                braced,
            },
        ));
    }
}

/// A monster in the fight, by its species, fatigue and spells: its guard
/// drops while it advances.
pub(super) fn judge_monsters(
    mut commands: Commands,
    monsters: Query<(Entity, &Monster, &Vitals, Has<CombatMove>), With<Combatant>>,
    content: If<Res<Catalogs>>,
) {
    for (entity, monster, vitals, advancing) in &monsters {
        let kind = &content.species[monster.species];
        let spells = vitals.enchantments.armour();
        commands.entity(entity).insert((
            Agility(attack::monster_agility(kind, &vitals.enchantments)),
            Reduction(attack::monster_reduction(kind, vitals)),
            Protection(attack::monster_protection(kind, vitals)),
            MissileShield(vitals.enchantments.missile_deflection()),
            BlindFighting(None),
            Backstab(scale_by_level(kind.level, 100.0) as i32),
            HitLocations(kind.hit_locations),
            ArmourClass {
                total: 10 - i32::from(kind.evasion) + spells,
                at: std::array::from_fn(|location| {
                    10 + spells - i32::from(kind.armour_by_location[location])
                }),
                by_mode: EnumMap::from_fn(|mode| i32::from(kind.armour_by_mode[mode])),
                exposed: 2 * i32::from(kind.vulnerable_from_behind),
                tired: attack::fatigue(vitals) / 10,
                distracted: advancing,
                braced: false,
            },
        ));
    }
}

/// What a body is to blows, gone once it is out of the fight.
fn forget(commands: &mut EntityCommands) {
    commands.try_remove::<(
        Footing,
        Agility,
        Reduction,
        Protection,
        MissileShield,
        BlindFighting,
        Backstab,
        HitLocations,
        ArmourClass,
    )>();
}

/// A monster leaving the fight is no longer judged.
pub(super) fn leave(left: On<Remove<Combatant>>, mut commands: Commands) {
    forget(&mut commands.entity(left.entity));
}

/// The fight is over: nobody is judged.
pub(super) fn disband(
    _: On<Remove<Battle>>,
    mut commands: Commands,
    judged: Query<Entity, With<Footing>>,
) {
    for body in &judged {
        forget(&mut commands.entity(body));
    }
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
            tired: 0,
            distracted: false,
            braced: false,
        };
        let swing = AttackMode::Swing;
        assert_eq!(armour.against_aim(swing, Surprise::default()), 8);
        let behind = Surprise { unaware: true, natural: false };
        assert_eq!(armour.against_aim(swing, behind), 8 - 5);
        let running = ArmourClass { distracted: true, ..armour };
        assert_eq!(running.against_aim(swing, Surprise::default()), 8 - 4);
        // Shots and throws do not find the gap; where struck the guard
        // counts twice.
        assert_eq!(running.against_aim(AttackMode::Shoot, Surprise::default()), 8);
        assert_eq!(running.at(swing, behind, 1), 8 - 2 * (5 + 4));
    }
}
