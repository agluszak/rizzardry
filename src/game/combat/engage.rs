//! Who comes into a fight, and how one opens (after retail
//! `UpdateMonsterGroups`, `MonsterGroupCanEngage`,
//! `CheckMonsterGroupsEnterCombat`, `ShouldMonsterGroupEnterCombat`,
//! `AlertSameFactionGroups`, `MakeTargetGroupHostile` and `StartCombat`).
//!
//! While a fight runs, a hostile group fighting turns hostile the other
//! groups of its faction whose leader sees its leader ([`alert_factions`]).
//! Then each group outside the fight, in authored order, takes its stance
//! ([`hostility::stance`]), turning hostile when the party comes within its
//! hostility radius in its sight ([`engage`]). A hostile group enters with
//! all its living members when the party has just acted against it.
//! Otherwise a fight starts when a member sees the party ([`Sight`]) and a
//! member can act on it: one of its attacks reaches, or its best attack is
//! close (touch or short) and it can walk up. A fight under way takes in a
//! hostile group that sees the party with its leader in reach, and a
//! neutral group standing with it ([`reinforces`]). NPC groups keep to
//! their roles. Those who come in join the fight, or open one
//! ([`open_battle`]). Retail's path-length check is taken as the straight
//! distance; its rendered-member clause and the party's own surprise rolls
//! are not modelled yet.

use super::*;

use super::ai::special_ready;
use super::monster_spells;
use crate::game::party::body::PartyBody;
use crate::game::world::footing::{Conspicuous, Eyes, Footing};
use crate::game::world::hostility::{Loyalty, Side};
use avian3d::prelude::Position;

/// Monsters coming into the fight, in placement order.
#[derive(Message)]
pub(super) struct Engaged(Vec<Entity>);

/// The first living member of a group.
fn leader(members: &GroupMembers, vitals: &Query<&Vitals, With<Monster>>) -> Option<Entity> {
    members.iter().find(|&m| vitals.get(m).is_ok_and(|vitals| !vitals.is_dead()))
}

/// While a fight runs, a fighting hostile group alerts the groups of its
/// faction whose leaders see its own (retail `AlertSameFactionGroups`).
pub(super) fn alert_factions(
    monsters: Query<(&Monster, &Footing, &Eyes, &Conspicuous, Has<Combatant>)>,
    vitals: Query<&Vitals, With<Monster>>,
    mut groups: Query<(Entity, &mut MonsterGroup, &GroupMembers, &LevelOwned)>,
    level: PlayedLevel,
    obstacles: WorldObstacles,
    factions: Res<crate::game::factions::Factions>,
    clock: Res<GameClock>,
    content: If<Res<Catalogs>>,
) {
    let (Some(far), Some(root)) = (level.view_distance(), level.entity()) else { return };
    let (catalog, now) = (&content.species, clock.ms);
    // Fighting hostile groups by their leaders, and their factions.
    let alerting: Vec<(Entity, i32, Entity)> = groups
        .iter()
        .filter_map(|(entity, group, members, owner)| {
            let leader = leader(members, &vitals).filter(|_| owner.0 == root)?;
            let (body, .., fighting) = monsters.get(leader).ok()?;
            let kind = &catalog[body.species];
            (fighting
                && kind.faction != crate::game::factions::UNALIGNED
                && hostility::stance(group.set, kind, &factions, now, true)
                    == hostility::Stance::Hostile)
                .then_some((entity, kind.faction, leader))
        })
        .collect();
    for (entity, mut group, members, owner) in &mut groups {
        let Some(leader) = leader(members, &vitals).filter(|_| owner.0 == root) else { continue };
        let Ok((body, at, eyes, _, fighting)) = monsters.get(leader) else { continue };
        let kind = &catalog[body.species];
        // NPCs keep to their own roles here.
        if kind.npc.is_some() {
            continue;
        }
        let alerted = alerting.iter().any(|&(source, faction, target)| {
            source != entity
                && faction == kind.faction
                && monsters.get(target).is_ok_and(|(_, there, _, seen, _)| {
                    eyes.see(at, (there, seen), far, &obstacles)
                })
        });
        if alerted
            && hostility::stance(group.set, kind, &factions, now, fighting)
                != hostility::Stance::Hostile
        {
            group.set = Some(hostility::SetStance { stance: hostility::Stance::Hostile, at: now });
        }
    }
}

