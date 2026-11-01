//! A member's attack: the hand due, its target, its swings and what each
//! blow does (retail `StartCharacterAttack`, `ResolveCharacterAttack`).
use super::*;

/// Who a member's swing could go astray onto (retail
/// `BuildCharacterTargetList`): themselves, the members their hand reaches,
/// and the monsters it reaches but the one aimed at.
pub(super) fn member_bystanders(
    field: &Field,
    fight: &Fight,
    member: Entity,
    reach: Reach,
    target: Entity,
) -> Vec<Victim> {
    let formation = &**field.formation;
    let mut bystanders: Vec<Victim> = field
        .party
        .iter()
        .filter(|&m| field.members.get(m).is_ok_and(|(_, v, ..)| !v.is_dead()))
        .filter(|&m| m == member || reach > Reach::Touch || !formation.screens(member, m))
        .map(Victim::Member)
        .collect();
    for &entity in fight.combatants {
        let (monster, at, walker, ..) = field.monsters.monsters().get(entity).unwrap();
        let distance = party_distance(fight.party.eye, monster, feet(at.translation(), walker));
        if entity != target
            && !monster.is_dead()
            && field.monsters.active(entity)
            && fight.party.reaches(formation, member, at.translation(), reach, distance)
        {
            bystanders.push(Victim::Monster(entity));
        }
    }
    bystanders
}

/// A Berserk order needs the trait, a hand that can attack and a main hand
/// for close quarters (retail `CanCharacterBerserk`).
pub(in crate::game::combat) fn can_berserk(character: &Character, grip: &attack::Grip<'_>) -> bool {
    character.has_trait(Trait::Berserk)
        && (grip.in_play(0) || grip.in_play(1))
        && (!grip.in_play(0) || grip.reach(0) <= Reach::Short)
}

/// Retail `ChooseMonsterTarget`: among the monsters a hand reaches, the
/// least afflicted, then those "in reach" of the party, then the nearest;
/// ties keep the fight's order.
pub(in crate::game::combat) fn melee_target<'a>(
    member: Entity,
    reach: Reach,
    field: &Field<'a, '_>,
    fight: &Fight,
) -> Option<(Entity, &'a Species)> {
    let formation = &**field.formation;
    let species = field.species();
    fight
        .combatants
        .iter()
        .filter_map(|&entity| {
            let (monster, at, walker, ..) = field.monsters.monsters().get(entity).ok()?;
            let kind = &species[monster.species];
            if kind.npc.is_some() {
                return None;
            }
            let distance = party_distance(fight.party.eye, monster, feet(at.translation(), walker));
            let turned = field
                .members
                .get(member)
                .is_ok_and(|(_, vitals, ..)| vitals.conditions.has(Condition::Turncoat));
            let side = field.monsters.stance(entity, field.now());
            let wanted = if turned { Stance::Friendly } else { Stance::Hostile };
            if (turned && side != wanted || !turned && side == Stance::Friendly)
                || !field.monsters.active(entity)
                || monster.is_dead()
                || !fight.party.reaches(formation, member, at.translation(), reach, distance)
            {
                return None;
            }
            let worst = monster.conditions.worst();
            let side = formation::quadrant(fight.party.eye, fight.party.yaw, at.translation());
            let in_reach = !kind.attacks.is_empty()
                && (side == Row::Rear
                    || (!formation.occupied(side) && formation.occupied(Row::Center)));
            Some(((worst, !in_reach, distance), (entity, kind)))
        })
        .min_by(|a, b| a.0.0.cmp(&b.0.0).then(a.0.1.cmp(&b.0.1)).then(a.0.2.total_cmp(&b.0.2)))
        .map(|(_, target)| target)
}

pub(super) fn member_target(
    member: Entity,
    reach: Reach,
    field: &Field,
    fight: &Fight,
    rng: &mut GameRng,
) -> Option<Victim> {
    if field
        .members
        .get(member)
        .is_ok_and(|(_, vitals, ..)| vitals.conditions.has(Condition::Turncoat))
    {
        let mut targets: Vec<_> = field
            .party
            .iter()
            .filter(|&target| {
                target != member
                    && field.members.get(target).is_ok_and(|(_, vitals, ..)| {
                        !vitals.is_dead() && !vitals.conditions.has(Condition::Turncoat)
                    })
            })
            .map(Victim::Member)
            .collect();
        if let Some((target, _)) = melee_target(member, reach, field, fight) {
            targets.push(Victim::Monster(target));
        }
        return (!targets.is_empty()).then(|| targets[rng.below(targets.len() as u32) as usize]);
    }
    melee_target(member, reach, field, fight).map(|(entity, _)| Victim::Monster(entity))
}

