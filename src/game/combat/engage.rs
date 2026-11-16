//! Who comes into a fight, and how one opens (`docs/rules/monsters.md`
//! section 3).
//!
//! While a fight runs, a hostile group fighting turns hostile the other
//! groups of its faction whose leader sees its leader ([`alert_factions`]).
//! A group may turn hostile by its kind's radius ([`turn_by_radius`]). Then
//! groups outside the fight come in by four rules: provoked
//! ([`come_in_provoked`]), opening a fight ([`look_to_open`]), or joining a
//! running one ([`join`]); those coming in are told of together ([`tell`]),
//! and join the fight or open one ([`open_battle`]). NPC bodies keep to
//! their roles.

use super::monster::aim::{self, AimsMade, Considers, Spoiled, Weigh, Worth};
use super::monster::slot::{AtEnemies, Breath, Castable, Slots};
use super::monster::special::SpecialReady;
use super::*;
use crate::game::world::navigation::walking_distance;
use crate::game::world::population::CastChance;

use super::monster::sight::{ENGAGEMENT_RANGE, InPartyView, MakesOutParty, Reinforces, Sight};
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

/// The fighters of a group outside the fight: its members alive, not an
/// NPC's body, and neither fighting nor coming in already, in its order.
fn fighters_of(
    members: &GroupMembers,
    monsters: &Query<(&Allegiance, Has<Combatant>, Has<ComingIn>)>,
    sight: &Sight,
) -> Vec<Entity> {
    members
        .iter()
        .filter(|&m| sight.alive(m))
        .filter(|&m| {
            monsters
                .get(m)
                .is_ok_and(|(allegiance, fighting, coming)| !allegiance.npc && !fighting && !coming)
        })
        .collect()
}

/// A group not hostile whose kind has a hostility radius turns hostile when
/// one of its fighters makes out the party and the nearest is within the
/// radius.
#[allow(clippy::type_complexity)]
pub(super) fn turn_by_radius(
    groups: Query<(Entity, &GroupMembers, &LevelOwned), With<MonsterGroup>>,
    monsters: Query<(&Allegiance, Has<Combatant>, Has<ComingIn>)>,
    making_out: Query<(), With<MakesOutParty>>,
    sight: Sight,
    level: PlayedLevel,
    mut commands: Commands,
) {
    let Some(root) = level.entity() else { return };
    for (group, members, owner) in &groups {
        let fighters = fighters_of(members, &monsters, &sight);
        let Some(&first) = fighters.first().filter(|_| owner.0 == root) else { continue };
        let Ok((allegiance, ..)) = monsters.get(first) else { continue };
        let Some(radius) = allegiance.hostility_radius() else { continue };
        let nearest = fighters.iter().map(|&m| sight.party_gap(m)).fold(f32::INFINITY, f32::min);
        let sees = fighters.iter().any(|&m| making_out.contains(m));
        if sight.side(first) != Some(Stance::Hostile) && sees && nearest <= radius {
            commands.trigger(Stand { group, stance: Some(Stance::Hostile) });
        }
    }
}

/// A group the party has just acted against comes in at once, however far.
#[allow(clippy::type_complexity)]
pub(super) fn come_in_provoked(
    groups: Query<(Entity, &GroupMembers), (With<MonsterGroup>, With<hostility::Provoked>)>,
    monsters: Query<(&Allegiance, Has<Combatant>, Has<ComingIn>)>,
    sight: Sight,
    mut commands: Commands,
) {
    for (group, members) in &groups {
        commands.entity(group).remove::<hostility::Provoked>();
        for fighter in fighters_of(members, &monsters, &sight) {
            if sight.side(fighter) == Some(Stance::Hostile) {
                commands.entity(fighter).insert(ComingIn);
            }
        }
    }
}

/// A look at the party a fighter takes for its group, to open a fight.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = Looks)]
struct LookFor(Entity);

/// The looks a group's fighters took.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = LookFor, linked_spawn)]
struct Looks(Vec<Entity>);

