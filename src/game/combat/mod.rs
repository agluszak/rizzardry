//! Combat requests, encounter lifetime and round execution. Accepted actions
//! and round history live in `round`; attacks and magic settle their consequences
//! before the scheduler advances to the next actor.

pub mod approach;
pub mod attack;
mod blows;
pub mod breath;
mod conduct;
mod engage;
use crate::game::GameplaySet;
use crate::game::data::Catalogs;
pub use conduct::Interrupt;
mod monster_spells;
mod monster_turn;
mod round;
use round::{Guard, Round, RoundAction, accept_round};
pub mod summon;

use crate::game::world::level::LevelOwned;
use crate::game::world::level::PlayedLevel;
use crate::game::world::navigation::{Walker, feet};
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::perception::{party_distance, sight_threshold};
use crate::game::world::{ActorPose, ActorPoseItem};
use avian3d::prelude::Position;
use bevy::prelude::*;

use crate::content::items::AttackMode;
use crate::content::species::AttackWord;

use crate::content::species::{MonsterCatalog, Reach, Species};
use crate::game::characters::{Character, Condition, Vitals};
use crate::game::inventory::EquipSlot;
use crate::game::magic::casting::{self, MagicChoice, MagicNews};
use crate::game::magic::{Effect, LearnedSpells, MagicContext, SpellPoints, Victim};
use crate::game::party::Members;
use crate::game::party::Order;
use crate::game::party::body::PartyBody;
use crate::game::party::formation::{self, Formation};
use crate::game::party::recovery;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::time::GameClock;
use crate::game::world::damage::{MonsterDamage, MonsterQuery};
use crate::game::world::hostility;
use crate::game::world::identify::{GroupMember, GroupMembers, GroupRecognition, MonsterGroup};
use crate::game::world::population::Monster;
use attack::Armour;
use bevy::ecs::relationship::RelationshipTarget;

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<CombatNews>()
            .add_message::<hostility::Provocation>()
            .add_message::<CyclePartyMove>()
            .add_message::<StartRound>()
            .add_message::<summon::Summon>()
            .add_message::<CombatControl>()
            .add_message::<crate::game::camp::CampNews>()
            .add_message::<GiveOrder>()
            .add_message::<engage::Engaged>()
            .init_resource::<CombatSettings>()
            .init_resource::<casting::FlightOrder>()
            .add_observer(approach::end_move)
            .add_observer(approach::stop_walking)
            .add_systems(
                Update,
                (
                    give_orders,
                    // Defeat is settled before a fight starts,
                    settle_defeat,
                    // alerts, then engagement, then the opening,
                    engage::alert_factions.run_if(any_with_component::<Battle>),
                    engage::engage,
                    engage::open_battle,
                    control_combat,
                )
                    .chain()
                    .in_set(GameplaySet::Combat)
                    .run_if(crate::game::world::level::level_active),
            )
            // Rounds run on the fixed step: when each actor acts, and so
            // every roll, follows simulation time, not how often frames
            // come. Requests made in Update wait for the next step, and a
            // move the round starts or ends governs the party's input on
            // that same step. Spell missiles fly on it too: an impact lands
            // at the step its travel ends, and a defeat it causes is settled
            // before anyone else acts.
            .add_systems(
                FixedUpdate,
                (
                    crate::game::magic::deliver_spells,
                    settle_defeat,
                    begin_round,
                    run_battle,
                    finish_round,
                    summon::summon,
                    // and again after the round's damage.
                    settle_defeat,
                )
                    .chain()
                    .in_set(GameplaySet::Combat)
                    .after(crate::game::party::body::turn_party)
                    .before(bevy_tnua::TnuaUserControlsSystems)
                    .run_if(crate::game::world::level::level_active),
            )
            // The party walks on the fixed step; its move is measured on
            // every step, once physics has moved it, so how frames batch
            // those steps changes neither the ground counted nor where the
            // budget runs out.
            .add_systems(
                FixedPostUpdate,
                (track_party_move, approach::spend_moves)
                    .after(avian3d::prelude::PhysicsSystems::Writeback)
                    .run_if(crate::game::world::level::level_active),
            )
            // Monsters' moves likewise: headed before navigation plans the
            // step, charged once physics has walked it.
            .add_systems(
                FixedPreUpdate,
                approach::steer_moves
                    .before(bevy_landmass::LandmassSystems::SyncExistence)
                    .run_if(crate::game::world::level::level_active),
            );
    }
}

/// Seconds of simulation between actors (retail `combat_delay_ms`, 1000 by
/// default).
const ACTION_SECONDS: f32 = 1.0;
/// Seconds of simulation between rounds, so that a round in which nobody
/// can strike still lets monsters close in.
const ROUND_SECONDS: f32 = 1.0;
/// Rounds in which nobody could attack before the fight peters out.
const PASSIVE_ROUNDS: u32 = 2;

/// Retail's combat settings (`ResetGameplaySettings`): phased by default,
/// each round started by the player.
#[derive(Resource, Debug, Default)]
pub struct CombatSettings {
    pub continuous: bool,
}

/// How the party moves in a fight (retail Walk and Run party actions).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartyPace {
    Walk,
    Run,
}

/// Give a member their order for the coming rounds.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct GiveOrder {
    pub member: Entity,
    pub order: crate::game::party::Order,
}

/// Orders are given to the party's members during a fight.
fn give_orders(
    mut requests: MessageReader<GiveOrder>,
    battle: Option<Single<&Battle>>,
    roster: Single<&crate::game::party::roster::PartyRoster>,
    mut orders: Query<(&mut crate::game::party::Order, &Character, &Vitals)>,
) {
    use crate::content::characters::Trait;
    for &GiveOrder { member, order } in requests.read() {
        if battle.is_some()
            && roster.contains(&member)
            && let Ok((mut current, character, vitals)) = orders.get_mut(member)
            // Only those with the trait go berserk.
            && (order != Order::Berserk || character.has_trait(Trait::Berserk))
            && (!matches!(order, Order::Breathe(_)) || breath::available(character, vitals))
        {
            current.set_if_neq(order);
        }
    }
}

/// Choose, between rounds, whether the party moves in the next ones.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct CyclePartyMove;

/// The party's move this round.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum PartyMove {
    #[default]
    None,
    /// Waiting for its phase; members do not act before it.
    Pending { pace: PartyPace, phase: u8 },
    /// Under way: metres allowed and covered (retail `GetPartyMovementSpeed`:
    /// 10000 units walking, 15000 running).
    Moving { pace: PartyPace, budget: f32, moved: f32, last: Option<Vec3> },
}

/// Start the next round (Enter, or the round button).
#[derive(Message, Debug, Clone, Copy)]
pub struct StartRound;

/// Player intent; gameplay interprets the round button using the current battle.
#[derive(Message, Clone, Copy, Debug)]
pub enum CombatControl {
    RoundButton,
    ToggleContinuous,
}

fn control_combat(
    mut requests: MessageReader<CombatControl>,
    battle: Option<Single<&Battle>>,
    mut settings: ResMut<CombatSettings>,
    mut rounds: MessageWriter<StartRound>,
) {
    for request in requests.read() {
        match request {
            CombatControl::ToggleContinuous => settings.continuous = !settings.continuous,
            CombatControl::RoundButton => {
                if let Some(battle) = &battle {
                    if battle.executing() {
                        settings.continuous = !settings.continuous;
                    } else {
                        rounds.write(StartRound);
                    }
                }
            }
        }
    }
}

