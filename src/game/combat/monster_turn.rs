//! A monster's turn (retail `UpdateMonsterAI`, `StartMonsterCombatAction`,
//! `StepMonsterCombatAction`). When its action comes ([`Acting`]) it
//! decides: a chain of systems, each of which may settle the decision with
//! the action it takes and [`Decided`]: to run, withdraw or hold back, to
//! breathe, to cast, to withdraw when it can strike nobody, to guard a
//! fellow, to hold back warily, or to fight. Its conditions may then
//! interrupt that ([`interrupt`]). Each action has its system: [`retreat`],
//! [`protect`], [`start_special`], [`start_monster_cast`], and the attack
//! systems ([`super::swings::monster_attacks`]).

use avian3d::prelude::Position;
use bevy::prelude::*;

use super::monster_spells::{self, Scene, SpellAim};
use super::round::{Acting, Fleeing, Guarding, MonsterTurn, SpecialCooldown};
use super::scene::{scene, special_ready};
use super::sight::Sight;
use super::{ACTION_SECONDS, Battle, CombatNews, Combatants, approach, conduct, summon};
use crate::content::spells::SpellId;
use crate::game::characters::Condition;
use crate::game::data::Catalogs;
use crate::game::hits::prototypes::{Definitions, Uses};
use crate::game::hits::spells::reach;
use crate::game::hits::{Act, Body, BodyReadOnlyItem, Deed};
use crate::game::magic::Strength;
use crate::game::magic::casting::{Aim, Cast, CastBy, MonsterBudget, Power, Target, UseContext};
use crate::game::magic::enchantments::PartyEffects;
use crate::game::party::body::PartyBody;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::hostility::{self, Stance};
use crate::game::world::level::{Level, PlayedLevel};
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::perception::party_distance;

/// A monster that settled on fighting this action; `waits` when it may
/// hold back instead.
#[derive(Component, Debug)]
pub(super) struct Fights {
    pub(super) waits: bool,
}

/// How one body stands toward another, by their sides and loyalties.
pub(super) fn toward(mine: (Stance, hostility::Loyalty), body: &BodyReadOnlyItem) -> Stance {
    hostility::between(mine, standing(body))
}

/// A body's side and loyalty.
pub(super) fn standing(body: &BodyReadOnlyItem) -> (Stance, hostility::Loyalty) {
    (body.side.0, *body.loyalty)
}

/// The monster has settled what it does with its action.
#[derive(Component, Debug)]
pub(super) struct Decided;

/// The monster goes on with its turn: it acts again later in the round
/// after moving, deciding nothing anew.
#[derive(Component, Debug)]
pub(super) struct Resumes;

/// Run, withdraw or hold back; `quiet` for a flight fear forced, which
/// retail does not announce.
#[derive(Component, Debug)]
pub(super) struct Retreats {
    conduct: conduct::Conduct,
    quiet: bool,
}

/// Guard a fellow this round.
#[derive(Component, Debug)]
pub(super) struct Protects(Entity);

/// Make its special attack at these victims, at this strength, cooling
/// down for so many rounds after.
#[derive(Component, Debug)]
pub(super) struct Breathes {
    kind: crate::content::species::SpecialAttackKind,
    cooldown: u8,
    strength: Strength,
    victims: Vec<Entity>,
}

/// Cast this spell, at this power and aim, at the fight as it saw it, by
/// its budget.
#[derive(Component, Debug)]
pub(super) struct CastsSpell {
    spell: SpellId,
    power: u32,
    aim: SpellAim,
    scene: Scene,
    budget: u32,
}

/// How far a monster is from the party, and whether one of its attacks
/// reaches the party from there.
fn bearing(
    body: &BodyReadOnlyItem,
    eye: Vec3,
    species: &crate::content::species::MonsterCatalog,
) -> (f32, bool) {
    let monster = body.monster.as_ref().unwrap();
    let nearest = party_distance(eye, monster, body.feet(eye));
    let kind = &species[monster.species];
    (nearest, kind.attacks.iter().any(|attack| nearest <= attack.reach.metres()))
}

