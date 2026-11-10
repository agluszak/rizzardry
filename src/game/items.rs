//! Items (`docs/rules/items.md`).
//!
//! An item is an entity. What kind of thing it is was read from its
//! definition as it was made and is its components; where it is, is a
//! relationship to what holds it. Moving an item changes that relationship
//! and nothing else, so an item is never in two places or none.

use bevy::ecs::query::QueryData;
use bevy::ecs::relationship::RelationshipTarget;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use enum_map::EnumMap;
use enumset::EnumSet;

use crate::content::characters::{Attribute, Gender, Profession, Race, Realm, Skill};
use crate::content::items::{
    AttackEffects, AttackMode, Dice, EquipClass, ItemCatalog, ItemDefinition, ItemId, ItemKind,
    QuantityKind, Recipe,
};
use crate::content::species::{MonsterKind, Reach};
use crate::game::actors::{Attributes, Refit};
use crate::game::characters::{Character, Practise, Vitals};
use crate::game::combat::Battle;
use crate::game::data::Catalogs;
use crate::game::inventory::EquipSlot;
use crate::game::party::roster::{PartyMember, PartyRoster};
use crate::game::rng::GameRng;
use crate::game::world::level::LevelOwned;

/// How many things a member's pack holds.
pub const PACK_SLOTS: usize = 8;
/// How many the party's pool holds.
pub const POOL_LIMIT: usize = 500;

/// An item, and the definition it was made from, which it is named and
/// shown by.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[require(Known)]
pub struct Item(pub ItemId);

impl Users {
    /// Whether a character may wear or wield what these allow (retail
    /// `CanCharacterUseItem`): profession, race and gender, and every
    /// attribute and skill minimum.
    pub fn admit(&self, character: &Character) -> bool {
        self.professions.contains(character.profession)
            && self.races.contains(character.race)
            && self.genders.contains(character.gender)
            && self
                .attributes
                .iter()
                .all(|(a, &least)| character.attributes[a].clamp(1, 125) >= u32::from(least))
            && self.skills.iter().all(|(s, &least)| character.skill(s) >= u32::from(least))
    }
}

/// What kind of thing it is, which a few rules single out: what loads a
/// gun, what a launcher shoots.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kind(pub ItemKind);

/// How many alike this one entity stands for, and the most that go
/// together. Only what stacks has a count; anything else is one.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Count {
    pub of: u16,
    pub limit: u16,
}

/// Uses or shots left, and the most it holds.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Charges {
    pub left: u16,
    pub full: u16,
}

/// It stays when its last charge is spent: it is still a weapon, armour
/// or a curse.
#[derive(Component, Clone, Debug)]
pub struct KeepsEmpty;

/// One is used up each time it is used: a stack's, or a drink.
#[derive(Component, Clone, Debug)]
pub struct SpentByUse;

/// What the party knows of an item.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Known {
    pub identified: bool,
    pub spell: bool,
    pub curse: bool,
}

/// What one of it weighs, in tenths of a pound.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Weight(pub u16);

/// The party may not part with it.
#[derive(Component, Clone, Debug)]
pub struct Crucial;

/// Worn in use, it cannot be taken off until its curse is released.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursed {
    pub released: bool,
}

/// Who may wear or wield it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Users {
    pub professions: EnumSet<Profession>,
    pub races: EnumSet<Race>,
    pub genders: EnumSet<Gender>,
    pub attributes: EnumMap<Attribute, u8>,
    pub skills: EnumMap<Skill, u8>,
}

/// It can be worn or wielded: where, and what it rules out beside it.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct Wearable {
    pub class: EquipClass,
    pub two_handed: bool,
    /// May be wielded in the off hand.
    pub off_hand: bool,
    pub reach: Reach,
    /// It is fought with by a skill; a weapon without one fits no hand.
    pub skilled: bool,
    pub users: Users,
}

/// How much a wielder's strength helps a weapon's blows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strength {
    Full,
    Half,
    /// Guns and the like.
    None,
}

/// It strikes: what a hand holding it fights with.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct Weapon {
    /// The skill it is fought with.
    pub skill: Skill,
    /// Its blows are ranged combat, not close combat.
    pub ranged: bool,
    pub reach: Reach,
    pub dice: Dice,
    /// The ways it strikes.
    pub modes: EnumSet<AttackMode>,
    /// What its blows may do besides damage.
    pub effects: AttackEffects,
    /// The monster kind it slays for an extra die.
    pub slays: Option<MonsterKind>,
    /// What it adds to the rating to hit.
    pub to_hit: i32,
    /// What it adds to initiative.
    pub initiative: i32,
    /// Swings it adds to each attack.
    pub extra_swings: i32,
    pub strength: Strength,
    /// An attack with it is one swing, whatever the wielder's speed.
    pub single_swing: bool,
    /// A sling, which the kill chance of a thrown weapon and of a shot weigh
    /// differently.
    pub sling: bool,
    /// Using it casts its spell, which has its own charges to spend.
    pub casts: bool,
    /// It counts shots, and with none left it cannot strike.
    pub shots: bool,
}

/// It shoots what the other hand holds: what it adds to the damage of that,
/// in percent.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Launcher {
    pub damage: i32,
}

/// It is shot from a launcher or thrown: what it adds to the launcher's
/// dice, effects, slaying and rating, and whether it survives the shot.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct Missile {
    pub dice: Dice,
    pub effects: AttackEffects,
    pub slays: Option<MonsterKind>,
    pub to_hit: i32,
    pub modes: EnumSet<AttackMode>,
    /// It is not used up when thrown.
    pub kept: bool,
}

/// It guards rather than strikes.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shield;

/// What wearing it in use gives: armour, resistance by realm, and health,
/// stamina and spell points given back.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Protection {
    pub armour: i32,
    pub resistance: EnumMap<Realm, i8>,
    pub regeneration: [i8; 3],
}

/// Where a member wears an item.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[relationship(relationship_target = Wears)]
pub struct Worn {
    #[relationship]
    pub by: Entity,
    pub slot: EquipSlot,
}

/// What a member wears.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = Worn, linked_spawn)]
pub struct Wears(Vec<Entity>);

/// Where in a member's pack an item is.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[relationship(relationship_target = Pack)]
pub struct Packed {
    #[relationship]
    pub by: Entity,
    pub slot: usize,
}

/// What is in a member's pack.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = Packed, linked_spawn)]
pub struct Pack(Vec<Entity>);

/// An item in the party's pool.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[relationship(relationship_target = Pool)]
pub struct Pooled(pub Entity);

/// The party's pool, in the order it is listed.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = Pooled, linked_spawn)]
pub struct Pool(Vec<Entity>);

/// The item the party has in hand.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[relationship(relationship_target = Hand)]
pub struct InHand(pub Entity);

/// What the party has in hand: one thing at most.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = InHand, linked_spawn)]
pub struct Hand(Vec<Entity>);

/// An item inside a container in the world.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[relationship(relationship_target = Stows)]
pub struct Stowed(pub Entity);

/// What a container holds.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = Stowed, linked_spawn)]
pub struct Stows(Vec<Entity>);

/// An item lying in the world, where anyone can take it. Its position is its
/// `Transform`, and it belongs to the level it lies in.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lying;

/// A lying item the party has not found yet: it cannot be seen, pointed at or
/// taken until a search spots it.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hidden;

/// It holds other things, and taking it takes what is in it instead (retail
/// `W8Item::container`): a bag lying in the world.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bag;

/// Somewhere an item can be put.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Place {
    Slot {
        member: Entity,
        slot: EquipSlot,
    },
    Pack {
        member: Entity,
        slot: usize,
    },
    /// In a party's pool, at a row counted from its top.
    Pool {
        party: Entity,
        row: usize,
    },
    Hand {
        party: Entity,
    },
    Container(Entity),
    /// Lying in a level at a point.
    Ground {
        level: Entity,
        at: Vec3,
    },
}

/// Make an item from its definition: `amount` of it, or that many charges,
/// as its kind counts them. A count of nothing means one (retail's legacy
/// zero counts); no charges is a valid state. It is nowhere until it is
/// [`Put`] somewhere.
pub fn make(
    commands: &mut Commands,
    definition: &ItemDefinition,
    amount: u16,
    identified: bool,
) -> Entity {
    let counted = definition.quantity == QuantityKind::Stack;
    let mut item = commands.spawn((
        Item(definition.id),
        Known { identified: identified || definition.auto_identify, ..default() },
        Weight(definition.weight),
        Kind(definition.kind),
    ));
    if counted {
        item.insert(Count { of: amount.max(1), limit: u16::from(definition.stack_limit) });
    } else if definition.quantity != QuantityKind::None {
        let full = definition.initial_quantity.maximum() as u16;
        item.insert(Charges { left: amount, full });
    }
    if counted || definition.class == EquipClass::Drink {
        item.insert(SpentByUse);
    }
    if definition.keeps_empty() {
        item.insert(KeepsEmpty);
    }
    if definition.crucial {
        item.insert(Crucial);
    }
    if definition.container {
        item.insert(Bag);
    }
    if definition.cursed {
        item.insert(Cursed { released: false });
    }
    shape(&mut item, definition);
    item.id()
}

/// What an item of this definition is to a hand, a body and a wearer: the
/// components a rule reads in its place. Written as it is made, and again
/// when it grows into another definition.
pub fn shape(item: &mut EntityCommands, definition: &ItemDefinition) {
    if let Some(wearable) = wearable(definition) {
        item.insert(wearable);
    }
    if let Some(weapon) = weapon(definition) {
        item.insert(weapon);
    }
    // The guns and bows that shoot what the other hand holds (retail
    // `ItemHasSingledOutGenericName`).
    if matches!(
        definition.kind,
        ItemKind::LONGBOW
            | ItemKind::CROSSBOW
            | ItemKind::SLING
            | ItemKind::GREATBOW
            | ItemKind::OMNIGUN
    ) {
        item.insert(Launcher { damage: definition.damage.base * 10 });
    }
    if matches!(definition.class, EquipClass::Ammunition | EquipClass::ThrownWeapon) {
        item.insert(Missile {
            dice: definition.damage,
            effects: definition.effects,
            slays: definition.slays,
            to_hit: definition.hit_bonus,
            modes: definition.attack_modes,
            kept: definition.never_depletes,
        });
    }
    if definition.class == EquipClass::Shield {
        item.insert(Shield);
    }
    let protection = Protection {
        armour: definition.armor_class_bonus,
        resistance: definition.resistance_bonus,
        regeneration: definition.regeneration,
    };
    if protection.armour != 0
        || protection.resistance.values().any(|&value| value != 0)
        || protection.regeneration.iter().any(|&value| value != 0)
    {
        item.insert(protection);
    }
}

/// What strikes, if an item of this definition does: a weapon fought with a
/// close or ranged combat skill.
fn weapon(definition: &ItemDefinition) -> Option<Weapon> {
    use EquipClass::*;
    if !matches!(definition.class, ShortWeapon | ExtendedWeapon | RangedWeapon | ThrownWeapon) {
        return None;
    }
    let skill = definition.weapon_skill?;
    let ranged = if skill as usize <= Skill::Shield as usize || skill == Skill::MartialArts {
        false
    } else if matches!(skill, Skill::ModernWeapon | Skill::Bow | Skill::ThrowingAndSling) {
        true
    } else {
        return None;
    };
    // Strength helps what is swung, by half what is only thrown, lashed or
    // shot, and not at all the guns among those.
    let strength = if !definition
        .attack_modes
        .is_subset(AttackMode::Throw | AttackMode::Lash | AttackMode::Shoot)
    {
        Strength::Full
    } else if matches!(
        definition.kind,
        ItemKind::POWER_GLOVE
            | ItemKind::PHASER
            | ItemKind::MUSKET
            | ItemKind::OMNIGUN
            | ItemKind::ROCKET_LAUNCHER
    ) {
        Strength::None
    } else {
        Strength::Half
    };
    Some(Weapon {
        skill,
        ranged,
        reach: definition.reach,
        dice: definition.damage,
        modes: definition.attack_modes,
        effects: definition.effects,
        slays: definition.slays,
        to_hit: definition.hit_bonus,
        initiative: definition.damage_bonus,
        extra_swings: definition.extra_swings,
        strength,
        single_swing: definition.kind == ItemKind::ROCKET_LAUNCHER,
        sling: definition.kind == ItemKind::SLING,
        casts: definition.spell.is_some(),
        shots: definition.quantity == QuantityKind::Shots,
    })
}

