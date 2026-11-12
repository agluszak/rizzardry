//! Whose side everyone is on (`docs/rules/actors.md`, "Side").
//!
//! An actor's [`Side`] toward the party is stored on it and written only by
//! what changes it: being made, its group being made to [`Stand`] some way
//! or that wearing off, and the party's standing with its faction moving.
//! Turncoat is not written into it: it is a condition, and swaps the side
//! where the side is read ([`Side::now`]).

use bevy::prelude::*;

use super::identify::{GroupMember, GroupMembers, MonsterGroup};
use crate::content::species::{Disposition, Species};
use crate::game::characters::{Condition, Conditions, Vitals};
use crate::game::factions::{Band, Factions, PARTY, UNALIGNED};
use crate::game::party::recovery::Elapsed;
use crate::game::time::GameClock;

/// How someone stands toward the party (retail `W8Disposition`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize)]
pub enum Stance {
    #[default]
    Neutral,
    Hostile,
    Friendly,
}

impl Stance {
    pub fn opposes(self, other: Self) -> bool {
        matches!((self, other), (Self::Hostile, Self::Friendly) | (Self::Friendly, Self::Hostile))
    }

    /// Turned about: Turncoat makes a hostile friendly and a friendly
    /// hostile (retail `InflictConditionOnMonster`); a neutral shrugs it
    /// off.
    pub fn turned(self) -> Self {
        match self {
            Self::Hostile => Self::Friendly,
            Self::Friendly => Self::Hostile,
            Self::Neutral => Self::Neutral,
        }
    }
}

/// The side an actor is on toward the party, its own turning aside: a
/// member the party's, a monster its group's while something has set one
/// and otherwise what its kind and faction make it.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Side(Stance);

impl Side {
    pub const FRIENDLY: Side = Side(Stance::Friendly);

    pub fn new(stance: Stance) -> Self {
        Side(stance)
    }

    /// The side it is on now: its own, turned about while it is Turncoat.
    /// The dead have no conditions, so they keep the side they died on
    /// ([`Side::die`]).
    pub fn now(&self, conditions: &Conditions) -> Stance {
        if conditions.has(Condition::Turncoat) { self.0.turned() } else { self.0 }
    }

    /// The side its group or kind gives it, whatever has turned it.
    pub fn own(&self) -> Stance {
        self.0
    }

    /// Dying fixes the side it was on as it died, since death lifts what
    /// had turned it.
    pub fn die(&mut self, conditions: &Conditions) {
        self.0 = self.now(conditions);
    }
}

/// The band an actor keeps to besides its side, which two monsters on
/// different sides weigh before they quarrel (retail `MonsterHostility`).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Loyalty {
    /// Of no faction: a member or an unaligned monster.
    #[default]
    None,
    Faction(i32),
    /// A bystander, who stays out of every quarrel.
    Bystander,
}

impl Loyalty {
    /// The band it keeps to now: one under Turncoat has broken with its own.
    pub fn now(self, conditions: &Conditions) -> Self {
        match self {
            Loyalty::Faction(_) if conditions.has(Condition::Turncoat) => Loyalty::None,
            loyalty => loyalty,
        }
    }
}

/// The species retail keeps out of every quarrel (record 0x224): neutral to
/// everyone, and only ever waiting in a fight.
pub const BYSTANDER_SPECIES: u32 = 0x224;

/// What a monster's side comes from when nothing has set one, written from
/// its species as it is made.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Allegiance {
    pub faction: i32,
    pub disposition: Disposition,
    /// NPCs keep to their dialogue and start neutral (retail reads their
    /// disposition band; NPCs do not fight yet).
    pub npc: bool,
    /// How long it holds a stance set on its group outside a fight: its
    /// Intelligence squared, in game minutes (retail
    /// `RefreshMonsterGroupHostility`).
    pub grudge: u64,
}

impl Allegiance {
    pub fn of(species: &Species) -> Self {
        let intelligence = u64::from(species.attributes.intelligence);
        Allegiance {
            faction: species.faction,
            disposition: species.disposition,
            npc: species.npc.is_some(),
            grudge: intelligence * intelligence * crate::game::time::MINUTE_MS,
        }
    }