/// Each group outside the fight, in authored order, then any monster
/// without one, comes in when its stance and sight of the party call for
/// it.
pub(super) fn engage(
    battles: Query<(), With<Battle>>,
    monsters: Query<(
        Entity,
        &Monster,
        &Vitals,
        &Footing,
        &Eyes,
        Option<&GroupMember>,
        Has<Combatant>,
    )>,
    mut groups: Query<(Entity, &mut MonsterGroup, &GroupMembers, &LevelOwned)>,
    members: Query<(&Footing, &Conspicuous, &Vitals), With<Character>>,
    bodies: Query<(
        &Vitals,
        &Footing,
        &Side,
        &Loyalty,
        Has<crate::game::party::roster::PartyMember>,
    )>,
    roster: Single<&PartyRoster>,
    level: PlayedLevel,
    obstacles: WorldObstacles,
    factions: Res<crate::game::factions::Factions>,
    mut provocations: MessageReader<hostility::Provocation>,
    clock: Res<GameClock>,
    content: If<Res<Catalogs>>,
    mut engaged: MessageWriter<Engaged>,
    definitions: Res<crate::game::hits::prototypes::Definitions>,
) {
    let provoked: Vec<Entity> = provocations
        .read()
        .filter_map(|&hostility::Provocation(m)| Some(monsters.get(m).ok()?.5?.0))
        .collect();
    let running = !battles.is_empty();
    let anyone_alive =
        roster.iter().any(|m| members.get(m).is_ok_and(|(.., vitals)| !vitals.is_dead()));
    if !running && !anyone_alive {
        return;
    }
    let (Some(far), Some(root)) = (level.view_distance(), level.entity()) else { return };
    let Some((party, seen_party, ..)) = roster.iter().find_map(|m| members.get(m).ok()) else {
        return;
    };
    let (catalog, spells) = (&content.species, &content.spells);
    let now = clock.ms;
    // Those of a group that fight: alive, and not an NPC.
    let fighters = |group: &GroupMembers| -> Vec<Entity> {
        group
            .iter()
            .filter(|&member| {
                monsters.get(member).is_ok_and(|(_, monster, vitals, ..)| {
                    !vitals.is_dead() && catalog[monster.species].npc.is_none()
                })
            })
            .collect()
    };
    // How far a fighter is, whether it sees the party, and whether it can
    // act on it (retail `MonsterGroupCanEngage`): one of its attacks
    // reaches, or its best attack is close and it can walk up, or it can
    // cast or breathe at the party, which it sees of the fight-to-be.
    let look = |fighter: Entity| -> Option<(f32, bool, bool)> {
        let (_, monster, vitals, at, eyes, ..) = monsters.get(fighter).ok()?;
        let kind = &catalog[monster.species];
        let distance = at.gap(party);
        let sees = eyes.see(at, (party, seen_party), far, &obstacles);
        let reaches = kind.attacks.iter().any(|a| distance <= a.reach.metres());
        let close = kind.attacks.iter().map(|a| a.reach).max().is_some_and(|r| r <= Reach::Short);
        let seen: Vec<Entity> = std::iter::once(fighter)
            .chain(roster.iter().filter(|_| sees))
            .filter(|&body| bodies.get(body).is_ok_and(|(vitals, ..)| !vitals.is_dead()))
            .collect();
        let casts =
            monster_spells::casts_at_foes(fighter, kind, spells, &seen, &bodies, &definitions);
        let breathes = kind.special.chance != 0
            && special_ready(vitals, kind, false, None)
            && spells.get(crate::content::spells::SpellId::SPECIAL_ATTACK_CONE).is_some_and(
                |cone| {
                    !monster_spells::aims(cone, fighter, &seen, &bodies, &definitions).is_empty()
                },
            );
        let acts = vitals.can_fight()
            && (reaches || (close && monster.body.speed > 0.0) || casts || breathes);
        Some((distance, sees, acts))
    };
    let footing = |monster: Entity| monsters.get(monster).ok().map(|(_, _, _, at, ..)| at);
    // Whether a group's leader (its first fighter) is in reach of the
    // party: its best attack's reach and one combat move, never under the
    // engagement range and its floor (retail `ShouldMonsterGroupEnterCombat`;
    // retail's path length check is taken as the straight distance).
    let leader_reaches = |kind: &Species, distance: f32| {
        let best = kind.attacks.iter().map(|a| a.reach.metres()).fold(0.0, f32::max);
        let reach = (best + kind.combat_move_range * approach::METRES_PER_MOVE_RANGE)
            .max(ENGAGEMENT_RANGE + ENGAGEMENT_FLOOR);
        distance <= reach
    };
    // The fight's hostile fighters in the party's view, as reinforcement
    // reads them (retail `CheckMonsterGroupsEnterCombat`).
    let hostiles: Vec<(Entity, &Footing)> = if running {
        monsters
            .iter()
            .filter(|&(_, monster, vitals, at, _, group, fighting)| {
                let set = group.and_then(|g| groups.get(g.0).ok()).and_then(|(_, g, ..)| g.set);
                fighting
                    && !vitals.is_dead()
                    && party.in_view(at, far, &obstacles)
                    && hostility::stance(set, &catalog[monster.species], &factions, now, true)
                        == hostility::Stance::Hostile
            })
            .map(|(entity, _, _, at, ..)| (entity, at))
            .collect()
    } else {
        Vec::new()
    };
    // Each group outside the fight in authored order, then any monster
    // without one.
    let mut units: Vec<(i32, Option<Entity>, Vec<Entity>)> = groups
        .iter()
        .filter(|(.., owner)| owner.0 == root)
        .map(|(entity, group, membership, ..)| (group.authored, Some(entity), fighters(membership)))
        .collect();
    units.sort_unstable_by_key(|&(authored, ..)| authored);
    units.extend(
        monsters
            .iter()
            .filter(|(_, monster, vitals, _, _, group, _)| {
                group.is_none() && !vitals.is_dead() && catalog[monster.species].npc.is_none()
            })
            .map(|(entity, ..)| (i32::MAX, None, vec![entity])),
    );
    let mut joined: Vec<(i32, Entity)> = Vec::new();
    for (_, group, fighters) in units {
        // Those already fighting stay; the rest of their group may join them.
        let fighters: Vec<Entity> =
            fighters.into_iter().filter(|&m| monsters.get(m).is_ok_and(|m| !m.6)).collect();
        let Some(&first) = fighters.first() else { continue };
        let kind = &catalog[monsters.get(first).unwrap().1.species];
        let looks: Vec<_> = fighters.iter().filter_map(|&m| look(m)).collect();
        let nearest = looks.iter().map(|l| l.0).fold(f32::INFINITY, f32::min);
        let sees = looks.iter().any(|l| l.1);
        let acts = looks.iter().any(|l| l.2);
        let provoked = group.is_some_and(|g| provoked.contains(&g));
        let mut group = group.map(|g| groups.get_mut(g).unwrap().1);
        let mut stance = match &group {
            Some(group) => hostility::stance(group.set, kind, &factions, now, false),
            None => hostility::default_stance(kind, &factions),
        };
        // Coming within a hostility radius in the group's sight turns it.
        if stance != hostility::Stance::Hostile && sees && hostility::within_radius(kind, nearest) {
            stance = hostility::Stance::Hostile;
            if let Some(group) = &mut group {
                group.set = Some(hostility::SetStance { stance, at: now });
            }
        }
        let enters = match stance {
            // A fight starts when a hostile group can act on the party
            // (retail `MonsterGroupCanEngage`); one under way takes in a
            // hostile group that sees the party with its leader in reach
            // (`ShouldMonsterGroupEnterCombat`).
            hostility::Stance::Hostile if running => {
                provoked
                    || (sees
                        && footing(first).is_some_and(|at| leader_reaches(kind, at.gap(party))))
            }
            hostility::Stance::Hostile => provoked || (sees && acts),
            // A neutral group joins a running fight it stands with.
            hostility::Stance::Neutral if running => {
                let standing: Vec<(Entity, &Footing)> = fighters
                    .iter()
                    .filter(|_| !kind.temper.camouflaged)
                    .filter_map(|&m| footing(m).map(|at| (m, at)))
                    .filter(|(_, at)| party.in_view(at, far, &obstacles))
                    .collect();
                reinforces(party, &standing, &hostiles)
            }
            _ => false,
        };
        if !enters {
            continue;
        }
        // An unaligned species with a hostility radius turns hostile as it
        // enters.
        if stance == hostility::Stance::Neutral
            && kind.npc.is_none()
            && kind.faction == crate::game::factions::UNALIGNED
            && matches!(kind.disposition, crate::content::species::Disposition::HostileWithin(_))
            && let Some(group) = &mut group
        {
            group.set = Some(hostility::SetStance { stance: hostility::Stance::Hostile, at: now });
        }
        joined.extend(fighters.iter().map(|&m| (monsters.get(m).unwrap().1.location, m)));
    }
    if joined.is_empty() {
        return;
    }
    joined.sort();
    engaged.write(Engaged(joined.into_iter().map(|(_, e)| e).collect()));
}

