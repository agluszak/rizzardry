//! A round's end (retail `EndCombatRound`, `AdvanceCombatRound`,
//! `AdvanceTimedEffects`). Once everyone has acted, nothing is in flight
//! and the last pause is over, [`judge_round`] passes a round's game time
//! and marks the fight [`RoundEnding`]; then, each its own system: groups
//! with nothing left to fight leave, the round's rest, cooldowns, aging and
//! recovery, the offensive clouds' pulses, spells aging, the defenders'
//! rest, whether the fight is over, the party looking again at who it
//! fights, and the end of the fight.

use avian3d::prelude::Position;
use bevy::prelude::*;

use super::approach::{self, CombatMove};
use super::sight::Sight;
use super::turn::{Ordered, SpecialCooldown, Turn};
use super::{
    Battle, CombatNews, Combatant, Combatants, LAST_MOMENT, PASSIVE_ROUNDS, PartyMove, Pause,
    ROUND_SECONDS, Round, Spoils,
};
use crate::content::spells::SpellId;
use crate::game::actors::Laid;
use crate::game::actors::Places;
use crate::game::actors::spells::{Enchantment, LaidOn, OFFENSIVE, Pulses};
use crate::game::characters::Vitals;
use crate::game::characters::{Character, Condition};
use crate::game::data::Catalogs;
use crate::game::hits::Flight;
use crate::game::hits::definitions::Definitions;
use crate::game::hits::effects::Strike;
use crate::game::hits::reach::{Recipients, within};
use crate::game::hits::{Act, Deed, Release};
use crate::game::magic::Strength;
use crate::game::party::body::PartyBody;
use crate::game::party::recovery;
use crate::game::party::roster::{PartyMember, PartyRoster};
use crate::game::rng::GameRng;
use crate::game::time::GameClock;
use crate::game::world::hostility::{Side, Stance};
use crate::game::world::identify::{GroupMember, GroupMembers, GroupRecognition};
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::{Monster, Temper, Untargetable};
use bevy::ecs::relationship::RelationshipTarget;

/// The round is ending, a round's game time having passed.
#[derive(Component, Debug)]
pub struct RoundEnding;

/// The monster's group leaves the fight as the round ends.
#[derive(Component, Debug)]
pub struct Leaves;

/// The fight is over as the round ends.
#[derive(Component, Debug)]
pub struct FightOver;

/// Once everyone has acted, the party's move is done, the last action's
/// pause is over and nothing flies, walks or is being cast, the round ends:
/// a round's game time passes, one aging tick.
#[allow(clippy::type_complexity)]
pub fn judge_round(
    mut commands: Commands,
    battle: Option<
        Single<(Entity, &Round, &PartyMove, &Pause), (With<Battle>, Without<RoundEnding>)>,
    >,
    busy: (Query<(), With<Flight>>, Query<(), With<CombatMove>>),
    mut clock: ResMut<GameClock>,
    mut elapsed: MessageWriter<recovery::Elapsed>,
) {
    let Some((fight, round, party_move, pause)) = battle.map(Single::into_inner) else { return };
    if round.moment <= LAST_MOMENT
        || matches!(party_move, PartyMove::Moving { .. })
        || !pause.0.is_finished()
        || !busy.0.is_empty()
        || !busy.1.is_empty()
    {
        return;
    }
    let ticks = clock.advance(crate::game::time::AGING_TICK_MS);
    elapsed.write(recovery::Elapsed { ticks, pace: recovery::Pace::Fighting });
    commands.entity(fight).insert(RoundEnding);
}