/// A monster taking part in a fight. Ending the fight ends the
/// participation, never the monster.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
#[relationship(relationship_target = Combatants)]
pub struct Combatant(pub Entity);

/// The monsters in a fight, in the order they joined (by location id),
/// maintained by Bevy from [`Combatant`].
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = Combatant)]
pub struct Combatants(Vec<Entity>);

impl Combatants {
    pub fn entities(&self) -> &[Entity] {
        &self.0
    }
}

/// The party's fight, on an entity owned by its level. Exists only while
/// there is one.
#[derive(Component, Debug, Default)]
#[require(Transform)]
pub struct Battle {
    pub round: u32,
    /// Accepted encounter history, consumed when the opening round is initialized.
    opening: Option<SurpriseCause>,
    phase: RoundPhase,
    this_round: Round,
    /// Experience from kills, shared at the end.
    experience: u32,
    kills: u32,
    passive_rounds: u32,
    /// The party's standing choice to walk or run in the next rounds.
    pub party_pace: Option<PartyPace>,
    party_move: PartyMove,
    /// Monsters running for their lives (retail `pCombat->advancing`).
    fleeing: Vec<Entity>,
    /// Rounds each monster still waits before its next special attack
    /// (retail `pCombat->special_cooldown`).
    cooldowns: Vec<(Entity, u8)>,
    /// Until the next action.
    wait: Timer,
}

/// Accepted work survives pacing/content waits without pretending the round
/// has already initialized its actors. Waiting cannot retain an executing phase.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum RoundPhase {
    #[default]
    Waiting,
    Accepted,
    Executing(u8),
}

impl Battle {
    /// A round is accepted or running, including its pacing delay.
    pub fn executing(&self) -> bool {
        self.phase != RoundPhase::Waiting
    }

    /// The effective action retained until the next round or battle removal.
    /// Spending or losing a turn does not end defense; a refused spell starts it.
    pub fn defends(&self, actor: Entity) -> bool {
        self.this_round
            .party
            .iter()
            .any(|work| work.actor == actor && matches!(work.action, RoundAction::Defend))
    }

    /// Whether the party may walk: in a fight, only during its own move.
    pub fn party_may_move(&self) -> bool {
        matches!(self.party_move, PartyMove::Moving { .. })
    }

    /// Whether the party's move is a run.
    pub fn party_running(&self) -> bool {
        matches!(self.party_move, PartyMove::Moving { pace: PartyPace::Run, .. })
    }

    /// A monster died to the party: its experience is shared at the end and
    /// it acts no more this round.
    pub fn credit_kill(
        &mut self,
        monster: Entity,
        experience: u32,
        news: &mut MessageWriter<CombatNews>,
    ) {
        self.experience += experience;
        self.kills += 1;
        if let Some(turn) = self.this_round.monsters.iter_mut().find(|turn| turn.actor == monster) {
            turn.end();
        }
        news.write(CombatNews::Killed { actor: monster });
    }

    /// Credit and announce the kills a spell's effects made, whether it
    /// landed at once or as a missile: the one place spell kills are counted.
    /// A member it killed is announced as one a monster's blow kills is.
    pub fn credit_kills(&mut self, effects: &[Effect], news: &mut MessageWriter<CombatNews>) {
        for effect in effects {
            let Effect::Damaged { victim, outcome, .. } = *effect else { continue };
            if !outcome.died {
                continue;
            }
            match victim {
                Victim::Monster(target) => self.credit_kill(target, outcome.experience, news),
                Victim::Member(member) => {
                    news.write(CombatNews::Killed { actor: member });
                }
            }
        }
    }

    /// How far through the round its play is, 0 to 1, while a round is
    /// being played.
    pub fn progress(&self) -> Option<f32> {
        match self.phase {
            RoundPhase::Waiting => None,
            RoundPhase::Accepted => Some(0.0),
            RoundPhase::Executing(step) => Some(f32::from(step.min(100)) / 100.0),
        }
    }
}

/// How an attack is named: by its mode's verb ("Swings", "Punches"), or for
/// a monster attack by its own word, with retail's weapon-name flag
/// deciding whether "at" follows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Announcement {
    Mode(AttackMode),
    Monster { word: AttackWord, mode: AttackMode, named: bool },
}

/// Why the party loses its opening turn (retail `RollCombatSurprise`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurpriseCause {
    Ambush,
    Running,
    Searching,
}

/// Automatic encounters already require a hostile that sees the party.
/// Test party awareness before combat turns the camera toward that attacker.
fn opening_surprise<'a>(
    seen: bool,
    shadow_hound: bool,
    camping: bool,
    running: bool,
    searching: bool,
    mut members: impl Iterator<Item = (&'a Character, &'a Vitals)>,
    rng: &mut GameRng,
) -> Option<SurpriseCause> {
    if !shadow_hound {
        if !seen {
            return Some(SurpriseCause::Ambush);
        }
        if camping {
            let guard = members.find(|(_, v)| v.lucid());
            let Some((profile, vitals)) = guard else { return Some(SurpriseCause::Ambush) };
            let senses = profile
                .stats(&vitals.enchantments)
                .attribute(crate::content::characters::Attribute::Senses);
            rng.below(1);
            if senses / 2 <= rng.below(100) {
                return Some(SurpriseCause::Ambush);
            }
        }
    }
    if (running || searching) && rng.below(100) < 20 {
        return Some(if running { SurpriseCause::Running } else { SurpriseCause::Searching });
    }
    None
}

/// What the fight tells the player.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum CombatNews {
    Began {
        nearest: Option<Entity>,
    },
    Surprised(SurpriseCause),
    Breathed {
        actor: Entity,
    },
    /// An attack begins (retail `StartCharacterAttack`,
    /// `AnnounceMonsterAttack`).
    Attack {
        attacker: Entity,
        target: Entity,
        how: Announcement,
        swings: u32,
        surprise: attack::Surprise,
    },
    /// A swing goes astray onto someone else (retail
    /// `AnnounceAccidentalStrike`).
    Fumble {
        attacker: Entity,
        victim: Victim,
    },
    /// What blows did besides damage.
    Effects(Vec<Effect>),
    /// A member's training paid off.
    Improved {
        member: Entity,
        skill: crate::content::characters::Skill,
    },
    /// What it did: hits (that landed, damage or not) and damage dealt.
    Result {
        attacker: Entity,
        target: Entity,
        hits: u32,
        damage: u32,
    },
    Killed {
        actor: Entity,
    },
    Won {
        kills: u32,
        experience_each: u32,
    },
    Lost,
    /// Nobody could reach anybody for a while.
    Over,
    /// A monster advances on the party, or charges it.
    Moves {
        monster: Entity,
        charge: bool,
    },
    /// A monster makes its special attack (retail `MonsterFleeAction`'s
    /// notice); what it did follows as [`CombatNews::Effects`].
    Special {
        monster: Entity,
        kind: crate::content::species::SpecialAttackKind,
    },
    /// A guard takes a blow meant for its ward (retail string 0x21a).
    Intercepts {
        guard: Entity,
        ward: Entity,
    },
    /// A monster's condition took its action (retail
    /// `GetConditionInterrupt`, by its interrupt number).
    Interrupted {
        monster: Entity,
        interrupt: Interrupt,
    },
    /// A monster runs for its life.
    Flees {
        monster: Entity,
    },
    /// A monster moves off from the party.
    Withdraws {
        monster: Entity,
    },
    /// A summoning found nowhere for its group to stand.
    SummonFails,
    /// The party's move begins; it ends when spent or stopped.
    PartyMoves,
    PartyStops,
}

