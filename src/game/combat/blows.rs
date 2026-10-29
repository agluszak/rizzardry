//! Weapon blows in a fight, swing by swing: a member's attack with the hand
//! due, and a monster's (retail `StartCharacterAttack`,
//! `ResolveCharacterAttack`, `AnnounceMonsterAttack`,
//! `ContinueMonsterAttack`).
//!
//! Retail's voices on a miss, a kill or a fumble draw from the same random
//! numbers as the rules; here they draw nothing.

use bevy::prelude::*;

use super::attack::{self, Aim, Defence, Hand, Surprise};
use super::{
    Announcement, Armour, Battle, CombatNews, HandTurns, PartyPace, PartyView, RoundAction,
    RoundPhase, Turn, distance_to_party,
};
use crate::content::characters::{Skill, Trait};
use crate::content::items::{AttackEffect, AttackMode};
use crate::content::species::{Reach, Species};
use crate::game::characters::{Character, CharacterStats, Condition, Vitals};
use crate::game::magic::effects::{self, Effect, Field, Victim};
use crate::game::magic::{LearnedSpells, SpellPoints};
use crate::game::party::body;
use crate::game::party::formation::{self, Row};
use crate::game::rng::GameRng;
use crate::game::world::navigation::feet;

/// What a blow needs of the fight besides the field it changes.
pub(super) struct Fight<'a, 'w, 's> {
    pub combatants: &'a [Entity],
    pub party: &'a PartyView,
    pub armour: &'a dyn Fn(Entity) -> Option<Armour>,
    /// Monsters advancing this round, which drop their guard.
    pub advancing: &'a dyn Fn(Entity) -> bool,
    pub points: &'a mut Query<'w, 's, (&'static LearnedSpells, &'static mut SpellPoints)>,
    /// Whom the acting monster may attack (retail `BuildMonsterActionQueue`'s
    /// `MonsterVsCharDisposition` and `MonsterHostility` tests); members
    /// act on their own orders.
    pub attacks: &'a dyn Fn(Victim) -> bool,
}

/// Whoever a swing landed on, and how.
#[derive(Clone, Copy)]
struct Landing {
    victim: Victim,
    roll: u32,
    /// It struck whatever its aim: a roll of 5 or under, a fumble or a
    /// missile that flew true.
    sure: bool,
    /// It gets through armour whatever the roll.
    through: bool,
    fumbled: bool,
}

/// What one attack did to whom, reported once its swings are over.
#[derive(Default)]
struct Tally {
    hits: u32,
    damage: u32,
    /// Monsters killed by the party, with their experience.
    kills: Vec<(Entity, u32)>,
    deaths: Vec<Entity>,
    effects: Vec<Effect>,
    improved: Vec<(Entity, Skill)>,
}

impl Tally {
    fn struck(&mut self, struck: Option<Effect>, by_party: bool) -> u32 {
        let Some(Effect::Damaged { victim, outcome }) = struck else { return 0 };
        if outcome.died {
            match victim {
                Victim::Monster(monster) if by_party => {
                    self.kills.push((monster, outcome.experience));
                }
                Victim::Monster(e) | Victim::Member(e) => self.deaths.push(e),
            }
        }
        outcome.damage
    }

    fn report(
        self,
        battle: &mut Battle,
        (attacker, target): (Entity, Entity),
        news: &mut MessageWriter<CombatNews>,
    ) {
        // The summary, then what else the blows did, then the casualties
        // (retail `ReportCharacterAttackResult`).
        news.write(CombatNews::Result { attacker, target, hits: self.hits, damage: self.damage });
        let mut effects = self.effects;
        effects.retain(|e| !matches!(e, Effect::Damaged { .. }));
        if !effects.is_empty() {
            news.write(CombatNews::Effects(effects));
        }
        for (member, skill) in self.improved {
            news.write(CombatNews::Improved { member, skill });
        }
        for (monster, experience) in self.kills {
            battle.credit_kill(monster, experience, news);
        }
        for actor in self.deaths {
            news.write(CombatNews::Killed { actor });
        }
    }
}

