//! A monster deciding its action (retail `UpdateMonsterAI`,
//! `BuildMonsterActionQueue`): a chain of systems, each of which may settle
//! the decision with the action it takes and [`Decided`]: to run, withdraw
//! or hold back, to breathe, to cast, to withdraw when it can strike
//! nobody, to guard a fellow, to hold back warily, or to fight. What it
//! decides from is what it sees of the fight ([`seen`]).

use bevy::prelude::*;

use super::monster_spells;
use super::turn::{
    Acting, Breathes, Casts, Decided, Fights, Fleeing, Ordered, Protects, Retreats, SpecialCooldown,
};
use super::{Battle, Combatants, conduct};
use crate::content::species::{Reach, Species};
use crate::content::spells::SpellId;
use crate::game::characters::{Condition, Vitals};
use crate::game::data::Catalogs;
use crate::game::hits::prototypes::Definitions;
use crate::game::magic::casting::{MagicChoice, Source};
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::footing::{Conspicuous, Eyes, Footing};
use crate::game::world::hostility::{self, Loyalty, Side, Stance};
use crate::game::world::level::PlayedLevel;
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::{Monster, SpellBudget};

/// Whether a monster can make its special attack now (retail
/// `CanMonsterFlee`): it has one, it is not cooling down, it has a tenth of
/// its stamina, and a silence does not stop it. A summoning is never what
/// starts a fight (`fighting` is `None` then, retail's `exclude_special`);
/// in one it needs the party within long reach and the summoner not on its
/// side. Any other kind also needs an aim, which its caller finds.
pub(super) fn special_ready(
    vitals: &Vitals,
    kind: &Species,
    cooling: bool,
    fighting: Option<(f32, Stance)>,
) -> bool {
    let silenced = vitals.conditions.has(Condition::Silenced);
    kind.special.kind.is_some_and(|special| {
        let definition = crate::game::hits::specials::special(special);
        let summons = special.summons().is_none()
            || fighting.is_some_and(|(nearest, stance)| {
                nearest <= Reach::Long.metres() && stance != Stance::Friendly
            });
        !cooling
            && vitals.stamina.current >= vitals.stamina.max / 10
            && !(silenced && definition.silenced)
            && summons
    })
}

/// Whom a monster sees of the fight (retail `IsVisibleUnderConditions`,
/// `CanMonsterSeeMonster`): the living party when its eyes see the party,
/// and the living monsters in the fight it has a clear line to, itself
/// among them.
pub(super) fn seen(
    monster: Entity,
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
            other == monster
                || bodies.get(other).is_ok_and(|(_, there, ..)| obstacles.clear(at.eye, there.eye))
        }))
        .filter(|&other| bodies.get(other).is_ok_and(|(vitals, ..)| !vitals.is_dead()))
        .collect()
}

/// How far the party is from a monster.
fn party_gap(at: &Footing, roster: &PartyRoster, footings: &Query<&Footing>) -> f32 {
    roster
        .iter()
        .find_map(|member| footings.get(member).ok())
        .map_or(f32::INFINITY, |there| at.gap(there))
}

