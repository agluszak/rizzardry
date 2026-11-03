//! Turns in a round. Accepting a round gives every actor its [`Turn`]: its
//! initiative, and the attacks it has with each weapon slot; a member's
//! order becomes what they are [`Ordered`] to do. When an actor's phase
//! comes ([`Acting`]) its turn is revalidated ([`begin_turns`]); one under
//! orders follows them ([`follow_orders`]), one without decides for itself
//! ([`super::ai`]); either way it ends up with one action component
//! ([`CurrentAction`]). Its conditions may then take the action off it
//! ([`interrupt`]), and each action has the system that makes it: the
//! attack systems ([`super::swings`]), [`start_casts`], [`start_specials`],
//! [`retreat`] and [`protect`].

use bevy::prelude::*;
use enum_map::EnumMap;

use super::attack::phase;
use super::fighters::{Arms, BlindFighting, Brawn, Initiative, Stride, reaches};
use super::{
    ACTION_SECONDS, Battle, CombatNews, Combatant, PartyMove, Pause, Round, RoundPhase, conduct,
    summon,
};
use crate::content::characters::Trait;
use crate::content::species::SpecialAttackKind;
use crate::content::spells::SpellId;
use crate::game::characters::{Character, Condition, Vitals};
use crate::game::data::Catalogs;
use crate::game::hits::definitions::{Definitions, Uses};
use crate::game::hits::spells::reach;
use crate::game::hits::{Act, By, Deed};
use crate::game::inventory::{Carried, PartyInventory, StackId, WeaponSlot};
use crate::game::magic::Strength;
use crate::game::magic::casting::{
    Cast, Chosen, MagicChoice, MagicNews, Refusal, UseContext, in_cone, reaches as spell_reaches,
};
use crate::game::party::Order;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::footing::Footing;
use crate::game::world::hostility::{self, Loyalty, Side, Stance};
use crate::game::world::navigation::Walker;
use crate::game::world::obstacles::WorldObstacles;

/// An actor's accepted work and history this round: when it acts next,
/// the attacks each weapon slot has left (retail `hand_attack_values`; a
/// monster's all in its primary slot), the slot due to attack next (retail
/// `current_hand`), the weapons the attacks were accepted with, and what
/// the round has done to it.
#[derive(Component, Clone, Debug)]
pub struct Turn {
    /// When it acts next, 1 to 100; 0 when done for the round.
    pub phase: u32,
    pub attacks: EnumMap<WeaponSlot, u32>,
    pub due: WeaponSlot,
    pub weapons: EnumMap<WeaponSlot, Option<StackId>>,
    /// A Samurai's Lightning Strike, readied for the primary slot's next
    /// attack (retail `extra_swings`), by their Samurai level.
    pub lightning: Option<u8>,
    /// It has made its move this round.
    pub moved: bool,
    /// It made its special attack this round.
    pub special: bool,
    /// Insanity turned it on its friends this round.
    pub berserk: bool,
    /// It held back and rests.
    pub waited: bool,
}

impl Turn {
    /// Done for the round: nothing left to act with.
    pub fn end(&mut self) {
        self.phase = 0;
        self.attacks = EnumMap::default();
    }
}

/// What a member was ordered to do this round, accepted as the round
/// began: later edits to their order wait for the next round.
#[derive(Component, Clone, Copy, Debug)]
pub struct Ordered(pub Action);

/// A member's accepted action.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    /// A Berserk order is an attack too.
    Attack {
        berserk: bool,
    },
    Defend,
    /// A spell or item use, revalidated and paid when its initiative comes.
    Magic(MagicChoice),
    Breathe(Entity),
}

impl Ordered {
    /// The accepted action stands until the next round: spending or losing
    /// a turn does not end defending; a refused spell starts it.
    pub fn defends(&self) -> bool {
        self.0 == Action::Defend
    }
}

/// The actor whose initiative has come this step, to act now.
#[derive(Component, Debug)]
pub struct Acting;

/// One executable decision. Alternatives are mutually exclusive by construction;
/// accepted player orders remain separate because later edits cannot change them.
#[derive(Component, Debug)]
pub enum CurrentAction {
    Fight(Fights),
    Retreat(Retreats),
    Protect(Entity),
    Special(Breathes),
    Magic(MagicChoice),
}

/// The actor goes on with its turn: it acts again later in the round after
/// moving, deciding nothing anew.
#[derive(Component, Debug)]
pub struct Resumes;

