//! Special attacks: breaths, gazes and the like, and the clouds that
//! linger over a fight (retail `ExecuteMonsterSpecialAttack`,
//! `TickCombatEffectSlots`).
use super::*;

/// What one of a special attack's two effects does to a victim (retail
/// `g_special_attack_table`'s entries).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecialEffect {
    /// A condition, for rounds rolled by its strength (entry 1).
    Inflict(Condition),
    /// Health taken straight off (entry 2, `DamageCharacter`).
    Harm,
    /// Magic drained (entry 3).
    Drain,
    /// Stamina spent (entry 4).
    Tire,
    /// A wound dealt as a blow's is (entry 5, `ApplyDamageToCharacter`).
    Wound,
}

/// What a special attack is (retail `g_special_attack_table`,
/// `g_special_attack_condition_table`, `g_special_attack_realm_table`,
/// `MonsterSpecialAttackHonorsCastingBlock`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpecialDefinition {
    /// Its two effects; a summoning has none and calls a group instead.
    pub effects: [Option<SpecialEffect>; 2],
    pub summons: bool,
    /// The realm victims resist it by.
    pub realm: Realm,
    /// Whether resistance cuts it at all; one kind's wound is not resisted.
    pub resisted: bool,
    /// Whether a silence stops it.
    pub silenced: bool,
}

/// A special attack's definition (retail `g_special_attack_table`,
/// `g_special_attack_condition_table`, `g_special_attack_realm_table`,
/// `MonsterSpecialAttackHonorsCastingBlock`).
pub fn special(kind: SpecialAttackKind) -> SpecialDefinition {
    use Condition::*;
    use Realm::*;
    use SpecialAttackKind as K;
    use SpecialEffect::*;
    let strike = |realm, first, second: Option<SpecialEffect>| SpecialDefinition {
        effects: [Some(first), second],
        summons: false,
        realm,
        resisted: true,
        silenced: false,
    };
    let summon = |silenced| SpecialDefinition {
        effects: [None, None],
        summons: true,
        realm: Fire,
        resisted: true,
        silenced,
    };
    let silenced = |definition| SpecialDefinition { silenced: true, ..definition };
    match kind {
        K::BlindingWater | K::BlindingWaterSecond => strike(Water, Inflict(Blind), None),
        K::HarmAndDrainDivine => strike(Divine, Harm, Some(Drain)),
        K::TiringWater => strike(Water, Tire, None),
        K::FearAndInsanityMental => {
            silenced(strike(Mental, Inflict(Afraid), Some(Inflict(Insane))))
        }
        K::FearAndTurncoatDivine => {
            silenced(strike(Divine, Inflict(Afraid), Some(Inflict(Turncoat))))
        }
        K::FearMental => silenced(strike(Mental, Inflict(Afraid), None)),
        K::WoundAndTireFire => strike(Fire, Wound, Some(Tire)),
        K::WoundFire => strike(Fire, Wound, None),
        K::InsanityAndWoundMental => {
            SpecialDefinition { resisted: false, ..strike(Mental, Inflict(Insane), Some(Wound)) }
        }
        K::InsanityAndTurncoatDivine => strike(Divine, Inflict(Insane), Some(Inflict(Turncoat))),
        K::InsanityMental => strike(Mental, Inflict(Insane), None),
        K::IrritationEarth => strike(Earth, Inflict(Irritated), None),
        K::IrritationAir => strike(Air, Inflict(Irritated), None),
        K::NauseaEarth => strike(Earth, Inflict(Nauseated), None),
        K::NauseaAir => strike(Air, Inflict(Nauseated), None),
        K::ParalysisMental => strike(Mental, Inflict(Paralyzed), None),
        K::ParalysisEarth => strike(Earth, Inflict(Paralyzed), None),
        K::PoisonAndIrritationWater => strike(Water, Inflict(Poisoned), Some(Inflict(Irritated))),
        K::PoisonAndWoundWater => strike(Water, Inflict(Poisoned), Some(Wound)),
        K::SleepAir => strike(Air, Inflict(Asleep), None),
        K::SlowAndParalysisWater => strike(Water, Inflict(Slowed), Some(Inflict(Paralyzed))),
        K::SlowEarth => strike(Earth, Inflict(Slowed), None),
        K::WoundWater => strike(Water, Wound, None),
        K::SummonsSpecies153 | K::SummonsSpecies151 | K::SummonsSpecies12 => summon(false),
        K::SummonsSpecies149
        | K::SummonsSpecies150
        | K::SummonsSpecies152
        | K::SummonsSpecies148 => summon(true),
    }
}

/// A character's breath (retail special-attack entry 30): a Water blast.
pub const CHARACTER_BREATH: SpecialDefinition = SpecialDefinition {
    effects: [Some(SpecialEffect::Harm), None],
    summons: false,
    realm: Realm::Water,
    resisted: true,
    silenced: false,
};

/// Whoever makes a special attack: how strong they are now.
#[derive(Clone, Copy, Debug)]
pub struct Breather {
    pub level: u8,
    pub stamina: u32,
    pub max_stamina: u32,
}

