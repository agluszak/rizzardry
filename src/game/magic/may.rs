//! Whether a cast may be made (`docs/rules/casting.md` section 2): who may
//! cast what, from what, when. A function of the facts it is about and
//! nothing else.

use bevy::prelude::*;

use crate::content::items::UseType;
use crate::content::spells::{SpellDefinition, Usable, Working};
use crate::game::items::Casts;

/// Why nothing was spent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    CannotAct,
    Silenced,
    NotLearned,
    NoSpellPoints,
    NoItem,
    NoCharges,
    CannotUse,
    /// Its effect is not implemented yet.
    Unsupported,
    /// Not now: only in combat, or only out of it.
    WrongTime,
    NoTarget,
    /// An instrument or gadget that could not be brought to play: its
    /// fatigue is paid, the item is not (retail string 0x1f3). The attempt
    /// is practice.
    PlayedBadly,
}

/// Where the party is, which decides which spells suit the moment: a
/// figure on the party, written by [`crate::game::magic::occasion`].
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UseContext {
    /// Exploring.
    #[default]
    World,
    Combat,
    Camp,
    Conversation,
    Lock,
    Trap,
}

/// Whether a spell suits the party's circumstances by its own record: at any
/// time, in or out of combat, or while camped; in a conversation only the
/// spells cast at the one the party talks with, at a lock only those cast at a
/// lock, at a trap only those cast at a trap.
pub fn usable_now(spell: &SpellDefinition, context: UseContext) -> bool {
    let working = spell.facts.working;
    match context {
        UseContext::Conversation => working.is_some_and(Working::speaks),
        UseContext::Lock => working.is_some_and(Working::at_lock),
        UseContext::Trap => working.is_some_and(Working::at_trap),
        _ => match spell.usable {
            Usable::Always => true,
            Usable::InCombat => context == UseContext::Combat,
            Usable::OutOfCombat => context != UseContext::Combat,
            Usable::Camped => context == UseContext::Camp,
            Usable::OnLockOrTrap => false,
        },
    }
}

/// Whether a silence stops this caster's spell: all but an Alchemy spell in
/// the hands of one who keeps their Alchemy (retail `IsSpellBlockedForCharacter`,
/// `IsSpellBlockedForMonster`).
pub fn silence_stops(spell: &SpellDefinition, keeps_alchemy: bool) -> bool {
    !(spell.books.contains(crate::content::spells::Spellbook::Alchemy) && keeps_alchemy)
}

/// Whether an item's spell may be set off by a character: it has a use,
/// it is worn where it is in use if it must be, and they are among those who
/// may use it, or it is a potion or food, which anyone takes.
pub fn activates(
    casts: &Casts,
    worn: bool,
    (calling, birth): (&crate::game::characters::Calling, &crate::game::characters::Birth),
    attributes: &crate::game::actors::Attributes,
    skills: &crate::game::actors::Skills,
) -> bool {
    !matches!(casts.by, UseType::None | UseType::Spellbook)
        && (!casts.worn || worn)
        && (casts.users.admit((calling, birth), attributes, skills)
            || matches!(casts.by, UseType::Potion | UseType::Food))
}

/// Whether a utility spell waits for another moment: some are not cast in a
/// fight or while the party is occupied, a spot is not marked in a camp, an
/// elemental is summoned only in a fight, and the one talked with must be
/// waiting for the party's word.
fn waits(
    spell: &SpellDefinition,
    context: UseContext,
    occupied: bool,
    camped: bool,
    spoken_to: bool,
    unsteady: bool,
) -> bool {
    let fighting = context == UseContext::Combat;
    match spell.facts.working {
        // A portal is not set where the party is not on steady ground.
        Some(Working::SetPortal) => fighting || occupied || unsteady,
        Some(Working::Identify | Working::ReturnToPortal) => fighting || occupied,
        Some(Working::Lure) => occupied || camped,
        Some(Working::Summon) => occupied || camped || !fighting,
        Some(Working::Charm | Working::Mindread) => !spoken_to,
        _ => false,
    }
}

/// Whether the party's moment suits a spell cast by a member: its record's
/// times and its own waiting; "repeat until whole" (`repeats`) is not for a
/// fight. The party's [`UseContext`], whether it is occupied, camped, spoken
/// to and unsteady are its figures.
pub fn suits(
    spell: &SpellDefinition,
    context: UseContext,
    occupied: bool,
    camped: bool,
    spoken_to: bool,
    unsteady: bool,
    repeats: bool,
) -> bool {
    usable_now(spell, context)
        && !(repeats && context == UseContext::Combat)
        && !waits(spell, context, occupied, camped, spoken_to, unsteady)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::spells::{SpellCatalog, SpellId};

    fn spell(id: SpellId) -> SpellDefinition {
        SpellCatalog::new(vec![SpellDefinition { id, ..Default::default() }]).0.remove(0)
    }

    /// Identify and the portals wait out a fight and a busy party; a lure
    /// waits out a camp as well; Summon Elemental is for a fight only; Charm
    /// waits for the party's word; while talking, nothing else is cast.
    #[test]
    fn a_utility_spell_waits_for_its_moment() {
        use UseContext::*;
        // (context, occupied, camped, spoken to)
        let at = |id, (context, occupied, camped, spoken_to)| {
            if suits(&spell(id), context, occupied, camped, spoken_to, false, false) {
                Ok(())
            } else {
                Err(Refusal::WrongTime)
            }
        };
        let free = (World, false, false, false);
        let fight = (Combat, false, false, false);
        let busy = (World, true, false, false);
        let camp = (Camp, false, true, false);
        for id in [SpellId::IDENTIFY_ITEM, SpellId::SET_PORTAL, SpellId::RETURN_TO_PORTAL] {
            assert_eq!(at(id, fight), Err(Refusal::WrongTime), "{id:?}");
            assert_eq!(at(id, busy), Err(Refusal::WrongTime), "{id:?}");
            assert_eq!(at(id, free), Ok(()), "{id:?}");
            assert_eq!(at(id, camp), Ok(()), "a camp does not stop {id:?}");
        }
        assert_eq!(at(SpellId::HYPNOTIC_LURE, camp), Err(Refusal::WrongTime));
        assert_eq!(at(SpellId::HYPNOTIC_LURE, busy), Err(Refusal::WrongTime));
        assert_eq!(at(SpellId::HYPNOTIC_LURE, free), Ok(()));
        assert_eq!(at(SpellId::HYPNOTIC_LURE, fight), Ok(()));
        assert_eq!(at(SpellId::SUMMON_ELEMENTAL, free), Err(Refusal::WrongTime));
        assert_eq!(at(SpellId::SUMMON_ELEMENTAL, fight), Ok(()));
        let talking = (Conversation, true, false, false);
        assert_eq!(at(SpellId::CHARM, talking), Err(Refusal::WrongTime));
        let heard = (Conversation, true, false, true);
        assert_eq!(at(SpellId::CHARM, heard), Ok(()));
        assert_eq!(at(SpellId::MINDREAD, heard), Ok(()));
        assert_eq!(at(SpellId::HEAL_WOUNDS, heard), Err(Refusal::WrongTime));
    }
}
