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
}
impl View {
    /// Retail party-side sight. Shadow Hound supplies the full-range override,
    /// while blindness, the target's concealment and solid scenery still apply.
    pub fn sees<'a>(
        self,
        mut members: impl Iterator<Item = (&'a Character, &'a Vitals)>,
        shadow_hound: bool,
        monster: &Monster,
        species: &Species,
        at: Vec3,
        in_combat: bool,
        obstacles: &WorldObstacles,
    ) -> bool {
        let distance = (at.distance(self.eye - Vec3::Y * crate::content::EYE_HEIGHT)
            - monster.body.radius
            - crate::game::party::body::RADIUS)
            .max(0.0);
        if monster.is_dead() || distance > self.far {
            return false;
        }
        let clear = Dir3::new(at - self.eye).map_or(true, |direction| {
            obstacles.first_hit(self.eye, direction, self.eye.distance(at)).is_none()
        });
        if !clear {
            return false;
        }
        let angle = if shadow_hound {
            0.0
        } else {
            self.forward.with_y(0.0).angle_between((at - self.eye).with_y(0.0))
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
            ) >= distance
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
            mana: 0,
            max_stamina: 1,
            conditions: vec![],
            body: ScaledBody { scale: 1.0, radius: 0.5, eye_height: 1.0, speed: 0.0 },
        };
        let mut species = Species { level: 30, ..default() };
        let view = View { eye: Vec3::Y * 2.0, forward: Vec3::NEG_Z, far: 60.0 };
        let sees = |profile: &Character,
                    vitals: &Vitals,
                    effects: &PartyEffects,
                    species: &Species,
                    in_combat| {
            view.sees(
                std::iter::once((profile, vitals)),
                effects.0.get(SpellId::SHADOW_HOUND).is_some(),
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
        effects.0.age(1);
        assert!(!sees(&profile, &vitals, &effects, &species, false));
    }
}
