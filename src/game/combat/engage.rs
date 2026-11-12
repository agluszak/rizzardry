//! Who comes into a fight, and how one opens.
//!
//! While a fight runs, a hostile group fighting turns hostile the other
//! groups of its faction whose leader sees its leader ([`alert_factions`]).
//! Then each group outside the fight, in authored order, comes in or not
//! ([`engage`]): this reads what each group's members make out and can do and
//! asks [`monster::coming`] whether it does. Those who come in join the fight,
//! or open one ([`open_battle`]). NPC groups keep to their roles. A group's
//! path to the party is taken as the straight distance, and a rendered member
//! near the party is not modelled (`docs/rules/monsters.md` section 3).

use super::*;
use crate::game::world::navigation::walking_distance;

use super::monster::coming;
use super::monster::sight::Sight;
use crate::game::actors::{Level, Places};
use crate::game::party::body::PartyBody;
use crate::game::world::hostility::{Allegiance, Stand};
use crate::game::world::identify::Leads;
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::Camouflage;
use avian3d::prelude::Position;

/// Monsters coming into the fight, in placement order.
#[derive(Message)]
pub(super) struct Engaged(pub(super) Vec<Entity>);

/// While a fight runs, a group of a faction turns hostile when its leader
/// sees the leader of another group of its faction that is hostile and
/// fighting (retail `AlertSameFactionGroups`). An NPC's group, an unaligned
/// one and one hostile already are not stirred.
pub(super) fn alert_factions(
    leading: Query<(Entity, &Leads, &Allegiance, Has<Combatant>, &LevelOwned), With<Monster>>,
    sight: Sight,
    level: PlayedLevel,
    mut commands: Commands,
) {
    let Some(root) = level.entity() else { return };
    let hostile = |monster: Entity| sight.side(monster) == Some(Stance::Hostile);
    let leaders: Vec<(Entity, Entity, &Allegiance, bool)> = leading
        .iter()
        .filter(|&(leader, .., owner)| owner.0 == root && sight.alive(leader))
        .map(|(leader, &Leads(group), allegiance, fighting, _)| {
            (group, leader, allegiance, fighting)
        })
        .collect();
    for &(group, leader, allegiance, _) in &leaders {
        let stirred = !allegiance.npc
            && allegiance.faction != crate::game::factions::UNALIGNED
            && !hostile(leader)
            && leaders.iter().any(|&(other, theirs, their_allegiance, fighting)| {
                other != group
                    && their_allegiance.faction == allegiance.faction
                    && fighting
                    && hostile(theirs)
                    && sight.sees(leader, theirs)
            });
        if stirred {
            commands.trigger(Stand { group, stance: Some(Stance::Hostile) });
        }
    }
}

