//! A monster's accepted turn (retail `UpdateMonsterAI`,
//! `StartMonsterCombatAction`, `StepMonsterCombatAction`). As its round's
//! action begins it decides ([`decide`]): to run, withdraw or hold back
//! ([`conduct::decide`]), to breathe or cast, to guard a fellow, or to
//! fight; then its conditions may interrupt that ([`conduct::interrupt`]).
//! The decision is then carried out: a move away ([`retreat`]), a breath or
//! spell ([`carry_out`]), or blows ([`strike`]), where with nobody in reach
//! it advances (or charges when hurt) and acts again later in the round.

use super::*;
use hostility::Stance;

/// What a monster settled on doing with its action besides striking.
enum Plan<'a> {
    Spell {
        spell: crate::content::spells::SpellDefinition,
        power: u32,
        aim: monster_spells::SpellAim,
        scene: monster_spells::Scene,
        caster: (&'a Species, u32),
    },
    Special {
        kind: crate::content::species::SpecialAttackKind,
        cooldown: u8,
        /// Its maker's level and stamina as it decided, which its strength
        /// follows.
        breather: (u8, crate::game::characters::Pool),
        victims: Vec<Entity>,
    },
}

/// What a monster settled on as its round's action began.
enum Decision<'a> {
    /// Run, withdraw or hold back; `quiet` for a flight fear forced, which
    /// retail does not announce.
    Move {
        conduct: conduct::Conduct,
        quiet: bool,
    },
    Plan(Box<Plan<'a>>),
    /// Guard a fellow this round.
    Protect {
        ward: Entity,
    },
    /// Strike someone it may attack, or close in to; `waits` when it may
    /// hold back instead.
    Fight {
        waits: bool,
    },
}

/// How a monster of allegiance `mine`, held as it began to act, stands toward
/// someone in the fight: another monster by its own allegiance now, a member
/// by whether they have turned.
pub(super) fn stance_toward(
    mine: hostility::Allegiance,
    victim: Entity,
    members: &Members,
    damage: &MonsterDamage,
    now: u64,
    species: &MonsterCatalog,
) -> Stance {
    match members.get(victim) {
        Ok((_, vitals, ..)) => {
            hostility::toward_member(mine.stance, vitals.conditions.has(Condition::Turncoat))
        }
        Err(_) => hostility::between(mine, allegiance(victim, damage, now, species)),
    }
}

/// The monster's turn, from its decision to its blows. Returns whether it
/// acted.
pub(super) fn monster_turn(
    commands: &mut Commands,
    battle: &mut Battle,
    monster: Entity,
    combatants: &[Entity],
    rng: &mut GameRng,
    (members, damage): (&mut Members, &mut MonsterDamage),
    party: &PartyView,
    magic: &mut BattleMagic,
    species: &MonsterCatalog,
    (news, summons): (&mut MessageWriter<CombatNews>, &mut MessageWriter<summon::Summon>),
) -> bool {
    if !combatants.contains(&monster) || !damage.monsters().get(monster).unwrap().5.can_fight() {
        magic.monster_turns.get_mut(monster).unwrap().end();
        return false;
    }
    let mine = allegiance(monster, damage, party.now, species);
    let turn = *magic.monster_turns.get(monster).unwrap();
    let kind = &species[damage.monsters().get(monster).unwrap().0.species];
    // A monster decides as its round's action begins, not again between
    // its attacks.
    let (mut decision, interrupted) =
        if turn.attacks_left == u32::from(kind.attacks_per_round) && !turn.moved {
            decide(commands, monster, combatants, rng, (members, damage), party, magic, species, mine)
        } else {
            (Decision::Fight { waits: false }, None)
        };
    match interrupted {
        None | Some(Interrupt::Turncoat) => {}
        // Insanity turns it on its friends this round.
        Some(Interrupt::Berserk) => {
            magic.monster_turns.get_mut(monster).unwrap().berserk = true;
            decision = Decision::Fight { waits: false };
        }
        // Fear sends it running, without its flight's notice.
        Some(Interrupt::Panics) => {
            commands.entity(monster).insert(Fleeing);
            decision = Decision::Move { conduct: conduct::Conduct::Flee, quiet: true };
        }
        Some(interrupt) => {
            if interrupt == Interrupt::Freed {
                damage.vitals_mut(monster).conditions.lift(Condition::Webbed);
            }
            news.write(CombatNews::Interrupted { monster, interrupt });
            magic.monster_turns.get_mut(monster).unwrap().end();
            return true;
        }
    }
    match decision {
        Decision::Move { conduct, quiet } => {
            retreat(commands, monster, (conduct, quiet), damage, party, magic, species, news)
        }
        Decision::Protect { ward } => {
            magic.monster_turns.get_mut(monster).unwrap().end();
            commands.entity(monster).insert(Guarding(ward));
            true
        }
        Decision::Plan(plan) => {
            magic.monster_turns.get_mut(monster).unwrap().end();
            battle.attacked(true);
            carry_out(
                *plan,
                commands,
                monster,
                rng,
                (members, damage),
                party,
                magic,
                (news, summons),
            );
            true
        }
        // The blows are the attack systems' to make.
        Decision::Fight { waits } => {
            commands.entity(monster).insert(swings::Fights { waits });
            true
        }
    }
}