/// One of a member's attacks with the hand due (retail
/// `StartCharacterAttack` and its swings): the best target the hand
/// reaches, its mode, its swings, each rolled to hit, to penetrate and for
/// damage and effects, then the hand due next and when.
pub(in crate::game::combat) fn member_attacks(
    battle: &mut Battle,
    index: usize,
    rng: &mut GameRng,
    field: &mut Field,
    fight: &mut Fight,
    news: &mut MessageWriter<CombatNews>,
) {
    let work = battle.this_round.party[index];
    let member = work.actor;
    let berserk = matches!(work.action, RoundAction::Attack { berserk: true, .. });
    let step = match battle.phase {
        RoundPhase::Executing(step) => step,
        _ => 1,
    };
    let (items, difficulty) = (field.items(), field.difficulty());
    let (character, vitals, carried, burden) = field.members.get(member).unwrap();
    let (character, load) = (character.clone(), burden.load);
    let controlled = vitals.conditions.has(Condition::Turncoat);
    let slowed = vitals.conditions.has(Condition::Slowed);
    let grip = attack::Grip::of(carried, items);
    let stats = character.stats(&vitals.enchantments);
    let fighter = attack::Fighter::new(&stats, &grip, load);
    let initiative = [fighter.initiative(0), fighter.initiative(1)];
    let most_swings = [fighter.swings(0), fighter.swings(1)];
    // The party's blessings guide every hand.
    let blessing = field.enchantments.hit_bonus();
    let mut turns = work.hands;
    // The hand due attacks if it still can; a hand that cannot (out of
    // shots, or nothing in its reach) gives up only its own attacks, and
    // the other hand takes over (retail `StartCharacterAttack`,
    // `ComputeCharacterActionPhase`).
    let mut due = None;
    for current in [turns.current, 1 - turns.current] {
        let target = (grip.in_play(current) && turns.left[current] > 0)
            .then(|| member_target(member, grip.reach(current), field, fight, rng))
            .flatten();
        match target {
            Some(target) => {
                due = Some((current, target));
                break;
            }
            None => turns.left[current] = 0,
        }
    }
    let Some((current, target)) = due else {
        battle.this_round.party[index].end();
        return;
    };
    turns.current = current;
    let hand = current;
    let reach = grip.reach(hand);
    battle.this_round.attacked = true;
    // Retail draws the attack's mode, then its swings.
    let mode = if berserk {
        AttackMode::Berserk
    } else {
        attack::choose_mode(rng, fighter.strike(hand).modes)
    };
    turns.left[current] -= 1;
    let mut swings = rng.below(most_swings[hand]) + 1;
    // A Rogue's sword or dagger at arm's length strikes where it hurts.
    let backstab = character.has_trait(Trait::Backstab)
        && reach <= Reach::Touch
        && matches!(grip.weapon_skill(hand), Skill::Sword | Skill::Dagger);
    swings = (swings as i32 + grip.extra_swings(hand)).max(1) as u32;
    // A Samurai's Lightning Strike, readied as the round began.
    if turns.lightning && current == 0 {
        if grip.weapon_skill(0) == Skill::Sword && !slowed {
            let level = u32::from(character.profession_levels[character.profession]);
            swings += if rng.below(11) + level < 20 { 3 } else { 4 };
        }
        turns.lightning = false;
    }
    let mut surprise = Surprise { unaware: false, natural: backstab };
    if reach <= Reach::Short
        && let Victim::Monster(target) = target
        && let Ok((_, at, ..)) = field.monsters.monsters().get(target)
    {
        surprise.unaware = back_to_party(at.forward(), at.translation(), fight.party.eye);
    }
    news.write(CombatNews::Attack {
        attacker: member,
        target: target.entity(),
        how: Announcement::Mode(mode),
        swings,
        surprise,
    });
    // Attacking a monster provokes its group, whatever the blows do
    // (retail `MakeTargetGroupHostile` as the action starts).
    if !controlled && let Victim::Monster(target) = target {
        field.provoke(target);
    }
    let backstab_chance = character.scale_by_profession(100.0) as i32;
    let mut tally = Tally::default();
    let mut left = swings;
    while left > 0 {
        left -= 1;
        let (_, vitals, ..) = field.members.get(member).unwrap();
        let vitals: Vitals = vitals.clone();
        if !vitals.can_fight() {
            break;
        }
        let stats = character.stats(&vitals.enchantments);
        let fighter = attack::Fighter::new(&stats, &grip, load).blessed(blessing);
        let Some((aimed_at, _)) = victim_defence(battle, field, fight, target) else { break };
        let aim_at = |target| {
            attack::member_aim(&fighter, &vitals, hand, mode, surprise, target, difficulty)
        };
        let aim = aim_at(Some(&aimed_at));
        let delivery = if reach >= Reach::Long {
            Delivery::Shot {
                from: fight.party.eye,
                blind: vitals.conditions.has(Condition::Blind),
                through: attack::sure_penetration(&fighter, hand),
            }
        } else {
            Delivery::Swing {
                bystanders: member_bystanders(field, fight, member, reach, target.entity()),
                odds: aim_at(None),
            }
        };
        let Ok(landing) = land(rng, field, fight, (target, aim), delivery, &mut surprise) else {
            break;
        };
        if let Some(mut landing) = landing {
            if landing.fumbled {
                news.write(CombatNews::Fumble { attacker: member, victim: landing.victim });
            }
            let pressing = attack::member_protection(&fighter, &vitals);
            landing.victim =
                intercept(battle, rng, field, (pressing, &mut surprise), landing.victim, news);
            let dealt = member_blow(
                battle,
                (member, &fighter, &vitals),
                (hand, mode),
                (surprise, aim, backstab_chance),
                landing,
                (rng, field, fight),
                &mut tally,
            );
            if dealt.is_some() && reach < Reach::Long {
                razor_backlash(
                    rng,
                    field,
                    fight,
                    battle,
                    landing.victim,
                    Victim::Member(member),
                    &mut tally,
                    news,
                );
            }
            if landing.fumbled {
                news.write(CombatNews::Result {
                    attacker: member,
                    target: landing.victim.entity(),
                    hits: 1,
                    damage: dealt.unwrap_or(0),
                });
            } else if let Some(dealt) = dealt {
                tally.hits += 1;
                tally.damage += dealt;
            }
        }
        // Every swing tires, twice as much in a berserk fury.
        let (_, mut vitals, mut carried, _) = field.members.get_mut(member).unwrap();
        let weight = grip.weight(hand);
        let cost = attack::swing_stamina(
            rng,
            mode,
            weight,
            berserk,
            vitals.conditions.has(Condition::Diseased),
        );
        let hasted = vitals.enchantments.has(crate::content::spells::SpellId::HASTE);
        vitals.spend_stamina(load.scale_fatigue(cost, slowed, hasted));
        // Arrows, bolts and stones are shot away, thrown weapons thrown,
        // and guns spend their shots.
        // A hand with nothing left to fire is done for the round.
        let paired = grip.paired(hand);
        if (paired || grip.ranged(hand)) && !spend_shot(&mut carried, items, current, paired) {
            left = 0;
            turns.left[current] = 0;
        }
    }
    tally.report(battle, (member, target.entity()), news);
    next_hand(battle, index, step, turns, initiative);
}

