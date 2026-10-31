//! A round's end: who leaves the fight, the round's time and recovery,
//! and whether the fight is over.

use super::*;

/// Groups with nothing left to fight leave it as the round ends (retail
/// `CheckMonsterGroupsLeaveCombat`): a neutral group once it no longer
/// stands with the fight ([`reinforces`]); a friendly group while one of
/// its members sees an enemy; a hostile group while a living member sees
/// the party. When no hostile
/// group is left engaged, everyone leaves. Returns the combatants that
/// stay; one outside any group stays while hostile groups fight on.
pub(super) fn leave_fight(
    commands: &mut Commands,
    combatants: &[Entity],
    (damage, now): (&MonsterDamage, u64),
    membership: &Query<&GroupMember>,
    sight: &Sight,
    species: &MonsterCatalog,
) -> Vec<Entity> {
    use hostility::Stance;
    let monsters = damage.monsters();
    let group_of = |monster: Entity| membership.get(monster).ok().map(|m| m.0);
    let stance_of = |monster: Entity| Some(damage.group_stance(monster, now));
    let seen = |monster: Entity| -> Option<(Seen, bool)> {
        let (body, at, walker, ..) = monsters.get(monster).ok()?;
        let kind = &species[body.species];
        if body.is_dead() {
            return None;
        }
        Some((sight.seen(monster, (body, &at, walker, kind)), kind.temper.camouflaged))
    };
    let hostiles: Vec<Seen> = combatants
        .iter()
        .filter(|&&c| stance_of(c) == Some(Stance::Hostile))
        .filter_map(|&c| seen(c).map(|(s, _)| s))
        .filter(|s| s.in_view)
        .collect();
    let (mut checked, mut leaving, mut engaged) = (Vec::new(), Vec::new(), false);
    for &combatant in combatants {
        let Some(group) = group_of(combatant) else {
            engaged |= stance_of(combatant) == Some(Stance::Hostile);
            continue;
        };
        if checked.contains(&group) {
            continue;
        }
        checked.push(group);
        let fellows: Vec<(Seen, bool)> = combatants
            .iter()
            .filter(|&&c| group_of(c) == Some(group))
            .filter_map(|&c| seen(c))
            .collect();
        let stays = match stance_of(combatant) {
            Some(Stance::Neutral) => reinforces(&fellows, &hostiles),
            Some(Stance::Hostile) => {
                let sees = fellows.iter().any(|(s, _)| s.sees);
                engaged |= sees;
                sees
            }
            Some(Stance::Friendly) => fellows.iter().any(|(fellow, _)| {
                let mine = allegiance(fellow.entity, damage, now, species);
                let (body, at, walker, ..) = monsters.get(fellow.entity).unwrap();
                let kind = &species[body.species];
                combatants.iter().any(|&enemy| {
                    let theirs = allegiance(enemy, damage, now, species);
                    let (target, target_at, target_walker, ..) = monsters.get(enemy).unwrap();
                    let target_kind = &species[target.species];
                    !target.is_dead()
                        && hostility::between(mine, theirs) == Stance::Hostile
                        && sight.sees_monster(
                            (body, &at, walker, kind),
                            (target, &target_at, target_walker, target_kind),
                        )
                })
            }),
            _ => false,
        };
        if !stays {
            leaving.push(group);
        }
    }
    combatants
        .iter()
        .copied()
        .filter(|&combatant| {
            let leaves = !engaged || group_of(combatant).is_some_and(|g| leaving.contains(&g));
            if leaves {
                commands.entity(combatant).remove::<Combatant>();
            }
            !leaves
        })
        .collect()
}

