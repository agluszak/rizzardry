//! A body deciding its action for itself (retail `UpdateMonsterAI`,
//! `BuildMonsterActionQueue`): a chain of systems, each of which may settle
//! the decision with the action it takes and [`Decided`]: to run, withdraw
//! or hold back by its [`Temper`], to make its [`Special`] attack, to cast
//! by its [`Spellcasting`], to withdraw when it can strike nobody, to guard
//! a fellow, to hold back warily, or to fight. What it decides from is what
//! it sees of the fight ([`seen`]) and the facts judged on everyone.

use bevy::prelude::*;

use super::fighters::{Arms, BlindFighting, Wit};
use super::monster_spells;
use super::turn::{
    Acting, Breathes, Casts, Decided, Fights, Fleeing, Ordered, Protects, Retreats, SpecialCooldown,
};
use super::{Battle, Combatants, conduct};
use crate::content::species::{Reach, SpecialAttack};
use crate::content::spells::SpellId;
use crate::game::characters::{Condition, Vitals};
use crate::game::data::Catalogs;
use crate::game::hits::defences::SaveLevel;
use crate::game::hits::prototypes::Definitions;
use crate::game::magic::casting::{MagicChoice, Source};
use crate::game::npcs::NpcBody;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::footing::{Conspicuous, Eyes, Footing};
use crate::game::world::hostility::{self, Loyalty, Side, Stance};
use crate::game::world::level::PlayedLevel;
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::{Special, SpellBudget, Spellcasting, Temper};

/// Whether a body can make its special attack now (retail
/// `CanMonsterFlee`): it has one, it is not cooling down, it has a tenth of
/// its stamina, and a silence does not stop it. A summoning is never what
/// starts a fight (`fighting` is `None` then, retail's `exclude_special`);
/// in one it needs the party within long reach and the summoner not on its
/// side. Any other kind also needs an aim, which its caller finds.
pub(super) fn special_ready(
    vitals: &Vitals,
    special: &SpecialAttack,
    cooling: bool,
    fighting: Option<(f32, Stance)>,
) -> bool {
    let silenced = vitals.conditions.has(Condition::Silenced);
    special.kind.is_some_and(|kind| {
        let definition = crate::game::hits::specials::special(kind);
        let summons = kind.summons().is_none()
            || fighting.is_some_and(|(nearest, stance)| {
                nearest <= Reach::Long.metres() && stance != Stance::Friendly
            });
        !cooling
            && vitals.stamina.current >= vitals.stamina.max / 10
            && !(silenced && definition.silenced)
            && summons
    })
}

/// Whom a body sees of the fight (retail `IsVisibleUnderConditions`,
/// `CanMonsterSeeMonster`): the living party when its eyes see the party,
/// and the living monsters in the fight it has a clear line to, itself
/// among them.
pub(super) fn seen(
    body: Entity,
    (at, eyes): (&Footing, &Eyes),
    (roster, combatants): (&PartyRoster, &[Entity]),
    bodies: &Query<(&Vitals, &Footing, &Side, &Loyalty)>,
    conspicuous: &Query<&Conspicuous>,
    far: f32,
    obstacles: &WorldObstacles,
) -> Vec<Entity> {
    let party = roster
        .iter()
        .find_map(|member| Some((bodies.get(member).ok()?.1, conspicuous.get(member).ok()?)));
    let sees_party = party.is_some_and(|party| eyes.see(at, party, far, obstacles));
    roster
        .iter()
        .filter(|_| sees_party)
        .chain(combatants.iter().copied().filter(|&other| {
            other == body
                || bodies.get(other).is_ok_and(|(_, there, ..)| obstacles.clear(at.eye, there.eye))
        }))
        .filter(|&other| bodies.get(other).is_ok_and(|(vitals, ..)| !vitals.is_dead()))
        .collect()
}

/// How far the party is from a body.
fn party_gap(at: &Footing, roster: &PartyRoster, footings: &Query<&Footing>) -> f32 {
    roster
        .iter()
        .find_map(|member| footings.get(member).ok())
        .map_or(f32::INFINITY, |there| at.gap(there))
}

/// Whether one of a body's arms reaches the party.
fn reaches_party(arms: &Arms, nearest: f32) -> bool {
    arms.0.iter().any(|arm| nearest <= arm.reach.metres())
}

/// Whether a body would close in to strike rather than cast or breathe:
/// it prefers blows, has some, and none reaches the party.
fn closes_in(temper: &Temper, arms: &Arms, nearest: f32) -> bool {
    !temper.0.prefers_range && !arms.0.is_empty() && !reaches_party(arms, nearest)
}