/// Spend what a ranged swing used (retail `ResolveCharacterAttack`'s
/// tail): the ammunition a launcher shot, a thrown weapon unless it never
/// runs out, or a gun's shot. Returns whether the hand can go on.
pub(super) fn spend_shot(
    carried: &mut crate::game::inventory::Carried,
    items: &crate::content::items::ItemCatalog,
    current: usize,
    paired: bool,
) -> bool {
    use crate::game::inventory::EquipSlot;
    let slots = [EquipSlot::Primary, EquipSlot::Secondary];
    let spent = if paired { slots[1 - current] } else { slots[current] };
    let Some(stack) = carried.worn(spent) else { return false };
    let definition = &items[stack.item];
    if !paired && definition.spell.is_some() {
        return true;
    }
    let thrown = definition.class == crate::content::items::EquipClass::ThrownWeapon;
    if !paired && thrown && definition.never_depletes {
        return true;
    }
    carried.consume_use(stack.id, definition, definition.keeps_empty());
    carried.worn(spent).is_some_and(|s| s.remaining_uses() != Some(0))
}

/// One member swing that struck someone: its penetration, damage and
/// effects. Returns the damage dealt when it hit, `None` when it was
/// dodged.
pub(super) fn member_blow(
    battle: &Battle,
    (attacker, fighter, vitals): (Entity, &attack::Fighter<'_>, &Vitals),
    (hand, mode): (usize, AttackMode),
    (surprise, aim, backstab): (Surprise, i32, i32),
    landing: Landing,
    (rng, field, fight): (&mut GameRng, &mut Field, &mut Fight),
    tally: &mut Tally,
) -> Option<u32> {
    let (character, grip) = (fighter.stats, fighter.grip);
    let strike = fighter.strike(hand);
    let difficulty = field.difficulty();
    let (struck, victim_character) = victim_defence(battle, field, fight, landing.victim)?;
    let at = struck.locate(rng, None);
    if !landing.sure && aim < landing.roll as i32 {
        return None;
    }
    // A launcher pierces by what it shoots.
    let piercing = if grip.paired(hand) {
        attack::choose_mode(rng, fighter.strike(1 - hand).modes)
    } else {
        mode
    };
    let through = attack::member_penetration(
        fighter,
        vitals,
        hand,
        piercing,
        surprise,
        (&struck, at),
        difficulty,
    );
    if !landing.through && through < landing.roll as i32 {
        return Some(0);
    }
    // Missile Shield turns aside what is shot or thrown.
    if let Victim::Monster(monster) = landing.victim
        && field
            .monsters
            .monsters()
            .get(monster)
            .is_ok_and(|(m, ..)| m.enchantments.deflects(mode, rng))
    {
        return Some(0);
    }
    let slaying = struck.species().map_or(0, |kind| grip.slays(hand, kind.kind));
    let count = attack::dice_count(rng, mode, &struck, surprise, backstab, slaying);
    let dice = match mode {
        AttackMode::Punch => fighter.strike(1).dice,
        AttackMode::Kick => fighter.strike(0).dice,
        _ => strike.dice,
    };
    let rolled = attack::roll_dice(rng, dice, count);
    let mut damage = attack::member_damage(fighter.damage_percent(hand), rolled, &struck);
    if let Some((victim, c)) = &victim_character {
        iron_skin(rng, field, *victim, c, tally);
    }
    if damage == 0 {
        return Some(0);
    }
    if landing.fumbled {
        damage = struck.fumble_cap(damage);
    }
    let struck = effects::hurt(rng, field, landing.victim, damage);
    let dealt = tally.struck(struck, true);
    if dealt > 0 {
        let mut blow = strike.effects;
        if !landing.fumbled {
            let kill = &mut blow.chances[AttackEffect::Kill];
            *kill = (*kill + attack::critical_chance(rng, fighter, hand)).min(100);
            if character.has_trait(Trait::Knockout) {
                let knockout = &mut blow.chances[AttackEffect::KnockOut];
                *knockout = (*knockout + character.scale_by_profession(5.0) as u32).min(100);
            }
        }
        let power = attack::effect_power(u32::from(character.level()));
        let landed = effects::blow(rng, field, landing.victim, &blow, power, false);
        for effect in &landed {
            tally.struck(Some(*effect), true);
        }
        tally.effects.extend(landed);
    }
    // Landing blows trains the attacker's strikes.
    if !landing.fumbled
        && dealt > 0
        && let Ok((mut trained, ..)) = field.members.get_mut(attacker)
    {
        let skills: &[Skill] = if !grip.ranged(hand) {
            &[Skill::CriticalStrike, Skill::PowerStrike]
        } else if trained.has_trait(Trait::ThrownCriticals) {
            &[Skill::EagleEye, Skill::CriticalStrike]
        } else {
            &[Skill::EagleEye]
        };
        for &skill in skills {
            if trained.skill_available(skill) && trained.practise(skill, 1, rng) {
                tally.improved.push((attacker, skill));
            }
        }
    }
    Some(dealt)
}

/// The hand due next and when (retail `ComputeCharacterActionPhase`): the
/// other hand while it has attacks, unless it has fewer and less
/// initiative; spread over the rest of the round, and a switch costs the
/// difference in the hands' initiative.
pub(super) fn next_hand(
    battle: &mut Battle,
    index: usize,
    step: u32,
    turns: HandTurns,
    initiative: [i32; 2],
) {
    let current = turns.current;
    let other = 1 - current;
    let (now, then) = (turns.left[current], turns.left[other]);
    let chosen = if then != 0 && !(then < now && initiative[other] < initiative[current]) {
        other
    } else {
        current
    };
    let left = turns.left[chosen];
    let phase = if left == 0 {
        None
    } else if chosen == current || then <= now {
        Some(step + (100 - step) / (left + 1))
    } else {
        Some(step)
    };
    let work = &mut battle.this_round.party[index];
    work.hands = turns;
    match phase.filter(|&phase| phase <= 100) {
        Some(phase) => {
            let phase = phase as i32
                + if chosen != current { initiative[current] - initiative[chosen] } else { 0 };
            work.hands.current = chosen;
            work.phase = phase.clamp(step as i32, 100) as u32;
        }
        None => work.end(),
    }
}
