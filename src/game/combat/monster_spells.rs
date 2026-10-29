//! Monsters casting in a fight (retail `UpdateMonsterAI`'s spell branch,
//! `IsSpellUsableByMonster`, `CollectMonsterSpellTargets`,
//! `MonsterSpellTargetOK`, `ChooseMonsterSpell`,
//! `ChooseMonsterSpellPowerLevel`, `MonsterCastsSpell`).
//!
//! A monster spends nothing to cast: its spell budget stands in for a
//! caster's skill in the failure roll, and the power it picks is the most
//! that budget comfortably covers.

use bevy::prelude::*;

use crate::content::species::{MonsterKind, Species};
use crate::content::spells::{
    MAX_POWER, SpellCatalog, SpellDefinition, SpellId, Spellbook, Targeting,
};
use crate::game::characters::Condition;
use crate::game::magic::casting::Report;
use crate::game::magic::effects::{self, Field, Standing, Victim};
use crate::game::magic::{LearnedSpells, Outcome, SpellPoints, Strength};
use crate::game::rng::GameRng;
use crate::game::world::hostility::Stance;
use crate::game::world::population::Monster;

/// How much each of the ten slots weighs in the choice (retail
/// `g_spell_cast_weights`).
const SLOT_WEIGHTS: [u32; 10] = [5, 5, 5, 10, 10, 10, 10, 15, 15, 15];

/// The kinds whose alchemy a silence leaves them (retail
/// `IsSpellBlockedForMonster`).
const SILENT_ALCHEMISTS: [MonsterKind; 3] =
    [MonsterKind::Ranger, MonsterKind::Alchemist, MonsterKind::Ninja];

/// Whom a monster's spell is cast at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SpellAim {
    /// Itself or another monster in the fight.
    Monster(Entity),
    Member(Entity),
    /// The party as a whole: a group, cone or blast spell.
    Party,
    /// Every foe it reaches: an all-enemies spell.
    Foes,
}

/// Someone in the fight as a casting monster sees them.
#[derive(Clone, Debug)]
pub(super) struct Someone {
    pub entity: Entity,
    /// Where they stand (a member at the party's point).
    pub at: Vec3,
    /// How far they are from the caster, beyond both bodies.
    pub from_caster: f32,
    /// What they lack.
    pub standing: Standing,
    /// How the caster stands toward them (retail
    /// `MonsterVsCharDisposition`, `MonsterHostility`).
    pub stance: Stance,
}

/// What a casting monster sees of the fight.
pub(super) struct Scene {
    pub caster: Entity,
    /// Where the caster stands, and the party's point.
    pub from: Vec3,
    pub party_at: Vec3,
    /// The living party.
    pub members: Vec<Someone>,
    /// The living monsters in the fight the caster has a clear line to,
    /// itself among them (standing friendly to itself).
    pub monsters: Vec<Someone>,
    /// How far the party is from the caster.
    pub party: f32,
    /// Whether the caster sees the party now, as engagement judges it
    /// (retail `IsVisibleUnderConditions`).
    pub sees_party: bool,
}

/// Whether `at` lies in a cone from `from` toward `toward`: within `reach`
/// and 45 degrees of its line.
fn in_cone(from: Vec3, toward: Vec3, reach: f32, at: Vec3, distance: f32) -> bool {
    let line = (toward - from).with_y(0.0);
    let to = (at - from).with_y(0.0);
    distance <= reach && (to == Vec3::ZERO || to.angle_between(line) < std::f32::consts::FRAC_PI_4)
}

/// Whom a cone aimed at `toward` catches, whatever their side (retail
/// `PopulateSpellTargetMarkers`): the party when its point lies in it, and
/// the other monsters in it.
pub(super) fn cone_victims(scene: &Scene, toward: Vec3, reach: f32) -> Vec<Victim> {
    let mut victims = Vec::new();
    if in_cone(scene.from, toward, reach, scene.party_at, scene.party) {
        victims.extend(scene.members.iter().map(|m| Victim::Member(m.entity)));
    }
    victims.extend(
        scene
            .monsters
            .iter()
            .filter(|m| {
                m.entity != scene.caster && in_cone(scene.from, toward, reach, m.at, m.from_caster)
            })
            .map(|m| Victim::Monster(m.entity)),
    );
    victims
}