    /// The band its kind keeps to.
    pub fn loyalty(&self, species: u32) -> Loyalty {
        match self.faction {
            _ if species == BYSTANDER_SPECIES => Loyalty::Bystander,
            UNALIGNED => Loyalty::None,
            faction => Loyalty::Faction(faction),
        }
    }

    /// Its side before anything sets one (retail
    /// `MonsterGroupCalcDefaultDisposition`): the unaligned by their
    /// authored disposition, which a hostility radius leaves neutral until
    /// the party comes close; the party's own faction friendly; any other
    /// faction by its band.
    pub fn default_stance(&self, factions: &Factions) -> Stance {
        if self.npc {
            return Stance::Neutral;
        }
        match self.faction {
            UNALIGNED => match self.disposition {
                Disposition::Hostile => Stance::Hostile,
                Disposition::Neutral | Disposition::HostileWithin(_) => Stance::Neutral,
            },
            PARTY => Stance::Friendly,
            faction => match factions.band(faction) {
                Band::Hostile => Stance::Hostile,
                Band::Neutral => Stance::Neutral,
                Band::Friendly => Stance::Friendly,
            },
        }
    }

    /// How near the party must come to turn an unaligned kind's group
    /// hostile (retail `RefreshMonsterGroupHostility`,
    /// `CheckMonsterGroupsEnterCombat`); a kind with no such radius has none.
    pub fn hostility_radius(&self) -> Option<f32> {
        match self.disposition {
            Disposition::HostileWithin(radius) if !self.npc && self.faction == UNALIGNED => {
                Some(radius)
            }
            _ => None,
        }
    }

    /// Whether a stance set at `at` has worn off by `now`: its grudge has
    /// run out, or its faction's band has moved since.
    fn forgets(&self, at: u64, now: u64, factions: &Factions) -> bool {
        now - at > self.grudge
            || (self.faction != UNALIGNED && at < factions.band_changed_at(self.faction))
    }
}

/// A stance something set on a group, and when (game milliseconds): the
/// party provoking it, or coming within its hostility radius.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetStance {
    pub stance: Stance,
    pub at: u64,
}

/// A group is made to stand some way toward the party from now, or, with
/// no stance, goes back to what its members' kinds make them.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Stand {
    #[event_target]
    pub group: Entity,
    pub stance: Option<Stance>,
}

fn stand(
    event: On<Stand>,
    mut groups: Query<(&mut MonsterGroup, Option<&GroupMembers>)>,
    mut monsters: Query<(&Allegiance, &mut Side)>,
    (factions, clock): (Res<Factions>, Res<GameClock>),
) {
    let Ok((mut group, members)) = groups.get_mut(event.group) else { return };
    group.set = event.stance.map(|stance| SetStance { stance, at: clock.ms });
    for member in members.into_iter().flat_map(GroupMembers::iter) {
        if let Ok((allegiance, mut side)) = monsters.get_mut(member) {
            let stance = event.stance.unwrap_or_else(|| allegiance.default_stance(&factions));
            side.set_if_neq(Side(stance));
        }
    }
}

/// A monster made, or come into a group, takes the side its group stands
/// on, or its kind's.
fn take_side(
    monster: Entity,
    monsters: &mut Query<(&Allegiance, &mut Side, Option<&GroupMember>)>,
    groups: &Query<&MonsterGroup>,
    factions: &Factions,
) {
    let Ok((allegiance, mut side, member)) = monsters.get_mut(monster) else { return };
    let set = member.and_then(|member| groups.get(member.0).ok()).and_then(|group| group.set);
    let stance = set.map_or_else(|| allegiance.default_stance(factions), |set| set.stance);
    side.set_if_neq(Side(stance));
}

fn made(
    made: On<Insert<Allegiance>>,
    mut monsters: Query<(&Allegiance, &mut Side, Option<&GroupMember>)>,
    mut loyalties: Query<(&Allegiance, &super::population::Monster, &mut Loyalty)>,
    groups: Query<&MonsterGroup>,
    factions: Res<Factions>,
) {
    take_side(made.entity, &mut monsters, &groups, &factions);
    if let Ok((allegiance, monster, mut loyalty)) = loyalties.get_mut(made.entity) {
        *loyalty = allegiance.loyalty(monster.species);
    }
}