/// Once everyone has acted, the round ends (retail `EndCombatRound`): groups
/// with nothing left to fight leave ([`leave_fight`]), the round's recovery
/// and its count of who could strike settle ([`end_round`]), the party looks
/// again at who it fights, and a fight with nobody left is over.
pub fn finish_round(
    mut commands: Commands,
    battle: Option<Single<(Entity, &mut Battle, Option<&Combatants>)>>,
    mut rng: ResMut<GameRng>,
    mut clock: ResMut<GameClock>,
    mut actors: ParamSet<((Members, MonsterDamage), GroupRecognition)>,
    party_body: Query<(&Position, &PartyBody)>,
    (roster, content, level): (Single<&PartyRoster>, If<Res<Catalogs>>, PlayedLevel),
    mut magic: BattleMagic,
    mut news: MessageWriter<CombatNews>,
) {
    let Some((entity, mut battle, combatants)) = battle.map(Single::into_inner) else { return };
    let RoundPhase::Executing(step) = battle.phase else { return };
    if step <= 100
        || matches!(battle.party_move, PartyMove::Moving { .. })
        || !battle.wait.is_finished()
        || magic.busy()
    {
        return;
    }
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    let species = &content.species;
    let Ok((position, body)) = party_body.single() else { return };
    let roster = &**roster;
    let party = PartyView::new((position, body), &level, clock.ms);
    let (mut members, mut damage) = actors.p0();
    let sight = party.sight(
        &members,
        roster,
        &magic.context.visibility,
        magic.context.enchantments.0.concealment(),
    );
    let staying = leave_fight(
        &mut commands,
        combatants,
        (&damage, party.now),
        &magic.groups,
        &sight,
        species,
    );
    let over = end_round(
        (entity, &mut battle),
        combatants,
        &mut clock,
        &mut rng,
        roster,
        (&mut members, &mut damage),
        party.eye,
        species,
        &mut news,
        &mut magic,
        &staying,
    );
    if party_fallen(roster, &members) {
        return;
    }
    if over {
        for &actor in combatants {
            damage.enchantments_mut(actor).end_combat();
        }
    }
    // Recognition is a required part of this round, before the next
    // accepted action or the battle's end can change its meaning.
    actors.p1().detect(
        party.eye,
        body.forward(),
        Some(combatants),
        magic.context.enchantments.0.get(crate::content::spells::SpellId::SHADOW_HOUND).is_some(),
        false,
        &mut rng,
    );
    if over {
        magic.context.enchantments.0.end_combat();
        commands.entity(entity).despawn();
    }
}

/// Whether nobody in the party is left alive.
pub(super) fn party_fallen(roster: &PartyRoster, members: &Members) -> bool {
    !crate::game::characters::anyone_alive(
        members.iter_many(roster.iter()).matched().map(|(_, vitals, ..)| vitals),
    )
}

/// The round is over: a round's game time passes and the defenders rest.
/// Returns whether the fight is over: no hostile is left or nobody could
/// attack for two rounds.
pub(super) fn end_round(
    (fight, battle): (Entity, &mut Battle),
    combatants: &[Entity],
    clock: &mut GameClock,
    rng: &mut GameRng,
    roster: &PartyRoster,
    (members, damage): (&mut Members, &mut MonsterDamage),
    party_eye: Vec3,
    species: &MonsterCatalog,
    news: &mut MessageWriter<CombatNews>,
    magic: &mut BattleMagic,
    staying: &[Entity],
) -> bool {
    pass_round(
        (fight, battle),
        combatants,
        clock,
        rng,
        roster,
        (members, damage),
        party_eye,
        news,
        magic,
    );
    // Poison may have killed the last of the party as the time passed: that
    // is their defeat, which `settle_defeat` reports, not a victory.
    if party_fallen(roster, members) {
        return false;
    }
    rest_defenders(battle, roster, members, rng);
    battle.wait = Timer::from_seconds(ROUND_SECONDS, TimerMode::Once);
    if !fight_over(battle, combatants, staying, damage, (clock.ms, party_eye), species) {
        return false;
    }
    award_victory(battle, roster, members, news);
    true
}