/// Where a monster is struck: the middle of its body.
fn body_centre(feet: Vec3, monster: &crate::game::world::population::Monster) -> Vec3 {
    feet + Vec3::Y * monster.body.eye_height * 0.5
}

/// Whether a monster has its back to the party: facing away by 135° or
/// more (retail `IsPartyLookingAwayFrom`).
fn back_to_party(forward: Vec3, at: Vec3, party_eye: Vec3) -> bool {
    let to_party = (party_eye - at).xz();
    let forward = forward.xz();
    to_party != Vec2::ZERO
        && forward != Vec2::ZERO
        && forward.angle_to(to_party).abs() >= std::f32::consts::FRAC_PI_4 * 3.0
}

/// A trait's chance by monster level (retail `ScaleValueByMonsterLevel`).
fn by_monster_level(level: u8, value: f32) -> f32 {
    if level > 20 { value } else { (f32::from(level) * 2.0 + 60.0) * value * 0.01 }
}

/// How a member stands this round, for the blows aimed at them.
fn member_defence(
    battle: &Battle,
    field: &Field,
    fight: &Fight,
    member: Entity,
) -> Option<(Character, Defence<'static>)> {
    let (character, vitals, ..) = field.members.get(member).ok()?;
    let armour = (fight.armour)(member)?;
    let running = battle.this_round.pace == Some(PartyPace::Run);
    let defending = battle
        .this_round
        .party
        .iter()
        .any(|p| p.actor == member && matches!(p.action, RoundAction::Defend));
    let stats = character.stats(&vitals.enchantments);
    Some((character.clone(), Defence::member(&stats, armour, running, defending)))
}

fn monster_defence<'a>(
    field: &Field<'a, '_, '_, '_, '_, '_, '_>,
    fight: &Fight,
    monster: Entity,
) -> Option<Defence<'a>> {
    let species = field.species;
    let (body, ..) = field.monsters.monsters().get(monster).ok()?;
    if body.is_dead() || !field.monsters.active(monster) {
        return None;
    }
    let kind = species.kind(body.species)?;
    Some(Defence::monster(kind, body, (fight.advancing)(monster)))
}

/// Who a member's swing could go astray onto (retail
/// `BuildCharacterTargetList`): themselves, the members their hand reaches,
/// and the monsters it reaches but the one aimed at.
fn member_bystanders(
    field: &Field,
    fight: &Fight,
    member: Entity,
    hand: &Hand,
    target: Entity,
) -> Vec<Victim> {
    let formation = &**field.formation;
    let mut bystanders: Vec<Victim> = field
        .party
        .iter()
        .filter(|&&m| field.members.get(m).is_ok_and(|(_, v, ..)| !v.is_dead()))
        .filter(|&&m| m == member || hand.reach > Reach::Touch || !formation.screens(member, m))
        .map(|&m| Victim::Member(m))
        .collect();
    for &entity in fight.combatants {
        let Ok((monster, at, walker, _)) = field.monsters.monsters().get(entity) else { continue };
        let distance = distance_to_party(fight.party.eye, monster, feet(at.translation(), walker));
        if entity != target
            && !monster.is_dead()
            && field.monsters.active(entity)
            && fight.party.reaches(formation, member, at.translation(), hand.reach, distance)
        {
            bystanders.push(Victim::Monster(entity));
        }
    }
    bystanders
}

