//! A monster's attack: whom it may reach, its announcement and its swings
//! (retail `AnnounceMonsterAttack`, `ContinueMonsterAttack`).
use super::*;
use crate::game::rng::GameRng;

/// Who a monster's swing could go astray onto (retail
/// `BuildMonsterTargetList`): the members its attack reaches but the one
/// aimed at, itself, and the monsters within its reach.
pub(super) fn monster_bystanders(
    field: &Field,
    fight: &Fight,
    attacker: Entity,
    reach: Reach,
    target: Entity,
) -> Vec<Victim> {
    let monsters = field.monsters.monsters();
    let (monster, at, walker, ..) = monsters.get(attacker).unwrap();
    let here = feet(at.translation(), walker);
    let distance = party_distance(fight.party.eye, monster, here);
    let formation = &**field.formation;
    let mut bystanders: Vec<Victim> = field
        .party
        .iter()
        .filter(|&m| m != target)
        .filter(|&m| field.members.get(m).is_ok_and(|(_, v, ..)| !v.is_dead()))
        .filter(|&m| fight.party.reaches(formation, m, at.translation(), reach, distance))
        .map(Victim::Member)
        .collect();
    for &entity in fight.combatants {
        let (other, other_at, other_walker, ..) = monsters.get(entity).unwrap();
        if entity == target || other.is_dead() || !field.monsters.active(entity) {
            continue;
        }
        let gap = monster.gap(here, other, feet(other_at.translation(), other_walker));
        if entity != target && (entity == attacker || gap <= reach.metres()) {
            bystanders.push(Victim::Monster(entity));
        }
    }
    bystanders
}

/// Whether a member notices a monster coming at them in time to turn
/// (retail `CharacterNoticesAttacker`): only one awake, unblinded and
/// defending; the first time each round always, then by senses, 25 harder
/// each time.
pub(super) fn notices(
    battle: &mut Battle,
    member: Entity,
    character: &Character,
    vitals: &Vitals,
    rng: &mut GameRng,
) -> bool {
    let alert = !vitals.is_dead()
        && vitals.stamina > 0
        && vitals.conditions.worst().is_none_or(|c| c < Condition::Webbed)
        && !vitals.conditions.has(Condition::Blind);
    let defending = battle
        .this_round
        .party
        .iter()
        .any(|p| p.actor == member && matches!(p.action, RoundAction::Defend));
    if !alert || !defending {
        return false;
    }
    let attempts = battle.this_round.spotted.entry(member).or_insert(0);
    if *attempts == 0 {
        *attempts = 1;
        return true;
    }
    // Retail compares unsigned: once senses fall short, noticing is certain.
    let senses = character
        .stats(&vitals.enchantments)
        .attribute(crate::content::characters::Attribute::Senses) as i32;
    let chance = (senses - *attempts as i32 * 25) as u32;
    *attempts += 1;
    rng.below(100) < chance
}

