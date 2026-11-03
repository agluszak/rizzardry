//! One swing is a calculation, not an entity or a partially constructed bundle.
use super::*;

fn practise(world: &mut World, actor: Entity, skill: Skill, rng: &mut GameRng) {
    if let Some(mut character) = world.get_mut::<Character>(actor)
        && character.skill_available(skill)
        && character.practise(skill, 1, rng)
    {
        world.write_message(CombatNews::Improved { member: actor, skill });
    }
}

fn cap_blind(aim: i32, vitals: &Vitals, fighting: BlindFighting) -> i32 {
    if aim <= 10 || !vitals.conditions.has(Condition::Blind) {
        return aim;
    }
    fighting.0.map_or(10, |level| {
        crate::game::characters::scale_by_level(level, (aim - 10) as f32) as i32 + 10
    })
}

/// All aim terms share one snapshot of the two bodies. Fumble odds deliberately
/// omit the target's guard; blind fighting and difficulty apply to both values.
fn aim(
    world: &World,
    by: Entity,
    target: Entity,
    arm: &Arm,
    mode: AttackMode,
    surprise: Surprise,
) -> (i32, i32) {
    let vitals = world.get::<Vitals>(by).unwrap();
    let fatigue = attack::fatigue(vitals);
    let base = arm.rating
        + attack::mode_to_hit(mode)
        + 10 * i32::from(surprise.unaware)
        + 10 * i32::from(surprise.natural)
        - if arm.modern { fatigue >> 1 } else { fatigue };
    let armour = world.get::<ArmourClass>(target).unwrap();
    let agility =
        (world.get::<Agility>(by).unwrap().0 - world.get::<Agility>(target).unwrap().0) / 10;
    let fighting = *world.get::<BlindFighting>(by).unwrap();
    let difficulty = world.resource::<Difficulty>();
    let side = world.get::<Side>(by).unwrap().0;
    (
        difficulty.scale(
            cap_blind(base - 5 * armour.against_aim(mode, surprise) + agility, vitals, fighting),
            side,
        ),
        difficulty.scale(cap_blind(base, vitals, fighting), side),
    )
}

/// Guarding has its own round lifetime. Interception is one decision that
/// updates that relationship's counter and returns the actual victim.
fn intercept(
    world: &mut World,
    by: Entity,
    ward: Entity,
    surprise: &mut Surprise,
    rng: &mut GameRng,
) -> Entity {
    let Some(guards) = world.get::<Guards>(ward) else { return ward };
    if world.get::<Vitals>(ward).is_none_or(Vitals::is_dead) {
        return ward;
    }
    let at = *world.get::<Footing>(ward).unwrap();
    let pressing = world.get::<Protection>(by).unwrap().0
        + 10 * i32::from(surprise.unaware)
        + 10 * i32::from(surprise.natural);
    let mut stepping = Vec::new();
    for &guard in guards.entities() {
        let Some(arms) = world.get::<Arms>(guard) else { continue };
        let Some(first) = arms.0.first() else { continue };
        let Some(taken) = world.get::<Interceptions>(guard) else { continue };
        if !world.get::<Vitals>(guard).is_some_and(Vitals::can_guard)
            || world.get::<Footing>(guard).is_none_or(|there| there.gap(&at) > first.reach.metres())
            || taken.taken >= taken.allowed
        {
            continue;
        }
        let Some(protection) = world.get::<Protection>(guard) else { continue };
        if rng.below(100) < (protection.0 - pressing + 50).clamp(5, 95) as u32 {
            stepping.push(guard);
        }
    }
    if stepping.is_empty() {
        return ward;
    }
    let guard = stepping[rng.below(stepping.len() as u32) as usize];
    world.get_mut::<Interceptions>(guard).unwrap().taken += 1;
    world.get_mut::<Vitals>(guard).unwrap().tire(rng.below(3) + 2);
    *surprise = Surprise::default();
    world.write_message(CombatNews::Intercepts { guard, ward });
    guard
}

fn blow(
    world: &mut World,
    act: Act,
    victim: Entity,
    strength: Strength,
    damage: u32,
    effects: AttackEffects,
    monster: bool,
) -> Entity {
    let definition = world.resource::<Definitions>().blow();
    let deed = world
        .spawn((
            Deed::new(act),
            Uses(definition),
            strength,
            BlowDamage(damage),
            WeaponEffects { effects, by_monster: monster },
        ))
        .id();
    world.spawn((crate::game::hits::HitOf(deed), crate::game::hits::Strikes(victim)));
    deed
}