/// What monsters see of the party (after retail `UpdateMonsterSight`).
struct Sight<'a, 'w, 's> {
    eye: Vec3,
    far: f32,
    /// The party's lowest level among those who can act, which weakens
    /// sight from behind.
    lowest_level: i32,
    concealment: u32,
    obstacles: &'a WorldObstacles<'w, 's>,
}

impl Sight<'_, '_, '_> {
    /// Whether a monster that can still fight sees the party: within the
    /// far clip, a clear line from its eyes past the level's static
    /// geometry, and its sight threshold covering the distance.
    fn sees(
        &self,
        monster: &Monster,
        at: &ActorPoseItem,
        walker: Option<&Walker>,
        kind: &Species,
    ) -> bool {
        let distance = party_distance(self.eye, monster, feet(at.translation(), walker));
        self.sees_point(
            (monster, at, walker, kind),
            self.eye,
            distance,
            self.lowest_level,
            self.concealment,
        )
    }

    /// A monster as the reinforcement and leaving rules read it.
    fn seen(
        &self,
        entity: Entity,
        (monster, at, walker, kind): (&Monster, &ActorPoseItem, Option<&Walker>, &Species),
    ) -> Seen {
        let at_feet = feet(at.translation(), walker);
        Seen {
            entity,
            at: at_feet,
            from_party: party_distance(self.eye, monster, at_feet),
            sees: self.sees(monster, at, walker, kind),
            in_view: self.in_view(monster, at, walker),
        }
    }

    /// Whether the party has a clear line to a monster
    /// ([`crate::game::world::perception::in_view`]), whichever way the
    /// monster faces and however keen it is.
    fn in_view(&self, monster: &Monster, at: &ActorPoseItem, walker: Option<&Walker>) -> bool {
        crate::game::world::perception::in_view(
            (self.eye, self.far),
            monster,
            feet(at.translation(), walker),
            self.obstacles,
        )
    }

    /// Whether a monster sees another (retail `CanMonsterSeeMonster`):
    /// the same rule, weakened from behind by the other's level.
    fn sees_monster(
        &self,
        observer: (&Monster, &ActorPoseItem, Option<&Walker>, &Species),
        target: (&Monster, &ActorPoseItem, Option<&Walker>, &Species),
    ) -> bool {
        let (body, at, walker, kind) = target;
        let (from, ..) = observer;
        let target_feet = feet(at.translation(), walker);
        let observer_feet = feet(observer.1.translation(), observer.2);
        let distance = from.gap(observer_feet, body, target_feet);
        let eye = body.eye(target_feet);
        self.sees_point(observer, eye, distance, i32::from(kind.level), 0)
    }

    fn sees_point(
        &self,
        (monster, at, walker, kind): (&Monster, &ActorPoseItem, Option<&Walker>, &Species),
        target: Vec3,
        distance: f32,
        penalty: i32,
        concealment: u32,
    ) -> bool {
        let eye = monster.eye(feet(at.translation(), walker));
        if !monster.can_fight() || distance > self.far || monster.blinded(kind) {
            return false;
        }
        let clear = self.obstacles.clear(eye, target);
        let forward = at.forward().with_y(0.0);
        let angle = forward.angle_between((target - eye).with_y(0.0));
        let senses = i32::from(kind.attributes.senses);
        let bonus = if kind.level_sight { (u32::from(kind.level) * 5).min(125) } else { 0 };
        clear
            && sight_threshold(angle, senses, bonus, penalty, concealment, self.far, distance)
                >= distance
    }
}

/// How far from the party a neutral member may stand and still reinforce a
/// fight (retail `GetMonsterEngagementRange`, 25000 world units).
const ENGAGEMENT_RANGE: f32 = 50.0;
/// What a group's leader reaches at least, beyond the engagement range, to
/// join a running fight (retail `g_monster_engagement_range_floor`).
const ENGAGEMENT_FLOOR: f32 = 15.0;
/// How far out of its way a neutral member may stand from a hostile's line
/// to the party and still be between them (retail
/// `g_reinforcement_distance_slack`).
const REINFORCEMENT_SLACK: f32 = 1.15;

/// A monster as the reinforcement and leaving rules read it: where its
/// feet are, how far it is from the party, whether it sees the party
/// ([`Sight::sees`]) and whether the party has a line to it
/// ([`Sight::in_view`]).
#[derive(Clone, Copy, Debug)]
struct Seen {
    entity: Entity,
    at: Vec3,
    from_party: f32,
    sees: bool,
    in_view: bool,
}

/// Whether a neutral group stands with a fight (retail
/// `MonsterGroupHasReinforcement`): one of its members in the party's view,
/// within the engagement range, stands roughly between the party and a
/// hostile fighter also in the party's view ([`Seen::in_view`], retail
/// `party_threat.los_flags[1]`). A camouflaged member does not count.
fn reinforces(members: &[(Seen, bool)], hostiles: &[Seen]) -> bool {
    members
        .iter()
        .filter(|(member, camouflaged)| {
            !camouflaged && member.in_view && member.from_party <= ENGAGEMENT_RANGE
        })
        .any(|(member, _)| {
            hostiles.iter().filter(|other| other.entity != member.entity && other.in_view).any(
                |other| {
                    let between = member.at.distance(other.at);
                    between + member.from_party < other.from_party * REINFORCEMENT_SLACK
                        && (member.from_party < other.from_party || between < other.from_party)
                },
            )
        })
}

/// The party's lowest level among those who can act.
fn lowest_level<'a>(party: impl Iterator<Item = (&'a Character, &'a Vitals)>) -> i32 {
    party.filter(|(_, v)| v.can_fight()).map(|(c, _)| i32::from(c.level())).min().unwrap_or(0)
}

/// Where the party stands and faces, and its formation, as reach needs them.
struct PartyView {
    eye: Vec3,
    yaw: f32,
    /// The level's far clip, which bounds monsters' sight.
    far: f32,
    /// The game time the round's actions happen at.
    now: u64,
}

impl PartyView {
    fn new((position, body): (&Position, &PartyBody), level: &PlayedLevel, now: u64) -> Self {
        Self {
            eye: PartyBody::eye(position),
            yaw: body.yaw,
            far: level.view_distance().unwrap_or(0.0),
            now,
        }
    }

    /// How monsters see the party now.
    fn sight<'a, 'w, 's>(
        &self,
        members: &Members,
        roster: &PartyRoster,
        obstacles: &'a WorldObstacles<'w, 's>,
        concealment: u32,
    ) -> Sight<'a, 'w, 's> {
        Sight {
            eye: self.eye,
            concealment,
            far: self.far,
            lowest_level: lowest_level(
                members.iter_many(roster.iter()).matched().map(|(c, v, ..)| (c, v)),
            ),
            obstacles,
        }
    }

    fn reaches(
        &self,
        formation: &Formation,
        member: Entity,
        at: Vec3,
        reach: Reach,
        distance: f32,
    ) -> bool {
        let side = formation::quadrant(self.eye, self.yaw, at);
        formation.reach(member, side, reach).is_some_and(|r| distance <= r.metres())
    }
}

/// Whether the party is fighting, for operations that hold the whole world.
pub fn in_battle(world: &mut World) -> bool {
    world.query_filtered::<(), With<Battle>>().iter(world).next().is_some()
}

