//! What a casting or breathing monster sees: the party's members, its
//! fellows and foes, and whether its special attack is ready.

use super::*;
use super::monster_turn::Decided;
use super::round::{Acting, MonsterTurn};
use crate::game::world::footing::{Conspicuous, Eyes, Footing};

/// Whether a monster can make its special attack now (retail
/// `CanMonsterFlee`): it has one, it is not cooling down, it has a tenth of
/// its stamina, and a silence does not stop it. A summoning is never what
/// starts a fight (`fighting` is `None` then, retail's `exclude_special`);
/// in one it needs the party within long reach and the summoner not on its
/// side. Any other kind also needs an aim, which its caller finds.
pub(super) fn special_ready(
    vitals: &Vitals,
    kind: &Species,
    cooling: bool,
    fighting: Option<(f32, hostility::Stance)>,
) -> bool {
    let silenced = vitals.conditions.has(Condition::Silenced);
    kind.special.kind.is_some_and(|special| {
        let definition = crate::game::hits::specials::special(special);
        let summons = special.summons().is_none()
            || fighting.is_some_and(|(nearest, stance)| {
                nearest <= Reach::Long.metres() && stance != hostility::Stance::Friendly
            });
        !cooling
            && vitals.stamina.current >= vitals.stamina.max / 10
            && !(silenced && definition.silenced)
            && summons
    })
}

/// A member as a casting monster sees them, at the party's point
/// `party` away.
pub(super) fn member_in_scene(
    member: Entity,
    vitals: &Vitals,
    party_at: Vec3,
    party: f32,
    toward: &dyn Fn(Entity) -> hostility::Stance,
) -> monster_spells::Someone {
    monster_spells::Someone {
        entity: member,
        at: party_at,
        from_caster: party,
        vitals: vitals.clone(),
        stance: toward(member),
    }
}

/// The fight as each monster deciding its action sees it: the living party
/// at the party's point, and the living monsters in the fight it has a
/// clear line to, itself among them standing friendly to itself, with how
/// it stands toward each; and whether it sees the party.
pub(super) fn see_fight(
    mut commands: Commands,
    deciding: Query<
        (Entity, &Footing, &Eyes, &hostility::Side, &hostility::Loyalty),
        (With<Acting>, With<MonsterTurn>, Without<Decided>),
    >,
    bodies: Query<(&Vitals, &Footing, &hostility::Side, &hostility::Loyalty)>,
    seen: Query<&Conspicuous>,
    fight: Single<Option<&Combatants>, With<Battle>>,
    roster: Single<&PartyRoster>,
    level: PlayedLevel,
    obstacles: WorldObstacles,
) {
    let combatants = fight.map_or(&[][..], Combatants::entities);
    let far = level.view_distance().unwrap_or(0.0);
    for (caster, at, eyes, side, loyalty) in &deciding {
        let mine = (side.0, *loyalty);
        let someone = |entity: Entity, there: &Footing, vitals: &Vitals, side: &hostility::Side, loyalty: &hostility::Loyalty| {
            monster_spells::Someone {
                entity,
                at: there.feet,
                from_caster: at.gap(there),
                vitals: vitals.clone(),
                stance: if entity == caster {
                    hostility::Stance::Friendly
                } else {
                    hostility::between(mine, (side.0, *loyalty))
                },
            }
        };
        let party = roster.iter().find_map(|m| Some((bodies.get(m).ok()?.1, seen.get(m).ok()?)));
        let members = roster
            .iter()
            .filter_map(|m| {
                let (vitals, there, side, loyalty) = bodies.get(m).ok()?;
                (!vitals.is_dead()).then(|| someone(m, there, vitals, side, loyalty))
            })
            .collect();
        // A fellow is in reach of its spell only along a clear line.
        let monsters = combatants
            .iter()
            .filter_map(|&m| {
                let (vitals, there, side, loyalty) = bodies.get(m).ok()?;
                let clear = m == caster || obstacles.clear(at.eye, there.eye);
                (!vitals.is_dead() && clear).then(|| someone(m, there, vitals, side, loyalty))
            })
            .collect();
        let scene = monster_spells::Scene {
            caster,
            from: at.feet,
            party_at: party.map_or(at.feet, |(there, _)| there.feet),
            members,
            monsters,
            party: party.map_or(f32::INFINITY, |(there, _)| at.gap(there)),
            sees_party: party.is_some_and(|party| eyes.see(at, party, far, &obstacles)),
        };
        commands.entity(caster).insert(scene);
    }
}

/// A monster's decision made, what it saw goes.
pub(super) fn forget_fight(mut commands: Commands, seeing: Query<Entity, With<monster_spells::Scene>>) {
    for monster in &seeing {
        commands.entity(monster).remove::<monster_spells::Scene>();
    }
}
