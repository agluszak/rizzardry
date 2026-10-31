//! A monster's accepted turn (retail `UpdateMonsterAI`,
//! `StartMonsterCombatAction`, `StepMonsterCombatAction`). As its round's
//! action begins it decides ([`decide`]): to run, withdraw or hold back
//! ([`conduct::decide`]), to breathe or cast, to guard a fellow, or to
//! fight; then its conditions may interrupt that ([`conduct::interrupt`]).
//! The decision is then carried out: a move away ([`retreat`]), a breath or
//! spell ([`carry_out`]), or blows ([`strike`]), where with nobody in reach
//! it advances (or charges when hurt) and acts again later in the round.

use super::*;
use crate::game::magic::effects::Victim;
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
        breather: crate::game::magic::effects::Breather,
        victims: Vec<Victim>,
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

/// How the acting monster stands toward everyone in the fight: each
/// monster by its own allegiance, each member by whether they have turned.
struct Sides {
    mine: hostility::Allegiance,
    allegiances: Vec<(Entity, hostility::Allegiance)>,
    turncoats: Vec<Entity>,
}

impl Sides {
    fn of(
        monster: Entity,
        combatants: &[Entity],
        (members, damage): (&Members, &MonsterDamage),
        roster: &PartyRoster,
        now: u64,
        species: &MonsterCatalog,
    ) -> Self {
        let allegiances: Vec<_> = combatants
            .iter()
            .map(|&entity| (entity, allegiance(entity, damage, now, species)))
            .collect();
        let mine = allegiances.iter().find(|&&(e, _)| e == monster).unwrap().1;
        let turncoats = roster
            .iter()
            .filter(|&m| members.get(m).unwrap().1.conditions.has(Condition::Turncoat))
            .collect();
        Self { mine, allegiances, turncoats }
    }

    fn toward(&self, victim: Victim) -> Stance {
        match victim {
            Victim::Member(member) => {
                hostility::toward_member(self.mine.stance, self.turncoats.contains(&member))
            }
            Victim::Monster(other) => self
                .allegiances
                .iter()
                .find(|&&(e, _)| e == other)
                .map_or(Stance::Neutral, |&(_, theirs)| hostility::between(self.mine, theirs)),
        }
    }
}

/// The monster's turn, from its decision to its blows. Returns whether it
/// acted.
pub(super) fn monster_turn(
    commands: &mut Commands,
    battle: &mut Battle,
    (index, step): (usize, u8),
    combatants: &[Entity],
    rng: &mut GameRng,
    (members, damage): (&mut Members, &mut MonsterDamage),
    party: &PartyView,
    magic: &mut BattleMagic,
    species: &MonsterCatalog,
    (news, summons): (&mut MessageWriter<CombatNews>, &mut MessageWriter<summon::Summon>),
) -> bool {
    let monster = battle.this_round.monsters[index].actor;
    if !combatants.contains(&monster) || !damage.monsters().get(monster).unwrap().0.can_fight() {
        battle.this_round.monsters[index].end();
        return false;
    }
    let sides = Sides::of(
        monster,
        combatants,
        (members, damage),
        &magic.context.roster,
        party.now,
        species,
    );
    let turn = battle.this_round.monsters[index];
    let kind = &species[damage.monsters().get(monster).unwrap().0.species];
    // A monster decides as its round's action begins, not again between
    // its attacks.
    let (mut decision, interrupted) =
        if turn.attacks_left == u32::from(kind.attacks_per_round) && !turn.moved {
            decide(battle, index, combatants, rng, (members, damage), party, magic, species, &sides)
        } else {
            (Decision::Fight { waits: false }, None)
        };
    match interrupted {
        None | Some(Interrupt::Turncoat) => {}
        // Insanity turns it on its friends this round.
        Some(Interrupt::Berserk) => {
            battle.this_round.monsters[index].berserk = true;
            decision = Decision::Fight { waits: false };
        }
        // Fear sends it running, without its flight's notice.
        Some(Interrupt::Panics) => {
            if !battle.fleeing.contains(&monster) {
                battle.fleeing.push(monster);
            }
            decision = Decision::Move { conduct: conduct::Conduct::Flee, quiet: true };
        }
        Some(interrupt) => {
            if interrupt == Interrupt::Freed {
                damage.conditions_mut(monster).lift(Condition::Webbed);
            }
            news.write(CombatNews::Interrupted { monster, interrupt });
            battle.this_round.monsters[index].end();
            return true;
        }
    }
    match decision {
        Decision::Move { conduct, quiet } => {
            retreat(commands, battle, index, (conduct, quiet), damage, party, species, news)
        }
        Decision::Protect { ward } => {
            battle.this_round.monsters[index].end();
            battle.this_round.guards.push(Guard { guard: monster, ward, interceptions: 0 });
            true
        }
        Decision::Plan(plan) => {
            battle.this_round.monsters[index].end();
            battle.this_round.attacked = true;
            carry_out(
                *plan,
                commands,
                battle,
                index,
                rng,
                (members, damage),
                party,
                magic,
                (news, summons),
            );
            true
        }
        Decision::Fight { waits } => {
            strike(
                commands,
                battle,
                (index, step, turn.attacks_left),
                combatants,
                rng,
                (members, damage),
                party,
                magic,
                species,
                (&sides, waits),
                news,
            );
            true
        }
    }
}