/// Strike someone, or close in to; `waits` when it may hold back instead;
/// `berserk` for a Berserk order's fury.
#[derive(Debug)]
pub struct Fights {
    pub waits: bool,
    pub berserk: bool,
}

/// Run, withdraw or hold back; `quiet` for a flight fear forced, which
/// retail does not announce.
#[derive(Debug)]
pub struct Retreats {
    pub conduct: conduct::Conduct,
    pub quiet: bool,
}

/// Make a special attack, or a breath: an act of this definition at this
/// strength toward `toward`, catching whoever stands in its cone, or a
/// summoning of a species; it costs stamina, and a monster's cools down.
#[derive(Debug)]
pub struct Breathes {
    pub special: Option<SpecialAttackKind>,
    pub definition: Option<crate::game::hits::definitions::DefinitionId>,
    pub summons: Option<u32>,
    pub strength: Strength,
    pub toward: Vec3,
    pub cost: u32,
    pub cooldown: u8,
}

/// A monster guarding a fellow this round (retail `ACTION_PROTECT`).
#[derive(Component, Debug)]
#[relationship(relationship_target = Guards)]
pub struct Guarding(pub Entity);

/// How many blows a guard has taken in its ward's place this round (retail
/// `interception_count`), and how many it may.
#[derive(Component, Debug, Default)]
pub struct Interceptions {
    pub taken: u32,
    pub allowed: u32,
}

/// A monster's guards this round, in the order they took it up, which also
/// fixes the order they try to intercept.
#[derive(Component, Debug)]
#[relationship_target(relationship = Guarding)]
pub struct Guards(Vec<Entity>);

impl Guards {
    pub fn entities(&self) -> &[Entity] {
        &self.0
    }
}

/// How often a member has tried to spot an attacker this round.
#[derive(Component, Debug, Default)]
pub struct Spotted(pub u32);

/// A monster running for its life (retail `pCombat->advancing`), for the
/// rest of the fight.
#[derive(Component, Debug)]
pub struct Fleeing;

/// Rounds a monster still waits before its next special attack (retail
/// `pCombat->special_cooldown`).
#[derive(Component, Debug)]
pub struct SpecialCooldown(pub u8);

/// The fight is over: what its rounds left on the members and monsters goes
/// with it.
#[allow(clippy::type_complexity)]
pub fn clear_fight(left: On<Remove<Combatant>>, mut commands: Commands) {
    commands.entity(left.entity).try_remove::<(
        Turn,
        Ordered,
        Acting,
        CurrentAction,
        Resumes,
        Guarding,
        Guards,
        Interceptions,
        Spotted,
        Fleeing,
        SpecialCooldown,
        super::approach::CombatMove,
    )>();
}

