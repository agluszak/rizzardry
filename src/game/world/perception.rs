//! Shared, stateless sight checks. Profiles, effects, physical poses and
//! scenery remain authoritative; no second current-visibility table is kept.
use crate::{
    content::{
        characters::{Attribute, Skill},
        species::Species,
    },
    game::{
        characters::{Character, Condition, Profession, Vitals},
        world::{obstacles::WorldObstacles, population::Monster},
    },
};
use bevy::prelude::*;

#[derive(Clone, Copy)]
pub struct View {
    pub eye: Vec3,
    pub forward: Vec3,
    pub far: f32,
    pub searching: bool,
}
/// The party's point, 1000 world units (2 m) under its eye: what reach and
/// sight measure from (retail `GetPointDistanceToPlayer`).
pub fn party_point(eye: Vec3) -> Vec3 {
    eye - Vec3::Y * crate::content::EYE_HEIGHT
}

/// How far a monster standing at `feet` is from the party (retail
/// `GetPointDistanceToPlayer`): from its feet to the party's point, less
/// both bodies' radii (its fight radius, the party's 1 m), never below 0.
/// Reach and sight compare against this.
pub fn party_distance(eye: Vec3, monster: &Monster, feet: Vec3) -> f32 {
    (feet.distance(party_point(eye)) - monster.body.radius - crate::game::party::body::RADIUS)
        .max(0.0)
}

/// Whether the party has a clear line to a monster (retail
/// `GetPlayerToMonsterSightFlags`): within the far clip, nothing solid
/// between the party's eye and the monster's. The party's line of sight,
/// whichever way it faces.
pub fn in_view(
    (eye, far): (Vec3, f32),
    monster: &Monster,
    feet: Vec3,
    obstacles: &WorldObstacles,
) -> bool {
    party_distance(eye, monster, feet) <= far && obstacles.clear(eye, monster.eye(feet))
}

impl View {
    pub fn in_range(self, monster: &Monster, feet: Vec3) -> bool {
        !monster.is_dead() && party_distance(self.eye, monster, feet) <= self.far
    }
    /// Retail party-side sight. Shadow Hound supplies the full-range override,
    /// while blindness, the target's concealment and solid scenery still apply.
    pub fn sees<'a>(
        self,
        mut members: impl Iterator<Item = (&'a Character, &'a Vitals)>,
        shadow_hound: bool,
        monster: &Monster,
        species: &Species,
        feet: Vec3,
        in_combat: bool,
        obstacles: &WorldObstacles,
    ) -> bool {
        if monster.is_dead() || !in_view((self.eye, self.far), monster, feet, obstacles) {
            return false;
        }
        let distance = party_distance(self.eye, monster, feet);
        let angle = if shadow_hound {
            0.0
        } else {
            self.forward.with_y(0.0).angle_between((feet - self.eye).with_y(0.0))
        };
        let concealment = if in_combat { 0 } else { u32::from(species.camouflage) };
        members.any(|(profile, vitals)| {
            if !vitals.can_fight()
                || vitals.has(Condition::Blind) && profile.profession != Profession::Monk
            {
                return false;
            }
            let stats = profile.stats(&vitals.enchantments);
            sight_threshold(
                angle,
                stats.attribute(Attribute::Senses) as i32,
                stats.skill(Skill::Scouting),
                i32::from(species.level),
                concealment,
                self.far,
                distance,
            ) * if self.searching { 0.6667 } else { 1.0 }
                >= distance
        })
    }
}

/// Retail `ComputeSightThreshold`: how far an observer facing away by
/// `angle` (radians) sees, given its senses, a ranged bonus percent and the
/// penalty source (the party's lowest level, doubled); within 45 degrees, or
/// at no distance, the whole `far` clip.
pub(crate) fn sight_threshold(
    angle: f32,
    senses: i32,
    bonus: u32,
    penalty: i32,
    concealment: u32,
    far: f32,
    distance: f32,
) -> f32 {
    let mut percent = if angle < std::f32::consts::FRAC_PI_4 || distance == 0.0 {
        100
    } else {
        ((senses - penalty * 2) as f32 * ((far - distance) / distance)) as i32
    };
    percent -= concealment as i32 * 10;
    if bonus != 0 {
        percent = (bonus as i32 + 100) * percent / 100;
    }
    percent.clamp(2, 100) as f32 * far * 0.01
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        content::spells::SpellId,
        game::{
            characters::{Gender, Lasting, Race},
            magic::enchantments::{Enchantment, PartyEffects},
            world::population::ScaledBody,
        },
    };
    use bevy::ecs::system::SystemState;

    #[test]
    fn shadow_hound_extends_peripheral_sight_but_preserves_blindness_and_camouflage_rules() {
        let mut world = World::new();
        world.init_resource::<avian3d::collider_tree::ColliderTrees>();
        let mut state = SystemState::<WorldObstacles>::new(&mut world);
        let obstacles = state.get(&world).unwrap();
        let mut profile = Character::new("Lookout", Race::Human, Gender::Male, Profession::Mage);
        profile.attributes[Attribute::Senses] = 40;
        let mut vitals = Vitals::new(&profile);
        let monster = Monster {
            enchantments: default(),
            location: 1,
            species: 0,
            hp: 10,
            max_hp: 10,
            stamina: 1,

            max_stamina: 1,
            spell_points: 0,
            conditions: vec![],
            body: ScaledBody { scale: 1.0, radius: 0.5, eye_height: 1.0, speed: 0.0 },
        };
        let mut species = Species { level: 30, ..default() };
        let view = View { eye: Vec3::Y * 2.0, forward: Vec3::NEG_Z, far: 60.0, searching: false };
        let sees = |profile: &Character,
                    vitals: &Vitals,
                    effects: &PartyEffects,
                    species: &Species,
                    in_combat| {
            view.sees(
                std::iter::once((profile, vitals)),
                effects.0.has(SpellId::SHADOW_HOUND),
                &monster,
                species,
                Vec3::Z * 30.0,
                in_combat,
                &obstacles,
            )
        };
        let mut effects = PartyEffects::default();
        assert!(!sees(&profile, &vitals, &effects, &species, false));
        effects.0.apply(Enchantment {
            spell: SpellId::SHADOW_HOUND,
            power: 1,
            boost: 0,
            turns: 1,
            absorption: 0,
        });
        assert!(sees(&profile, &vitals, &effects, &species, false));
        species.camouflage = 8;
        assert!(!sees(&profile, &vitals, &effects, &species, false));
        assert!(sees(&profile, &vitals, &effects, &species, true));
        species.camouflage = 0;
        vitals.inflict(Condition::Blind, Lasting::Indefinite);
        assert!(!sees(&profile, &vitals, &effects, &species, false));
        profile.profession = Profession::Monk;
        assert!(sees(&profile, &vitals, &effects, &species, false));
        let frontal = Vec3::NEG_Z * 50.0;
        assert!(view.sees(
            std::iter::once((&profile, &Vitals::new(&profile))),
            true,
            &monster,
            &species,
            frontal,
            false,
            &obstacles
        ));
        assert!(
            !View { searching: true, ..view }.sees(
                std::iter::once((&profile, &Vitals::new(&profile))),
                true,
                &monster,
                &species,
                frontal,
                false,
                &obstacles
            ),
            "searching reduces the deterministic sight threshold even under Shadow Hound"
        );
        effects.0.age(1);
        assert!(!sees(&profile, &vitals, &effects, &species, false));
    }
}