/// What the monster sets out to do as its round's action begins, and what
/// its conditions make of that.
fn decide<'a>(
    commands: &mut Commands,
    monster: Entity,
    combatants: &[Entity],
    rng: &mut GameRng,
    (members, damage): (&Members, &MonsterDamage),
    party: &PartyView,
    magic: &BattleMagic,
    species: &'a MonsterCatalog,
    mine: hostility::Allegiance,
) -> (Decision<'a>, Option<Interrupt>) {
    let toward = |victim: Entity| stance_toward(mine, victim, members, damage, party.now, species);
    let (body, at, walker, .., vitals) = damage.monsters().get(monster).unwrap();
    let kind = &species[body.species];
    let mut decision = Decision::Fight { waits: false };
    let feet = feet(at.translation(), walker);
    let nearest = party_distance(party.eye, body, feet);
    let was_fleeing = magic.fleeing.contains(monster);
    let mut fleeing = was_fleeing;
    let conduct = conduct::decide(rng, (body, vitals), kind, mine.stance, &mut fleeing, nearest);
    match (was_fleeing, fleeing) {
        (false, true) => {
            commands.entity(monster).insert(Fleeing);
        }
        (true, false) => {
            commands.entity(monster).remove::<Fleeing>();
        }
        _ => {}
    }
    let roster = &magic.context.roster;
    let sight = || {
        party.sight(
            members,
            roster,
            &magic.context.visibility,
            magic.context.enchantments.0.concealment(),
        )
    };
    // One that would close in on the party to strike does so rather
    // than breathe or cast; one that can strike does either at its
    // chance, and one that cannot always tries.
    let reaches = kind.attacks.iter().any(|a| nearest <= a.reach.metres());
    let closes_in = !kind.temper.prefers_range && !kind.attacks.is_empty() && !reaches;
    let special = kind.special;
    let cooling = magic.cooldowns.get(monster).is_ok_and(|cooldown| cooldown.0 > 0);
    if conduct != conduct::Conduct::Fight {
        decision = Decision::Move { conduct, quiet: false };
    } else if special.chance != 0
        && !closes_in
        && let Some(special_kind) = special.kind
        && special_ready(vitals, kind, cooling, Some((nearest, mine.stance)))
        && let Some(aim) = if special_kind.summons().is_some() {
            // A summoning aims at the summoner's own spot (retail
            // `AimFleeingMonster`); it needs no one in sight.
            Some(None)
        } else {
            magic
                .context
                .content
                .spells
                .get(crate::content::spells::SpellId::SPECIAL_ATTACK_CONE)
                .map(|cone| {
                    let scene = spell_scene(
                        (monster, kind),
                        (combatants, party_of(members, roster)),
                        damage.monsters(),
                        &sight(),
                        &toward,
                    );
                    let aims = monster_spells::aims(cone, &scene);
                    (cone, scene, aims)
                })
                .filter(|(_, _, aims)| !aims.is_empty())
                .map(Some)
        }
        && rng.below(100) < if reaches { u32::from(special.chance) } else { 100 }
    {
        // A breath at one of its aims at random (retail
        // `AimMonsterAtSpellTarget`), catching whoever stands in the cone.
        let victims = match aim {
            Some((cone, scene, aims)) => {
                let aim = aims[rng.below(aims.len() as u32) as usize];
                monster_spells::cone_victims(
                    &scene,
                    monster_spells::aimed_at(&scene, aim),
                    cone.reach.metres(),
                )
            }
            None => Vec::new(),
        };
        let breather = (kind.level, vitals.stamina);
        decision = Decision::Plan(Box::new(Plan::Special {
            kind: special_kind,
            cooldown: special.cooldown,
            breather,
            victims,
        }));
    } else if kind.magic.chance != 0 && !closes_in {
        let spells = &magic.context.content.spells;
        let chance = if reaches { u32::from(kind.magic.chance) } else { 100 };
        let scene = spell_scene(
            (monster, kind),
            (combatants, party_of(members, roster)),
            damage.monsters(),
            &sight(),
            &toward,
        );
        if let Some(plan) =
            monster_spells::choose(rng, vitals, kind, chance, spells, &scene).map(|(id, aim)| {
                let spell = spells[id].clone();
                let budget = body.spell_budget(kind);
                let power = monster_spells::power(rng, budget, &spell);
                Plan::Spell { spell, power, aim, scene, caster: (kind, budget) }
            })
        {
            decision = Decision::Plan(Box::new(plan));
        }
    }
    // One that can strike nobody, having no attack or, blind, nobody
    // in reach, withdraws (retail `UpdateMonsterAI`'s
    // `RETURN_TO_START`); a Monk fights blind.
    let blind = crate::game::world::population::blinded(kind, vitals);
    if matches!(decision, Decision::Fight { .. })
        && (kind.attacks.is_empty() || (blind && !reaches))
    {
        decision = Decision::Move { conduct: conduct::Conduct::Withdraw, quiet: false };
    }
    let near = |other: Entity, reach: f32| {
        let (o, o_at, o_walker, ..) = damage.monsters().get(other).unwrap();
        body.gap(feet, o, self::feet(o_at.translation(), o_walker)) <= reach
    };
    // One with someone in reach builds its choices (retail
    // `BuildMonsterActionQueue`): a bodyguard, as clever as it is, may
    // guard a stronger fellow in trouble; a wary one may hold back.
    if matches!(decision, Decision::Fight { .. }) && reaches {
        let intelligence = u32::from(kind.attributes.intelligence);
        let able = kind.temper.bodyguard && vitals.can_guard() && !kind.attacks.is_empty();
        let wards: Vec<Entity> = if able && rng.below(100) < intelligence {
            let reach = kind.attacks[0].reach.metres();
            combatants
                .iter()
                .copied()
                .filter(|&other| {
                    let (o, .., o_vitals) = damage.monsters().get(other).unwrap();
                    other != monster
                        && toward(other) == Stance::Friendly
                        && !o_vitals.is_dead()
                        && near(other, reach)
                        && species[o.species].level > kind.level
                        && o_vitals.health.current * 100 / o_vitals.health.max < 40
                })
                .collect()
        } else {
            Vec::new()
        };
        if !wards.is_empty() {
            let ward = wards[rng.below(wards.len() as u32) as usize];
            decision = Decision::Protect { ward };
        } else if kind.temper.wary && rng.below(100) < intelligence {
            decision = Decision::Fight { waits: true };
        }
    }
    // Its conditions, then, as the action begins.
    let against_foes = match &decision {
        Decision::Move { .. } | Decision::Protect { .. } => false,
        Decision::Plan(plan) => match &**plan {
            Plan::Spell { spell, .. } => spell.aimed_at_enemies(),
            Plan::Special { .. } => true,
        },
        Decision::Fight { .. } => true,
    };
    let strikes_friend = kind.attacks.iter().any(|attack| {
        let reach = attack.reach.metres();
        (nearest <= reach && roster.iter().any(|m| toward(m) == Stance::Friendly))
            || combatants.iter().any(|&other| {
                other != monster && toward(other) == Stance::Friendly && near(other, reach)
            })
    });
    let berserk = magic.monster_turns.get(monster).unwrap().berserk;
    let fleeing_now = matches!(decision, Decision::Move { conduct: conduct::Conduct::Flee, .. });
    let interrupted =
        conduct::interrupt(rng, vitals, kind, (fleeing_now, strikes_friend, berserk, against_foes));
    (decision, interrupted)
}