/// Where an item of this definition may be worn, if it is worn at all.
fn wearable(definition: &ItemDefinition) -> Option<Wearable> {
    definition.requires_worn().then(|| Wearable {
        class: definition.class,
        two_handed: definition.two_handed,
        off_hand: definition.off_hand,
        reach: definition.reach,
        skilled: definition.weapon_skill.is_some(),
        users: Users {
            professions: definition.professions,
            races: definition.races,
            genders: definition.genders,
            attributes: definition.attribute_minimums,
            skills: definition.skill_minimums,
        },
    })
}

/// How many an item of this definition is made with when it is found or
/// made: its initial quantity rolled, or at most.
pub fn roll_amount(definition: &ItemDefinition, rng: &mut GameRng, maximum: bool) -> u16 {
    (if definition.quantity == QuantityKind::None {
        1
    } else if maximum {
        definition.initial_quantity.maximum()
    } else {
        rng.roll(definition.initial_quantity)
    }) as u16
}

/// The items there are and where they are, to read.
#[derive(SystemParam)]
pub struct Items<'w, 's> {
    items: Query<
        'w,
        's,
        (
            &'static Item,
            Option<&'static Count>,
            Option<&'static Charges>,
            &'static Known,
            &'static Weight,
        ),
    >,
    worn: Query<'w, 's, &'static Worn>,
    packed: Query<'w, 's, &'static Packed>,
    pooled: Query<'w, 's, &'static Pooled>,
    in_hand: Query<'w, 's, &'static InHand>,
    wears: Query<'w, 's, &'static Wears>,
    packs: Query<'w, 's, &'static Pack>,
    pools: Query<'w, 's, &'static Pool>,
    hands: Query<'w, 's, &'static Hand>,
    stows: Query<'w, 's, &'static Stows>,
    hidden: Query<'w, 's, (), With<Hidden>>,
    protections: Query<'w, 's, (&'static Protection, Has<Shield>)>,
}

impl Items<'_, '_> {
    /// What a member wears in a slot.
    pub fn worn(&self, member: Entity, slot: EquipSlot) -> Option<Entity> {
        let wears = self.wears.get(member).ok()?;
        wears.iter().find(|&item| self.worn.get(item).is_ok_and(|worn| worn.slot == slot))
    }

    /// What protects, worn in a slot, and whether it is a shield.
    pub fn protection(&self, member: Entity, slot: EquipSlot) -> Option<(&Protection, bool)> {
        self.protections.get(self.worn(member, slot)?).ok()
    }

    /// Everything a member wears, with its slot.
    pub fn wearing(&self, member: Entity) -> impl Iterator<Item = (EquipSlot, Entity)> + '_ {
        let wears = self.wears.get(member).into_iter().flat_map(Wears::iter);
        wears.filter_map(|item| Some((self.worn.get(item).ok()?.slot, item)))
    }

    /// What a member wears in the slots in use (the stored weapon set is
    /// not worn).
    pub fn in_use(&self, member: Entity) -> impl Iterator<Item = (EquipSlot, Entity)> + '_ {
        self.wearing(member).filter(|(slot, _)| slot.is_active())
    }

    /// A member's pack, slot by slot.
    pub fn pack(&self, member: Entity) -> [Option<Entity>; PACK_SLOTS] {
        let mut slots = [None; PACK_SLOTS];
        for item in self.packs.get(member).into_iter().flat_map(Pack::iter) {
            if let Ok(packed) = self.packed.get(item) {
                slots[packed.slot] = Some(item);
            }
        }
        slots
    }

    /// Everything a member carries: what they wear, then their pack in order.
    pub fn carried(&self, member: Entity) -> impl Iterator<Item = Entity> + '_ {
        let worn = self.wearing(member).map(|(_, item)| item);
        worn.chain(self.pack(member).into_iter().flatten())
    }

    /// A party's pool, top row first.
    pub fn pool(&self, party: Entity) -> &[Entity] {
        self.pools.get(party).map_or(&[], |pool| pool.0.as_slice())
    }

    /// What a container, or a bag lying in the world, holds.
    pub fn stowed(&self, container: Entity) -> &[Entity] {
        self.stows.get(container).map_or(&[], |stows| stows.0.as_slice())
    }

    /// Whether a lying item has not been found yet.
    pub fn hidden(&self, item: Entity) -> bool {
        self.hidden.contains(item)
    }

    /// What a party has in hand.
    pub fn held(&self, party: Entity) -> Option<Entity> {
        self.hands.get(party).ok()?.0.first().copied()
    }

    /// Where an item is, among what the party has: a slot, a pack slot, a
    /// row of the pool or the hand.
    pub fn place(&self, item: Entity) -> Option<Place> {
        if let Ok(&Worn { by, slot }) = self.worn.get(item) {
            return Some(Place::Slot { member: by, slot });
        }
        if let Ok(&Packed { by, slot }) = self.packed.get(item) {
            return Some(Place::Pack { member: by, slot });
        }
        let Ok(&Pooled(party)) = self.pooled.get(item) else {
            let &InHand(party) = self.in_hand.get(item).ok()?;
            return Some(Place::Hand { party });
        };
        let row = self.pool(party).iter().position(|&row| row == item)?;
        Some(Place::Pool { party, row })
    }

    /// Whom an item is worn or packed by.
    pub fn bearer(&self, item: Entity) -> Option<Entity> {
        match self.place(item)? {
            Place::Slot { member, .. } | Place::Pack { member, .. } => Some(member),
            _ => None,
        }
    }

    /// The slot an item is worn in.
    pub fn slot(&self, item: Entity) -> Option<EquipSlot> {
        self.worn.get(item).ok().map(|worn| worn.slot)
    }

    /// Whether an item is in a party's pool.
    pub fn pooled(&self, item: Entity) -> bool {
        self.pooled.contains(item)
    }

    /// Which definition an item was made from.
    pub fn id(&self, item: Entity) -> Option<ItemId> {
        self.items.get(item).ok().map(|(&Item(id), ..)| id)
    }

    /// What the party knows of an item.
    pub fn known(&self, item: Entity) -> Option<Known> {
        self.items.get(item).ok().map(|(.., known, _)| *known)
    }

    /// How many an item stands for, if it stacks.
    pub fn count(&self, item: Entity) -> Option<u16> {
        self.items.get(item).ok()?.1.map(|count| count.of)
    }

    /// The uses or shots an item has left, if it counts them.
    pub fn charges(&self, item: Entity) -> Option<u16> {
        self.items.get(item).ok()?.2.map(|charges| charges.left)
    }

    /// How many an item stands for.
    pub fn units(&self, item: Entity) -> u32 {
        self.items
            .get(item)
            .map_or(0, |(_, count, ..)| count.map_or(1, |count| u32::from(count.of)))
    }

    /// What an item weighs, all of it.
    pub fn weight(&self, item: Entity) -> u32 {
        self.items.get(item).map_or(0, |(.., weight)| u32::from(weight.0)) * self.units(item)
    }

    /// What an item is called, as far as the party knows it.
    pub fn name(&self, item: Entity, catalog: &ItemCatalog, count: bool) -> Option<String> {
        let (&Item(id), count_of, _, known, _) = self.items.get(item).ok()?;
        let name = catalog[id].known_name(known.identified);
        Some(match count_of {
            Some(Count { of, .. }) if count && *of > 1 => format!("{name} ({of})"),
            _ => name.to_owned(),
        })
    }

    /// Whether two are stacks of the same thing, which meet by joining (if
    /// the party knows them alike) and never by changing places.
    pub fn alike(&self, a: Entity, b: Entity) -> bool {
        let (Ok((item, Some(_), ..)), Ok((other, Some(_), ..))) =
            (self.items.get(a), self.items.get(b))
        else {
            return false;
        };
        a != b && item == other
    }

    /// How many of `source` could join `target`: they are alike (made from
    /// the same definition, known the same) and stack, up to the limit.
    pub fn room(&self, target: Entity, source: Entity) -> u16 {
        let (Ok((item, Some(count), _, known, _)), Ok((other, Some(_), _, theirs, _))) =
            (self.items.get(target), self.items.get(source))
        else {
            return 0;
        };
        if target == source || item != other || known.identified != theirs.identified {
            return 0;
        }
        count.limit.max(count.of) - count.of
    }

    /// How many units of an item of this definition, known or not, the
    /// stacks among `holding` have room for.
    pub fn room_among(
        &self,
        holding: impl IntoIterator<Item = Entity>,
        id: ItemId,
        identified: bool,
    ) -> u32 {
        holding
            .into_iter()
            .filter_map(|stack| self.items.get(stack).ok())
            .filter(|(item, count, _, known, _)| {
                item.0 == id && count.is_some() && known.identified == identified
            })
            .filter_map(|(_, count, ..)| count)
            .map(|count| u32::from(count.limit.max(count.of) - count.of))
            .sum()
    }

    /// How many units of this definition a party's pool could take: what its
    /// alike rows have room for, and a row's worth (`per_row`) for each row
    /// still free.
    pub fn pool_room(&self, party: Entity, id: ItemId, identified: bool, per_row: u32) -> u32 {
        let rows = self.pool(party);
        self.room_among(rows.iter().copied(), id, identified)
            + (POOL_LIMIT - rows.len()) as u32 * per_row
    }

    /// How many of `wanted` units a member could take: what the stacks they
    /// carry have room for, and all of it if a pack slot is free.
    pub fn pack_room(&self, member: Entity, id: ItemId, identified: bool, wanted: u32) -> u32 {
        let alike = self.room_among(self.carried(member), id, identified);
        if self.pack(member).iter().any(Option::is_none) { alike.max(wanted) } else { alike }
    }
}