/// Run, withdraw or hold back, as it stands and fares (retail
/// `UpdateMonsterAI`'s flight test); a flight begun goes on in later rounds
/// while the party stays off.
#[allow(clippy::type_complexity)]
pub(super) fn decide_conduct(
    mut commands: Commands,
    actors: Query<
        (Entity, &Temper, &Vitals, &Side, &Loyalty, &Footing, Has<NpcBody>, Has<Fleeing>),
        (With<Acting>, Without<Ordered>, Without<Decided>),
    >,
    footings: Query<&Footing>,
    roster: Single<&PartyRoster>,
    mut rng: ResMut<GameRng>,
) {
    for (body, temper, vitals, side, loyalty, at, npc, was_fleeing) in &actors {
        let nearest = party_gap(at, &roster, &footings);
        let mut fleeing = was_fleeing;
        let bystander = *loyalty == Loyalty::Bystander;
        let conduct = conduct::decide(
            &mut rng,
            vitals,
            &temper.0,
            (npc, bystander),
            side.0,
            &mut fleeing,
            nearest,
        );
        let mut entity = commands.entity(body);
        match (was_fleeing, fleeing) {
            (false, true) => {
                entity.insert(Fleeing);
            }
            (true, false) => {
                entity.remove::<Fleeing>();
            }
            _ => {}
        }
        if conduct != conduct::Conduct::Fight {
            entity.insert((Retreats { conduct, quiet: false }, Decided));
        }
    }
}

/// A body with a special attack ready may make it (retail
/// `UpdateMonsterAI`'s special branch): always if it cannot strike the
/// party, else at its chance; one that would close in to strike does that
/// instead. A breath is aimed at one of its aims at random (retail
/// `AimMonsterAtSpellTarget`) and catches whoever stands in its cone; a
/// summoning needs no one in sight.
#[allow(clippy::type_complexity)]
pub(super) fn decide_special(
    mut commands: Commands,
    actors: Query<
        (
            Entity,
            (&Special, &Temper, &Arms, &SaveLevel),
            (&Vitals, &Side, &Footing, &Eyes),
            Option<&SpecialCooldown>,
        ),
        (With<Acting>, Without<Ordered>, Without<Decided>),
    >,
    bodies: Query<(&Vitals, &Footing, &Side, &Loyalty)>,
    conspicuous: Query<&Conspicuous>,
    footings: Query<&Footing>,
    fight: Single<Option<&Combatants>, With<Battle>>,
    roster: Single<&PartyRoster>,
    level: PlayedLevel,
    obstacles: WorldObstacles,
    content: If<Res<Catalogs>>,
    definitions: Res<Definitions>,
    mut rng: ResMut<GameRng>,
) {
    let combatants = fight.map_or(&[][..], Combatants::entities);
    let far = level.view_distance().unwrap_or(0.0);
    for (body, (special, temper, arms, level), (vitals, side, at, eyes), cooldown) in &actors {
        let special = special.0;
        let Some(kind) = special.kind else { continue };
        let nearest = party_gap(at, &roster, &footings);
        let reaches = reaches_party(arms, nearest);
        let cooling = cooldown.is_some_and(|cooldown| cooldown.0 > 0);
        if special.chance == 0
            || closes_in(temper, arms, nearest)
            || !special_ready(vitals, &special, cooling, Some((nearest, side.0)))
        {
            continue;
        }
        let toward = if kind.summons().is_some() {
            at.feet
        } else {
            let Some(cone) = content.spells.get(SpellId::SPECIAL_ATTACK_CONE) else { continue };
            let seen = seen(
                body,
                (at, eyes),
                (&roster, combatants),
                &bodies,
                &conspicuous,
                far,
                &obstacles,
            );
            let aims = monster_spells::aims(cone, body, &seen, &bodies, &roster, &definitions);
            if aims.is_empty() {
                continue;
            }
            let aim = aims[rng.below(aims.len() as u32) as usize];
            aim.body()
                .and_then(|aimed| footings.get(aimed).ok())
                .map_or(at.feet, |there| there.feet)
        };
        if rng.below(100) >= if reaches { u32::from(special.chance) } else { 100 } {
            continue;
        }
        commands.entity(body).insert((
            Breathes {
                special: Some(kind),
                definition: definitions.special(kind),
                summons: kind.summons(),
                strength: crate::game::hits::specials::strength(level.0 as u8, vitals.stamina),
                toward,
                cost: vitals.stamina.max / 10,
                cooldown: special.cooldown,
            },
            Decided,
        ));
    }
}

