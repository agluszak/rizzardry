//! Whether a cast may be made (`docs/rules/casting.md` section 2): who may
//! cast what, from what, when. A function of the facts it is about and
//! nothing else.

use crate::content::items::UseType;
use crate::content::spells::{PowerChoice, SpellDefinition, Usable, Working};
use crate::game::characters::{Condition, Trained};
use crate::game::hits::effects::BodyReadOnlyItem;
use crate::game::items::{Casts, Items};
use crate::game::magic::casting::Source;
use crate::game::magic::power::AUTO;

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

/// Where the party is, which decides which spells suit the moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseContext {
    /// Exploring.
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
    character: Trained,
    attributes: &crate::game::actors::Attributes,
    skills: &crate::game::actors::Skills,
) -> bool {
    !matches!(casts.by, UseType::None | UseType::Spellbook)
        && (!casts.worn || worn)
        && (casts.users.admit((character.calling, character.birth), attributes, skills)
            || matches!(casts.by, UseType::Potion | UseType::Food))
}

/// How the party stands.
#[derive(Clone, Copy, Debug)]
pub struct Moment {
    pub context: UseContext,
    /// The party is held by something: a conversation, a flight, work on a
    /// device, a level being entered.
    pub occupied: bool,
    /// A camp is up.
    pub camped: bool,
    /// A conversation waits for the party's word.
    pub spoken_to: bool,
    /// The party is in the air or standing on a prop.
    pub unsteady: bool,
}

/// Whether a utility spell waits for another moment: some are not cast in a
/// fight or while the party is occupied, a spot is not marked in a camp, an
/// elemental is summoned only in a fight, and the one talked with must be
/// waiting for the party's word.
fn waits(spell: &SpellDefinition, moment: Moment) -> bool {
    match spell.facts.working {
        // A portal is not set where the party is not on steady ground.
        Some(Working::SetPortal) => {
            moment.context == UseContext::Combat || moment.occupied || moment.unsteady
        }
        Some(Working::Identify | Working::ReturnToPortal) => {
            moment.context == UseContext::Combat || moment.occupied
        }
        Some(Working::Lure) => moment.occupied || moment.camped,
        Some(Working::Summon) => {
            moment.occupied || moment.camped || moment.context != UseContext::Combat
        }
        Some(Working::Charm | Working::Mindread) => !moment.spoken_to,
        _ => false,
    }
}

/// Whether a moment suits a spell cast by a member: its record's times and
/// its own waiting; "repeat until whole" (`repeats`) is not for a fight.
pub fn suits(spell: &SpellDefinition, moment: Moment, repeats: bool) -> bool {
    usable_now(spell, moment.context)
        && !(repeats && moment.context == UseContext::Combat)
        && !waits(spell, moment)
}