/// Which equipment slots an item may go in, given what the member's hands
/// hold but for the slots being emptied for it (retail
/// `GetItemEquipSlotMask`). `worn` says what is worn in a slot.
pub fn fits<'a>(
    item: &Wearable,
    worn: impl Fn(EquipSlot) -> Option<&'a Wearable>,
    vacating: &[EquipSlot],
) -> Vec<EquipSlot> {
    use EquipClass::*;
    let worn = |slot: EquipSlot| worn(slot).filter(|_| !vacating.contains(&slot));
    let empty = |slot: EquipSlot| worn(slot).is_none();
    // A main hand also counts as free for the off hand when its weapon is
    // one-handed.
    let main_free = |slot: EquipSlot| worn(slot).is_none_or(|main| !main.two_handed);
    let off_hands = || {
        [
            (EquipSlot::Primary, EquipSlot::Secondary),
            (EquipSlot::AlternatePrimary, EquipSlot::AlternateSecondary),
        ]
        .into_iter()
        .filter(|&(main, _)| main_free(main))
        .map(|(_, off)| off)
    };
    match item.class {
        ShortWeapon | ExtendedWeapon | ThrownWeapon | RangedWeapon if !item.skilled => vec![],
        ShortWeapon | ExtendedWeapon | ThrownWeapon | RangedWeapon if item.two_handed => [
            (EquipSlot::Primary, EquipSlot::Secondary),
            (EquipSlot::AlternatePrimary, EquipSlot::AlternateSecondary),
        ]
        .into_iter()
        .filter(|&(_, off)| empty(off))
        .map(|(main, _)| main)
        .collect(),
        ShortWeapon | ExtendedWeapon | ThrownWeapon | RangedWeapon => {
            let mut slots = vec![EquipSlot::Primary, EquipSlot::AlternatePrimary];
            if item.off_hand {
                slots.extend(off_hands());
            }
            slots
        }
        Ammunition | Shield => off_hands().collect(),
        Torso => vec![EquipSlot::Torso],
        Legs => vec![EquipSlot::Legs],
        Head => vec![EquipSlot::Head],
        Hands => vec![EquipSlot::Hands],
        Feet => vec![EquipSlot::Feet],
        Cloak => vec![EquipSlot::Cloak],
        Misc => vec![EquipSlot::Misc1, EquipSlot::Misc2],
        _ => vec![],
    }
}

/// Guns and what loads them (retail `g_compatible_partner_pairs`).
const LOADED_BY: [(ItemKind, ItemKind); 5] = [
    (ItemKind::MUSKET, ItemKind::POWDER_SHOT),
    (ItemKind::PHASER, ItemKind::POWER_PACK),
    (ItemKind::POWER_GLOVE, ItemKind::POWER_PACK),
    (ItemKind::ROCKET_LAUNCHER, ItemKind::ROCKET),
    (ItemKind::FLAME_THROWER, ItemKind::FUEL),
];

/// What loads an item of this kind (retail `GetItemMergeKind`).
pub fn loaded_by(kind: ItemKind) -> Option<ItemKind> {
    LOADED_BY.iter().find(|&&(gun, _)| gun == kind).map(|&(_, ammunition)| ammunition)
}

/// What the rules of holding things together read of an item.
#[derive(QueryData)]
pub struct Fit {
    pub item: &'static Item,
    pub kind: &'static Kind,
    pub wear: &'static Wearable,
}

/// Whether `other` may be held with the ranged weapon `ranged` (retail
/// `CompatiblePartnerItems`): bows, crossbows and slings with their
/// missiles, guns with what loads them, and each omnigun (by which one it
/// is) with what it has learnt to fire.
pub fn partners(ranged: &FitItem, other: &FitItem) -> bool {
    use ItemKind as K;
    let kind = other.kind.0;
    let dagger = other.wear.class == EquipClass::ThrownWeapon && kind == K::DAGGER;
    let small = matches!(kind, K::SLING_STONE | K::POWDER_SHOT);
    let stars = matches!(kind, K::DARTS | K::SHURIKEN | K::THROWING_SHURIKEN);
    match ranged.kind.0 {
        K::LONGBOW | K::GREATBOW => kind == K::ARROW,
        K::CROSSBOW => kind == K::QUARREL,
        K::SLING => kind == K::SLING_STONE,
        K::POWER_GLOVE | K::PHASER | K::MUSKET => {
            (u8::from(other.wear.class) > u8::from(EquipClass::RangedWeapon)
                || ranged.wear.reach == other.wear.reach)
                && loaded_by(ranged.kind.0).is_none_or(|loads| loads == kind)
        }
        K::OMNIGUN => match ranged.item.0.0 {
            599 => small,
            600 | 601 => small || kind == K::DARTS,
            602..=605 => small || stars || dagger,
            606..=610 => small || stars || dagger || matches!(kind, K::ARROW | K::QUARREL),
            _ => false,
        },
        _ => false,
    }
}

/// Whether two items can share a weapon set (retail `CanHoldItemsTogether`):
/// nothing pairs with a two-handed item, a ranged weapon or ammunition by
/// what each takes, and two weapons by their reach.
pub fn hold_together(a: &FitItem, b: &FitItem) -> bool {
    use EquipClass::*;
    if a.wear.two_handed || b.wear.two_handed {
        return false;
    }
    if a.wear.class == RangedWeapon || b.wear.class == Ammunition {
        return partners(a, b);
    }
    if b.wear.class == RangedWeapon || a.wear.class == Ammunition {
        return partners(b, a);
    }
    u8::from(a.wear.class) >= u8::from(Ammunition)
        || u8::from(b.wear.class) >= u8::from(Ammunition)
        || a.wear.reach == b.wear.reach
}

/// Put some of one stack onto another alike: as many as it has room for.
/// The source emptied is gone. How many moved.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Stack {
    /// The stack that grows.
    #[event_target]
    pub onto: Entity,
    pub from: Entity,
}

fn stack(event: On<Stack>, mut stacks: Query<(&Item, &mut Count, &Known)>, mut commands: Commands) {
    let Ok([(item, mut onto, known), (other, mut from, theirs)]) =
        stacks.get_many_mut([event.onto, event.from])
    else {
        return;
    };
    if item != other || known.identified != theirs.identified {
        return;
    }
    let moved = (onto.limit.max(onto.of) - onto.of).min(from.of);
    onto.of += moved;
    from.of -= moved;
    if from.of == 0 {
        commands.entity(event.from).despawn();
    }
    commands.trigger(Reweigh);
}

/// What is left of a part taken from a stack, still stowed beside the stack
/// it was taken from, joins it again. A part that went elsewhere stays there.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Rejoin {
    /// The part.
    #[event_target]
    pub part: Entity,
    pub whole: Entity,
}

fn rejoin(event: On<Rejoin>, stowed: Query<&Stowed>, mut commands: Commands) {
    let (Ok(part), Ok(whole)) = (stowed.get(event.part), stowed.get(event.whole)) else { return };
    if part == whole {
        commands.trigger(Stack { onto: event.whole, from: event.part });
    }
}

/// An item goes to a place, leaving wherever it was. Whoever asks has
/// checked that it may (see [`fits`]); this only moves it.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Put {
    #[event_target]
    pub item: Entity,
    pub at: Place,
}

fn put(event: On<Put>, mut curses: Query<&mut Cursed>, mut commands: Commands) {
    let item = event.item;
    commands.entity(item).remove::<(
        Worn,
        Packed,
        Pooled,
        InHand,
        Stowed,
        Lying,
        Hidden,
        LevelOwned,
        Transform,
    )>();
    match event.at {
        Place::Slot { member, slot } => {
            // A curse takes hold anew each time it is put on.
            if let Ok(mut curse) = curses.get_mut(item) {
                curse.released = false;
            }
            commands.entity(item).insert(Worn { by: member, slot });
        }
        Place::Pack { member, slot } => {
            commands.entity(item).insert(Packed { by: member, slot });
        }
        Place::Pool { party, row } => {
            commands.entity(party).insert_related::<Pooled>(row, &[item]);
        }
        Place::Hand { party } => {
            commands.entity(item).insert(InHand(party));
        }
        Place::Container(container) => {
            commands.entity(item).insert(Stowed(container));
        }
        Place::Ground { level, at } => {
            commands.entity(item).insert((
                Lying,
                LevelOwned(level),
                Transform::from_translation(at),
            ));
        }
    }
    commands.trigger(Reweigh);
}

/// A part of a stack, taken as an item of its own: `amount` of it, with the
/// facts of the whole copied and in the same place. The whole is left with
/// the rest. A proper part only.
pub fn split(commands: &mut Commands, stack: Entity, amount: u16) -> Entity {
    let part = commands.entity(stack).clone_and_spawn().id();
    commands.entity(part).entry::<Count>().and_modify(move |mut count| count.of = amount);
    commands.trigger(Lose { item: stack, amount });
    part
}

/// A gun or a charged thing is loaded from what loads it (retail
/// `MergeItemUses`): as many charges as it has room for, spent from the
/// source one at a time. An overfull one takes none.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Load {
    #[event_target]
    pub into: Entity,
    pub from: Entity,
}

fn load(
    event: On<Load>,
    mut charges: Query<&mut Charges>,
    mut counts: Query<&mut Count>,
    keeps: Query<(), With<KeepsEmpty>>,
    mut commands: Commands,
) {
    let Ok(taking) = charges.get(event.into).copied() else { return };
    let stacked = counts.get(event.from).ok().map(|count| count.of);
    let Some(available) = stacked.or(charges.get(event.from).ok().map(|from| from.left)) else {
        return;
    };
    let moved = available.min(taking.full.max(taking.left) - taking.left);
    charges.get_mut(event.into).unwrap().left += moved;
    let left = available - moved;
    match counts.get_mut(event.from) {
        Ok(mut count) => count.of = left,
        Err(_) => charges.get_mut(event.from).unwrap().left = left,
    }
    if left == 0 && (stacked.is_some() || !keeps.contains(event.from)) {
        commands.entity(event.from).despawn();
    }
    commands.trigger(Reweigh);
}

/// An item is used: a charge is spent, or one of a stack, or the thing
/// itself; or `amount` of them, as far as it has. Spent out, it is gone,
/// unless it keeps when empty.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Spend {
    #[event_target]
    pub item: Entity,
    pub amount: u16,
}

fn spend(
    event: On<Spend>,
    mut items: Query<(Option<&mut Charges>, Option<&mut Count>, Has<SpentByUse>, Has<KeepsEmpty>)>,
    worn: Query<&Worn>,
    mut commands: Commands,
) {
    let Ok((charges, count, spent_by_use, keeps)) = items.get_mut(event.item) else { return };
    let (gone, empty) = match (charges, count) {
        (Some(mut charges), _) => {
            charges.left -= charges.left.min(event.amount);
            (charges.left == 0 && !keeps, charges.left == 0)
        }
        (None, Some(mut count)) if spent_by_use => {
            count.of -= count.of.min(event.amount);
            (count.of == 0, count.of == 0)
        }
        (None, _) => (spent_by_use, spent_by_use),
    };
    if gone {
        // A worn item going is what refits its wearer.
        commands.entity(event.item).despawn();
    } else if empty {
        // One that stays, out of shots, no longer strikes.
        refit_wearer(event.item, &worn, &mut commands);
    }
    commands.trigger(Reweigh);
}

/// Some of an item leave the party for good, sold, given away or burnt: that
/// many of a stack, or all of anything else.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Lose {
    #[event_target]
    pub item: Entity,
    pub amount: u16,
}

fn lose(event: On<Lose>, mut counts: Query<&mut Count>, mut commands: Commands) {
    let gone = match counts.get_mut(event.item) {
        Ok(mut count) => {
            count.of -= count.of.min(event.amount);
            count.of == 0
        }
        Err(_) => true,
    };
    if gone {
        commands.entity(event.item).despawn();
    }
    commands.trigger(Reweigh);
}

/// An item comes to the party: to a member if one is named, into what they
/// carry alike and then their first free pack slot; and to the pool, into
/// rows alike and then a new row at its top. The pool is tried first when
/// it is to be preferred, otherwise after the member. What nobody has room
/// for stays where it was.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Give {
    #[event_target]
    pub item: Entity,
    pub to: Option<Entity>,
    pub party: Entity,
    pub pool_first: bool,
    /// Whether the party is told who took it, or that nobody could.
    pub told: bool,
}

/// Put as much of `item` onto the stacks alike to it among `onto`, in order,
/// as they have room for; `left` is how many of it are yet to place. Whether
/// any went.
fn join(
    items: &Items,
    commands: &mut Commands,
    item: Entity,
    onto: impl IntoIterator<Item = Entity>,
    left: &mut u16,
) -> bool {
    let before = *left;
    for onto in onto {
        let moved = items.room(onto, item).min(*left);
        if moved > 0 {
            commands.trigger(Stack { onto, from: item });
            *left -= moved;
        }
    }
    *left < before
}