/// Every (attack, mode, target) the monster reaches and may attack, equally
/// likely: members, then the other monsters in the fight. Those it may attack
/// are whom it stands `wanted` toward, from allegiance `mine` (retail
/// `BuildMonsterActionQueue`'s `MonsterVsCharDisposition` and
/// `MonsterHostility` tests). A member's Stealth may keep it out of the
/// monster's eye this time (three quarters of it, in percent), and one that
/// stayed out of it may learn from it.
pub(super) fn reachable_targets(
    entity: Entity,
    kind: &Species,
    field: &mut Field,
    fight: &Fight,
    (mine, wanted): (hostility::Allegiance, Stance),
    news: &mut MessageWriter<CombatNews>,
) -> Vec<(usize, AttackMode, Victim)> {
    let (now, species) = (field.now(), field.species());
    let attacks = |victim: Victim, field: &Field| {
        crate::game::combat::monster_turn::stance_toward(
            mine,
            victim,
            &field.members,
            &field.monsters,
            now,
            species,
        ) == wanted
    };
    let (monster, at, walker, ..) = field.monsters.monsters().get(entity).unwrap();
    let here = feet(at.translation(), walker);
    let distance = party_distance(fight.party.eye, monster, here);
    let eye = monster.eye(here);
    let formation = &**field.formation;
    let mut unseen = Vec::new();
    for member in field.party.iter() {
        let stealth = field.members.get(member).unwrap().0.skill(Skill::Stealth);
        if field.rng.below(100) < stealth * 75 / 100 {
            unseen.push(member);
        }
    }
    let mut candidates = vec![];
    let mut evaded = Vec::new();
    for (a, attack) in kind.attacks.iter().enumerate() {
        for member in field.party.iter() {
            let (_, vitals, ..) = field.members.get(member).unwrap();
            if !vitals.is_dead()
                && attacks(Victim::Member(member), field)
                && fight.party.reaches(formation, member, at.translation(), attack.reach, distance)
            {
                if unseen.contains(&member) {
                    if !evaded.contains(&member) {
                        evaded.push(member);
                    }
                    continue;
                }
                for mode in attack.modes {
                    candidates.push((a, mode, Victim::Member(member)));
                }
            }
        }
        for &other in fight.combatants {
            let (body, other_at, other_walker, ..) = field.monsters.monsters().get(other).unwrap();
            let there = feet(other_at.translation(), other_walker);
            if other != entity
                && !body.is_dead()
                && field.monsters.active(other)
                && field.visibility.clear(eye, body_centre(there, body))
                && attacks(Victim::Monster(other), field)
                && monster.gap(here, body, there) <= attack.reach.metres()
            {
                for mode in attack.modes {
                    candidates.push((a, mode, Victim::Monster(other)));
                }
            }
        }
    }
    for member in evaded {
        if field.rng.below(2) == 0
            && field.members.get_mut(member).unwrap().0.practise(Skill::Stealth, 1, &mut field.rng)
        {
            news.write(CombatNews::Improved { member, skill: Skill::Stealth });
        }
    }
    candidates
}

