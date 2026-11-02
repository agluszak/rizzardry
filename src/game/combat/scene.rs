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

/// The party's members in roster order, as a scene reads them.
pub(super) fn party_of<'a>(
    members: &'a Members,
    roster: &'a PartyRoster,
) -> impl Iterator<Item = (Entity, &'a Character, &'a Vitals)> {
    roster.iter().map(|member| {
        let (character, vitals, ..) = members.get(member).unwrap();
        (member, character, vitals)
    })
}

/// The fight as a monster about to cast or breathe sees it: the living
/// party and the living monsters in the fight it has a clear line to, where
/// each stands, what each lacks, and how it stands toward each (`toward`).
pub(super) fn spell_scene<'a>(
    (caster, kind): (Entity, &Species),
    (combatants, party_members): (
        &[Entity],
        impl Iterator<Item = (Entity, &'a Character, &'a Vitals)>,
    ),
    monsters: &MonsterQuery,
    sight: &Sight,
    toward: &dyn Fn(Entity) -> hostility::Stance,
) -> monster_spells::Scene {
    let party_eye = sight.eye;
    let party_at = PartyBody::feet_below(party_eye);
    let (body, at, walker, .., caster_vitals) = monsters.get(caster).unwrap();
    let feet_at = feet(at.translation(), walker);
    let eye = body.eye(feet_at);
    let party = party_distance(party_eye, body, feet_at);
    // A fellow is in reach of its spell only along a clear line, as the
    // party is only where the caster sees it.
    let clear = |to: Vec3| sight.obstacles.clear(eye, to);
    monster_spells::Scene {
        caster,
        from: feet_at,
        party_at,
        party,
        sees_party: sight.sees((body, caster_vitals), &at, walker, kind),
        members: party_members
            .filter(|(_, _, vitals)| !vitals.is_dead())
            .map(|(member, _, vitals)| member_in_scene((member, vitals), (party_at, party), toward))
            .collect(),
        monsters: combatants
            .iter()
            .filter_map(|&entity| {
                let (other, at, walker, .., vitals) = monsters.get(entity).ok()?;
                if vitals.is_dead() {
                    return None;
                }
                let at = feet(at.translation(), walker);
                if entity != caster && !clear(other.eye(at)) {
                    return None;
                }
                Some(monster_spells::Someone {
                    entity,
                    at,
                    from_caster: body.gap(feet_at, other, at),
                    vitals: vitals.clone(),
                    stance: if entity == caster {
                        hostility::Stance::Friendly
                    } else {
                        toward(entity)
                    },
                })
            })
            .collect(),
    }
}
