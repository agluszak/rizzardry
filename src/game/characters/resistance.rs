//! Resistance from a character's profile and currently worn equipment.
use crate::content::characters::{Attribute, Realm, Resist, Skill};
#[cfg(test)]
use crate::game::characters::Character;
use crate::game::inventory::EquipSlot;
use crate::game::items::Protection;
use enum_map::EnumMap;
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
    let vitality = a[Attribute::Vitality] as i32;
    let racial = |realm: Realm| match character.race.row().resistance[realm] {
        None => 0,
        Some(Resist::Flat(amount)) => amount,
        Some(Resist::OfVitality(divisor)) => vitality / divisor,
    };
    EnumMap::from_fn(|realm: Realm| {
        let skill = realm.skill();
        let iron_will = if character.skill_available(Skill::IronWill) {
            character.skill(Skill::IronWill) as i32 / 5 + 5
        } else {
            0
        };
        let mental = if realm == Realm::Mental {
            (a[Attribute::Intelligence] as i32 - 80).max(0) / 2
        } else {
            0
        };
        let divine =
            if realm == Realm::Divine { (a[Attribute::Piety] as i32 - 80).max(0) / 2 } else { 0 };
        let base = 25
            + character.skill(skill) as i32 / 10
            + iron_will
            + character.profession.row().resistance
            + racial(realm)
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
    use crate::content::characters::{Gender, Profession, Race};
    use enum_map::enum_map;
    /// A Dwarf resists fire by a fifth of vitality, and a Mage every realm by
    /// five more.
    #[test]
    fn a_dwarfs_fire_follows_vitality_and_a_mages_every_realm_is_five_higher() {
        let dwarf = Character::new("D", Race::Dwarf, Gender::Male, Profession::Fighter);
        let sheet = crate::game::actors::Attributes::of_sheet(&dwarf);
        let none = |_| None;
        // 25, and vitality 60 / 5 = 12.
        assert_eq!(resistances(&dwarf, &sheet, none)[Realm::Fire].base, 37);
        assert_eq!(resistances(&dwarf, &sheet, none)[Realm::Water].base, 25);
        let mage = Character::new("M", Race::Human, Gender::Male, Profession::Mage);
        let sheet = crate::game::actors::Attributes::of_sheet(&mage);
        assert_eq!(resistances(&mage, &sheet, none)[Realm::Water].base, 30);
    }

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