/// A group's fighters have looked at the party.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct Looked {
    #[event_target]
    group: Entity,
}

/// With no fight on, a hostile group one of whose fighters makes out the
/// party looks at it: each fighter as it would deliberate, at itself and
/// the living members, weighing its slots at enemies that may be cast, if
/// it casts, and its breath if it is ready.
#[allow(clippy::type_complexity)]
pub(super) fn look_to_open(
    groups: Query<(Entity, &GroupMembers, &LevelOwned), With<MonsterGroup>>,
    monsters: Query<(&Allegiance, Has<Combatant>, Has<ComingIn>)>,
    making_out: Query<(), With<MakesOutParty>>,
    casters: Query<(&CastChance, &Slots, Has<SpecialReady>)>,
    slots: Query<(Has<AtEnemies>, Has<Castable>, Has<Breath>)>,
    sight: Sight,
    level: PlayedLevel,
    mut commands: Commands,
) {
    let Some(root) = level.entity() else { return };
    if !sight.roster().iter().any(|member| sight.alive(member)) {
        return;
    }
    for (group, members, owner) in &groups {
        let fighters = fighters_of(members, &monsters, &sight);
        let hostile = fighters.first().is_some_and(|&m| sight.side(m) == Some(Stance::Hostile));
        if owner.0 != root || !hostile || !fighters.iter().any(|&m| making_out.contains(m)) {
            continue;
        }
        for &fighter in &fighters {
            let look = commands.spawn((Considers(fighter), LookFor(group))).id();
            let living = std::iter::once(fighter)
                .chain(sight.roster().iter().filter(|&member| sight.alive(member)));
            aim::view(&mut commands, look, living);
            let Ok((chance, owned, ready)) = casters.get(fighter) else { continue };
            for slot in owned.iter() {
                let Ok((at_enemies, castable, breath)) = slots.get(slot) else { continue };
                let weighed = match breath {
                    true => ready,
                    false => chance.0 != 0 && castable && at_enemies,
                };
                if weighed {
                    commands.trigger(Weigh { slot, considering: look });
                }
            }
        }
        commands.trigger(Looked { group });
    }
}

/// The group opens a fight if one of its fighters can act on the party
/// (retail `MonsterGroupCanEngage`): one able to fight, one of whose arms
/// reaches the party, or whose longest is for close quarters and it walks,
/// or which has an aim worth casting that spoils nothing.
#[allow(clippy::type_complexity)]
fn open_if_able(
    looked: On<Looked>,
    groups: Query<&Looks>,
    looks: Query<(&Considers, Option<&AimsMade>)>,
    fighters: Query<(
        &Monster,
        &crate::game::actors::arm::Furthest,
        Has<crate::game::actors::CanFight>,
    )>,
    good: Query<(), (With<Worth>, Without<Spoiled>)>,
    sight: Sight,
    mut commands: Commands,
) {
    let Ok(taken) = groups.get(looked.group) else { return };
    let can_act = |(&Considers(fighter), made): (&Considers, Option<&AimsMade>)| {
        let Ok((monster, reach, fit)) = fighters.get(fighter) else { return false };
        let close = reach.0.is_some_and(|r| r <= crate::content::species::Reach::Short);
        let aimed = made.is_some_and(|aims| aims.iter().any(|aim| good.contains(aim)));
        fit && (reach.reaches(sight.party_gap(fighter))
            || (close && monster.body.speed > 0.0)
            || aimed)
    };
    if looks.iter_many(taken.iter()).flatten().any(can_act) {
        for (&Considers(fighter), _) in looks.iter_many(taken.iter()).flatten() {
            commands.entity(fighter).insert(ComingIn);
        }
    }
    commands.entity(looked.group).despawn_related::<Looks>();
}

