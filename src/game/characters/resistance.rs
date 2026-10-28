//! Resistance from a character's profile and currently worn equipment.
use crate::content::characters::{Attribute, Realm, Skill};
use crate::content::items::ItemCatalog;
#[cfg(test)]
use crate::game::characters::Character;
use crate::game::characters::{Profession, Race};
use crate::game::inventory::Carried;
#[cfg(test)]
use crate::game::inventory::EquipSlot;
use enum_map::{EnumMap, enum_map};

/// Base resistance and its current value, before unmodelled spell effects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Resistance {
    pub base: i32,
    pub total: i32,
}

/// Retail `RecalculateCharacterResistances` and equipment modifier accumulation.
/// Other temporary effects are not represented yet.
pub fn resistances(
    character: &crate::game::characters::CharacterStats<'_>,
    carried: &Carried,
    items: &ItemCatalog,
) -> EnumMap<Realm, Resistance> {
    use Realm::*;
    let mut racial: EnumMap<Realm, i32> = enum_map! { _ => 0 };
    let vitality = character.attribute(Attribute::Vitality) as i32 / 5;
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
    EnumMap::from_fn(|realm| {
        let skill = Skill::try_from(Skill::FireMagic as u8 + realm as u8)
            .expect("realm skills follow realm order");
        let iron_will = if character.skill_available(Skill::IronWill) {
            character.skill(Skill::IronWill) as i32 / 5 + 5
        } else {
            0
        };
        let mental = if realm == Mental {
            (character.attribute(Attribute::Intelligence) as i32 - 80).max(0) / 2
        } else {
            0
        };
        let divine = if realm == Divine {
            (character.attribute(Attribute::Piety) as i32 - 80).max(0) / 2
        } else {
            0
        };
        let base = 25
            + character.skill(skill) as i32 / 10
            + iron_will
            + i32::from(character.profession == Profession::Mage) * 5
            + racial[realm]
            + mental
            + divine;
        let equipment: i32 = carried
            .active_equipment()
            .map(|(_, equipped)| &equipped.stack)
            .filter_map(|item| items.get(item.item))
            .map(|item| i32::from(item.resistance_bonus[realm]))
            .sum::<i32>()
            .clamp(-125, 125);
        Resistance { base, total: (base + equipment).min(100) }
    })
}

/// A party member's resistances, the party's protections included.
pub fn member_resistances(
    character: &crate::game::characters::CharacterStats<'_>,
    carried: &Carried,
    items: &ItemCatalog,
    party: &crate::game::magic::enchantments::Enchantments,
) -> EnumMap<Realm, Resistance> {
    let mut scores = resistances(character, carried, items);
    for (realm, value) in &mut scores {
        value.total = (value.total + party.resistance(realm)).min(100);
    }
    scores
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::{Gender, Profession};
    use crate::content::items::ItemDefinition;
    use crate::game::inventory::{Equipped, ItemIds, ItemStack};

    #[test]
    fn racial_and_worn_item_resistances_apply_without_alternate_weapons() {
        let elf = Character::new("Elf", Race::Elf, Gender::Female, Profession::Mage);
        let items = ItemCatalog(vec![ItemDefinition {
            resistance_bonus: enum_map! { Realm::Air => 12, Realm::Fire => -7, _ => 0 },
            ..Default::default()
        }]);
        let item = ItemStack::new(&items.0[0], &mut ItemIds::default(), 1, false);
        let mut carried = Carried::default();
        carried.equipment[EquipSlot::AlternatePrimary] = Some(Equipped::new(item.clone()));
        let base = resistances(&elf.base_stats(), &carried, &items);
        assert_eq!(base[Realm::Air], Resistance { base: 40, total: 40 });
        assert_eq!(base[Realm::Mental], Resistance { base: 50, total: 50 });
        carried.equipment[EquipSlot::Primary] = Some(Equipped::new(item));
        let worn = resistances(&elf.base_stats(), &carried, &items);
        assert_eq!(worn[Realm::Air], Resistance { base: 40, total: 52 });
        assert_eq!(worn[Realm::Fire], Resistance { base: 30, total: 23 });
    }
}