fn give(
    event: On<Give>,
    items: Items,
    shown: Query<(&Item, &Known)>,
    names: Query<&Character>,
    content: Res<Catalogs>,
    mut news: MessageWriter<ItemNews>,
    mut commands: Commands,
) {
    let Give { item, to, party, pool_first, told } = *event;
    let Ok((&Item(id), known)) = shown.get(item) else { return };
    let mut left = items.units(item) as u16;
    let mut taker = None;
    if pool_first && take_into_pool(&items, &mut commands, (item, party), &mut left) {
        taker = Some(None);
    }
    if let Some(member) = to.filter(|_| left > 0)
        && take_into_pack(&items, &mut commands, (item, member), &mut left)
    {
        taker = Some(Some(member));
    }
    if !pool_first && left > 0 && take_into_pool(&items, &mut commands, (item, party), &mut left) {
        taker = Some(None);
    }
    if !told {
        return;
    }
    let name = content.items[id].known_name(known.identified).to_owned();
    if let Some(member) = taker {
        news.write(ItemNews::Takes {
            member: member.and_then(|member| names.get(member).ok()).map(|c| c.name.clone()),
            item: name,
        });
    }
    // What nobody had room for stays where it was, and the party is told.
    if left > 0 {
        news.write(ItemNews::NoRoom);
    }
}

/// A member takes what they can of an item: into what they carry alike,
/// then their first free pack slot. Whether any went.
fn take_into_pack(
    items: &Items,
    commands: &mut Commands,
    (item, member): (Entity, Entity),
    left: &mut u16,
) -> bool {
    let carried = items.carried(member).filter(|&other| other != item).collect::<Vec<_>>();
    let mut took = join(items, commands, item, carried, left);
    let free = items.pack(member).iter().position(Option::is_none);
    if let Some(slot) = free.filter(|_| *left > 0) {
        commands.trigger(Put { item, at: Place::Pack { member, slot } });
        (*left, took) = (0, true);
    }
    took
}

/// The party's pool takes what it can of an item: into the rows alike, then
/// a new row at its top. Whether any went.
fn take_into_pool(
    items: &Items,
    commands: &mut Commands,
    (item, party): (Entity, Entity),
    left: &mut u16,
) -> bool {
    let rows = items.pool(party);
    let mut took = join(items, commands, item, rows.iter().copied(), left);
    if *left > 0 && rows.len() < POOL_LIMIT {
        commands.trigger(Put { item, at: Place::Pool { party, row: 0 } });
        (*left, took) = (0, true);
    }
    took
}

/// The party's hand meets a place: what is held goes there and what was
/// there comes to hand. Alike things stack instead, and what loads a gun
/// loads it. A slot refuses what does not fit it, what its wearer may not
/// use and what the other hand rules out, and a cursed thing worn in use
/// refuses to leave.
#[derive(Event, Clone, Copy, Debug)]
pub struct Swap {
    pub party: Entity,
    pub at: Place,
}

/// A swap that was refused, and why.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refused(pub Refusal);

fn swap(
    event: On<Swap>,
    items: Items,
    fit: Query<Fit>,
    things: Query<(&Kind, Option<&Cursed>, Has<Charges>)>,
    members: Query<(&Character, &Vitals), With<PartyMember>>,
    mut refused: MessageWriter<Refused>,
    mut commands: Commands,
) {
    let Swap { party, at } = *event;
    let held = items.held(party);
    let outcome = match at {
        Place::Pool { row, .. } => {
            let rows = items.pool(party);
            match (held, rows.get(row).copied()) {
                (None, None) => Err(Refusal::Nothing),
                (None, Some(there)) => {
                    commands.trigger(Put { item: there, at: Place::Hand { party } });
                    Ok(())
                }
                (Some(held), there) if there.is_some_and(|gun| loads(&things, gun, held)) => {
                    commands.trigger(Load { into: there.unwrap(), from: held });
                    Ok(())
                }
                // The row it is dropped on fills first, then the rest, then
                // it takes a row of its own there.
                (Some(held), there) => {
                    let mut left = items.units(held) as u16;
                    let before = left;
                    let others = rows.iter().copied().filter(|&other| Some(other) != there);
                    join(&items, &mut commands, held, there.into_iter().chain(others), &mut left);
                    if left > 0 && rows.len() < POOL_LIMIT {
                        let row = row.min(rows.len());
                        commands.trigger(Put { item: held, at: Place::Pool { party, row } });
                        left = 0;
                    }
                    if left < before { Ok(()) } else { Err(Refusal::NoRoom) }
                }
            }
        }
        Place::Slot { member, .. } | Place::Pack { member, .. } => {
            carry(member, party, at, held, &items, &fit, &things, &members, &mut commands)
        }
        Place::Hand { .. } | Place::Container(_) | Place::Ground { .. } => Err(Refusal::Nothing),
    };
    if let Err(refusal) = outcome {
        refused.write(Refused(refusal));
    }
}

/// Whether `ammunition` is what loads `gun`.
fn loads(
    things: &Query<(&Kind, Option<&Cursed>, Has<Charges>)>,
    gun: Entity,
    ammunition: Entity,
) -> bool {
    let (Ok((gun, _, true)), Ok((ammunition, ..))) = (things.get(gun), things.get(ammunition))
    else {
        return false;
    };
    loaded_by(gun.0) == Some(ammunition.0)
}

/// The hand meets something a member carries: a slot they wear in, or one
/// of their pack.
#[allow(clippy::too_many_arguments)]
fn carry(
    member: Entity,
    party: Entity,
    at: Place,
    held: Option<Entity>,
    items: &Items,
    fit: &Query<Fit>,
    things: &Query<(&Kind, Option<&Cursed>, Has<Charges>)>,
    members: &Query<(&Character, &Vitals), With<PartyMember>>,
    commands: &mut Commands,
) -> Result<(), Refusal> {
    let (character, vitals) = members.get(member).map_err(|_| Refusal::Nothing)?;
    let (slot, there) = match at {
        Place::Slot { slot, .. } => (Some(slot), items.worn(member, slot)),
        Place::Pack { slot, .. } => (None, items.pack(member)[slot]),
        _ => unreachable!("a member carries in slots and their pack"),
    };
    if held.is_none() && there.is_none() {
        return Err(Refusal::Nothing);
    }
    // Only what its wearer may use goes on, or onto what is worn.
    let usable = |item: Entity| {
        slot.is_none() || fit.get(item).is_ok_and(|item| item.wear.users.admit(character))
    };
    if let (Some(held), Some(there)) = (held, there)
        && items.alike(there, held)
    {
        return if usable(held) && items.room(there, held) > 0 {
            commands.trigger(Stack { onto: there, from: held });
            Ok(())
        } else {
            Err(Refusal::DoesNotFit)
        };
    }
    // A cursed thing worn in use holds on while its wearer lives, and the
    // party learns of it by trying.
    let held_fast = |worn: Entity, slot: EquipSlot, commands: &mut Commands| {
        let cursed = things.get(worn).is_ok_and(|(_, curse, _)| curse.is_some_and(|c| !c.released));
        if cursed && slot.is_active() && !vitals.is_dead() {
            commands.entity(worn).entry::<Known>().and_modify(|mut known| known.curse = true);
            return Err(Refusal::Cursed);
        }
        Ok(())
    };
    if let (Some(slot), Some(there)) = (slot, there) {
        held_fast(there, slot, commands)?;
    }
    let Some(held) = held else {
        commands.trigger(Put { item: there.unwrap(), at: Place::Hand { party } });
        return Ok(());
    };
    if let Some(gun) = there.filter(|&gun| loads(things, gun, held)) {
        if !usable(held) {
            return Err(Refusal::DoesNotFit);
        }
        commands.trigger(Load { into: gun, from: held });
        return Ok(());
    }
    if let Some(slot) = slot {
        let coming = fit.get(held).ok().filter(|coming| coming.wear.users.admit(character));
        let coming = coming.ok_or(Refusal::DoesNotFit)?;
        let worn = |slot| items.worn(member, slot).and_then(|worn| fit.get(worn).ok());
        // What the other hand holds may have to go.
        let partner = slot.partner().and_then(|other| Some((other, items.worn(member, other)?)));
        let displaced = partner.filter(|&(_, other)| {
            fit.get(other).is_ok_and(|other| !hold_together(&coming, &other))
        });
        let vacating: Vec<EquipSlot> =
            std::iter::once(slot).chain(displaced.map(|(other, _)| other)).collect();
        if let Some((other, leaving)) = displaced {
            held_fast(leaving, other, commands)?;
        }
        if !fits(coming.wear, |slot| worn(slot).map(|worn| worn.wear), &vacating).contains(&slot) {
            return Err(Refusal::DoesNotFit);
        }
        if let Some((_, leaving)) = displaced {
            // It goes to their pack or the pool, which must have room.
            let stays = |&other: &Entity| Some(other) != there && other != leaving;
            let room: u32 = items
                .carried(member)
                .filter(stays)
                .chain(items.pool(party).iter().copied())
                .map(|onto| u32::from(items.room(onto, leaving)))
                .sum();
            let pack_full = items.pack(member).iter().all(Option::is_some);
            if pack_full && items.pool(party).len() >= POOL_LIMIT && room < items.units(leaving) {
                return Err(Refusal::NoRoom);
            }
        }
        commands.trigger(Put { item: held, at });
        if let Some(there) = there {
            commands.trigger(Put { item: there, at: Place::Hand { party } });
        }
        if let Some((_, leaving)) = displaced {
            commands.trigger(Give {
                item: leaving,
                to: Some(member),
                party,
                pool_first: false,
                told: false,
            });
        }
        return Ok(());
    }
    commands.trigger(Put { item: held, at });
    if let Some(there) = there {
        commands.trigger(Put { item: there, at: Place::Hand { party } });
    }
    Ok(())
}

/// The portrait of a member is clicked with something in hand: it is given
/// to them. One who cannot act takes nothing, and in a fight the pool is
/// tried first.
#[derive(Event, Clone, Copy, Debug)]
pub struct Pass {
    pub party: Entity,
    pub to: Entity,
}

fn pass(
    event: On<Pass>,
    items: Items,
    members: Query<&Vitals, With<PartyMember>>,
    battles: Query<(), With<Battle>>,
    mut commands: Commands,
) {
    let Pass { party, to } = *event;
    let (Some(item), Ok(vitals)) = (items.held(party), members.get(to)) else { return };
    let eligible = vitals.can_act();
    let pool_first = !eligible || !battles.is_empty();
    commands.trigger(Give { item, to: eligible.then_some(to), party, pool_first, told: true });
}

/// The party throws what it has in hand, and it comes to rest at a point of a
/// level. A crucial item is never thrown away: it stays in hand, and the
/// party is told.
#[derive(Event, Clone, Copy, Debug)]
pub struct Throw {
    pub party: Entity,
    pub level: Entity,
    pub at: Vec3,
}

fn throw(
    event: On<Throw>,
    items: Items,
    crucial: Query<(), With<Crucial>>,
    mut news: MessageWriter<ItemNews>,
    mut commands: Commands,
) {
    let Throw { party, level, at } = *event;
    let Some(item) = items.held(party) else { return };
    if crucial.contains(item) {
        news.write(ItemNews::CannotDrop);
        return;
    }
    commands.trigger(Put { item, at: Place::Ground { level, at } });
}