/// Accept a round (retail `BeginCombatExecution`): lock each member's order
/// and weapons, and everyone's initiative and attacks, the party first,
/// then the combatants in order. Pacing delays only its execution.
pub fn accept_round(
    commands: &mut Commands,
    (battle, round, party_move): (&mut Battle, &mut Round, &mut PartyMove),
    combatants: &[Entity],
    roster: &PartyRoster,
    bodies: &Query<(&Vitals, &Arms, &Initiative, Option<(&Character, &Order)>)>,
    rng: &mut GameRng,
) {
    battle.round += 1;
    let surprised = battle.opening.take().is_some();
    let pace = if surprised { None } else { battle.party_pace };
    round.pace = pace;
    round.attacked = false;
    // Samurai ready their Lightning Strike with a sword before anyone's
    // initiative is rolled (retail `BeginCombatExecution`).
    let lightning: Vec<Option<u8>> = roster
        .iter()
        .map(|member| {
            let Ok((_, arms, _, Some((character, order)))) = bodies.get(member) else {
                return None;
            };
            (!surprised
                && *order == Order::Attack
                && character.has_trait(Trait::LightningStrike)
                && arms.of(WeaponSlot::Primary).any(|arm| arm.sword)
                && rng.below(100) < u32::from(character.scale_by_profession(12.0) as u8))
            .then(|| character.profession_levels[character.profession])
        })
        .collect();
    for &actor in combatants {
        let lightning = roster.get_index_of(&actor).and_then(|seat| lightning[seat]);
        let Ok((vitals, arms, initiative, ordered)) = bodies.get(actor) else { continue };
        // A member turned on the party attacks, whatever its order; one
        // who can no longer berserk attacks as usual.
        let action = ordered.map(|(character, order)| match *order {
            _ if vitals.conditions.has(Condition::Turncoat) => Action::Attack { berserk: false },
            Order::Attack => Action::Attack { berserk: false },
            // A Berserk order needs the trait, an arm, and a primary for
            // close quarters (retail `CanCharacterBerserk`).
            Order::Berserk => Action::Attack {
                berserk: character.has_trait(Trait::Berserk)
                    && !arms.0.is_empty()
                    && arms
                        .of(WeaponSlot::Primary)
                        .all(|arm| arm.reach <= crate::content::species::Reach::Short),
            },
            Order::Defend => Action::Defend,
            Order::Magic(choice) => Action::Magic(choice),
            Order::Breathe(target) => Action::Breathe(target),
        });
        let mut turn = Turn {
            phase: 0,
            attacks: EnumMap::default(),
            due: WeaponSlot::Primary,
            weapons: arms.weapons(),
            lightning,
            moved: false,
            special: false,
            berserk: false,
            waited: false,
        };
        // An attacker opens with the slot with the most attacks, its arm's
        // initiative counting (retail `AssignCombatPhases`).
        if vitals.can_fight() {
            match action {
                None | Some(Action::Attack { .. }) => {
                    if let Some(slot) = arms.opening() {
                        turn.due = slot;
                        turn.attacks = EnumMap::from_fn(|slot| arms.attacks(slot));
                        turn.phase = phase(rng, initiative.0 + arms.initiative(slot));
                    }
                }
                Some(Action::Magic(_) | Action::Breathe(_)) => {
                    turn.phase = phase(rng, initiative.0);
                }
                Some(Action::Defend) => {}
            }
        }
        let mut entity = commands.entity(actor);
        entity.insert(turn).remove::<(Guarding, Interceptions)>();
        match action {
            Some(action) => {
                entity.insert((Ordered(action), Spotted(0)));
            }
            None => {
                entity.remove::<Ordered>();
            }
        }
    }
    // The party's own move takes the phase its slowest living member
    // allows, by the initiative its actions use, on a step of ten (retail
    // `InitializePartyMovementPhase`).
    *party_move = match pace {
        Some(pace) => {
            let latest = roster
                .iter()
                .filter_map(|member| bodies.get(member).ok())
                .filter(|(vitals, ..)| !vitals.is_dead())
                .map(|(_, _, initiative, _)| phase(rng, initiative.0))
                .max()
                .unwrap_or(90);
            let phase = (latest + 5) - (latest + 5) % 10;
            PartyMove::Pending { pace, phase: phase.clamp(10, 100) }
        }
        None => PartyMove::None,
    };
    round.phase = RoundPhase::Accepted;
}

/// An actor whose action has come, revalidated (retail
/// `ExecuteCharacterAction`, `StartMonsterCombatAction`): one no longer
/// in the fight or able, or whose weapons changed since its attacks were
/// accepted, loses its turn; defending takes no turn. One going on with
/// its turn after moving, or with attacks already spent, fights on
/// without deciding again.
pub(super) fn begin_turns(
    mut commands: Commands,
    mut actors: Query<
        (Entity, &mut Turn, &Vitals, &Arms, Has<Combatant>, Option<&Ordered>),
        (With<Acting>, Without<CurrentAction>),
    >,
) {
    for (actor, mut turn, vitals, arms, combatant, ordered) in &mut actors {
        // The weapons accepted, or nothing where one was used up.
        let weapons_hold =
            arms.weapons().iter().all(|(slot, &now)| now.is_none() || now == turn.weapons[slot]);
        let able = combatant
            && vitals.can_fight()
            && match ordered.map(|ordered| ordered.0) {
                Some(Action::Attack { .. }) => weapons_hold,
                Some(Action::Defend) => false,
                _ => true,
            };
        if !able {
            turn.end();
            commands.entity(actor).remove::<Acting>();
            continue;
        }
        if ordered.is_none()
            && (turn.attacks[WeaponSlot::Primary] != arms.attacks(WeaponSlot::Primary)
                || turn.moved)
        {
            commands
                .entity(actor)
                .insert((CurrentAction::Fight(Fights { waits: false, berserk: false }), Resumes));
        }
    }
}

