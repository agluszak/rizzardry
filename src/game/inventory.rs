//! Owned nonempty stacks, concrete containers, and synchronous inventory moves.
//! Equipment release belongs to a placement, never to the transferable object.
pub mod merge;
pub mod omnigun;
pub mod operations;

use super::characters::{Character, Load, Vitals};
use super::party::roster::PartyRoster;
use super::rng::GameRng;
use crate::content::characters::Skill;
use crate::content::items::{
    EquipClass, ItemCatalog, ItemDefinition, ItemId, ItemKind, QuantityKind,
};
use bevy::prelude::*;
use enum_map::{Enum, EnumMap};
use enumset::EnumSetType;
use std::num::NonZeroU16;

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<ItemIds>()
        .add_message::<ItemNews>()
        .add_observer(operations::put_away_held)
        .add_systems(
            Update,
            distribute_burden
                .run_if(not(any_with_component::<super::combat::Battle>))
                .in_set(super::GameplaySet::Burden)
                .run_if(super::world::level::level_active),
        );
}

/// Identifies a surviving stack, not each physical object within it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StackId(pub u64);

#[derive(Resource, Clone, Debug, Default)]
pub struct ItemIds {
    next: u64,
}
impl ItemIds {
    pub fn allocate(&mut self) -> StackId {
        self.next += 1;
        StackId(self.next)
    }
    /// The identity the next allocation will return.
    fn upcoming(&self) -> StackId {
        StackId(self.next + 1)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ItemStack {
    pub id: StackId,
    pub item: ItemId,
    count: NonZeroU16,
    remaining_uses: Option<u16>,
    pub identified: bool,
    pub spell_known: bool,
    pub curse_known: bool,
}
impl ItemStack {
    /// Import an authored/generated quantity. Legacy zero physical counts mean
    /// one object; zero charges are valid. Overfull physical stacks are retained.
    pub fn new(
        definition: &ItemDefinition,
        ids: &mut ItemIds,
        amount: u16,
        identify: bool,
    ) -> Self {
        Self {
            id: ids.allocate(),
            item: definition.id,
            count: NonZeroU16::new(if definition.quantity == QuantityKind::Stack {
                amount.max(1)
            } else {
                1
            })
            .unwrap(),
            remaining_uses: match definition.quantity {
                QuantityKind::None | QuantityKind::Stack => None,
                _ => Some(amount),
            },
            identified: identify || definition.auto_identify,
            spell_known: false,
            curse_known: false,
        }
    }
    pub fn count(&self) -> u16 {
        self.count.get()
    }
    pub fn units(&self) -> u32 {
        u32::from(self.count())
    }
    pub fn remaining_uses(&self) -> Option<u16> {
        self.remaining_uses
    }
    pub fn display_name(&self, definition: &ItemDefinition, count: bool) -> String {
        let name = definition.known_name(self.identified);
        if count && self.count() > 1 {
            format!("{name} ({})", self.count())
        } else {
            name.to_owned()
        }
    }
    /// Offer a proper part of this stack to a receiver, which returns what it
    /// refuses and whether it kept the part as a stack of its own. Refused
    /// units come back here, so a refusal changes nothing; only a part kept
    /// as its own stack takes an identity from `ids`. Returns the units moved.
    pub fn give_part(
        &mut self,
        amount: u16,
        ids: &mut ItemIds,
        receive: impl FnOnce(ItemStack) -> (Option<ItemStack>, bool),
    ) -> u16 {
        if amount == 0 || amount >= self.count() {
            return 0;
        }
        let mut part = self.clone();
        part.id = ids.upcoming();
        part.count = NonZeroU16::new(amount).unwrap();
        self.count = NonZeroU16::new(self.count() - amount).unwrap();
        let (rest, kept) = receive(part);
        if kept {
            ids.allocate();
        }
        let refused = rest.map_or(0, |rest| rest.count());
        self.count = NonZeroU16::new(self.count() + refused).unwrap();
        amount - refused
    }
    /// Consume up to `amount` physical units, retaining the remainder's identity
    /// and metadata. Callers that settle payment validate the exact quantity first.
    pub(crate) fn remove_units(mut self, amount: u16) -> Option<Self> {
        self.count = NonZeroU16::new(self.count() - amount.min(self.count()))?;
        Some(self)
    }
    /// Units of this definition that can join the stack without changing its metadata.
    pub(crate) fn room_for(&self, definition: &ItemDefinition, identified: bool) -> u16 {
        if self.item == definition.id
            && self.identified == identified
            && definition.quantity == QuantityKind::Stack
            && self.count() < u16::from(definition.stack_limit)
        {
            u16::from(definition.stack_limit) - self.count()
        } else {
            0
        }
    }
    /// Ordinary absorption retains destination metadata and source remainder identity.
    pub fn absorb(&mut self, mut source: Self, definition: &ItemDefinition) -> Option<Self> {
        let moved = source.count().min(self.room_for(definition, source.identified));
        self.count = NonZeroU16::new(self.count() + moved).unwrap();
        source.count = NonZeroU16::new(source.count() - moved)?;
        Some(source)
    }
    /// Load this item from another's uses or units, up to as many as it can
    /// hold (retail `MergeItemUses`), spending them from the other one at a
    /// time. A gun takes the shots its ammunition holds. Only items counting
    /// uses can be loaded.
    pub fn load(
        &mut self,
        from: Self,
        definition: &ItemDefinition,
        source: &ItemDefinition,
    ) -> Loaded {
        let Some(uses) = self.remaining_uses else {
            return Loaded { charges: 0, rest: Some(from) };
        };
        let available = match source.quantity {
            QuantityKind::Stack => from.count(),
            _ => from.remaining_uses.unwrap_or(0),
        };
        let maximum = definition.initial_quantity.maximum().min(u32::from(u16::MAX)) as u16;
        // An overfull item has no room; loading never increases it further.
        let moved = available.min(maximum.max(uses) - uses);
        self.remaining_uses = Some(uses + moved);
        let mut rest = Some(from);
        for _ in 0..moved {
            rest = rest.and_then(|from| from.consume_use(source, source.keeps_empty()));
        }
        Loaded { charges: moved, rest }
    }
    /// Consumption owns its input: exhausted physical stacks never survive as values.
    pub fn consume_use(mut self, definition: &ItemDefinition, keep_empty: bool) -> Option<Self> {
        if let Some(uses) = &mut self.remaining_uses {
            *uses -= (*uses).min(1);
            if *uses == 0 && !keep_empty {
                return None;
            }
        } else if definition.quantity == QuantityKind::Stack
            || definition.class == EquipClass::Drink
        {
            self.count = NonZeroU16::new(self.count() - 1)?;
        }
        Some(self)
    }
}

pub fn make_item(
    definition: &ItemDefinition,
    ids: &mut ItemIds,
    rng: &mut GameRng,
    maximum: bool,
    identify: bool,
) -> ItemStack {
    let amount = if definition.quantity == QuantityKind::None {
        1
    } else if maximum {
        definition.initial_quantity.maximum()
    } else {
        rng.roll(definition.initial_quantity)
    };
    ItemStack::new(definition, ids, amount.min(u32::from(u8::MAX)) as u16, identify)
}

/// Apply absorption in the container's policy order. Empty placement is separate.
fn fill<'a>(
    targets: impl Iterator<Item = &'a mut ItemStack>,
    mut incoming: ItemStack,
    catalog: &ItemCatalog,
) -> Option<ItemStack> {
    let definition = &catalog[incoming.item];
    for target in targets {
        incoming = target.absorb(incoming, definition)?;
    }
    Some(incoming)
}

/// Where an item is worn, in retail's slot order.
#[derive(Debug, Hash, Enum, EnumSetType)]
pub enum EquipSlot {
    Head,
    Misc1,
    Misc2,
    Cloak,
    Torso,
    Hands,
    /// Main hand of the weapon set in use.
    Primary,
    /// Off hand of the weapon set in use.
    Secondary,
    /// The stored weapon set; not worn, so curses do not hold there.
    AlternatePrimary,
    AlternateSecondary,
    Legs,
    Feet,
}

impl EquipSlot {
    pub fn is_active(self) -> bool {
        !matches!(self, Self::AlternatePrimary | Self::AlternateSecondary)
    }