/// A member raised is the party's again, whatever had turned them as they
/// died.
fn raised(
    raised: On<Remove<crate::game::characters::Dead>>,
    mut members: Query<&mut Side, With<crate::game::party::roster::PartyMember>>,
) {
    if let Ok(mut side) = members.get_mut(raised.entity) {
        *side = Side::FRIENDLY;
    }
}

/// Whoever joins the party is on its side.
fn joined(
    joined: On<Insert<crate::game::party::roster::PartyMember>>,
    mut sides: Query<&mut Side>,
) {
    if let Ok(mut side) = sides.get_mut(joined.entity) {
        *side = Side::FRIENDLY;
    }
}

fn grouped(
    grouped: On<Insert<GroupMember>>,
    mut monsters: Query<(&Allegiance, &mut Side, Option<&GroupMember>)>,
    groups: Query<&MonsterGroup>,
    factions: Res<Factions>,
) {
    take_side(grouped.entity, &mut monsters, &groups, &factions);
}

/// The party's standing with a faction moved: those nothing has set a
/// stance on stand as its band has them now.
fn follow_factions(
    mut monsters: Query<(&Allegiance, &mut Side, Option<&GroupMember>)>,
    groups: Query<&MonsterGroup>,
    factions: Res<Factions>,
) {
    for (allegiance, mut side, member) in &mut monsters {
        let set = member.and_then(|member| groups.get(member.0).ok()).and_then(|group| group.set);
        if set.is_none() {
            side.set_if_neq(Side(allegiance.default_stance(&factions)));
        }
    }
}

/// As time passes ([`Elapsed`]), a stance set on a group wears off: once its leader's
/// grudge has run out, or its faction's band has moved since. Never while
/// the group fights (retail `RefreshMonsterGroupHostility`, which runs only
/// outside a fight).
fn forget(
    _: On<Elapsed>,
    groups: Query<(Entity, &MonsterGroup, &GroupMembers)>,
    monsters: Query<(&Allegiance, Has<crate::game::combat::Combatant>)>,
    (factions, clock): (Res<Factions>, Res<GameClock>),
    mut commands: Commands,
) {
    for (group, kept, members) in &groups {
        let Some(set) = kept.set else { continue };
        let members = || members.iter().filter_map(|member| monsters.get(member).ok());
        let fighting = members().any(|(_, fighting)| fighting);
        let forgets =
            members().next().is_none_or(|(leader, _)| leader.forgets(set.at, clock.ms, &factions));
        if !fighting && forgets {
            commands.trigger(Stand { group, stance: None });
        }
    }
}

/// How two bodies stand toward each other, by their sides and loyalties
/// (retail `MonsterHostility`, `MonsterVsCharDisposition`): a bystander
/// stays out; alike, friends; either neutral, neutral; otherwise foes, but
/// for two loyal to one faction, who stay out of it.
pub fn between(first: (Stance, Loyalty), second: (Stance, Loyalty)) -> Stance {
    use Loyalty::*;
    match (first, second) {
        ((_, Bystander), _) | (_, (_, Bystander)) => Stance::Neutral,
        ((a, _), (b, _)) if a == b => Stance::Friendly,
        ((Stance::Neutral, _), _) | (_, (Stance::Neutral, _)) => Stance::Neutral,
        ((_, Faction(a)), (_, Faction(b))) if a == b => Stance::Neutral,
        _ => Stance::Hostile,
    }
}

/// The party acted against this monster: a blow aimed at it, a hostile
/// spell aimed at it or catching it, whatever came of them (retail
/// `MakeTargetGroupHostile`, `ProvokeListedMonsterGroups`).
#[derive(EntityEvent, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Provocation {
    #[event_target]
    pub monster: Entity,
}

/// On a group: the party has just acted against one of its monsters. A
/// hostile group so provoked enters the fight at once; engagement takes
/// this off as it looks at the group.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Provoked;

/// A monster's group is provoked as the action against it starts, and
/// turns hostile before the blows or spells land (retail
/// `MakeTargetGroupHostile`); a dead monster, or one under Turncoat, turns
/// nobody.
pub fn take_offence(
    offence: On<Provocation>,
    monsters: Query<(&Side, &Vitals, &Conditions, &GroupMember)>,
    mut commands: Commands,
) {
    let Ok((side, vitals, conditions, member)) = monsters.get(offence.monster) else { return };
    commands.entity(member.0).insert(Provoked);
    if vitals.is_dead() || conditions.has(Condition::Turncoat) {
        return;
    }
    if side.own() != Stance::Hostile {
        commands.trigger(Stand { group: member.0, stance: Some(Stance::Hostile) });
    }
}