/// Terminal state is settled independently of whose action dealt the damage.
/// Used before/after combat execution and at the end of environmental damage.
pub fn settle_defeat(
    mut commands: Commands,
    battle: Option<Single<(Entity, Option<&Combatants>), With<Battle>>>,
    roster: Single<&PartyRoster>,
    members: Query<&Vitals>,
    mut news: MessageWriter<CombatNews>,
    mut effects: Single<&mut crate::game::magic::enchantments::PartyEffects>,
    mut monsters: Query<&mut Monster>,
) {
    let Some((battle, combatants)) = battle.map(Single::into_inner) else { return };
    if !crate::game::characters::anyone_alive(members.iter_many(roster.iter()).matched()) {
        // What lasts a fight ends on the party and on those in it, as when
        // the party wins.
        effects.0.end_combat();
        let combatants = combatants.map_or(&[][..], Combatants::entities);
        for &monster in combatants {
            if let Ok(mut monster) = monsters.get_mut(monster) {
                monster.enchantments.end_combat();
            }
        }
        news.write(CombatNews::Lost);
        // Its combatants leave the fight and stop closing in; they remain.
        commands.entity(battle).despawn();
    }
}

/// A combatant's current allegiance, shared by target choice and engagement.
fn allegiance(
    entity: Entity,
    damage: &MonsterDamage,
    now: u64,
    species: &MonsterCatalog,
) -> hostility::Allegiance {
    let (body, ..) = damage.monsters().get(entity).unwrap();
    hostility::Allegiance {
        species: body.species,
        stance: damage.stance(entity, now),
        faction: species[body.species].faction,
        turncoat: body.conditions.has(Condition::Turncoat),
    }
}

/// Groups with nothing left to fight leave it as the round ends (retail
/// `CheckMonsterGroupsLeaveCombat`): a neutral group once it no longer
/// stands with the fight ([`reinforces`]); a friendly group while one of
/// its members sees an enemy; a hostile group while a living member sees
/// the party. When no hostile
/// group is left engaged, everyone leaves. Returns the combatants that
/// stay; one outside any group stays while hostile groups fight on.
fn leave_fight(
    commands: &mut Commands,
    combatants: &[Entity],
    (damage, now): (&MonsterDamage, u64),
    membership: &Query<&GroupMember>,
    sight: &Sight,
    species: &MonsterCatalog,
) -> Vec<Entity> {
    use hostility::Stance;
    let monsters = damage.monsters();
    let group_of = |monster: Entity| membership.get(monster).ok().map(|m| m.0);
    let stance_of = |monster: Entity| Some(damage.group_stance(monster, now));
    let seen = |monster: Entity| -> Option<(Seen, bool)> {
        let (body, at, walker, ..) = monsters.get(monster).ok()?;
        let kind = &species[body.species];
        if body.is_dead() {
            return None;
        }
        Some((sight.seen(monster, (body, &at, walker, kind)), kind.temper.camouflaged))
    };
    let hostiles: Vec<Seen> = combatants
        .iter()
        .filter(|&&c| stance_of(c) == Some(Stance::Hostile))
        .filter_map(|&c| seen(c).map(|(s, _)| s))
        .filter(|s| s.in_view)
        .collect();
    let (mut checked, mut leaving, mut engaged) = (Vec::new(), Vec::new(), false);
    for &combatant in combatants {
        let Some(group) = group_of(combatant) else {
            engaged |= stance_of(combatant) == Some(Stance::Hostile);
            continue;
        };
        if checked.contains(&group) {
            continue;
        }
        checked.push(group);
        let fellows: Vec<(Seen, bool)> = combatants
            .iter()
            .filter(|&&c| group_of(c) == Some(group))
            .filter_map(|&c| seen(c))
            .collect();
        let stays = match stance_of(combatant) {
            Some(Stance::Neutral) => reinforces(&fellows, &hostiles),
            Some(Stance::Hostile) => {
                let sees = fellows.iter().any(|(s, _)| s.sees);
                engaged |= sees;
                sees
            }
            Some(Stance::Friendly) => fellows.iter().any(|(fellow, _)| {
                let mine = allegiance(fellow.entity, damage, now, species);
                let (body, at, walker, ..) = monsters.get(fellow.entity).unwrap();
                let kind = &species[body.species];
                combatants.iter().any(|&enemy| {
                    let theirs = allegiance(enemy, damage, now, species);
                    let (target, target_at, target_walker, ..) = monsters.get(enemy).unwrap();
                    let target_kind = &species[target.species];
                    !target.is_dead()
                        && hostility::between(mine, theirs) == Stance::Hostile
                        && sight.sees_monster(
                            (body, &at, walker, kind),
                            (target, &target_at, target_walker, target_kind),
                        )
                })
            }),
            _ => false,
        };
        if !stays {
            leaving.push(group);
        }
    }
    combatants
        .iter()
        .copied()
        .filter(|&combatant| {
            let leaves = !engaged || group_of(combatant).is_some_and(|g| leaving.contains(&g));
            if leaves {
                commands.entity(combatant).remove::<Combatant>();
            }
            !leaves
        })
        .collect()
}

/// Start the round the player asks for, or in a continuous fight the next
/// one once the last action's pause is over and nothing is in flight
/// (retail `UpdateCombat`, `BeginCombatExecution`). Choosing the party's
/// pace between rounds is taken here too.
pub fn begin_round(
    time: Res<Time>,
    battle: Option<Single<(&mut Battle, Option<&Combatants>)>>,
    mut rng: ResMut<GameRng>,
    (mut members, damage): (Members, MonsterDamage),
    (roster, orders, settings): (Single<&PartyRoster>, Query<&Order>, Res<CombatSettings>),
    content: If<Res<Catalogs>>,
    (in_flight, moving): (
        Query<(), With<casting::SpellInFlight>>,
        Query<(), With<approach::CombatMove>>,
    ),
    mut news: MessageWriter<CombatNews>,
    (mut starts, mut moves): (MessageReader<StartRound>, MessageReader<CyclePartyMove>),
) {
    let Some((mut battle, combatants)) = battle.map(Single::into_inner) else {
        starts.clear();
        moves.clear();
        return;
    };
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    let start = starts.read().count() > 0;
    for _ in moves.read() {
        if !battle.executing() {
            battle.party_pace = match battle.party_pace {
                None => Some(PartyPace::Walk),
                Some(PartyPace::Walk) => Some(PartyPace::Run),
                Some(PartyPace::Run) => None,
            };
        }
    }
    // Starting the round while the party moves ends its move (retail
    // `MGSKeyboard.cpp:568-573`).
    if start && matches!(battle.party_move, PartyMove::Moving { .. }) {
        finish_party_move(&mut battle, &mut rng, &mut members);
        news.write(CombatNews::PartyStops);
    }
    let (items, species) = (&content.items, &content.species);
    let roster = &**roster;
    let accept = |battle: &mut Battle, rng: &mut GameRng| {
        accept_round(
            battle,
            combatants,
            roster,
            &members,
            &orders,
            damage.monsters(),
            (items, species),
            rng,
        )
    };
    // Between rounds a phased fight waits for the player; a continuous one
    // goes on.
    if (start || battle.opening.is_some()) && !battle.executing() {
        accept(&mut battle, &mut rng);
    }
    // A finished pause is not ticked again: a fight waiting for the player
    // stays unchanged.
    if !battle.wait.is_finished() && !battle.wait.tick(time.delta()).is_finished() {
        return;
    }
    // The next actor waits until the last one's missiles have landed
    // (retail `UpdateCombat`).
    if !in_flight.is_empty() || !moving.is_empty() {
        return;
    }
    if !battle.executing() && settings.continuous {
        accept(&mut battle, &mut rng);
    }
}

