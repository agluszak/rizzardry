//! Whether a cast may be made (`docs/rules/casting.md` section 2): who may
//! cast what, from what, when. A function of the facts it is about and
//! nothing else.

use crate::content::spells::{PowerChoice, SpellDefinition, Usable, Working};
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

/// Whether the user may activate an item.
#[derive(Clone, Copy, Debug)]
pub struct Activation {
    /// It has a use (not a plain thing, nor a spellbook).
    pub has_use: bool,
    /// It works only worn (weapons, armour, worn misc things, anything marked).
    pub must_be_worn: bool,
    /// It is worn in a slot in use.
    pub worn: bool,
    /// The user's profession, race and sex may use it.
    pub usable_by_user: bool,
    /// Potions and food are usable by anyone who holds them.
    pub by_anyone: bool,
}

/// Whether the user may activate the item: it has a use, is worn if it must
/// be, and is usable by them, or by anyone.
pub fn activates(item: &Activation) -> bool {
    item.has_use && (!item.must_be_worn || item.worn) && (item.usable_by_user || item.by_anyone)
}

/// An item a cast would use.
#[derive(Clone, Copy, Debug)]
pub struct Held {
    /// The power of the spell its record carries; none if it carries no spell.
    pub power: Option<u32>,
    /// An instrument or gadget, which is played at a power of the player's
    /// reach rather than carrying one of its own.
    pub played: bool,
    /// Its use is voiced (an instrument).
    pub voiced: bool,
    pub activation: Activation,
    /// A charged item with no charge left.
    pub spent: bool,
}

/// What a cast is paid for from.
#[derive(Clone, Copy, Debug)]
pub enum Source {
    /// A learned spell, at the power asked for (or [`AUTO`]), which the points
    /// pay for up to `affordable`.
    Learned { learned: bool, asked: u32, affordable: u32 },
    /// An item, in the party's pool and not carried by the caster if `pooled`;
    /// none if it is no longer there, or not the caster's to use.
    Item { pooled: bool, held: Option<Held> },
    /// A monster's own spell.
    Own,
}

/// The caster, as the rule asks of them.
#[derive(Clone, Copy, Debug)]
pub struct Caster {
    /// Alive, with nothing as bad as Asleep.
    pub able: bool,
    pub member: bool,
    pub silenced: bool,
    /// Their Alchemy survives a silence: a member's with the skill, a
    /// monster's of a kind that keeps it.
    pub keeps_alchemy: bool,
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
}

/// What a cast asks.
#[derive(Clone, Copy, Debug)]
pub struct Asks<'a> {
    pub spell: &'a SpellDefinition,
    pub caster: Caster,
    pub source: Source,
    pub moment: Moment,
    /// The game resolves the spell: it has a row, or is a utility spell.
    pub resolved: bool,
}