/// What the party is told of items coming to it and being put together.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum ItemNews {
    Takes {
        member: Option<String>,
        item: String,
    },
    NoRoom,
    /// An item the party may not part with stays in hand instead of being
    /// thrown away (retail string 0x4ef).
    CannotDrop,
    /// Merging needs both items identified.
    Unidentified,
    /// Nothing is made from the two.
    CannotMerge,
    /// Two items were stacked or something was made of them.
    Merged,
    /// The member lacks the skill making it takes.
    Unskilled {
        member: String,
    },
    /// A Gadgeteer was given their omnigun (tier 0) or it reached a tier
    /// (1 to 11 above the first).
    Omnigun {
        member: String,
        gender: Gender,
        tier: usize,
    },
}

/// Why something was not done with an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Nothing,
    Crucial,
    Cursed,
    DoesNotFit,
    HandFull,
    NoRoom,
}

/// The first item made from these two, in item order (retail scans every
/// record and takes the first whose ingredients match in either order).
pub fn made_from(catalog: &ItemCatalog, a: ItemId, b: ItemId) -> Option<(&ItemDefinition, Recipe)> {
    catalog.0.iter().find_map(|definition| {
        let recipe = definition.recipe?;
        (recipe.ingredients == [a, b] || recipe.ingredients == [b, a])
            .then_some((definition, recipe))
    })
}

/// The hand's item is merged with one the party has (retail `MergeItems`).
/// Both must be identified. When an item is made from the two and the
/// member has the skill it takes (practising it), both give up as many units
/// as the smaller stack holds (one of anything else), and the made item
/// takes the place of whichever is used up. Otherwise two of one item stack
/// together.
///
/// Deviations, recorded: retail puts the made item into an equipment slot
/// whatever it is, and loses it when the used-up ingredient was the pool's
/// last row; here it goes to the hand when the slot cannot hold it (the
/// merge is refused, changing nothing, when the hand stays full), and it
/// takes the row's place in the pool.
#[derive(Event, Clone, Copy, Debug)]
pub struct Merge {
    pub party: Entity,
    pub member: Entity,
    pub onto: Entity,
}

#[allow(clippy::too_many_arguments)]
fn merge(
    event: On<Merge>,
    items: Items,
    fit: Query<Fit>,
    characters: Query<&Character, With<PartyMember>>,
    content: Res<Catalogs>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<ItemNews>,
    mut refused: MessageWriter<Refused>,
    mut commands: Commands,
) {
    let Merge { party, member, onto } = *event;
    let (Some(held), Ok(character)) = (items.held(party), characters.get(member)) else {
        refused.write(Refused(Refusal::Nothing));
        return;
    };
    let place = items.place(onto);
    let (
        Some(held_id),
        Some(onto_id),
        Some(Place::Slot { .. } | Place::Pack { .. } | Place::Pool { .. }),
    ) = (items.id(held), items.id(onto), place)
    else {
        refused.write(Refused(Refusal::Nothing));
        return;
    };
    let place = place.unwrap();
    if !items.known(held).is_some_and(|known| known.identified)
        || !items.known(onto).is_some_and(|known| known.identified)
    {
        news.write(ItemNews::Unidentified);
        return;
    }
    let made = made_from(&content.items, held_id, onto_id);
    let skilled = made.is_some_and(|(_, recipe)| {
        recipe.skill.is_none_or(|(skill, level)| character.skill(skill) >= u32::from(level))
    });
    let Some((product, recipe)) = made.filter(|_| skilled) else {
        if held_id == onto_id {
            if items.room(onto, held) > 0 {
                commands.trigger(Stack { onto, from: held });
                news.write(ItemNews::Merged);
            }
        } else if made.is_some() {
            news.write(ItemNews::Unskilled { member: character.name.clone() });
        } else {
            news.write(ItemNews::CannotMerge);
        }
        return;
    };
    // Two stacks give up the smaller's units, which then holds the made
    // item; anything else gives up one, and the made item goes where the
    // non-stack was (the hand when both are).
    let (held_count, onto_count) = (items.count(held), items.count(onto));
    let (units, mut into_hand) = match (held_count, onto_count) {
        (Some(held), Some(onto)) => (held.min(onto), onto > held),
        (Some(_), None) => (1, false),
        _ => (1, true),
    };
    let held_survives = held_count.is_some_and(|count| count > units);
    if !into_hand && let Place::Slot { slot, .. } = place {
        let worn = |slot| items.worn(member, slot).and_then(|worn| fit.get(worn).ok());
        let fits = character.can_use(product)
            && wearable(product).is_some_and(|wear| {
                fits(&wear, |slot| worn(slot).map(|worn| worn.wear), &[slot]).contains(&slot)
            });
        if !fits && held_survives {
            refused.write(Refused(Refusal::DoesNotFit));
            return;
        }
        into_hand = !fits;
    }
    if let Some((skill, level)) = recipe.skill {
        commands.trigger(Practise { member, skill, usage: u32::from(level / 10) });
    }
    for item in [held, onto] {
        commands.trigger(Lose { item, amount: units });
    }
    let amount = if product.quantity == QuantityKind::Stack {
        units
    } else {
        roll_amount(product, &mut rng, false)
    };
    let made = make(&mut commands, product, amount, false);
    let at = if into_hand { Place::Hand { party } } else { place };
    commands.trigger(Put { item: made, at });
    commands.trigger(Reweigh);
    news.write(ItemNews::Merged);
}

/// The party tries to identify an item (retail `PartyAttemptsToIdentifyItem`):
/// everyone alive and sane tries in turn, by their Artifacts skill. If any
/// succeeds the party knows what it is and its spell, and whether it binds
/// if they were far enough above its difficulty.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Identify {
    #[event_target]
    pub item: Entity,
    pub party: Entity,
}

fn identify(
    event: On<Identify>,
    shown: Query<(&Item, &Known)>,
    rosters: Query<&PartyRoster>,
    members: Query<(&Character, &Vitals)>,
    content: Res<Catalogs>,
    mut commands: Commands,
) {
    let (Ok((&Item(id), &known)), Ok(roster)) = (shown.get(event.item), rosters.get(event.party))
    else {
        return;
    };
    if known.identified {
        return;
    }
    let difficulty = content.items[id].identify_difficulty;
    let (mut identified, mut binding) = (false, false);
    for attempting in roster.iter() {
        let Ok((character, vitals)) = members.get(attempting) else { continue };
        let Some(attempt) = identify_attempt(character, vitals, difficulty) else { continue };
        identified = true;
        binding |= attempt.binding;
        if attempt.practises {
            commands.trigger(Practise { member: attempting, skill: Skill::Artifacts, usage: 2 });
        }
    }
    if identified {
        commands.entity(event.item).insert(Known {
            identified: true,
            spell: true,
            curse: known.curse || binding,
        });
    }
}

/// What one member's go at identifying an item came to.
struct Identified {
    /// Far enough above the item's difficulty to tell whether it binds.
    binding: bool,
    /// Only just made, which practises Artifacts.
    practises: bool,
}

/// One member's go at identifying an item of this difficulty (retail
/// `TryIdentifyItemFor`): someone alive and sane manages it when a sixth of
/// their Artifacts reaches it. Higher difficulty never becomes easier
/// by overflowing a signed byte.
fn identify_attempt(character: &Character, vitals: &Vitals, difficulty: u8) -> Option<Identified> {
    if !vitals.lucid() {
        return None;
    }
    let strength = character.skill(Skill::Artifacts) / 6;
    let difficulty = difficulty as u32;
    if difficulty > strength {
        return None;
    }
    let margin = strength - difficulty;
    Some(Identified { binding: margin >= 3, practises: (0..3).contains(&margin) })
}

/// Starting equipment may equip; ordinary receipt never does. An item of a
/// new member's starting set is worn where it belongs if its wearer may use
/// it and it fits there, joins what is alike worn there, or else goes to
/// their pack (retail `AddCharacterStartingEquipment`).
#[derive(Event, Clone, Copy, Debug)]
pub struct Outfit {
    pub member: Entity,
    pub item: Entity,
}

fn outfit(
    event: On<Outfit>,
    items: Items,
    fit: Query<Fit>,
    characters: Query<&Character>,
    mut commands: Commands,
) {
    let Outfit { member, item } = *event;
    let (Ok(character), Ok(coming)) = (characters.get(member), fit.get(item)) else { return };
    let worn = |slot| items.worn(member, slot).and_then(|worn| fit.get(worn).ok());
    let slot = creation_slot(&coming, &worn, character).filter(|slot| {
        coming.wear.users.admit(character)
            && fits(coming.wear, |slot| worn(slot).map(|worn| worn.wear), &[]).contains(slot)
    });
    let mut left = items.units(item) as u16;
    if let Some(slot) = slot {
        match items.worn(member, slot) {
            None => {
                commands.trigger(Put { item, at: Place::Slot { member, slot } });
                return;
            }
            Some(there) => {
                join(&items, &mut commands, item, [there], &mut left);
                if left == 0 {
                    return;
                }
            }
        }
    }
    if !take_into_pack(&items, &mut commands, (item, member), &mut left) {
        warn!("no room for {:?} in their pack", coming.item.0);
    }
}

/// Where creation puts an item (retail `GetItemDefaultEquipSlot` with the
/// creation adjustments of `ChooseCharacterEquipSlot`). Moving a worn
/// weapon to the off hand to make room is not modelled.
fn creation_slot<'a>(
    coming: &FitItem,
    worn: &impl Fn(EquipSlot) -> Option<FitItem<'a, 'a>>,
    member: &Character,
) -> Option<EquipSlot> {
    use EquipClass::*;
    let empty = |slot| worn(slot).is_none();
    let ranged_in = |slot| {
        worn(slot)
            .is_some_and(|worn| worn.wear.class == RangedWeapon && hold_together(&worn, coming))
    };
    Some(match coming.wear.class {
        ShortWeapon | ExtendedWeapon if empty(EquipSlot::Primary) => EquipSlot::Primary,
        ShortWeapon | ExtendedWeapon
            if member.skill_points[Skill::DualWeapons] > 0 && empty(EquipSlot::Secondary) =>
        {
            EquipSlot::Secondary
        }
        RangedWeapon if empty(EquipSlot::Primary) && empty(EquipSlot::Secondary) => {
            EquipSlot::Primary
        }
        RangedWeapon
            if empty(EquipSlot::AlternatePrimary) && empty(EquipSlot::AlternateSecondary) =>
        {
            EquipSlot::AlternatePrimary
        }
        Ammunition if ranged_in(EquipSlot::Primary) => EquipSlot::Secondary,
        Ammunition if ranged_in(EquipSlot::AlternatePrimary) => EquipSlot::AlternateSecondary,
        ThrownWeapon => EquipSlot::AlternatePrimary,
        Shield => EquipSlot::Secondary,
        Misc => EquipSlot::Misc1,
        Torso => EquipSlot::Torso,
        Legs => EquipSlot::Legs,
        Head => EquipSlot::Head,
        Hands => EquipSlot::Hands,
        Feet => EquipSlot::Feet,
        Cloak => EquipSlot::Cloak,
        _ => return None,
    })
}

/// What a member carries, counting their share of the pool's weight (retail
/// `total_carried_weight` and `load_category`), as the last weighing left it.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Burden {
    /// Tenths of a pound.
    pub weight: u32,
    pub load: crate::game::characters::Load,
}

/// What a party's members carry or can carry has changed: their items, the
/// pool, their strength, who is alive or in the party. Fired by whatever
/// changes it; not run in a fight, which keeps the loads it began with until
/// it ends.
#[derive(Event, Clone, Copy, Debug)]
pub struct Reweigh;