/// A monster's attack (retail `StartMonsterAttack` and its swings): one of
/// its attacks and modes at a member or a fellow monster it may attack and
/// reaches, all equally likely, its swings each rolled to hit, to penetrate
/// and for damage and effects. Returns whether it had nobody to attack.
pub(in crate::game::combat) fn monster_attacks(
    battle: &mut Battle,
    index: usize,
    field: &mut Field,
    fight: &mut Fight,
    attackers_side: (hostility::Allegiance, Stance),
    news: &mut MessageWriter<CombatNews>,
) -> bool {
    let turn = battle.this_round.monsters[index];
    let entity = turn.actor;
    let species = field.species();
    let difficulty = field.difficulty();
    let (monster, at, walker, ..) = field.monsters.monsters().get(entity).unwrap();
    let kind = &species[monster.species];
    let position = at.translation();
    let eye = monster.eye(feet(position, walker));
    let stance = field.monsters.stance(entity, field.now());
    let candidates = reachable_targets(entity, kind, field, fight, attackers_side, news);
    let turn_left = turn.attacks_left.max(1) - 1;
    battle.this_round.monsters[index].end();
    // A wary monster's wait comes first among its choices.
    let waits = usize::from(fight.waits);
    if candidates.len() + waits == 0 {
        return true;
    }
    let chosen = field.rng.below((candidates.len() + waits) as u32) as usize;
    if chosen < waits {
        battle.this_round.monsters[index].waited = true;
        return false;
    }
    battle.this_round.attacked = true;
    let (a, mode, target) = candidates[chosen - waits];
    let attack = &kind.attacks[a];
    // Retail draws for the mode again, from just the one chosen.
    let mode = attack::choose_mode(&mut field.rng, enumset::EnumSet::only(mode));
    let swings = field.rng.below(u32::from(kind.swings_per_round.max(1))) + 1;
    // A natural ambusher strikes from hiding at arm's length.
    let mut surprise = Surprise {
        unaware: false,
        natural: kind.kind == crate::content::species::MonsterKind::Rogue
            && attack.reach <= Reach::Touch,
    };
    // A member not facing it may turn in time, or be struck from behind.
    if attack.reach <= Reach::Short
        && let Victim::Member(member) = target
    {
        let (character, vitals, ..) = field.members.get(member).unwrap();
        let side = formation::quadrant(fight.party.eye, fight.party.yaw, position);
        let facing = field.formation.facing(member);
        if facing.is_some_and(|f| f != side) {
            if notices(battle, member, character, vitals, &mut field.rng) {
                field.formation.turn(member, side);
            } else {
                surprise.unaware = facing.is_some_and(|f| f.opposite() == Some(side));
            }
        }
    }
    news.write(CombatNews::Attack {
        attacker: entity,
        target: target.entity(),
        how: Announcement::Monster { word: attack.word, mode, named: attack.named() },
        swings,
        surprise,
    });
    let backstab = crate::game::characters::scale_by_level(kind.level, 100.0) as i32;
    let mut tally = Tally::default();
    for _ in 0..swings {
        let (monster, ..) = field.monsters.monsters().get(entity).unwrap();
        let monster = monster.clone();
        // A fumble may incapacitate the attacker between its swings.
        if !monster.can_fight() {
            break;
        }
        let Some((aimed_at, _)) = victim_defence(battle, field, fight, target) else { break };
        let blows = attack::Attacker::monster((kind, &monster, stance), attack);
        let aim = blows.aim(mode, surprise, Some(&aimed_at), difficulty);
        let delivery = if attack.reach >= Reach::Long {
            Delivery::Shot {
                from: eye,
                blind: monster.conditions.has(Condition::Blind),
                through: false,
            }
        } else {
            Delivery::Swing {
                bystanders: monster_bystanders(field, fight, entity, attack.reach, target.entity()),
                odds: blows.aim(mode, surprise, None, difficulty),
            }
        };
        let Ok(landing) = land(field, fight, (target, aim), delivery, &mut surprise) else {
            break;
        };
        let Some(mut landing) = landing else { continue };
        if landing.fumbled {
            news.write(CombatNews::Fumble { attacker: entity, victim: landing.victim });
        }
        let pressing = attack::monster_protection(kind, &monster);
        landing.victim = intercept(battle, field, (pressing, &mut surprise), landing.victim, news);
        let Some((struck, victim_character)) = victim_defence(battle, field, fight, landing.victim)
        else {
            continue;
        };
        let at = struck.locate(&mut field.rng, victim_character.as_ref().map(|_| kind));
        if !landing.sure && aim < landing.roll as i32 {
            continue;
        }
        // The party's Missile Shield turns aside what is shot or thrown.
        if matches!(landing.victim, Victim::Member(_))
            && field.enchantments.deflects(mode, &mut field.rng)
        {
            continue;
        }
        let through = blows.penetration(mode, surprise, (&struck, at), difficulty);
        let dealt = if landing.through || through >= landing.roll as i32 {
            let count = attack::dice_count(&mut field.rng, mode, &struck, surprise, backstab, 0);
            let rolled = attack::roll_dice(&mut field.rng, attack.damage, count);
            let mut damage = attack::monster_damage(rolled, &struck);
            if let Some((victim, c)) = &victim_character {
                iron_skin(field, *victim, c, &mut tally);
            }
            if damage > 0 && landing.fumbled {
                damage = struck.fumble_cap(damage);
            }
            let dealt = if damage > 0 {
                tally.struck(effects::hurt(field, landing.victim, damage), false)
            } else {
                0
            };
            if dealt > 0 {
                let power = attack::effect_power(u32::from(kind.level));
                let landed = effects::blow(field, landing.victim, &attack.effects, power, true);
                for effect in &landed {
                    tally.struck(Some(*effect), false);
                }
                tally.effects.extend(landed);
            }
            dealt
        } else {
            0
        };
        if attack.reach < Reach::Long {
            razor_backlash(
                field,
                fight,
                battle,
                landing.victim,
                Victim::Monster(entity),
                &mut tally,
                news,
            );
        }
        if landing.fumbled {
            news.write(CombatNews::Result {
                attacker: entity,
                target: landing.victim.entity(),
                hits: 1,
                damage: dealt,
            });
        } else {
            tally.hits += 1;
            tally.damage += dealt;
        }
    }
    tally.report(battle, (entity, target.entity()), news);
    let work = &mut battle.this_round.monsters[index];
    work.phase = crate::game::combat::next_phase(turn.phase, turn_left);
    work.attacks_left = turn_left;
    false
}
