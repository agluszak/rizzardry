//! Resistance from what an actor is, what it trained and what it wears.
use crate::content::characters::{Attribute, Realm, Resist, Skill};
use crate::game::actors::{Attributes, Nature, Skills};
use crate::game::characters::Trained;
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
fn natural(nature: &Nature, realm: Realm, vitality: i32) -> i32 {
    match nature.resistance[realm] {
        None => 0,
        Some(Resist::Flat(amount)) => amount,
        Some(Resist::OfVitality(divisor)) => vitality / divisor,
    }
}

/// What training adds in a realm (retail `RecalculateCharacterResistances`):
/// 25, a tenth of the realm's skill, Iron Will, the profession's own, and
/// high Intelligence or Piety against the mind or the divine.
fn trained(c: Trained, a: &Attributes, skills: &Skills, realm: Realm) -> i32 {
    let iron_will =
        if c.can_use(Skill::IronWill) { skills[Skill::IronWill] as i32 / 5 + 5 } else { 0 };
    let high = |attribute| (a[attribute] as i32 - 80).max(0) / 2;
    let keen = match realm {
        Realm::Mental => high(Attribute::Intelligence),
        Realm::Divine => high(Attribute::Piety),
        _ => 0,
    };
    25 + skills[realm.skill()] as i32 / 10
        + iron_will
        + c.calling.profession.row().resistance
        + keen
}

/// An actor's resistance in each realm: what it has by nature and by
/// training, and with what it wears in the slots in use, 100 at most.
///
/// `worn` is what protects, worn in a slot, and whether it is a shield.
pub fn resistances<'a>(
    (nature, c): (&Nature, Option<Trained>),
    a: &Attributes,
    skills: &Skills,
    worn: impl Fn(EquipSlot) -> Option<(&'a Protection, bool)>,
) -> EnumMap<Realm, Resistance> {
    let vitality = a[Attribute::Vitality] as i32;
    EnumMap::from_fn(|realm: Realm| {
        let base = natural(nature, realm, vitality) + c.map_or(0, |c| trained(c, a, skills, realm));
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
    use crate::content::characters::{Gender, Profession, Race};
    use crate::game::characters::Character;

    /// A Dwarf resists fire by a fifth of vitality, and a Mage every realm by
    /// five more.
    #[test]
    fn a_dwarfs_fire_follows_vitality_and_a_mages_every_realm_is_five_higher() {
        let base = |sheet: &Character, realm| {
            let nature = Nature::of_race(sheet.birth.race);
            let (a, skills) = (Attributes::of_sheet(sheet), Skills::of_sheet(sheet));
            resistances((&nature, Some(sheet.trained())), &a, &skills, |_| None)[realm].base
        };
        let dwarf = Character::new("D", Race::Dwarf, Gender::Male, Profession::Fighter);
        // 25, and vitality 60 / 5 = 12.
        assert_eq!(base(&dwarf, Realm::Fire), 37);
        assert_eq!(base(&dwarf, Realm::Water), 25);
        let mage = Character::new("M", Race::Human, Gender::Male, Profession::Mage);
        assert_eq!(base(&mage, Realm::Water), 30);
    }
}
