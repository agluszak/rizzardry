//! An actor's figures, each written by one system from what the actor is
//! made of, when that changes. A figure is what the actor has by nature,
//! what it has by training, what it wears, what is laid on it and what
//! ails it, taken together. The two families of the original's formulas
//! stay; which applies is a matter of what the actor carries, a calling or
//! not, never of what kind of entity it is.

use bevy::prelude::*;
use enum_map::EnumMap;
use enumset::EnumSet;

use super::arms::{Hands, Held};
use super::modifiers::Modifiers;
use super::sources::{Abilities, Drained, Frame, Kind, NaturalArms, Nature, Traits};
use super::time::Mending;
use super::{Attacks, Attributes, Level, MissileShield, Reduction, Resistances, Skills};
use crate::content::characters::{Attribute, Skill, Trait};
use crate::game::characters::{
    Birth, Calling, Condition, Conditions, Schooling, Trained, Training, Vitals, scale_by_level,
    trained_stamina, traits_of,
};
use crate::game::items::Redressed;
use crate::game::items::{Burden, Items};

/// The level a trait's strength grows with: the level in the profession
/// followed now for one who trained, its own level otherwise (retail
/// `ScaleValueByProfessionLevel`, `ScaleValueByMonsterLevel`).
fn mastery(calling: Option<&Calling>, level: &Level) -> u8 {
    calling.map_or(level.0 as u8, Calling::mastery)
}

/// One who trained has the traits of their race and the profession they
/// follow now, and the level of all their professions together.
pub(super) fn follow_calling(
    mut trained: Query<(&Calling, &Birth, &mut Traits, &mut Level), Changed<Calling>>,
) {
    for (calling, birth, mut traits, mut level) in &mut trained {
        traits.set_if_neq(Traits(traits_of(birth.race, calling)));
        level.set_if_neq(Level(u32::from(calling.level())));
    }
}

