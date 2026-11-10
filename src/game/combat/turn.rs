//! Turns in a round. Accepting a round gives every actor its [`Turn`]: its
//! initiative, and the attacks it has with each weapon slot; a member's
//! order becomes what they are [`Ordered`] to do. When an actor's phase
//! comes it is [`Called`] and takes its turn ([`take_turn`]): revalidated,
//! one under orders follows them, one without decides for itself
//! ([`super::ai`]), and its conditions may take the action off it. What it
//! then does is said once ([`Perform`]) and carried out by the system for
//! it: a weapon attack by [`super::swings`], anything else by [`perform`].

use bevy::prelude::*;
use enum_map::EnumMap;

use super::attack::phase;
use super::{
    ACTION_SECONDS, Battle, CombatNews, Combatant, Interrupt, PartyMove, Pause, Round, RoundPhase,
    ai, conduct, summon,
};
use crate::content::characters::{Attribute, Trait};
use crate::content::species::SpecialAttackKind;
use crate::content::spells::SpellId;
use crate::game::actors::{Attacks, reaches};
use crate::game::characters::{Character, Condition, Vitals};
use crate::game::data::Catalogs;
use crate::game::hits::definitions::Definitions;
use crate::game::hits::effects::{Body, Strike};
use crate::game::hits::{Act, Deed, Release};
use crate::game::inventory::{Carried, PartyInventory, StackId, WeaponSlot};
use crate::game::magic::casting::{
    Cast, MagicChoice, MagicNews, Refusal, UseContext, in_cone, reaches as spell_reaches,
};
use crate::game::magic::{Strength, Utility};
use crate::game::party::Order;
use crate::game::party::formation::Formation;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::hostility::{self, Stance};
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
#[derive(Message, Clone, Copy, Debug)]
pub struct Called(pub Entity);

/// What an actor does with its turn, decided and not interrupted
/// ([`take_turn`]); the system for its kind of action carries it out.
#[derive(Message, Clone, Copy, Debug)]
pub struct Perform {
    pub actor: Entity,
    pub action: Chosen,
}

/// One executable decision. Accepted player orders remain separate
/// ([`Ordered`]) because later edits cannot change them.
#[derive(Clone, Copy, Debug)]
pub enum Chosen {
    Fight(Fights),
    Retreat(Retreats),
    Protect(Entity),
    Special(Breathes),
    Magic(MagicChoice),
}

/// Strike someone, or close in to; `waits` when it may hold back instead;
/// `berserk` for a Berserk order's fury.
#[derive(Clone, Copy, Debug)]
pub struct Fights {
    pub waits: bool,
    pub berserk: bool,
}

/// Run, withdraw or hold back; `quiet` for a flight fear forced, which
/// retail does not announce.
#[derive(Clone, Copy, Debug)]
pub struct Retreats {
    pub conduct: conduct::Conduct,
    pub quiet: bool,
}

/// Make a special attack, or a breath: a deed of this definition at this
/// strength toward `toward`, catching whoever stands in its cone, or a
/// summoning of a species; it costs stamina, and a monster's cools down.
#[derive(Clone, Copy, Debug)]
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
    bodies: &Query<(&Vitals, &Attacks, Option<(&Character, &Order)>)>,
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
            let Ok((_, arms, Some((character, order)))) = bodies.get(member) else {
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
        let Ok((vitals, arms, ordered)) = bodies.get(actor) else { continue };
        // A member turned on the party attacks, whatever its order; one
        // who can no longer berserk attacks as usual.
        let action = ordered.map(|(character, order)| match *order {
            _ if vitals.conditions.has(Condition::Turncoat) => Action::Attack { berserk: false },
            Order::Attack => Action::Attack { berserk: false },
            // A Berserk order needs the trait, an arm, and a primary for
            // close quarters (retail `CanCharacterBerserk`).
            Order::Berserk => Action::Attack {
                berserk: character.has_trait(Trait::Berserk)
                    && !arms.arms.is_empty()
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
                        turn.phase = phase(rng, arms.initiative + arms.slot_initiative(slot));
                    }
                }
                Some(Action::Magic(_) | Action::Breathe(_)) => {
                    turn.phase = phase(rng, arms.initiative);
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
                .map(|(_, arms, _)| phase(rng, arms.initiative))
                .max()
                .unwrap_or(90);
            let phase = (latest + 5) - (latest + 5) % 10;
            PartyMove::Pending { pace, phase: phase.clamp(10, 100) }
        }
        None => PartyMove::None,
    };
    round.phase = RoundPhase::Accepted;
}

