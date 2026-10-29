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
    /// Control reads the actual condition; expiry/cure restores the group's
    /// current stance without a second writable allegiance field.
    pub fn controlled(self, monster: &Monster) -> Self {
        if !monster
            .conditions
            .iter()
            .any(|(c, _)| *c == crate::game::characters::Condition::Turncoat)
        {
            return self;
        }
        match self {
            Self::Hostile => Self::Friendly,
            Self::Friendly => Self::Hostile,
            Self::Neutral => Self::Neutral,
        }
    }
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
    intelligence * intelligence * 60_000
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
                || (now.saturating_sub(set.at) <= holds_for(species)
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

/// A monster's stance now, through its group, for operations that hold the
/// whole world. A monster outside any group stands by its species' default.
pub fn monster_stance(world: &World, monster: Entity) -> Stance {
    let Some(species) = world
        .get::<Monster>(monster)
        .and_then(|m| crate::game::data::GameData::species(world)?.kind(m.species))
    else {
        return Stance::Neutral;
    };
    let set = world
        .get::<GroupMember>(monster)
        .and_then(|member| world.get::<MonsterGroup>(member.0))
        .and_then(|group| group.set);
    let fighting = world.get::<crate::game::combat::Combatant>(monster).is_some();
    let now = world.resource::<GameClock>().ms;
    stance(set, species, world.resource::<Factions>(), now, fighting)
        .controlled(world.get::<Monster>(monster).unwrap())
}

/// Monsters' stances now, through their groups, for systems that only
/// read them (the fight's snapshot, the radar). Time is the caller's: the
/// fight holds the clock itself.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Stances<'w, 's> {
    monsters: Query<
        'w,
        's,
        (&'static Monster, Option<&'static GroupMember>, Has<crate::game::combat::Combatant>),
    >,
    groups: Query<'w, 's, &'static MonsterGroup>,
    factions: Res<'w, Factions>,
}

impl Stances<'_, '_> {
    /// A monster's group's stance now; one outside any group by its
    /// species' default.
    pub fn group(&self, monster: Entity, species: &MonsterCatalog, now: u64) -> Option<Stance> {
        let (body, member, fighting) = self.monsters.get(monster).ok()?;
        let kind = species.kind(body.species)?;
        let set = member.and_then(|m| self.groups.get(m.0).ok()).and_then(|g| g.set);
        Some(stance(set, kind, &self.factions, now, fighting))
    }

    /// A monster's own stance now: its group's, swapped by its own Turncoat
    /// (retail's per-monster `ubDisposition`).
    pub fn own(&self, monster: Entity, species: &MonsterCatalog, now: u64) -> Option<Stance> {
        let group = self.group(monster, species, now)?;
        let (body, ..) = self.monsters.get(monster).ok()?;
        let turncoat = body.has(Condition::Turncoat);
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

/// How a monster stands toward a member: as it stands toward the party,
/// hostile and friendly swapped for a member under Turncoat (retail
/// `MonsterVsCharDisposition`).
pub fn toward_member(monster: Stance, member_turncoat: bool) -> Stance {
    own_stance(monster, member_turncoat)
}

/// A monster as another sees it: its species, own stance, faction and
/// whether it is under Turncoat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allegiance {
    pub species: u32,
    pub stance: Stance,
    pub faction: i32,
    pub turncoat: bool,
}

/// How two monsters stand toward each other (retail `MonsterHostility`):
/// alike, friends; either neutral, neutral; otherwise foes, but for two of
/// one faction, who quarrel only when Turncoat holds one and not the other.
pub fn between(first: Allegiance, second: Allegiance) -> Stance {
    if first.species == BYSTANDER_SPECIES || second.species == BYSTANDER_SPECIES {
        return Stance::Neutral;
    }
    if first.stance == second.stance {
        return Stance::Friendly;
    }
    if first.stance == Stance::Neutral || second.stance == Stance::Neutral {
        return Stance::Neutral;
    }
    if first.faction == UNALIGNED || second.faction == UNALIGNED || first.faction != second.faction
    {
        return Stance::Hostile;
    }
    if first.turncoat == second.turncoat { Stance::Neutral } else { Stance::Hostile }
}

/// The party acted against this monster: a blow aimed at it, a hostile
/// spell aimed at it or catching it, whatever came of them (retail
/// `MakeTargetGroupHostile`, `ProvokeListedMonsterGroups`).
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Provocation(pub Entity);

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
        assert_eq!(toward_member(Stance::Hostile, true), Stance::Friendly);
        let side = |stance, faction, turncoat| Allegiance { species: 1, stance, faction, turncoat };
        let foe = side(Stance::Hostile, UNALIGNED, false);
        let turned = side(own_stance(Stance::Hostile, true), UNALIGNED, true);
        assert_eq!(between(foe, foe), Stance::Friendly);
        assert_eq!(between(foe, turned), Stance::Hostile);
        assert_eq!(between(foe, side(Stance::Neutral, UNALIGNED, false)), Stance::Neutral);
        let mook = |stance, turncoat| side(stance, crate::game::factions::MOOK, turncoat);
        assert_eq!(
            between(mook(Stance::Hostile, false), mook(Stance::Friendly, false)),
            Stance::Neutral
        );
        assert_eq!(
            between(mook(Stance::Hostile, false), mook(Stance::Friendly, true)),
            Stance::Hostile
        );
        let bystander = Allegiance { species: BYSTANDER_SPECIES, ..foe };
        assert_eq!(between(foe, bystander), Stance::Neutral);
    }

    /// A provocation holds for Intelligence² game minutes outside a fight,
    /// and for the whole fight; a faction's band moving since releases it.
    #[test]
    fn a_set_stance_holds_for_its_while_unless_the_faction_moves() {
        let mut factions = Factions::default();
        let crab = species(Disposition::HostileWithin(15.0), UNALIGNED, 10);
        let set = Some(SetStance { stance: Stance::Hostile, at: 1_000 });
        let hold = 10 * 10 * 60_000;
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
