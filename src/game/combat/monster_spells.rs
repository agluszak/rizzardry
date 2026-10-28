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
    /// The party as a whole: a group, cone, blast or all-enemies spell.
    Party,
}

/// What a casting monster sees of the fight.
pub(super) struct Scene {
    pub caster: Entity,
    /// The living party: who, and what they lack.
    pub members: Vec<(Entity, Standing)>,
    /// The living monsters in the fight: who, how far from the caster and
    /// from the party, and what they lack.
    pub monsters: Vec<(Entity, f32, f32, Standing)>,
    /// How far the party is from the caster.
    pub party: f32,
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

/// Where the spell may be cast (retail `CollectMonsterSpellTargets`): at
/// itself, at a fellow in reach, at a member in reach, or at the party when
/// at least half of it would feel the spell. Each is one choice.
fn aims(spell: &SpellDefinition, scene: &Scene) -> Vec<SpellAim> {
    let reach = spell.reach.metres();
    let worth = |standing: &Standing| effects::worthwhile(spell.id, standing);
    match spell.targeting {
        Targeting::Caster => scene
            .monsters
            .iter()
            .find(|(m, ..)| *m == scene.caster)
            .filter(|(.., standing)| worth(standing))
            .map(|_| vec![SpellAim::Monster(scene.caster)])
            .unwrap_or_default(),
        Targeting::Ally => scene
            .monsters
            .iter()
            .filter(|&(_, from_caster, _, standing)| *from_caster <= reach && worth(standing))
            .map(|&(m, ..)| SpellAim::Monster(m))
            .collect(),
        Targeting::Enemy if scene.party <= reach => scene
            .members
            .iter()
            .filter(|(_, standing)| worth(standing))
            .map(|&(m, _)| SpellAim::Member(m))
            .collect(),
        Targeting::EnemyGroup | Targeting::Cone | Targeting::Radius | Targeting::AllEnemies
            if scene.party <= reach =>
        {
            let valid = scene.members.iter().filter(|(_, standing)| worth(standing)).count();
            if valid != 0 && scene.members.len().div_ceil(2) <= valid {
                vec![SpellAim::Party]
            } else {
                vec![]
            }
        }
        _ => vec![],
    }
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
            if !castable(spell, caster, kind) {
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
pub(super) fn power(rng: &mut GameRng, budget: i32, spell: &SpellDefinition) -> u8 {
    let budget = budget.max(0) as u32;
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

/// Whom the cast lands on: its aim, or for the party all of it; a blast
/// also catches the monsters in the fight within its radius of the
/// party. A backfire turns a spell on one victim back on its caster, and
/// a wider one on the caster's own side.
fn victims(
    spell: &SpellDefinition,
    power: u8,
    aim: SpellAim,
    scene: &Scene,
    backfired: bool,
) -> Vec<Victim> {
    if backfired {
        return match spell.targeting {
            Targeting::Caster | Targeting::Ally | Targeting::Enemy => {
                vec![Victim::Monster(scene.caster)]
            }
            _ => scene.monsters.iter().map(|&(m, ..)| Victim::Monster(m)).collect(),
        };
    }
    match aim {
        SpellAim::Monster(monster) => vec![Victim::Monster(monster)],
        SpellAim::Member(member) => vec![Victim::Member(member)],
        SpellAim::Party => {
            let mut victims: Vec<_> =
                scene.members.iter().map(|&(m, _)| Victim::Member(m)).collect();
            if spell.targeting == Targeting::Radius {
                let radius = spell.radius + spell.radius_per_power * f32::from(power);
                victims.extend(
                    scene
                        .monsters
                        .iter()
                        .filter(|&&(_, _, from_party, _)| from_party <= radius)
                        .map(|&(m, ..)| Victim::Monster(m)),
                );
            }
            victims
        }
    }
}

/// Cast it (retail `MonsterCastsSpell`): its level sets the strength
/// victims save against, its budget the failure chance, and the spell
/// lands at once.
pub(super) fn cast(
    (spell, power, aim): (&SpellDefinition, u8, SpellAim),
    kind: &Species,
    scene: &Scene,
    rng: &mut GameRng,
    field: &mut Field,
    points: &mut Query<(&LearnedSpells, &mut SpellPoints)>,
) -> Report {
    let strength = Strength::at(u32::from(kind.level), spell, power, 0);
    let failure = crate::game::magic::spell_failure(kind.magic.budget.max(0) as u32, spell, power);
    let outcome = crate::game::magic::roll(rng, failure, spell, false);
    let effects = match outcome {
        Outcome::Fizzled => Vec::new(),
        _ => {
            let backfired = outcome == Outcome::Backfired;
            let victims = victims(spell, power, aim, scene, backfired);
            effects::resolve(backfired, spell, strength, &victims, rng, field, points)
        }
    };
    Report {
        caster: scene.caster,
        spell: spell.id,
        power,
        item: None,
        outcome,
        effects,
        improved: std::mem::take(&mut field.improved),
        repeats: 0,
    }
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

    fn scene(caster: Entity, hurt: bool) -> Scene {
        let standing = Standing { hurt, ..Default::default() };
        Scene {
            caster,
            members: vec![(Entity::from_bits(10), Standing::default())],
            monsters: vec![(caster, 0.0, 10.0, standing)],
            party: 10.0,
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
