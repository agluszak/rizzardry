//! What a member's armour and the spells on them stop, by body part.
use super::*;

/// The equipment slot whose armour covers a character's hit location.
pub(super) fn armour_slot(part: BodyPart) -> EquipSlot {
    match part {
        BodyPart::Head => EquipSlot::Head,
        BodyPart::Body => EquipSlot::Torso,
        BodyPart::Leg => EquipSlot::Legs,
        BodyPart::Hand => EquipSlot::Hands,
        BodyPart::Foot => EquipSlot::Feet,
    }
}

/// Where a blow lands, by cumulative percentages against one `below(100)`
/// draw; anything past them hits `fallback`.
pub(super) fn location<L: Copy>(
    rng: &mut GameRng,
    chances: impl IntoIterator<Item = (L, u8)>,
    fallback: L,
) -> L {
    let roll = rng.below(100);
    let mut total = 0;
    for (location, chance) in chances {
        total += u32::from(chance);
        if roll < total {
            return location;
        }
    }
    fallback
}

/// A member's total and per-location armour, without skills or conditions.
/// Calculated from current inputs, never stored on the member.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Armour {
    pub total: i32,
    pub by_location: EnumMap<BodyPart, i32>,
    /// What speed adds, which a blow from behind gets past.
    pub speed: i32,
    /// What a shield and the skill to use it add, likewise.
    pub shield: i32,
}

impl Armour {
    /// Hit-location-weighted average displayed on the character portrait.
    pub fn average(&self) -> i32 {
        let weight = |part| match part {
            BodyPart::Head => 15,
            BodyPart::Body => 40,
            BodyPart::Leg => 30,
            BodyPart::Hand => 10,
            BodyPart::Foot => 5,
        };
        let total: i32 = self.by_location.iter().map(|(part, value)| value * weight(part)).sum();
        (total as f32 / 100.0).round() as i32
    }
}

/// A member's armour with the spells on them and, for a party member, the
/// party's protections.
pub fn enchanted_armour(
    c: &crate::game::characters::CharacterStats<'_>,
    carried: &Carried,
    catalog: &ItemCatalog,
    load: Load,
    defending: bool,
    (personal, party): (
        &crate::game::magic::enchantments::Enchantments,
        Option<&crate::game::magic::enchantments::Enchantments>,
    ),
) -> Armour {
    let mut armour = armour(c, carried, catalog, load, defending);
    let bonus = personal.armour() + party.map_or(0, |party| party.armour());
    armour.total += bonus;
    for (_, value) in armour.by_location.iter_mut() {
        *value += bonus;
    }
    armour
}

/// A character's armour (retail `CalcArmorClasses`). Defending adds 2
/// everywhere and lets Shield skill count for more.
pub fn armour(
    c: &crate::game::characters::CharacterStats<'_>,
    carried: &Carried,
    catalog: &ItemCatalog,
    load: Load,
    defending: bool,
) -> Armour {
    let definition = |slot| carried.worn(slot).map(|i| &catalog[i.item]);
    let speed = c.attribute(Attribute::Speed);
    let speed = i32::from(speed > 79) + i32::from(speed > 89)
        - i32::from(speed < 20)
        - i32::from(speed < 10);
    let mut total = i32::from(c.race == Race::Faerie) * 2 + speed - load.armour_penalty()
        + if defending { 2 } else { 0 };
    let mut shield = 0;
    // Shields and magic items worn in the misc, cloak and hand slots.
    for slot in [
        EquipSlot::Misc1,
        EquipSlot::Misc2,
        EquipSlot::Cloak,
        EquipSlot::Primary,
        EquipSlot::Secondary,
    ] {
        if let Some(d) = definition(slot) {
            total += i32::from(d.armor_class_bonus);
            if d.class == EquipClass::Shield {
                let bonus = i32::from(d.armor_class_bonus);
                let skill = c.skill(Skill::Shield) as i32;
                let skill = if defending {
                    (skill / 15).min(bonus * 3 / 2)
                } else {
                    (skill / 25).min(bonus)
                };
                total += skill;
                shield += bonus + skill;
            }
        }
    }
    let by_location = EnumMap::from_fn(|part| {
        let worn = definition(armour_slot(part));
        total + worn.map_or(0, |d: &ItemDefinition| i32::from(d.armor_class_bonus))
    });
    Armour { total, by_location, speed, shield }
}