/// The actor whose initiative has come takes its turn (retail
/// `ExecuteCharacterAction`, `StartMonsterCombatAction`). It is revalidated:
/// one no longer in the fight or able, or whose weapons changed since its
/// attacks were accepted, loses its turn; defending takes no turn. One
/// going on with its turn after moving, or with attacks already spent,
/// fights on. Otherwise a member follows its orders and anyone else decides
/// for itself ([`ai::decide`]); then its conditions may take the action off
/// it (retail `GetConditionInterrupt`): insanity may turn it on its friends
/// this round, fear send a monster running unannounced.
#[allow(clippy::type_complexity)]
pub(super) fn take_turn(
    mut commands: Commands,
    mut called: MessageReader<Called>,
    mut performs: MessageWriter<Perform>,
    mut bodies: Query<Body>,
    mut turns: Query<(&mut Turn, Option<&mut Ordered>), With<Combatant>>,
    minds: Query<ai::Mind>,
    laid: crate::game::actors::Laid,
    places: crate::game::actors::Places,
    fight: Single<(&mut Pause, Option<&super::Combatants>), With<Battle>>,
    party: Single<(Entity, &PartyRoster, &Formation, &PartyInventory)>,
    level: crate::game::world::level::PlayedLevel,
    obstacles: WorldObstacles,
    (content, definitions): (Res<Catalogs>, Res<Definitions>),
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<CombatNews>,
) {
    let (mut pause, combatants) = fight.into_inner();
    let combatants = combatants.map_or(&[][..], super::Combatants::entities);
    let (party, roster, formation, inventory) = *party;
    for &Called(actor) in called.read() {
        let Ok((mut turn, ordered)) = turns.get_mut(actor) else { continue };
        let readonly = bodies.as_readonly();
        let Ok(own) = readonly.get(actor) else { continue };
        let arms = own.attacks;
        let order = ordered.as_ref().map(|ordered| ordered.0);
        // The weapons accepted, or nothing where one was used up.
        let weapons_hold =
            arms.weapons().iter().all(|(slot, &now)| now.is_none() || now == turn.weapons[slot]);
        let able = own.vitals.can_fight()
            && match order {
                Some(Action::Attack { .. }) => weapons_hold,
                Some(Action::Defend) => false,
                _ => true,
            };
        if !able {
            turn.end();
            continue;
        }
        let fights_on = Chosen::Fight(Fights { waits: false, berserk: false });
        if order.is_none()
            && (turn.attacks[WeaponSlot::Primary] != arms.attacks(WeaponSlot::Primary)
                || turn.moved)
        {
            performs.write(Perform { actor, action: fights_on });
            continue;
        }
        let action = match order {
            Some(Action::Attack { berserk }) => Chosen::Fight(Fights { waits: false, berserk }),
            Some(Action::Magic(choice)) => Chosen::Magic(choice),
            // A breath at the foe chosen costs a fifth of their stamina;
            // one who cannot breathe, or whose foe is out of the cone,
            // defends instead.
            Some(Action::Breathe(target)) => {
                let cone = &content.spells[SpellId::SPECIAL_ATTACK_CONE];
                let aimed = readonly.get(target).ok().filter(|their| {
                    combatants.contains(&target)
                        && !their.vitals.is_dead()
                        && spell_reaches(
                            &places.of(actor),
                            &places.of(their.entity),
                            cone.reach.metres(),
                            &obstacles,
                        )
                });
                let breathes = own
                    .character
                    .filter(|character| {
                        super::breath::available(actor, character, own.vitals, &laid)
                    })
                    .zip(aimed);
                let Some((character, aimed)) = breathes else {
                    ordered.unwrap().0 = Action::Defend;
                    turn.end();
                    continue;
                };
                Chosen::Special(Breathes {
                    special: None,
                    definition: Some(definitions.breath()),
                    summons: None,
                    strength: crate::game::hits::specials::strength(
                        character.level(),
                        own.vitals.stamina,
                    ),
                    toward: places.of(aimed.entity).feet,
                    cost: own.vitals.stamina.max / 5,
                    cooldown: 0,
                })
            }
            Some(Action::Defend) => unreachable!("defending takes no turn"),
            None => match minds.get(actor) {
                Ok(mind) => {
                    let fight = ai::Fight {
                        bodies: &readonly,
                        laid: &laid,
                        places: &places,
                        party,
                        roster,
                        combatants,
                        far: level.view_distance().unwrap_or(0.0),
                        obstacles: &obstacles,
                        content: &content,
                        definitions: &definitions,
                    };
                    let (action, fleeing) = ai::decide(actor, &mind, &fight, &mut rng);
                    match (mind.fleeing, fleeing) {
                        (false, true) => {
                            commands.entity(actor).insert(Fleeing);
                        }
                        (true, false) => {
                            commands.entity(actor).remove::<Fleeing>();
                        }
                        _ => {}
                    }
                    action
                }
                Err(_) => fights_on,
            },
        };
        let against_foes = match action {
            Chosen::Magic(choice) => choice
                .source
                .spell_id(own.carried.unwrap_or(&Carried::default()), inventory, &content.items)
                .is_some_and(|spell| content.spells[spell].aimed_at_enemies()),
            _ => matches!(action, Chosen::Special(_) | Chosen::Fight(_)),
        };
        // Whether one of its arms reaches a friend it sees: a living body
        // on its side.
        let mine = own.standing();
        let strikes_friend = arms.arms.iter().any(|arm| {
            combatants.iter().copied().any(|other| {
                other != actor
                    && readonly.get(other).is_ok_and(|their| {
                        !their.vitals.is_dead()
                            && hostility::between(mine, their.standing()) == Stance::Friendly
                            && reaches(
                                formation,
                                (actor, &places.of(actor)),
                                (other, &places.of(their.entity)),
                                arm.reach,
                            )
                    })
            })
        });
        let fleeing = matches!(action, Chosen::Retreat(r) if r.conduct == conduct::Conduct::Flee);
        let interrupted = conduct::interrupt(
            &mut rng,
            own.vitals,
            (own.attributes[Attribute::Strength], arms.blind_fighting.is_some(), order.is_none()),
            fleeing,
            strikes_friend,
            turn.berserk,
            against_foes,
        );
        let action = match interrupted {
            None | Some(Interrupt::Turncoat) => action,
            // Insanity turns it on its friends this round.
            Some(Interrupt::Berserk) => {
                turn.berserk = true;
                fights_on
            }
            // Fear sends it running, without its flight's notice.
            Some(Interrupt::Panics) => {
                commands.entity(actor).insert(Fleeing);
                Chosen::Retreat(Retreats { conduct: conduct::Conduct::Flee, quiet: true })
            }
            Some(interrupt) => {
                if interrupt == Interrupt::Freed {
                    bodies.get_mut(actor).unwrap().vitals.conditions.lift(Condition::Webbed);
                }
                news.write(CombatNews::Interrupted { actor, interrupt });
                turn.end();
                pause.0 = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
                continue;
            }
        };
        performs.write(Perform { actor, action });
    }
}

