//! Who comes into a fight, and how one opens.
//!
//! While a fight runs, a hostile group fighting turns hostile the other
//! groups of its faction whose leader sees its leader ([`alert_factions`]).
//! Then each group outside the fight, in authored order, turns hostile when
//! the party comes within its hostility radius in its sight ([`engage`]). A
//! hostile group enters with all its living members when the party has
//! just acted against it. Otherwise a fight starts when a member sees the
//! party and a member can act on it: one of its attacks reaches, or its
//! best attack is close (touch or short) and it can walk up, or it can
//! cast or breathe at the party. A fight under way takes in a hostile
//! group that sees the party with its leader in reach, and a neutral group
//! standing with it. NPC groups keep to their roles. Those who come in
//! join the fight, or open one ([`open_battle`]). (The rules are retail's
//! `MonsterGroupCanEngage`, `ShouldMonsterGroupEnterCombat`,
//! `AlertSameFactionGroups`; its path-length check is taken as the
//! straight distance, and its rendered-member clause is not modelled.)

use super::*;

use super::ai::special_ready;
use super::monster_spells;
use super::sight::Sight;
use crate::game::actors::Places;
use crate::game::hits::effects::BodyReadOnly;
use crate::game::party::body::PartyBody;
use crate::game::world::hostility::{Allegiance, Stand};
use crate::game::world::population::{Special, Spellcasting, Temper};
use avian3d::prelude::Position;

/// Monsters coming into the fight, in placement order.
#[derive(Message)]
pub(super) struct Engaged(Vec<Entity>);

/// While a fight runs, a fighting hostile group alerts the groups of its
/// faction whose leaders see its own. A group's leader is its first living
/// member.
pub(super) fn alert_factions(
    monsters: Query<(&Allegiance, Has<Combatant>), With<Monster>>,
    groups: Query<(Entity, &GroupMembers, &LevelOwned)>,
    sight: Sight,
    level: PlayedLevel,
    mut commands: Commands,
) {
    let Some(root) = level.entity() else { return };
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
    let alerting = |faction: i32| {
        leaders.iter().filter(move |&&(_, leader, theirs, fighting)| {
            fighting && theirs.faction == faction && hostile(leader)
        })
    };
    for &(group, leader, allegiance, _) in &leaders {
        // NPCs keep to their own roles here.
        if allegiance.npc
            || allegiance.faction == crate::game::factions::UNALIGNED
            || hostile(leader)
        {
            continue;
        }
        if alerting(allegiance.faction)
            .any(|&(source, target, ..)| source != group && sight.sees(leader, target))
        {
            commands.trigger(Stand { group, stance: Some(Stance::Hostile) });
        }
    }
}