/// Where an aim points.
pub(super) fn aimed_at(scene: &Scene, aim: SpellAim) -> Vec3 {
    let find = |list: &[Someone], entity| {
        list.iter().find(|s| s.entity == entity).map_or(scene.party_at, |s| s.at)
    };
    match aim {
        SpellAim::Monster(monster) => find(&scene.monsters, monster),
        SpellAim::Member(member) => find(&scene.members, member),
        SpellAim::Party | SpellAim::Foes => scene.party_at,
    }
}

/// A monster's standing, as a spell on it would find it.
pub(super) fn monster_standing(monster: &Monster) -> Standing {
    Standing {
        hurt: monster.hp < monster.max_hp,
        tired: monster.stamina < monster.max_stamina,
        conditions: monster.conditions.iter().map(|&(c, _)| c).collect(),
        enchanted: monster.enchantments.0.iter().map(|e| e.spell).collect(),
    }
}

/// Whether the monster may cast the spell at all (retail
/// `IsSpellUsableByMonster`): monsters may cast it, its effect is modelled
/// here, its aim is one a monster takes, and a silence does not stop it.
fn castable(spell: &SpellDefinition, caster: &Monster, kind: &Species) -> bool {
    let silenced = caster.conditions.iter().any(|&(c, _)| c == Condition::Silenced);
    spell.monsters_cast
        && effects::modelled(spell.id)
        && !matches!(
            spell.targeting,
            Targeting::Party | Targeting::Point | Targeting::Item | Targeting::LockOrTrap
        )
        && (!silenced
            || (spell.books.contains(Spellbook::Alchemy) && SILENT_ALCHEMISTS.contains(&kind.kind)))
}

/// Where the spell may be cast (retail `CollectMonsterSpellTargets`), each
/// one choice: at itself; at a friend in reach (a fellow along a clear
/// line, a member of a party it sees); at a foe likewise; a group spell at
/// the party it sees; a cone or blast at a foe monster, or at the party
/// once for each member it stands against; an all-enemies spell at its
/// foes. Group, cone and blast spells at the party need at least half of it
/// to feel them; retail's group spells ask nothing of the members' side.
pub(super) fn aims(spell: &SpellDefinition, scene: &Scene) -> Vec<SpellAim> {
    let reach = spell.reach.metres();
    let worth = |standing: &Standing| effects::worthwhile(spell.id, standing);
    let party_reached = scene.sees_party && scene.party <= reach;
    let monsters = |stance: Stance| {
        scene
            .monsters
            .iter()
            .filter(move |m| m.stance == stance && m.from_caster <= reach && worth(&m.standing))
            .map(|m| SpellAim::Monster(m.entity))
    };
    let members = |stance: Stance| {
        scene
            .members
            .iter()
            .filter(move |m| party_reached && m.stance == stance && worth(&m.standing))
            .map(|m| SpellAim::Member(m.entity))
    };
    let half = || {
        let valid = scene.members.iter().filter(|m| worth(&m.standing)).count();
        party_reached && valid != 0 && scene.members.len().div_ceil(2) <= valid
    };
    let hostile_members = scene.members.iter().filter(|m| m.stance == Stance::Hostile).count();
    match spell.targeting {
        Targeting::Caster => scene
            .monsters
            .iter()
            .find(|m| m.entity == scene.caster && worth(&m.standing))
            .map(|_| vec![SpellAim::Monster(scene.caster)])
            .unwrap_or_default(),
        Targeting::Ally => monsters(Stance::Friendly).chain(members(Stance::Friendly)).collect(),
        Targeting::Enemy => monsters(Stance::Hostile).chain(members(Stance::Hostile)).collect(),
        Targeting::EnemyGroup if half() => vec![SpellAim::Party],
        Targeting::Cone | Targeting::Radius => {
            let mut aims: Vec<_> = monsters(Stance::Hostile).collect();
            if half() {
                aims.extend(std::iter::repeat_n(SpellAim::Party, hostile_members));
            }
            aims
        }
        Targeting::AllEnemies => {
            let foes = monsters(Stance::Hostile).count() + members(Stance::Hostile).count();
            if foes != 0 { vec![SpellAim::Foes] } else { vec![] }
        }
        _ => vec![],
    }
}

