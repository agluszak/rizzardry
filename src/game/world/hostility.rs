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
use crate::content::species::{Disposition, Species};
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
