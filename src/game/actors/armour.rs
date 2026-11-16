//! How well an actor is guarded (`docs/rules/arms.md`, "Armour").
//!
//! An actor's armour is a list of named parts that add up, as the character
//! screen lists them. Each part is written by what it comes from, when that
//! changes; [`Armour`] is what they come to, overall and at each place of
//! the body. A blow reads `Armour`, and the parts it gets past by name.
//!
//! A monster has the same thing with fewer parts: what its species gives it
//! overall and at each of its places, and what spells, conditions and
//! tiredness make of that.

use bevy::prelude::*;
use enum_map::{Enum, EnumMap};
use serde::Deserialize;

use super::modifiers::Modifiers;
use super::sources::{Abilities, Nature, Traits};
use super::{Attributes, Skills};
use crate::content::characters::{Attribute, BodyPart, Race, Skill};
use crate::content::items::AttackMode;
use crate::game::characters::{Calling, Condition, Conditions, Vitals, skill_available};
use crate::game::combat::Turn;
use crate::game::inventory::EquipSlot;
use crate::game::items::{Burden, Items};

/// A part of an actor's armour, as the character screen lists them.
#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmourPart {
    Race,
    Speed,
    Stealth,
    Shield,
    MagicItems,
    MagicSpells,
    VsPenetration,
    Encumbrance,
    Conditions,
    Fatigue,
    DefensiveAction,
    Reflextion,
}

/// The parts of an actor's armour. Each is written by what it comes from.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct ArmourParts(pub EnumMap<ArmourPart, i32>);

impl ArmourParts {
    /// The part is this now; nothing is touched if it already was.
    fn set(parts: &mut Mut<ArmourParts>, part: ArmourPart, value: i32) {
        if parts.0[part] != value {
            parts.0[part] = value;
        }
    }
}

/// A place of a body where a blow can land.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitPlace {
    /// The part of a character's body it is: what is worn there covers it,
    /// and an attacker with chances of its own for the parts lands by those.
    pub part: Option<BodyPart>,
    /// How often a blow lands here, in a hundred.
    pub chance: u32,
    /// What covers it, beyond the parts.
    pub cover: i32,
}

/// The places of a body where a blow can land.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct HitPlaces {
    pub places: Vec<HitPlace>,
    /// Where a blow lands that the chances do not place.
    pub trunk: usize,
}

/// What it has against each way of attacking.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Against(pub EnumMap<AttackMode, i32>);

/// A monster caught unaware is 2 worse guarded, and it blocks what misses
/// it by 10 or less (retail monster flag 0x02).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct VulnerableFromBehind;

/// How well an actor is guarded: the parts together. Higher is better
/// guarded, five off a blow for each point.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Armour {
    /// Against being struck at all.
    pub total: i32,
    /// Against being pierced where struck, besides the total.
    pub pierced: i32,
    /// Over its places, each weighed by how often a blow lands there.
    pub average: i32,
}

impl Armour {
    /// Where struck: the total, what it has against being pierced, and what
    /// covers the place.
    pub fn at(&self, place: &HitPlace) -> i32 {
        self.total + self.pierced + place.cover
    }
}

/// One row of `assets/tables/hit_places.ron`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BodyRow {
    part: BodyPart,
    chance: u32,
    slot: EquipSlot,
}

static BODY: std::sync::LazyLock<Vec<BodyRow>> = std::sync::LazyLock::new(|| {
    ron::from_str(include_str!("../../../assets/tables/hit_places.ron"))
        .expect("assets/tables/hit_places.ron is a table of a character's hit places")
});

/// The armour one born of a race is made with: what the race gives, and
/// the places of a character's body, bare.
pub fn born(race: Race) -> impl Bundle {
    let mut parts = ArmourParts::default();
    parts.0[ArmourPart::Race] = race.row().armour;
    let places = BODY
        .iter()
        .map(|row| HitPlace { part: Some(row.part), chance: row.chance, cover: 0 })
        .collect();
    let trunk = BODY.iter().position(|row| row.part == BodyPart::Body).unwrap_or(0);
    (parts, HitPlaces { places, trunk })
}

/// The armour one of a species is made with: its own, its places and what
/// it has against each way of attacking.
pub fn of_species(kind: &crate::content::species::Species) -> impl Bundle {
    let mut parts = ArmourParts::default();
    parts.0[ArmourPart::Race] = kind.armour;
    let places = kind
        .places
        .iter()
        .map(|place| HitPlace { part: None, chance: u32::from(place.chance), cover: place.cover })
        .collect();
    (parts, HitPlaces { places, trunk: kind.trunk }, Against(kind.against))
}

