//! An actor's figures, each written by one system from what the actor is
//! made of, when that changes. A figure is what the actor has by nature,
//! what it has by training, what it wears, what is laid on it and what
//! ails it, taken together. The two families of the original's formulas
//! stay; which applies is a matter of what the actor carries, a calling or
//! not, never of what kind of entity it is.

use bevy::prelude::*;
use enum_map::EnumMap;
use enumset::EnumSet;

use super::modifiers::Modifiers;
use super::sources::{Abilities, Drained, Frame, Nature, Traits};
use super::time::Mending;
use super::{
    Agility, Attributes, CasterLevels, Fatigue, Level, Mastery, MissileShield, Reduction,
    Resistances, Skills, TrainedMaxima, TrainedReduction, TrainedResistance, UsableSkills,
};
use crate::content::characters::{Attribute, Realm, Skill, Trait};
use crate::game::characters::{
    Birth, Calling, Condition, Conditions, Training, Vitals, scale_by_level, trained_stamina,
    traits_of,
};
use crate::game::items::Redressed;
use crate::game::items::{Burden, Items};
use crate::game::magic::{LearnedSpells, SpellPoints, max_points};

/// One who trained has the traits of their race and the profession they
/// follow now, the level of all their professions together, and the level
/// in the one followed now as their mastery.
pub(super) fn follow_calling(
    mut trained: Query<(&Calling, &Birth, &mut Traits, &mut Level, &mut Mastery), Changed<Calling>>,
) {
    for (calling, birth, mut traits, mut level, mut mastery) in &mut trained {
        traits.set_if_neq(Traits(traits_of(birth.race, calling)));
        level.set_if_neq(Level(u32::from(calling.level())));
        mastery.set_if_neq(Mastery(calling.mastery()));
    }
}

#[allow(clippy::type_complexity)]
pub(super) fn attributes(
    mut actors: Query<
        (&Abilities, &Modifiers, &Conditions, &Traits, &Mastery, &mut Attributes),
        Or<(
            Changed<Abilities>,
            Changed<Modifiers>,
            Changed<Conditions>,
            Changed<Traits>,
            Changed<Calling>,
        )>,
    >,
) {
    for (base, modifiers, conditions, traits, mastery, mut written) in &mut actors {
        // Only a member has a trait that eases a condition; a monster's
        // conditions are never eased (`Gameplay Mods.cpp:417`).
        written.set_if_neq(Attributes::affected(
            base,
            &modifiers.laid,
            conditions,
            (traits, mastery.0),
        ));
    }
}

#[allow(clippy::type_complexity)]
pub(super) fn skills(
    mut trained: Query<
        (&Training, &Calling, &Conditions, &mut Skills),
        Or<(Changed<Training>, Changed<Calling>, Changed<Conditions>)>,
    >,
) {
    for (training, calling, conditions, mut written) in &mut trained {
        written.set_if_neq(Skills::affected(training, calling, conditions));
    }
}

/// The skills one who trained may use (retail `IsCharacterSkillAvailable`)
/// follow their calling, abilities, traits and the spells they know.
#[allow(clippy::type_complexity)]
pub(super) fn usable_skills(
    mut trained: Query<
        (&Calling, &Abilities, &Traits, &LearnedSpells, &mut UsableSkills),
        Or<(Changed<Calling>, Changed<Abilities>, Changed<Traits>, Changed<LearnedSpells>)>,
    >,
) {
    for (calling, abilities, traits, learned, mut written) in &mut trained {
        let trained = (calling, abilities, traits.0);
        let usable = EnumSet::all()
            .iter()
            .filter(|&skill| crate::game::magic::skill_available(trained, learned, skill))
            .collect();
        written.set_if_neq(UsableSkills(usable));
    }
}

/// Agility follows the attributes.
pub(super) fn agility(mut actors: Query<(&Attributes, &mut Agility), Changed<Attributes>>) {
    for (a, mut agility) in &mut actors {
        agility.set_if_neq(Agility(a.agility()));
    }
}