/// A member under orders follows them: an attack, a cast, or a breath at
/// the foe chosen, costing a fifth of their stamina. One who cannot
/// breathe, or whose foe is out of the cone, defends instead.
#[allow(clippy::type_complexity)]
pub(super) fn follow_orders(
    mut commands: Commands,
    mut actors: Query<
        (Entity, &mut Ordered, &mut Turn, &Character, &Vitals, &Footing),
        (With<Acting>, Without<CurrentAction>),
    >,
    targets: Query<(&Vitals, &Footing)>,
    fight: Single<Option<&super::Combatants>, With<Battle>>,
    obstacles: WorldObstacles,
    content: If<Res<Catalogs>>,
    definitions: Res<Definitions>,
) {
    let combatants = fight.map_or(&[][..], super::Combatants::entities);
    for (actor, mut ordered, mut turn, character, vitals, at) in &mut actors {
        let mut entity = commands.entity(actor);
        match ordered.0 {
            Action::Attack { berserk } => {
                entity.insert(CurrentAction::Fight(Fights { waits: false, berserk }));
            }
            Action::Magic(choice) => {
                entity.insert(CurrentAction::Magic(choice));
            }
            Action::Breathe(target) => {
                let cone = &content.spells[SpellId::SPECIAL_ATTACK_CONE];
                let aimed = combatants.contains(&target)
                    && targets.get(target).is_ok_and(|(theirs, there)| {
                        !theirs.is_dead()
                            && spell_reaches(at, there, cone.reach.metres(), &obstacles)
                    });
                if !super::breath::available(character, vitals) || !aimed {
                    ordered.0 = Action::Defend;
                    turn.end();
                    entity.remove::<Acting>();
                    continue;
                }
                let toward = targets.get(target).unwrap().1.feet;
                entity.insert((CurrentAction::Special(Breathes {
                    special: None,
                    definition: Some(definitions.breath()),
                    summons: None,
                    strength: crate::game::hits::specials::strength(
                        character.level(),
                        vitals.stamina,
                    ),
                    toward,
                    cost: vitals.stamina.max / 5,
                    cooldown: 0,
                }),));
            }
            Action::Defend => {
                turn.end();
                entity.remove::<Acting>();
            }
        }
    }
}

/// An actor's conditions as its action begins (retail
/// `GetConditionInterrupt`): insanity may turn it on its friends this round,
/// fear send a monster running unannounced, or another condition take its
/// action.
#[allow(clippy::type_complexity)]
pub(super) fn interrupt(
    mut commands: Commands,
    mut actors: Query<
        (
            Entity,
            &mut Turn,
            (&Brawn, &BlindFighting, &Arms, &Footing, &Side, &Loyalty),
            &CurrentAction,
            (Has<Ordered>, Option<&Carried>),
        ),
        (With<Acting>, With<CurrentAction>, Without<Resumes>),
    >,
    mut vitals: Query<&mut Vitals, With<Acting>>,
    bodies: Query<(&Vitals, &Footing, &Side, &Loyalty), Without<Acting>>,
    fight: Single<(&mut Pause, Option<&super::Combatants>), With<Battle>>,
    formation: Single<&crate::game::party::formation::Formation>,
    inventory: Single<&PartyInventory>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    use super::Interrupt;
    let (mut pause, combatants) = fight.into_inner();
    let combatants = combatants.map_or(&[][..], super::Combatants::entities);
    for (
        actor,
        mut turn,
        (brawn, blind_fighting, arms, at, side, loyalty),
        action,
        (ordered, carried),
    ) in &mut actors
    {
        let Ok(mut own) = vitals.get_mut(actor) else { continue };
        let against_foes = match action {
            CurrentAction::Magic(choice) => choice
                .source
                .spell_id(carried.unwrap_or(&Carried::default()), &inventory, &content.items)
                .is_some_and(|spell| content.spells[spell].aimed_at_enemies()),
            _ => matches!(action, CurrentAction::Special(_) | CurrentAction::Fight(_)),
        };
        // Whether one of its arms reaches a friend it sees: a living body
        // on its side.
        let mine = (side.0, *loyalty);
        let strikes_friend = arms.0.iter().any(|arm| {
            combatants.iter().copied().any(|other| {
                other != actor
                    && bodies.get(other).is_ok_and(|(theirs, there, their_side, their_loyalty)| {
                        !theirs.is_dead()
                            && hostility::between(mine, (their_side.0, *their_loyalty))
                                == Stance::Friendly
                            && reaches(&formation, (actor, at), (other, there), arm.reach)
                    })
            })
        });
        let fleeing =
            matches!(action, CurrentAction::Retreat(r) if r.conduct == conduct::Conduct::Flee);
        let interrupted = conduct::interrupt(
            &mut rng,
            &own,
            (brawn.0, blind_fighting.0.is_some(), !ordered),
            fleeing,
            strikes_friend,
            turn.berserk,
            against_foes,
        );
        let mut entity = commands.entity(actor);
        match interrupted {
            None | Some(Interrupt::Turncoat) => {}
            // Insanity turns it on its friends this round.
            Some(Interrupt::Berserk) => {
                turn.berserk = true;
                entity.insert(CurrentAction::Fight(Fights { waits: false, berserk: false }));
            }
            // Fear sends it running, without its flight's notice.
            Some(Interrupt::Panics) => {
                let flee = CurrentAction::Retreat(Retreats {
                    conduct: conduct::Conduct::Flee,
                    quiet: true,
                });
                entity.insert((Fleeing, flee));
            }
            Some(interrupt) => {
                entity.remove::<(CurrentAction, Acting)>();
                if interrupt == Interrupt::Freed {
                    own.conditions.lift(Condition::Webbed);
                }
                news.write(CombatNews::Interrupted { actor, interrupt });
                turn.end();
                pause.0 = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
            }
        }
    }
}