/// One who trained is guarded by their quickness: 1 from Speed 80 and 2
/// from 90; 1 worse under 20 and 2 worse under 10 (retail
/// `CalcArmorClasses`).
fn speed(mut actors: Query<(&Attributes, &mut ArmourParts), (Changed<Attributes>, With<Calling>)>) {
    for (attributes, mut parts) in &mut actors {
        let speed = attributes[Attribute::Speed];
        let part = i32::from(speed > 79) + i32::from(speed > 89)
            - i32::from(speed < 20)
            - i32::from(speed < 10);
        ArmourParts::set(&mut parts, ArmourPart::Speed, part);
    }
}

/// A tenth of Stealth, and a twentieth of Reflextion and one for one who
/// can use it (retail `CalcArmorClasses`).
#[allow(clippy::type_complexity)]
fn skills(
    mut actors: Query<
        (&Skills, &Calling, &Abilities, &mut ArmourParts),
        Or<(Changed<Skills>, Changed<Calling>, Changed<Abilities>)>,
    >,
) {
    for (skills, calling, abilities, mut parts) in &mut actors {
        ArmourParts::set(&mut parts, ArmourPart::Stealth, skills[Skill::Stealth] as i32 / 10);
        let reflextion = if skill_available(calling, abilities, Skill::Reflextion) {
            skills[Skill::Reflextion] as i32 / 20 + 1
        } else {
            0
        };
        ArmourParts::set(&mut parts, ArmourPart::Reflextion, reflextion);
    }
}

/// Where what guards the whole body is worn: the two accessory places, as
/// a cloak, and in the hands in use. The stored weapon set counts for
/// nothing.
const GUARDING: [EquipSlot; 5] = [
    EquipSlot::Misc1,
    EquipSlot::Misc2,
    EquipSlot::Cloak,
    EquipSlot::Primary,
    EquipSlot::Secondary,
];

/// What is worn: the shields' worth, the worth of the other things that
/// guard the whole body, and what covers each part of it. One alive and
/// about, whose shields are worth anything, adds a 25th of Shield skill up
/// to their worth; defending, a 15th up to one and a half times it (retail
/// `CalcArmorClasses`).
#[allow(clippy::type_complexity)]
fn worn(
    mut actors: Query<
        (Entity, &Skills, &Vitals, &Conditions, &mut ArmourParts, &mut HitPlaces),
        Or<(
            Changed<Modifiers>,
            Changed<Skills>,
            Changed<Vitals>,
            Changed<Conditions>,
            Changed<ArmourParts>,
        )>,
    >,
    items: Items,
) {
    for (actor, skills, vitals, conditions, mut parts, mut places) in &mut actors {
        let (mut shield, mut others) = (0, 0);
        for (protection, is_shield) in
            GUARDING.into_iter().filter_map(|slot| items.protection(actor, slot))
        {
            *(if is_shield { &mut shield } else { &mut others }) += protection.armour;
        }
        if shield > 0 && about(vitals, conditions) {
            let skill = skills[Skill::Shield] as i32;
            shield += if parts.0[ArmourPart::DefensiveAction] > 0 {
                (skill / 15).min(shield * 3 / 2)
            } else {
                (skill / 25).min(shield)
            };
        }
        ArmourParts::set(&mut parts, ArmourPart::Shield, shield);
        ArmourParts::set(&mut parts, ArmourPart::MagicItems, others);
        for place in 0..places.places.len() {
            let Some(part) = places.places[place].part else { continue };
            let cover = BODY
                .iter()
                .find(|row| row.part == part)
                .and_then(|row| items.protection(actor, row.slot))
                .map_or(0, |(protection, _)| protection.armour);
            if places.places[place].cover != cover {
                places.places[place].cover = cover;
            }
        }
    }
}

/// What the spells laid on it, and on its party, add: against being struck
/// at all, and against being pierced where struck.
fn spells(mut actors: Query<(&Modifiers, &mut ArmourParts), Changed<Modifiers>>) {
    for (modifiers, mut parts) in &mut actors {
        ArmourParts::set(&mut parts, ArmourPart::MagicSpells, modifiers.laid.armour);
        ArmourParts::set(&mut parts, ArmourPart::VsPenetration, modifiers.laid.penetration);
    }
}

/// What ails it takes from its armour.
fn conditions(mut actors: Query<(&Conditions, &mut ArmourParts), Changed<Conditions>>) {
    for (conditions, mut parts) in &mut actors {
        ArmourParts::set(&mut parts, ArmourPart::Conditions, conditions.armour());
    }
}