/// Whether a utility spell waits for another moment: some are not cast in a
/// fight or while the party is occupied, a spot is not marked in a camp, an
/// elemental is summoned only in a fight, and the one talked with must be
/// waiting for the party's word.
fn waits(spell: &SpellDefinition, moment: Moment) -> bool {
    match spell.facts.working {
        Some(Working::Identify | Working::SetPortal | Working::ReturnToPortal) => {
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

/// Whether the cast may be made, or the first reason it may not
/// (retail `ExecuteCharacterSpellCast`, `UseItem`): by someone able to take an
/// action, a member unless it is a monster's own spell; from what a member
/// carries, in a fight, not the pool; not voiced under silence; a learned
/// spell known, at a power above 0 the points pay for; an item still there,
/// carrying a spell the user may activate, with a charge left; a spell the
/// game resolves; at a moment that suits it. The refusal costs nothing.
pub fn may(asks: &Asks) -> Result<(), Refusal> {
    let Asks { spell, caster, source, moment, resolved } = *asks;
    let own = matches!(source, Source::Own);
    if !caster.able || !(caster.member || own) {
        return Err(Refusal::CannotAct);
    }
    if let Source::Item { pooled: true, .. } = source
        && moment.context == UseContext::Combat
    {
        return Err(Refusal::CannotUse);
    }
    let voiced = match source {
        Source::Item { held: Some(held), .. } => held.voiced,
        _ => true,
    };
    if caster.silenced && voiced && silence_stops(spell, caster.keeps_alchemy) {
        return Err(Refusal::Silenced);
    }
    match source {
        Source::Learned { learned, asked, affordable } => {
            if asked == 0 {
                return Err(Refusal::CannotUse);
            }
            if !learned {
                return Err(Refusal::NotLearned);
            }
            if affordable == 0 {
                return Err(Refusal::NoSpellPoints);
            }
        }
        Source::Item { held: None, .. } => return Err(Refusal::NoItem),
        Source::Item { held: Some(held), .. } => {
            let Some(power) = held.power else { return Err(Refusal::CannotUse) };
            let powerless = power == 0 && !held.played;
            if powerless || !activates(&held.activation) {
                return Err(Refusal::CannotUse);
            }
            if held.spent {
                return Err(Refusal::NoCharges);
            }
        }
        Source::Own => {}
    }
    if !resolved {
        return Err(Refusal::Unsupported);
    }
    let repeats = matches!(source, Source::Learned { asked: AUTO, .. })
        && spell.power == PowerChoice::ChosenOrRepeat;
    let fighting = moment.context == UseContext::Combat;
    if !own && (!usable_now(spell, moment.context) || (repeats && fighting) || waits(spell, moment))
    {
        return Err(Refusal::WrongTime);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::spells::{SpellCatalog, SpellId, Spellbook};

    fn spell(id: SpellId) -> SpellDefinition {
        SpellCatalog::new(vec![SpellDefinition { id, ..Default::default() }]).0.remove(0)
    }

    const FREE: Moment =
        Moment { context: UseContext::World, occupied: false, camped: false, spoken_to: false };
    const ABLE: Caster = Caster { able: true, member: true, silenced: false, keeps_alchemy: false };

    fn item(held: Option<Held>) -> Source {
        Source::Item { pooled: false, held }
    }

    fn learned(asked: u32, affordable: u32) -> Source {
        Source::Learned { learned: true, asked, affordable }
    }

    fn asks(spell: &SpellDefinition, source: Source) -> Asks<'_> {
        Asks { spell, caster: ABLE, source, moment: FREE, resolved: true }
    }

    fn holding() -> Held {
        Held {
            power: Some(3),
            played: false,
            voiced: false,
            activation: Activation {
                has_use: true,
                must_be_worn: false,
                worn: false,
                usable_by_user: true,
                by_anyone: false,
            },
            spent: false,
        }
    }

    /// Silence stops a learned spell and an instrument, not another item; an
    /// Alchemy spell is let through to one who keeps their Alchemy.
    #[test]
    fn a_silence_stops_what_is_voiced() {
        let heal = spell(SpellId::HEAL_WOUNDS);
        let silent = Caster { silenced: true, ..ABLE };
        let silenced = |source| may(&Asks { caster: silent, ..asks(&heal, source) });
        assert_eq!(silenced(learned(3, 5)), Err(Refusal::Silenced));
        assert_eq!(
            silenced(item(Some(Held { voiced: true, ..holding() }))),
            Err(Refusal::Silenced)
        );
        assert_eq!(silenced(item(Some(holding()))), Ok(()));
        // An item that is not there is judged as a spell is.
        assert_eq!(silenced(item(None)), Err(Refusal::Silenced));
        let alchemy = SpellDefinition { books: Spellbook::Alchemy.into(), ..heal.clone() };
        assert_eq!(
            may(&Asks { caster: silent, ..asks(&alchemy, learned(3, 5)) }),
            Err(Refusal::Silenced)
        );
        let alchemist = Caster { silenced: true, keeps_alchemy: true, ..ABLE };
        assert_eq!(may(&Asks { caster: alchemist, ..asks(&alchemy, learned(3, 5)) }), Ok(()));
        // Only Alchemy: the alchemist's other spells are stopped.
        assert_eq!(
            may(&Asks { caster: alchemist, ..asks(&heal, learned(3, 5)) }),
            Err(Refusal::Silenced)
        );
        // A monster's own spell is stopped the same way.
        let monster = Caster { member: false, silenced: true, keeps_alchemy: false, ..ABLE };
        assert_eq!(
            may(&Asks { caster: monster, ..asks(&heal, Source::Own) }),
            Err(Refusal::Silenced)
        );
    }

    #[test]
    fn a_learned_spell_must_be_known_and_asked_at_a_power_the_points_pay_for() {
        let heal = spell(SpellId::HEAL_WOUNDS);
        assert_eq!(may(&asks(&heal, learned(0, 5))), Err(Refusal::CannotUse));
        let unknown = Source::Learned { learned: false, asked: 3, affordable: 5 };
        assert_eq!(may(&asks(&heal, unknown)), Err(Refusal::NotLearned));
        assert_eq!(may(&asks(&heal, learned(3, 0))), Err(Refusal::NoSpellPoints));
        // Power 0 is judged before the spell is known.
        let both = Source::Learned { learned: false, asked: 0, affordable: 5 };
        assert_eq!(may(&asks(&heal, both)), Err(Refusal::CannotUse));
    }

    #[test]
    fn an_item_must_be_there_carry_a_spell_be_activated_and_have_a_charge() {
        let heal = spell(SpellId::HEAL_WOUNDS);
        let try_item = |held| may(&asks(&heal, item(held)));
        assert_eq!(try_item(None), Err(Refusal::NoItem));
        assert_eq!(try_item(Some(holding())), Ok(()));
        assert_eq!(try_item(Some(Held { power: None, ..holding() })), Err(Refusal::CannotUse));
        // Power 0 is refused, unless it is played.
        assert_eq!(try_item(Some(Held { power: Some(0), ..holding() })), Err(Refusal::CannotUse));
        assert_eq!(try_item(Some(Held { power: Some(0), played: true, ..holding() })), Ok(()));
        assert_eq!(try_item(Some(Held { spent: true, ..holding() })), Err(Refusal::NoCharges));
        let not_theirs = Activation { usable_by_user: false, ..holding().activation };
        assert_eq!(
            try_item(Some(Held { activation: not_theirs, ..holding() })),
            Err(Refusal::CannotUse)
        );
    }

    fn suiting(usable: Usable) -> SpellDefinition {
        SpellDefinition { usable, ..spell(SpellId::HEAL_WOUNDS) }
    }

    /// "Repeat until whole" is refused in a fight, only for a spell whose record
    /// offers it.
    #[test]
    fn the_repeat_choice_of_power_is_refused_in_a_fight() {
        let repeats =
            SpellDefinition { power: PowerChoice::ChosenOrRepeat, ..suiting(Usable::Always) };
        let fight = Moment { context: UseContext::Combat, ..FREE };
        let in_fight = |spell: &SpellDefinition, asked| {
            may(&Asks { moment: fight, ..asks(spell, learned(asked, 5)) })
        };
        assert_eq!(in_fight(&repeats, AUTO), Err(Refusal::WrongTime));
        assert_eq!(in_fight(&repeats, 4), Ok(()));
        assert_eq!(in_fight(&suiting(Usable::Always), AUTO), Ok(()));
        assert_eq!(may(&asks(&repeats, learned(AUTO, 5))), Ok(()));
    }

    /// Identify and the portals wait out a fight and a busy party; a lure
    /// waits out a camp as well; Summon Elemental is for a fight only; Charm
    /// waits for the party's word.
    #[test]
    fn a_utility_spell_waits_for_its_moment() {
        let at = |id, moment| may(&Asks { moment, ..asks(&spell(id), learned(3, 5)) });
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