/// The action over: the actor holds the fight for its pause, unless it held
/// back and did nothing.
fn done(commands: &mut Commands, actor: Entity, pause: Option<&mut Pause>) {
    commands.entity(actor).remove::<(CurrentAction, Resumes, Acting)>();
    if let Some(pause) = pause {
        pause.0 = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
    }
}

/// Run, withdraw or hold back: a monster that cannot walk, or has moved
/// this round, waits instead (retail falls back on `WAIT` when the move
/// fails).
pub(super) fn retreat(
    mut commands: Commands,
    mut actors: Query<(
        Entity,
        &CurrentAction,
        &mut Turn,
        &Stride,
        &Vitals,
        &Footing,
        Option<&Walker>,
    )>,
    mut pause: Single<&mut Pause>,
    roster: Single<&PartyRoster>,
    footings: Query<&Footing>,
    mut news: MessageWriter<CombatNews>,
) {
    let party_eye = roster.iter().find_map(|member| footings.get(member).ok()).map(|at| at.eye);
    for (monster, retreats, mut turn, stride, vitals, at, walker) in &mut actors {
        let CurrentAction::Retreat(retreats) = retreats else { continue };
        commands.entity(monster).remove::<CurrentAction>();
        turn.end();
        if retreats.conduct == conduct::Conduct::Hold {
            done(&mut commands, monster, None);
            continue;
        }
        done(&mut commands, monster, Some(&mut pause));
        if !walker.is_some_and(|w| w.speed > 0.0) || turn.moved {
            turn.waited = true;
            continue;
        }
        turn.moved = true;
        let metres = super::approach::move_metres(vitals, stride.0, false);
        let away = super::approach::away_from(at.feet, party_eye.unwrap_or(at.feet), metres);
        commands.entity(monster).insert(super::approach::CombatMove {
            budget: metres,
            moved: 0.0,
            goal: super::approach::MoveGoal::Retreat(away),
        });
        if !retreats.quiet {
            news.write(match retreats.conduct {
                conduct::Conduct::Flee => CombatNews::Flees { monster },
                _ => CombatNews::Withdraws { monster },
            });
        }
    }
}

/// Guard the fellow chosen, this round; a guard takes blows in its ward's
/// place up to its attacks a round.
pub(super) fn protect(
    mut commands: Commands,
    mut actors: Query<(Entity, &CurrentAction, &mut Turn, &Arms)>,
    mut pause: Single<&mut Pause>,
) {
    for (guard, protects, mut turn, arms) in &mut actors {
        let CurrentAction::Protect(protects) = protects else { continue };
        turn.end();
        commands.entity(guard).remove::<CurrentAction>().insert((
            Guarding(*protects),
            Interceptions { taken: 0, allowed: arms.attacks(WeaponSlot::Primary) },
        ));
        done(&mut commands, guard, Some(&mut pause));
    }
}