/// Whether it could cast at a foe now (retail `MonsterGroupCanEngage`'s
/// spell test): it casts at all, and one of its spells aimed at enemies is
/// castable with an aim.
pub(super) fn casts_at_foes(
    caster: &Monster,
    kind: &Species,
    spells: &SpellCatalog,
    scene: &Scene,
) -> bool {
    kind.magic.chance != 0
        && kind.magic.spells.iter().flatten().filter_map(|&id| spells.get(id)).any(|spell| {
            spell.aimed_at_enemies()
                && castable(spell, caster, kind)
                && !aims(spell, scene).is_empty()
        })
}

/// The spell it casts, if it does, and where (retail `UpdateMonsterAI`):
/// at the first slot holding a spell it can aim, it casts at `chance`, one
/// of its castable spells by the slots' weights (`ChooseMonsterSpell`),
/// at one of that spell's aims at random.
pub(super) fn choose(
    rng: &mut GameRng,
    caster: &Monster,
    kind: &Species,
    chance: u32,
    spells: &SpellCatalog,
    scene: &Scene,
) -> Option<(SpellId, SpellAim)> {
    let usable: Vec<(u32, &SpellDefinition, Vec<SpellAim>)> = kind
        .magic
        .spells
        .iter()
        .zip(SLOT_WEIGHTS)
        .filter_map(|(id, weight)| {
            let spell = spells.get((*id)?)?;
            if !castable(spell, caster, kind)
                || !crate::game::magic::impacts::ready(spell.id, spells)
            {
                return None;
            }
            let aims = aims(spell, scene);
            (!aims.is_empty()).then_some((weight, spell, aims))
        })
        .collect();
    if usable.is_empty() || rng.below(100) >= chance {
        return None;
    }
    let total: u32 = usable.iter().map(|&(w, ..)| w).sum();
    let mut roll = rng.below(total);
    let (_, spell, aims) = usable
        .iter()
        .find(|&&(weight, ..)| {
            let here = roll < weight;
            roll = roll.saturating_sub(weight);
            here
        })
        .unwrap_or(&usable[usable.len() - 1]);
    let aim = aims[rng.below(aims.len() as u32) as usize];
    Some((spell.id, aim))
}

/// How hard it casts (retail `ChooseMonsterSpellPowerLevel`): up from 1
/// while its budget covers the next level's need within 9/70, back one,
/// then a quarter of the time one less (not below 2) and a quarter one
/// more.
pub(super) fn power(rng: &mut GameRng, budget: u32, spell: &SpellDefinition) -> u8 {
    let mut power = 1;
    loop {
        if crate::game::magic::spell_failure(budget, spell, power) >= 9 {
            break;
        }
        power += 1;
        if power >= 8 {
            break;
        }
    }
    if power > 1 {
        power -= 1;
    }
    match rng.below(4) {
        0 if power > 2 => power - 1,
        1 if power < MAX_POWER => power + 1,
        _ => power,
    }
}

