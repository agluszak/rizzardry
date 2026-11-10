//! Monsters casting in a fight (retail `UpdateMonsterAI`'s spell branch,
//! `IsSpellUsableByMonster`, `CollectMonsterSpellTargets`,
//! `MonsterSpellTargetOK`, `ChooseMonsterSpell`,
//! `ChooseMonsterSpellPowerLevel`, `MonsterCastsSpell`): the rules, over
//! the bodies the caster sees ([`super::ai::seen`]).
//!
//! A monster spends nothing to cast: its spell budget stands in for a
//! caster's skill in the failure roll, and the power it picks is the most
//! that budget comfortably covers.

use bevy::prelude::*;

use super::ai::Fight;
use crate::content::spells::{MAX_POWER, SpellDefinition, SpellId, Spellbook, Targeting};
use crate::game::characters::{Condition, Vitals};
use crate::game::hits::definitions::Definitions;
use crate::game::magic::casting::Aim;
use crate::game::rng::GameRng;
use crate::game::world::hostility::{self, Stance};
use crate::game::world::population::Spellcasting;

/// How much each of the ten slots weighs in the choice (retail
/// `g_spell_cast_weights`).
const SLOT_WEIGHTS: [u32; 10] = [5, 5, 5, 10, 10, 10, 10, 15, 15, 15];

/// Whether the body may cast the spell at all (retail
/// `IsSpellUsableByMonster`): monsters may cast it, its effect is modelled
/// here, its aim is one a monster takes, and a silence does not stop it.
fn castable(
    spell: &SpellDefinition,
    caster: &Vitals,
    casting: &Spellcasting,
    definitions: &Definitions,
) -> bool {
    let silenced = caster.conditions.has(Condition::Silenced);
    spell.monsters_cast
        && definitions.spell(spell.id).is_some()
        && !matches!(
            spell.targeting,
            Targeting::Party | Targeting::Point | Targeting::Item | Targeting::LockOrTrap
        )
        && (!silenced || (spell.books.contains(Spellbook::Alchemy) && casting.silent_alchemy))
}

/// Where the spell may be cast (retail `CollectMonsterSpellTargets`), each
/// one choice: at itself; at a friend in reach; at a foe likewise; a group
/// spell at a member of a party it sees; a cone or blast at a foe
/// monster, or at a member once for each member it stands against; an
/// all-enemies spell at its foes. Group, cone and blast spells at the party
/// need at least half of it to feel them; retail's group spells ask nothing
/// of the members' side. `seen` are the bodies the caster sees, itself
/// among them.
pub(super) fn aims(
    spell: &SpellDefinition,
    caster: Entity,
    seen: &[Entity],
    fight: &Fight,
) -> Vec<Aim> {
    let Fight { bodies, laid, places, roster, definitions, .. } = *fight;
    let Ok(own) = bodies.get(caster) else { return vec![] };
    let (at, mine) = (places.of(caster), own.standing());
    let own = own.vitals;
    let reach = spell.reach.metres();
    let worth = |body: Entity, vitals: &Vitals| {
        definitions.worthwhile(spell.id, vitals, laid.has(body, spell.id))
    };
    // Each body seen: whether it is of the party, how the caster stands
    // toward it, whether the spell reaches it, and whether it would do
    // anything.
    let looked: Vec<(Entity, bool, Stance, bool, bool)> = seen
        .iter()
        .filter_map(|&other| {
            let their = bodies.get(other).ok()?;
            let stance = if other == caster {
                Stance::Friendly
            } else {
                hostility::between(mine, their.standing())
            };
            let member = roster.contains(&other);
            Some((
                other,
                member,
                stance,
                at.gap(&places.of(other)) <= reach,
                worth(other, their.vitals),
            ))
        })
        .collect();
    let standing = |member: bool, stance: Stance| {
        looked
            .iter()
            .filter(move |&&(_, of_party, theirs, reached, worth)| {
                of_party == member && theirs == stance && reached && worth
            })
            .map(|&(entity, ..)| Aim::Body(entity))
    };
    let party = looked.iter().filter(|&&(_, member, ..)| member).count();
    let half = || {
        let valid = looked
            .iter()
            .filter(|&&(_, member, _, reached, worth)| member && reached && worth)
            .count();
        valid != 0 && party.div_ceil(2) <= valid
    };
    let hostile_members: Vec<Aim> = looked
        .iter()
        .filter(|&&(_, member, theirs, reached, _)| member && reached && theirs == Stance::Hostile)
        .map(|&(entity, ..)| Aim::Body(entity))
        .collect();
    match spell.targeting {
        Targeting::Caster => {
            if worth(caster, own) {
                vec![Aim::Body(caster)]
            } else {
                vec![]
            }
        }
        Targeting::Ally => {
            standing(false, Stance::Friendly).chain(standing(true, Stance::Friendly)).collect()
        }
        Targeting::Enemy => {
            standing(false, Stance::Hostile).chain(standing(true, Stance::Hostile)).collect()
        }
        Targeting::EnemyGroup if half() => hostile_members.first().copied().into_iter().collect(),
        Targeting::Cone | Targeting::Radius => {
            let mut aims: Vec<_> = standing(false, Stance::Hostile).collect();
            if half() {
                aims.extend(hostile_members);
            }
            aims
        }
        Targeting::AllEnemies => {
            let foes =
                standing(false, Stance::Hostile).count() + standing(true, Stance::Hostile).count();
            if foes != 0 { vec![Aim::Ahead] } else { vec![] }
        }
        _ => vec![],
    }
}