pub fn plugin(app: &mut App) {
    app.add_observer(stand)
        .add_observer(take_offence)
        .add_observer(forget)
        .add_observer(made)
        .add_observer(grouped)
        .add_observer(joined)
        .add_observer(raised)
        .add_systems(
            Update,
            follow_factions
                .run_if(resource_changed::<Factions>)
                .in_set(crate::game::GameplaySet::Bodies),
        );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allegiance(disposition: Disposition, faction: i32, intelligence: u8) -> Allegiance {
        let mut species = Species { disposition, faction, ..default() };
        species.attributes.intelligence = intelligence;
        Allegiance::of(&species)
    }

    /// Crabs keep to themselves until the party comes within their fifteen
    /// metres; aligned species follow their faction's band.
    #[test]
    fn the_default_follows_the_record_and_the_faction_band() {
        let factions = Factions::default();
        let crab = allegiance(Disposition::HostileWithin(15.0), UNALIGNED, 10);
        assert_eq!(crab.default_stance(&factions), Stance::Neutral);
        assert_eq!(crab.hostility_radius(), Some(15.0));
        let slime = allegiance(Disposition::Hostile, UNALIGNED, 10);
        assert_eq!(slime.default_stance(&factions), Stance::Hostile);
        assert_eq!(slime.hostility_radius(), None, "only a radius promotes");
        let rapax = allegiance(Disposition::Neutral, 15, 10);
        assert_eq!(rapax.default_stance(&factions), Stance::Hostile, "common Rapax");
        let umpani = allegiance(Disposition::Neutral, 4, 10);
        assert_eq!(umpani.default_stance(&factions), Stance::Friendly);
        let allies = allegiance(Disposition::Hostile, PARTY, 10);
        assert_eq!(allies.default_stance(&factions), Stance::Friendly);
    }

    /// Turncoat swaps a monster's side, not a neutral's; monsters alike
    /// are friends, a neutral stays out, and two of one faction quarrel
    /// only when Turncoat holds one of them (retail `MonsterHostility`).
    #[test]
    fn monsters_stand_by_their_sides_and_turncoat_swaps_them() {
        assert_eq!(Stance::Hostile.turned(), Stance::Friendly);
        assert_eq!(Stance::Neutral.turned(), Stance::Neutral);
        // A member is of no faction, so a monster turned on its own side
        // quarrels with them and its former fellows alike.
        let foe = (Stance::Hostile, Loyalty::None);
        let turned = (Stance::Hostile.turned(), Loyalty::None);
        assert_eq!(between(foe, foe), Stance::Friendly);
        assert_eq!(between(foe, turned), Stance::Hostile);
        assert_eq!(between(foe, (Stance::Neutral, Loyalty::None)), Stance::Neutral);
        let mook = |stance| (stance, Loyalty::Faction(crate::game::factions::MOOK));
        assert_eq!(between(mook(Stance::Hostile), mook(Stance::Friendly)), Stance::Neutral);
        assert_eq!(between(mook(Stance::Hostile), turned), Stance::Hostile);
        assert_eq!(between(foe, (Stance::Hostile, Loyalty::Bystander)), Stance::Neutral);
    }

    /// A provocation holds for Intelligence² game minutes, and a faction's
    /// band moving since releases it.
    #[test]
    fn a_set_stance_wears_off_with_its_grudge_or_when_the_faction_moves() {
        let mut factions = Factions::default();
        let crab = allegiance(Disposition::HostileWithin(15.0), UNALIGNED, 10);
        let hold = 10 * 10 * crate::game::time::MINUTE_MS;
        assert!(!crab.forgets(1_000, 1_000 + hold, &factions));
        assert!(crab.forgets(1_000, 1_001 + hold, &factions));
        let mook = allegiance(Disposition::Neutral, crate::game::factions::MOOK, 10);
        assert!(!mook.forgets(1_000, 2_000, &factions));
        assert!(factions.record_offence(crate::game::factions::MOOK, false, 1_500).is_some());
        assert!(mook.forgets(1_000, 2_000, &factions), "the Mook band fell after it was set");
    }
}