/// A monster whose action has come, revalidated: one no longer in the fight
/// or able loses its turn; one going on with its turn after moving fights
/// on without deciding again.
pub(super) fn begin_monster_turns(
    mut commands: Commands,
    mut actors: Query<(Entity, &mut MonsterTurn), (With<Acting>, Without<Decided>)>,
    bodies: Query<Body>,
    fight: Single<Option<&Combatants>, With<Battle>>,
    content: If<Res<Catalogs>>,
) {
    let combatants = fight.map_or(&[][..], Combatants::entities);
    for (monster, mut turn) in &mut actors {
        let Ok(body) = bodies.get(monster) else { continue };
        if !combatants.contains(&monster) || !body.vitals.can_fight() {
            turn.end();
            commands.entity(monster).remove::<Acting>();
            continue;
        }
        let kind = &content.species[body.monster.as_ref().unwrap().species];
        if turn.attacks_left != u32::from(kind.attacks_per_round) || turn.moved {
            commands.entity(monster).insert((Fights { waits: false }, Decided, Resumes));
        }
    }
}

/// Run, withdraw or hold back, as it stands and fares (retail
/// `UpdateMonsterAI`'s flight test); a flight begun goes on in later rounds
/// while the party stays off.
pub(super) fn decide_conduct(
    mut commands: Commands,
    actors: Query<(Entity, Has<Fleeing>), (With<Acting>, With<MonsterTurn>, Without<Decided>)>,
    bodies: Query<Body>,
    stand: Single<&Position, With<PartyBody>>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    let eye = PartyBody::eye(*stand);
    for (monster, was_fleeing) in &actors {
        let body = bodies.get(monster).unwrap();
        let kind = &content.species[body.monster.as_ref().unwrap().species];
        let (nearest, _) = bearing(&body, eye, &content.species);
        let side = body.side.0;
        let mut fleeing = was_fleeing;
        let conduct = conduct::decide(
            &mut rng,
            body.monster.as_ref().unwrap(),
            body.vitals,
            kind,
            side,
            &mut fleeing,
            nearest,
        );
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

/// The fight as a monster about to breathe or cast sees it.
fn sees(
    monster: Entity,
    bodies: &mut Query<Body>,
    combatants: &[Entity],
    roster: &PartyRoster,
    eye: Vec3,
    far: f32,
    concealment: u32,
    obstacles: &WorldObstacles,
    species: &crate::content::species::MonsterCatalog,
) -> Scene {
    let lowest_level = crate::game::party::lowest_level(
        roster
            .iter()
            .filter_map(|member| bodies.get(member).ok())
            .filter_map(|body| Some((body.character?, body.vitals))),
    );
    let sight = Sight { eye, far, lowest_level, concealment, obstacles };
    let mine = standing(&bodies.get(monster).unwrap());
    scene(monster, combatants, roster, bodies, &sight, species, &|body: &BodyReadOnlyItem| {
        toward(mine, body)
    })
}

/// A monster with a special attack ready may make it (retail
/// `UpdateMonsterAI`'s special branch): always if it cannot strike the
/// party, else at its chance; one that would close in to strike does that
/// instead. A breath is aimed at one of its aims at random (retail
/// `AimMonsterAtSpellTarget`) and catches whoever stands in its cone; a
/// summoning needs no one in sight.
pub(super) fn decide_special(
    mut commands: Commands,
    actors: Query<Entity, (With<Acting>, With<MonsterTurn>, Without<Decided>)>,
    mut bodies: Query<Body>,
    cooldowns: Query<&SpecialCooldown>,
    fight: Single<Option<&Combatants>, With<Battle>>,
    stand: Single<&Position, With<PartyBody>>,
    roster: Single<&PartyRoster>,
    party: Single<&PartyEffects>,
    level: PlayedLevel,
    obstacles: WorldObstacles,
    content: If<Res<Catalogs>>,
    definitions: Res<Definitions>,
    mut rng: ResMut<GameRng>,
) {
    let combatants = fight.map_or(&[][..], Combatants::entities);
    let eye = PartyBody::eye(*stand);
    let far = level.view_distance().unwrap_or(0.0);
    for monster in &actors {
        let body = bodies.get(monster).unwrap();
        let kind = &content.species[body.monster.as_ref().unwrap().species];
        let (nearest, reaches) = bearing(&body, eye, &content.species);
        let closes_in = !kind.temper.prefers_range && !kind.attacks.is_empty() && !reaches;
        let side = body.side.0;
        let cooling = cooldowns.get(monster).is_ok_and(|cooldown| cooldown.0 > 0);
        let special = kind.special;
        let Some(special_kind) = special.kind else { continue };
        if special.chance == 0
            || closes_in
            || !special_ready(body.vitals, kind, cooling, Some((nearest, side)))
        {
            continue;
        }
        let breather = (kind.level, body.vitals.stamina);
        let victims = if special_kind.summons().is_some() {
            Some(Vec::new())
        } else {
            let Some(cone) = content.spells.get(SpellId::SPECIAL_ATTACK_CONE) else { continue };
            let scene = sees(
                monster,
                &mut bodies,
                combatants,
                &roster,
                eye,
                far,
                party.0.concealment(),
                &obstacles,
                &content.species,
            );
            let aims = monster_spells::aims(cone, &scene, &definitions);
            (!aims.is_empty()).then(|| {
                let aim = aims[rng.below(aims.len() as u32) as usize];
                let toward = monster_spells::aimed_at(&scene, aim);
                monster_spells::cone_victims(&scene, toward, cone.reach.metres())
            })
        };
        let Some(victims) = victims else { continue };
        if rng.below(100) >= if reaches { u32::from(special.chance) } else { 100 } {
            continue;
        }
        let strength = crate::game::hits::specials::strength(breather.0, breather.1);
        commands.entity(monster).insert((
            Breathes { kind: special_kind, cooldown: special.cooldown, strength, victims },
            Decided,
        ));
    }
}

/// A monster that casts may cast (retail `UpdateMonsterAI`'s spell
/// branch): always if it cannot strike the party, else at its chance; one
/// that would close in to strike does that instead.
pub(super) fn decide_spell(
    mut commands: Commands,
    actors: Query<Entity, (With<Acting>, With<MonsterTurn>, Without<Decided>)>,
    mut bodies: Query<Body>,
    fight: Single<Option<&Combatants>, With<Battle>>,
    stand: Single<&Position, With<PartyBody>>,
    roster: Single<&PartyRoster>,
    party: Single<&PartyEffects>,
    level: PlayedLevel,
    obstacles: WorldObstacles,
    content: If<Res<Catalogs>>,
    definitions: Res<Definitions>,
    mut rng: ResMut<GameRng>,
) {
    let combatants = fight.map_or(&[][..], Combatants::entities);
    let eye = PartyBody::eye(*stand);
    let far = level.view_distance().unwrap_or(0.0);
    for monster in &actors {
        let body = bodies.get(monster).unwrap();
        let caster = body.monster.unwrap().clone();
        let kind = &content.species[caster.species];
        let (_, reaches) = bearing(&body, eye, &content.species);
        let closes_in = !kind.temper.prefers_range && !kind.attacks.is_empty() && !reaches;
        if kind.magic.chance == 0 || closes_in {
            continue;
        }
        let vitals = body.vitals.clone();
        let chance = if reaches { u32::from(kind.magic.chance) } else { 100 };
        let scene = sees(
            monster,
            &mut bodies,
            combatants,
            &roster,
            eye,
            far,
            party.0.concealment(),
            &obstacles,
            &content.species,
        );
        let Some((spell, aim)) = monster_spells::choose(
            &mut rng,
            &vitals,
            kind,
            chance,
            &content.spells,
            &scene,
            &definitions,
        ) else {
            continue;
        };
        let budget = caster.spell_budget(kind);
        let power = monster_spells::power(&mut rng, budget, &content.spells[spell]);
        commands.entity(monster).insert((CastsSpell { spell, power, aim, scene, budget }, Decided));
    }
}

/// One that can strike nobody, having no attack or, blind, nobody in reach,
/// withdraws (retail `UpdateMonsterAI`'s `RETURN_TO_START`); a Monk fights
/// blind.
pub(super) fn withdraw_if_helpless(
    mut commands: Commands,
    actors: Query<Entity, (With<Acting>, With<MonsterTurn>, Without<Decided>)>,
    bodies: Query<Body>,
    stand: Single<&Position, With<PartyBody>>,
    content: If<Res<Catalogs>>,
) {
    let eye = PartyBody::eye(*stand);
    for monster in &actors {
        let body = bodies.get(monster).unwrap();
        let kind = &content.species[body.monster.as_ref().unwrap().species];
        let (_, reaches) = bearing(&body, eye, &content.species);
        let blind = crate::game::world::population::blinded(kind, body.vitals);
        if kind.attacks.is_empty() || (blind && !reaches) {
            let withdraw = Retreats { conduct: conduct::Conduct::Withdraw, quiet: false };
            commands.entity(monster).insert((withdraw, Decided));
        }
    }
}

/// A bodyguard with someone in reach, as clever as it is, may guard a
/// stronger fellow in its reach in trouble (retail
/// `BuildMonsterActionQueue`); a wary one may hold back instead.
pub(super) fn decide_guard(
    mut commands: Commands,
    actors: Query<Entity, (With<Acting>, With<MonsterTurn>, Without<Decided>)>,
    bodies: Query<Body>,
    fight: Single<Option<&Combatants>, With<Battle>>,
    stand: Single<&Position, With<PartyBody>>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    let combatants = fight.map_or(&[][..], Combatants::entities);
    let eye = PartyBody::eye(*stand);
    let species = &content.species;
    for monster in &actors {
        let body = bodies.get(monster).unwrap();
        let me = body.monster.unwrap().clone();
        let kind = &species[me.species];
        let (_, reaches) = bearing(&body, eye, species);
        if !reaches {
            continue;
        }
        let (feet, can_guard) = (body.feet(eye), body.vitals.can_guard());
        let mine = standing(&body);
        let intelligence = u32::from(kind.attributes.intelligence);
        let able = kind.temper.bodyguard && can_guard && !kind.attacks.is_empty();
        let wards: Vec<Entity> = if able && rng.below(100) < intelligence {
            let reach = kind.attacks[0].reach.metres();
            combatants
                .iter()
                .copied()
                .filter(|&other| {
                    other != monster
                        && bodies.get(other).is_ok_and(|o| {
                            let fellow = o.monster.as_ref().unwrap();
                            toward(mine, &o) == Stance::Friendly
                                && !o.is_dead()
                                && me.gap(feet, fellow, o.feet(eye)) <= reach
                                && species[fellow.species].level > kind.level
                                && o.vitals.health.current * 100 / o.vitals.health.max < 40
                        })
                })
                .collect()
        } else {
            Vec::new()
        };
        if !wards.is_empty() {
            let ward = wards[rng.below(wards.len() as u32) as usize];
            commands.entity(monster).insert((Protects(ward), Decided));
        } else if kind.temper.wary && rng.below(100) < intelligence {
            commands.entity(monster).insert((Fights { waits: true }, Decided));
        }
    }
}

/// Anything else fights: strikes someone, or closes in to.
pub(super) fn default_fight(
    mut commands: Commands,
    actors: Query<Entity, (With<Acting>, With<MonsterTurn>, Without<Decided>)>,
) {
    for monster in &actors {
        commands.entity(monster).insert((Fights { waits: false }, Decided));
    }
}

/// A monster's conditions as its action begins (retail
/// `GetConditionInterrupt`): insanity may turn it on its friends this round,
/// fear send it running unannounced, or another condition take its action.
pub(super) fn interrupt(
    mut commands: Commands,
    mut actors: Query<
        (
            Entity,
            &mut MonsterTurn,
            (Option<&Retreats>, Option<&CastsSpell>, Has<Breathes>, Has<Fights>),
        ),
        (With<Acting>, With<Decided>, Without<Resumes>),
    >,
    mut bodies: Query<Body>,
    fight: Single<(&mut Battle, Option<&Combatants>)>,
    stand: Single<&Position, With<PartyBody>>,
    roster: Single<&PartyRoster>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    use super::Interrupt;
    let (mut battle, combatants) = fight.into_inner();
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    let eye = PartyBody::eye(*stand);
    let species = &content.species;
    for (monster, mut turn, (retreats, casts, breathes, fights)) in &mut actors {
        let body = bodies.get(monster).unwrap();
        let me = body.monster.unwrap().clone();
        let kind = &species[me.species];
        let (nearest, _) = bearing(&body, eye, species);
        let (feet, vitals) = (body.feet(eye), body.vitals.clone());
        let mine = standing(&body);
        let against_foes = match casts {
            Some(cast) => content.spells[cast.spell].aimed_at_enemies(),
            None => breathes || fights,
        };
        let friendly_member = roster.iter().any(|member| {
            bodies.get(member).is_ok_and(|b| toward(mine, &b) == Stance::Friendly)
        });
        let strikes_friend = kind.attacks.iter().any(|attack| {
            let reach = attack.reach.metres();
            (nearest <= reach && friendly_member)
                || combatants.iter().any(|&other| {
                    other != monster
                        && bodies.get(other).is_ok_and(|o| {
                            toward(mine, &o) == Stance::Friendly
                                && me.gap(feet, o.monster.as_ref().unwrap(), o.feet(eye)) <= reach
                        })
                })
        });
        let fleeing = retreats.is_some_and(|r| r.conduct == conduct::Conduct::Flee);
        let interrupted = conduct::interrupt(
            &mut rng,
            &vitals,
            kind,
            fleeing,
            strikes_friend,
            turn.berserk,
            against_foes,
        );
        let mut entity = commands.entity(monster);
        match interrupted {
            None | Some(Interrupt::Turncoat) => {}
            // Insanity turns it on its friends this round.
            Some(Interrupt::Berserk) => {
                turn.berserk = true;
                entity
                    .remove::<(Retreats, CastsSpell, Breathes, Protects)>()
                    .insert(Fights { waits: false });
            }
            // Fear sends it running, without its flight's notice.
            Some(Interrupt::Panics) => {
                let flee = Retreats { conduct: conduct::Conduct::Flee, quiet: true };
                entity.remove::<(CastsSpell, Breathes, Protects, Fights)>().insert((Fleeing, flee));
            }
            Some(interrupt) => {
                entity
                    .remove::<(Retreats, CastsSpell, Breathes, Protects, Fights, Decided, Acting)>(
                    );
                if interrupt == Interrupt::Freed {
                    bodies.get_mut(monster).unwrap().vitals.conditions.lift(Condition::Webbed);
                }
                news.write(CombatNews::Interrupted { monster, interrupt });
                turn.end();
                battle.wait = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
            }
        }
    }
}

/// The action over: the monster holds the fight for its pause, unless it
/// held back and did nothing.
fn done(commands: &mut Commands, monster: Entity, battle: Option<&mut Battle>) {
    commands.entity(monster).remove::<(Decided, Resumes, Acting)>();
    if let Some(battle) = battle {
        battle.wait = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
    }
}

/// Run, withdraw or hold back: one that cannot walk, or has moved this
/// round, waits instead (retail falls back on `WAIT` when the move fails).
pub(super) fn retreat(
    mut commands: Commands,
    mut actors: Query<(Entity, &Retreats, &mut MonsterTurn)>,
    bodies: Query<Body>,
    mut battle: Single<&mut Battle>,
    stand: Single<&Position, With<PartyBody>>,
    content: If<Res<Catalogs>>,
    mut news: MessageWriter<CombatNews>,
) {
    let eye = PartyBody::eye(*stand);
    for (monster, retreats, mut turn) in &mut actors {
        commands.entity(monster).remove::<Retreats>();
        turn.end();
        if retreats.conduct == conduct::Conduct::Hold {
            done(&mut commands, monster, None);
            continue;
        }
        done(&mut commands, monster, Some(&mut battle));
        let body = bodies.get(monster).unwrap();
        if !body.walker.is_some_and(|w| w.speed > 0.0) || turn.moved {
            turn.waited = true;
            continue;
        }
        turn.moved = true;
        let kind = &content.species[body.monster.as_ref().unwrap().species];
        let metres = approach::move_metres(body.vitals, kind, false);
        let away = approach::away_from(body.feet(eye), eye, metres);
        commands.entity(monster).insert(approach::CombatMove {
            budget: metres,
            moved: 0.0,
            goal: approach::MoveGoal::Retreat(away),
        });
        if !retreats.quiet {
            news.write(match retreats.conduct {
                conduct::Conduct::Flee => CombatNews::Flees { monster },
                _ => CombatNews::Withdraws { monster },
            });
        }
    }
}

/// Guard the fellow chosen, this round.
pub(super) fn protect(
    mut commands: Commands,
    mut actors: Query<(Entity, &Protects, &mut MonsterTurn)>,
    mut battle: Single<&mut Battle>,
) {
    for (monster, protects, mut turn) in &mut actors {
        turn.end();
        commands.entity(monster).remove::<Protects>().insert(Guarding(protects.0));
        done(&mut commands, monster, Some(&mut battle));
    }
}

/// Make the special attack (retail `ExecuteMonsterSpecialAttack`): an act
/// reaching its victims, or a summoning calling its group up; the cooldown
/// starts and it costs a tenth of the monster's stamina.
pub(super) fn start_special(
    mut commands: Commands,
    mut actors: Query<(Entity, &Breathes, &mut MonsterTurn)>,
    mut bodies: Query<Body>,
    mut battle: Single<&mut Battle>,
    definitions: Res<Definitions>,
    mut news: MessageWriter<CombatNews>,
    mut summons: MessageWriter<summon::Summon>,
) {
    for (monster, breathes, mut turn) in &mut actors {
        turn.end();
        turn.special = true;
        battle.attacked(true);
        news.write(CombatNews::Special { monster, kind: breathes.kind });
        // A summoning calls its group up rather than landing on anyone.
        if let Some(species) = breathes.kind.summons() {
            summons.write(summon::Summon { summoner: monster, species });
        } else if let Some(uses) = definitions.special(breathes.kind) {
            let act = commands.spawn((Deed::new(Act::Fight), Uses(uses), breathes.strength)).id();
            let victims: Vec<_> = breathes.victims.iter().map(|&victim| (victim, None)).collect();
            reach(&mut commands, act, &victims);
        }
        commands.entity(monster).remove::<Breathes>().insert(SpecialCooldown(breathes.cooldown));
        let mut body = bodies.get_mut(monster).unwrap();
        let tired = body.vitals.stamina.max / 10;
        body.vitals.tire(tired);
        done(&mut commands, monster, Some(&mut battle));
    }
}

/// Cast the spell chosen (retail `MonsterCastsSpell`): a cast reaching whom
/// its aim takes, as the monster saw the fight; a blast whoever stands
/// within its radius of where it bursts. The cast stages roll it.
pub(super) fn start_monster_cast(
    mut commands: Commands,
    mut actors: Query<(Entity, &CastsSpell, &mut MonsterTurn)>,
    mut bodies: Query<Body>,
    mut battle: Single<&mut Battle>,
    party: Single<(&Position, &PartyRoster), With<PartyBody>>,
    levels: Query<(), With<Level>>,
    obstacles: WorldObstacles,
    content: If<Res<Catalogs>>,
) {
    let (position, roster) = *party;
    let eye = PartyBody::eye(position);
    for (monster, cast, mut turn) in &mut actors {
        turn.end();
        battle.attacked(true);
        let spell = &content.spells[cast.spell];
        let victims = match monster_spells::victims(spell, cast.aim, &cast.scene) {
            Some(victims) => victims,
            None => {
                // A blast bursts on the monster aimed at, or on the party.
                let (centre, origin) = match cast.aim {
                    SpellAim::Monster(target) => match bodies.get(target) {
                        Ok(body) => (body.feet(eye), body.eye(eye)),
                        Err(_) => (PartyBody::feet_below(eye), eye),
                    },
                    _ => (PartyBody::feet_below(eye), eye),
                };
                let side = bodies.get(monster).unwrap().side.0;
                crate::game::hits::reach::radius(
                    centre,
                    origin,
                    spell.radius + spell.radius_per_power * cast.power as f32,
                    crate::game::hits::reach::Recipients::Opponents(side),
                    Some((eye, roster)),
                    &mut bodies,
                    &levels,
                    &content.species,
                    &obstacles,
                )
            }
        };
        let aim = match cast.aim {
            SpellAim::Monster(entity) => Aim::Monster(entity),
            SpellAim::Member(entity) => Aim::Member(entity),
            SpellAim::Party | SpellAim::Foes => match victims.first() {
                Some(&entity) if roster.contains(&entity) => Aim::Member(entity),
                Some(&entity) => Aim::Monster(entity),
                None => Aim::Monster(monster),
            },
        };
        let caster = bodies.get(monster).unwrap();
        let level = content.species[caster.monster.as_ref().unwrap().species].level;
        let act = commands
            .spawn((
                Cast { spell: spell.id, context: UseContext::Combat },
                MonsterBudget { budget: cast.budget, level, aim },
                CastBy(monster),
                Power(cast.power),
                Target(None),
            ))
            .id();
        let victims: Vec<_> = victims.into_iter().map(|victim| (victim, None)).collect();
        reach(&mut commands, act, &victims);
        commands.entity(monster).remove::<CastsSpell>();
        done(&mut commands, monster, Some(&mut battle));
    }
}