/// What the monster sets out to do as its round's action begins, and what
/// its conditions make of that.
fn decide<'a>(
    battle: &mut Battle,
    index: usize,
    combatants: &[Entity],
    rng: &mut GameRng,
    (members, damage): (&Members, &MonsterDamage),
    party: &PartyView,
    magic: &BattleMagic,
    species: &'a MonsterCatalog,
    sides: &Sides,
) -> (Decision<'a>, Option<Interrupt>) {
    let monster = battle.this_round.monsters[index].actor;
    let toward = |victim: Victim| sides.toward(victim);
    let (body, at, walker, ..) = damage.monsters().get(monster).unwrap();
    let kind = &species[body.species];
    let mut decision = Decision::Fight { waits: false };
    let feet = feet(at.translation(), walker);
    let nearest = party_distance(party.eye, body, feet);
    let mut fleeing = battle.fleeing.contains(&monster);
    let conduct = conduct::decide(rng, body, kind, sides.mine.stance, &mut fleeing, nearest);
    battle.fleeing.retain(|&m| m != monster);
    if fleeing {
        battle.fleeing.push(monster);
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
    let cooling = battle.cooldowns.iter().any(|&(m, left)| m == monster && left > 0);
    if conduct != conduct::Conduct::Fight {
        decision = Decision::Move { conduct, quiet: false };
    } else if special.chance != 0
        && !closes_in
        && let Some(special_kind) = special.kind
        && special_ready(body, kind, cooling, Some((nearest, sides.mine.stance)))
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
        let breather = crate::game::magic::effects::Breather {
            level: kind.level,
            stamina: body.stamina,
            max_stamina: body.max_stamina,
        };
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
            monster_spells::choose(rng, body, kind, chance, spells, &scene).map(|(id, aim)| {
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
    let blind = body.blinded(kind);
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
        let able = kind.temper.bodyguard && body.can_guard() && !kind.attacks.is_empty();
        let wards: Vec<Entity> = if able && rng.below(100) < intelligence {
            let reach = kind.attacks[0].reach.metres();
            combatants
                .iter()
                .copied()
                .filter(|&other| {
                    let (o, ..) = damage.monsters().get(other).unwrap();
                    other != monster
                        && toward(Victim::Monster(other)) == Stance::Friendly
                        && !o.is_dead()
                        && near(other, reach)
                        && species[o.species].level > kind.level
                        && o.hp * 100 / o.max_hp.max(1) < 40
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
        (nearest <= reach && roster.iter().any(|m| toward(Victim::Member(m)) == Stance::Friendly))
            || combatants.iter().any(|&other| {
                other != monster
                    && toward(Victim::Monster(other)) == Stance::Friendly
                    && near(other, reach)
            })
    });
    let berserk = battle.this_round.monsters[index].berserk;
    let fleeing_now = matches!(decision, Decision::Move { conduct: conduct::Conduct::Flee, .. });
    let interrupted =
        conduct::interrupt(rng, body, kind, (fleeing_now, strikes_friend, berserk, against_foes));
    (decision, interrupted)
}

/// Run, withdraw or hold back: one that cannot walk, or has moved this
/// round, waits instead (retail falls back on `WAIT` when the move fails).
/// Returns whether it acted.
fn retreat(
    commands: &mut Commands,
    battle: &mut Battle,
    index: usize,
    (conduct, quiet): (conduct::Conduct, bool),
    damage: &MonsterDamage,
    party: &PartyView,
    species: &MonsterCatalog,
    news: &mut MessageWriter<CombatNews>,
) -> bool {
    let work = &mut battle.this_round.monsters[index];
    work.end();
    let monster = work.actor;
    if conduct == conduct::Conduct::Hold {
        return false;
    }
    let (body, at, walker, ..) = damage.monsters().get(monster).unwrap();
    if !walker.is_some_and(|w| w.speed > 0.0) || work.moved {
        work.waited = true;
        return true;
    }
    work.moved = true;
    let metres = approach::move_metres(body, &species[body.species], false);
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
    battle: &mut Battle,
    index: usize,
    rng: &mut GameRng,
    (members, damage): (&mut Members, &mut MonsterDamage),
    party: &PartyView,
    magic: &mut BattleMagic,
    (news, summons): (&mut MessageWriter<CombatNews>, &mut MessageWriter<summon::Summon>),
) {
    let monster = battle.this_round.monsters[index].actor;
    let BattleMagic { context, points, .. } = magic;
    match plan {
        Plan::Spell { spell, power, aim, scene, caster } => {
            let (report, flights) = context.resolve(party.now, members, damage, |field| {
                monster_spells::cast((&spell, power, aim), caster, &scene, rng, field, points)
            });
            battle.credit_kills(&report.effects, news);
            let level = context.level();
            context.flight_order.launch(commands, flights, level);
            context.news.write(crate::game::magic::casting::MagicNews::Done(report));
        }
        Plan::Special { kind, cooldown, breather, victims } => {
            battle.this_round.monsters[index].special = true;
            news.write(CombatNews::Special { monster, kind });
            // A summoning calls its group up rather than landing on anyone.
            if let Some(species) = kind.summons() {
                summons.write(summon::Summon { summoner: monster, species });
            }
            let effects = context.resolve(party.now, members, damage, |field| {
                crate::game::magic::effects::special_attack(
                    crate::game::magic::effects::special(kind),
                    breather,
                    &victims,
                    rng,
                    field,
                    points,
                )
            });
            if !effects.is_empty() {
                news.write(CombatNews::Effects(effects));
            }
            battle.cooldowns.retain(|&(m, _)| m != monster);
            battle.cooldowns.push((monster, cooldown));
            damage.fatigue(monster, breather.max_stamina / 10);
        }
    }
}

/// Strike someone it may attack, a berserk monster its friends and any
/// other its foes; with nobody in reach, close in on a foe (a friendly
/// monster on the nearest monster it may attack, a hostile one on the
/// party) and act again later in the round.
fn strike(
    commands: &mut Commands,
    battle: &mut Battle,
    (index, step, attacks_left): (usize, u8, u32),
    combatants: &[Entity],
    rng: &mut GameRng,
    (members, damage): (&mut Members, &mut MonsterDamage),
    party: &PartyView,
    magic: &mut BattleMagic,
    species: &MonsterCatalog,
    (sides, waits): (&Sides, bool),
    news: &mut MessageWriter<CombatNews>,
) {
    let monster = battle.this_round.monsters[index].actor;
    let BattleMagic { context, points, moving, .. } = magic;
    let wanted =
        if battle.this_round.monsters[index].berserk { Stance::Friendly } else { Stance::Hostile };
    let may_attack = |victim: Victim| sides.toward(victim) == wanted;
    let mut fight = blows::Fight {
        combatants,
        party,
        armours: party_armours(battle, members, context),
        moving,
        points,
        attacks: &may_attack,
        waits,
    };
    let idle = context.resolve(party.now, members, damage, |field| {
        blows::monster_attacks(battle, index, rng, field, &mut fight, news)
    });
    if !idle || battle.this_round.monsters[index].moved {
        return;
    }
    let (body, here, walker, ..) = damage.monsters().get(monster).unwrap();
    let budget = if sides.mine.stance == Stance::Friendly {
        let target = combatants
            .iter()
            .filter_map(|&entity| {
                let (other, at, ..) = damage.monsters().get(entity).unwrap();
                (entity != monster && !other.is_dead() && may_attack(Victim::Monster(entity)))
                    .then_some((here.translation().distance_squared(at.translation()), entity))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, entity)| entity);
        target.filter(|_| walker.is_some_and(|walker| walker.speed > 0.0)).map(|target| {
            let charge = body.charges();
            let metres = approach::move_metres(body, &species[body.species], charge);
            (approach::MoveBudget { metres, charge }, approach::MoveGoal::Attack(target))
        })
    } else if sides.mine.stance == Stance::Hostile {
        approach::move_budget(body, here.translation(), walker, party.eye, species)
            .map(|budget| (budget, approach::MoveGoal::Party))
    } else {
        None
    };
    if let Some((budget, goal)) = budget {
        let work = &mut battle.this_round.monsters[index];
        work.moved = true;
        work.phase = (step * 2).min(100);
        work.attacks_left = attacks_left;
        commands.entity(monster).insert(approach::CombatMove {
            budget: budget.metres,
            moved: 0.0,
            goal,
        });
        news.write(CombatNews::Moves { monster, charge: budget.charge });
    }
}