/// Whether a cast may be made, and if not, the first reason why not, in
/// this order (retail `CastSpellFromSource`, `ValidateSpellCast`):
///
/// 1. One who cannot act casts nothing; a monster casts only its own spells.
/// 2. In a fight the party's pool is out of reach.
/// 3. A silence stops what is voiced, a learned spell, a monster's own or an
///    instrument, but for Alchemy in the hands of one who keeps it.
/// 4. A learned spell is asked at some power, is known, and the points pay
///    for it at power 1 at least.
/// 5. An item is there (carried by the caster or in the pool), casts a spell
///    at some power (an instrument's is played), may be set off by its user
///    and has a charge left.
/// 6. The game resolves the spell.
/// 7. The moment suits it; a monster's own spell is not judged by the moment.
pub fn may(
    spell: &SpellDefinition,
    caster: &BodyReadOnlyItem,
    source: Source,
    held: &Items,
    (affordable, keeps_alchemy): (u32, bool),
    moment: Moment,
    resolved: bool,
) -> Result<(), Refusal> {
    let own = matches!(source, Source::Innate { .. });
    if !caster.can_fight || !(caster.member || own) {
        return Err(Refusal::CannotAct);
    }
    let item = match source {
        Source::Item(item) => Some(item),
        _ => None,
    };
    let pooled = item.is_some_and(|item| held.pooled(item));
    if pooled && moment.context == UseContext::Combat {
        return Err(Refusal::CannotUse);
    }
    let there = item
        .filter(|&item| pooled || held.bearer(item) == Some(caster.entity))
        .filter(|&item| held.id(item).is_some());
    let casts = there.map(|item| held.casts(item));
    let voiced = casts.is_none_or(|casts| casts.is_some_and(|c| c.by == UseType::Instrument));
    if caster.conditions.has(Condition::Silenced) && voiced && silence_stops(spell, keeps_alchemy) {
        return Err(Refusal::Silenced);
    }
    match source {
        Source::Spell { spell: id, power } => {
            if power == 0 {
                return Err(Refusal::CannotUse);
            }
            if !caster.spells.is_some_and(|(learned, _)| learned.0.contains(&id)) {
                return Err(Refusal::NotLearned);
            }
            if affordable == 0 {
                return Err(Refusal::NoSpellPoints);
            }
        }
        Source::Item(_) => {
            let Some(item) = there else { return Err(Refusal::NoItem) };
            let Some(casts) = held.casts(item) else { return Err(Refusal::CannotUse) };
            let played = matches!(casts.by, UseType::Instrument | UseType::Gadget);
            let worn = held.slot(item).is_some_and(|slot| slot.is_active());
            let user = caster
                .trained()
                .is_some_and(|c| activates(casts, worn, c, caster.attributes, caster.skills));
            if (casts.power == 0 && !played) || !user {
                return Err(Refusal::CannotUse);
            }
            if held.charges(item) == Some(0) {
                return Err(Refusal::NoCharges);
            }
        }
        Source::Innate { .. } => {}
    }
    if !resolved {
        return Err(Refusal::Unsupported);
    }
    let repeats = matches!(source, Source::Spell { power: AUTO, .. })
        && spell.power == PowerChoice::ChosenOrRepeat;
    if !own && !suits(spell, moment, repeats) {
        return Err(Refusal::WrongTime);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::spells::{SpellCatalog, SpellId};

    fn spell(id: SpellId) -> SpellDefinition {
        SpellCatalog::new(vec![SpellDefinition { id, ..Default::default() }]).0.remove(0)
    }

    const FREE: Moment = Moment {
        context: UseContext::World,
        occupied: false,
        camped: false,
        spoken_to: false,
        unsteady: false,
    };

    /// Identify and the portals wait out a fight and a busy party; a lure
    /// waits out a camp as well; Summon Elemental is for a fight only; Charm
    /// waits for the party's word.
    #[test]
    fn a_utility_spell_waits_for_its_moment() {
        let at = |id, moment| {
            if suits(&spell(id), moment, false) { Ok(()) } else { Err(Refusal::WrongTime) }
        };
        let fight = Moment { context: UseContext::Combat, ..FREE };
        let busy = Moment { occupied: true, ..FREE };
        let camp = Moment { context: UseContext::Camp, camped: true, ..FREE };
        for id in [SpellId::IDENTIFY_ITEM, SpellId::SET_PORTAL, SpellId::RETURN_TO_PORTAL] {
            assert_eq!(at(id, fight), Err(Refusal::WrongTime), "{id:?}");
            assert_eq!(at(id, busy), Err(Refusal::WrongTime), "{id:?}");
            assert_eq!(at(id, FREE), Ok(()), "{id:?}");
            assert_eq!(at(id, camp), Ok(()), "a camp does not stop {id:?}");
        }
        assert_eq!(at(SpellId::HYPNOTIC_LURE, camp), Err(Refusal::WrongTime));
        assert_eq!(at(SpellId::HYPNOTIC_LURE, busy), Err(Refusal::WrongTime));
        assert_eq!(at(SpellId::HYPNOTIC_LURE, FREE), Ok(()));
        assert_eq!(at(SpellId::HYPNOTIC_LURE, fight), Ok(()));
        assert_eq!(at(SpellId::SUMMON_ELEMENTAL, FREE), Err(Refusal::WrongTime));
        assert_eq!(at(SpellId::SUMMON_ELEMENTAL, fight), Ok(()));
        let talking = Moment { context: UseContext::Conversation, occupied: true, ..FREE };
        assert_eq!(at(SpellId::CHARM, talking), Err(Refusal::WrongTime));
        let heard = Moment { spoken_to: true, ..talking };
        assert_eq!(at(SpellId::CHARM, heard), Ok(()));
        assert_eq!(at(SpellId::MINDREAD, heard), Ok(()));
        assert_eq!(at(SpellId::HEAL_WOUNDS, heard), Err(Refusal::WrongTime));
    }
}