/// Those who came in join the fight under way, or open one: the party may
/// be surprised, a camp is broken up, and the party looks for whoever came
/// (retail `StartCombat`).
pub(super) fn open_battle(
    mut commands: Commands,
    mut engaged: MessageReader<Engaged>,
    battles: Query<Entity, With<Battle>>,
    camp: Option<Res<crate::game::camp::Camp>>,
    search: Res<crate::game::world::search::Search>,
    mut camp_news: MessageWriter<crate::game::camp::CampNews>,
    monsters: Query<(&Monster, &Vitals, &Footing)>,
    party: Single<(
        &Position,
        &PartyBody,
        &crate::game::magic::enchantments::PartyEffects,
        &PartyRoster,
        &crate::game::party::body::PartyMovement,
    )>,
    members: Query<(&Character, &Vitals)>,
    level: PlayedLevel,
    obstacles: WorldObstacles,
    content: If<Res<Catalogs>>,
    mut recognition: GroupRecognition,
    mut news: MessageWriter<CombatNews>,
    mut rng: ResMut<GameRng>,
) {
    let Some(Engaged(joined)) = engaged.read().last() else { return };
    if let Ok(battle) = battles.single() {
        for &monster in joined {
            commands.entity(monster).insert(Combatant(battle));
        }
        return;
    }
    let (Some(far), Some(root)) = (level.view_distance(), level.entity()) else { return };
    info!("combat: {} monsters", joined.len());
    let catalog = &content.species;
    let (position, body, effects, roster, movement) = *party;
    let (eye, forward) = (PartyBody::eye(position), body.forward());
    let shadow_hound = effects.0.has(crate::content::spells::SpellId::SHADOW_HOUND);
    let searching = search.on(false);
    let view = crate::game::world::perception::View { eye, forward, far, searching };
    let members = || members.iter_many(roster.iter()).matched();
    let seen = joined.iter().any(|&entity| {
        let (monster, vitals, at) = monsters.get(entity).unwrap();
        view.sees(
            members(),
            shadow_hound,
            (monster, vitals),
            &catalog[monster.species],
            at.feet,
            false,
            &obstacles,
        )
    });
    let opening = opening_surprise(
        seen,
        shadow_hound,
        camp.is_some(),
        movement.running,
        searching,
        members(),
        &mut rng,
    );
    if camp.is_some() {
        commands.remove_resource::<crate::game::camp::Camp>();
        camp_news.write(crate::game::camp::CampNews::Interrupted);
    }
    // The nearest of them, which the view may turn to (retail `StartCombat`
    // hands it to `PointCameraAtMonster`); the party's facing is the
    // player's.
    let nearest = joined
        .iter()
        .map(|&m| (m, monsters.get(m).unwrap().2.feet))
        .min_by(|a, b| a.1.distance(eye).total_cmp(&b.1.distance(eye)))
        .map(|(m, _)| m);
    let battle = commands
        .spawn((Name::new("Battle"), Battle { opening, ..default() }, LevelOwned(root)))
        .id();
    for &monster in joined {
        commands.entity(monster).insert(Combatant(battle));
    }
    recognition.detect(eye, forward, Some(joined), shadow_hound, searching, &mut rng);
    news.write(CombatNews::Began { nearest });
    if let Some(cause) = opening {
        news.write(CombatNews::Surprised(cause));
    }
}
