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

use super::monster::special::SpecialReady;
use super::monster::spell::{
    self, Aims, Breath, Considers, SeesParty, WeighedIn, Weighing, Weighs,
};
use super::*;
use crate::content::spells::SpellId;
use crate::game::characters::{Condition, Conditions};
use crate::game::world::navigation::walking_distance;
use crate::game::world::population::{Special, Spellcasting};

use super::monster::sight::{ENGAGEMENT_RANGE, Sight};
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
/// one, comes in or not ([`ComesIn`]). Whether each of its fighters could act
/// on the party is looked at first: each weighs the spells of its slots aimed
/// at enemies, and its breath if it is ready, as it would in a fight, at
/// itself and the party. Those who come in are told of together, in the
/// order of their places in the level ([`Told`]).
#[allow(clippy::type_complexity)]
pub(super) fn engage(
    battles: Query<(), With<Battle>>,
    monsters: Query<(Entity, &Allegiance, Option<&GroupMember>, Has<Combatant>)>,
    groups: Query<(Entity, &MonsterGroup, &GroupMembers, &LevelOwned, Has<hostility::Provoked>)>,
    leading: Query<(), With<Leads>>,
    kinds: Query<(
        (&Special, &Spellcasting, &Conditions),
        Has<SpecialReady>,
        &crate::game::actors::arm::Furthest,
    )>,
    sight: Sight,
    level: PlayedLevel,
    content: If<Res<Catalogs>>,
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
            && monsters.get(monster).is_ok_and(|(_, allegiance, ..)| !allegiance.npc)
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
            .filter(|&(monster, _, group, _)| group.is_none() && fights(monster))
            .map(|(monster, ..)| (i32::MAX, None, vec![monster])),
    );
    for (_, group, fighters) in units {
        // Those already fighting stay; the rest of their group may join them.
        let fighters: Vec<Entity> =
            fighters.into_iter().filter(|&m| monsters.get(m).is_ok_and(|m| !m.3)).collect();
        let Some(&first) = fighters.first() else { continue };
        let (_, allegiance, ..) = monsters.get(first).unwrap();
        let side = (sight.side(first).unwrap_or_default(), allegiance.hostility_radius());
        // Its leader, if it is among those who may come; else the first of them.
        let leader =
            fighters.iter().copied().find(|&fighter| leading.contains(fighter)).unwrap_or(first);
        let provoked = group.is_some_and(|g| provoked.contains(&g));
        // Whether a fighter can act on the party is asked only of a group
        // that would open a fight by it: hostile, or turning so by its
        // radius, not provoked, with a member who sees the party.
        let asks = !running
            && !provoked
            && (side.0 == Stance::Hostile || side.1.is_some())
            && fighters.iter().any(|&fighter| sight.sees_party(fighter));
        let mut probes = Vec::new();
        for &fighter in fighters.iter().filter(|_| asks) {
            let Ok(((special, casting, conditions), ready, _)) = kinds.get(fighter) else {
                continue;
            };
            let bodies = std::iter::once(fighter).chain(sight.roster().iter()).collect();
            let probe = commands
                .spawn((Considers { by: fighter, bodies }, SeesParty(sight.sees_party(fighter))))
                .id();
            if casting.chance != 0 {
                let silenced = conditions.has(Condition::Silenced);
                for (id, weight) in casting.spells.iter().zip(spell::SLOT_WEIGHTS) {
                    let Some(spell) = id.and_then(|id| content.spells.get(id)) else { continue };
                    let modelled = crate::game::hits::definitions::row(spell).is_some();
                    if spell.aimed_at_enemies()
                        && spell::castable(spell, modelled, silenced, casting.silent_alchemy)
                    {
                        commands.spawn((Weighs { spell: spell.id, weight }, WeighedIn(probe)));
                    }
                }
            }
            // With no fight on, a special attack is a breath; a summoning
            // never opens a fight.
            if ready && !super::monster::special::summons(&special.0) {
                let cone = Weighs { spell: SpellId::SPECIAL_ATTACK_CONE, weight: 0 };
                commands.spawn((cone, Breath, WeighedIn(probe)));
            }
            probes.push((fighter, probe));
        }
        commands.trigger(ComesIn {
            group,
            fighters,
            leader,
            side,
            running,
            provoked,
            probes,
            hostiles: hostiles.clone(),
        });
    }
    commands.trigger(Told);
}

/// A group outside the fight, or a monster of none, comes in or not.
#[derive(Event, Clone, Debug)]
struct ComesIn {
    group: Option<Entity>,
    fighters: Vec<Entity>,
    leader: Entity,
    side: (Stance, Option<f32>),
    running: bool,
    provoked: bool,
    /// Each fighter's look at the party, its spells and breath weighed.
    probes: Vec<(Entity, Entity)>,
    hostiles: Vec<Entity>,
}

/// One coming into the fight, by its place in the level.
#[derive(Component, Clone, Copy, Debug)]
struct ComingIn(i32);