/// Fatigue follows the share of stamina spent: 0 under half, then 5, 10, 20
/// and 40 from 50, 70, 85 and 95 percent (retail `FatigueArmorPenalty`).
pub(super) fn fatigue(mut actors: Query<(&Vitals, &mut Fatigue), Changed<Vitals>>) {
    for (vitals, mut fatigue) in &mut actors {
        fatigue.set_if_neq(Fatigue::of_spent(vitals.stamina.spent_percent()));
    }
}

/// What the eyes are worth follows the Senses, what the kind adds, and
/// whether it can fight and see: shut when it cannot fight, or is blind and
/// does not fight blind.
#[allow(clippy::type_complexity)]
pub(super) fn eyes(
    mut actors: Query<(
        (&Conditions, Has<super::CanFight>, Has<super::ActsThoughBlind>),
        (&Attributes, &Nature),
        &mut super::Eyes,
    )>,
) {
    for ((conditions, can_fight, fights_blind), (a, nature), mut eyes) in &mut actors {
        let blinded = conditions.has(Condition::Blind) && !fights_blind;
        let now = super::Eyes::of(a[Attribute::Senses], nature.keen_sight, can_fight && !blinded);
        eyes.set_if_neq(now);
    }
}

/// How well an actor presses a blow past a guard, or guards against one
/// (retail `GetCharacterProtectionScore`, `GetMonsterProtectionScore`): its
/// Speed, its first arm's rating and what its spells and conditions add to
/// its aim, less its fatigue, which tells half as much when that arm is
/// modern.
#[allow(clippy::type_complexity)]
pub(super) fn pressing(
    mut actors: Query<(
        &Attributes,
        &super::AimBonus,
        &Fatigue,
        &super::arm::Arms,
        &mut super::Pressing,
    )>,
    arms: Query<(&super::arm::Rating, Has<super::arm::Modern>)>,
) {
    for (a, bonus, fatigue, own, mut pressing) in &mut actors {
        let first = own.entities().first().and_then(|&arm| arms.get(arm).ok());
        let (rating, modern) = first.map_or((0, false), |(rating, modern)| (rating.0, modern));
        let tired = if modern { fatigue.0 / 2 } else { fatigue.0 };
        let press = a[Attribute::Speed] as i32 + rating + bonus.0;
        pressing.set_if_neq(super::Pressing(press - tired));
    }
}

/// Being blinded follows what ails it.
pub(super) fn blind(
    actors: Query<(Entity, &Conditions, Has<super::Blinded>), Changed<Conditions>>,
    mut commands: Commands,
) {
    for (actor, conditions, was) in &actors {
        match (conditions.has(Condition::Blind), was) {
            (true, false) => {
                commands.entity(actor).insert(super::Blinded);
            }
            (false, true) => {
                commands.entity(actor).remove::<super::Blinded>();
            }
            _ => {}
        }
    }
}

/// A member keeps their Alchemy under a silence while they have the skill.
pub(super) fn keeps_alchemy(
    actors: Query<(Entity, &Skills, Has<super::KeepsAlchemy>), (With<Calling>, Changed<Skills>)>,
    mut commands: Commands,
) {
    for (actor, skills, was) in &actors {
        match (skills[crate::content::characters::Skill::Alchemy] > 0, was) {
            (true, false) => {
                commands.entity(actor).insert(super::KeepsAlchemy);
            }
            (false, true) => {
                commands.entity(actor).remove::<super::KeepsAlchemy>();
            }
            _ => {}
        }
    }
}

/// The most spell points each realm can hold, by the spells learned
/// ([`crate::game::magic::max_points`]); what is spent stays spent.
#[allow(clippy::type_complexity)]
pub(super) fn spell_points(
    mut casters: Query<
        (&LearnedSpells, &Level, &Attributes, &Skills, &mut SpellPoints),
        Or<(Changed<LearnedSpells>, Changed<Level>, Changed<Attributes>, Changed<Skills>)>,
    >,
) {
    for (learned, level, a, skills, mut points) in &mut casters {
        let most = max_points(level.0, a[Attribute::Piety], |skill| skills[skill], learned);
        if EnumSet::<Realm>::all().iter().any(|realm| points.most(realm) != most[realm]) {
            points.set_most(most);
        }
    }
}