/// Whom the cast lands on: its aim; a cone everyone in it; a blast
/// everyone within its radius of where it bursts; an all-enemies spell
/// every foe it reaches. A backfire turns a spell on one victim back on its
/// caster, and a wider one on the caster's own side.
fn victims(
    spell: &SpellDefinition,
    power: u8,
    aim: SpellAim,
    scene: &Scene,
    backfired: bool,
    field: &Field,
) -> Vec<Victim> {
    if spell.targeting == Targeting::Radius {
        let target = if backfired {
            Some(scene.caster)
        } else {
            match aim {
                SpellAim::Monster(entity) => Some(entity),
                _ => None,
            }
        };
        let party_eye = scene.party_at + Vec3::Y * crate::content::EYE_HEIGHT;
        let (centre, origin) = if let Some(target) = target {
            let Ok((body, at, walker, ..)) = field.monsters.monsters().get(target) else {
                return vec![];
            };
            let feet = crate::game::world::navigation::feet(at.translation(), walker);
            (feet, feet + Vec3::Y * body.body.eye_height)
        } else {
            (scene.party_at, party_eye)
        };
        let side = field.monsters.stance(scene.caster, field.now);
        let source = if backfired {
            match side {
                crate::game::world::hostility::Stance::Hostile => {
                    crate::game::world::hostility::Stance::Friendly
                }
                _ => crate::game::world::hostility::Stance::Hostile,
            }
        } else {
            side
        };
        return crate::game::magic::targeting::radius(
            field,
            centre,
            origin,
            spell.radius + spell.radius_per_power * f32::from(power),
            Some(party_eye),
            crate::game::magic::targeting::Recipients::Opponents(source),
        );
    }
    if backfired {
        return match spell.targeting {
            Targeting::Caster | Targeting::Ally | Targeting::Enemy => {
                vec![Victim::Monster(scene.caster)]
            }
            _ => scene
                .monsters
                .iter()
                .filter(|m| m.stance == Stance::Friendly)
                .map(|m| Victim::Monster(m.entity))
                .collect(),
        };
    }
    let reach = spell.reach.metres();
    match (spell.targeting, aim) {
        (Targeting::Cone, aim) => cone_victims(scene, aimed_at(scene, aim), reach),
        (_, SpellAim::Foes) => {
            let party_reached = scene.sees_party && scene.party <= reach;
            scene
                .members
                .iter()
                .filter(|m| party_reached && m.stance == Stance::Hostile)
                .map(|m| Victim::Member(m.entity))
                .chain(
                    scene
                        .monsters
                        .iter()
                        .filter(|m| m.stance == Stance::Hostile && m.from_caster <= reach)
                        .map(|m| Victim::Monster(m.entity)),
                )
                .collect()
        }
        (_, SpellAim::Party) => scene.members.iter().map(|m| Victim::Member(m.entity)).collect(),
        (_, SpellAim::Monster(monster)) => vec![Victim::Monster(monster)],
        (_, SpellAim::Member(member)) => vec![Victim::Member(member)],
    }
}