/// Groups with nothing left to fight leave it (retail
/// `CheckMonsterGroupsLeaveCombat`): a neutral group once it no longer
/// stands with the fight; a friendly group unless one of its members sees
/// an enemy; a hostile group unless a living member sees the party. When no
/// hostile group is left engaged, everyone leaves. One outside any group
/// stays while hostile groups fight on.
pub(super) fn leave_fight(
    mut commands: Commands,
    fight: Single<&Combatants, With<RoundEnding>>,
    fighters: Query<(&Temper, Option<&GroupMember>), With<Monster>>,
    groups: Query<&GroupMembers>,
    sight: Sight,
) {
    let combatants = fight.entities();
    let fighting = |monster: Entity| combatants.contains(&monster) && sight.alive(monster);
    let hostile = |monster: Entity| sight.side(monster) == Some(Stance::Hostile);
    let group_of = |monster: Entity| fighters.get(monster).ok()?.1.map(|group| group.0);
    let hostiles: Vec<Entity> = combatants
        .iter()
        .copied()
        .filter(|&c| sight.alive(c) && sight.in_party_view(c) && hostile(c))
        .collect();
    // A monster outside any group keeps hostile groups engaged.
    let mut engaged = combatants
        .iter()
        .any(|&c| fighters.contains(c) && group_of(c).is_none() && sight.alive(c) && hostile(c));
    let mut fought: Vec<Entity> = Vec::new();
    for group in combatants.iter().filter_map(|&c| group_of(c)) {
        if !fought.contains(&group) {
            fought.push(group);
        }
    }
    let mut leaving = Vec::new();
    for group in fought {
        let Ok(members) = groups.get(group) else { continue };
        let fellows: Vec<Entity> = members.iter().filter(|&m| fighting(m)).collect();
        let side = members.iter().find(|m| combatants.contains(m)).and_then(|m| sight.side(m));
        let stays = match side {
            Some(Stance::Neutral) => {
                let plain = |&m: &Entity| fighters.get(m).is_ok_and(|(t, _)| !t.0.camouflaged);
                let standing: Vec<Entity> = fellows.iter().copied().filter(plain).collect();
                sight.reinforces(&standing, &hostiles)
            }
            Some(Stance::Hostile) => {
                let sees = fellows.iter().any(|&m| sight.sees_party(m));
                engaged |= sees;
                sees
            }
            Some(Stance::Friendly) => fellows.iter().any(|&m| sight.sees_foe(m, combatants)),
            None => continue,
        };
        if !stays {
            leaving.push(group);
        }
    }
    for &combatant in combatants {
        if !engaged || group_of(combatant).is_some_and(|g| leaving.contains(&g)) {
            commands.entity(combatant).insert(Leaves);
        }
    }
}

/// Whoever held back this round (a body that waited, a member who
/// defended), or lies asleep or knocked out, gets its breath back (retail
/// `AdvanceCombatRound`); a Fighter's sheet scales it.
pub(super) fn rest_the_idle(
    fight: Single<&Combatants, With<RoundEnding>>,
    mut bodies: Query<(&mut Vitals, Option<&Turn>, Option<&Ordered>, Option<&Character>)>,
    mut rng: ResMut<GameRng>,
) {
    for &body in fight.entities() {
        let Ok((mut vitals, turn, ordered, character)) = bodies.get_mut(body) else { continue };
        let resting = turn.is_some_and(|turn| turn.waited)
            || ordered.is_some_and(Ordered::defends)
            || matches!(
                vitals.conditions.worst(),
                Some(Condition::Asleep | Condition::Unconscious)
            );
        if vitals.is_dead() || !resting {
            continue;
        }
        let maximum = vitals.stamina.max;
        let rolled = recovery::rest_roll(maximum, &mut rng);
        let gained =
            character.map_or(rolled, |character| recovery::combat_rest_amount(character, rolled));
        if vitals.stamina.current < maximum || vitals.conditions.exhausted() {
            vitals.rest(gained);
        }
    }
}

/// A special attack cools down over the rounds the monster does not make
/// one (retail `AdvanceCombatRound`).
pub(super) fn cool_specials(
    fight: Single<&Combatants, With<RoundEnding>>,
    mut monsters: Query<(&mut SpecialCooldown, Option<&Turn>)>,
) {
    for &monster in fight.entities() {
        if let Ok((mut cooldown, turn)) = monsters.get_mut(monster)
            && cooldown.0 > 0
            && !turn.is_some_and(|turn| turn.special)
        {
            cooldown.0 -= 1;
        }
    }
}

/// Each pulsing spell laid pulses on whoever it hangs over (retail
/// `AdvanceTimedEffects`; a spell's last turn still pulses): one on the
/// party on its living members, one on a monster on it, and Ring of Fire
/// on the monsters close around the party, sampled anew each pulse. A pulse
/// is a deed with no caster. A character without a seat is not among those
/// close around the party.
pub(super) fn pulse_clouds(
    mut commands: Commands,
    fight: Single<&Combatants, With<RoundEnding>>,
    pulsing: Query<(&Enchantment, &LaidOn), With<Pulses>>,
    bodies: Query<
        (Entity, &Vitals, &Side),
        (Without<Untargetable>, Or<(With<PartyMember>, Without<Character>)>),
    >,
    places: Places,
    party: Single<(Entity, &Position, &PartyRoster)>,
    obstacles: WorldObstacles,
    content: Res<Catalogs>,
    definitions: Res<Definitions>,
) {
    let (party, position, roster) = *party;
    let alive = |body: Entity| bodies.get(body).is_ok_and(|(_, vitals, ..)| !vitals.is_dead());
    // The party's first, then each monster's, in the order clouds pulse.
    let mut order: Vec<_> = pulsing.iter().collect();
    order.sort_by_key(|(laid, on)| {
        (on.0 != party, OFFENSIVE.iter().position(|&cloud| cloud == laid.spell))
    });
    for (laid, &LaidOn(bearer)) in order {
        let Some(definition) = definitions.pulse(laid.spell) else { continue };
        let ring = laid.spell == SpellId::RING_OF_FIRE;
        let victims: Vec<Entity> = if ring {
            let centre = PartyBody::centre(PartyBody::eye(position));
            let reach =
                crate::game::party::body::RADIUS + crate::content::species::Reach::Short.metres();
            within(
                centre,
                centre,
                reach,
                Recipients::Opponents(Stance::Friendly),
                bodies.iter().map(|(entity, vitals, side)| {
                    (entity, vitals, side.now(vitals), places.of(entity))
                }),
                (roster, fight.entities()),
                &obstacles,
            )
        } else if bearer == party {
            roster.iter().filter(|&member| alive(member)).collect()
        } else {
            fight.entities().iter().copied().filter(|&m| m == bearer && alive(m)).collect()
        };
        let strength = if ring {
            Strength { power: laid.power, boost: 0, against: laid.power * 3 }
        } else {
            Strength::at(0, &content.spells[laid.spell], laid.power, laid.boost)
        };
        commands.trigger(Release {
            deed: Deed::new(
                Act::Triggered { spell: laid.spell },
                Strike { definition, strength, blow: None },
            ),
            victims: victims.into_iter().map(|victim| (victim, None)).collect(),
        });
    }
}