/// What training gives one who trained to their most health and stamina:
/// their calling's health at their Vitality, and stamina by their level and
/// Strength, Piety and Vitality.
pub(super) fn trained_maxima(
    mut trained: Query<
        (&Calling, &Level, &Attributes, &mut TrainedMaxima),
        Or<(Changed<Calling>, Changed<Level>, Changed<Attributes>)>,
    >,
) {
    for (calling, level, a, mut written) in &mut trained {
        written.set_if_neq(TrainedMaxima {
            health: calling.health(a[Attribute::Vitality]),
            stamina: trained_stamina(level.0, a),
        });
    }
}

/// The most health and stamina: what the actor was born with and what its
/// training gives, less what is drained; health never under one. What is
/// left follows ([`Vitals::set_most`]).
pub(super) fn maxima(
    mut actors: Query<
        (&mut Vitals, &Frame, &TrainedMaxima, &Drained),
        Or<(Changed<Frame>, Changed<TrainedMaxima>, Changed<Drained>)>,
    >,
) {
    for (mut vitals, frame, trained, drained) in &mut actors {
        let health = frame.health + trained.health;
        let stamina = frame.stamina + trained.stamina;
        let health = health.max(drained.health + 1) - drained.health;
        let stamina = stamina.max(drained.stamina) - drained.stamina;
        if (vitals.health.max, vitals.stamina.max) != (health, stamina) {
            vitals.set_most(health, stamina);
        }
    }
}

/// What training adds to the resistance of one who trained, in each realm.
#[allow(clippy::type_complexity)]
pub(super) fn trained_resistance(
    mut trained: Query<
        (&Calling, &UsableSkills, &Attributes, &Skills, &mut TrainedResistance),
        Or<(Changed<Calling>, Changed<UsableSkills>, Changed<Attributes>, Changed<Skills>)>,
    >,
) {
    for (calling, usable, a, skills, mut written) in &mut trained {
        let iron_will = usable.0.contains(Skill::IronWill).then(|| skills[Skill::IronWill]);
        let own = calling.profession.row().resistance;
        let keen = (a[Attribute::Intelligence], a[Attribute::Piety]);
        let resistance = EnumMap::from_fn(|realm: Realm| {
            crate::game::characters::resistance::trained(
                realm,
                skills[realm.skill()],
                iron_will,
                own,
                keen,
            )
        });
        written.set_if_neq(TrainedResistance(resistance));
    }
}

/// How strongly one who trained casts: the caster level of the profession
/// followed now, and that of each other profession held, with its books
/// (retail `GetTotalCasterLevel`; only what is above nothing counts).
pub(super) fn caster_levels(mut trained: Query<(&Calling, &mut CasterLevels), Changed<Calling>>) {
    for (calling, mut written) in &mut trained {
        written.set_if_neq(CasterLevels::of(calling));
    }
}

#[allow(clippy::type_complexity)]
pub(super) fn resistances(
    mut actors: Query<(
        Entity,
        (&Nature, &Traits),
        &TrainedResistance,
        (&Attributes, &Modifiers),
        &mut Resistances,
    )>,
    changed: Query<
        (),
        Or<(
            Changed<Nature>,
            Changed<Traits>,
            Changed<TrainedResistance>,
            Changed<Attributes>,
            Changed<Modifiers>,
        )>,
    >,
    (items, mut redressed): (Items, Redressed),
) {
    let dressed = redressed.wearers();
    for (entity, (nature, traits), trained, (a, modifiers), mut written) in &mut actors {
        if !changed.contains(entity) && !dressed.contains(&entity) {
            continue;
        }
        let own = crate::game::characters::resistance::resistances(
            &nature.resistance,
            &trained.0,
            a[Attribute::Vitality],
            |slot| items.protection(entity, slot),
        );
        let mut wards = Vec::new();
        if traits.has(Trait::AndroidConditionImmunity) {
            use Condition::*;
            wards.extend([Afraid, Diseased, Irritated, Nauseated, Poisoned, Asleep]);
        }
        if traits.has(Trait::Fearless) {
            wards.push(Condition::Afraid);
        }
        if traits.has(Trait::MentalConditionImmunity) {
            wards.extend([Condition::Insane, Condition::Turncoat]);
        }
        written.set_if_neq(Resistances {
            realms: EnumMap::from_fn(|realm| {
                (own[realm].total + modifiers.laid.resistance[realm]).min(100)
            }),
            wards,
        });
    }
}