    /// The other hand of the same weapon set.
    pub fn partner(self) -> Option<EquipSlot> {
        Some(match self {
            EquipSlot::Primary => EquipSlot::Secondary,
            EquipSlot::Secondary => EquipSlot::Primary,
            EquipSlot::AlternatePrimary => EquipSlot::AlternateSecondary,
            EquipSlot::AlternateSecondary => EquipSlot::AlternatePrimary,
            _ => return None,
        })
    }
}

pub const BACKPACK_SLOTS: usize = 8;
pub const POOL_LIMIT: usize = 500;

#[derive(Clone, Debug, PartialEq)]
pub struct Equipped {
    pub stack: ItemStack,
    pub curse_released: bool,
}
impl Equipped {
    pub fn new(stack: ItemStack) -> Self {
        Self { stack, curse_released: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Equipment(EquipSlot),
    Backpack(usize),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemberSlot {
    pub member: Entity,
    pub slot: Slot,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemOwner {
    Member(Entity),
    Pool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemRef {
    pub owner: ItemOwner,
    pub id: StackId,
}
/// What loading an item came to: the charges it took, and what is left of
/// the source, which may still be whole when it holds charges.
#[derive(Debug, PartialEq)]
pub struct Loaded {
    pub charges: u16,
    pub rest: Option<ItemStack>,
}

/// An item within the reviewed member's reach: the hand, one of their
/// slots, or a row of the party's pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemAt {
    Hand,
    Slot(Slot),
    Pool(StackId),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolPosition {
    Front,
    Before(StackId),
    End,
}

#[derive(Component, Clone, Debug, Default, PartialEq)]
#[require(Burden)]
pub struct Carried {
    pub equipment: EnumMap<EquipSlot, Option<Equipped>>,
    pub backpack: [Option<ItemStack>; BACKPACK_SLOTS],
}
impl Carried {
    pub fn active_equipment(&self) -> impl Iterator<Item = (EquipSlot, &Equipped)> {
        self.equipment
            .iter()
            .filter(|(slot, _)| slot.is_active())
            .filter_map(|(slot, equipped)| equipped.as_ref().map(|equipped| (slot, equipped)))
    }

    pub fn worn(&self, slot: EquipSlot) -> Option<&ItemStack> {
        self.equipment[slot].as_ref().map(|e| &e.stack)
    }
    pub fn slot(&self, slot: Slot) -> Option<&ItemStack> {
        match slot {
            Slot::Equipment(s) => self.worn(s),
            Slot::Backpack(i) => self.backpack.get(i)?.as_ref(),
        }
    }
    fn stack_mut(&mut self, slot: Slot) -> Option<&mut ItemStack> {
        match slot {
            Slot::Equipment(s) => self.equipment[s].as_mut().map(|e| &mut e.stack),
            Slot::Backpack(i) => self.backpack.get_mut(i)?.as_mut(),
        }
    }
    fn take(&mut self, slot: Slot) -> Option<ItemStack> {
        match slot {
            Slot::Equipment(s) => self.equipment[s].take().map(|e| e.stack),
            Slot::Backpack(i) => self.backpack.get_mut(i)?.take(),
        }
    }
    /// A partial extraction preserves the equipment placement; whole removal
    /// destroys it. Receivers return the remainder still owned by this slot.
    fn offer_slot(&mut self, slot: Slot, receiver: impl FnOnce(ItemStack) -> Option<ItemStack>) {
        match slot {
            Slot::Equipment(s) => {
                if let Some(Equipped { stack, curse_released }) = self.equipment[s].take() {
                    self.equipment[s] =
                        receiver(stack).map(|stack| Equipped { stack, curse_released });
                }
            }
            Slot::Backpack(i) => {
                if let Some(stack) = self.backpack[i].take() {
                    self.backpack[i] = receiver(stack);
                }
            }
        }
    }
    fn put(&mut self, slot: Slot, stack: Option<ItemStack>) {
        match slot {
            Slot::Equipment(s) => self.equipment[s] = stack.map(Equipped::new),
            Slot::Backpack(i) => self.backpack[i] = stack,
        }
    }
    fn find(&self, id: StackId) -> Option<Slot> {
        self.equipment
            .iter()
            .find_map(|(slot, e)| {
                e.as_ref().filter(|e| e.stack.id == id).map(|_| Slot::Equipment(slot))
            })
            .or_else(|| {
                self.backpack
                    .iter()
                    .position(|s| s.as_ref().is_some_and(|s| s.id == id))
                    .map(Slot::Backpack)
            })
    }
    /// A carried stack by identity, worn or in the backpack.
    pub fn find_stack(&self, id: StackId) -> Option<&ItemStack> {
        self.slot(self.find(id)?)
    }
    /// Edit metadata of an existing stack without moving or rebinding it.
    pub(crate) fn find_stack_mut(&mut self, id: StackId) -> Option<&mut ItemStack> {
        self.stack_mut(self.find(id)?)
    }
    /// Trade and other unworn-item operations cannot extract equipment.
    pub fn backpack_stack(&self, id: StackId) -> Option<&ItemStack> {
        self.backpack.iter().flatten().find(|stack| stack.id == id)
    }
    /// Spend `amount` units of a backpack stack.
    pub fn consume_backpack_units(&mut self, id: StackId, amount: u16) {
        let at = self.backpack.iter().position(|s| s.as_ref().is_some_and(|s| s.id == id)).unwrap();
        self.backpack[at] = self.backpack[at].take().unwrap().remove_units(amount);
    }
    /// Spend one use of a carried stack (retail `RemoveCharacterItem`): an
    /// exhausted stack leaves its slot unless `keep_empty`. The caller owns
    /// a live stack resolved in the same operation.
    pub fn consume_use(&mut self, id: StackId, definition: &ItemDefinition, keep_empty: bool) {
        let slot = self.find(id).unwrap();
        self.offer_slot(slot, |stack| stack.consume_use(definition, keep_empty));
    }
    pub fn stacks(&self) -> impl Iterator<Item = &ItemStack> {
        self.equipment.values().flatten().map(|e| &e.stack).chain(self.backpack.iter().flatten())
    }
    fn stacks_mut(&mut self) -> impl Iterator<Item = &mut ItemStack> {
        self.equipment
            .values_mut()
            .flatten()
            .map(|e| &mut e.stack)
            .chain(self.backpack.iter_mut().flatten())
    }
    /// Worn stacks, backpack stacks, then the first empty backpack slot.
    pub fn receive(&mut self, incoming: ItemStack, catalog: &ItemCatalog) -> Option<ItemStack> {
        let rest = fill(self.stacks_mut(), incoming, catalog)?;
        match self.backpack.iter_mut().find(|s| s.is_none()) {
            Some(empty) => {
                *empty = Some(rest);
                None
            }
            None => Some(rest),
        }
    }
    fn check_removal(
        &mut self,
        slot: Slot,
        catalog: &ItemCatalog,
        dead: bool,
    ) -> Result<(), Refusal> {
        if let Slot::Equipment(s) = slot {
            let Some(e) = self.equipment[s].as_mut() else { return Ok(()) };
            let d = &catalog[e.stack.item];
            if d.cursed && !e.curse_released && !dead && s.is_active() {
                e.stack.curse_known = true;
                return Err(Refusal::Cursed);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemPool {
    pub stacks: Vec<ItemStack>,
}
impl ItemPool {
    /// Capacity follows the same absorption rule as insertion, plus empty rows.
    pub(crate) fn room_for(&self, definition: &ItemDefinition, identified: bool) -> u32 {
        let per_row = if definition.quantity == QuantityKind::Stack {
            u32::from(definition.stack_limit).max(1)
        } else {
            1
        };
        self.stacks.iter().map(|s| u32::from(s.room_for(definition, identified))).sum::<u32>()
            + (POOL_LIMIT - self.stacks.len()) as u32 * per_row
    }

    fn position(&self, position: PoolPosition) -> Result<usize, Refusal> {
        match position {
            PoolPosition::Front => Ok(0),
            PoolPosition::End => Ok(self.stacks.len()),
            PoolPosition::Before(id) => {
                self.stacks.iter().position(|s| s.id == id).ok_or(Refusal::Nothing)
            }
        }
    }
    pub fn receive(&mut self, incoming: ItemStack, catalog: &ItemCatalog) -> Option<ItemStack> {
        self.deposit(incoming, PoolPosition::Front, catalog)
    }
    /// Missing anchors reject without mutation; overflow is an ordinary remainder.
    pub fn deposit(
        &mut self,
        incoming: ItemStack,
        position: PoolPosition,
        catalog: &ItemCatalog,
    ) -> Option<ItemStack> {
        let Ok(index) = self.position(position) else { return Some(incoming) };
        let mut rest = incoming;
        if let Some(target) = self.stacks.get_mut(index) {
            rest = fill(std::iter::once(target), rest, catalog)?;
        }
        rest = fill(
            self.stacks.iter_mut().enumerate().filter(|(i, _)| *i != index).map(|(_, s)| s),
            rest,
            catalog,
        )?;
        if self.stacks.len() >= POOL_LIMIT {
            return Some(rest);
        }
        self.stacks.insert(index, rest);
        None
    }
    /// Drain owned contents through the same pool-transfer rules for chests and
    /// world bags. Partial stack merges are reported; every remainder stays owned
    /// by the source, in its original order.
    pub fn collect(
        &mut self,
        contents: &mut Vec<ItemStack>,
        catalog: &ItemCatalog,
    ) -> Vec<(String, u32)> {
        let mut taken = Vec::new();
        let mut remaining = Vec::new();
        for item in contents.drain(..) {
            let before = item.units();
            let name = item.display_name(&catalog[item.item], false);
            let after = match self.receive(item, catalog) {
                None => 0,
                Some(item) => {
                    let units = item.units();
                    remaining.push(item);
                    units
                }
            };
            if before > after {
                taken.push((name, before - after));
            }
        }
        *contents = remaining;
        taken
    }

    /// Spend a use from the same shared row, retaining its identity if it survives.
    pub fn consume_use(&mut self, id: StackId, definition: &ItemDefinition) {
        let at = self.stacks.iter().position(|stack| stack.id == id).unwrap();
        if let Some(remaining) =
            self.stacks.remove(at).consume_use(definition, definition.keeps_empty())
        {
            self.stacks.insert(at, remaining);
        }
    }

    /// Consume physical units while preserving the remainder's identity and row.
    pub fn consume_units(&mut self, id: StackId, amount: u16) {
        let at = self.stacks.iter().position(|s| s.id == id).unwrap();
        if let Some(rest) = self.stacks.remove(at).remove_units(amount) {
            self.stacks.insert(at, rest);
        }
    }
    pub fn take(&mut self, id: StackId) -> Option<ItemStack> {
        let index = self.stacks.iter().position(|s| s.id == id)?;
        Some(self.stacks.remove(index))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hand {
    pub held: Option<ItemStack>,
}
impl Hand {
    /// The hand owns every rejected remainder, including after partial receipt.
    fn offer(&mut self, receiver: impl FnOnce(ItemStack) -> Option<ItemStack>) -> u32 {
        let Some(held) = self.held.take() else { return 0 };
        let before = held.units();
        self.held = receiver(held);
        before - self.held.as_ref().map_or(0, |h| h.units())
    }
}
/// On the party: what it shares, the pool, the item in hand and the purse.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct PartyInventory {
    pub pool: ItemPool,
    pub hand: Hand,
    pub gold: u32,
}
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
    /// The member lacks the skill making it takes.
    Unskilled {
        member: String,
    },
    /// A Gadgeteer was given their omnigun (tier 0) or it reached a tier
    /// (1 to 11 above the first).
    Omnigun {
        member: String,
        gender: crate::content::characters::Gender,
        tier: usize,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Given {
    Member,
    Pool,
    NoRoom,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Nothing,
    Crucial,
    Cursed,
    DoesNotFit,
    HandFull,
    NoRoom,
}

/// Which equipment slots an item may go in, given what the hands hold
/// (retail `GetItemEquipSlotMask`).
pub fn allowed_slots(
    definition: &ItemDefinition,
    carried: &Carried,
    catalog: &ItemCatalog,
    vacating: &[EquipSlot],
) -> Vec<EquipSlot> {
    use EquipClass::*;
    // What is worn, but for the slot being emptied for this item.
    let worn = |slot: EquipSlot| carried.worn(slot).filter(|_| !vacating.contains(&slot));
    let empty = |slot: EquipSlot| worn(slot).is_none();
    // A main hand also counts as free for the off hand when its weapon is
    // one-handed.
    let main_free = |slot: EquipSlot| worn(slot).is_none_or(|i| !catalog[i.item].two_handed);
    match definition.class {
        ShortWeapon | ExtendedWeapon | ThrownWeapon | RangedWeapon
            if definition.weapon_skill.is_none() =>
        {
            vec![]
        }
        ShortWeapon | ExtendedWeapon | ThrownWeapon | RangedWeapon if definition.two_handed => {
            let mut slots = vec![];
            if empty(EquipSlot::Secondary) {
                slots.push(EquipSlot::Primary);
            }
            if empty(EquipSlot::AlternateSecondary) {
                slots.push(EquipSlot::AlternatePrimary);
            }
            slots
        }
        ShortWeapon | ExtendedWeapon | ThrownWeapon | RangedWeapon => {
            let mut slots = vec![EquipSlot::Primary, EquipSlot::AlternatePrimary];
            if definition.off_hand {
                if main_free(EquipSlot::Primary) {
                    slots.push(EquipSlot::Secondary);
                }
                if main_free(EquipSlot::AlternatePrimary) {
                    slots.push(EquipSlot::AlternateSecondary);
                }
            }
            slots
        }
        Ammunition | Shield => {
            let mut slots = vec![];
            if main_free(EquipSlot::Primary) {
                slots.push(EquipSlot::Secondary);
            }
            if main_free(EquipSlot::AlternatePrimary) {
                slots.push(EquipSlot::AlternateSecondary);
            }
            slots
        }
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

/// Whether two items can share a weapon set (retail `CanHoldItemsTogether`):
/// nothing pairs with a two-handed item, a ranged weapon or ammunition by
/// what each takes, and two weapons by their reach.
pub fn can_hold_together(a: &ItemDefinition, b: &ItemDefinition) -> bool {
    use EquipClass::*;
    if a.two_handed || b.two_handed {
        return false;
    }
    if a.class == RangedWeapon || b.class == Ammunition {
        return partners(a, b);
    }
    if b.class == RangedWeapon || a.class == Ammunition {
        return partners(b, a);
    }
    if u8::from(a.class) >= u8::from(Ammunition) {
        return true;
    }
    u8::from(b.class) >= u8::from(Ammunition) || a.reach == b.reach
}

/// What one member's go at identifying an item came to.
pub(crate) struct Identified {
    /// Far enough above the item's difficulty to tell whether it binds.
    pub binding: bool,
    /// Only just made, which practises Artifacts.
    pub practises: bool,
}

/// One member's go at identifying an item of this difficulty (retail
/// `TryIdentifyItemFor`): someone alive and sane manages it when a sixth of
/// their Artifacts reaches it. Higher difficulty never becomes easier
/// by overflowing a signed byte.
pub(crate) fn identify_attempt(
    character: &Character,
    vitals: &Vitals,
    difficulty: u8,
) -> Option<Identified> {
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

/// Whether `other` may be held with the ranged weapon `ranged` (retail
/// `CompatiblePartnerItems`): bows, crossbows and slings with their
/// missiles, guns with what loads them, and each omnigun tier with what it
/// has learnt to fire.
pub(crate) fn partners(ranged: &ItemDefinition, other: &ItemDefinition) -> bool {
    use ItemKind as K;
    let kind = other.kind;
    match ranged.kind {
        K::LONGBOW | K::GREATBOW => kind == K::ARROW,
        K::CROSSBOW => kind == K::QUARREL,
        K::SLING => kind == K::SLING_STONE,
        K::POWER_GLOVE | K::PHASER | K::MUSKET => {
            (u8::from(other.class) > u8::from(EquipClass::RangedWeapon)
                || ranged.reach == other.reach)
                && loaded_by(ranged.kind).is_none_or(|loads| loads == kind)
        }
        K::OMNIGUN => {
            let dagger = other.class == EquipClass::ThrownWeapon && kind == K::DAGGER;
            match ranged.id.0 {
                599 => matches!(kind, K::SLING_STONE | K::POWDER_SHOT),
                600 | 601 => matches!(kind, K::SLING_STONE | K::POWDER_SHOT | K::DARTS),
                602..=605 => {
                    matches!(
                        kind,
                        K::SLING_STONE
                            | K::POWDER_SHOT
                            | K::DARTS
                            | K::SHURIKEN
                            | K::THROWING_SHURIKEN
                    ) || dagger
                }
                606..=610 => {
                    matches!(
                        kind,
                        K::SLING_STONE
                            | K::POWDER_SHOT
                            | K::DARTS
                            | K::SHURIKEN
                            | K::THROWING_SHURIKEN
                            | K::ARROW
                            | K::QUARREL
                    ) || dagger
                }
                _ => false,
            }
        }
        _ => false,
    }
}

/// Starting equipment may equip; ordinary receipt never does.
pub fn place_at_creation(
    carried: &mut Carried,
    item: ItemStack,
    catalog: &ItemCatalog,
    member: &Character,
) -> Option<ItemStack> {
    let mut item = item;
    let d = &catalog[item.item];
    if member.can_use(d)
        && let Some(slot) = creation_slot(d, carried, catalog, member)
        && allowed_slots(d, carried, catalog, &[]).contains(&slot)
    {
        match &mut carried.equipment[slot] {
            None => {
                carried.equipment[slot] = Some(Equipped::new(item));
                return None;
            }
            Some(worn) => {
                item = worn.stack.absorb(item, d)?;
            }
        }
    }
    carried.receive(item, catalog)
}
pub fn item_weight(item: &ItemStack, catalog: &ItemCatalog) -> u32 {
    u32::from(catalog[item.item].weight) * item.units()
}
pub fn carried_weight(carried: &Carried, catalog: &ItemCatalog) -> u32 {
    carried.stacks().map(|i| item_weight(i, catalog)).sum()
}
/// Where creation puts an item (retail `GetItemDefaultEquipSlot` with the
/// creation adjustments of `ChooseCharacterEquipSlot`). Moving a worn
/// weapon to the off hand to make room is not modelled.
fn creation_slot(
    definition: &ItemDefinition,
    carried: &Carried,
    catalog: &ItemCatalog,
    member: &Character,
) -> Option<EquipSlot> {
    use EquipClass::*;
    let empty = |slot| carried.worn(slot).is_none();
    let ranged_in = |slot| {
        carried
            .worn(slot)
            .map(|i| &catalog[i.item])
            .filter(|d| d.class == RangedWeapon && can_hold_together(d, definition))
            .is_some()
    };
    Some(match definition.class {
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
/// `total_carried_weight` and `load_category`), as the last redistribution
/// left it.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Burden {
    /// Tenths of a pound.
    pub weight: u32,
    pub load: Load,
}

/// Share the pool's weight among the members and settle each member's load
/// (retail `RedistributePartyEncumbrance`). It does not run in a fight, so a
/// fight keeps the loads it began with until it ends.
pub fn distribute_burden(
    party: Single<(Ref<PartyRoster>, Ref<PartyInventory>)>,
    mut members: Query<(Ref<Character>, Ref<Vitals>, Ref<Carried>, &mut Burden)>,
    content: If<Res<crate::game::data::Catalogs>>,
) {
    let (roster, inventory) = party.into_inner();
    let catalog = &content.items;
    let changed = roster.is_changed()
        || inventory.is_changed()
        || members
            .iter_many(roster.iter())
            .matched()
            .any(|(c, v, carried, _)| c.is_changed() || v.is_changed() || carried.is_changed());
    if !changed {
        return;
    }
    let own: Vec<_> = members
        .iter_many(roster.iter())
        .matched()
        .map(|(character, vitals, carried, _)| {
            (
                carried_weight(&carried, catalog),
                (!vitals.is_dead()).then(|| character.stats(&vitals.enchantments).capacity()),
            )
        })
        .collect();
    let pool = inventory.pool.stacks.iter().map(|item| item_weight(item, catalog)).sum();
    let eligible: Vec<_> =
        own.iter().map(|&(weight, capacity)| capacity.map(|c| (weight, c))).collect();
    let shares = pool_shares(pool, &eligible);
    let members = members.iter_many_unique_mut(roster.iter()).matched();
    for ((character, vitals, _, mut burden), (&(own, _), share)) in
        members.zip(own.iter().zip(shares))
    {
        let weight = own + share;
        burden.set_if_neq(Burden {
            weight,
            load: character.stats(&vitals.enchantments).load(weight),
        });
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
        (capacity as i32 - own as i32 - share as i32) as f32 * 100.0 / capacity as f32
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

impl PartyInventory {
    /// A stack the party holds by identity: in hand or in the pool.
    pub fn find_stack(&self, id: StackId) -> Option<&ItemStack> {
        self.hand.held.iter().chain(&self.pool.stacks).find(|s| s.id == id)
    }

    /// Edit metadata of a stack the party holds without moving it.
    pub(crate) fn find_stack_mut(&mut self, id: StackId) -> Option<&mut ItemStack> {
        self.hand.held.iter_mut().chain(&mut self.pool.stacks).find(|s| s.id == id)
    }

    pub fn give_held(
        &mut self,
        member: Option<&mut Carried>,
        pool_first: bool,
        catalog: &ItemCatalog,
    ) -> Option<Given> {
        self.hand.held.as_ref()?;
        let mut given = Given::NoRoom;
        if pool_first && self.hand.offer(|item| self.pool.receive(item, catalog)) > 0 {
            given = Given::Pool;
        }
        if let Some(member) = member
            && self.hand.offer(|item| member.receive(item, catalog)) > 0
        {
            given = Given::Member;
        }
        if !pool_first && self.hand.offer(|item| self.pool.receive(item, catalog)) > 0 {
            given = Given::Pool;
        }
        Some(given)
    }
    /// Load a pool row from the held ammunition when it is that row's gun's
    /// (retail `MergeItemUses` from the pool), returning the charges it took;
    /// none when the held item does not load it.
    pub fn load_row(&mut self, id: StackId, catalog: &ItemCatalog) -> Option<u16> {
        let held = self.hand.held.as_ref()?;
        let gun = self.pool.stacks.iter_mut().find(|s| s.id == id)?;
        let (source, definition) = (&catalog[held.item], &catalog[gun.item]);
        if loaded_by(definition.kind) != Some(source.kind) {
            return None;
        }
        let Loaded { charges, rest } = gun.load(self.hand.held.take()?, definition, source);
        self.hand.held = rest;
        Some(charges)
    }
    pub fn take_from_pool(&mut self, id: StackId) -> Result<(), Refusal> {
        if self.hand.held.is_some() {
            return Err(Refusal::HandFull);
        }
        let item = self.pool.take(id).ok_or(Refusal::Nothing)?;
        self.hand.held = Some(item);
        Ok(())
    }
    pub fn deposit_hand(
        &mut self,
        position: PoolPosition,
        catalog: &ItemCatalog,
    ) -> Result<u32, Refusal> {
        self.pool.position(position)?;
        if self.hand.held.is_none() {
            return Err(Refusal::Nothing);
        }
        let pool = &mut self.pool;
        Ok(self.hand.offer(|item| pool.deposit(item, position, catalog)))
    }
    /// Merge or exchange a concrete member slot. Compound weapon displacement
    /// checks destination room before changing either hand; no rollback copies.
    pub fn interact_slot(
        &mut self,
        character: &Character,
        vitals: &Vitals,
        carried: &mut Carried,
        slot: Slot,
        catalog: &ItemCatalog,
    ) -> Result<(), Refusal> {
        let current = carried.slot(slot);
        if current.is_none() && self.hand.held.is_none() {
            return Err(Refusal::Nothing);
        }
        if let Some((held, target)) = self.hand.held.as_ref().zip(current) {
            let d = &catalog[held.item];
            if held.item == target.item && d.quantity == QuantityKind::Stack {
                if matches!(slot, Slot::Equipment(_)) && !character.can_use(d) {
                    return Err(Refusal::DoesNotFit);
                }
                let target = carried.stack_mut(slot).unwrap();
                return if self.hand.offer(|item| target.absorb(item, d)) > 0 {
                    Ok(())
                } else {
                    Err(Refusal::DoesNotFit)
                };
            }
        }
        carried.check_removal(slot, catalog, vitals.is_dead())?;
        // Ammunition held over its gun loads it.
        if let Some((held, target)) = self.hand.held.as_ref().zip(carried.slot(slot)) {
            let source = &catalog[held.item];
            let definition = &catalog[target.item];
            if loaded_by(definition.kind) == Some(source.kind) {
                if matches!(slot, Slot::Equipment(_)) && !character.can_use(source) {
                    return Err(Refusal::DoesNotFit);
                }
                let target = carried.stack_mut(slot).unwrap();
                let held = self.hand.held.take().unwrap();
                self.hand.held = target.load(held, definition, source).rest;
                return Ok(());
            }
        }
        if let (Slot::Equipment(equip), Some(held)) = (slot, self.hand.held.as_ref()) {
            let definition = &catalog[held.item];
            if !character.can_use(definition) {
                return Err(Refusal::DoesNotFit);
            }
            let displaced = equip.partner().filter(|p| {
                carried
                    .worn(*p)
                    .is_some_and(|partner| !can_hold_together(definition, &catalog[partner.item]))
            });
            if let Some(partner) = displaced {
                carried.check_removal(Slot::Equipment(partner), catalog, vitals.is_dead())?;
                if !allowed_slots(definition, carried, catalog, &[equip, partner]).contains(&equip)
                {
                    return Err(Refusal::DoesNotFit);
                }
                let displaced = carried.worn(partner).unwrap();
                let displaced_definition = &catalog[displaced.item];
                // Both weapon slots leave before receipt, so neither can absorb the partner.
                let room = carried
                    .equipment
                    .iter()
                    .filter(|(s, _)| *s != equip && *s != partner)
                    .filter_map(|(_, e)| e.as_ref().map(|e| &e.stack))
                    .chain(carried.backpack.iter().flatten())
                    .map(|s| u32::from(s.room_for(displaced_definition, displaced.identified)))
                    .sum::<u32>()
                    + self.pool.room_for(displaced_definition, displaced.identified);
                if !carried.backpack.iter().any(Option::is_none)
                    && self.pool.stacks.len() >= POOL_LIMIT
                    && room < displaced.units()
                {
                    return Err(Refusal::NoRoom);
                }
                let partner_item = carried.take(Slot::Equipment(partner)).unwrap();
                let current = carried.take(slot);
                let _ = carried
                    .receive(partner_item, catalog)
                    .and_then(|rest| self.pool.receive(rest, catalog));
                carried.put(slot, self.hand.held.take());
                self.hand.held = current;
                return Ok(());
            }
            if !allowed_slots(definition, carried, catalog, &[equip]).contains(&equip) {
                return Err(Refusal::DoesNotFit);
            }
        }
        let current = carried.take(slot);
        carried.put(slot, self.hand.held.take());
        self.hand.held = current;
        Ok(())
    }
}