/// Each group outside the fight, in authored order, then any monster
/// without one, comes in when its side and sight of the party call for it.
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
    )>,
    groups: Query<(Entity, &MonsterGroup, &GroupMembers, &LevelOwned)>,
    bodies: Query<BodyReadOnly>,
    (sight, laid): (Sight, crate::game::actors::Laid),
    party: Single<(Entity, &PartyRoster)>,
    level: PlayedLevel,
    mut provocations: MessageReader<hostility::Provocation>,
    content: If<Res<Catalogs>>,
    definitions: Res<crate::game::hits::definitions::Definitions>,
    mut engaged: MessageWriter<Engaged>,
    mut commands: Commands,
) {
    let provoked: Vec<Entity> = provocations
        .read()
        .filter_map(|&hostility::Provocation(m)| Some(monsters.get(m).ok()?.6?.0))
        .collect();
    let running = !battles.is_empty();
    let (the_party, roster) = *party;
    if !running && !roster.iter().any(|member| sight.alive(member)) {
        return;
    }
    let (Some(far), Some(root)) = (level.view_distance(), level.entity()) else { return };
    let fight = ai::Fight {
        bodies: &bodies,
        laid: &laid,
        places: &sight.places,
        party: the_party,
        roster,
        combatants: &[],
        far,
        obstacles: &sight.obstacles,
        content: &content,
        definitions: &definitions,
    };
    // Those who fight: alive, and not an NPC.
    let fights = |monster: Entity| {
        sight.alive(monster)
            && monsters.get(monster).is_ok_and(|(_, _, allegiance, ..)| !allegiance.npc)
    };
    // How far a fighter is, whether it sees the party, and whether it can
    // act on it: one of its attacks reaches, or its best attack is close
    // and it can walk up, or it can cast or breathe at the party, which it
    // sees of the fight-to-be.
    let look = |fighter: Entity| -> Option<(f32, bool, bool)> {
        let (_, monster, _, _, casting, special, ..) = monsters.get(fighter).ok()?;
        let own = bodies.get(fighter).ok()?;
        let (distance, sees) = (sight.party_gap(fighter), sight.sees_party(fighter));
        let arms = &own.attacks.arms;
        let reaches = arms.iter().any(|arm| distance <= arm.reach.metres());
        let close = arms.iter().map(|arm| arm.reach).max().is_some_and(|r| r <= Reach::Short);
        let seen: Vec<Entity> = std::iter::once(fighter)
            .chain(roster.iter().filter(|_| sees))
            .filter(|&body| sight.alive(body))
            .collect();
        let casts = monster_spells::casts_at_foes(fighter, casting, &seen, &fight);
        let cone = content.spells.get(crate::content::spells::SpellId::SPECIAL_ATTACK_CONE);
        let breathes = special.0.chance != 0
            && special_ready(own.vitals, &special.0, false, None)
            && cone
                .is_some_and(|cone| !monster_spells::aims(cone, fighter, &seen, &fight).is_empty());
        let acts = own.vitals.can_fight()
            && (reaches || (close && monster.body.speed > 0.0) || casts || breathes);
        Some((distance, sees, acts))
    };
    // Whether a group's leader (its first fighter) is in reach of the
    // party: its best attack's reach and one combat move, never under the
    // engagement range and its floor.
    let leader_reaches = |leader: Entity| {
        bodies.get(leader).is_ok_and(|own| {
            let best = own.attacks.arms.iter().map(|arm| arm.reach.metres()).fold(0.0, f32::max);
            let reach = (best + own.attacks.stride.unwrap_or(0.0))
                .max(sight::ENGAGEMENT_RANGE + sight::ENGAGEMENT_FLOOR);
            sight.party_gap(leader) <= reach
        })
    };
    // The fight's hostile fighters in the party's view, whom a neutral
    // group may stand with.
    let hostiles: Vec<Entity> = monsters
        .iter()
        .filter(|&(monster, .., fighting)| {
            fighting
                && sight.alive(monster)
                && sight.in_party_view(monster)
                && sight.side(monster) == Some(Stance::Hostile)
        })
        .map(|(monster, ..)| monster)
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
            .filter(|&(monster, .., group, _)| group.is_none() && fights(monster))
            .map(|(monster, ..)| (i32::MAX, None, vec![monster])),
    );
    let mut joined: Vec<(i32, Entity)> = Vec::new();
    for (_, group, fighters) in units {
        // Those already fighting stay; the rest of their group may join them.
        let fighters: Vec<Entity> =
            fighters.into_iter().filter(|&m| monsters.get(m).is_ok_and(|m| !m.7)).collect();
        let Some(&first) = fighters.first() else { continue };
        let (_, _, allegiance, temper, ..) = monsters.get(first).unwrap();
        let looks: Vec<_> = fighters.iter().filter_map(|&m| look(m)).collect();
        let nearest = looks.iter().map(|l| l.0).fold(f32::INFINITY, f32::min);
        let sees = looks.iter().any(|l| l.1);
        let acts = looks.iter().any(|l| l.2);
        let provoked = group.is_some_and(|g| provoked.contains(&g));
        let side = sight.side(first).unwrap_or_default();
        // Coming within a hostility radius in the group's sight turns it.
        let turns = side != Stance::Hostile && sees && allegiance.within_radius(nearest);
        let enters = match if turns { Stance::Hostile } else { side } {
            // A fight starts when a hostile group can act on the party; one
            // under way takes in a hostile group that sees the party with
            // its leader in reach.
            Stance::Hostile if running => provoked || (sees && leader_reaches(first)),
            Stance::Hostile => provoked || (sees && acts),
            // A neutral group joins a running fight it stands with.
            Stance::Neutral if running => {
                let standing: Vec<Entity> =
                    fighters.iter().copied().filter(|_| !temper.0.camouflaged).collect();
                sight.reinforces(&standing, &hostiles)
            }
            _ => false,
        };
        // A kind with a hostility radius turns hostile as it enters, too.
        if let Some(group) = group
            && (turns || (enters && side == Stance::Neutral && allegiance.within_radius(0.0)))
        {
            commands.trigger(Stand { group, stance: Some(Stance::Hostile) });
        }
        if enters {
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
    monsters: Query<(&Monster, &Vitals)>,
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
    let (position, body, the_party, roster, movement) = *party;
    let (eye, forward) = (PartyBody::eye(position), body.forward());
    let shadow_hound = laid.has(the_party, crate::content::spells::SpellId::SHADOW_HOUND);
    let searching = search.on(false);
    let view = crate::game::world::perception::View { eye, forward, far, searching };
    let members = || members.iter_many(roster.iter()).matched();
    let seen = joined.iter().any(|&entity| {
        let (monster, vitals) = monsters.get(entity).unwrap();
        view.sees(
            members(),
            shadow_hound,
            (monster, vitals),
            &catalog[monster.species],
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