/// Carry out the round's actions, one per pause, once nothing is in flight.
pub fn run_battle(
    mut commands: Commands,
    battle: Option<Single<(Entity, &mut Battle, Option<&Combatants>)>>,
    mut rng: ResMut<GameRng>,
    clock: Res<GameClock>,
    (mut members, mut damage): (Members, MonsterDamage),
    party_body: Query<(&Position, &PartyBody)>,
    (roster, content, level): (Single<&PartyRoster>, If<Res<Catalogs>>, PlayedLevel),
    mut magic: BattleMagic,
    (mut news, mut summons): (MessageWriter<CombatNews>, MessageWriter<summon::Summon>),
) {
    let Some((_, mut battle, combatants)) = battle.map(Single::into_inner) else { return };
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    let species = &content.species;
    let Ok((position, body)) = party_body.single() else { return };
    if !battle.executing() || !battle.wait.is_finished() || magic.busy() {
        return;
    }
    let roster = &**roster;
    let party = PartyView::new((position, body), &level, clock.ms);
    if battle.phase == RoundPhase::Accepted {
        battle.phase = RoundPhase::Executing(1);
    }

    // The next actor due: accepted party order first, then monsters in order.
    while let RoundPhase::Executing(step) = battle.phase
        && step <= 100
    {
        if matches!(battle.party_move, PartyMove::Moving { .. }) {
            return;
        }
        if let PartyMove::Pending { pace, phase } = battle.party_move
            && phase <= step
        {
            let haste = roster
                .iter()
                .try_fold(0, |sum, member| {
                    let (_, vitals, ..) = members.get(member).unwrap();
                    let effect = vitals.enchantments.get(crate::content::spells::SpellId::HASTE)?;
                    Some(sum + effect.scaled(u32::from(effect.power) * 10))
                })
                .map_or(0.0, |sum| (sum / roster.len().max(1) as u32) as f32 * 0.2);
            battle.party_move = PartyMove::Moving {
                pace,
                budget: pace.combat_budget() + haste,
                moved: 0.0,
                last: Some(position.0),
            };
            news.write(CombatNews::PartyMoves);
            return;
        }
        // Nobody in the party acts while its move is still to come.
        let party_waits = matches!(battle.party_move, PartyMove::Pending { .. });
        if let Some(index) =
            battle.this_round.party.iter().position(|p| p.phase == step).filter(|_| !party_waits)
        {
            let acted = party_turn(
                &mut commands,
                &mut battle,
                index,
                combatants,
                &mut rng,
                (&mut members, &mut damage),
                &party,
                &mut magic,
                &mut news,
            );
            if acted {
                battle.wait = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
                return;
            }
            continue;
        }
        if let Some(index) = battle.this_round.monsters.iter().position(|turn| turn.phase == step) {
            let acted = monster_turn::monster_turn(
                &mut commands,
                &mut battle,
                (index, step),
                combatants,
                &mut rng,
                (&mut members, &mut damage),
                &party,
                &mut magic,
                species,
                (&mut news, &mut summons),
            );
            if acted {
                battle.wait = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
                return;
            }
            continue;
        }
        battle.phase = RoundPhase::Executing(step + 1);
    }
}

/// Once everyone has acted, the round ends (retail `EndCombatRound`): groups
/// with nothing left to fight leave ([`leave_fight`]), the round's recovery
/// and its count of who could strike settle ([`end_round`]), the party looks
/// again at who it fights, and a fight with nobody left is over.
pub fn finish_round(
    mut commands: Commands,
    battle: Option<Single<(Entity, &mut Battle, Option<&Combatants>)>>,
    mut rng: ResMut<GameRng>,
    mut clock: ResMut<GameClock>,
    mut actors: ParamSet<((Members, MonsterDamage), GroupRecognition)>,
    party_body: Query<(&Position, &PartyBody)>,
    (roster, content, level): (Single<&PartyRoster>, If<Res<Catalogs>>, PlayedLevel),
    mut magic: BattleMagic,
    mut news: MessageWriter<CombatNews>,
) {
    let Some((entity, mut battle, combatants)) = battle.map(Single::into_inner) else { return };
    let RoundPhase::Executing(step) = battle.phase else { return };
    if step <= 100
        || matches!(battle.party_move, PartyMove::Moving { .. })
        || !battle.wait.is_finished()
        || magic.busy()
    {
        return;
    }
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    let species = &content.species;
    let Ok((position, body)) = party_body.single() else { return };
    let roster = &**roster;
    let party = PartyView::new((position, body), &level, clock.ms);
    let (mut members, mut damage) = actors.p0();
    let sight = party.sight(
        &members,
        roster,
        &magic.context.visibility,
        magic.context.enchantments.0.concealment(),
    );
    let staying = leave_fight(
        &mut commands,
        combatants,
        (&damage, party.now),
        &magic.groups,
        &sight,
        species,
    );
    let over = end_round(
        (entity, &mut battle),
        combatants,
        &mut clock,
        &mut rng,
        roster,
        (&mut members, &mut damage),
        party.eye,
        species,
        &mut news,
        &mut magic,
        &staying,
    );
    if party_fallen(roster, &members) {
        return;
    }
    if over {
        for &actor in combatants {
            damage.enchantments_mut(actor).end_combat();
        }
    }
    // Recognition is a required part of this round, before the next
    // accepted action or the battle's end can change its meaning.
    actors.p1().detect(
        party.eye,
        body.forward(),
        Some(combatants),
        magic.context.enchantments.0.get(crate::content::spells::SpellId::SHADOW_HOUND).is_some(),
        false,
        &mut rng,
    );
    if over {
        magic.context.enchantments.0.end_combat();
        commands.entity(entity).despawn();
    }
}

/// The party's armour as this round's blows meet it: the spells on each
/// member and the party's, and whether they defend (retail
/// `CalcArmorClasses`).
fn party_armours(
    battle: &Battle,
    members: &Members,
    context: &MagicContext,
) -> Vec<(Entity, Armour)> {
    let items = &context.content.items;
    context
        .roster
        .iter()
        .map(|member| {
            let (character, vitals, carried, burden) = members.get(member).unwrap();
            let armour = attack::enchanted_armour(
                &character.stats(&vitals.enchantments),
                carried,
                items,
                burden.load,
                battle.defends(member),
                (&vitals.enchantments, Some(&context.enchantments.0)),
            );
            (member, armour)
        })
        .collect()
}

