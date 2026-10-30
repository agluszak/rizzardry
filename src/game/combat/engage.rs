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

/// Monsters coming into the fight, in placement order.
#[derive(Message)]
pub(super) struct Engaged(Vec<Entity>);

type MonsterSight<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static Monster,
        ActorPose,
        Option<&'static Walker>,
        Option<&'static GroupMember>,
        Has<Combatant>,
    ),
>;
type Groups<'w, 's> =
    Query<'w, 's, (Entity, &'static mut MonsterGroup, &'static GroupMembers, &'static LevelOwned)>;

/// The party as monsters look for it on the played level.
#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct PartySight<'w, 's> {
    party: Single<'w, 's, (&'static Position, &'static PartyBody)>,
    effects: Single<'w, 's, &'static crate::game::magic::enchantments::PartyEffects>,
    members: Query<'w, 's, (&'static Character, &'static Vitals)>,
    roster: Single<'w, 's, &'static PartyRoster>,
    level: PlayedLevel<'w, 's>,
    obstacles: WorldObstacles<'w, 's>,
}

impl PartySight<'_, '_> {
    /// Monsters' sight of the party and the level it is played on.
    fn sight(&self) -> Option<(Sight<'_, '_, '_>, Entity)> {
        Some((
            Sight {
                eye: PartyBody::eye(self.party.0),
                concealment: self.effects.0.concealment(),
                far: self.level.view_distance()?,
                lowest_level: lowest_level(self.members.iter_many(self.roster.iter()).matched()),
                obstacles: &self.obstacles,
            },
            self.level.entity()?,
        ))
    }
}