/// Carry out what does not strike with a weapon (the attack systems do
/// that, [`super::swings`]), the action holding the fight for its pause:
///
/// - Run, withdraw or hold back: a monster that cannot walk, or has moved
///   this round, waits instead (retail falls back on `WAIT` when the move
///   fails); holding back takes no pause.
/// - Guard the fellow chosen this round, taking blows in its ward's place
///   up to its attacks a round.
/// - Make the special attack or breath (retail
///   `ExecuteMonsterSpecialAttack`, special-attack entry 30): a deed
///   reaching whoever stands in its cone, whatever their side, or a
///   summoning calling its group up; it costs its stamina, and a monster's
///   cooldown starts.
/// - Cast: an accepted cast for [`crate::game::magic::casting::cast`], or
///   for a point cast the utility magic it is.
#[allow(clippy::type_complexity)]
pub(super) fn perform(
    mut commands: Commands,
    mut performs: MessageReader<Perform>,
    mut actors: Query<(&mut Turn, Option<&Walker>)>,
    mut bodies: Query<Body>,
    fight: Single<(&mut Round, &mut Pause, Option<&super::Combatants>), With<Battle>>,
    party: Single<(Entity, &PartyRoster, &PartyInventory)>,
    places: crate::game::actors::Places,
    obstacles: WorldObstacles,
    content: Res<Catalogs>,
    mut news: MessageWriter<CombatNews>,
    (mut summons, mut released, mut casts, mut utilities, mut magic_news): (
        MessageWriter<summon::Summon>,
        MessageWriter<Release>,
        MessageWriter<Cast>,
        MessageWriter<Utility>,
        MessageWriter<MagicNews>,
    ),
) {
    let (mut round, mut pause, combatants) = fight.into_inner();
    let combatants = combatants.map_or(&[][..], super::Combatants::entities);
    let (party, _roster, inventory) = *party;
    for &Perform { actor, action } in performs.read() {
        let Ok((mut turn, walker)) = actors.get_mut(actor) else { continue };
        let Ok(mut own) = bodies.get_mut(actor) else { continue };
        if matches!(action, Chosen::Fight(_)) {
            continue;
        }
        turn.end();
        if !matches!(action, Chosen::Retreat(r) if r.conduct == conduct::Conduct::Hold) {
            pause.0 = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
        }
        match action {
            Chosen::Fight(_) => {}
            Chosen::Retreat(retreats) => {
                if retreats.conduct == conduct::Conduct::Hold {
                    continue;
                }
                if !walker.is_some_and(|w| w.speed > 0.0) || turn.moved {
                    turn.waited = true;
                    continue;
                }
                turn.moved = true;
                let metres = super::approach::move_metres(own.attacks, &own.vitals, false);
                let feet = places.of(actor).feet;
                let away = super::approach::away_from(feet, places.of(party).eye, metres);
                commands.entity(actor).insert(super::approach::CombatMove {
                    budget: metres,
                    moved: 0.0,
                    goal: super::approach::MoveGoal::Retreat(away),
                });
                if !retreats.quiet {
                    news.write(match retreats.conduct {
                        conduct::Conduct::Flee => CombatNews::Flees { monster: actor },
                        _ => CombatNews::Withdraws { monster: actor },
                    });
                }
            }
            Chosen::Protect(ward) => {
                commands.entity(actor).insert((
                    Guarding(ward),
                    Interceptions { taken: 0, allowed: own.attacks.attacks(WeaponSlot::Primary) },
                ));
            }
            Chosen::Special(breathes) => {
                turn.special = true;
                round.attacked(true);
                news.write(match breathes.special {
                    Some(kind) => CombatNews::Special { monster: actor, kind },
                    None => CombatNews::Breathed { actor },
                });
                own.vitals.tire(breathes.cost);
                if breathes.special.is_some() {
                    commands.entity(actor).insert(SpecialCooldown(breathes.cooldown));
                }
                let from = places.of(actor);
                if let Some(species) = breathes.summons {
                    summons.write(summon::Summon { summoner: actor, species });
                } else if let Some(definition) = breathes.definition {
                    let cone = content.spells[SpellId::SPECIAL_ATTACK_CONE].reach.metres();
                    let toward = breathes.toward - from.eye;
                    let victims = combatants
                        .iter()
                        .copied()
                        .filter(|&other| other != actor)
                        .filter(|&other| {
                            bodies.get(other).is_ok_and(|their| {
                                !their.vitals.is_dead()
                                    && spell_reaches(
                                        &from,
                                        &places.of(their.entity),
                                        cone,
                                        &obstacles,
                                    )
                                    && in_cone(toward, places.of(their.entity).feet - from.eye)
                            })
                        })
                        .map(|victim| (victim, None))
                        .collect();
                    let strike = Strike { definition, strength: breathes.strength, blow: None };
                    released.write(Release { deed: Deed::new(Act::Fight, strike), victims });
                }
            }
            Chosen::Magic(choice) => {
                let spell = choice.source.spell_id(
                    own.carried.as_deref().unwrap_or(&Carried::default()),
                    inventory,
                    &content.items,
                );
                let Some(spell) = spell else {
                    let refusal = Refusal::NoItem;
                    magic_news.write(MagicNews::Refused { caster: actor, refusal });
                    continue;
                };
                if crate::game::magic::pointcasts::spell(spell) {
                    utilities.write(Utility { caster: actor, spell, choice });
                    continue;
                }
                casts.write(Cast { caster: actor, spell, context: UseContext::Combat, choice });
            }
        }
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