/// A member's accepted turn comes: revalidated, then their spell or attack.
/// Returns whether they acted; an actor who can no longer act loses the turn.
fn party_turn(
    commands: &mut Commands,
    battle: &mut Battle,
    index: usize,
    combatants: &[Entity],
    rng: &mut GameRng,
    (members, damage): (&mut Members, &mut MonsterDamage),
    party: &PartyView,
    magic: &mut BattleMagic,
    news: &mut MessageWriter<CombatNews>,
) -> bool {
    let work = battle.this_round.party[index];
    let member = work.actor;
    let able_now = magic.context.roster.contains(&member)
        && members.get(member).is_ok_and(|(_, vitals, carried, ..)| {
            vitals.can_fight()
                && match work.action {
                    // The weapons accepted, or nothing where one was used up.
                    RoundAction::Attack { weapons, .. } => {
                        [EquipSlot::Primary, EquipSlot::Secondary].into_iter().zip(weapons).all(
                            |(slot, accepted)| {
                                let now = carried.worn(slot).map(|i| i.id);
                                now.is_none() || now == accepted
                            },
                        )
                    }
                    RoundAction::Magic(_) | RoundAction::Breathe(_) | RoundAction::Defend => true,
                }
        });
    if !able_now {
        battle.this_round.party[index].end();
        return false;
    }
    match work.action {
        RoundAction::Breathe(target) => {
            battle.this_round.party[index].end();
            breathe(battle, index, target, combatants, rng, (members, damage), party, magic, news);
        }
        RoundAction::Magic(choice) => {
            battle.this_round.party[index].end();
            cast(
                commands,
                battle,
                index,
                choice,
                combatants,
                rng,
                (members, damage),
                party,
                magic,
                news,
            );
        }
        RoundAction::Attack { .. } => {
            let BattleMagic { context, points, moving, .. } = magic;
            let mut fight = blows::Fight {
                combatants,
                party,
                armours: party_armours(battle, members, context),
                moving,
                points,
                attacks: &|_| false,
                waits: false,
            };
            context.resolve(party.now, members, damage, |field| {
                blows::member_attacks(battle, index, rng, field, &mut fight, news);
            });
        }
        // Defending takes no turn.
        RoundAction::Defend => {
            battle.this_round.party[index].end();
            return false;
        }
    }
    true
}

/// A member breathes at their chosen target; a breath that cannot be made
/// falls back to defending.
fn breathe(
    battle: &mut Battle,
    index: usize,
    target: Entity,
    combatants: &[Entity],
    rng: &mut GameRng,
    (members, damage): (&mut Members, &mut MonsterDamage),
    party: &PartyView,
    magic: &mut BattleMagic,
    news: &mut MessageWriter<CombatNews>,
) {
    let member = battle.this_round.party[index].actor;
    let result =
        member_acts(combatants, (members, damage), party, magic, |situation, field, points| {
            breath::execute(member, target, situation, rng, field, points)
        });
    match result {
        Ok(effects) => {
            battle.this_round.attacked = true;
            news.write(CombatNews::Breathed { actor: member });
            battle.credit_kills(&effects, news);
            news.write(CombatNews::Effects(effects));
        }
        Err(_) => battle.this_round.party[index].action = RoundAction::Defend,
    }
}

/// A member casts or uses an item. A refused action falls back to
/// defending (retail `ExecuteCharacterAction`), its turn spent; a point
/// cast needs the whole world and settles once this step is done.
fn cast(
    commands: &mut Commands,
    battle: &mut Battle,
    index: usize,
    choice: MagicChoice,
    combatants: &[Entity],
    rng: &mut GameRng,
    (members, damage): (&mut Members, &mut MonsterDamage),
    party: &PartyView,
    magic: &mut BattleMagic,
    news: &mut MessageWriter<CombatNews>,
) {
    let member = battle.this_round.party[index].actor;
    let point_cast = choice
        .source
        .spell_id(
            members.get(member).unwrap().2,
            &magic.context.inventory,
            &magic.context.content.items,
        )
        .is_some_and(crate::game::magic::pointcasts::spell);
    if point_cast {
        commands.queue(move |world: &mut World| {
            if crate::game::magic::execute_in_world(world, member, choice).is_err() {
                let mut battle = world.query::<&mut Battle>().single_mut(world).unwrap();
                let work = battle.this_round.party.iter_mut().find(|w| w.actor == member).unwrap();
                work.action = RoundAction::Defend;
            }
        });
        return;
    }
    match member_casts(combatants, member, choice, rng, members, damage, party, magic) {
        Ok((report, flights)) => {
            battle.this_round.attacked = true;
            battle.credit_kills(&report.effects, news);
            let level = magic.context.level();
            magic.context.flight_order.launch(commands, flights, level);
            magic.context.news.write(MagicNews::Done(report));
        }
        Err(refusal) => {
            battle.this_round.party[index].action = RoundAction::Defend;
            magic.context.news.write(MagicNews::Refused { caster: member, refusal });
        }
    }
}

/// Count the ground the party covers while it moves in a fight, and end the
/// move when it is spent.
pub fn track_party_move(
    battle: Option<Single<&mut Battle>>,
    party: Query<&Position, With<PartyBody>>,
    mut rng: ResMut<GameRng>,
    mut members: Members,
    mut news: MessageWriter<CombatNews>,
) {
    let Some(mut battle) = battle else { return };
    let Ok(position) = party.single() else { return };
    let spent = {
        let PartyMove::Moving { budget, moved, last, .. } = &mut battle.party_move else {
            return;
        };
        if let Some(last) = *last {
            *moved += last.with_y(0.0).distance(position.0.with_y(0.0));
        }
        *last = Some(position.0);
        *moved >= *budget
    };
    if spent {
        finish_party_move(&mut battle, &mut rng, &mut members);
        news.write(CombatNews::PartyStops);
    }
}

impl PartyPace {
    /// The metres a move covers in a fight (retail `GetPartyMovementSpeed`:
    /// 10000 units walking, 15000 running).
    fn combat_budget(self) -> f32 {
        match self {
            PartyPace::Walk => 20.0,
            PartyPace::Run => 30.0,
        }
    }

    /// Stamina a metre costs an unencumbered member in a fight: retail
    /// charges `distance × load × 2 (combat)` units at one stamina per 2500
    /// (`MGSPartyMovement.cpp:302-358`).
    fn stamina_per_metre(self) -> f32 {
        match self {
            PartyPace::Walk => 0.2,
            PartyPace::Run => 0.4,
        }
    }
}

/// The move is over (retail `CompletePartyMovementTurns`, `HandlePartyMovement`):
/// each member keeps their action with the chance the unspent share gives
/// (spent counting half again when running), and pays stamina for the
/// ground covered, by load, half again when diseased.
fn finish_party_move(battle: &mut Battle, rng: &mut GameRng, members: &mut Members) {
    let PartyMove::Moving { pace, budget, moved, .. } = battle.party_move else { return };
    let step = match battle.phase {
        RoundPhase::Executing(step) => step,
        _ => 1,
    };
    battle.party_move = PartyMove::None;
    let mut spent = (moved / budget.max(0.01) * 100.0).min(100.0);
    if pace == PartyPace::Run {
        spent *= 1.5;
    }
    let keep = 100u32.saturating_sub(spent as u32);
    for work in &mut battle.this_round.party {
        if work.phase == 0 {
            continue;
        }
        if rng.below(100) < keep {
            work.phase = work.phase.max(step);
        } else {
            work.end();
        }
    }
    for work in &battle.this_round.party {
        let (_, mut vitals, _, burden) = members.get_mut(work.actor).unwrap();
        if vitals.is_dead() {
            continue;
        }
        let disease = if vitals.conditions.has(Condition::Diseased) { 1.5 } else { 1.0 };
        let cost = moved * pace.stamina_per_metre() * burden.load.movement_factor() * disease;
        vitals.spend_stamina(cost.round() as u32);
    }
}

/// When the remaining attacks of a turn come: spread over the rest of the
/// round, or none.
fn next_phase(step: u8, attacks_left: u32) -> u8 {
    if attacks_left == 0 {
        return 0;
    }
    let next = u32::from(step) + (100 - u32::from(step)) / (attacks_left + 1);
    if next > 100 { 0 } else { next as u8 }
}