/// Share the pool's weight among the members and settle each member's load
/// (retail `RedistributePartyEncumbrance`). A member whose load changed is
/// refitted.
fn reweigh(
    _: On<Reweigh>,
    parties: Query<(Entity, &PartyRoster)>,
    items: Items,
    mut members: Query<(Entity, &Character, &Vitals, &Attributes, &mut Burden)>,
    battles: Query<(), With<Battle>>,
    mut commands: Commands,
) {
    if !battles.is_empty() {
        return;
    }
    for (party, roster) in &parties {
        let own: Vec<_> = members
            .iter_many(roster.iter())
            .matched()
            .map(|(member, character, vitals, attributes, _)| {
                let weight = items.carried(member).map(|item| items.weight(item)).sum::<u32>();
                (weight, (!vitals.is_dead()).then(|| character.capacity(attributes)))
            })
            .collect();
        let pool = items.pool(party).iter().map(|&item| items.weight(item)).sum();
        let eligible: Vec<_> =
            own.iter().map(|&(weight, capacity)| capacity.map(|c| (weight, c))).collect();
        let shares = pool_shares(pool, &eligible);
        let members = members.iter_many_unique_mut(roster.iter()).matched();
        for ((member, character, _, attributes, mut burden), (&(own, _), share)) in
            members.zip(own.iter().zip(shares))
        {
            let weight = own + share;
            let load = character.load(attributes, weight);
            if burden.set_if_neq(Burden { weight, load }) {
                commands.trigger(Refit { entity: member });
            }
        }
    }
}

/// How the pool's weight is shared among the members (retail
/// `DistributePartyWeight`): half of it, a tenth of a pound at a time, each
/// to the member with the largest share of their capacity still free (the
/// first on ties). `members` gives each member's own weight and capacity,
/// or `None` for the dead, who carry none of it.
pub fn pool_shares(pool_weight: u32, members: &[Option<(u32, u32)>]) -> Vec<u32> {
    let mut shares = vec![0; members.len()];
    let free = |own: u32, capacity: u32, share: u32| {
        (capacity as f32 - (own + share) as f32) * 100.0 / capacity as f32
    };
    for _ in 0..pool_weight / 2 {
        let best = members
            .iter()
            .zip(&shares)
            .enumerate()
            .filter_map(|(i, (m, &share))| m.map(|(own, cap)| (i, free(own, cap, share))))
            .fold(None, |best: Option<(usize, f32)>, (i, ratio)| match best {
                Some((_, b)) if b >= ratio => best,
                _ => Some((i, ratio)),
            });
        let Some((index, _)) = best else { break };
        shares[index] += 1;
    }
    shares
}

fn reweigh_added<T: Component>(_: On<Add<T>>, mut commands: Commands) {
    commands.trigger(Reweigh);
}

fn reweigh_removed<T: Component>(_: On<Remove<T>>, mut commands: Commands) {
    commands.trigger(Reweigh);
}

/// Wearing or taking off in a slot in use refits the wearer.
fn refit_wearer(item: Entity, worn: &Query<&Worn>, commands: &mut Commands) {
    if let Ok(&Worn { by, slot }) = worn.get(item)
        && slot.is_active()
    {
        commands.trigger(Refit { entity: by });
    }
}

fn worn_on(worn: On<Insert<Worn>>, items: Query<&Worn>, mut commands: Commands) {
    refit_wearer(worn.entity, &items, &mut commands);
}

fn worn_off(worn: On<Discard<Worn>>, items: Query<&Worn>, mut commands: Commands) {
    refit_wearer(worn.entity, &items, &mut commands);
}