/// Make the special attack or breath (retail `ExecuteMonsterSpecialAttack`,
/// special-attack entry 30): an act reaching whoever stands in its cone,
/// whatever their side, or a summoning calling its group up; it costs its
/// stamina, and a monster's cooldown starts.
pub(super) fn start_specials(
    mut commands: Commands,
    mut actors: Query<(Entity, &CurrentAction, &mut Turn, &Footing, &mut Vitals), With<Acting>>,
    bodies: Query<(&Vitals, &Footing), Without<Acting>>,
    fight: Single<(&mut Round, &mut Pause, Option<&super::Combatants>), With<Battle>>,
    obstacles: WorldObstacles,
    content: If<Res<Catalogs>>,
    mut news: MessageWriter<CombatNews>,
    mut summons: MessageWriter<summon::Summon>,
) {
    let (mut round, mut pause, combatants) = fight.into_inner();
    let combatants = combatants.map_or(&[][..], super::Combatants::entities);
    let cone = content.spells[SpellId::SPECIAL_ATTACK_CONE].reach.metres();
    for (actor, breathes, mut turn, from, mut own) in &mut actors {
        let CurrentAction::Special(breathes) = breathes else { continue };
        turn.end();
        turn.special = true;
        round.attacked(true);
        news.write(match breathes.special {
            Some(kind) => CombatNews::Special { monster: actor, kind },
            None => CombatNews::Breathed { actor },
        });
        if let Some(species) = breathes.summons {
            summons.write(summon::Summon { summoner: actor, species });
        } else if let Some(definition) = breathes.definition {
            let toward = breathes.toward - from.eye;
            let victims: Vec<_> = combatants
                .iter()
                .copied()
                .filter(|&other| other != actor)
                .filter(|&other| {
                    bodies.get(other).is_ok_and(|(theirs, there)| {
                        !theirs.is_dead()
                            && spell_reaches(from, there, cone, &obstacles)
                            && in_cone(toward, there.feet - from.eye)
                    })
                })
                .map(|victim| (victim, None))
                .collect();
            let act = commands
                .spawn((Deed::new(Act::Fight), Uses(definition), breathes.strength, By(actor)))
                .id();
            reach(&mut commands, act, &victims);
        }
        let mut entity = commands.entity(actor);
        entity.remove::<CurrentAction>();
        if breathes.special.is_some() {
            entity.insert(SpecialCooldown(breathes.cooldown));
        }
        own.tire(breathes.cost);
        done(&mut commands, actor, Some(&mut pause));
    }
}

/// The actor casts: a cast the cast stages revalidate and roll; a point
/// cast, which needs the whole world, at once. A member's cast refused
/// falls back to defending ([`defend_instead`]).
pub(super) fn start_casts(
    mut commands: Commands,
    mut actors: Query<(Entity, &CurrentAction, &mut Turn, Option<&Carried>)>,
    mut fight: Single<&mut Pause, With<Battle>>,
    inventory: Single<&PartyInventory>,
    content: If<Res<Catalogs>>,
    mut news: MessageWriter<MagicNews>,
) {
    for (actor, casts, mut turn, carried) in &mut actors {
        let CurrentAction::Magic(casts) = casts else { continue };
        let choice = *casts;
        turn.end();
        commands.entity(actor).remove::<CurrentAction>();
        done(&mut commands, actor, Some(&mut fight));
        let spell = choice.source.spell_id(
            carried.unwrap_or(&Carried::default()),
            &inventory,
            &content.items,
        );
        let Some(spell) = spell else {
            news.write(MagicNews::Refused { caster: actor, refusal: Refusal::NoItem });
            continue;
        };
        if crate::game::magic::pointcasts::spell(spell) {
            commands.queue(move |world: &mut World| {
                if crate::game::magic::execute_in_world(world, actor, choice).is_err()
                    && let Some(mut ordered) = world.get_mut::<Ordered>(actor)
                {
                    ordered.0 = Action::Defend;
                }
            });
            continue;
        }
        commands.spawn((Cast { spell, context: UseContext::Combat }, Chosen(choice), By(actor)));
    }
}

/// A member whose cast was refused defends instead (retail
/// `ExecuteCharacterAction`), the turn spent.
pub(super) fn defend_instead(mut news: MessageReader<MagicNews>, mut orders: Query<&mut Ordered>) {
    for news in news.read() {
        if let MagicNews::Refused { caster, .. } = *news
            && let Ok(mut ordered) = orders.get_mut(caster)
            && matches!(ordered.0, Action::Magic(_))
        {
            ordered.0 = Action::Defend;
        }
    }
}