/// What casting in a fight needs beside the fight itself.
#[derive(bevy::ecs::system::SystemParam)]
pub struct BattleMagic<'w, 's> {
    context: MagicContext<'w, 's>,
    points: Query<'w, 's, (&'static LearnedSpells, &'static mut SpellPoints)>,
    groups: Query<'w, 's, &'static GroupMember>,
    group_members: Query<'w, 's, &'static GroupMembers>,
    /// Missiles still on their way hold the fight's next action, as do
    /// monsters still advancing.
    in_flight: Query<'w, 's, (), With<casting::SpellInFlight>>,
    moving: Query<'w, 's, (), With<approach::CombatMove>>,
}

impl BattleMagic<'_, '_> {
    /// Whether the fight waits on missiles still flying or monsters still
    /// walking.
    fn busy(&self) -> bool {
        !self.in_flight.is_empty() || !self.moving.is_empty()
    }
}

/// Live, active combatants with unobstructed scenery sight, shared by casts
/// and breath. Accepted aim never redirects to a replacement actor.
fn visible_monsters(
    combatants: &[Entity],
    damage: &MonsterDamage,
    obstacles: &WorldObstacles,
    party_eye: Vec3,
) -> Vec<casting::Candidate> {
    combatants
        .iter()
        .filter_map(|&entity| {
            let (monster, at, walker, ..) = damage.monsters().get(entity).unwrap();
            if !damage.targetable(entity) {
                return None;
            }
            let feet = feet(at.translation(), walker);
            let eye = monster.eye(feet);
            obstacles.clear(party_eye, eye).then(|| casting::Candidate {
                entity,
                at: feet,
                distance: party_distance(party_eye, monster, feet),
            })
        })
        .collect()
}

/// A member casts or uses an item at their initiative.
fn member_casts(
    combatants: &[Entity],
    member: Entity,
    choice: MagicChoice,
    rng: &mut GameRng,
    members: &mut Members,
    damage: &mut MonsterDamage,
    party: &PartyView,
    magic: &mut BattleMagic,
) -> Result<(casting::Report, Vec<casting::SpellInFlight>), casting::Refusal> {
    member_acts(combatants, (members, damage), party, magic, |situation, field, points| {
        casting::perform(member, choice, situation, points, rng, field)
    })
}

/// A member's magic or breath at their initiative, in the fight as it
/// stands now: the monsters it may aim at are in the fight, targetable,
/// and in the party's sight, with their groups (retail
/// `CharacterActionReachesTarget`).
fn member_acts<R>(
    combatants: &[Entity],
    (members, damage): (&mut Members, &mut MonsterDamage),
    party: &PartyView,
    magic: &mut BattleMagic,
    act: impl FnOnce(
        &casting::Situation,
        &mut crate::game::magic::effects::Field,
        &mut Query<(&LearnedSpells, &mut SpellPoints)>,
    ) -> Result<R, casting::Refusal>,
) -> Result<R, casting::Refusal> {
    let BattleMagic { context, points, groups, group_members, .. } = magic;
    let candidates = visible_monsters(combatants, damage, &context.visibility, party.eye);
    let group_of = |monster: Entity| -> Vec<Entity> {
        let group = groups.get(monster).ok().map(|group| group.0);
        match group.and_then(|g| group_members.get(g).ok()) {
            Some(mates) => mates.iter().collect(),
            None => vec![monster],
        }
    };
    context.resolve(party.now, members, damage, |field| {
        let situation = casting::Situation {
            context: casting::UseContext::Combat,
            party: field.party,
            party_at: party.eye,
            facing: crate::game::party::body::heading(party.yaw),
            monsters: &candidates,
            group_of: &group_of,
        };
        act(&situation, field, points)
    })
}

/// Whether a monster can make its special attack now (retail
/// `CanMonsterFlee`): it has one, it is not cooling down, it has a tenth of
/// its stamina, and a silence does not stop it. A summoning is never what
/// starts a fight (`fighting` is `None` then, retail's `exclude_special`);
/// in one it needs the party within long reach and the summoner not on its
/// side. Any other kind also needs an aim, which its caller finds.
fn special_ready(
    monster: &Monster,
    kind: &Species,
    cooling: bool,
    fighting: Option<(f32, hostility::Stance)>,
) -> bool {
    let silenced = monster.conditions.has(Condition::Silenced);
    kind.special.kind.is_some_and(|special| {
        let definition = crate::game::magic::effects::special(special);
        let summons = special.summons().is_none()
            || fighting.is_some_and(|(nearest, stance)| {
                nearest <= Reach::Long.metres() && stance != hostility::Stance::Friendly
            });
        !cooling
            && monster.stamina >= monster.max_stamina / 10
            && !(silenced && definition.silenced)
            && summons
    })
}

/// A member as a casting monster sees them, at the party's point
/// `party` away.
fn member_in_scene(
    (member, character, vitals): (Entity, &Character, &Vitals),
    (party_at, party): (Vec3, f32),
    toward: &dyn Fn(crate::game::magic::effects::Victim) -> hostility::Stance,
) -> monster_spells::Someone {
    let stats = character.stats(&vitals.enchantments);
    monster_spells::Someone {
        entity: member,
        at: party_at,
        from_caster: party,
        standing: crate::game::magic::effects::Standing {
            hurt: vitals.hp < stats.max_hp(),
            tired: vitals.stamina < stats.max_stamina(),
            conditions: vitals.conditions.clone(),
            enchantments: vitals.enchantments.clone(),
        },
        stance: toward(crate::game::magic::effects::Victim::Member(member)),
    }
}

/// The party's members in roster order, as a scene reads them.
fn party_of<'a>(
    members: &'a Members,
    roster: &'a PartyRoster,
) -> impl Iterator<Item = (Entity, &'a Character, &'a Vitals)> {
    roster.iter().map(|member| {
        let (character, vitals, ..) = members.get(member).unwrap();
        (member, character, vitals)
    })
}

/// The fight as a monster about to cast or breathe sees it: the living
/// party and the living monsters in the fight it has a clear line to, where
/// each stands, what each lacks, and how it stands toward each (`toward`).
fn spell_scene<'a>(
    (caster, kind): (Entity, &Species),
    (combatants, party_members): (
        &[Entity],
        impl Iterator<Item = (Entity, &'a Character, &'a Vitals)>,
    ),
    monsters: &MonsterQuery,
    sight: &Sight,
    toward: &dyn Fn(crate::game::magic::effects::Victim) -> hostility::Stance,
) -> monster_spells::Scene {
    use crate::game::magic::effects::Victim;
    let party_eye = sight.eye;
    let party_at = PartyBody::feet_below(party_eye);
    let (body, at, walker, ..) = monsters.get(caster).unwrap();
    let feet_at = feet(at.translation(), walker);
    let eye = body.eye(feet_at);
    let party = party_distance(party_eye, body, feet_at);
    // A fellow is in reach of its spell only along a clear line, as the
    // party is only where the caster sees it.
    let clear = |to: Vec3| sight.obstacles.clear(eye, to);
    monster_spells::Scene {
        caster,
        from: feet_at,
        party_at,
        party,
        sees_party: sight.sees(body, &at, walker, kind),
        members: party_members
            .filter(|(_, _, vitals)| !vitals.is_dead())
            .map(|(member, character, vitals)| {
                member_in_scene((member, character, vitals), (party_at, party), toward)
            })
            .collect(),
        monsters: combatants
            .iter()
            .filter_map(|&entity| {
                let (other, at, walker, ..) = monsters.get(entity).ok()?;
                if other.is_dead() {
                    return None;
                }
                let at = feet(at.translation(), walker);
                if entity != caster && !clear(other.eye(at)) {
                    return None;
                }
                Some(monster_spells::Someone {
                    entity,
                    at,
                    from_caster: body.gap(feet_at, other, at),
                    standing: monster_spells::monster_standing(other),
                    stance: if entity == caster {
                        hostility::Stance::Friendly
                    } else {
                        toward(Victim::Monster(entity))
                    },
                })
            })
            .collect(),
    }
}