/// Whether it could cast at a foe now (retail `MonsterGroupCanEngage`'s
/// spell test): it casts at all, and one of its spells aimed at enemies is
/// castable with an aim.
pub(super) fn casts_at_foes(
    caster: Entity,
    casting: &Spellcasting,
    seen: &[Entity],
    fight: &Fight,
) -> bool {
    let (spells, definitions) = (&fight.content.spells, fight.definitions);
    let Ok(own) = fight.bodies.get(caster).map(|body| body.vitals) else { return false };
    casting.chance != 0
        && casting.spells.iter().flatten().map(|&id| &spells[id]).any(|spell| {
            spell.aimed_at_enemies()
                && castable(spell, own, casting, definitions)
                && !aims(spell, caster, seen, fight).is_empty()
        })
}

/// The spell it casts, if it does, and where (retail `UpdateMonsterAI`):
/// at the first slot holding a spell it can aim, it casts at `chance`, one
/// of its castable spells by the slots' weights (`ChooseMonsterSpell`),
/// at one of that spell's aims at random.
pub(super) fn choose(
    rng: &mut GameRng,
    caster: Entity,
    casting: &Spellcasting,
    chance: u32,
    seen: &[Entity],
    fight: &Fight,
) -> Option<(SpellId, Aim)> {
    let (spells, definitions) = (&fight.content.spells, fight.definitions);
    let own = fight.bodies.get(caster).ok()?.vitals;
    let usable: Vec<(u32, &SpellDefinition, Vec<Aim>)> = casting
        .spells
        .iter()
        .zip(SLOT_WEIGHTS)
        .filter_map(|(id, weight)| {
            let spell = &spells[(*id)?];
            if !castable(spell, own, casting, definitions) {
                return None;
            }
            let aims = aims(spell, caster, seen, fight);
            (!aims.is_empty()).then_some((weight, spell, aims))
        })
        .collect();
    if usable.is_empty() || rng.below(100) >= chance {
        return None;
    }
    let total: u32 = usable.iter().map(|&(w, ..)| w).sum();
    let weights = usable.iter().map(|&(w, ..)| w);
    let index = crate::game::rng::weighted(rng.below(total), weights).unwrap_or(usable.len() - 1);
    let (_, spell, aims) = &usable[index];
    let aim = aims[rng.below(aims.len() as u32) as usize];
    Some((spell.id, aim))
}