/// Who a monster's swing could go astray onto (retail
/// `BuildMonsterTargetList`): the members its attack reaches but the one
/// aimed at, itself, and the monsters within its reach.
fn monster_bystanders(
    field: &Field,
    fight: &Fight,
    attacker: Entity,
    reach: Reach,
    target: Entity,
) -> Vec<Victim> {
    let monsters = field.monsters.monsters();
    let Ok((monster, at, walker, _)) = monsters.get(attacker) else { return Vec::new() };
    let here = feet(at.translation(), walker);
    let distance = distance_to_party(fight.party.eye, monster, here);
    let formation = &**field.formation;
    let mut bystanders: Vec<Victim> = field
        .party
        .iter()
        .filter(|&&m| m != target)
        .filter(|&&m| field.members.get(m).is_ok_and(|(_, v, ..)| !v.is_dead()))
        .filter(|&&m| fight.party.reaches(formation, m, at.translation(), reach, distance))
        .map(|&m| Victim::Member(m))
        .collect();
    for &entity in fight.combatants {
        let Ok((other, other_at, other_walker, _)) = monsters.get(entity) else { continue };
        if other.is_dead() || !field.monsters.active(entity) {
            continue;
        }
        let gap = here.distance(feet(other_at.translation(), other_walker))
            - monster.body.radius
            - other.body.radius;
        if entity != target && (entity == attacker || gap.max(0.0) <= reach.metres()) {
            bystanders.push(Victim::Monster(entity));
        }
    }
    bystanders
}

/// A Berserk order needs the trait, a hand that can attack and a main hand
/// for close quarters (retail `CanCharacterBerserk`).
pub(super) fn can_berserk(character: &Character, hands: &attack::Hands) -> bool {
    let [main, off] = &hands.hands;
    character.has_trait(Trait::Berserk)
        && (main.in_play || off.in_play)
        && (!main.in_play || main.reach <= Reach::Short)
}

/// Retail `ChooseMonsterTarget`: among the monsters a hand reaches, the
/// least afflicted, then those "in reach" of the party, then the nearest;
/// ties keep the fight's order.
pub(super) fn melee_target<'a>(
    member: Entity,
    hand: &Hand,
    field: &Field<'a, '_, '_, '_, '_, '_, '_>,
    fight: &Fight,
) -> Option<(Entity, &'a Species)> {
    let formation = &**field.formation;
    let species = field.species;
    fight
        .combatants
        .iter()
        .filter_map(|&entity| {
            let (monster, at, walker, _) = field.monsters.monsters().get(entity).ok()?;
            let kind = species.kind(monster.species).filter(|kind| kind.npc.is_none())?;
            let distance =
                distance_to_party(fight.party.eye, monster, feet(at.translation(), walker));
            if !field.monsters.active(entity)
                || monster.is_dead()
                || !fight.party.reaches(formation, member, at.translation(), hand.reach, distance)
            {
                return None;
            }
            let worst = monster.conditions.iter().map(|&(c, _)| c).max();
            let side = formation::quadrant(fight.party.eye, fight.party.yaw, at.translation());
            let in_reach = !kind.attacks.is_empty()
                && (side == Row::Rear
                    || (!formation.occupied(side) && formation.occupied(Row::Center)));
            Some(((worst, !in_reach, distance), (entity, kind)))
        })
        .min_by(|a, b| a.0.0.cmp(&b.0.0).then(a.0.1.cmp(&b.0.1)).then(a.0.2.total_cmp(&b.0.2)))
        .map(|(_, target)| target)
}

