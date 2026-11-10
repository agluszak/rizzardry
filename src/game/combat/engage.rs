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

use super::monster::coming::{self, Acting, Alert, Alerted, Entering, Leader, Looking};
use super::monster::facts::{Fray, Kind, Records, Sight};
use super::monster::{special, spell};
use crate::game::actors::{Level, Places};
use crate::game::hits::effects::BodyReadOnly;
use crate::game::party::body::PartyBody;
use crate::game::world::hostility::{Allegiance, Stand};
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::{Camouflage, Special, Spellcasting, Temper};
use avian3d::prelude::Position;

/// Monsters coming into the fight, in placement order.
#[derive(Message)]
pub(super) struct Engaged(pub(super) Vec<Entity>);

/// While a fight runs, a fighting hostile group alerts the groups of its
/// faction whose leaders see its own. A group's leader is its first living
/// member.
#[allow(clippy::type_complexity)]
pub(super) fn alert_factions(
    monsters: Query<(&Allegiance, Has<Combatant>), With<Monster>>,
    groups: Query<(Entity, &GroupMembers, &LevelOwned)>,
    (bodies, places, laid, obstacles, group_members): (
        Query<BodyReadOnly>,
        Places,
        crate::game::actors::Laid,
        WorldObstacles,
        Query<&'static GroupMember>,
    ),
    party: Single<(Entity, &PartyRoster)>,
    level: PlayedLevel,
    mut commands: Commands,
) {
    let (Some(root), Some(far)) = (level.entity(), level.view_distance()) else { return };
    let (party, roster) = *party;
    let sight = Sight {
        bodies: &bodies,
        places: &places,
        laid: &laid,
        groups: &group_members,
        party,
        roster,
        far,
        obstacles: &obstacles,
    };
    let hostile = |monster: Entity| sight.side(monster) == Some(Stance::Hostile);
    let leaders: Vec<(Entity, Entity, &Allegiance, bool)> = groups
        .iter()
        .filter(|(.., owner)| owner.0 == root)
        .filter_map(|(group, members, _)| {
            let leader = members.iter().find(|&member| sight.alive(member))?;
            let (allegiance, fighting) = monsters.get(leader).ok()?;
            Some((group, leader, allegiance, fighting))
        })
        .collect();
    for &(group, leader, allegiance, _) in &leaders {
        let alerts: Vec<Alert> = leaders
            .iter()
            .map(|&(other, theirs, their_allegiance, fighting)| {
                let same_faction = their_allegiance.faction == allegiance.faction;
                let fighting_hostile = fighting && hostile(theirs);
                Alert {
                    other: other != group,
                    same_faction,
                    fighting_hostile,
                    // Looked for only where the rest holds, as nothing else can come of it.
                    seen: same_faction && fighting_hostile && sight.sees(leader, theirs),
                }
            })
            .collect();
        let alerted = Alerted {
            npc: allegiance.npc,
            unaligned: allegiance.faction == crate::game::factions::UNALIGNED,
            hostile: hostile(leader),
            leaders: &alerts,
        };
        if coming::is_alerted(&alerted) {
            commands.trigger(Stand { group, stance: Some(Stance::Hostile) });
        }
    }
}