/// Three rolls under `bound`, summed (retail `RollSpecialAttackSum`).
pub(super) fn three(rng: &mut GameRng, bound: u32) -> u32 {
    rng.below(bound) + rng.below(bound) + rng.below(bound)
}

/// What resistance leaves of `amount` on a victim, or nothing for one dead
/// or gone (retail `ApplyEffectAndAnnounce`).
pub(super) fn resisted(
    rng: &mut GameRng,
    world: &Field,
    victim: Victim,
    realm: Realm,
    amount: u32,
    against: u32,
) -> Option<u32> {
    let target = target(world, victim, realm, None);
    (!target.dead).then(|| resist(rng, amount, target.resistance, target.level, against))
}

/// A monster's special attack (a breath, a gaze) landing on its victims
/// (retail `ResolveMonsterGroupAttack`). Its strength follows its level and
/// how fresh it is; each victim may escape each of its two effects (the
/// first one time in ten, the second one in two), and resists each by its
/// realm. Party members are taken first, then monsters, as retail does. A
/// summoning does nothing here.
pub fn special_attack(
    definition: SpecialDefinition,
    breather: Breather,
    victims: &[Victim],
    rng: &mut GameRng,
    world: &mut Field,
    points: &mut Query<(&LearnedSpells, &mut SpellPoints)>,
) -> Vec<Effect> {
    if definition.summons {
        return Vec::new();
    }
    let realm = definition.realm;
    let level = u32::from(breather.level);
    let fresh = |extra: u32| {
        (breather.stamina * (level + level.min(15) + extra) / breather.max_stamina.max(1)).max(1)
    };
    // Retail's doubling of a fifth-entry effect reads the table's first
    // rows, which never hold one.
    let (against, bound) = (fresh(0), fresh(5));
    let ordered = victims
        .iter()
        .filter(|v| matches!(v, Victim::Member(_)))
        .chain(victims.iter().filter(|v| matches!(v, Victim::Monster(_))));
    let mut effects = Vec::new();
    for &victim in ordered {
        let monster = matches!(victim, Victim::Monster(_));
        for (slot, effect) in definition.effects.iter().enumerate() {
            let escapes = if slot == 0 { rng.below(100) <= 9 } else { rng.below(100) <= 49 };
            let Some(effect) = effect.filter(|_| !escapes) else { continue };
            let landed = match effect {
                SpecialEffect::Inflict(condition) => {
                    let rounds = if monster { (bound / 3).max(1) } else { rng.below(bound) };
                    // Retail rolls a poison's strength, which is not kept.
                    if condition == Condition::Poisoned {
                        rng.below(if monster { 2 } else { bound });
                    }
                    afflict(rng, world, victim, (Some(condition), 0), realm, against, Some(rounds))
                }
                SpecialEffect::Harm => {
                    let rolled = three(rng, bound) / 3 + 1;
                    damage(rng, world, victim, realm, rolled, against)
                }
                SpecialEffect::Drain => {
                    let rolled = three(rng, bound) / if monster { 6 } else { 3 } + 1;
                    match resisted(rng, world, victim, realm, rolled, against) {
                        None => None,
                        Some(0) => Some(Effect::Resisted { victim }),
                        Some(amount) => match victim {
                            // Retail drains the realm six times over.
                            Victim::Member(member) => {
                                let alive = !world.members.get(member).unwrap().1.is_dead();
                                let drained = if alive {
                                    let (known, mut pool) = points.get_mut(member).unwrap();
                                    let maximum = world.mana_maximum(member, known);
                                    pool.drain_realm(realm, amount * 6, &maximum)
                                } else {
                                    0
                                };
                                Some(Effect::Drained { victim, amount: drained })
                            }
                            Victim::Monster(entity) => {
                                world.monsters.drain_magic(entity, amount / 2);
                                Some(Effect::Drained { victim, amount })
                            }
                        },
                    }
                }
                SpecialEffect::Tire => {
                    let rolled = three(rng, bound) + u32::from(!monster);
                    match resisted(rng, world, victim, realm, rolled, against) {
                        None => None,
                        Some(0) => Some(Effect::Resisted { victim }),
                        Some(amount) => Some(tire(world, victim, amount)),
                    }
                }
                SpecialEffect::Wound => {
                    let rolled = three(rng, bound) / 3 + 1;
                    if definition.resisted {
                        damage(rng, world, victim, realm, rolled, against)
                    } else {
                        if target(world, victim, realm, None).dead {
                            None
                        } else {
                            hurt(rng, world, victim, rolled)
                        }
                    }
                }
            };
            effects.extend(landed);
        }
    }
    effects
}