/// One of a member's attacks with the hand due (retail
/// `StartCharacterAttack` and its swings): the best target the hand
/// reaches, its mode, its swings, each rolled to hit, to penetrate and for
/// damage and effects, then the hand due next and when.
pub(super) fn member_attacks(
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
    let (items, difficulty) = (field.items, field.difficulty);
    let Ok((character, vitals, carried, burden)) = field.members.get(member) else { return };
    let (character, load) = (character.clone(), burden.load);
    let slowed = vitals.has(Condition::Slowed);
    let mut hands = attack::hands(&character.stats(&vitals.enchantments), carried, items, load);
    // The party's blessings guide every hand.
    for hand in &mut hands.hands {
        hand.to_hit += field.enchantments.hit_bonus();
    }
    let mut turns = work.hands;
    // The hand due attacks if it still can; a hand that cannot (out of
    // shots, or nothing in its reach) gives up only its own attacks, and
    // the other hand takes over (retail `StartCharacterAttack`,
    // `ComputeCharacterActionPhase`).
    let mut due = None;
    for current in [turns.current, 1 - turns.current] {
        let hand = &hands.hands[current];
        let target = (hand.in_play && turns.left[current] > 0)
            .then(|| melee_target(member, hand, field, fight))
            .flatten();
        match target {
            Some((target, _)) => {
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
    let hand = hands.hands[current].clone();
    battle.this_round.attacked = true;
    // Retail draws the attack's mode, then its swings.
    let mode = if berserk { AttackMode::Berserk } else { attack::choose_mode(rng, hand.modes) };
    turns.left[current] -= 1;
    let mut swings = rng.below(hand.swings) + 1;
    // A Rogue's sword or dagger at arm's length strikes where it hurts.
    let backstab = character.has_trait(Trait::Backstab)
        && hand.reach <= Reach::Touch
        && matches!(hand.weapon_skill, Skill::Sword | Skill::Dagger);
    swings = (swings as i32 + hand.extra_swings).max(1) as u32;
    // A Samurai's Lightning Strike, readied as the round began.
    if turns.lightning && current == 0 {
        if hands.hands[0].weapon_skill == Skill::Sword && !slowed {
            let level = u32::from(character.profession_levels[character.profession]);
            swings += if rng.below(11) + level < 20 { 3 } else { 4 };
        }
        turns.lightning = false;
    }
    let mut surprise = Surprise { unaware: false, natural: backstab };
    if hand.reach <= Reach::Short
        && let Ok((_, at, ..)) = field.monsters.monsters().get(target)
    {
        surprise.unaware = back_to_party(at.forward(), at.translation(), fight.party.eye);
    }
    news.write(CombatNews::Attack {
        attacker: member,
        target,
        how: Announcement::Mode(mode),
        swings,
        surprise,
    });
    // Attacking a monster provokes its group, whatever the blows do
    // (retail `MakeTargetGroupHostile` as the action starts).
    field.provoke(target);
    let backstab_chance = character.scale_by_profession(100.0) as i32;
    let mut tally = Tally::default();
    let mut left = swings;
    while left > 0 {
        left -= 1;
        let Ok((_, vitals, ..)) = field.members.get(member) else { break };
        let vitals: Vitals = vitals.clone();
        if !vitals.can_fight() {
            break;
        }
        let stats = character.stats(&vitals.enchantments);
        let Some(aimed_at) = monster_defence(field, fight, target) else { break };
        let aim =
            attack::member_aim(&stats, &vitals, &hand, mode, surprise, Some(&aimed_at), difficulty);
        let landing = if hand.reach >= Reach::Long {
            let Ok((monster, at, walker, _)) = field.monsters.monsters().get(target) else { break };
            let centre = body_centre(feet(at.translation(), walker), monster);
            let shot =
                attack::scatter(rng, fight.party.eye, centre, aim, vitals.has(Condition::Blind));
            let half = monster.body.eye_height * 0.5;
            attack::strikes(centre, shot, monster.body.radius, half).then(|| Landing {
                victim: Victim::Monster(target),
                roll: rng.below(100) + 1,
                sure: true,
                through: attack::sure_penetration(&stats, &hand),
                fumbled: false,
            })
        } else {
            let bystanders = member_bystanders(field, fight, member, &hand, target);
            let odds = attack::member_aim(&stats, &vitals, &hand, mode, surprise, None, difficulty);
            let fumble = attack::fumble_chance(bystanders.len(), odds);
            match attack::swing(rng, bystanders.len(), fumble) {
                Aim::Missed => None,
                Aim::Reaches { roll, sure } => Some(Landing {
                    victim: Victim::Monster(target),
                    roll,
                    sure,
                    through: sure,
                    fumbled: false,
                }),
                Aim::Fumbled { bystander, roll } => {
                    surprise = Surprise::default();
                    Some(Landing {
                        victim: bystanders[bystander],
                        roll,
                        sure: true,
                        through: false,
                        fumbled: true,
                    })
                }
            }
        };
        if let Some(landing) = landing {
            if landing.fumbled {
                news.write(CombatNews::Fumble { attacker: member, victim: landing.victim });
            }
            let dealt = member_blow(
                battle,
                (member, &stats, &vitals),
                (&hands, current, mode),
                (surprise, aim, backstab_chance),
                landing,
                (rng, field, fight),
                &mut tally,
            );
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
        let Ok((_, mut vitals, mut carried, _)) = field.members.get_mut(member) else { break };
        let weight = hand.weight;
        let cost =
            attack::swing_stamina(rng, mode, weight, berserk, vitals.has(Condition::Diseased));
        let hasted = vitals.enchantments.get(crate::content::spells::SpellId::HASTE).is_some();
        vitals.spend_stamina(load.scale_fatigue(cost, slowed, hasted));
        // Arrows, bolts and stones are shot away, thrown weapons thrown,
        // and guns spend their shots.
        // A hand with nothing left to fire is done for the round.
        if (hand.paired || hand.ranged) && !spend_shot(&mut carried, items, current, &hand) {
            left = 0;
            turns.left[current] = 0;
        }
    }
    tally.report(battle, (member, target), news);
    next_hand(battle, index, step, turns, &hands);
}

/// Spend what a ranged swing used (retail `ResolveCharacterAttack`'s
/// tail): the ammunition a launcher shot, a thrown weapon unless it never
/// runs out, or a gun's shot. Returns whether the hand can go on.
fn spend_shot(
    carried: &mut crate::game::inventory::Carried,
    items: &crate::content::items::ItemCatalog,
    current: usize,
    hand: &Hand,
) -> bool {
    use crate::game::inventory::EquipSlot;
    let slots = [EquipSlot::Primary, EquipSlot::Secondary];
    let spent = if hand.paired { slots[1 - current] } else { slots[current] };
    let Some(stack) = carried.worn(spent) else { return false };
    let Some(definition) = items.get(stack.item) else { return false };
    if !hand.paired && definition.spell.is_some() {
        return true;
    }
    let thrown = definition.class == crate::content::items::EquipClass::ThrownWeapon;
    if !hand.paired && thrown && definition.never_depletes {
        return true;
    }
    carried.consume_use(stack.id, definition, definition.keeps_empty());
    carried.worn(spent).is_some_and(|s| s.remaining_uses() != Some(0))
}

/// One member swing that struck someone: its penetration, damage and
/// effects. Returns the damage dealt when it hit, `None` when it was
/// dodged.
fn member_blow(
    battle: &Battle,
    (attacker, character, vitals): (Entity, &CharacterStats, &Vitals),
    (hands, current, mode): (&attack::Hands, usize, AttackMode),
    (surprise, aim, backstab): (Surprise, i32, i32),
    landing: Landing,
    (rng, field, fight): (&mut GameRng, &mut Field, &mut Fight),
    tally: &mut Tally,
) -> Option<u32> {
    let hand = &hands.hands[current];
    let difficulty = field.difficulty;
    let (struck, victim_character) = match landing.victim {
        Victim::Monster(monster) => (monster_defence(field, fight, monster)?, None),
        Victim::Member(m) => {
            let (c, defence) = member_defence(battle, field, fight, m)?;
            (defence, Some((m, c)))
        }
    };
    let at = struck.locate(rng, None);
    if !landing.sure && aim < landing.roll as i32 {
        return None;
    }
    // A launcher pierces by what it shoots.
    let piercing =
        if hand.paired { attack::choose_mode(rng, hands.hands[1 - current].modes) } else { mode };
    let through = attack::member_penetration(
        character,
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
    let slaying = struck.species().map_or(0, |kind| {
        hand.slays.iter().flatten().filter(|&&slays| slays == kind.kind).count() as u32
    });
    let count = attack::dice_count(rng, mode, &struck, surprise, backstab, slaying);
    let dice = match mode {
        AttackMode::Punch => hands.hands[1].dice,
        AttackMode::Kick => hands.hands[0].dice,
        _ => hand.dice,
    };
    let rolled = attack::roll_dice(rng, dice, count);
    let mut damage = attack::member_damage(hand, rolled, &struck);
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
        let mut blow = hand.effects;
        if !landing.fumbled {
            let kill = &mut blow.chances[AttackEffect::Kill];
            *kill = kill.wrapping_add(attack::critical_chance(rng, character, hand));
            if character.has_trait(Trait::Knockout) {
                let knockout = &mut blow.chances[AttackEffect::KnockOut];
                *knockout = knockout.wrapping_add(character.scale_by_profession(5.0) as u8);
            }
        }
        let power = attack::effect_power(u32::from(character.level));
        let landed = effects::blow(rng, field, fight.points, landing.victim, &blow, power, false);
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
        let skills: &[Skill] = if !hand.ranged {
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

/// A member struck practises Iron Skin as their reduction is worked out
/// (retail `ApplyCharacterDamageReduction`).
fn iron_skin(
    rng: &mut GameRng,
    field: &mut Field,
    member: Entity,
    character: &Character,
    tally: &mut Tally,
) {
    if character.skill_available(Skill::IronSkin)
        && let Ok((mut c, ..)) = field.members.get_mut(member)
        && c.practise(Skill::IronSkin, 1, rng)
    {
        tally.improved.push((member, Skill::IronSkin));
    }
}

/// The hand due next and when (retail `ComputeCharacterActionPhase`): the
/// other hand while it has attacks, unless it has fewer and less
/// initiative; spread over the rest of the round, and a switch costs the
/// difference in the hands' initiative.
fn next_hand(battle: &mut Battle, index: usize, step: u8, turns: HandTurns, hands: &attack::Hands) {
    let current = turns.current;
    let other = 1 - current;
    let (now, then) = (turns.left[current], turns.left[other]);
    let initiative = |hand: usize| hands.hands[hand].initiative;
    let chosen = if then != 0 && !(then < now && initiative(other) < initiative(current)) {
        other
    } else {
        current
    };
    let left = turns.left[chosen];
    let phase = if left == 0 {
        None
    } else if chosen == current || then <= now {
        Some(u32::from(step) + (100 - u32::from(step)) / (left + 1))
    } else {
        Some(u32::from(step))
    };
    let work = &mut battle.this_round.party[index];
    work.hands = turns;
    match phase.filter(|&phase| phase <= 100) {
        Some(phase) => {
            let phase = phase as i32
                + if chosen != current { initiative(current) - initiative(chosen) } else { 0 };
            work.hands.current = chosen;
            work.phase = phase.clamp(i32::from(step), 100) as u8;
        }
        None => work.end(),
    }
}

/// Whether a member notices a monster coming at them in time to turn
/// (retail `CharacterNoticesAttacker`): only one awake, unblinded and
/// defending; the first time each round always, then by senses, 25 harder
/// each time.
fn notices(
    battle: &mut Battle,
    member: Entity,
    character: &Character,
    vitals: &Vitals,
    rng: &mut GameRng,
) -> bool {
    let alert = !vitals.is_dead()
        && vitals.stamina > 0
        && vitals.worst_condition().is_none_or(|c| c < Condition::Webbed)
        && !vitals.has(Condition::Blind);
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
    let senses = character.attribute(crate::content::characters::Attribute::Senses) as i32;
    let chance = (senses - *attempts as i32 * 25) as u32;
    *attempts += 1;
    rng.below(100) < chance
}

/// A monster's attack (retail `StartMonsterAttack` and its swings): one of
/// its attacks and modes at a member or a fellow monster it may attack and
/// reaches, all equally likely, its swings each rolled to hit, to penetrate
/// and for damage and effects. Returns whether it had nobody to attack.
pub(super) fn monster_attacks(
    battle: &mut Battle,
    index: usize,
    rng: &mut GameRng,
    field: &mut Field,
    fight: &mut Fight,
    news: &mut MessageWriter<CombatNews>,
) -> bool {
    let (entity, turn) = battle.this_round.monsters[index];
    let species = field.species;
    let difficulty = field.difficulty;
    let Ok((monster, at, walker, _)) = field.monsters.monsters().get(entity) else { return false };
    let Some(kind) = species.kind(monster.species) else { return false };
    let distance = distance_to_party(fight.party.eye, monster, feet(at.translation(), walker));
    let position = at.translation();
    let eye = feet(position, walker) + Vec3::Y * monster.body.eye_height;
    let formation = &**field.formation;
    let here = feet(position, walker);
    // Every (attack, mode, target) that reaches is equally likely: members,
    // then the other monsters in the fight.
    let mut candidates = vec![];
    for (a, attack) in kind.attacks.iter().enumerate() {
        for &member in field.party {
            let Ok((_, vitals, ..)) = field.members.get(member) else { continue };
            if vitals.can_fight()
                && (fight.attacks)(Victim::Member(member))
                && fight.party.reaches(formation, member, position, attack.reach, distance)
            {
                for mode in attack.modes {
                    candidates.push((a, mode, Victim::Member(member)));
                }
            }
        }
        for &other in fight.combatants {
            let Ok((body, other_at, other_walker, _)) = field.monsters.monsters().get(other) else {
                continue;
            };
            let gap = here.distance(feet(other_at.translation(), other_walker))
                - monster.body.radius
                - body.body.radius;
            if other != entity
                && !body.is_dead()
                && field.monsters.active(other)
                && (fight.attacks)(Victim::Monster(other))
                && gap.max(0.0) <= attack.reach.metres()
            {
                for mode in attack.modes {
                    candidates.push((a, mode, Victim::Monster(other)));
                }
            }
        }
    }
    let turn_left = turn.attacks_left.saturating_sub(1);
    battle.this_round.monsters[index].1 = Turn { phase: 0, attacks_left: 0 };
    if candidates.is_empty() {
        return true;
    }
    battle.this_round.attacked = true;
    let (a, mode, target) = candidates[rng.below(candidates.len() as u32) as usize];
    let attack = &kind.attacks[a];
    // Retail draws for the mode again, from just the one chosen.
    let mode = attack::choose_mode(rng, enumset::EnumSet::only(mode));
    let swings = rng.below(u32::from(kind.swings_per_round.max(1))) + 1;
    // A natural ambusher strikes from hiding at arm's length.
    let mut surprise = Surprise {
        unaware: false,
        natural: kind.kind == crate::content::species::MonsterKind::Rogue
            && attack.reach <= Reach::Touch,
    };
    // A member not facing it may turn in time, or be struck from behind.
    if attack.reach <= Reach::Short
        && let Victim::Member(member) = target
        && let Ok((character, vitals, ..)) = field.members.get(member)
    {
        let side = formation::quadrant(fight.party.eye, fight.party.yaw, position);
        let facing = field.formation.facing(member);
        if facing.is_some_and(|f| f != side) {
            let (character, vitals) = (character.clone(), vitals.clone());
            if notices(battle, member, &character, &vitals, rng) {
                field.formation.turn(member, side);
            } else {
                surprise.unaware = facing.is_some_and(|f| opposite(f) == Some(side));
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
    let backstab = by_monster_level(kind.level, 100.0) as i32;
    let mut tally = Tally::default();
    for _ in 0..swings {
        let Ok((monster, ..)) = field.monsters.monsters().get(entity) else { break };
        let monster = monster.clone();
        // A fumble may incapacitate the attacker between its swings.
        if !monster.can_fight() {
            break;
        }
        let aimed_at = match target {
            Victim::Member(member) => member_defence(battle, field, fight, member)
                .filter(|_| field.members.get(member).is_ok_and(|(_, v, ..)| !v.is_dead()))
                .map(|(_, defence)| defence),
            Victim::Monster(other) => monster_defence(field, fight, other),
        };
        let Some(aimed_at) = aimed_at else { break };
        let aim = attack::monster_aim(
            (kind, &monster),
            attack,
            mode,
            surprise,
            Some(&aimed_at),
            difficulty,
        );
        let landing = if attack.reach >= Reach::Long {
            let blind = monster.conditions.iter().any(|&(c, _)| c == Condition::Blind);
            // At the party's eye, or a monster's middle.
            let (aimed, radius, half_height) = match target {
                Victim::Member(_) => (fight.party.eye, body::RADIUS, body::RADIUS),
                Victim::Monster(other) => {
                    let Ok((body, at, walker, _)) = field.monsters.monsters().get(other) else {
                        break;
                    };
                    let centre = body_centre(feet(at.translation(), walker), body);
                    (centre, body.body.radius, body.body.eye_height * 0.5)
                }
            };
            let shot = attack::scatter(rng, eye, aimed, aim, blind);
            attack::strikes(aimed, shot, radius, half_height).then(|| Landing {
                victim: target,
                roll: rng.below(100) + 1,
                sure: true,
                through: false,
                fumbled: false,
            })
        } else {
            let bystanders =
                monster_bystanders(field, fight, entity, attack.reach, target.entity());
            let odds =
                attack::monster_aim((kind, &monster), attack, mode, surprise, None, difficulty);
            let fumble = attack::fumble_chance(bystanders.len(), odds);
            match attack::swing(rng, bystanders.len(), fumble) {
                Aim::Missed => None,
                Aim::Reaches { roll, sure } => {
                    Some(Landing { victim: target, roll, sure, through: sure, fumbled: false })
                }
                Aim::Fumbled { bystander, roll } => {
                    surprise = Surprise::default();
                    Some(Landing {
                        victim: bystanders[bystander],
                        roll,
                        sure: true,
                        through: false,
                        fumbled: true,
                    })
                }
            }
        };
        let Some(landing) = landing else { continue };
        if landing.fumbled {
            news.write(CombatNews::Fumble { attacker: entity, victim: landing.victim });
        }
        let (struck, victim_character) = match landing.victim {
            Victim::Member(m) => {
                let Some((c, defence)) = member_defence(battle, field, fight, m) else {
                    continue;
                };
                (defence, Some((m, c)))
            }
            Victim::Monster(m) => {
                let Some(defence) = monster_defence(field, fight, m) else { continue };
                (defence, None)
            }
        };
        let at = struck.locate(rng, victim_character.as_ref().map(|_| kind));
        if !landing.sure && aim < landing.roll as i32 {
            continue;
        }
        // The party's Missile Shield turns aside what is shot or thrown.
        if matches!(landing.victim, Victim::Member(_)) && field.enchantments.deflects(mode, rng) {
            continue;
        }
        let through = attack::monster_penetration(
            (kind, &monster),
            attack,
            mode,
            surprise,
            (&struck, at),
            difficulty,
        );
        let dealt = if landing.through || through >= landing.roll as i32 {
            let count = attack::dice_count(rng, mode, &struck, surprise, backstab, 0);
            let rolled = attack::roll_dice(rng, attack.damage, count);
            let mut damage = attack::monster_damage(rolled, &struck);
            if let Some((victim, c)) = &victim_character {
                iron_skin(rng, field, *victim, c, &mut tally);
            }
            if damage > 0 && landing.fumbled {
                damage = struck.fumble_cap(damage);
            }
            let dealt = if damage > 0 {
                tally.struck(effects::hurt(rng, field, landing.victim, damage), false)
            } else {
                0
            };
            if dealt > 0 {
                let power = attack::effect_power(u32::from(kind.level));
                let landed = effects::blow(
                    rng,
                    field,
                    fight.points,
                    landing.victim,
                    &attack.effects,
                    power,
                    true,
                );
                for effect in &landed {
                    tally.struck(Some(*effect), false);
                }
                tally.effects.extend(landed);
            }
            dealt
        } else {
            0
        };
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
    battle.this_round.monsters[index].1 =
        Turn { phase: super::next_phase(turn.phase, turn_left), attacks_left: turn_left };
    false
}

fn opposite(side: Row) -> Option<Row> {
    match side {
        Row::Front => Some(Row::Rear),
        Row::Rear => Some(Row::Front),
        Row::Right => Some(Row::Left),
        Row::Left => Some(Row::Right),
        Row::Center => None,
    }
}
