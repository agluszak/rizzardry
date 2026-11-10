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

use crate::content::characters::{Attribute, Gender, Profession, Race, Skill};
use crate::content::items::{
    EquipClass, ItemCatalog, ItemDefinition, ItemId, ItemKind, QuantityKind, Recipe,
};
use crate::content::species::Reach;
use crate::game::actors::{Attributes, Refit};
use crate::game::characters::{Character, Practise, Vitals};
use crate::game::combat::Battle;
use crate::game::data::Catalogs;
use crate::game::inventory::EquipSlot;
use crate::game::party::roster::{PartyMember, PartyRoster};
use crate::game::rng::GameRng;

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

/// Somewhere an item can be put.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    if definition.cursed {
        item.insert(Cursed { released: false });
    }
    if let Some(wearable) = wearable(definition) {
        item.insert(wearable);
    }
    item.id()
}

/// Where an item of this definition may be worn, if it is worn at all.
pub fn wearable(definition: &ItemDefinition) -> Option<Wearable> {
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

/// An item that is not in the world as a thing: what a stored character
/// holds, what lies on the ground, in a container, in a monster's loot or
/// in a merchant's hands. The party's own items are entities; this is the
/// value they are converted to and from where they cross into those places
/// ([`unpack`] and [`Items::stack`]), until those become things too.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemStack {
    pub item: ItemId,
    count: u16,
    remaining_uses: Option<u16>,
    pub identified: bool,
    pub spell_known: bool,
    pub curse_known: bool,
}

impl ItemStack {
    /// Import an authored/generated quantity. Legacy zero physical counts mean
    /// one object; zero charges are valid.
    pub fn new(definition: &ItemDefinition, amount: u16, identify: bool) -> Self {
        Self {
            item: definition.id,
            count: if definition.quantity == QuantityKind::Stack { amount.max(1) } else { 1 },
            remaining_uses: match definition.quantity {
                QuantityKind::None | QuantityKind::Stack => None,
                _ => Some(amount),
            },
            identified: identify || definition.auto_identify,
            spell_known: false,
            curse_known: false,
        }
    }

    /// One found or made, its quantity rolled (see [`roll_amount`]).
    pub fn rolled(
        definition: &ItemDefinition,
        rng: &mut GameRng,
        maximum: bool,
        identify: bool,
    ) -> Self {
        Self::new(definition, roll_amount(definition, rng, maximum), identify)
    }

    pub fn count(&self) -> u16 {
        self.count
    }

    pub fn units(&self) -> u32 {
        u32::from(self.count)
    }

    pub fn remaining_uses(&self) -> Option<u16> {
        self.remaining_uses
    }

    pub fn display_name(&self, definition: &ItemDefinition, count: bool) -> String {
        let name = definition.known_name(self.identified);
        if count && self.count > 1 { format!("{name} ({})", self.count) } else { name.to_owned() }
    }

    /// Take `amount` units off this stack as a stack of their own, when that
    /// is a proper part of it.
    pub fn split_off(&mut self, amount: u16) -> Option<Self> {
        if amount == 0 || amount >= self.count {
            return None;
        }
        self.count -= amount;
        Some(Self { count: amount, ..self.clone() })
    }

    /// What is left of this once `taken` of its units have gone, if any.
    pub fn without(mut self, taken: u16) -> Option<Self> {
        if taken >= self.count {
            return None;
        }
        self.count -= taken;
        Some(self)
    }
}

/// An item made from the value that stood for it, nowhere until it is
/// [`Put`] somewhere: what the party takes from the world, a merchant or a
/// record becomes a thing.
pub fn unpack(commands: &mut Commands, definition: &ItemDefinition, stack: &ItemStack) -> Entity {
    let amount = stack.remaining_uses.unwrap_or(stack.count);
    let item = make(commands, definition, amount, stack.identified);
    commands.entity(item).insert(Known {
        identified: stack.identified,
        spell: stack.spell_known,
        curse: stack.curse_known,
    });
    item
}

/// Values come to the party, in order: to a member if one is named, else
/// to the pool (see [`ItemStack`]). Each becomes a thing as far as there is
/// room for it, a proper part of it if not all. How many units of each were
/// taken; the rest are the caller's still.
pub fn receive(
    items: &Items,
    commands: &mut Commands,
    catalog: &ItemCatalog,
    (party, to): (Entity, Option<Entity>),
    (stacks, told): (&[ItemStack], bool),
) -> Vec<u16> {
    let accepts = match to {
        Some(member) => items.pack_accepts(member, stacks),
        None => items.pool_accepts(party, stacks),
    };
    for (stack, &taking) in stacks.iter().zip(&accepts) {
        if taking == 0 {
            continue;
        }
        let mut part = stack.clone();
        if taking < part.count() {
            part = part.split_off(taking).unwrap();
        }
        let item = unpack(commands, &catalog[part.item], &part);
        commands.trigger(Give { item, to, party, pool_first: to.is_none(), told });
    }
    accepts
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
}