/// A body that casts may cast (retail `UpdateMonsterAI`'s spell branch):
/// always if it cannot strike the party, else at its chance; one that would
/// close in to strike does that instead.
#[allow(clippy::type_complexity)]
pub(super) fn decide_spell(
    mut commands: Commands,
    actors: Query<
        (Entity, (&Spellcasting, &Temper, &Arms, &SpellBudget, &SaveLevel), (&Footing, &Eyes)),
        (With<Acting>, Without<Ordered>, Without<Decided>),
    >,
    bodies: Query<(&Vitals, &Footing, &Side, &Loyalty)>,
    conspicuous: Query<&Conspicuous>,
    footings: Query<&Footing>,
    fight: Single<Option<&Combatants>, With<Battle>>,
    roster: Single<&PartyRoster>,
    level: PlayedLevel,
    obstacles: WorldObstacles,
    content: If<Res<Catalogs>>,
    definitions: Res<Definitions>,
    mut rng: ResMut<GameRng>,
) {
    let combatants = fight.map_or(&[][..], Combatants::entities);
    let far = level.view_distance().unwrap_or(0.0);
    for (body, (casting, temper, arms, budget, level), (at, eyes)) in &actors {
        let nearest = party_gap(at, &roster, &footings);
        if casting.chance == 0 || closes_in(temper, arms, nearest) {
            continue;
        }
        let chance = if reaches_party(arms, nearest) { u32::from(casting.chance) } else { 100 };
        let seen =
            seen(body, (at, eyes), (&roster, combatants), &bodies, &conspicuous, far, &obstacles);
        let Some((spell, aim)) = monster_spells::choose(
            &mut rng,
            body,
            casting,
            chance,
            &content.spells,
            &seen,
            &bodies,
            &roster,
            &definitions,
        ) else {
            continue;
        };
        let power = monster_spells::power(&mut rng, budget.0, &content.spells[spell]);
        let source = Source::Innate { spell, power, budget: budget.0, level: level.0 as u8 };
        commands.entity(body).insert((Casts(MagicChoice { source, aim }), Decided));
    }
}

/// One that can strike nobody, having no arm or, blind, nobody in reach,
/// withdraws (retail `UpdateMonsterAI`'s `RETURN_TO_START`); one that
/// fights blind fights on.
pub(super) fn withdraw_if_helpless(
    mut commands: Commands,
    actors: Query<
        (Entity, &Arms, &Vitals, &BlindFighting, &Footing),
        (With<Acting>, Without<Ordered>, Without<Decided>),
    >,
    footings: Query<&Footing>,
    roster: Single<&PartyRoster>,
) {
    for (body, arms, vitals, blind_fighting, at) in &actors {
        let nearest = party_gap(at, &roster, &footings);
        let blind = vitals.conditions.has(Condition::Blind) && blind_fighting.0.is_none();
        if arms.0.is_empty() || (blind && !reaches_party(arms, nearest)) {
            let withdraw = Retreats { conduct: conduct::Conduct::Withdraw, quiet: false };
            commands.entity(body).insert((withdraw, Decided));
        }
    }
}

/// A bodyguard with someone in reach, as clever as it is, may guard a
/// stronger fellow in its reach in trouble (retail
/// `BuildMonsterActionQueue`); a wary one may hold back instead.
#[allow(clippy::type_complexity)]
pub(super) fn decide_guard(
    mut commands: Commands,
    actors: Query<
        (Entity, (&Temper, &Wit, &Arms, &SaveLevel), (&Vitals, &Footing, &Side, &Loyalty)),
        (With<Acting>, Without<Ordered>, Without<Decided>),
    >,
    fellows: Query<(&SaveLevel, &Vitals, &Footing, &Side, &Loyalty)>,
    footings: Query<&Footing>,
    fight: Single<Option<&Combatants>, With<Battle>>,
    roster: Single<&PartyRoster>,
    mut rng: ResMut<GameRng>,
) {
    let combatants = fight.map_or(&[][..], Combatants::entities);
    for (body, (temper, wit, arms, level), (vitals, at, side, loyalty)) in &actors {
        let nearest = party_gap(at, &roster, &footings);
        if !reaches_party(arms, nearest) {
            continue;
        }
        let able = temper.0.bodyguard && vitals.can_guard() && !arms.0.is_empty();
        let mine = (side.0, *loyalty);
        let wards: Vec<Entity> = if able && rng.below(100) < wit.0 {
            let reach = arms.0[0].reach.metres();
            combatants
                .iter()
                .copied()
                .filter(|&other| other != body)
                .filter(|&other| {
                    fellows.get(other).is_ok_and(
                        |(theirs, their_vitals, there, their_side, their_loyalty)| {
                            !their_vitals.is_dead()
                                && hostility::between(mine, (their_side.0, *their_loyalty))
                                    == Stance::Friendly
                                && at.gap(there) <= reach
                                && theirs.0 > level.0
                                && their_vitals.health.current * 100 / their_vitals.health.max < 40
                        },
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        if !wards.is_empty() {
            let ward = wards[rng.below(wards.len() as u32) as usize];
            commands.entity(body).insert((Protects(ward), Decided));
        } else if temper.0.wary && rng.below(100) < wit.0 {
            commands.entity(body).insert((Fights { waits: true, berserk: false }, Decided));
        }
    }
}

/// Anything else fights: strikes someone, or closes in to.
pub(super) fn default_fight(
    mut commands: Commands,
    actors: Query<Entity, (With<Acting>, Without<Ordered>, Without<Decided>)>,
) {
    for body in &actors {
        commands.entity(body).insert((Fights { waits: false, berserk: false }, Decided));
    }
}