/// Whether nobody in the party is left alive.
fn party_fallen(roster: &PartyRoster, members: &Members) -> bool {
    !crate::game::characters::anyone_alive(
        members.iter_many(roster.iter()).matched().map(|(_, vitals, ..)| vitals),
    )
}

/// The round is over: a round's game time passes and the defenders rest.
/// Returns whether the fight is over: no hostile is left or nobody could
/// attack for two rounds.
fn end_round(
    (fight, battle): (Entity, &mut Battle),
    combatants: &[Entity],
    clock: &mut GameClock,
    rng: &mut GameRng,
    roster: &PartyRoster,
    (members, damage): (&mut Members, &mut MonsterDamage),
    party_eye: Vec3,
    species: &MonsterCatalog,
    news: &mut MessageWriter<CombatNews>,
    magic: &mut BattleMagic,
    staying: &[Entity],
) -> bool {
    pass_round(
        (fight, battle),
        combatants,
        clock,
        rng,
        roster,
        (members, damage),
        party_eye,
        news,
        magic,
    );
    // Poison may have killed the last of the party as the time passed: that
    // is their defeat, which `settle_defeat` reports, not a victory.
    if party_fallen(roster, members) {
        return false;
    }
    rest_defenders(battle, roster, members, rng);
    battle.wait = Timer::from_seconds(ROUND_SECONDS, TimerMode::Once);
    if !fight_over(battle, combatants, staying, damage, (clock.ms, party_eye), species) {
        return false;
    }
    award_victory(battle, roster, members, news);
    true
}

/// A round's game time passes, with its meaning: conditions age, spell
/// points come back at half rate and nothing else recovers (retail
/// `AdvanceCombatRound`, `AdvanceTimedEffects`). A round takes one aging
/// tick.
fn pass_round(
    (fight, battle): (Entity, &mut Battle),
    combatants: &[Entity],
    clock: &mut GameClock,
    rng: &mut GameRng,
    roster: &PartyRoster,
    (members, damage): (&mut Members, &mut MonsterDamage),
    party_eye: Vec3,
    news: &mut MessageWriter<CombatNews>,
    magic: &mut BattleMagic,
) {
    let ticks = clock.advance(crate::game::time::AGING_TICK_MS);
    // A monster that waited, or lies asleep or knocked out, gets its
    // breath back (retail `AdvanceCombatRound`).
    for &entity in combatants {
        let (monster, ..) = damage.monsters().get(entity).unwrap();
        let resting =
            battle.this_round.monsters.iter().any(|turn| turn.actor == entity && turn.waited)
                || monster
                    .conditions
                    .iter()
                    .any(|&(c, _)| matches!(c, Condition::Asleep | Condition::Unconscious));
        if resting {
            let maximum = monster.max_stamina;
            damage.restore_stamina(entity, recovery::rest_roll(maximum, rng));
        }
    }
    // A special attack cools down over the rounds the monster does not
    // make one (retail `AdvanceCombatRound`).
    for (monster, left) in &mut battle.cooldowns {
        if *left > 0
            && !battle.this_round.monsters.iter().any(|turn| turn.actor == *monster && turn.special)
        {
            *left -= 1;
        }
    }
    for &entity in combatants {
        let mut conditions = damage.conditions_mut(entity);
        if conditions.any_timed() {
            conditions.age(ticks.aging);
        }
    }
    for actor in roster.iter() {
        let (mut profile, mut vitals, ..) = members.get_mut(actor).unwrap();
        let (known, mut pool) = magic.points.get_mut(actor).unwrap();
        recovery::pass(
            &mut profile,
            &mut vitals,
            ticks,
            recovery::Pace::Fighting,
            (known, &mut pool),
            &magic.context.content.spells,
            &mut crate::game::characters::Fate { fight: Some(fight), rng },
        );
    }
    // AdvanceTimedEffects applies offensive slots after personal aging and
    // before expiring the slots. Their final remaining turn still pulses.
    let reports = magic.context.resolve(clock.ms, members, damage, |field| {
        crate::game::magic::effects::pulse_clouds(
            combatants,
            party_eye,
            rng,
            field,
            &mut magic.points,
        )
    });
    for (spell, effects) in reports {
        battle.credit_kills(&effects, news);
        battle.this_round.attacked |= crate::game::magic::effects::affected(&effects);
        magic.context.news.write(casting::MagicNews::Triggered {
            spell,
            effects,
            improved: vec![],
        });
    }
    magic.context.enchantments.0.age(ticks.aging);
    for &entity in combatants {
        damage.enchantments_mut(entity).age(ticks.aging);
    }
    battle.phase = RoundPhase::Waiting;
}

/// Members who defended, or lie asleep or knocked out, get their breath
/// back after their conditions age, in accepted actor order.
fn rest_defenders(battle: &Battle, roster: &PartyRoster, members: &mut Members, rng: &mut GameRng) {
    for work in &battle.this_round.party {
        let member = work.actor;
        if !roster.contains(&member) {
            continue;
        }
        let (character, mut vitals, ..) = members.get_mut(member).unwrap();
        let resting = battle.defends(member)
            || matches!(
                vitals.conditions.worst(),
                Some(Condition::Asleep | Condition::Unconscious)
            );
        if vitals.is_dead() || !resting {
            continue;
        }
        let maximum = character.stats(&vitals.enchantments).max_stamina();
        let gained = recovery::combat_rest_amount(&character, recovery::rest_roll(maximum, rng));
        if vitals.stamina < maximum || vitals.conditions.exhausted() {
            vitals.restore_stamina(gained, maximum);
        }
    }
}

/// Whether the fight is over: no hostile stays in it, or nobody attacked
/// or closed in for two rounds.
fn fight_over(
    battle: &mut Battle,
    combatants: &[Entity],
    staying: &[Entity],
    damage: &MonsterDamage,
    (now, party_eye): (u64, Vec3),
    species: &MonsterCatalog,
) -> bool {
    let monsters = damage.monsters();
    let approaching = combatants.iter().any(|&entity| {
        let (monster, at, walker, ..) = monsters.get(entity).unwrap();
        approach::needs_approach(monster, at.translation(), walker, party_eye, species)
    });
    let active = battle.this_round.attacked || approaching;
    battle.passive_rounds = if active { 0 } else { battle.passive_rounds + 1 };
    let hostiles_left = staying.iter().any(|&e| {
        damage.stance(e, now) == hostility::Stance::Hostile && !monsters.get(e).unwrap().0.is_dead()
    });
    !hostiles_left || battle.passive_rounds >= PASSIVE_ROUNDS
}

/// Experience is shared among the living, the unconscious included.
fn award_victory(
    battle: &Battle,
    roster: &PartyRoster,
    members: &mut Members,
    news: &mut MessageWriter<CombatNews>,
) {
    let living =
        members.iter_many(roster.iter()).matched().filter(|(_, v, ..)| !v.is_dead()).count() as u32;
    let each = battle.experience / living.max(1);
    if each > 0 {
        for (mut character, vitals, ..) in members.iter_many_unique_mut(roster.iter()).matched() {
            if !vitals.is_dead() {
                character.award_experience(each);
            }
        }
    }
    news.write(if battle.kills > 0 {
        CombatNews::Won { kills: battle.kills, experience_each: each }
    } else {
        CombatNews::Over
    });
}