impl Items<'_, '_> {
    /// What a member wears in a slot.
    pub fn worn(&self, member: Entity, slot: EquipSlot) -> Option<Entity> {
        let wears = self.wears.get(member).ok()?;
        wears.iter().find(|&item| self.worn.get(item).is_ok_and(|worn| worn.slot == slot))
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

    /// The value that stands for an item where it is not a thing (see
    /// [`ItemStack`]); the item itself is left to be despawned by whoever
    /// moves it.
    pub fn stack(&self, item: Entity, catalog: &ItemCatalog) -> Option<ItemStack> {
        let (&Item(id), count, charges, known, _) = self.items.get(item).ok()?;
        let mut stack = ItemStack::new(&catalog[id], 1, known.identified);
        if let Some(count) = count {
            stack.count = count.of;
        }
        if let Some(charges) = charges {
            stack.remaining_uses = Some(charges.left);
        }
        (stack.spell_known, stack.curse_known) = (known.spell, known.curse);
        Some(stack)
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

    /// How many units of each of these stacks, taken in order, a party's
    /// pool would take: into the rows alike, then a row of its own for what
    /// is left.
    pub fn pool_accepts(&self, party: Entity, stacks: &[ItemStack]) -> Vec<u16> {
        let mut rows = self.pool(party).len();
        let mut used = Vec::new();
        stacks
            .iter()
            .map(|stack| {
                let alike =
                    self.room_among(self.pool(party).iter().copied(), stack.item, stack.identified);
                let taking = Self::fill(&mut used, stack, alike, rows < POOL_LIMIT);
                rows += usize::from(taking == stack.count() && taking > 0 && alike < stack.units());
                taking
            })
            .collect()
    }

    /// How many units of each of these stacks, taken in order, a member
    /// would take: into what they carry alike, then a free pack slot for
    /// what is left.
    pub fn pack_accepts(&self, member: Entity, stacks: &[ItemStack]) -> Vec<u16> {
        let mut free = self.pack(member).iter().filter(|slot| slot.is_none()).count();
        let mut used = Vec::new();
        stacks
            .iter()
            .map(|stack| {
                let alike = self.room_among(self.carried(member), stack.item, stack.identified);
                let taking = Self::fill(&mut used, stack, alike, free > 0);
                free -= usize::from(taking == stack.count() && taking > 0 && alike < stack.units());
                taking
            })
            .collect()
    }

    /// How much of a stack goes where: the room already alike that earlier
    /// stacks have not used, and all of it if there is a place of its own.
    fn fill(
        used: &mut Vec<(ItemId, bool, u32)>,
        stack: &ItemStack,
        alike: u32,
        own_place: bool,
    ) -> u16 {
        let key = (stack.item, stack.identified);
        let at = used.iter().position(|&(item, known, _)| (item, known) == key);
        let spent = at.map_or(0, |at| used[at].2);
        let from_alike = (alike.max(spent) - spent).min(stack.units());
        match at {
            Some(at) => used[at].2 += from_alike,
            None => used.push((key.0, key.1, from_alike)),
        }
        if from_alike < stack.units() && own_place { stack.count() } else { from_alike as u16 }
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
    commands.entity(item).remove::<(Worn, Packed, Pooled, InHand, Stowed)>();
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
    }
    commands.trigger(Reweigh);
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
    mut commands: Commands,
) {
    let Ok((charges, count, spent_by_use, keeps)) = items.get_mut(event.item) else { return };
    let gone = match (charges, count) {
        (Some(mut charges), _) => {
            charges.left -= charges.left.min(event.amount);
            charges.left == 0 && !keeps
        }
        (None, Some(mut count)) if spent_by_use => {
            count.of -= count.of.min(event.amount);
            count.of == 0
        }
        (None, _) => spent_by_use,
    };
    if gone {
        commands.entity(event.item).despawn();
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
    news.write(match taker {
        Some(member) => ItemNews::Takes {
            member: member.and_then(|member| names.get(member).ok()).map(|c| c.name.clone()),
            item: name,
        },
        None => ItemNews::NoRoom,
    });
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
        Place::Hand { .. } | Place::Container(_) => Err(Refusal::Nothing),
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

/// What the party is told of items coming to it and being put together.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum ItemNews {
    Takes {
        member: Option<String>,
        item: String,
    },
    NoRoom,
    /// An item the party may not part with stays in hand instead of being
    /// dropped (retail string 0x4ef).
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
        .add_observer(load)
        .add_observer(spend)
        .add_observer(lose)
        .add_observer(give)
        .add_observer(swap)
        .add_observer(pass)
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
            Observer::new(load),
            Observer::new(spend),
            Observer::new(lose),
            Observer::new(give),
            Observer::new(swap),
            Observer::new(pass),
            Observer::new(merge),
            Observer::new(identify),
            Observer::new(outfit),
            Observer::new(reweigh),
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
}