/// Run, withdraw or hold back: one that cannot walk, or has moved this
/// round, waits instead (retail falls back on `WAIT` when the move fails).
/// Returns whether it acted.
fn retreat(
    commands: &mut Commands,
    monster: Entity,
    (conduct, quiet): (conduct::Conduct, bool),
    damage: &MonsterDamage,
    party: &PartyView,
    magic: &mut BattleMagic,
    species: &MonsterCatalog,
    news: &mut MessageWriter<CombatNews>,
) -> bool {
    let mut work = magic.monster_turns.get_mut(monster).unwrap();
    work.end();
    if conduct == conduct::Conduct::Hold {
        return false;
    }
    let (body, at, walker, .., vitals) = damage.monsters().get(monster).unwrap();
    if !walker.is_some_and(|w| w.speed > 0.0) || work.moved {
        work.waited = true;
        return true;
    }
    work.moved = true;
    let metres = approach::move_metres(vitals, &species[body.species], false);
    let away = approach::away_from(feet(at.translation(), walker), party.eye, metres);
    commands.entity(monster).insert(approach::CombatMove {
        budget: metres,
        moved: 0.0,
        goal: approach::MoveGoal::Retreat(away),
    });
    if !quiet {
        news.write(match conduct {
            conduct::Conduct::Flee => CombatNews::Flees { monster },
            _ => CombatNews::Withdraws { monster },
        });
    }
    true
}