/// How hard it casts (retail `ChooseMonsterSpellPowerLevel`): up from 1
/// while its budget covers the next level's need within 9/70, back one,
/// then a quarter of the time one less (not below 2) and a quarter one
/// more.
pub(super) fn power(rng: &mut GameRng, budget: u32, spell: &SpellDefinition) -> u32 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::species::{MonsterMagic, Species};
    use crate::content::spells::SpellCatalog;
    use crate::game::actors::{Armour, Attacks, Attributes, Laid, Level, Places, Resistances};
    use crate::game::data::Catalogs;
    use crate::game::hits::effects::BodyReadOnly;
    use crate::game::party::body::PartyBody;
    use crate::game::party::roster::PartyRoster;
    use crate::game::world::hostility::{Loyalty, Side};
    use crate::game::world::obstacles::WorldObstacles;
    use crate::game::world::population::{Monster, ScaledBody};
    use avian3d::prelude::Position;
    use bevy::ecs::system::RunSystemOnce;

    fn spell(id: u8, targeting: Targeting) -> SpellDefinition {
        SpellDefinition {
            id: SpellId(id),
            targeting,
            level: 1,
            cost: 2,
            monsters_cast: true,
            realm: Some(crate::content::characters::Realm::Divine),
            reach: crate::content::species::Reach::Long,
            ..Default::default()
        }
    }

    /// A monster at `distance` from the origin on a side, hurt or whole.
    fn body(world: &mut World, stance: Stance, distance: f32, hurt: bool) -> Entity {
        let mut vitals = Vitals::full(10, 10);
        vitals.health.current -= u32::from(hurt);
        world
            .spawn((
                vitals,
                Side::new(stance),
                Loyalty::None,
                Attributes::default(),
                Level::default(),
                Resistances::default(),
                Armour::default(),
                Attacks::default(),
                Monster {
                    location: 0,
                    species: 0,
                    body: ScaledBody { scale: 1.0, radius: 0.5, eye_height: 1.0, speed: 1.0 },
                },
                Transform::from_translation(Vec3::Z * distance),
            ))
            .id()
    }

    /// A monster heals itself only when hurt, aims its bolt at a member in
    /// reach, and casts none of them past its chance.
    #[test]
    fn a_monster_casts_what_would_work_at_its_chance() {
        let mut catalog = SpellCatalog(vec![SpellDefinition::default(); 80]);
        catalog.0[SpellId::HEAL_WOUNDS.0 as usize] =
            spell(SpellId::HEAL_WOUNDS.0, Targeting::Caster);
        catalog.0[SpellId::ENERGY_BLAST.0 as usize] =
            spell(SpellId::ENERGY_BLAST.0, Targeting::Enemy);
        let mut world = World::new();
        world.init_resource::<avian3d::collider_tree::ColliderTrees>();
        world.insert_resource(Catalogs { spells: catalog, ..default() });
        world.run_system_once(crate::game::hits::definitions::define).unwrap();
        world.flush();
        let caster = body(&mut world, Stance::Hostile, 0.0, false);
        let hurt = body(&mut world, Stance::Hostile, 0.0, true);
        // The party's members stand where it does: it is moved to put the
        // member in reach or out of it.
        let member = world
            .spawn((
                Vitals::full(10, 10),
                Side::FRIENDLY,
                Loyalty::None,
                Attributes::default(),
                Level::default(),
                Resistances::default(),
                Armour::default(),
                Attacks::default(),
            ))
            .id();
        let party = crate::game::party::roster::test_party(&mut world, &[member]);
        world.entity_mut(party).insert((PartyBody::default(), Position(Vec3::Z * 10.0)));
        let fellow = body(&mut world, Stance::Hostile, 10.0, false);
        let mut spells = [None; 10];
        spells[0] = Some(SpellId::HEAL_WOUNDS);
        let healer =
            Species { magic: MonsterMagic { chance: 50, spells, budget: 50 }, ..default() };
        let mut spells = [None; 10];
        spells[9] = Some(SpellId::ENERGY_BLAST);
        let blaster =
            Species { magic: MonsterMagic { chance: 50, spells, budget: 50 }, ..default() };
        let choose =
            |world: &mut World, caster: Entity, kind: &Species, chance: u32, seen: &[Entity]| {
                let (casting, seen) = (Spellcasting::of(kind), seen.to_vec());
                world
                    .run_system_once(
                        move |bodies: Query<BodyReadOnly>,
                              roster: Single<(Entity, &PartyRoster)>,
                              definitions: Res<Definitions>,
                              laid: Laid,
                              places: Places,
                              obstacles: WorldObstacles,
                              content: Res<Catalogs>| {
                            let (party, roster) = *roster;
                            let fight = Fight {
                                bodies: &bodies,
                                laid: &laid,
                                places: &places,
                                party,
                                roster,
                                combatants: &seen,
                                far: 100.0,
                                obstacles: &obstacles,
                                content: &content,
                                definitions: &definitions,
                            };
                            let mut rng = GameRng::seeded(3);
                            super::choose(&mut rng, caster, &casting, chance, &seen, &fight)
                        },
                    )
                    .unwrap()
            };
        assert_eq!(choose(&mut world, caster, &healer, 100, &[caster, member]), None, "whole");
        assert_eq!(
            choose(&mut world, hurt, &healer, 100, &[hurt, member]),
            Some((SpellId::HEAL_WOUNDS, Aim::Body(hurt)))
        );
        assert_eq!(choose(&mut world, hurt, &healer, 0, &[hurt, member]), None);
        assert_eq!(
            choose(&mut world, caster, &blaster, 100, &[caster, member]),
            Some((SpellId::ENERGY_BLAST, Aim::Body(member)))
        );
        world.entity_mut(party).insert(Position(Vec3::Z * 30.0));
        assert_eq!(
            choose(&mut world, caster, &blaster, 100, &[caster, member]),
            None,
            "out of reach"
        );
        // Turned, it stands with the party and against its fellow.
        world.entity_mut(party).insert(Position(Vec3::Z * 10.0));
        world
            .get_mut::<Vitals>(caster)
            .unwrap()
            .afflict(Condition::Turncoat, crate::game::characters::Lasting::Rounds(5));
        assert_eq!(
            choose(&mut world, caster, &blaster, 100, &[caster, member, fellow]),
            Some((SpellId::ENERGY_BLAST, Aim::Body(fellow))),
            "a turned caster blasts its fellow, not the party"
        );
    }

    /// A small budget casts at low power; a large one at the top.
    #[test]
    fn the_budget_sets_the_power() {
        let blast = spell(SpellId::ENERGY_BLAST.0, Targeting::Enemy);
        let mut rng = GameRng::seeded(1);
        let low: Vec<u32> = (0..20).map(|_| power(&mut rng, 0, &blast)).collect();
        assert!(low.iter().all(|&p| p <= 2), "{low:?}");
        let high: Vec<u32> = (0..20).map(|_| power(&mut rng, 500, &blast)).collect();
        assert!(high.iter().all(|&p| p >= 6), "{high:?}");
    }
}