/// Cast it (retail `MonsterCastsSpell`): its level sets the strength
/// victims save against, its budget ([`Monster::spell_budget`], the one
/// its power was chosen by) the failure chance, and the spell lands at
/// once.
pub(super) fn cast(
    (spell, power, aim): (&SpellDefinition, u8, SpellAim),
    (kind, budget): (&Species, u32),
    scene: &Scene,
    rng: &mut GameRng,
    field: &mut Field,
    points: &mut Query<(&LearnedSpells, &mut SpellPoints)>,
) -> (Report, Vec<crate::game::magic::casting::SpellInFlight>) {
    let strength = Strength::at(u32::from(kind.level), spell, power, 0);
    let failure = crate::game::magic::spell_failure(budget, spell, power);
    let outcome = crate::game::magic::roll(rng, failure, spell, false, field.underwater);
    let mut flights = Vec::new();
    let effects = match outcome {
        Outcome::Fizzled => Vec::new(),
        _ => {
            let backfired = outcome == Outcome::Backfired;
            let victims = victims(spell, power, aim, scene, backfired, field);
            let original_aim = match aim {
                SpellAim::Monster(entity) => crate::game::magic::casting::Aim::Monster(entity),
                SpellAim::Member(entity) => crate::game::magic::casting::Aim::Member(entity),
                SpellAim::Party | SpellAim::Foes => match victims.first() {
                    Some(Victim::Member(entity)) => {
                        crate::game::magic::casting::Aim::Member(*entity)
                    }
                    Some(Victim::Monster(entity)) => {
                        crate::game::magic::casting::Aim::Monster(*entity)
                    }
                    None => crate::game::magic::casting::Aim::Monster(scene.caster),
                },
            };
            let finish = crate::game::magic::casting::Finalization {
                aim: original_aim,
                victims: victims.clone(),
                reflected: false,
            };
            if let Some(spells) = field.spells {
                crate::game::magic::impacts::resolve(
                    scene.caster,
                    crate::game::magic::casting::Origin::Actor(Victim::Monster(scene.caster)),
                    backfired,
                    spell,
                    strength,
                    &victims,
                    Some(&finish),
                    scene.party_at + Vec3::Y * crate::content::EYE_HEIGHT,
                    spells,
                    rng,
                    field,
                    points,
                    &mut flights,
                )
            } else {
                vec![]
            }
        }
    };
    (
        Report {
            caster: scene.caster,
            spell: spell.id,
            power,
            item: None,
            outcome,
            effects,
            improved: std::mem::take(&mut field.improved),
            repeats: 0,
        },
        flights,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::species::MonsterMagic;

    fn spell(id: u8, targeting: Targeting) -> SpellDefinition {
        SpellDefinition {
            id: SpellId(id),
            targeting,
            level: 1,
            cost: 2,
            monsters_cast: true,
            reach: crate::content::species::Reach::Long,
            ..Default::default()
        }
    }

    fn someone(entity: Entity, stance: Stance, hurt: bool) -> Someone {
        Someone {
            entity,
            at: Vec3::ZERO,
            from_caster: if stance == Stance::Friendly { 0.0 } else { 10.0 },
            standing: Standing { hurt, ..Default::default() },
            stance,
        }
    }

    fn scene(caster: Entity, hurt: bool) -> Scene {
        Scene {
            caster,
            from: Vec3::NEG_Z * 10.0,
            party_at: Vec3::ZERO,
            members: vec![someone(Entity::from_bits(10), Stance::Hostile, false)],
            monsters: vec![someone(caster, Stance::Friendly, hurt)],
            party: 10.0,
            sees_party: true,
        }
    }

    /// A monster heals itself only when hurt, aims its bolt at a member in
    /// reach, and casts none of them past its chance.
    #[test]
    fn a_monster_casts_what_would_work_at_its_chance() {
        let caster = Entity::from_bits(1);
        let mut catalog = SpellCatalog(vec![SpellDefinition::default(); 80]);
        catalog.0[SpellId::HEAL_WOUNDS.0 as usize] =
            spell(SpellId::HEAL_WOUNDS.0, Targeting::Caster);
        catalog.0[SpellId::ENERGY_BLAST.0 as usize] =
            spell(SpellId::ENERGY_BLAST.0, Targeting::Enemy);
        let mut spells = [None; 10];
        spells[0] = Some(SpellId::HEAL_WOUNDS);
        let kind = Species { magic: MonsterMagic { chance: 50, spells, budget: 50 }, ..default() };
        let monster = super::super::conduct::tests::monster(100);
        let mut rng = GameRng::seeded(3);
        assert_eq!(
            choose(&mut rng, &monster, &kind, 100, &catalog, &scene(caster, false)),
            None,
            "whole"
        );
        assert_eq!(
            choose(&mut rng, &monster, &kind, 100, &catalog, &scene(caster, true)),
            Some((SpellId::HEAL_WOUNDS, SpellAim::Monster(caster)))
        );
        assert_eq!(choose(&mut rng, &monster, &kind, 0, &catalog, &scene(caster, true)), None);
        let mut spells = [None; 10];
        spells[9] = Some(SpellId::ENERGY_BLAST);
        let kind = Species { magic: MonsterMagic { spells, ..kind.magic }, ..kind };
        assert_eq!(
            choose(&mut rng, &monster, &kind, 100, &catalog, &scene(caster, false)),
            Some((SpellId::ENERGY_BLAST, SpellAim::Member(Entity::from_bits(10))))
        );
        let mut far = scene(caster, false);
        far.party = 30.0;
        assert_eq!(choose(&mut rng, &monster, &kind, 100, &catalog, &far), None, "out of reach");
        // Turned, it stands with the party and against its fellow.
        let mut turned = scene(caster, false);
        turned.members[0].stance = Stance::Friendly;
        let fellow = Entity::from_bits(2);
        turned.monsters.push(someone(fellow, Stance::Hostile, false));
        assert_eq!(
            choose(&mut rng, &monster, &kind, 100, &catalog, &turned),
            Some((SpellId::ENERGY_BLAST, SpellAim::Monster(fellow))),
            "a turned caster blasts its fellow, not the party"
        );
    }

    /// A small budget casts at low power; a large one at the top.
    #[test]
    fn the_budget_sets_the_power() {
        let blast = spell(SpellId::ENERGY_BLAST.0, Targeting::Enemy);
        let mut rng = GameRng::seeded(1);
        let low: Vec<u8> = (0..20).map(|_| power(&mut rng, 0, &blast)).collect();
        assert!(low.iter().all(|&p| p <= 2), "{low:?}");
        let high: Vec<u8> = (0..20).map(|_| power(&mut rng, 500, &blast)).collect();
        assert!(high.iter().all(|&p| p >= 6), "{high:?}");
    }
}