/// The percent cut off every blow at it: by nature, by training and by the
/// spells on it, at most all of it; and the chance a Missile Shield turns a
/// shot aside.
#[allow(clippy::type_complexity)]
pub(super) fn reduction(
    mut actors: Query<
        (&Nature, &Modifiers, &TrainedReduction, (&mut Reduction, &mut MissileShield)),
        Or<(Changed<Nature>, Changed<Modifiers>, Changed<TrainedReduction>)>,
    >,
) {
    for (nature, modifiers, trained, (mut reduction, mut shield)) in &mut actors {
        reduction.set_if_neq(Reduction(
            (nature.reduction + trained.0 + modifiers.laid.reduction).min(100),
        ));
        shield.set_if_neq(MissileShield(modifiers.laid.missiles));
    }
}

/// What training takes off every blow at one who trained.
#[allow(clippy::type_complexity)]
pub(super) fn trained_reduction_of(
    mut trained: Query<
        (&Calling, &Traits, &UsableSkills, (&Attributes, &Skills), &mut TrainedReduction),
        Or<(
            Changed<Calling>,
            Changed<Traits>,
            Changed<UsableSkills>,
            Changed<Attributes>,
            Changed<Skills>,
        )>,
    >,
) {
    for (calling, traits, usable, (a, skills), mut written) in &mut trained {
        let iron_skin = usable.0.contains(Skill::IronSkin).then(|| skills[Skill::IronSkin]);
        let now = trained_reduction(
            traits.0,
            calling.scale(15.0) as i32,
            iron_skin,
            a[Attribute::Vitality],
        );
        written.set_if_neq(TrainedReduction(now));
    }
}

/// Percent training takes off every blow (retail `RecalculateCharacterState`):
/// a Dwarf's toughness, a tenth of Vitality; a Monk's training, at its scale
/// of 15 by their level in the profession (`monk`); and Iron Skin, a quarter
/// of the skill and five, if it may be used.
pub fn trained_reduction(
    traits: EnumSet<Trait>,
    monk: i32,
    iron_skin: Option<u32>,
    vitality: u32,
) -> i32 {
    let mut reduction = 0;
    if traits.contains(Trait::DwarfDamageResistance) {
        reduction += vitality as i32 / 10;
    }
    if traits.contains(Trait::MonkDamageResistance) {
        reduction += monk;
    }
    if let Some(skill) = iron_skin {
        reduction += (skill >> 2) as i32 + 5;
    }
    reduction
}

/// When one who trained acts in a round: `(level + 1) / 2 + SEN / 5 + SPD /
/// 5 - 10`, plus Snake Speed, less 1, 2, 4 or 8 by load (retail
/// `CalcInitiative`).
pub fn trained_initiative(
    (level, training): (u32, &Training),
    a: &Attributes,
    skills: &Skills,
    load: crate::game::characters::Load,
) -> i32 {
    let mut initiative =
        (level as i32 + 1) / 2 + a[Attribute::Senses] as i32 / 5 + a[Attribute::Speed] as i32 / 5
            - 10;
    if training.0[Skill::SnakeSpeed] > 0 {
        initiative += skills[Skill::SnakeSpeed] as i32 / 10 + 1;
    }
    initiative - load.penalty()
}

/// What an actor's spells and conditions add to its aim, five a point.
pub(super) fn aim_bonus(
    mut actors: Query<
        (&Modifiers, &Conditions, &mut super::AimBonus),
        Or<(Changed<Modifiers>, Changed<Conditions>)>,
    >,
) {
    for (modifiers, conditions, mut bonus) in &mut actors {
        bonus.set_if_neq(super::AimBonus(5 * (modifiers.laid.to_hit + conditions.to_hit())));
    }
}