/// Each group outside the fight, in authored order, then any monster without
/// one, comes in or not ([`coming::comes_in`]). Those who come in are told
/// of together, in the order of their places in the level.
pub(super) fn engage(
    battles: Query<(), With<Battle>>,
    monsters: Query<(Entity, &Monster, &Allegiance, Option<&GroupMember>, Has<Combatant>)>,
    groups: Query<(Entity, &MonsterGroup, &GroupMembers, &LevelOwned, Has<hostility::Provoked>)>,
    leading: Query<(), With<Leads>>,
    sight: Sight,
    level: PlayedLevel,
    content: If<Res<Catalogs>>,
    (navigation, archipelagos): (
        Option<Single<&crate::game::world::navigation::LevelNavigation>>,
        Query<&bevy_landmass::Archipelago3d>,
    ),
    mut engaged: MessageWriter<Engaged>,
    mut commands: Commands,
) {
    // What provoked a group is looked at this once.
    let provoked: Vec<Entity> =
        groups.iter().filter(|group| group.4).map(|(group, ..)| group).collect();
    for &group in &provoked {
        commands.entity(group).remove::<hostility::Provoked>();
    }
    let running = !battles.is_empty();
    let (Some(_), Some(root)) = (level.view_distance(), level.entity()) else { return };
    if !running && !sight.roster().iter().any(|member| sight.alive(member)) {
        return;
    }
    // Those who fight: alive, and not an NPC.
    let fights = |monster: Entity| {
        sight.alive(monster)
            && monsters.get(monster).is_ok_and(|(_, _, allegiance, ..)| !allegiance.npc)
    };
    // The fight's hostile fighters, whom a neutral group may stand with.
    let hostiles: Vec<Entity> = monsters
        .iter()
        .filter(|&(monster, .., fighting)| {
            fighting && sight.alive(monster) && sight.side(monster) == Some(Stance::Hostile)
        })
        .map(|(monster, ..)| monster)
        .collect();
    let mut units: Vec<(i32, Option<Entity>, Vec<Entity>)> = groups
        .iter()
        .filter(|(.., owner, _)| owner.0 == root)
        .map(|(entity, group, members, ..)| {
            (group.authored, Some(entity), members.iter().filter(|&m| fights(m)).collect())
        })
        .collect();
    units.sort_unstable_by_key(|&(authored, ..)| authored);
    units.extend(
        monsters
            .iter()
            .filter(|&(monster, .., group, _)| group.is_none() && fights(monster))
            .map(|(monster, ..)| (i32::MAX, None, vec![monster])),
    );
    let mut joined: Vec<(i32, Entity)> = Vec::new();
    for (_, group, fighters) in units {
        // Those already fighting stay; the rest of their group may join them.
        let fighters: Vec<Entity> =
            fighters.into_iter().filter(|&m| monsters.get(m).is_ok_and(|m| !m.4)).collect();
        let Some(&first) = fighters.first() else { continue };
        let (_, _, allegiance, ..) = monsters.get(first).unwrap();
        let side = (sight.side(first).unwrap_or_default(), allegiance.hostility_radius());
        let provoked = group.is_some_and(|g| provoked.contains(&g));
        // The way the leader would walk to the party, over the navmesh for
        // its size (retail `FindNavigatorPathDistance`).
        let walks = |leader: Entity, reach: f32| {
            let Ok((_, monster, ..)) = monsters.get(leader) else { return false };
            let archipelago = navigation.as_ref().and_then(|navigation| {
                archipelagos.get(navigation.archipelago_for(monster.body.radius)).ok()
            });
            let (from, to) = (sight.places.of(leader).feet, sight.places.of(sight.party()).feet);
            archipelago
                .and_then(|archipelago| walking_distance(archipelago, from, to))
                .is_some_and(|walked| walked <= reach)
        };
        // Its leader, if it is among those who may come; else the first of them.
        let leader =
            fighters.iter().copied().find(|&fighter| leading.contains(fighter)).unwrap_or(first);
        let came = coming::comes_in(
            &sight,
            (&fighters, leader),
            side,
            (running, provoked),
            &hostiles,
            &content,
            walks,
        );
        if let Some(group) = group
            && came.turns_hostile
        {
            commands.trigger(Stand { group, stance: Some(Stance::Hostile) });
        }
        if came.enters {
            joined.extend(fighters.iter().map(|&m| (monsters.get(m).unwrap().1.location, m)));
        }
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
    monsters: Query<(&Monster, &Vitals, &Level, Option<&Camouflage>)>,
    (laid, places): (crate::game::actors::Laid, Places),
    party: Single<(
        &Position,
        &PartyBody,
        Entity,
        &PartyRoster,
        &crate::game::party::body::PartyMovement,
    )>,
    members: Query<
        (
            &crate::game::actors::Traits,
            &crate::game::characters::Conditions,
            &crate::game::actors::Attributes,
            &crate::game::actors::Skills,
        ),
        With<crate::game::actors::CanFight>,
    >,
    watch: Query<&crate::game::actors::Attributes, With<crate::game::actors::Lucid>>,
    level: PlayedLevel,
    obstacles: WorldObstacles,
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
    let (position, body, the_party, roster, movement) = *party;
    let (eye, forward) = (PartyBody::eye(position), body.forward());
    let shadow_hound = laid.bonus(the_party).warns;
    let searching = search.on(false);
    let view = crate::game::world::perception::View { eye, forward, far, searching };
    let members = || members.iter_many(roster.iter()).matched();
    let seen = joined.iter().any(|&entity| {
        let (monster, vitals, level, camouflage) = monsters.get(entity).unwrap();
        view.sees(
            members(),
            shadow_hound,
            (monster, vitals),
            (*level, camouflage.copied()),
            places.of(entity).feet,
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
        watch.iter_many(roster.iter()).matched(),
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
        .map(|&m| (m, places.of(m).feet))
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