/// Retail offensive slots follow their affected actors; they are not stationary
/// geometry. Tick before slot expiry, using indirect-source difficulty rather
/// than retaining a caster or borrowing a later caster's stats.
pub(crate) fn pulse_clouds(
    combatants: &[Entity],
    party_eye: Vec3,
    rng: &mut GameRng,
    world: &mut Field,
    points: &mut Query<(&LearnedSpells, &mut SpellPoints)>,
) -> Vec<(SpellId, Vec<Effect>)> {
    let spells = world.spells();
    let mut reports = Vec::new();
    let party: Vec<_> = world.party.iter().map(Victim::Member).collect();
    let targets = std::iter::once(None).chain(combatants.iter().copied().map(Some));
    for actor in targets {
        for id in crate::game::magic::enchantments::OFFENSIVE {
            let effect = match actor {
                None => world.enchantments.get(id).copied(),
                Some(actor) => world
                    .monsters
                    .monsters()
                    .get(actor)
                    .ok()
                    .filter(|(monster, ..)| !monster.is_dead())
                    .and_then(|(monster, ..)| monster.enchantments.get(id).copied()),
            };
            let Some(effect) = effect else { continue };
            let spell = &spells[id];
            let Some(realm) = spell.realm else { continue };
            let strength = Strength::at(0, spell, effect.power, effect.boost);
            let victims = actor.map_or_else(|| party.clone(), |a| vec![Victim::Monster(a)]);
            let mut landed = Vec::new();
            for victim in victims {
                if target(world, victim, realm, None).dead {
                    continue;
                }
                if id == SpellId::DEATH_CLOUD {
                    // TickCombatEffectSlots makes a death save directly: no
                    // duration roll or duration resistance reduction.
                    let immune = match victim {
                        Victim::Monster(actor) => {
                            let (body, ..) = world.monsters.monsters().get(actor).unwrap();
                            world.species()[body.species].death_immune
                        }
                        Victim::Member(_) => false,
                    };
                    let target = target(world, victim, realm, None);
                    let result = if immune
                        || saves(rng, world, victim, &target, DEATH_SAVE, strength.against)
                    {
                        Some(Effect::Resisted { victim })
                    } else {
                        kill(rng, world, victim)
                    };
                    landed.extend(result);
                } else {
                    let amount = magnitude(rng, spell.dice, strength);
                    let Some(amount) =
                        resisted_magnitude(rng, world, victim, realm, amount, strength.against)
                    else {
                        continue;
                    };
                    let hit = if amount == 0 {
                        Some(Effect::Resisted { victim })
                    } else {
                        hurt(rng, world, victim, amount)
                    };
                    if id == SpellId::DRAINING_CLOUD && amount > 0 {
                        match victim {
                            Victim::Member(member) => {
                                let (known, mut mana) = points.get_mut(member).unwrap();
                                let maximum = world.mana_maximum(member, known);
                                for realm in enumset::EnumSet::<Realm>::all() {
                                    mana.drain_realm(realm, amount, &maximum);
                                }
                            }
                            Victim::Monster(monster) => {
                                world.monsters.drain_magic(monster, amount / 2);
                            }
                        }
                        tire(world, victim, amount);
                    }
                    landed.extend(hit);
                    if id == SpellId::TOXIC_CLOUD {
                        let victim_stats = target(world, victim, realm, None);
                        if victim_stats.dead {
                            continue;
                        }
                        let turns = duration(rng, spell, strength);
                        let turns = resist(
                            rng,
                            turns,
                            victim_stats.resistance,
                            victim_stats.level,
                            strength.against,
                        );
                        if turns == 0 {
                            continue;
                        }
                        let lasting = Lasting::Rounds(turns.max(2).min(u32::from(u16::MAX)) as u16);
                        // One duration reduction, followed by three separate saves.
                        for condition in
                            [Condition::Nauseated, Condition::Poisoned, Condition::Unconscious]
                        {
                            let target = target(world, victim, realm, Some(condition));
                            if target.immune
                                || saves(
                                    rng,
                                    world,
                                    victim,
                                    &target,
                                    CONDITION_SAVE[condition as usize + 1],
                                    strength.against,
                                )
                                || shrugs(world, victim, condition, false)
                            {
                                landed.push(Effect::Resisted { victim });
                            } else if inflict_on(world, victim, (condition, 0), lasting).is_some() {
                                landed.push(Effect::Inflicted { victim, condition });
                            }
                        }
                    }
                }
            }
            reports.push((id, landed));
        }
        // Ring of Fire follows the party and samples current nearby actors on
        // every pulse, including actors that entered its radius after casting.
        if actor.is_none()
            && let Some(effect) = world.enchantments.get(SpellId::RING_OF_FIRE).copied()
            && let spell = &spells[SpellId::RING_OF_FIRE]
            && let Some(realm) = spell.realm
        {
            let centre = crate::game::party::body::PartyBody::centre(party_eye);
            let reach =
                crate::game::party::body::RADIUS + crate::content::species::Reach::Short.metres();
            let victims = crate::game::magic::targeting::radius(
                world,
                centre,
                centre,
                reach,
                None,
                crate::game::magic::targeting::Recipients::Opponents(Stance::Friendly),
            );
            let mut landed = Vec::new();
            for victim in victims {
                let amount = magnitude(
                    rng,
                    spell.dice,
                    Strength {
                        power: effect.power,
                        boost: 0,
                        against: u32::from(effect.power) * 3,
                    },
                );
                landed.extend(damage(
                    rng,
                    world,
                    victim,
                    realm,
                    amount,
                    u32::from(effect.power) * 3,
                ));
            }
            reports.push((spell.id, landed));
        }
    }
    reports
}