/// Cast the chosen spell, or make the special attack (retail
/// `ExecuteMonsterSpecialAttack`: it lands, the cooldown starts and it
/// costs a tenth of the monster's stamina).
fn carry_out(
    plan: Plan,
    commands: &mut Commands,
    monster: Entity,
    rng: &mut GameRng,
    (members, damage): (&mut Members, &mut MonsterDamage),
    party: &PartyView,
    magic: &mut BattleMagic,
    (news, summons): (&mut MessageWriter<CombatNews>, &mut MessageWriter<summon::Summon>),
) {
    let BattleMagic { context, monster_turns, .. } = magic;
    match plan {
        Plan::Spell { spell, power, aim, scene, caster } => {
            let flights = context.resolve(party.now, members, damage, |field| {
                monster_spells::cast(commands, (&spell, power, aim), caster, &scene, rng, field)
            });
            let level = context.level();
            context.flight_order.launch(commands, flights, level);
        }
        Plan::Special { kind, cooldown, breather, victims } => {
            monster_turns.get_mut(monster).unwrap().special = true;
            news.write(CombatNews::Special { monster, kind });
            // A summoning calls its group up rather than landing on anyone.
            if let Some(species) = kind.summons() {
                summons.write(summon::Summon { summoner: monster, species });
            }
            let deed =
                commands.spawn(crate::game::hits::Deed::new(crate::game::hits::Act::Fight)).id();
            let definition = crate::game::hits::specials::special(kind);
            crate::game::hits::specials::breathe(commands, deed, definition, breather, &victims);
            commands.entity(monster).insert(SpecialCooldown(cooldown));
            damage.vitals_mut(monster).tire(breather.1.max / 10);
        }
    }
}