pub(crate) fn plugin(app: &mut App) {
    app.add_message::<Refused>()
        .add_message::<ItemNews>()
        .add_observer(stack)
        .add_observer(put)
        .add_observer(rejoin)
        .add_observer(load)
        .add_observer(spend)
        .add_observer(lose)
        .add_observer(give)
        .add_observer(swap)
        .add_observer(pass)
        .add_observer(throw)
        .add_observer(merge)
        .add_observer(identify)
        .add_observer(outfit)
        .add_observer(reweigh)
        .add_observer(reweigh_added::<crate::game::characters::Dead>)
        .add_observer(reweigh_removed::<crate::game::characters::Dead>)
        .add_observer(reweigh_removed::<Battle>)
        .add_observer(worn_on)
        .add_observer(worn_off);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    fn arrows(world: &mut World, of: u16, identified: bool) -> Entity {
        world
            .spawn((
                Item(ItemId(7)),
                Known { identified, ..default() },
                Weight(2),
                Count { of, limit: 100 },
            ))
            .id()
    }

    fn world() -> World {
        let mut world = World::new();
        world.add_observer(stack);
        world.add_observer(put);
        world
    }

    /// Alike stacks join up to the limit, the emptied one going; stacks the
    /// party knows differently stay apart.
    #[test]
    fn alike_stacks_join_up_to_the_limit() {
        let mut world = world();
        let (quiver, more, strange) = (
            arrows(&mut world, 70, true),
            arrows(&mut world, 50, true),
            arrows(&mut world, 5, false),
        );
        world.trigger(Stack { onto: quiver, from: more });
        world.trigger(Stack { onto: quiver, from: strange });
        world.flush();
        let count = |world: &World, item| world.get::<Count>(item).map(|count| count.of);
        assert_eq!((count(&world, quiver), count(&world, more)), (Some(100), Some(20)));
        assert_eq!(count(&world, strange), Some(5), "not known alike");
        let spare = arrows(&mut world, 30, true);
        world.trigger(Stack { onto: spare, from: more });
        world.flush();
        assert_eq!(count(&world, spare), Some(50));
        assert!(world.get_entity(more).is_err(), "an emptied stack is gone");
    }

    /// An item put somewhere is there and nowhere else, and the pool keeps
    /// the order things were put in it.
    #[test]
    fn an_item_is_in_one_place() {
        let mut world = world();
        let (member, party) = (world.spawn_empty().id(), world.spawn_empty().id());
        let (sword, shield) = (arrows(&mut world, 1, true), arrows(&mut world, 1, false));
        world.trigger(Put { item: sword, at: Place::Pack { member, slot: 3 } });
        world.trigger(Put { item: shield, at: Place::Pool { party, row: 0 } });
        world.flush();
        world.trigger(Put { item: sword, at: Place::Slot { member, slot: EquipSlot::Primary } });
        world.flush();
        world.trigger(Put { item: sword, at: Place::Pool { party, row: 0 } });
        world.flush();
        let seen = world
            .run_system_once(move |items: Items| {
                (
                    items.worn(member, EquipSlot::Primary),
                    items.pack(member),
                    items.pool(party).to_vec(),
                    items.weight(sword),
                )
            })
            .unwrap();
        assert_eq!(seen, (None, [None; PACK_SLOTS], vec![sword, shield], 2));
    }

    use crate::content::items::ItemCatalog;
    use crate::game::characters::{Character, Vitals};

    const ARROWS: ItemId = ItemId(0);
    const BOW: ItemId = ItemId(1);
    const SHIELD: ItemId = ItemId(2);
    const RING: ItemId = ItemId(3);

    /// A party of one in a world with a quiver's worth of definitions.
    fn party() -> (World, Entity, Entity) {
        let anyone = |name: &str, class, kind| ItemDefinition {
            name: name.to_owned(),
            class,
            kind,
            professions: EnumSet::all(),
            races: EnumSet::all(),
            genders: EnumSet::all(),
            weapon_skill: Some(Skill::Bow),
            weight: 2,
            ..default()
        };
        let mut definitions = vec![
            ItemDefinition {
                quantity: QuantityKind::Stack,
                stack_limit: 100,
                ..anyone("Arrows", EquipClass::Ammunition, ItemKind::ARROW)
            },
            anyone("Longbow", EquipClass::RangedWeapon, ItemKind::LONGBOW),
            anyone("Shield", EquipClass::Shield, ItemKind::DAGGER),
            ItemDefinition { cursed: true, ..anyone("Ring", EquipClass::Misc, ItemKind::DAGGER) },
        ];
        for (id, definition) in definitions.iter_mut().enumerate() {
            definition.id = ItemId(id as u16);
        }
        let mut world = World::new();
        world.insert_resource(Catalogs { items: ItemCatalog(definitions), ..default() });
        world.init_resource::<Messages<ItemNews>>();
        world.init_resource::<Messages<Refused>>();
        world.insert_resource(GameRng::seeded(1));
        for observer in [
            Observer::new(stack),
            Observer::new(put),
            Observer::new(rejoin),
            Observer::new(load),
            Observer::new(spend),
            Observer::new(lose),
            Observer::new(give),
            Observer::new(swap),
            Observer::new(pass),
            Observer::new(throw),
            Observer::new(merge),
            Observer::new(identify),
            Observer::new(outfit),
            Observer::new(reweigh),
            Observer::new(crate::game::world::containers::emptied),
            Observer::new(crate::game::world::containers::bag_emptied),
        ] {
            world.spawn(observer);
        }
        let sheet = Character::new(
            "Vi",
            crate::content::characters::Race::Human,
            Gender::Female,
            Profession::Ranger,
        );
        let vitals = Vitals::new(&sheet);
        let party = world.spawn(crate::game::party::roster::Party).id();
        let member = world.spawn((sheet, vitals, PartyMember(party))).id();
        (world, member, party)
    }

    fn new(world: &mut World, id: ItemId, amount: u16) -> Entity {
        let item = world
            .run_system_once(move |mut commands: Commands, content: Res<Catalogs>| {
                make(&mut commands, &content.items[id], amount, true)
            })
            .unwrap();
        world.flush();
        item
    }

    fn held(world: &mut World, party: Entity) -> Option<Entity> {
        world.run_system_once(move |items: Items| items.held(party)).unwrap()
    }

    fn hold(world: &mut World, party: Entity, item: Entity) {
        world.trigger(Put { item, at: Place::Hand { party } });
        world.flush();
    }

    fn refusals(world: &mut World) -> Vec<Refusal> {
        world.resource_mut::<Messages<Refused>>().drain().map(|refused| refused.0).collect()
    }

    /// What comes to a member joins what they carry alike, then takes their
    /// first free pack slot; with no room it goes to the top of the pool.
    #[test]
    fn what_is_given_joins_alike_then_takes_a_free_slot() {
        let (mut world, member, party) = party();
        let quiver = new(&mut world, ARROWS, 90);
        world.trigger(Give {
            item: quiver,
            to: Some(member),
            party,
            pool_first: false,
            told: true,
        });
        world.flush();
        let more = new(&mut world, ARROWS, 30);
        world.trigger(Give { item: more, to: Some(member), party, pool_first: false, told: true });
        world.flush();
        let pack = world.run_system_once(move |items: Items| items.pack(member)).unwrap();
        assert_eq!(pack[..2], [Some(quiver), Some(more)]);
        let count = |world: &World, item| world.get::<Count>(item).unwrap().of;
        assert_eq!((count(&world, quiver), count(&world, more)), (100, 20));
        // A full pack sends the next thing to the pool.
        for _ in 2..PACK_SLOTS {
            let bow = new(&mut world, BOW, 1);
            world.trigger(Give {
                item: bow,
                to: Some(member),
                party,
                pool_first: false,
                told: true,
            });
            world.flush();
        }
        let extra = new(&mut world, BOW, 1);
        world.trigger(Give { item: extra, to: Some(member), party, pool_first: false, told: true });
        world.flush();
        let pool = world.run_system_once(move |items: Items| items.pool(party).to_vec()).unwrap();
        assert_eq!(pool, [extra]);
        let told: Vec<_> = world.resource_mut::<Messages<ItemNews>>().drain().collect();
        let takes = |member: Option<&str>, item: &str| ItemNews::Takes {
            member: member.map(str::to_owned),
            item: item.to_owned(),
        };
        assert_eq!(told.first(), Some(&takes(Some("Vi"), "Arrows")));
        assert_eq!(told.last(), Some(&takes(None, "Longbow")));
    }

    /// The hand and a slot change places; a bow sends a shield from the
    /// other hand to the pack, where its arrows may then go; a cursed ring
    /// in use will not come off.
    #[test]
    fn the_hand_and_a_slot_change_places_by_the_rules_of_fit() {
        use EquipSlot::*;
        let (mut world, member, party) = party();
        let click = |world: &mut World, slot| {
            world.trigger(Swap { party, at: Place::Slot { member, slot } });
            world.flush();
        };
        let worn = |world: &mut World, slot| {
            world.run_system_once(move |items: Items| items.worn(member, slot)).unwrap()
        };
        let (shield, bow, arrows, ring) = (
            new(&mut world, SHIELD, 1),
            new(&mut world, BOW, 1),
            new(&mut world, ARROWS, 20),
            new(&mut world, RING, 1),
        );
        hold(&mut world, party, shield);
        click(&mut world, Torso);
        assert_eq!(refusals(&mut world), [Refusal::DoesNotFit], "a shield is not armour");
        click(&mut world, Secondary);
        assert_eq!((worn(&mut world, Secondary), held(&mut world, party)), (Some(shield), None));
        // The bow takes the main hand; the shield cannot stay beside it.
        hold(&mut world, party, bow);
        click(&mut world, Primary);
        assert_eq!((worn(&mut world, Primary), worn(&mut world, Secondary)), (Some(bow), None));
        let pack = world.run_system_once(move |items: Items| items.pack(member)).unwrap();
        assert_eq!(pack[0], Some(shield));
        hold(&mut world, party, arrows);
        click(&mut world, Secondary);
        assert_eq!(worn(&mut world, Secondary), Some(arrows));
        // A click on an empty hand's slot takes what is there.
        click(&mut world, Secondary);
        assert_eq!(held(&mut world, party), Some(arrows));
        click(&mut world, Secondary);
        // The ring goes on, and holds.
        hold(&mut world, party, ring);
        click(&mut world, Misc1);
        click(&mut world, Misc1);
        assert_eq!(refusals(&mut world), [Refusal::Cursed]);
        assert_eq!(worn(&mut world, Misc1), Some(ring));
        assert!(world.get::<Known>(ring).unwrap().curse, "trying taught the party");
    }

    /// Someone who is not in the party takes nothing and gives nothing.
    #[test]
    fn a_swap_with_someone_outside_the_party_is_refused() {
        let (mut world, member, party) = party();
        world.entity_mut(member).remove::<PartyMember>();
        let sword = new(&mut world, BOW, 1);
        hold(&mut world, party, sword);
        world.trigger(Swap { party, at: Place::Pack { member, slot: 0 } });
        world.trigger(Pass { party, to: member });
        world.flush();
        assert_eq!(refusals(&mut world), [Refusal::Nothing]);
        assert_eq!(held(&mut world, party), Some(sword));
    }

    /// Giving the hand to a portrait puts the item in that member's pack;
    /// one who cannot act leaves it to the pool.
    #[test]
    fn the_hand_is_passed_to_a_member_who_can_act_else_the_pool() {
        let (mut world, member, party) = party();
        let bow = new(&mut world, BOW, 1);
        hold(&mut world, party, bow);
        world.trigger(Pass { party, to: member });
        world.flush();
        let pack = world.run_system_once(move |items: Items| items.pack(member)).unwrap();
        assert_eq!(pack[0], Some(bow));
        let shield = new(&mut world, SHIELD, 1);
        hold(&mut world, party, shield);
        world.get_mut::<Vitals>(member).unwrap().health.current = 0;
        world.trigger(Pass { party, to: member });
        world.flush();
        let pool = world.run_system_once(move |items: Items| items.pool(party).to_vec()).unwrap();
        assert_eq!(pool, [shield]);
    }

    /// Of a stack, losing some leaves the rest; losing all of a thing, or
    /// the last of a stack, takes it away.
    #[test]
    fn losing_part_of_a_stack_leaves_the_rest() {
        let (mut world, _, _) = party();
        let (quiver, bow) = (new(&mut world, ARROWS, 10), new(&mut world, BOW, 1));
        world.trigger(Lose { item: quiver, amount: 4 });
        world.trigger(Lose { item: bow, amount: 1 });
        world.flush();
        assert_eq!(world.get::<Count>(quiver).map(|count| count.of), Some(6));
        assert!(world.get_entity(bow).is_err());
        world.trigger(Spend { item: quiver, amount: 6 });
        world.flush();
        assert!(world.get_entity(quiver).is_err());
    }

    /// Two stacks of arrows held and clicked together merge into one.
    #[test]
    fn merging_two_alike_stacks_stacks_them() {
        let (mut world, member, party) = party();
        let (quiver, more) = (new(&mut world, ARROWS, 10), new(&mut world, ARROWS, 5));
        world.trigger(Put { item: quiver, at: Place::Pack { member, slot: 0 } });
        world.flush();
        hold(&mut world, party, more);
        world.trigger(Merge { party, member, onto: quiver });
        world.flush();
        assert_eq!(world.get::<Count>(quiver).map(|count| count.of), Some(15));
        assert!(world.get_entity(more).is_err());
        let told: Vec<_> = world.resource_mut::<Messages<ItemNews>>().drain().collect();
        assert_eq!(told, [ItemNews::Merged]);
    }

    /// Wearing something in a slot in use is what refits the wearer; the
    /// stored weapon set is not worn.
    #[test]
    fn only_a_slot_in_use_refits_its_wearer() {
        let (mut world, member, _) = party();
        let refits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = refits.clone();
        world.add_observer(move |_: On<Refit>| {
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        world.add_observer(worn_on);
        world.add_observer(worn_off);
        // Weightless, so that its weight does not refit its bearer.
        let ring = new(&mut world, RING, 1);
        world.entity_mut(ring).insert(Weight(0));
        let at = |slot| Put { item: ring, at: Place::Slot { member, slot } };
        world.trigger(at(EquipSlot::AlternatePrimary));
        world.flush();
        assert_eq!(refits.load(std::sync::atomic::Ordering::SeqCst), 0);
        world.trigger(at(EquipSlot::Misc1));
        world.flush();
        // Leaving the stored slot is not a refit; arriving in Misc1 is.
        assert_eq!(refits.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    /// What an item is to a hand and a body is read from its definition as
    /// it is made, and is its components.
    #[test]
    fn an_item_made_from_a_definition_has_the_components_of_what_it_is() {
        let weapon = |kind, skill, modes: EnumSet<AttackMode>| ItemDefinition {
            class: EquipClass::RangedWeapon,
            kind,
            weapon_skill: Some(skill),
            attack_modes: modes,
            damage: Dice { base: 2, count: 1, sides: 4 },
            ..default()
        };
        let made = |definition: ItemDefinition| {
            let mut world = World::new();
            let item = world
                .run_system_once(move |mut commands: Commands| {
                    make(&mut commands, &definition, 1, true)
                })
                .unwrap();
            world.flush();
            (world, item)
        };
        let shot = AttackMode::Shoot | AttackMode::Throw;
        // A bow is strong-armed, a launcher worth a fifth of its base.
        let (bow, item) = made(weapon(ItemKind::LONGBOW, Skill::Bow, AttackMode::Shoot.into()));
        let strike = bow.get::<Weapon>(item).unwrap();
        assert_eq!(
            (strike.strength, strike.ranged, strike.single_swing),
            (Strength::Half, true, false)
        );
        assert_eq!(bow.get::<Launcher>(item), Some(&Launcher { damage: 20 }));
        // A gun is not helped by strength, a swung blade is helped fully.
        let (gun, item) = made(weapon(ItemKind::MUSKET, Skill::ModernWeapon, shot));
        assert_eq!(gun.get::<Weapon>(item).unwrap().strength, Strength::None);
        assert!(gun.get::<Launcher>(item).is_none());
        let (blade, item) = made(ItemDefinition {
            class: EquipClass::ShortWeapon,
            weapon_skill: Some(Skill::Sword),
            attack_modes: AttackMode::Swing | AttackMode::Shoot,
            ..default()
        });
        assert_eq!(blade.get::<Weapon>(item).unwrap().strength, Strength::Full);
        // The rocket launcher swings once.
        let (rocket, item) =
            made(weapon(ItemKind::ROCKET_LAUNCHER, Skill::ModernWeapon, AttackMode::Shoot.into()));
        assert!(rocket.get::<Weapon>(item).unwrap().single_swing);
        // Arrows are missiles; a thrown weapon is a weapon and a missile, and
        // a kept one is not used up.
        let (arrow, item) = made(ItemDefinition {
            class: EquipClass::Ammunition,
            weapon_skill: Some(Skill::Bow),
            ..default()
        });
        assert!(arrow.get::<Missile>(item).is_some_and(|missile| !missile.kept));
        assert!(arrow.get::<Weapon>(item).is_none());
        let (star, item) = made(ItemDefinition {
            class: EquipClass::ThrownWeapon,
            weapon_skill: Some(Skill::ThrowingAndSling),
            never_depletes: true,
            ..default()
        });
        assert!(star.get::<Missile>(item).is_some_and(|missile| missile.kept));
        assert!(star.get::<Weapon>(item).is_some());
        // A shield guards; what protects carries its worth.
        let (guard, item) = made(ItemDefinition {
            class: EquipClass::Shield,
            armor_class_bonus: 3,
            resistance_bonus: enum_map::enum_map! { Realm::Fire => 5, _ => 0 },
            ..default()
        });
        assert!(guard.get::<Shield>(item).is_some());
        let protection = guard.get::<Protection>(item).unwrap();
        assert_eq!((protection.armour, protection.resistance[Realm::Fire]), (3, 5));
        let (plain, item) = made(ItemDefinition { class: EquipClass::Torso, ..default() });
        assert!(plain.get::<Protection>(item).is_none());
    }

    /// A stack of missiles worn in a hand running out ends what the hand
    /// shoots: the wearer is refitted, and not before.
    #[test]
    fn emptying_a_worn_missile_stack_refits_its_wearer() {
        let (mut world, member, _) = party();
        let refits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = refits.clone();
        world.add_observer(move |_: On<Refit>| {
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        world.add_observer(worn_on);
        world.add_observer(worn_off);
        let quiver = new(&mut world, ARROWS, 20);
        world.entity_mut(quiver).insert(Weight(0));
        world.trigger(Put { item: quiver, at: Place::Slot { member, slot: EquipSlot::Secondary } });
        world.flush();
        let refitted = || refits.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(refitted(), 1, "wearing it refits");
        world.trigger(Spend { item: quiver, amount: 5 });
        world.flush();
        assert_eq!(refitted(), 1, "arrows are still there");
        world.trigger(Spend { item: quiver, amount: 15 });
        world.flush();
        assert!(world.get_entity(quiver).is_err());
        assert_eq!(refitted(), 2, "the last arrow shot refits");
        // Losing a stack worn does so as well.
        let more = new(&mut world, ARROWS, 3);
        world.entity_mut(more).insert(Weight(0));
        world.trigger(Put { item: more, at: Place::Slot { member, slot: EquipSlot::Secondary } });
        world.flush();
        world.trigger(Lose { item: more, amount: 3 });
        world.flush();
        assert_eq!(refitted(), 4);
        // A gun out of shots stays, and no longer strikes.
        let gun = new(&mut world, RING, 1);
        world.entity_mut(gun).insert((Weight(0), Charges { left: 1, full: 1 }, KeepsEmpty));
        world.trigger(Put { item: gun, at: Place::Slot { member, slot: EquipSlot::Primary } });
        world.flush();
        let before = refitted();
        world.trigger(Spend { item: gun, amount: 1 });
        world.flush();
        assert!(world.get_entity(gun).is_ok());
        assert_eq!(refitted(), before + 1);
    }

    /// The pool's weight goes half to the members, a tenth of a pound at a
    /// time, each to whoever has the most of their capacity free; the dead
    /// carry none.
    #[test]
    fn the_pool_is_shared_by_free_capacity() {
        assert_eq!(pool_shares(10, &[Some((0, 100)), Some((0, 100))]), [3, 2]);
        assert_eq!(pool_shares(10, &[Some((90, 100)), Some((0, 100))]), [0, 5]);
        assert_eq!(pool_shares(10, &[None, Some((0, 100))]), [0, 5]);
    }

    /// A new member's starting kit is worn where it belongs: the bow in the
    /// main hand with its arrows beside it, a second quiver in the pack, a
    /// ring in the first misc slot.
    #[test]
    fn starting_equipment_is_worn_where_it_belongs() {
        let (mut world, member, _) = party();
        for (id, amount) in [(BOW, 1), (ARROWS, 20), (ARROWS, 20), (RING, 1)] {
            let item = new(&mut world, id, amount);
            world.trigger(Outfit { member, item });
            world.flush();
        }
        let seen = world
            .run_system_once(move |items: Items| {
                (
                    items.worn(member, EquipSlot::Primary).and_then(|i| items.id(i)),
                    items.worn(member, EquipSlot::Secondary).and_then(|i| items.units(i).into()),
                    items.worn(member, EquipSlot::Misc1).and_then(|i| items.id(i)),
                )
            })
            .unwrap();
        // The second quiver joins the first up to the limit.
        assert_eq!(seen, (Some(BOW), Some(40), Some(RING)));
    }

    /// What a member carries weighs on them, and is worked out when it
    /// changes: a heavy pack makes a member burdened, taking it off eases it.
    #[test]
    fn burden_follows_what_is_carried() {
        let (mut world, member, _) = party();
        let attributes = Attributes::of_sheet(world.get::<Character>(member).unwrap());
        world.entity_mut(member).insert(attributes);
        let quiver = new(&mut world, ARROWS, 100);
        world.trigger(Put { item: quiver, at: Place::Pack { member, slot: 0 } });
        world.flush();
        assert_eq!(world.get::<Burden>(member).unwrap().weight, 200);
        world.trigger(Put {
            item: quiver,
            at: Place::Hand { party: world.get::<PartyMember>(member).unwrap().0 },
        });
        world.flush();
        assert_eq!(world.get::<Burden>(member).unwrap().weight, 0);
    }

    /// Fill every row of the party's pool, so that it takes nothing more.
    fn fill_pool(world: &mut World, party: Entity) {
        for _ in 0..POOL_LIMIT {
            let bow = new(world, BOW, 1);
            world.trigger(Put { item: bow, at: Place::Pool { party, row: 0 } });
        }
        world.flush();
    }

    /// Lay an item on the ground of a level.
    fn lay_down(world: &mut World, item: Entity) -> Entity {
        let level = world.spawn_empty().id();
        world.trigger(Put { item, at: Place::Ground { level, at: Vec3::new(1.0, 2.0, 3.0) } });
        world.flush();
        level
    }

    /// A member whose pack is full, with `arrows` in the first slot.
    fn full_pack(world: &mut World, member: Entity, arrows: u16) -> Entity {
        let quiver = new(world, ARROWS, arrows);
        world.trigger(Put { item: quiver, at: Place::Pack { member, slot: 0 } });
        for slot in 1..PACK_SLOTS {
            let bow = new(world, BOW, 1);
            world.trigger(Put { item: bow, at: Place::Pack { member, slot } });
        }
        world.flush();
        quiver
    }

    /// A stack lying on the ground that only partly fits is partly taken, and
    /// the rest still lies where it was; what does not fit anywhere is told.
    #[test]
    fn a_stack_on_the_ground_is_taken_as_far_as_it_fits() {
        let (mut world, member, party) = party();
        let quiver = full_pack(&mut world, member, 90);
        fill_pool(&mut world, party);
        let lying = new(&mut world, ARROWS, 30);
        let level = lay_down(&mut world, lying);
        world.trigger(Give { item: lying, to: Some(member), party, pool_first: false, told: true });
        world.flush();
        assert_eq!(world.get::<Count>(quiver).unwrap().of, 100);
        assert_eq!(world.get::<Count>(lying).unwrap().of, 20, "the rest is left");
        assert!(world.get::<Lying>(lying).is_some());
        assert_eq!(world.get::<LevelOwned>(lying), Some(&LevelOwned(level)));
        assert_eq!(world.get::<Transform>(lying).unwrap().translation, Vec3::new(1.0, 2.0, 3.0));
        let told: Vec<_> = world.resource_mut::<Messages<ItemNews>>().drain().collect();
        assert_eq!(
            told,
            [
                ItemNews::Takes { member: Some("Vi".to_owned()), item: "Arrows".to_owned() },
                ItemNews::NoRoom
            ]
        );
        // With nothing to take it into, all of it stays.
        let more = new(&mut world, BOW, 1);
        lay_down(&mut world, more);
        world.trigger(Give { item: more, to: Some(member), party, pool_first: false, told: true });
        world.flush();
        assert!(world.get::<Lying>(more).is_some());
    }

    /// An item taken from the ground no longer lies there.
    #[test]
    fn an_item_taken_up_stops_lying() {
        let (mut world, member, party) = party();
        let bow = new(&mut world, BOW, 1);
        lay_down(&mut world, bow);
        world.trigger(Give { item: bow, to: Some(member), party, pool_first: false, told: false });
        world.flush();
        assert!(world.get::<Lying>(bow).is_none() && world.get::<Transform>(bow).is_none());
        assert!(world.get::<LevelOwned>(bow).is_none());
        let pack = world.run_system_once(move |items: Items| items.pack(member)).unwrap();
        assert_eq!(pack[0], Some(bow));
    }

    /// A bag lying in the world is gone once the pool has taken all of it.
    #[test]
    fn a_bag_is_gone_once_it_is_emptied() {
        let (mut world, _, party) = party();
        let bag = world.spawn((Bag, Weight(0))).id();
        lay_down(&mut world, bag);
        let (bow, quiver) = (new(&mut world, BOW, 1), new(&mut world, ARROWS, 5));
        for item in [bow, quiver] {
            world.trigger(Put { item, at: Place::Container(bag) });
        }
        world.flush();
        assert_eq!(world.run_system_once(move |items: Items| items.stowed(bag).len()).unwrap(), 2);
        for item in [bow, quiver] {
            world.trigger(Give { item, to: None, party, pool_first: true, told: true });
        }
        world.flush();
        assert!(world.get_entity(bag).is_err());
        let pool = world.run_system_once(move |items: Items| items.pool(party).to_vec()).unwrap();
        assert_eq!(pool.len(), 2);
    }

    /// The chosen items are taken from a container, a stack in part; what a
    /// member has no room for stays in it, joined to the rest of its stack,
    /// and the looking ends when it is empty.
    #[test]
    fn items_are_taken_from_a_container() {
        use crate::game::world::containers::{Container, Recipient, take_from_containers};
        use crate::game::world::interaction::{
            AttemptOn, DeviceAct, DeviceInteraction, DeviceRequest, Work,
        };
        let (mut world, member, party) = party();
        world.init_resource::<Messages<DeviceRequest>>();
        let chest = world.spawn(Container { table: None, gold: 0 }).id();
        let attempt =
            world.spawn((DeviceInteraction { work: Work::Looting }, AttemptOn(chest))).id();
        let (arrows, bow) = (new(&mut world, ARROWS, 10), new(&mut world, BOW, 1));
        for item in [arrows, bow] {
            world.trigger(Put { item, at: Place::Container(chest) });
        }
        world.flush();
        let take = |world: &mut World, picks: Vec<(Entity, u16)>| {
            world.write_message(DeviceRequest {
                on: attempt,
                act: DeviceAct::Take { picks, to: Recipient::Member(member) },
            });
            world.run_system_once(take_from_containers).unwrap();
            world.resource_mut::<Messages<DeviceRequest>>().clear();
            world.flush();
        };
        let inside = |world: &mut World| {
            world.run_system_once(move |items: Items| items.stowed(chest).to_vec()).unwrap()
        };
        // Part of the stack is taken; the rest is still in the chest.
        take(&mut world, vec![(arrows, 4)]);
        assert_eq!(world.get::<Count>(arrows).unwrap().of, 6);
        let pack = world.run_system_once(move |items: Items| items.pack(member)).unwrap();
        let part = pack[0].expect("the part is in their pack");
        assert_eq!(world.get::<Count>(part).unwrap().of, 4);
        assert_eq!(inside(&mut world), [arrows, bow]);
        // With a full pack and pool, only what joins what they carry goes.
        world.entity_mut(part).insert(Count { of: 97, limit: 100 });
        for slot in 1..PACK_SLOTS {
            let spare = new(&mut world, BOW, 1);
            world.trigger(Put { item: spare, at: Place::Pack { member, slot } });
        }
        fill_pool(&mut world, party);
        take(&mut world, vec![(arrows, 5)]);
        assert_eq!(world.get::<Count>(part).unwrap().of, 100);
        assert_eq!(world.get::<Count>(arrows).unwrap().of, 1 + 2, "what did not fit rejoined");
        assert_eq!(inside(&mut world), [arrows, bow]);
        // A pick that names something not in the chest takes nothing.
        take(&mut world, vec![(part, 1), (bow, 2)]);
        assert_eq!(inside(&mut world), [arrows, bow]);
        // When the last of it is taken the look is over.
        assert!(world.get_entity(attempt).is_ok());
        world.trigger(Put { item: arrows, at: Place::Hand { party } });
        world.trigger(Put { item: bow, at: Place::Pool { party, row: 0 } });
        world.flush();
        assert!(world.get_entity(attempt).is_err());
    }

    /// What is held is thrown to lie where it lands; a crucial item stays in
    /// hand and the party is told.
    #[test]
    fn a_held_item_is_thrown_but_not_a_crucial_one() {
        let (mut world, _, party) = party();
        let level = world.spawn_empty().id();
        let at = Vec3::new(4.0, 0.25, -2.0);
        let (ring, bow) = (new(&mut world, RING, 1), new(&mut world, BOW, 1));
        world.entity_mut(ring).insert(Crucial);
        hold(&mut world, party, ring);
        world.trigger(Throw { party, level, at });
        world.flush();
        assert_eq!(held(&mut world, party), Some(ring));
        assert!(world.get::<Lying>(ring).is_none());
        let told: Vec<_> = world.resource_mut::<Messages<ItemNews>>().drain().collect();
        assert_eq!(told, [ItemNews::CannotDrop]);
        world.trigger(Put { item: ring, at: Place::Pool { party, row: 0 } });
        world.flush();
        hold(&mut world, party, bow);
        world.trigger(Throw { party, level, at });
        world.flush();
        assert_eq!(held(&mut world, party), None);
        assert!(world.get::<Lying>(bow).is_some());
        assert_eq!(world.get::<Transform>(bow).unwrap().translation, at);
        assert_eq!(world.get::<LevelOwned>(bow), Some(&LevelOwned(level)));
    }
}
