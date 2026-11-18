//! Resistance from what an actor is, what it trained and what it wears.
use crate::content::characters::{Realm, Resist};
use crate::game::inventory::EquipSlot;
use crate::game::items::Protection;
use enum_map::EnumMap;
use enumset::EnumSet;

/// Base resistance and its value with what is worn, before spells.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Resistance {
    pub base: i32,
    pub total: i32,
}

/// What a nature resists in a realm at this Vitality.
fn natural(resist: Option<Resist>, vitality: i32) -> i32 {
    match resist {
        None => 0,
        Some(Resist::Flat(amount)) => amount,
        Some(Resist::OfVitality(divisor)) => vitality / divisor,
    }
}

/// What training adds in a realm (retail `RecalculateCharacterResistances`):
/// 25, a tenth of the realm's skill, Iron Will's a fifth and five if it may be
/// used, the profession's own, and half of Intelligence past 80 against the
/// mind or of Piety past 80 against the divine.
pub fn trained(
    realm: Realm,
    realm_skill: u32,
    iron_will: Option<u32>,
    profession: i32,
    (intelligence, piety): (u32, u32),
) -> i32 {
    let iron_will = iron_will.map_or(0, |skill| skill as i32 / 5 + 5);
    let high = |attribute: u32| (attribute as i32 - 80).max(0) / 2;
    let keen = match realm {
        Realm::Mental => high(intelligence),
        Realm::Divine => high(piety),
        _ => 0,
    };
    25 + realm_skill as i32 / 10 + iron_will + profession + keen
}

/// An actor's resistance in each realm: what it has by `nature` at this
/// Vitality, what its training adds, and with what it wears in the slots in
/// use, 100 at most.
///
/// `worn` is what protects, worn in a slot, and whether it is a shield.
pub fn resistances<'a>(
    nature: &EnumMap<Realm, Option<Resist>>,
    trained: &EnumMap<Realm, i32>,
    vitality: u32,
    worn: impl Fn(EquipSlot) -> Option<(&'a Protection, bool)>,
) -> EnumMap<Realm, Resistance> {
    EnumMap::from_fn(|realm: Realm| {
        let base = natural(nature[realm], vitality as i32) + trained[realm];
        let equipment: i32 = EnumSet::<EquipSlot>::all()
            .iter()
            .filter(|slot| slot.is_active())
            .filter_map(&worn)
            .map(|(protection, _)| i32::from(protection.resistance[realm]))
            .sum::<i32>()
            .clamp(-125, 125);
        Resistance { base, total: (base + equipment).min(100) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::{Profession, Race};

    /// A Dwarf resists fire by a fifth of vitality, and a Mage every realm by
    /// five more.
    #[test]
    fn a_dwarfs_fire_follows_vitality_and_a_mages_every_realm_is_five_higher() {
        let base = |race: Race, profession: Profession, realm| {
            let own = profession.row().resistance;
            let trained = EnumMap::from_fn(|realm| trained(realm, 0, None, own, (50, 50)));
            resistances(&race.row().resistance, &trained, 60, |_| None)[realm].base
        };
        // 25, and vitality 60 / 5 = 12.
        assert_eq!(base(Race::Dwarf, Profession::Fighter, Realm::Fire), 37);
        assert_eq!(base(Race::Dwarf, Profession::Fighter, Realm::Water), 25);
        assert_eq!(base(Race::Human, Profession::Mage, Realm::Water), 30);
    }
}