/// The fight is over when no hostile stays in it, or nobody attacked or
/// closed in for two rounds; the living, the unconscious included, share
/// its experience. Poison may have killed the last of the party as the
/// round passed: that is their defeat, not a victory.
#[allow(clippy::type_complexity)]
pub(super) fn judge_victory(
    mut commands: Commands,
    fight: Single<
        (Entity, &mut Battle, &Round, &Spoils, &mut Pause, &Combatants),
        With<RoundEnding>,
    >,
    mut members: Query<(&mut Character, &Vitals)>,
    monsters: Query<(
        &crate::game::actors::Attacks,
        &Vitals,
        &Side,
        Option<&crate::game::world::navigation::Walker>,
    )>,
    places: Places,
    leaving: Query<(), With<Leaves>>,
    roster: Single<&PartyRoster>,
    mut news: MessageWriter<CombatNews>,
) {
    let (fight, mut battle, round, spoils, mut pause, combatants) = fight.into_inner();
    let living: Vec<Entity> =
        roster.iter().filter(|&m| members.get(m).is_ok_and(|(_, v)| !v.is_dead())).collect();
    if living.is_empty() {
        return;
    }
    pause.0 = Timer::from_seconds(ROUND_SECONDS, TimerMode::Once);
    let approaching = combatants.entities().iter().any(|&entity| {
        monsters.get(entity).is_ok_and(|(arms, vitals, _, walker)| {
            let gap = roster.first().map_or(f32::INFINITY, |&member| places.gap(entity, member));
            approach::needs_approach(arms.reach(), vitals, walker, gap)
        })
    });
    let active = round.attacked || approaching;
    battle.passive_rounds = if active { 0 } else { battle.passive_rounds + 1 };
    let hostiles_left = combatants.entities().iter().any(|&entity| {
        !leaving.contains(entity)
            && monsters.get(entity).is_ok_and(|(_, vitals, side, ..)| {
                !vitals.is_dead() && side.now(vitals) == Stance::Hostile
            })
    });
    if hostiles_left && battle.passive_rounds < PASSIVE_ROUNDS {
        return;
    }
    let each = spoils.experience / living.len() as u32;
    if each > 0 {
        for &member in &living {
            members.get_mut(member).unwrap().0.award_experience(each);
        }
    }
    news.write(if spoils.kills > 0 {
        CombatNews::Won { kills: spoils.kills, experience_each: each }
    } else {
        CombatNews::Over
    });
    commands.entity(fight).insert(FightOver);
}

/// Recognition is part of each round's end, before the next accepted action
/// or the fight's end can change its meaning: the party looks again at who
/// it fights.
pub(super) fn look_again(
    fight: Single<&Combatants, With<RoundEnding>>,
    party: Query<(Entity, &Position, &PartyBody)>,
    laid: Laid,
    mut recognition: GroupRecognition,
    mut rng: ResMut<GameRng>,
) {
    let Ok((party, position, body)) = party.single() else { return };
    recognition.detect(
        PartyBody::eye(position),
        body.forward(),
        Some(fight.entities()),
        laid.has(party, SpellId::SHADOW_HOUND),
        false,
        &mut rng,
    );
}

/// The round has ended: those leaving leave, the next round waits to be
/// accepted, and a fight that is over goes; what lasted only the fight is
/// lifted as it does.
pub fn end_round(
    mut commands: Commands,
    fight: Single<(Entity, &mut Round, Has<FightOver>), With<RoundEnding>>,
    leaving: Query<Entity, With<Leaves>>,
) {
    let (fight, mut round, over) = fight.into_inner();
    for monster in &leaving {
        commands.entity(monster).remove::<(Leaves, Combatant)>();
    }
    round.playing = false;
    round.moment = 0;
    commands.entity(fight).remove::<(RoundEnding, FightOver)>();
    if over {
        commands.entity(fight).despawn();
    }
}