/// What training adds to when one who trained acts.
#[allow(clippy::type_complexity)]
pub(super) fn trained_initiative_of(
    mut trained: Query<
        (&Training, &Burden, &Level, &Attributes, &Skills, &mut super::TrainedInitiative),
        Or<(
            Changed<Training>,
            Changed<Burden>,
            Changed<Level>,
            Changed<Attributes>,
            Changed<Skills>,
        )>,
    >,
) {
    for (training, burden, level, a, skills, mut written) in &mut trained {
        let now = trained_initiative((level.0, training), a, skills, burden.load);
        written.set_if_neq(super::TrainedInitiative(now));
    }
}

/// When an actor acts: its nature's initiative and what training adds.
pub(super) fn initiative(
    mut actors: Query<
        (&Nature, &super::TrainedInitiative, &mut super::Initiative),
        Or<(Changed<Nature>, Changed<super::TrainedInitiative>)>,
    >,
) {
    for (nature, trained, mut initiative) in &mut actors {
        initiative.set_if_neq(super::Initiative(nature.initiative + trained.0));
    }
}

/// For one who trained, fighting blind (which leaves them unhindered by
/// blindness) and knocking out follow the traits, at their mastery; the
/// backstab follows their mastery. A monster's are written as it is made.
#[allow(clippy::type_complexity)]
pub(super) fn trait_arms(
    actors: Query<
        (
            Entity,
            (&Traits, &Mastery),
            (Option<&super::BlindFighting>, Option<&super::KnockOut>, &super::Backstab),
        ),
        (With<Calling>, Or<(Changed<Traits>, Changed<Mastery>)>),
    >,
    mut commands: Commands,
) {
    for (actor, (traits, mastery), (blind, knock_out, backstab)) in &actors {
        let mut actor = commands.entity(actor);
        let level = mastery.0;
        let fights_blind =
            traits.has(Trait::EffectiveWhileBlind).then_some(super::BlindFighting(level));
        if fights_blind != blind.copied() {
            match fights_blind {
                Some(fights) => actor.insert((fights, super::ActsThoughBlind)),
                None => actor.remove::<(super::BlindFighting, super::ActsThoughBlind)>(),
            };
        }
        let knocks = traits.has(Trait::Knockout).then_some(super::KnockOut(level));
        if knocks != knock_out.copied() {
            match knocks {
                Some(knocks) => actor.insert(knocks),
                None => actor.remove::<super::KnockOut>(),
            };
        }
        let stabs = super::Backstab(scale_by_level(level, 100.0) as i32);
        if stabs != *backstab {
            actor.insert(stabs);
        }
    }
}

#[allow(clippy::type_complexity)]
pub(super) fn mending(
    mut actors: Query<(Entity, &Nature, &Traits, &Mastery, &mut Mending)>,
    changed: Query<(), Or<(Changed<Nature>, Changed<Traits>, Changed<Mastery>)>>,
    (items, mut redressed): (Items, Redressed),
) {
    let dressed = redressed.wearers();
    for (entity, nature, traits, mastery, mut written) in &mut actors {
        if !changed.contains(entity) && !dressed.contains(&entity) {
            continue;
        }
        let mut worn = [0; 3];
        for (slot, _) in items.in_use(entity) {
            if let Some((protection, _)) = items.protection(entity, slot) {
                for (sum, &bonus) in worn.iter_mut().zip(&protection.regeneration) {
                    *sum += i32::from(bonus);
                }
            }
        }
        let scale = mastery.0;
        let scaled = |ability, value| traits.has(ability).then(|| scale_by_level(scale, value));
        let mut magic_rate = 1.0;
        if traits.has(Trait::FastMagicRecovery) {
            magic_rate *= 1.25;
        }
        if traits.has(Trait::LizardmanMagicRecovery) {
            magic_rate *= 0.75;
        }
        written.set_if_neq(Mending {
            health: nature.mending.0 + worn[0],
            stamina: nature.mending.1 + worn[1],
            magic: worn[2],
            breathes: nature.breathes,
            hale: scaled(Trait::HealthRegeneration, 16.67),
            tireless: scaled(Trait::StaminaRegeneration, 3.3),
            magic_rate,
        });
    }
}