/// Into a running fight a hostile group comes when one of its fighters
/// makes out the party and its leader (or its first fighter) is in reach of
/// it: its longest reach and a stride, 65 m at the least, both as the crow
/// flies and by the way it would walk over the navmesh for its size (retail
/// `FindNavigatorPathDistance`); or when one of its fighters in the party's
/// view, not of a kind that keeps hidden, stands within extreme reach
/// (`MonsterGroupHasRenderableMember`). A neutral group comes when one of
/// its fighters stands with the fight (`ShouldMonsterGroupEnterCombat`).
#[allow(clippy::type_complexity)]
pub(super) fn join(
    groups: Query<(&GroupMembers, &LevelOwned), With<MonsterGroup>>,
    monsters: Query<(&Allegiance, Has<Combatant>, Has<ComingIn>)>,
    leading: Query<(), With<Leads>>,
    bodies: Query<(&Monster, &crate::game::actors::Nature, &crate::game::actors::arm::Furthest)>,
    making_out: Query<(), With<MakesOutParty>>,
    in_view: Query<(), With<InPartyView>>,
    reinforcing: Query<(), With<Reinforces>>,
    hiding: Query<&crate::game::world::population::Temper>,
    sight: Sight,
    level: PlayedLevel,
    (navigation, archipelagos): (
        Option<Single<&crate::game::world::navigation::LevelNavigation>>,
        Query<&bevy_landmass::Archipelago3d>,
    ),
    mut commands: Commands,
) {
    let Some(root) = level.entity() else { return };
    let leader_in_reach = |fighters: &[Entity]| {
        let leader = fighters.iter().copied().find(|&m| leading.contains(m));
        let Some(leader) = leader.or(fighters.first().copied()) else { return false };
        let Ok((monster, nature, reach)) = bodies.get(leader) else { return false };
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
    let near_in_view = |m: Entity| {
        in_view.contains(m)
            && !hiding.get(m).is_ok_and(|temper| temper.0.camouflaged)
            && sight.party_gap(m) <= crate::content::species::Reach::Extreme.metres()
    };
    for (members, owner) in &groups {
        let fighters = fighters_of(members, &monsters, &sight);
        let Some(&first) = fighters.first().filter(|_| owner.0 == root) else { continue };
        let enters = match sight.side(first) {
            Some(Stance::Hostile) => {
                let makes_out = fighters.iter().any(|&m| making_out.contains(m));
                (makes_out && leader_in_reach(&fighters))
                    || fighters.iter().any(|&m| near_in_view(m))
            }
            Some(Stance::Neutral) => fighters.iter().any(|&m| reinforcing.contains(m)),
            _ => false,
        };
        if enters {
            for fighter in fighters {
                commands.entity(fighter).insert(ComingIn);
            }
        }
    }
}

/// What is added to the engagement range for a leader's reach (retail's
/// 15 m).
const ENGAGEMENT_FLOOR: f32 = 15.0;

/// On a monster coming into the fight, until it is told of.
#[derive(Component, Clone, Copy, Debug, Default)]
pub(super) struct ComingIn;

/// A neutral group whose kind has a hostility radius turns hostile as it
/// comes in.
fn turn_as_it_comes(
    coming: On<Add<ComingIn>>,
    monsters: Query<(&Allegiance, &GroupMember)>,
    sight: Sight,
    mut commands: Commands,
) {
    let monster = coming.entity;
    let Ok((allegiance, &GroupMember(group))) = monsters.get(monster) else { return };
    if sight.side(monster) == Some(Stance::Neutral) && allegiance.hostility_radius().is_some() {
        commands.trigger(Stand { group, stance: Some(Stance::Hostile) });
    }
}

/// Those coming in are told of together, in the order of their places in the
/// level.
pub(super) fn tell(
    coming: Query<(Entity, &Monster), With<ComingIn>>,
    mut engaged: MessageWriter<Engaged>,
    mut commands: Commands,
) {
    let mut joined: Vec<(i32, Entity)> =
        coming.iter().map(|(monster, kind)| (kind.location, monster)).collect();
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
    app.add_observer(open_if_able).add_observer(turn_as_it_comes);
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