#[allow(clippy::type_complexity)]
pub(super) fn attributes(
    mut actors: Query<
        (&Abilities, &Modifiers, &Conditions, &Traits, &Level, Option<&Calling>, &mut Attributes),
        Or<(
            Changed<Abilities>,
            Changed<Modifiers>,
            Changed<Conditions>,
            Changed<Traits>,
            Changed<Calling>,
        )>,
    >,
) {
    for (base, modifiers, conditions, traits, level, calling, mut written) in &mut actors {
        // Only one who trained makes anything of a trait that eases a
        // condition.
        let eases = calling.map(|calling| (traits, mastery(Some(calling), level)));
        written.set_if_neq(Attributes::affected(base, &modifiers.laid, conditions, eases));
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

/// The most health and stamina: what the actor was born with and what its
/// training gives at its Vitality, less what is drained; health never under
/// one. What is left follows ([`Vitals::set_most`]).
#[allow(clippy::type_complexity)]
pub(super) fn maxima(
    mut actors: Query<
        (&mut Vitals, Option<&Frame>, Option<&Calling>, &Level, &Attributes, &Drained),
        (
            Or<(With<Frame>, With<Calling>)>,
            Or<(
                Changed<Frame>,
                Changed<Calling>,
                Changed<Level>,
                Changed<Attributes>,
                Changed<Drained>,
            )>,
        ),
    >,
) {
    for (mut vitals, frame, calling, level, a, drained) in &mut actors {
        let frame = frame.copied().unwrap_or_default();
        let health = frame.health + calling.map_or(0, |c| c.health(a[Attribute::Vitality]));
        let stamina = frame.stamina + calling.map_or(0, |_| trained_stamina(level.0, a));
        let health = health.max(drained.health + 1) - drained.health;
        let stamina = stamina.max(drained.stamina) - drained.stamina;
        if (vitals.health.max, vitals.stamina.max) != (health, stamina) {
            vitals.set_most(health, stamina);
        }
    }
}

#[allow(clippy::type_complexity)]
pub(super) fn resistances(
    mut actors: Query<(
        Entity,
        (&Nature, Option<&Kind>, &Traits, &Level),
        Option<Schooling>,
        (&Attributes, &Skills, &Modifiers),
        &mut Resistances,
    )>,
    changed: Query<
        (),
        Or<(
            Changed<Nature>,
            Changed<Traits>,
            Changed<Level>,
            Changed<Calling>,
            Changed<Attributes>,
            Changed<Skills>,
            Changed<Modifiers>,
        )>,
    >,
    (items, mut redressed): (Items, Redressed),
) {
    let dressed = redressed.wearers();
    for (entity, (nature, kind, traits, level), trained, (a, skills, modifiers), mut written) in
        &mut actors
    {
        if !changed.contains(entity) && !dressed.contains(&entity) {
            continue;
        }
        let trained = trained.as_ref().map(|schooling| schooling.view());
        let own = crate::game::characters::resistance::resistances(
            (nature, trained),
            a,
            skills,
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
            immune: kind.map_or(&[], |kind| crate::game::hits::defences::immune(kind.0)),
            wards,
            death_immune: nature.death_immune,
            kind: kind.map(|kind| kind.0),
            caster: EnumMap::from_fn(|book| match trained {
                Some(c) => crate::game::magic::total_caster_level(c.calling, EnumSet::only(book))
                    .max(0) as u32,
                None => level.0,
            }),
        });
    }
}

/// The percent cut off every blow at it: by nature, by training and by the
/// spells on it, at most all of it; and the chance a Missile Shield turns a
/// shot aside.
#[allow(clippy::type_complexity)]
pub(super) fn reduction(
    mut actors: Query<
        (
            &Nature,
            &Modifiers,
            Option<Schooling>,
            (&Attributes, &Skills),
            (&mut Reduction, &mut MissileShield),
        ),
        Or<(
            Changed<Nature>,
            Changed<Traits>,
            Changed<Modifiers>,
            Changed<Calling>,
            Changed<Attributes>,
            Changed<Skills>,
        )>,
    >,
) {
    for (nature, modifiers, trained, (a, skills), (mut reduction, mut shield)) in &mut actors {
        let trained =
            trained.as_ref().map_or(0, |schooling| trained_reduction(schooling.view(), a, skills));
        reduction.set_if_neq(Reduction(
            (nature.reduction + trained + modifiers.laid.reduction).min(100),
        ));
        shield.set_if_neq(MissileShield(modifiers.laid.missiles));
    }
}

/// Percent training takes off every blow: a Dwarf's toughness, a Monk's
/// training and Iron Skin (retail `RecalculateCharacterState`).
pub fn trained_reduction(c: Trained, a: &Attributes, skills: &Skills) -> i32 {
    let mut reduction = 0;
    if c.traits.contains(Trait::DwarfDamageResistance) {
        reduction += a[Attribute::Vitality] as i32 / 10;
    }
    if c.traits.contains(Trait::MonkDamageResistance) {
        reduction += c.calling.scale(15.0) as i32;
    }
    if c.can_use(Skill::IronSkin) {
        reduction += (skills[Skill::IronSkin] >> 2) as i32 + 5;
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

#[allow(clippy::type_complexity)]
pub(super) fn attacks(
    mut actors: Query<(
        Entity,
        (&Nature, Option<&NaturalArms>, &Traits, &Level, &Modifiers, &Conditions),
        Option<(Schooling, &Burden)>,
        (&Attributes, &Skills),
        &mut Attacks,
    )>,
    changed: Query<
        (),
        Or<(
            Changed<Nature>,
            Changed<NaturalArms>,
            Changed<Traits>,
            Changed<Level>,
            Changed<Modifiers>,
            Changed<Conditions>,
            Changed<Calling>,
            Changed<Training>,
            Changed<Burden>,
            Changed<Attributes>,
            Changed<Skills>,
        )>,
    >,
    (items, hands, mut redressed): (Items, Query<Held>, Redressed),
) {
    let dressed = redressed.wearers();
    for (
        entity,
        (nature, natural, traits, level, modifiers, conditions),
        trained,
        (a, skills),
        mut written,
    ) in &mut actors
    {
        if !changed.contains(entity) && !dressed.contains(&entity) {
            continue;
        }
        let hit = modifiers.laid.to_hit + conditions.to_hit();
        let scale = mastery(trained.as_ref().map(|(schooling, _)| schooling.view().calling), level);
        let mut made = match trained {
            // By nature: what it strikes with while holding nothing.
            None => {
                let arms: Vec<_> = natural
                    .into_iter()
                    .flat_map(|arms| &arms.0)
                    .map(|arm| super::Arm { rating: arm.rating + 5 * hit, ..arm.clone() })
                    .collect();
                let first = natural.and_then(|arms| arms.0.first());
                Attacks {
                    initiative: nature.initiative,
                    press: a[Attribute::Speed] as i32 + first.map_or(0, |arm| arm.rating) + hit * 5,
                    tires_little: first.is_some_and(|arm| arm.modern),
                    arms,
                    ..default()
                }
            }
            // By training: what is in its hands.
            Some((schooling, burden)) => {
                let c = schooling.view();
                let held = Hands::of(|slot| {
                    items.worn(entity, slot).and_then(|item| hands.get(item).ok())
                });
                Attacks {
                    initiative: nature.initiative
                        + trained_initiative((level.0, c.training), a, skills, burden.load),
                    ..held.attacks(c, skills, a, burden.load, hit)
                }
            }
        };
        made.blind_fighting = traits.has(Trait::EffectiveWhileBlind).then_some(scale);
        made.knock_out = traits.has(Trait::Knockout).then_some(scale);
        made.backstab = scale_by_level(scale, 100.0) as i32;
        made.keen_sight = nature.keen_sight;
        made.stride = nature.stride;
        made.hasted = modifiers.laid.stride;
        written.set_if_neq(made);
    }
}

#[allow(clippy::type_complexity)]
pub(super) fn mending(
    mut actors: Query<(Entity, &Nature, &Traits, &Level, Option<&Calling>, &mut Mending)>,
    changed: Query<(), Or<(Changed<Nature>, Changed<Traits>, Changed<Level>, Changed<Calling>)>>,
    (items, mut redressed): (Items, Redressed),
) {
    let dressed = redressed.wearers();
    for (entity, nature, traits, level, calling, mut written) in &mut actors {
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
        let scale = mastery(calling, level);
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
            // One who trained gets their wind back by the breath.
            breathes: calling.is_some(),
            hale: scaled(Trait::HealthRegeneration, 16.67),
            tireless: scaled(Trait::StaminaRegeneration, 3.3),
            magic_rate,
        });
    }
}