/// A group outside the fight comes in or not (retail `MonsterGroupCanEngage`,
/// `ShouldMonsterGroupEnterCombat`). A group not hostile whose kind has a
/// radius turns hostile when a member sees the party and its nearest is
/// within the radius. A hostile group comes in at once when the party has
/// just acted against it. Otherwise, with no fight on, it opens one when a
/// member sees the party and a member can act on it: one able to fight, one
/// of whose arms reaches the party, or whose longest is for close quarters
/// and it walks, or one of whose spells weighed or whose breath has an aim.
/// Into a running fight it comes when a member sees the party and its leader
/// is in reach of it (its longest reach and a stride, 65 m at the least,
/// both as the crow flies and by the way it would walk over the navmesh for
/// its size, retail `FindNavigatorPathDistance`), or when one of its members
/// the party can see, not of a kind that keeps hidden, stands within extreme
/// reach (`MonsterGroupHasRenderableMember`). A neutral group joins only a
/// running fight it stands with, turning hostile as it does if its kind has
/// a radius. A friendly group never comes in. Those who come in are marked
/// to be told.
#[allow(clippy::type_complexity)]
fn comes_in(
    group: On<ComesIn>,
    monsters: Query<(
        (&Monster, &crate::game::actors::Nature),
        &crate::game::actors::arm::Furthest,
        Has<crate::game::actors::CanFight>,
    )>,
    weighing: Query<&Weighing>,
    weighed: Query<&Aims>,
    sight: Sight,
    (navigation, archipelagos): (
        Option<Single<&crate::game::world::navigation::LevelNavigation>>,
        Query<&bevy_landmass::Archipelago3d>,
    ),
    mut commands: Commands,
) {
    let ComesIn { group, ref fighters, leader, side, running, provoked, ref probes, ref hostiles } =
        *group;
    let (stance, radius) = side;
    let nearest = fighters.iter().map(|&m| sight.party_gap(m)).fold(f32::INFINITY, f32::min);
    let sees = || fighters.iter().any(|&m| sight.sees_party(m));
    let turns = stance != Stance::Hostile && radius.is_some_and(|r| nearest <= r) && sees();
    let acts = || {
        probes.iter().any(|&(fighter, probe)| {
            let Ok(((monster, _), reach, fit)) = monsters.get(fighter) else { return false };
            let close = reach.0.is_some_and(|r| r <= crate::content::species::Reach::Short);
            let aimed = weighing.get(probe).is_ok_and(|spells| {
                spells
                    .iter()
                    .filter_map(|spell| weighed.get(spell).ok())
                    .any(|aims| !aims.0.is_empty())
            });
            fit && (reach.reaches(sight.party_gap(fighter))
                || (close && monster.body.speed > 0.0)
                || aimed)
        })
    };
    let leader_in_reach = || {
        let Ok(((monster, nature), reach, _)) = monsters.get(leader) else { return false };
        let reach = reach.0.map_or(0.0, |reach| reach.metres()) + nature.stride.unwrap_or(0.0);
        let reach = reach.max(ENGAGEMENT_RANGE + ENGAGEMENT_FLOOR);
        let archipelago = navigation.as_ref().and_then(|navigation| {
            archipelagos.get(navigation.archipelago_for(monster.body.radius)).ok()
        });
        let (from, to) = (sight.places.of(leader).feet, sight.places.of(sight.party()).feet);
        sight.party_gap(leader) <= reach
            && archipelago
                .and_then(|archipelago| walking_distance(archipelago, from, to))
                .is_some_and(|walked| walked <= reach)
    };
    let hidden = |m: Entity| sight.tempers.get(m).is_ok_and(|temper| temper.0.camouflaged);
    let in_sight_near = || {
        fighters.iter().any(|&m| {
            sight.in_party_view(m)
                && !hidden(m)
                && sight.party_gap(m) <= crate::content::species::Reach::Extreme.metres()
        })
    };
    let enters = match if turns { Stance::Hostile } else { stance } {
        Stance::Hostile if provoked => true,
        Stance::Hostile if running => (sees() && leader_in_reach()) || in_sight_near(),
        Stance::Hostile => sees() && acts(),
        Stance::Neutral if running => sight.stands_with(fighters, hostiles),
        _ => false,
    };
    for &(_, probe) in probes {
        commands.entity(probe).despawn();
    }
    let radius_as_it_enters = enters && stance == Stance::Neutral && radius.is_some();
    if let Some(group) = group
        && (turns || radius_as_it_enters)
    {
        commands.trigger(Stand { group, stance: Some(Stance::Hostile) });
    }
    if enters {
        for &fighter in fighters {
            if let Ok(((monster, _), ..)) = monsters.get(fighter) {
                commands.entity(fighter).insert(ComingIn(monster.location));
            }
        }
    }
}

/// What is added to the engagement range for a leader's reach (retail's
/// 15 m).
const ENGAGEMENT_FLOOR: f32 = 15.0;

/// Every group has been looked at.
#[derive(Event, Clone, Copy, Debug)]
struct Told;

/// Those coming in are told of together, in the order of their places in the
/// level.
fn told(
    _: On<Told>,
    coming: Query<(Entity, &ComingIn)>,
    mut engaged: MessageWriter<Engaged>,
    mut commands: Commands,
) {
    let mut joined: Vec<(i32, Entity)> =
        coming.iter().map(|(monster, &ComingIn(location))| (location, monster)).collect();
    for &(_, monster) in &joined {
        commands.entity(monster).remove::<ComingIn>();
    }
    if joined.is_empty() {
        return;
    }
    joined.sort();
    engaged.write(Engaged(joined.into_iter().map(|(_, monster)| monster).collect()));
}

pub(super) fn plugin(app: &mut App) {
    app.add_observer(comes_in).add_observer(told);
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