/// A medium, heavy or extreme load costs 1, 2 or 4.
fn load(mut actors: Query<(&Burden, &mut ArmourParts), Changed<Burden>>) {
    for (burden, mut parts) in &mut actors {
        ArmourParts::set(&mut parts, ArmourPart::Encumbrance, -burden.load.armour_penalty());
    }
}

/// A tenth of what tiredness takes from an aim: 0, 0, 1, 2 or 4 by the
/// share of stamina spent. It is lost, but for one whose tiredness guards
/// it, who gains it (retail `CalcArmorClasses` takes it from a character;
/// `GetTargetArmorClassModifier` adds it to a monster).
fn fatigue(mut actors: Query<(&Vitals, &Nature, &mut ArmourParts), Changed<Vitals>>) {
    for (vitals, nature, mut parts) in &mut actors {
        let tenth = crate::game::combat::attack::fatigue(vitals) / 10;
        let part = if nature.fatigue_guards { tenth } else { -tenth };
        ArmourParts::set(&mut parts, ArmourPart::Fatigue, part);
    }
}

/// One that holds back on guard this round (a member defending, a monster
/// that waits) is 2 better guarded (retail `CalcArmorClasses`,
/// `GetTargetArmorClassModifier`).
fn braced(mut actors: Query<(&Turn, &mut ArmourParts), Changed<Turn>>) {
    for (turn, mut parts) in &mut actors {
        let part = if turn.waited { 2 } else { 0 };
        ArmourParts::set(&mut parts, ArmourPart::DefensiveAction, part);
    }
}

/// The fight is over, and what holding back added with it.
fn fight_over(event: On<Remove<Turn>>, mut actors: Query<&mut ArmourParts>) {
    if let Ok(mut parts) = actors.get_mut(event.entity) {
        ArmourParts::set(&mut parts, ArmourPart::DefensiveAction, 0);
    }
}

/// Alive and not taken away: no worse than unconscious.
fn about(vitals: &Vitals, conditions: &Conditions) -> bool {
    !vitals.is_dead() && conditions.below(Condition::Missing)
}

/// The parts worn, which still guard one who is dead or away.
const WORN: [ArmourPart; 2] = [ArmourPart::Shield, ArmourPart::MagicItems];

/// What the parts come to (retail `CalcArmorClasses`). Overall: every part
/// but what it has against being pierced; one kept from its place in the
/// formation is guarded no better than -5. At a place: that, what it has
/// against being pierced, and what covers the place. Only what is worn
/// guards one who is dead or away. The average weighs each place by how
/// often a blow lands there, rounded to the nearest.
#[allow(clippy::type_complexity)]
fn armour(
    mut actors: Query<
        (&ArmourParts, &HitPlaces, &Vitals, &Conditions, &Traits, &mut Armour),
        Or<(
            Changed<ArmourParts>,
            Changed<HitPlaces>,
            Changed<Vitals>,
            Changed<Conditions>,
            Changed<Traits>,
        )>,
    >,
) {
    for (parts, places, vitals, conditions, traits, mut written) in &mut actors {
        let about = about(vitals, conditions);
        let counted = |part: ArmourPart| about || WORN.contains(&part);
        let sum: i32 = parts
            .0
            .iter()
            .filter(|&(part, _)| part != ArmourPart::VsPenetration && counted(part))
            .map(|(_, value)| value)
            .sum();
        let total = if conditions.unseats(traits.0) { sum.min(-5) } else { sum };
        let pierced = if about { parts.0[ArmourPart::VsPenetration] } else { 0 };
        let mut armour = Armour { total, pierced, average: 0 };
        let weighed: i32 =
            places.places.iter().map(|place| place.chance as i32 * armour.at(place)).sum();
        armour.average = if weighed < 0 { (weighed - 50) / 100 } else { (weighed + 50) / 100 };
        written.set_if_neq(armour);
    }
}