/// A round's game time passes, with its meaning: conditions age, spell
/// points come back at half rate and nothing else recovers (retail
/// `AdvanceCombatRound`, `AdvanceTimedEffects`). A round takes one aging
/// tick.
pub(super) fn pass_round(
    (fight, battle): (Entity, &mut Battle),
    combatants: &[Entity],
    clock: &mut GameClock,
    rng: &mut GameRng,
    roster: &PartyRoster,
    (members, damage): (&mut Members, &mut MonsterDamage),
    party_eye: Vec3,
    news: &mut MessageWriter<CombatNews>,
    magic: &mut BattleMagic,
) {
    let ticks = clock.advance(crate::game::time::AGING_TICK_MS);
    // A monster that waited, or lies asleep or knocked out, gets its
    // breath back (retail `AdvanceCombatRound`).
    for &entity in combatants {
        let (monster, ..) = damage.monsters().get(entity).unwrap();
        let resting =
            battle.this_round.monsters.iter().any(|turn| turn.actor == entity && turn.waited)
                || monster
                    .conditions
                    .iter()
                    .any(|&(c, _)| matches!(c, Condition::Asleep | Condition::Unconscious));
        if resting {
            let maximum = monster.max_stamina;
            damage.restore_stamina(entity, recovery::rest_roll(maximum, rng));
        }
    }
    // A special attack cools down over the rounds the monster does not
    // make one (retail `AdvanceCombatRound`).
    for (monster, left) in &mut battle.cooldowns {
        if *left > 0
            && !battle.this_round.monsters.iter().any(|turn| turn.actor == *monster && turn.special)
        {
            *left -= 1;
        }
    }
    for &entity in combatants {
        let mut conditions = damage.conditions_mut(entity);
        if conditions.any_timed() {
            conditions.age(ticks.aging);
        }
    }
    for actor in roster.iter() {
        let (mut profile, mut vitals, ..) = members.get_mut(actor).unwrap();
        let (known, mut pool) = magic.points.get_mut(actor).unwrap();
        recovery::pass(
            &mut profile,
            &mut vitals,
            ticks,
            recovery::Pace::Fighting,
            (known, &mut pool),
            &magic.context.content.spells,
            &mut crate::game::characters::Fate { fight: Some(fight), rng },
        );
    }
    // AdvanceTimedEffects applies offensive slots after personal aging and
    // before expiring the slots. Their final remaining turn still pulses.
    let reports = magic.context.resolve(clock.ms, members, damage, |field| {
        crate::game::magic::effects::pulse_clouds(
            combatants,
            party_eye,
            rng,
            field,
            &mut magic.points,
        )
    });
    for (spell, effects) in reports {
        battle.credit_kills(&effects, news);
        battle.this_round.attacked |= crate::game::magic::effects::affected(&effects);
        magic.context.news.write(casting::MagicNews::Triggered {
            spell,
            effects,
            improved: vec![],
        });
    }
    magic.context.enchantments.0.age(ticks.aging);
    for &entity in combatants {
        damage.enchantments_mut(entity).age(ticks.aging);
    }
    battle.phase = RoundPhase::Waiting;
}

/// Members who defended, or lie asleep or knocked out, get their breath
/// back after their conditions age, in accepted actor order.
pub(super) fn rest_defenders(
    battle: &Battle,
    roster: &PartyRoster,
    members: &mut Members,
    rng: &mut GameRng,
) {
    for work in &battle.this_round.party {
        let member = work.actor;
        if !roster.contains(&member) {
            continue;
        }
        let (character, mut vitals, ..) = members.get_mut(member).unwrap();
        let resting = battle.defends(member)
            || matches!(
                vitals.conditions.worst(),
                Some(Condition::Asleep | Condition::Unconscious)
            );
        if vitals.is_dead() || !resting {
            continue;
        }
        let maximum = character.stats(&vitals.enchantments).max_stamina();
        let gained = recovery::combat_rest_amount(&character, recovery::rest_roll(maximum, rng));
        if vitals.stamina < maximum || vitals.conditions.exhausted() {
            vitals.restore_stamina(gained, maximum);
        }
    }
}

/// Whether the fight is over: no hostile stays in it, or nobody attacked
/// or closed in for two rounds.
pub(super) fn fight_over(
    battle: &mut Battle,
    combatants: &[Entity],
    staying: &[Entity],
    damage: &MonsterDamage,
    (now, party_eye): (u64, Vec3),
    species: &MonsterCatalog,
) -> bool {
    let monsters = damage.monsters();
    let approaching = combatants.iter().any(|&entity| {
        let (monster, at, walker, ..) = monsters.get(entity).unwrap();
        approach::needs_approach(monster, at.translation(), walker, party_eye, species)
    });
    let active = battle.this_round.attacked || approaching;
    battle.passive_rounds = if active { 0 } else { battle.passive_rounds + 1 };
    let hostiles_left = staying.iter().any(|&e| {
        damage.stance(e, now) == hostility::Stance::Hostile && !monsters.get(e).unwrap().0.is_dead()
    });
    !hostiles_left || battle.passive_rounds >= PASSIVE_ROUNDS
}

/// Experience is shared among the living, the unconscious included.
pub(super) fn award_victory(
    battle: &Battle,
    roster: &PartyRoster,
    members: &mut Members,
    news: &mut MessageWriter<CombatNews>,
) {
    let living =
        members.iter_many(roster.iter()).matched().filter(|(_, v, ..)| !v.is_dead()).count() as u32;
    let each = battle.experience / living.max(1);
    if each > 0 {
        for (mut character, vitals, ..) in members.iter_many_unique_mut(roster.iter()).matched() {
            if !vitals.is_dead() {
                character.award_experience(each);
            }
        }
    }
    news.write(if battle.kills > 0 {
        CombatNews::Won { kills: battle.kills, experience_each: each }
    } else {
        CombatNews::Over
    });
}