fn riposte(world: &mut World, by: Entity, victim: Entity, arm: &Arm, rng: &mut GameRng) {
    if arm.reach >= Reach::Long {
        return;
    }
    let Some(charge) = world
        .get_mut::<Vitals>(victim)
        .and_then(|mut v| v.enchantments.spend_charge(SpellId::RAZOR_CLOAK))
    else {
        return;
    };
    let dice = world.resource::<Catalogs>().spells[SpellId::RAZOR_CLOAK].dice;
    let strength = Strength { power: charge.power, boost: 0, against: 0 };
    let rolled = crate::game::hits::definitions::magnitude(rng, dice, strength);
    let reduction = world.get::<Reduction>(by).map_or(0, |r| r.0);
    let amount = attack::reduce(rolled as i32, reduction);
    if amount == 0 {
        world.write_message(CombatNews::Result {
            attacker: victim,
            target: by,
            hits: 1,
            damage: 0,
        });
    } else {
        blow(
            world,
            Act::Blow { by: victim, at: by },
            by,
            Strength::default(),
            amount,
            AttackEffects::default(),
            false,
        );
    }
}

/// Resolve hit, penetration and damage with locals. Costs are paid whether the
/// swing lands or misses. Its deed is settled by the caller before another swing.
pub(super) fn swing(world: &mut World, rng: &mut GameRng, attack_entity: Entity) -> Option<Entity> {
    let by = world.get::<By>(attack_entity)?.0;
    let target = world.get::<AimsAt>(attack_entity)?.0;
    let arm = world.get::<Striking>(attack_entity)?.0.clone();
    let mode = world.get::<Mode>(attack_entity)?.0;
    let mut surprise = *world.get::<Surprise>(attack_entity)?;
    let (aim, odds) = aim(world, by, target, &arm, mode, surprise);
    let mut victim = target;
    let mut fumbled = false;
    let mut through = false;
    let landed = if arm.reach >= Reach::Long {
        let from = world.get::<Footing>(by).unwrap();
        let at = world.get::<Footing>(target).unwrap();
        let blind = world.get::<Vitals>(by).unwrap().conditions.has(Condition::Blind);
        let point = attack::scatter(rng, from.eye, at.centre, aim, blind);
        if attack::strikes(at.centre, point, at.radius, at.half_height) {
            through = arm.sure_throw;
            Some((rng.below(100) + 1, true))
        } else {
            None
        }
    } else {
        let mut formations = world.query::<&Formation>();
        let mut participants = world.query::<&Combatants>();
        let from = world.get::<Footing>(by).unwrap();
        let formation = formations.single(world).unwrap();
        let fighters = participants.single(world).unwrap();
        let bystanders: Vec<_> = fighters
            .entities()
            .iter()
            .copied()
            .filter(|&other| other != target)
            .filter(|&other| {
                world.get::<Vitals>(other).is_some_and(|v| !v.is_dead())
                    && (other == by
                        || world.get::<Footing>(other).is_some_and(|there| {
                            reaches(formation, (by, from), (other, there), arm.reach)
                        }))
            })
            .collect();
        match attack::swing(rng, bystanders.len(), attack::fumble_chance(bystanders.len(), odds)) {
            attack::Aim::Missed => None,
            attack::Aim::Reaches { roll, sure } => {
                through = sure;
                Some((roll, sure))
            }
            attack::Aim::Fumbled { bystander, roll } => {
                surprise = Surprise::default();
                victim = bystanders[bystander];
                fumbled = true;
                world.write_message(CombatNews::Fumble { attacker: by, victim });
                Some((roll, true))
            }
        }
    };
    let mut deed = None;
    if let Some((roll, sure)) = landed {
        victim = intercept(world, by, victim, &mut surprise, rng);
        let location = match (world.get::<HitLocations>(victim), arm.lands_on_party) {
            (Some(table), _) => attack::locate(rng, &table.0, attack::MONSTER_BODY),
            (None, Some(hits)) => attack::locate(rng, hits.as_slice(), BodyPart::Body as usize),
            (None, None) => attack::locate(rng, &attack::MEMBER_LOCATIONS, BodyPart::Body as usize),
        };
        if sure || aim >= roll as i32 {
            let piercing = arm.loaded.map_or(mode, |loaded| attack::choose_mode(rng, loaded));
            let armour = *world.get::<ArmourClass>(victim).unwrap();
            let fatigue = if arm.fatigue_pierces {
                attack::fatigue(world.get::<Vitals>(by).unwrap())
            } else {
                0
            };
            let agility = (world.get::<Agility>(by).unwrap().0
                - world.get::<Agility>(victim).unwrap().0)
                / 10;
            let penetration = world.resource::<Difficulty>().scale(
                attack::mode_penetration(piercing)
                    - fatigue
                    - 5 * armour.at(piercing, surprise, location.0)
                    + agility,
                world.get::<Side>(by).unwrap().0,
            );
            let shield = world.get::<MissileShield>(victim).map_or(0, |s| s.0);
            let deflected = matches!(mode, AttackMode::Shoot | AttackMode::Throw)
                && shield > 0
                && rng.below(100) < shield;
            let mut damage = 0;
            let mut chances = arm.effects;
            if !deflected && (through || penetration >= roll as i32) {
                let mut dice_count =
                    if (surprise.unaware || armour.distracted) && !attack::keeps_distance(mode) {
                        2
                    } else {
                        1
                    };
                if surprise.natural && !attack::keeps_distance(mode) {
                    dice_count +=
                        attack::backstab_dice(rng, world.get::<Backstab>(by).map_or(0, |b| b.0));
                }
                if mode == AttackMode::Berserk {
                    dice_count += 1;
                }
                if dice_count > 1 && armour.braced && !surprise.unaware {
                    dice_count -= 1;
                }
                if let Some(kind) = world.get::<crate::game::hits::defences::Kind>(victim) {
                    dice_count += arm.slays.iter().filter(|&&k| k == kind.0).count() as u32;
                }
                let rolled = attack::roll_dice(
                    rng,
                    world.get::<DamageDice>(attack_entity).unwrap().0,
                    dice_count,
                );
                damage = (((100 + arm.percent) * rolled as i32 + 50) / 100).max(1) as u32;
                damage = attack::reduce(
                    damage as i32,
                    world.get::<Reduction>(victim).map_or(0, |r| r.0),
                );
                if fumbled && damage > 0 {
                    damage =
                        attack::fumble_cap(damage, world.get::<Vitals>(victim).unwrap().health.max);
                }
                practise(world, victim, Skill::IronSkin, rng);
                if !fumbled && let Some(character) = world.get::<Character>(by) {
                    if let Some(skill) = arm.critical_skill {
                        let skill = character
                            .stats(&world.get::<Vitals>(by).unwrap().enchantments)
                            .skill(skill);
                        chances.chances[AttackEffect::Kill] = (chances.chances[AttackEffect::Kill]
                            + attack::critical_chance(rng, skill))
                        .min(100);
                    }
                    if character.has_trait(Trait::Knockout) {
                        chances.chances[AttackEffect::KnockOut] = (chances.chances
                            [AttackEffect::KnockOut]
                            + character.scale_by_profession(5.0) as u32)
                            .min(100);
                    }
                }
            }
            if damage > 0 {
                let act = if fumbled { Act::Blow { by, at: victim } } else { Act::Swing };
                deed = Some(blow(
                    world,
                    act,
                    victim,
                    *world.get::<Strength>(attack_entity).unwrap(),
                    damage,
                    chances,
                    arm.word.is_some(),
                ));
                if !fumbled {
                    let skills: &[Skill] = if !arm.ranged {
                        &[Skill::CriticalStrike, Skill::PowerStrike]
                    } else if world
                        .get::<Character>(by)
                        .is_some_and(|c| c.has_trait(Trait::ThrownCriticals))
                    {
                        &[Skill::EagleEye, Skill::CriticalStrike]
                    } else {
                        &[Skill::EagleEye]
                    };
                    for &skill in skills {
                        practise(world, by, skill, rng);
                    }
                }
            }
            riposte(world, by, victim, &arm, rng);
            if !fumbled {
                world.get_mut::<Tally>(attack_entity).unwrap().hits += 1;
            } else if damage == 0 {
                world.write_message(CombatNews::Result {
                    attacker: by,
                    target: victim,
                    hits: 1,
                    damage: 0,
                });
            }
        }
    }
    *world.get_mut::<Surprise>(attack_entity).unwrap() = surprise;
    if arm.tires
        && let Some(burden) = world.get::<Burden>(by)
    {
        let load = burden.load;
        let berserk = world.get::<Berserk>(attack_entity).is_some();
        let mut vitals = world.get_mut::<Vitals>(by).unwrap();
        let cost = attack::swing_stamina(
            rng,
            mode,
            arm.weight,
            berserk,
            vitals.conditions.has(Condition::Diseased),
        );
        let cost = load.scale_fatigue(
            cost,
            vitals.conditions.has(Condition::Slowed),
            vitals.enchantments.has(SpellId::HASTE),
        );
        vitals.tire(cost);
    }
    deed
}

pub(super) fn spend_ammunition(world: &mut World, attack: Entity) -> bool {
    let by = world.get::<By>(attack).unwrap().0;
    let arm = world.get::<Striking>(attack).unwrap().0.clone();
    if !arm.ranged || arm.weapon.is_none() {
        return false;
    }
    world.resource_scope(|world, content: Mut<Catalogs>| {
        let loaded = arm.loaded.is_some();
        let slot = if loaded { arm.slot.other() } else { arm.slot };
        let Some(mut carried) = world.get_mut::<Carried>(by) else { return false };
        let Some(stack) = carried.worn(slot.equip()) else { return false };
        let (id, definition) = (stack.id, &content.items[stack.item]);
        let thrown = definition.class == crate::content::items::EquipClass::ThrownWeapon;
        if !loaded && (definition.spell.is_some() || (thrown && definition.never_depletes)) {
            return false;
        }
        carried.consume_use(id, definition, definition.keeps_empty());
        if carried.worn(slot.equip()).is_none_or(|stack| stack.remaining_uses() == Some(0)) {
            if let Some(mut turn) = world.get_mut::<Turn>(by) {
                turn.attacks[arm.slot] = 0;
            }
            true
        } else {
            false
        }
    })
}
