//! What a casting or breathing monster sees: the party's members, its
//! fellows and foes, and whether its special attack is ready.

use super::*;

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
    (member, vitals): (Entity, &Vitals),
    (party_at, party): (Vec3, f32),
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

/// The fight as a monster about to cast or breathe sees it: the living
/// party at the party's point, and the living monsters in the fight it has
/// a clear line to, itself among them standing friendly to itself, with
/// how it stands toward each.
pub(super) fn scene(
    caster: Entity,
    (combatants, roster): (&[Entity], &PartyRoster),
    bodies: &mut Query<crate::game::hits::Body>,
    (sight, species): (&Sight, &MonsterCatalog),
    toward: &dyn Fn(&crate::game::hits::BodyReadOnlyItem) -> hostility::Stance,
) -> monster_spells::Scene {
    let party_eye = sight.eye;
    let party_at = PartyBody::feet_below(party_eye);
    let me = bodies.get(caster).unwrap();
    let body = me.monster.unwrap().clone();
    let feet_at = me.feet(party_eye);
    let eye = me.eye(party_eye);
    let party = party_distance(party_eye, &body, feet_at);
    let sees_party = me.pose.as_ref().is_some_and(|pose| {
        sight.sees((&body, me.vitals), pose, me.walker, &species[body.species])
    });
    let members = roster
        .iter()
        .filter_map(|member| {
            let body = bodies.get(member).ok()?;
            (!body.is_dead()).then(|| monster_spells::Someone {
                entity: member,
                at: party_at,
                from_caster: party,
                vitals: body.vitals.clone(),
                stance: toward(&body),
            })
        })
        .collect();
    let monsters = combatants
        .iter()
        .filter_map(|&entity| {
            let other = bodies.get(entity).ok()?;
            let them = other.monster?;
            if other.is_dead() {
                return None;
            }
            let at = other.feet(party_eye);
            // A fellow is in reach of its spell only along a clear line.
            if entity != caster && !sight.obstacles.clear(eye, them.eye(at)) {
                return None;
            }
            Some(monster_spells::Someone {
                entity,
                at,
                from_caster: body.gap(feet_at, them, at),
                vitals: other.vitals.clone(),
                stance: if entity == caster { hostility::Stance::Friendly } else { toward(&other) },
            })
        })
        .collect();
    monster_spells::Scene { caster, from: feet_at, party_at, members, monsters, party, sees_party }
}