/// Each group outside the fight, in authored order, then any monster
/// without one, comes in when its side and sight of the party call for it.
#[allow(clippy::type_complexity)]
pub(super) fn engage(
    battles: Query<(), With<Battle>>,
    monsters: Query<(
        Entity,
        &Monster,
        &Allegiance,
        &Temper,
        &Spellcasting,
        &Special,
        Option<&GroupMember>,
        Has<Combatant>,
        Option<&Camouflage>,
    )>,
    groups: Query<(Entity, &MonsterGroup, &GroupMembers, &LevelOwned)>,
    (bodies, places, laid, obstacles, group_members): (
        Query<BodyReadOnly>,
        Places,
        crate::game::actors::Laid,
        WorldObstacles,
        Query<&'static GroupMember>,
    ),
    party: Single<(Entity, &PartyRoster)>,
    level: PlayedLevel,
    mut provocations: MessageReader<hostility::Provoked>,
    content: If<Res<Catalogs>>,
    definitions: Res<crate::game::hits::definitions::Definitions>,
    mut engaged: MessageWriter<Engaged>,
    mut commands: Commands,
) {
    let provoked: Vec<Entity> = provocations
        .read()
        .filter_map(|&hostility::Provoked(m)| Some(monsters.get(m).ok()?.6?.0))
        .collect();
    let running = !battles.is_empty();
    let (the_party, roster) = *party;
    let (Some(far), Some(root)) = (level.view_distance(), level.entity()) else { return };
    let sight = Sight {
        bodies: &bodies,
        places: &places,
        laid: &laid,
        groups: &group_members,
        party: the_party,
        roster,
        far,
        obstacles: &obstacles,
    };
    if !running && !roster.iter().any(|member| sight.alive(member)) {
        return;
    }
    let records = Records { spells: &content.spells, definitions: &definitions };
    // Those who fight: alive, and not an NPC.
    let fights = |monster: Entity| {
        sight.alive(monster)
            && monsters.get(monster).is_ok_and(|(_, _, allegiance, ..)| !allegiance.npc)
    };
    // How far a fighter is, whether it sees the party, and whether it can act
    // on it, which it sees of the fight-to-be: itself and the party it makes out.
    let look = |fighter: Entity| -> Option<Looking> {
        let (_, monster, _, temper, casting, special, ..) = monsters.get(fighter).ok()?;
        let own = bodies.get(fighter).ok()?;
        let sees_party = sight.sees_party(fighter);
        let kind = Kind { temper, special, casting, cooling: false, npc: false, fleeing: false };
        let fray = std::iter::once(fighter).chain(roster.iter()).collect::<Vec<_>>();
        let known = sight.know(
            fighter,
            kind,
            Fray { bodies: &fray, fighting: false, sees_party },
            records,
        )?;
        let acting = Acting {
            can_fight: own.vitals.can_fight(),
            striking: known.striking(),
            walks: monster.body.speed > 0.0,
            casts: spell::casts_at_foes(&known.casting()),
            breathes: special::ready(&known.special()),
        };
        Some(Looking {
            party_gap: sight.party_gap(fighter),
            sees_party,
            acts: coming::acts_on_party(&acting),
        })
    };
    // The fight's hostile fighters in the party's view, whom a neutral group may
    // stand with.
    let hostiles: Vec<coming::Standing> = monsters
        .iter()
        .filter(|&(monster, .., fighting, _)| {
            fighting && sight.alive(monster) && sight.side(monster) == Some(Stance::Hostile)
        })
        .map(|(monster, _, _, temper, ..)| sight.standing(monster, temper.0.camouflaged))
        .collect();
    // Each group outside the fight in authored order, then any monster
    // without one.
    let mut units: Vec<(i32, Option<Entity>, Vec<Entity>)> = groups
        .iter()
        .filter(|(.., owner)| owner.0 == root)
        .map(|(entity, group, members, _)| {
            (group.authored, Some(entity), members.iter().filter(|&m| fights(m)).collect())
        })
        .collect();
    units.sort_unstable_by_key(|&(authored, ..)| authored);
    units.extend(
        monsters
            .iter()
            .filter(|&(monster, .., group, _, _)| group.is_none() && fights(monster))
            .map(|(monster, ..)| (i32::MAX, None, vec![monster])),
    );
    let mut joined: Vec<(i32, Entity)> = Vec::new();
    for (_, group, fighters) in units {
        // Those already fighting stay; the rest of their group may join them.
        let fighters: Vec<Entity> =
            fighters.into_iter().filter(|&m| monsters.get(m).is_ok_and(|m| !m.7)).collect();
        let Some(&first) = fighters.first() else { continue };
        let (_, _, allegiance, ..) = monsters.get(first).unwrap();
        let looks: Vec<Looking> = fighters.iter().filter_map(|&m| look(m)).collect();
        let standing: Vec<coming::Standing> = fighters
            .iter()
            .filter_map(|&m| {
                let temper = monsters.get(m).ok()?.3;
                Some(sight.standing(m, temper.0.camouflaged))
            })
            .collect();
        let leader = bodies.get(first).unwrap();
        let side = sight.side(first).unwrap_or_default();
        let entering = Entering {
            stance: side,
            fight_running: running,
            provoked: group.is_some_and(|g| provoked.contains(&g)),
            radius: allegiance.hostility_radius(),
            members: &looks,
            leader: Leader {
                party_gap: sight.party_gap(first),
                reach: leader.attacks.arms.iter().map(|arm| arm.reach).max(),
                stride: leader.attacks.stride.unwrap_or(0.0),
            },
            standing: &standing,
            hostiles: &hostiles,
        };
        let came = coming::comes_in(&entering);
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
    members: Query<(&Character, &Vitals, &crate::game::actors::Attributes)>,
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
    let shadow_hound = laid.has(the_party, crate::content::spells::SpellId::SHADOW_HOUND);
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
        members().map(|(_, vitals, attributes)| (attributes, vitals)),
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