pub(super) fn plugin(app: &mut App) {
    // The parts, then what they come to, once the figures they are made
    // from are written.
    app.add_systems(
        super::Settle,
        ((speed, skills, spells, conditions, load, fatigue, braced), worn, armour)
            .chain()
            .in_set(super::Written)
            .after(super::figures::skills)
            .after(super::figures::maxima)
            .before(super::figures::attacks),
    )
    .add_observer(fight_over);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::{Gender, Profession};
    use crate::content::items::{EquipClass, ItemDefinition};
    use crate::game::actors::Settle;
    use crate::game::characters::{Character, Lasting};
    use crate::game::inventory::WeaponSlot;
    use crate::game::items::{Worn, make};
    use bevy::ecs::system::RunSystemOnce;

    /// A world that settles bodies, and a Mage in it.
    fn mage() -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(crate::game::actors::plugin).init_resource::<crate::game::rng::GameRng>();
        let character = Character::new("A", Race::Human, Gender::Male, Profession::Mage);
        let vitals = Vitals::new(&character);
        let member = app.world_mut().spawn((born(Race::Human), character, vitals)).id();
        (app, member)
    }

    fn wear(app: &mut App, member: Entity, slot: EquipSlot, class: EquipClass, armour: i32) {
        let definition =
            ItemDefinition { class, armor_class_bonus: armour, ..ItemDefinition::default() };
        let world = app.world_mut();
        let item = world
            .run_system_once(move |mut commands: Commands| {
                make(&mut commands, &definition, 1, true)
            })
            .unwrap();
        world.flush();
        world.entity_mut(item).insert(Worn { by: member, slot });
    }

    fn settled(app: &mut App, member: Entity) -> (ArmourParts, Armour) {
        app.world_mut().run_schedule(Settle);
        let world = app.world();
        (world.get::<ArmourParts>(member).unwrap().clone(), *world.get::<Armour>(member).unwrap())
    }

    /// Retail `CalcArmorClasses`: a shield's worth and a 25th of the skill
    /// up to it; defending adds 2 and lets a 15th count, up to one and a
    /// half times the shield.
    #[test]
    fn defending_adds_two_and_lets_shield_skill_count_for_more() {
        let (mut app, member) = mage();
        wear(&mut app, member, EquipSlot::Secondary, EquipClass::Shield, 4);
        let skilled = |app: &mut App, points, defending| {
            let world = app.world_mut();
            world.get_mut::<crate::game::characters::Training>(member).unwrap().0[Skill::Shield] =
                points;
            world.entity_mut(member).insert(Turn {
                when: None,
                attacks: EnumMap::default(),
                due: WeaponSlot::Primary,
                weapons: EnumMap::default(),
                lightning: None,
                moved: false,
                special: false,
                berserk: false,
                waited: defending,
            });
            let (parts, armour) = settled(app, member);
            (parts.0[ArmourPart::Shield], parts.0[ArmourPart::DefensiveAction], armour.total)
        };
        let bare = skilled(&mut app, 0, false).2 - 4;
        assert_eq!(skilled(&mut app, 60, false), (4 + 2, 0, bare + 6));
        assert_eq!(skilled(&mut app, 60, true), (4 + 4, 2, bare + 10));
        assert_eq!(skilled(&mut app, 125, false), (4 + 4, 0, bare + 8));
        assert_eq!(skilled(&mut app, 125, true), (4 + 6, 2, bare + 12));
    }

    /// What is worn at a place covers only that place; what guards against
    /// piercing counts only where a blow lands; one kept from their place is
    /// guarded no better than -5 overall; only what is worn guards the dead.
    #[test]
    fn the_parts_come_to_a_total_and_a_figure_at_each_place() {
        let (mut app, member) = mage();
        wear(&mut app, member, EquipSlot::Head, EquipClass::Head, 3);
        wear(&mut app, member, EquipSlot::Cloak, EquipClass::Cloak, 1);
        app.world_mut().get_mut::<Modifiers>(member).unwrap().laid.penetration = 2;
        let (parts, armour) = settled(&mut app, member);
        assert_eq!(parts.0[ArmourPart::MagicItems], 1);
        let total = armour.total;
        let at = |app: &App, armour: &Armour| -> Vec<i32> {
            let places = app.world().get::<HitPlaces>(member).unwrap();
            places.places.iter().map(|place| armour.at(place)).collect()
        };
        // Head, body, legs, hands, feet.
        assert_eq!(at(&app, &armour), [total + 5, total + 2, total + 2, total + 2, total + 2]);
        // 15 in a hundred land on the head: 3 x 15 = 45 of 100, under a half.
        assert_eq!(armour.average, total + 2);

        let asleep = (Condition::Asleep, Lasting::Rounds(3));
        app.world_mut().get_mut::<Conditions>(member).unwrap().afflict(asleep.0, asleep.1);
        let (_, armour) = settled(&mut app, member);
        assert_eq!((armour.total, at(&app, &armour)[0]), (-5, -5 + 5));

        app.world_mut().get_mut::<Vitals>(member).unwrap().slay(None);
        let (_, armour) = settled(&mut app, member);
        let dead = at(&app, &armour);
        assert_eq!((armour.total, dead[0], dead[1]), (1, 4, 1));
    }
}