/// While a fight runs, a fighting hostile group alerts the groups of its
/// faction whose leaders see its own (retail `AlertSameFactionGroups`).
pub(super) fn alert_factions(
    monsters: MonsterSight,
    mut groups: Groups,
    factions: Res<crate::game::factions::Factions>,
    clock: Res<GameClock>,
    content: If<Res<Catalogs>>,
    party: PartySight,
) {
    let catalog = &content.species;
    let Some((sight, root)) = party.sight() else { return };
    let now = clock.ms;
    let living = |membership: &GroupMembers| {
        membership
            .iter()
            .find(|&m| monsters.get(m).is_ok_and(|(_, monster, ..)| !monster.is_dead()))
    };
    let view = |monster: Entity| {
        monsters
            .get(monster)
            .ok()
            .map(|(_, body, at, walker, ..)| (body, at, walker, &catalog[body.species]))
    };
    // Fighting hostile groups by their leaders, and their factions.
    let alerting: Vec<(Entity, i32, Entity)> = groups
        .iter()
        .filter_map(|(entity, group, membership, owner)| {
            let leader = living(membership).filter(|_| owner.0 == root)?;
            let (_, body, .., fighting) = monsters.get(leader).ok()?;
            let kind = &catalog[body.species];
            (fighting
                && kind.faction != crate::game::factions::UNALIGNED
                && hostility::stance(group.set, kind, &factions, now, true)
                    == hostility::Stance::Hostile)
                .then_some((entity, kind.faction, leader))
        })
        .collect();
    for (entity, mut group, membership, owner) in &mut groups {
        let Some(leader) = living(membership).filter(|_| owner.0 == root) else { continue };
        let Some(observer) = view(leader) else { continue };
        // NPCs keep to their own roles here.
        if observer.3.npc.is_some() {
            continue;
        }
        let alerted = alerting.iter().any(|&(source, faction, target)| {
            source != entity
                && faction == observer.3.faction
                && view(target).is_some_and(|target| {
                    sight.sees_monster(
                        (observer.0, &observer.1, observer.2, observer.3),
                        (target.0, &target.1, target.2, target.3),
                    )
                })
        });
        let fighting = monsters.get(leader).is_ok_and(|m| m.5);
        if alerted
            && hostility::stance(group.set, observer.3, &factions, now, fighting)
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
    monsters: MonsterSight,
    (mut groups, factions): (Groups, Res<crate::game::factions::Factions>),
    mut provocations: MessageReader<hostility::Provocation>,
    clock: Res<GameClock>,
    content: If<Res<Catalogs>>,
    party: PartySight,
    mut engaged: MessageWriter<Engaged>,
) {
    let provoked: Vec<Entity> = provocations
        .read()
        .filter_map(|&hostility::Provocation(m)| Some(monsters.get(m).ok()?.4?.0))
        .collect();
    let running = !battles.is_empty();
    let roster = party.roster.iter();
    if !running
        && !crate::game::characters::anyone_alive(
            party.members.iter_many(roster).matched().map(|(_, v)| v),
        )
    {
        return;
    }
    let Some((sight, root)) = party.sight() else { return };
    let (catalog, spells) = (&content.species, &content.spells);
    let now = clock.ms;
    // Those of a group that fight: alive, and not an NPC.
    let fighters = |group: &GroupMembers| -> Vec<Entity> {
        group
            .iter()
            .filter(|&member| {
                monsters.get(member).is_ok_and(|(_, monster, ..)| {
                    !monster.is_dead() && catalog[monster.species].npc.is_none()
                })
            })
            .collect()
    };
    // How far a fighter is, whether it sees the party, and whether it can
    // act on it (retail `MonsterGroupCanEngage`): one of its attacks
    // reaches, or its best attack is close and it can walk up, or it can
    // cast or breathe at the party.
    let look = |fighter: Entity| -> Option<(f32, bool, bool)> {
        let (_, monster, at, walker, ..) = monsters.get(fighter).ok()?;
        let kind = &catalog[monster.species];
        let at_feet = feet(at.translation(), walker);
        let distance = party_distance(sight.eye, monster, at_feet);
        let sees = sight.sees(monster, &at, walker, kind);
        let reaches = kind.attacks.iter().any(|a| distance <= a.reach.metres());
        let close = kind.attacks.iter().map(|a| a.reach).max().is_some_and(|r| r <= Reach::Short);
        let party_at = PartyBody::feet_below(sight.eye);
        let toward = |victim: crate::game::magic::effects::Victim| {
            let turncoat = party
                .members
                .get(victim.entity())
                .is_ok_and(|(_, v)| v.conditions.has(Condition::Turncoat));
            hostility::toward_member(hostility::Stance::Hostile, turncoat)
        };
        let scene = monster_spells::Scene {
            caster: fighter,
            from: at_feet,
            party_at,
            members: party
                .roster
                .iter()
                .map(|m| (m, party.members.get(m).unwrap()))
                .filter(|(_, (_, v))| !v.is_dead())
                .map(|(m, (c, v))| member_in_scene((m, c, v), (party_at, distance), &toward))
                .collect(),
            monsters: Vec::new(),
            party: distance,
            sees_party: sees,
        };
        let casts = monster_spells::casts_at_foes(monster, kind, spells, &scene);
        let breathes = kind.special.chance != 0
            && special_ready(monster, kind, false, None)
            && spells
                .get(crate::content::spells::SpellId::SPECIAL_ATTACK_CONE)
                .is_some_and(|cone| !monster_spells::aims(cone, &scene).is_empty());
        let acts = monster.can_fight()
            && (reaches || (close && monster.body.speed > 0.0) || casts || breathes);
        Some((distance, sees, acts))
    };
    let seen = |member: Entity| -> Option<Seen> {
        let (_, monster, at, walker, ..) = monsters.get(member).ok()?;
        let kind = &catalog[monster.species];
        let at_feet = feet(at.translation(), walker);
        Some(Seen {
            entity: member,
            at: at_feet,
            from_party: party_distance(sight.eye, monster, at_feet),
            sees: sight.sees(monster, &at, walker, kind),
            in_view: sight.in_view(monster, &at, walker),
        })
    };
    // How far a monster is from the party.
    let nearest_of = |member: Entity| {
        monsters.get(member).map_or(f32::INFINITY, |(_, monster, at, walker, ..)| {
            party_distance(sight.eye, monster, feet(at.translation(), walker))
        })
    };
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
    // The fight's hostile fighters in the party's sight, as reinforcement
    // reads them (retail `CheckMonsterGroupsEnterCombat`).
    let hostiles: Vec<Seen> = if running {
        monsters
            .iter()
            .filter(|&(_, monster, .., group, fighting)| {
                let set = group.and_then(|g| groups.get(g.0).ok()).and_then(|(_, g, ..)| g.set);
                fighting
                    && !monster.is_dead()
                    && hostility::stance(set, &catalog[monster.species], &factions, now, true)
                        == hostility::Stance::Hostile
            })
            .filter_map(|(entity, ..)| seen(entity).filter(|s| s.in_view))
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
            .filter(|(_, monster, .., group, _)| {
                group.is_none() && !monster.is_dead() && catalog[monster.species].npc.is_none()
            })
            .map(|(entity, ..)| (i32::MAX, None, vec![entity])),
    );
    let mut joined: Vec<(i32, Entity)> = Vec::new();
    for (_, group, fighters) in units {
        // Those already fighting stay; the rest of their group may join them.
        let fighters: Vec<Entity> =
            fighters.into_iter().filter(|&m| monsters.get(m).is_ok_and(|m| !m.5)).collect();
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
                provoked || (sees && leader_reaches(kind, nearest_of(first)))
            }
            hostility::Stance::Hostile => provoked || (sees && acts),
            // A neutral group joins a running fight it stands with.
            hostility::Stance::Neutral if running => {
                let members: Vec<(Seen, bool)> = fighters
                    .iter()
                    .filter_map(|&m| seen(m))
                    .map(|s| (s, kind.temper.camouflaged))
                    .collect();
                reinforces(&members, &hostiles)
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
    (camp, search, movement): (
        Option<Res<crate::game::camp::Camp>>,
        Res<crate::game::world::search::Search>,
        Single<&crate::game::party::body::PartyMovement>,
    ),
    mut camp_news: MessageWriter<crate::game::camp::CampNews>,
    monsters: MonsterSight,
    content: If<Res<Catalogs>>,
    party: PartySight,
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
    let Some((sight, root)) = party.sight() else { return };
    info!("combat: {} monsters", joined.len());
    let catalog = &content.species;
    let forward = party.party.1.forward();
    let shadow_hound = party.effects.0.has(crate::content::spells::SpellId::SHADOW_HOUND);
    let searching = search.on(false);
    let view =
        crate::game::world::perception::View { eye: sight.eye, forward, far: sight.far, searching };
    let members = || party.members.iter_many(party.roster.iter()).matched();
    let seen = joined.iter().any(|&entity| {
        let (_, monster, at, walker, ..) = monsters.get(entity).unwrap();
        view.sees(
            members(),
            shadow_hound,
            monster,
            &catalog[monster.species],
            feet(at.translation(), walker),
            false,
            sight.obstacles,
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
        .map(|&m| (m, monsters.get(m).unwrap().2.translation()))
        .min_by(|a, b| a.1.distance(sight.eye).total_cmp(&b.1.distance(sight.eye)))
        .map(|(m, _)| m);
    let battle = commands
        .spawn((Name::new("Battle"), Battle { opening, ..default() }, LevelOwned(root)))
        .id();
    for &monster in joined {
        commands.entity(monster).insert(Combatant(battle));
    }
    recognition.detect(sight.eye, forward, Some(joined), shadow_hound, searching, &mut rng);
    news.write(CombatNews::Began { nearest });
    if let Some(cause) = opening {
        news.write(CombatNews::Surprised(cause));
    }
}
