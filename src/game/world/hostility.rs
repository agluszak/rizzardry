//! How monster groups stand toward the party (retail `Combat Hostility.cpp`,
//! `MonsterGroup.cpp`).
//!
//! A group's disposition belongs to the group, and its members stand as it
//! does: what last set it, and when (retail `ubDisposition`,
//! `hostility_set_at`). The party's standing with a faction is the
//! faction's ([`Factions`]); a member's own control effects (Turncoat) are
//! exceptions read where they matter, never copied onto the group.

use bevy::prelude::*;

use super::identify::{GroupMember, MonsterGroup};
use super::population::Monster;
use crate::content::species::{Disposition, MonsterCatalog, Species};
use crate::game::characters::Condition;
use crate::game::factions::{Band, Factions, PARTY, UNALIGNED};
use crate::game::time::GameClock;

/// How a group stands toward the party (retail `W8Disposition`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
}

/// A stance something set on a group, and when (game milliseconds): the
/// party provoking it, or coming within its hostility radius.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetStance {
    pub stance: Stance,
    pub at: u64,
}

/// A group's stance before anything sets one (retail
/// `MonsterGroupCalcDefaultDisposition`): unaligned species by their
/// authored disposition, which a hostility radius leaves neutral until the
/// party comes close; the party's own faction friendly; any other faction
/// by its band. NPCs keep to their dialogue and start neutral here (retail
/// reads their disposition band; NPCs do not fight yet).
pub fn default_stance(species: &Species, factions: &Factions) -> Stance {
    if species.npc.is_some() {
        return Stance::Neutral;
    }
    match species.faction {
        UNALIGNED => match species.disposition {
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

/// How long a set stance holds outside a fight: the species' Intelligence
/// squared, in game minutes (retail `RefreshMonsterGroupHostility`).
fn holds_for(species: &Species) -> u64 {
    let intelligence = u64::from(species.attributes.intelligence);
    intelligence * intelligence * crate::game::time::MINUTE_MS
}

/// A group's stance now (retail `RefreshMonsterGroupHostility`, which runs
/// only outside a fight): a set stance holds while the group fights, and
/// otherwise for its while, unless the faction's band has changed since;
/// then the default stands again.
pub fn stance(
    set: Option<SetStance>,
    species: &Species,
    factions: &Factions,
    now: u64,
    fighting: bool,
) -> Stance {
    match set {
        Some(set)
            if fighting
                || (now - set.at <= holds_for(species)
                    && (species.faction == UNALIGNED
                        || set.at >= factions.band_changed_at(species.faction))) =>
        {
            set.stance
        }
        _ => default_stance(species, factions),
    }
}

/// Whether the party has come within an unaligned species' hostility
/// radius, which turns its group hostile (retail
/// `RefreshMonsterGroupHostility`, `CheckMonsterGroupsEnterCombat`).
pub fn within_radius(species: &Species, nearest: f32) -> bool {
    species.npc.is_none()
        && species.faction == UNALIGNED
        && matches!(species.disposition, Disposition::HostileWithin(radius) if nearest <= radius)
}

/// Which side a body is on toward the party now: a member the party's, a
/// monster its group's while the group's set stance holds ([`stance`]),
/// either swapped by its own Turncoat ([`own_stance`]).
///
/// Derived, so that every rule reads one fact instead of the group, the
/// factions and the clock. [`judge_sides`] produces it from those sources
/// at the start of each frame and fixed step, for every body; rules may
/// trust it until the next. A body spawned in between reads it as neutral
/// until then.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Side(pub Stance);

/// Judge every body's side from its group, the factions and the clock.
pub fn judge_sides(
    mut bodies: Query<(
        &mut Side,
        &mut Loyalty,
        &crate::game::characters::Vitals,
        Option<&Monster>,
        Option<&GroupMember>,
        Has<crate::game::combat::Combatant>,
    )>,
    groups: Query<&MonsterGroup>,
    content: If<Res<crate::game::data::Catalogs>>,
    (factions, clock): (Res<Factions>, Res<GameClock>),
) {
    for (mut side, mut loyalty, vitals, monster, member, fighting) in &mut bodies {
        let turncoat = vitals.conditions.has(Condition::Turncoat);
        let (own, loyal) = match monster {
            Some(monster) => {
                let kind = &content.species[monster.species];
                let set = member.and_then(|member| groups.get(member.0).ok()).and_then(|g| g.set);
                let loyal = if monster.species == BYSTANDER_SPECIES {
                    Loyalty::Bystander
                } else if turncoat || kind.faction == UNALIGNED {
                    Loyalty::None
                } else {
                    Loyalty::Faction(kind.faction)
                };
                (stance(set, kind, &factions, clock.ms, fighting), loyal)
            }
            None => (Stance::Friendly, Loyalty::None),
        };
        side.set_if_neq(Side(own_stance(own, turncoat)));
        loyalty.set_if_neq(loyal);
    }
}

/// A monster's own stance now: its group's set stance while it holds
/// ([`stance`]), swapped by the monster's own Turncoat ([`own_stance`]).
/// Every reader of a monster's side goes through this.
pub fn monster_own(
    vitals: &crate::game::characters::Vitals,
    set: Option<SetStance>,
    species: &Species,
    factions: &Factions,
    now: u64,
    fighting: bool,
) -> Stance {
    own_stance(
        stance(set, species, factions, now, fighting),
        vitals.conditions.has(Condition::Turncoat),
    )
}

/// A monster's stance now, through its group, for operations that hold the
/// whole world. A monster outside any group stands by its species' default.
pub fn monster_stance(world: &World, monster: Entity) -> Stance {
    let Some(species) = world
        .get::<Monster>(monster)
        .map(|m| &world.resource::<crate::game::data::Catalogs>().species[m.species])
    else {
        return Stance::Neutral;
    };
    let set = world
        .get::<GroupMember>(monster)
        .and_then(|member| world.get::<MonsterGroup>(member.0))
        .and_then(|group| group.set);
    let fighting = world.get::<crate::game::combat::Combatant>(monster).is_some();
    let now = world.resource::<GameClock>().ms;
    let body = world.get::<crate::game::characters::Vitals>(monster).unwrap();
    monster_own(body, set, species, world.resource::<Factions>(), now, fighting)
}

/// Monsters' stances now, through their groups, for systems that only
/// read them (the fight's snapshot, the radar). Time is the caller's: the
/// fight holds the clock itself.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Stances<'w, 's> {
    monsters: Query<
        'w,
        's,
        (
            &'static Monster,
            Option<&'static GroupMember>,
            Has<crate::game::combat::Combatant>,
            &'static crate::game::characters::Vitals,
        ),
    >,
    groups: Query<'w, 's, &'static MonsterGroup>,
    factions: Res<'w, Factions>,
}

impl Stances<'_, '_> {
    /// A monster's group's stance now; one outside any group by its
    /// species' default.
    pub fn group(&self, monster: Entity, species: &MonsterCatalog, now: u64) -> Option<Stance> {
        let (body, member, fighting, _) = self.monsters.get(monster).ok()?;
        let kind = &species[body.species];
        let set = member.and_then(|m| self.groups.get(m.0).ok()).and_then(|g| g.set);
        Some(stance(set, kind, &self.factions, now, fighting))
    }

    /// A monster's own stance now: its group's, swapped by its own Turncoat
    /// (retail's per-monster `ubDisposition`).
    pub fn own(&self, monster: Entity, species: &MonsterCatalog, now: u64) -> Option<Stance> {
        let group = self.group(monster, species, now)?;
        let (.., vitals) = self.monsters.get(monster).ok()?;
        let turncoat = vitals.conditions.has(Condition::Turncoat);
        Some(own_stance(group, turncoat))
    }
}

/// The species retail keeps out of every quarrel (record 0x224): neutral to
/// everyone, and only ever waiting in a fight.
pub const BYSTANDER_SPECIES: u32 = 0x224;

/// A monster's own stance: its group's, swapped by Turncoat, which turns a
/// hostile monster friendly and a friendly one hostile (retail
/// `InflictConditionOnMonster`); a neutral one shrugs it off.
pub fn own_stance(group: Stance, turncoat: bool) -> Stance {
    match (group, turncoat) {
        (Stance::Hostile, true) => Stance::Friendly,
        (Stance::Friendly, true) => Stance::Hostile,
        (stance, _) => stance,
    }
}

/// A member's stance toward the party: on its side, against it under
/// Turncoat, by the same rule as a monster's.
pub fn member_stance(turncoat: bool) -> Stance {
    own_stance(Stance::Friendly, turncoat)
}

/// The band a body keeps to besides its side, which two monsters on
/// different sides weigh before they quarrel (retail `MonsterHostility`).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Loyalty {
    /// Of no faction: a member, an unaligned monster, or one under Turncoat,
    /// which has broken with its own.
    #[default]
    None,
    Faction(i32),
    /// A bystander, who stays out of every quarrel.
    Bystander,
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
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Provocation(pub Entity);

/// A monster provoked turns its group hostile as the action starts, before
/// the blows or spells land (retail `MakeTargetGroupHostile`); a dead
/// monster, or one under Turncoat, provokes nobody. Runs wherever
/// provocations are made: in the swing and cast stages, and on the frame
/// before groups engage.
pub fn take_offence(
    mut provocations: MessageReader<Provocation>,
    monsters: Query<(&Monster, &crate::game::characters::Vitals, Option<&GroupMember>)>,
    mut groups: Query<&mut MonsterGroup>,
    content: If<Res<crate::game::data::Catalogs>>,
    factions: Res<Factions>,
    clock: Res<GameClock>,
) {
    for &Provocation(provoked) in provocations.read() {
        let Ok((monster, vitals, member)) = monsters.get(provoked) else { continue };
        if vitals.is_dead() || vitals.conditions.has(Condition::Turncoat) {
            continue;
        }
        let kind = &content.species[monster.species];
        if let Some(member) = member
            && let Ok(mut group) = groups.get_mut(member.0)
            && stance(group.set, kind, &factions, clock.ms, true) != Stance::Hostile
        {
            group.set = Some(SetStance { stance: Stance::Hostile, at: clock.ms });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn species(disposition: Disposition, faction: i32, intelligence: u8) -> Species {
        let mut species = Species { disposition, faction, ..default() };
        species.attributes.intelligence = intelligence;
        species
    }

    /// Crabs keep to themselves until the party comes within their fifteen
    /// metres; aligned species follow their faction's band.
    #[test]
    fn the_default_follows_the_record_and_the_faction_band() {
        let factions = Factions::default();
        let crab = species(Disposition::HostileWithin(15.0), UNALIGNED, 10);
        assert_eq!(default_stance(&crab, &factions), Stance::Neutral);
        assert!(within_radius(&crab, 10.0) && !within_radius(&crab, 20.0));
        let slime = species(Disposition::Hostile, UNALIGNED, 10);
        assert_eq!(default_stance(&slime, &factions), Stance::Hostile);
        assert!(!within_radius(&slime, 0.0), "only a radius promotes");
        let rapax = species(Disposition::Neutral, 15, 10);
        assert_eq!(default_stance(&rapax, &factions), Stance::Hostile, "common Rapax");
        let umpani = species(Disposition::Neutral, 4, 10);
        assert_eq!(default_stance(&umpani, &factions), Stance::Friendly);
        let allies = species(Disposition::Hostile, PARTY, 10);
        assert_eq!(default_stance(&allies, &factions), Stance::Friendly);
    }

    /// Turncoat swaps a monster's side, not a neutral's; monsters alike
    /// are friends, a neutral stays out, and two of one faction quarrel
    /// only when Turncoat holds one of them (retail `MonsterHostility`).
    #[test]
    fn monsters_stand_by_their_sides_and_turncoat_swaps_them() {
        assert_eq!(own_stance(Stance::Hostile, true), Stance::Friendly);
        assert_eq!(own_stance(Stance::Neutral, true), Stance::Neutral);
        // A member is of no faction, so a monster turned on its own side
        // quarrels with them and its former fellows alike.
        let foe = (Stance::Hostile, Loyalty::None);
        let turned = (own_stance(Stance::Hostile, true), Loyalty::None);
        assert_eq!(between(foe, foe), Stance::Friendly);
        assert_eq!(between(foe, turned), Stance::Hostile);
        assert_eq!(between(foe, (Stance::Neutral, Loyalty::None)), Stance::Neutral);
        let mook = |stance| (stance, Loyalty::Faction(crate::game::factions::MOOK));
        assert_eq!(between(mook(Stance::Hostile), mook(Stance::Friendly)), Stance::Neutral);
        assert_eq!(between(mook(Stance::Hostile), turned), Stance::Hostile);
        assert_eq!(between(foe, (Stance::Hostile, Loyalty::Bystander)), Stance::Neutral);
    }

    /// A provocation holds for Intelligence² game minutes outside a fight,
    /// and for the whole fight; a faction's band moving since releases it.
    #[test]
    fn a_set_stance_holds_for_its_while_unless_the_faction_moves() {
        let mut factions = Factions::default();
        let crab = species(Disposition::HostileWithin(15.0), UNALIGNED, 10);
        let set = Some(SetStance { stance: Stance::Hostile, at: 1_000 });
        let hold = 10 * 10 * crate::game::time::MINUTE_MS;
        assert_eq!(stance(set, &crab, &factions, 1_000 + hold, false), Stance::Hostile);
        assert_eq!(stance(set, &crab, &factions, 1_001 + hold, false), Stance::Neutral);
        assert_eq!(stance(set, &crab, &factions, 1_001 + hold, true), Stance::Hostile);
        assert_eq!(stance(None, &crab, &factions, 0, true), Stance::Neutral);
        let mook = species(Disposition::Neutral, crate::game::factions::MOOK, 10);
        let calm = Some(SetStance { stance: Stance::Neutral, at: 1_000 });
        assert_eq!(stance(calm, &mook, &factions, 2_000, false), Stance::Neutral);
        assert!(factions.record_offence(crate::game::factions::MOOK, false, 1_500).is_some());
        assert_eq!(
            stance(calm, &mook, &factions, 2_000, false),
            Stance::Hostile,
            "the Mook band fell after the stance was set"
        );
    }
}