/// Run, withdraw or hold back, as it stands and fares (retail
/// `UpdateMonsterAI`'s flight test); a flight begun goes on in later rounds
/// while the party stays off.
pub(super) fn decide_conduct(
    mut commands: Commands,
    actors: Query<
        (Entity, &Monster, &Vitals, &Side, &Footing, Has<Fleeing>),
        (With<Acting>, Without<Ordered>, Without<Decided>),
    >,
    footings: Query<&Footing>,
    roster: Single<&PartyRoster>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    for (monster, body, vitals, side, at, was_fleeing) in &actors {
        let kind = &content.species[body.species];
        let nearest = party_gap(at, &roster, &footings);
        let mut fleeing = was_fleeing;
        let conduct = conduct::decide(&mut rng, body, vitals, kind, side.0, &mut fleeing, nearest);
        let mut entity = commands.entity(monster);
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

/// Whether a monster would close in to strike rather than cast or breathe:
/// it prefers blows, has some, and none reaches the party.
fn closes_in(kind: &Species, nearest: f32) -> bool {
    let reaches = kind.attacks.iter().any(|attack| nearest <= attack.reach.metres());
    !kind.temper.prefers_range && !kind.attacks.is_empty() && !reaches
}

/// A monster with a special attack ready may make it (retail
/// `UpdateMonsterAI`'s special branch): always if it cannot strike the
/// party, else at its chance; one that would close in to strike does that
/// instead. A breath is aimed at one of its aims at random (retail
/// `AimMonsterAtSpellTarget`) and catches whoever stands in its cone; a
/// summoning needs no one in sight.
#[allow(clippy::type_complexity)]
pub(super) fn decide_special(
    mut commands: Commands,
    actors: Query<
        (Entity, &Monster, &Vitals, &Side, &Footing, &Eyes, Option<&SpecialCooldown>),
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
    for (monster, body, vitals, side, at, eyes, cooldown) in &actors {
        let kind = &content.species[body.species];
        let special = kind.special;
        let Some(special_kind) = special.kind else { continue };
        let nearest = party_gap(at, &roster, &footings);
        let reaches = kind.attacks.iter().any(|attack| nearest <= attack.reach.metres());
        let cooling = cooldown.is_some_and(|cooldown| cooldown.0 > 0);
        if special.chance == 0
            || closes_in(kind, nearest)
            || !special_ready(vitals, kind, cooling, Some((nearest, side.0)))
        {
            continue;
        }
        let toward = if special_kind.summons().is_some() {
            at.feet
        } else {
            let Some(cone) = content.spells.get(SpellId::SPECIAL_ATTACK_CONE) else { continue };
            let seen = seen(
                monster,
                (at, eyes),
                (&roster, combatants),
                &bodies,
                &conspicuous,
                far,
                &obstacles,
            );
            let aims = monster_spells::aims(cone, monster, &seen, &bodies, &roster, &definitions);
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
        commands.entity(monster).insert((
            Breathes {
                special: Some(special_kind),
                definition: definitions.special(special_kind),
                summons: special_kind.summons(),
                strength: crate::game::hits::specials::strength(kind.level, vitals.stamina),
                toward,
                cost: vitals.stamina.max / 10,
                cooldown: special.cooldown,
            },
            Decided,
        ));
    }
}

/// A monster that casts may cast (retail `UpdateMonsterAI`'s spell
/// branch): always if it cannot strike the party, else at its chance; one
/// that would close in to strike does that instead.
#[allow(clippy::type_complexity)]
pub(super) fn decide_spell(
    mut commands: Commands,
    actors: Query<
        (Entity, &Monster, &SpellBudget, &Footing, &Eyes),
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
    for (monster, caster, budget, at, eyes) in &actors {
        let kind = &content.species[caster.species];
        let nearest = party_gap(at, &roster, &footings);
        if kind.magic.chance == 0 || closes_in(kind, nearest) {
            continue;
        }
        let reaches = kind.attacks.iter().any(|attack| nearest <= attack.reach.metres());
        let chance = if reaches { u32::from(kind.magic.chance) } else { 100 };
        let seen = seen(
            monster,
            (at, eyes),
            (&roster, combatants),
            &bodies,
            &conspicuous,
            far,
            &obstacles,
        );
        let Some((spell, aim)) = monster_spells::choose(
            &mut rng,
            monster,
            kind,
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
        let source = Source::Innate { spell, power, budget: budget.0, level: kind.level };
        commands.entity(monster).insert((Casts(MagicChoice { source, aim }), Decided));
    }
}

/// One that can strike nobody, having no attack or, blind, nobody in reach,
/// withdraws (retail `UpdateMonsterAI`'s `RETURN_TO_START`); a Monk fights
/// blind.
pub(super) fn withdraw_if_helpless(
    mut commands: Commands,
    actors: Query<
        (Entity, &Monster, &Vitals, &Footing),
        (With<Acting>, Without<Ordered>, Without<Decided>),
    >,
    footings: Query<&Footing>,
    roster: Single<&PartyRoster>,
    content: If<Res<Catalogs>>,
) {
    for (monster, body, vitals, at) in &actors {
        let kind = &content.species[body.species];
        let nearest = party_gap(at, &roster, &footings);
        let reaches = kind.attacks.iter().any(|attack| nearest <= attack.reach.metres());
        let blind = crate::game::world::population::blinded(kind, vitals);
        if kind.attacks.is_empty() || (blind && !reaches) {
            let withdraw = Retreats { conduct: conduct::Conduct::Withdraw, quiet: false };
            commands.entity(monster).insert((withdraw, Decided));
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
        (Entity, &Monster, &Vitals, &Footing, &Side, &Loyalty),
        (With<Acting>, Without<Ordered>, Without<Decided>),
    >,
    fellows: Query<(&Monster, &Vitals, &Footing, &Side, &Loyalty)>,
    footings: Query<&Footing>,
    fight: Single<Option<&Combatants>, With<Battle>>,
    roster: Single<&PartyRoster>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    let combatants = fight.map_or(&[][..], Combatants::entities);
    let species = &content.species;
    for (monster, me, vitals, at, side, loyalty) in &actors {
        let kind = &species[me.species];
        let nearest = party_gap(at, &roster, &footings);
        if !kind.attacks.iter().any(|attack| nearest <= attack.reach.metres()) {
            continue;
        }
        let intelligence = u32::from(kind.attributes.intelligence);
        let able = kind.temper.bodyguard && vitals.can_guard() && !kind.attacks.is_empty();
        let mine = (side.0, *loyalty);
        let wards: Vec<Entity> = if able && rng.below(100) < intelligence {
            let reach = kind.attacks[0].reach.metres();
            combatants
                .iter()
                .copied()
                .filter(|&other| other != monster)
                .filter(|&other| {
                    fellows.get(other).is_ok_and(
                        |(fellow, theirs, there, their_side, their_loyalty)| {
                            !theirs.is_dead()
                                && hostility::between(mine, (their_side.0, *their_loyalty))
                                    == Stance::Friendly
                                && at.gap(there) <= reach
                                && species[fellow.species].level > kind.level
                                && theirs.health.current * 100 / theirs.health.max < 40
                        },
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        if !wards.is_empty() {
            let ward = wards[rng.below(wards.len() as u32) as usize];
            commands.entity(monster).insert((Protects(ward), Decided));
        } else if kind.temper.wary && rng.below(100) < intelligence {
            commands.entity(monster).insert((Fights { waits: true, berserk: false }, Decided));
        }
    }
}

/// Anything else fights: strikes someone, or closes in to.
pub(super) fn default_fight(
    mut commands: Commands,
    actors: Query<Entity, (With<Acting>, Without<Ordered>, Without<Decided>)>,
) {
    for monster in &actors {
        commands.entity(monster).insert((Fights { waits: false, berserk: false }, Decided));
    }
}
