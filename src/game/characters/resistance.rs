//! Resistance from a character's profile and currently worn equipment.
use crate::content::characters::{Attribute, Realm, Skill};
#[cfg(test)]
use crate::game::characters::Character;
use crate::game::characters::{Profession, Race};
use crate::game::inventory::EquipSlot;
use crate::game::items::Protection;
use enum_map::{EnumMap, enum_map};
use enumset::EnumSet;

/// Base resistance and its current value, before unmodelled spell effects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Resistance {
    pub base: i32,
    pub total: i32,
}

/// Retail `RecalculateCharacterResistances` and equipment modifier accumulation.
/// Other temporary effects are not represented yet.
///
/// `worn` is what protects, worn in a slot, and whether it is a shield.
pub fn resistances<'a>(
    character: &crate::game::characters::Character,
    a: &crate::game::actors::Attributes,
    worn: impl Fn(EquipSlot) -> Option<(&'a Protection, bool)>,
) -> EnumMap<Realm, Resistance> {
    use Realm::*;
    let mut racial: EnumMap<Realm, i32> = enum_map! { _ => 0 };
    let vitality = a[Attribute::Vitality] as i32 / 5;
    let changes: &[(Realm, i32)] = match character.race {
        Race::Human => &[],
        Race::Elf => &[(Mental, 20), (Air, 10)],
        Race::Dwarf => &[(Fire, vitality)],
        Race::Gnome => &[(Mental, vitality), (Earth, 10)],
        Race::Hobbit => &[(Earth, vitality)],
        Race::Faerie => &[(Air, 15), (Earth, 15), (Mental, 15), (Divine, 15)],
        Race::Lizardman => &[(Water, 10), (Earth, 10), (Fire, 15), (Divine, -10), (Mental, -10)],
        Race::Dracon => &[(Water, 15), (Air, 5), (Divine, -5), (Mental, -5)],
        Race::Felpurr => &[(Water, -15), (Earth, 10), (Air, 10), (Mental, 10)],
        Race::Rawulf => &[(Water, 10), (Earth, 5), (Divine, 15)],
        Race::Mook => &[(Divine, 10), (Water, 15), (Mental, 15)],
        Race::Trynnie => &[(Earth, 20), (Air, 20), (Mental, -10)],
        Race::TRang => &[(Mental, 15), (Water, 15)],
        Race::Umpani => &[(Earth, 15), (Air, 15)],
        Race::Rapax => &[(Fire, 45), (Mental, -20)],
        Race::Android => &[(Mental, 75), (Divine, -25)],
    };
    for &(realm, amount) in changes {
        racial[realm] += amount;
    }
    EnumMap::from_fn(|realm: Realm| {
        let skill = realm.skill();
        let iron_will = if character.skill_available(Skill::IronWill) {
            character.skill(Skill::IronWill) as i32 / 5 + 5
        } else {
            0
        };
        let mental =
            if realm == Mental { (a[Attribute::Intelligence] as i32 - 80).max(0) / 2 } else { 0 };
        let divine = if realm == Divine { (a[Attribute::Piety] as i32 - 80).max(0) / 2 } else { 0 };
        let base = 25
            + character.skill(skill) as i32 / 10
            + iron_will
            + i32::from(character.profession == Profession::Mage) * 5
            + racial[realm]
            + mental
            + divine;
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
    use crate::content::characters::{Gender, Profession};
    #[test]
    fn racial_and_worn_item_resistances_apply_without_alternate_weapons() {
        let elf = Character::new("Elf", Race::Elf, Gender::Female, Profession::Mage);
        let charm = Protection {
            armour: 0,
            resistance: enum_map! { Realm::Air => 12, Realm::Fire => -7, _ => 0 },
            regeneration: [0; 3],
        };
        let sheet = crate::game::actors::Attributes::of_sheet(&elf);
        let base = resistances(&elf, &sheet, |slot| {
            (slot == EquipSlot::AlternatePrimary).then_some((&charm, false))
        });
        assert_eq!(base[Realm::Air], Resistance { base: 40, total: 40 });
        assert_eq!(base[Realm::Mental], Resistance { base: 50, total: 50 });
        let worn = resistances(&elf, &sheet, |slot| {
            matches!(slot, EquipSlot::AlternatePrimary | EquipSlot::Primary)
                .then_some((&charm, false))
        });
        assert_eq!(worn[Realm::Air], Resistance { base: 40, total: 52 });
        assert_eq!(worn[Realm::Fire], Resistance { base: 30, total: 23 });
    }
}
