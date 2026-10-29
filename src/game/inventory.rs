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

/// Identifies a surviving stack, not each physical object within it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StackId(pub u64);

#[derive(Resource, Clone, Debug, Default)]
pub struct ItemIds {
    next: u64,
}
impl ItemIds {
    pub fn allocate(&mut self) -> StackId {
        self.next = self.next.checked_add(1).expect("stack identity exhausted");
        StackId(self.next)
    }
    /// The identity the next allocation will return.
    fn upcoming(&self) -> StackId {
        StackId(self.next.checked_add(1).expect("stack identity exhausted"))
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
    pub remarked: bool,
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
            remarked: false,
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
    /// Split a proper subset; only the additional surviving stack gets an ID.
    pub fn split_off(&mut self, amount: u16, ids: &mut ItemIds) -> Option<Self> {
        if amount == 0 || amount >= self.count() {
            return None;
        }
        let mut split = self.clone();
        split.id = ids.allocate();
        split.count = NonZeroU16::new(amount).unwrap();
        self.count = NonZeroU16::new(self.count() - amount).unwrap();
        Some(split)
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
        self.count = NonZeroU16::new(self.count().saturating_sub(amount))?;
        Some(self)
    }
    /// Ordinary absorption retains destination metadata and source remainder identity.
    pub fn absorb(&mut self, mut source: Self, definition: &ItemDefinition) -> Option<Self> {
        if definition.id != self.item
            || definition.quantity != QuantityKind::Stack
            || self.item != source.item
            || self.identified != source.identified
            || self.remaining_uses.is_some()
            || source.remaining_uses.is_some()
        {
            return Some(source);
        }
        let moved =
            source.count().min(u16::from(definition.stack_limit).saturating_sub(self.count()));
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
        // Retail's unsigned room wraps for an overfull item, taking everything.
        let moved = maximum.checked_sub(uses).map_or(available, |room| room.min(available));
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
            *uses = uses.saturating_sub(1);
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
    let Some(definition) = catalog.get(incoming.item) else { return Some(incoming) };
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
    pub fn consume_backpack_units(&mut self, id: StackId, amount: u16) -> Result<(), Refusal> {
        let at = self
            .backpack
            .iter()
            .position(|s| s.as_ref().is_some_and(|s| s.id == id))
            .ok_or(Refusal::Nothing)?;
        let stack = self.backpack[at].as_ref().unwrap();
        if amount == 0 || amount > stack.count() {
            return Err(Refusal::Nothing);
        }
        self.backpack[at] = self.backpack[at].take().unwrap().remove_units(amount);
        Ok(())
    }
    /// Spend one use of a carried stack (retail `RemoveCharacterItem`): an
    /// exhausted stack leaves its slot unless `keep_empty`. Returns whether
    /// the stack was there.
    pub fn consume_use(
        &mut self,
        id: StackId,
        definition: &ItemDefinition,
        keep_empty: bool,
    ) -> bool {
        let Some(slot) = self.find(id) else { return false };
        self.offer_slot(slot, |stack| stack.consume_use(definition, keep_empty));
        true
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
            let d = catalog.get(e.stack.item).ok_or(Refusal::MissingContent)?;
            if d.cursed && !e.curse_released && !dead && s.is_active() {
                e.stack.curse_known = true;
                return Err(Refusal::Cursed);
            }
        }
        Ok(())
    }
    /// New placements intentionally lose release, including the alternate set.
    fn switch_weapon_sets(&mut self, catalog: &ItemCatalog, dead: bool) -> Result<(), Refusal> {
        for s in [EquipSlot::Primary, EquipSlot::Secondary] {
            self.check_removal(Slot::Equipment(s), catalog, dead)?;
        }
        for (a, b) in [
            (EquipSlot::Primary, EquipSlot::AlternatePrimary),
            (EquipSlot::Secondary, EquipSlot::AlternateSecondary),
        ] {
            let left = self.take(Slot::Equipment(a));
            let right = self.take(Slot::Equipment(b));
            self.put(Slot::Equipment(a), right);
            self.put(Slot::Equipment(b), left);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemPool {
    pub stacks: Vec<ItemStack>,
}
impl ItemPool {
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
            let name =
                catalog.get(item.item).map(|d| item.display_name(d, false)).unwrap_or_default();
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
    pub fn consume_use(&mut self, id: StackId, definition: &ItemDefinition) -> bool {
        let Some(at) = self.stacks.iter().position(|stack| stack.id == id) else { return false };
        if let Some(remaining) =
            self.stacks.remove(at).consume_use(definition, definition.keeps_empty())
        {
            self.stacks.insert(at, remaining);
        }
        true
    }

    /// Consume physical units while preserving the remainder's identity and row.
    pub fn consume_units(&mut self, id: StackId, amount: u16) -> Result<(), Refusal> {
        let at = self.stacks.iter().position(|s| s.id == id).ok_or(Refusal::Nothing)?;
        if amount == 0 || amount > self.stacks[at].count() {
            return Err(Refusal::Nothing);
        }
        if let Some(rest) = self.stacks.remove(at).remove_units(amount) {
            self.stacks.insert(at, rest);
        }
        Ok(())
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
    MissingContent,
    NotMember,
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
    vacating: Option<EquipSlot>,
) -> Vec<EquipSlot> {
    use EquipClass::*;
    // What is worn, but for the slot being emptied for this item.
    let worn = |slot: EquipSlot| carried.worn(slot).filter(|_| vacating != Some(slot));
    let empty = |slot: EquipSlot| worn(slot).is_none();
    // A main hand also counts as free for the off hand when its weapon is
    // one-handed.
    let main_free = |slot: EquipSlot| {
        worn(slot).is_none_or(|i| catalog.get(i.item).is_none_or(|d| !d.two_handed))
    };
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
/// their Artifacts reaches it. Retail compares signed bytes, so a
/// difficulty past 127 is always managed.
pub(crate) fn identify_attempt(
    character: &Character,
    vitals: &Vitals,
    difficulty: u8,
) -> Option<Identified> {
    if !vitals.lucid() {
        return None;
    }
    let strength = (character.skill(Skill::Artifacts) / 6) as i8;
    if difficulty as i8 > strength {
        return None;
    }
    let margin = i32::from(strength) - i32::from(difficulty);
    Some(Identified {
        binding: difficulty.wrapping_add(3) as i8 <= strength,
        practises: (0..3).contains(&margin),
    })
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
    if let Some(d) = catalog.get(item.item).filter(|d| member.can_use(d))
        && let Some(slot) = creation_slot(d, carried, catalog, member)
        && allowed_slots(d, carried, catalog, None).contains(&slot)
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
    catalog.get(item.item).map_or(0, |d| u32::from(d.weight) * item.units())
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
            .and_then(|i| catalog.get(i.item))
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
    content: crate::game::data::Content,
) {
    let (roster, inventory) = party.into_inner();
    let Some(catalog) = content.items() else { return };
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
        let (source, definition) = (catalog.get(held.item)?, catalog.get(gun.item)?);
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
    /// stages only the touched containers; no identities or random rolls occur.
    pub fn interact_slot(
        &mut self,
        character: &Character,
        vitals: &Vitals,
        carried: &mut Carried,
        slot: Slot,
        catalog: &ItemCatalog,
    ) -> Result<(), Refusal> {
        if matches!(slot, Slot::Backpack(i) if i >= BACKPACK_SLOTS) {
            return Err(Refusal::Nothing);
        }
        let current = carried.slot(slot);
        if current.is_none() && self.hand.held.is_none() {
            return Err(Refusal::Nothing);
        }
        if let Some((held, target)) = self.hand.held.as_ref().zip(current) {
            let d = catalog.get(held.item).ok_or(Refusal::MissingContent)?;
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
            let source = catalog.get(held.item).ok_or(Refusal::MissingContent)?;
            let definition = catalog.get(target.item).ok_or(Refusal::MissingContent)?;
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
            let definition = catalog.get(held.item).ok_or(Refusal::MissingContent)?;
            if !character.can_use(definition) {
                return Err(Refusal::DoesNotFit);
            }
            let displaced = equip.partner().filter(|p| {
                carried.worn(*p).is_some_and(|partner| {
                    catalog.get(partner.item).is_none_or(|d| !can_hold_together(definition, d))
                })
            });
            if let Some(partner) = displaced {
                carried.check_removal(Slot::Equipment(partner), catalog, vitals.is_dead())?;
                let mut candidate = carried.clone();
                let mut pool = self.pool.clone();
                let partner_item = candidate.take(Slot::Equipment(partner)).unwrap();
                if !allowed_slots(definition, &candidate, catalog, Some(equip)).contains(&equip) {
                    return Err(Refusal::DoesNotFit);
                }
                // Do not merge the displaced partner back into equipment about to leave.
                let current = candidate.take(slot);
                if candidate
                    .receive(partner_item, catalog)
                    .and_then(|rest| pool.receive(rest, catalog))
                    .is_some()
                {
                    return Err(Refusal::NoRoom);
                }
                candidate.put(slot, self.hand.held.take());
                self.hand.held = current;
                *carried = candidate;
                self.pool = pool;
                return Ok(());
            }
            if !allowed_slots(definition, carried, catalog, Some(equip)).contains(&equip) {
                return Err(Refusal::DoesNotFit);
            }
        }
        let current = carried.take(slot);
        carried.put(slot, self.hand.held.take());
        self.hand.held = current;
        Ok(())
    }
}

impl Hand {
    fn check_take_units(
        &self,
        source: &ItemStack,
        amount: u16,
        definition: &ItemDefinition,
    ) -> Result<(), Refusal> {
        if definition.quantity != QuantityKind::Stack || amount == 0 || amount > source.count() {
            return Err(Refusal::Nothing);
        }
        if definition.crucial {
            return Err(Refusal::Crucial);
        }
        if let Some(held) = &self.held
            && (held.item != source.item
                || u32::from(held.count()) + u32::from(amount) > u32::from(definition.stack_limit))
        {
            return Err(Refusal::HandFull);
        }
        Ok(())
    }
    /// After validation, move units into the hand and return the source remainder.
    /// Shift-taking retains held knowledge and allocates no ID when merging.
    fn take_units(
        &mut self,
        mut source: ItemStack,
        amount: u16,
        ids: &mut ItemIds,
    ) -> Option<ItemStack> {
        if let Some(held) = &mut self.held {
            held.count = NonZeroU16::new(held.count() + amount).unwrap();
            source.count = NonZeroU16::new(source.count() - amount)?;
        } else if amount == source.count() {
            self.held = Some(source);
            return None;
        } else {
            self.held = source.split_off(amount, ids);
        }
        Some(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::{Gender, Profession, Race};

    /// The pool's weight lands on whoever has room, and their load follows
    /// it.
    #[test]
    fn the_pool_weighs_on_the_members_with_room() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        let rock = ItemDefinition {
            id: ItemId(0),
            weight: 100,
            quantity: QuantityKind::Stack,
            stack_limit: 99,
            ..default()
        };
        let mut ids = ItemIds::default();
        let mut catalogs = Assets::<ItemCatalog>::default();
        let items = catalogs.add(ItemCatalog(vec![rock.clone()]));
        world.insert_resource(catalogs);
        world.insert_resource(crate::game::data::GameData {
            items,
            loot: default(),
            species: default(),
            spells: default(),
        });
        let strong = Character::new("Strong", Race::Dwarf, Gender::Male, Profession::Fighter);
        let weak = Character::new("Weak", Race::Faerie, Gender::Female, Profession::Mage);
        let members =
            [strong, weak].map(|c| world.spawn((Vitals::new(&c), c, Carried::default())).id());
        let party = crate::game::party::roster::test_party(&mut world, &members);
        let mut inventory = PartyInventory::default();
        inventory.pool.stacks.push(ItemStack::new(&rock, &mut ids, 60, true));
        world.entity_mut(party).insert(inventory);
        world.run_system_once(distribute_burden).unwrap();
        let [strong, weak] = members.map(|m| *world.get::<Burden>(m).unwrap());
        assert_eq!(strong.weight + weak.weight, 3000, "half the pool is shared");
        assert!(strong.weight > weak.weight, "the stronger has more room");
        assert!(strong.load > Load::None);
    }
    use crate::content::items::Dice;
    use enumset::EnumSet;

    fn catalog() -> ItemCatalog {
        let mut items: Vec<_> = [
            EquipClass::Ammunition,
            EquipClass::ShortWeapon,
            EquipClass::ExtendedWeapon,
            EquipClass::Shield,
            EquipClass::Head,
            EquipClass::Misc,
        ]
        .into_iter()
        .enumerate()
        .map(|(id, class)| ItemDefinition {
            id: ItemId(id as u16),
            class,
            name: format!("item {id}"),
            weight: 10,
            professions: EnumSet::all(),
            races: EnumSet::all(),
            genders: EnumSet::all(),
            ..default()
        })
        .collect();
        items[0].quantity = QuantityKind::Stack;
        items[0].stack_limit = 20;
        items[0].initial_quantity = Dice { count: 2, sides: 6, base: 0 };
        items[1].weapon_skill = Some(Skill::Sword);
        items[2].weapon_skill = Some(Skill::Axe);
        items[2].two_handed = true;
        items[4].cursed = true;
        items[5].quantity = QuantityKind::Charges;
        ItemCatalog(items)
    }
    fn stack(catalog: &ItemCatalog, ids: &mut ItemIds, item: usize, count: u16) -> ItemStack {
        ItemStack::new(&catalog.0[item], ids, count, true)
    }
    fn wearer() -> Character {
        Character::new("Wearer", Race::Human, Gender::Male, Profession::Fighter)
    }

    #[test]
    fn split_absorb_move_and_shift_take_have_batch_identity() {
        let (c, mut ids) = (catalog(), ItemIds::default());
        let mut source = stack(&c, &mut ids, 0, 10);
        let original = source.id;
        let part = source.split_off(4, &mut ids).unwrap();
        let split_id = part.id;
        assert_ne!(original, split_id);
        assert_eq!(source.count(), 6);
        let mut hand = Hand { held: Some(part) };
        source.identified = false;
        let next = ids.next;
        hand.check_take_units(&source, 3, &c.0[0]).unwrap();
        let source = hand.take_units(source, 3, &mut ids).unwrap();
        assert_eq!(ids.next, next, "merging into an existing hand allocates no identity");
        assert_eq!((source.id, source.count()), (original, 3));
        assert_eq!(
            (hand.held.as_ref().unwrap().id, hand.held.as_ref().unwrap().count()),
            (split_id, 7)
        );
        let mut pool = ItemPool::default();
        hand.offer(|s| pool.receive(s, &c));
        assert_eq!(pool.stacks[0].id, split_id);
        assert!(hand.held.is_none());
        // Ordinary merging requires matching identification, unlike shift-taking.
        let source = pool.stacks[0].absorb(source, &c.0[0]).unwrap();
        assert_eq!(source.id, original);
        assert_eq!(source.count(), 3);
    }

    #[test]
    fn partial_collection_conserves_counts_metadata_and_source_identity() {
        let (c, mut ids) = (catalog(), ItemIds::default());
        let mut party = PartyInventory::default();
        party.pool.stacks = (0..POOL_LIMIT).map(|_| stack(&c, &mut ids, 0, 20)).collect();
        party.pool.stacks[17] = stack(&c, &mut ids, 0, 19);
        let target_id = party.pool.stacks[17].id;
        let mut source = stack(&c, &mut ids, 0, 7);
        source.remarked = true;
        let source_id = source.id;
        let mut contents = vec![source];
        let taken = party.pool.collect(&mut contents, &c);
        assert_eq!(taken, vec![("item 0".to_owned(), 1)]);
        assert_eq!(
            (contents[0].id, contents[0].count(), contents[0].remarked),
            (source_id, 6, true)
        );
        assert_eq!(
            (
                party.pool.stacks[17].id,
                party.pool.stacks[17].count(),
                party.pool.stacks[17].remarked
            ),
            (target_id, 20, false)
        );
        let before = (party.clone(), contents.clone());
        assert!(party.pool.collect(&mut contents, &c).is_empty());
        assert_eq!((party, contents), before);
    }

    #[test]
    fn equipment_release_lives_until_the_placement_ends() {
        let (c, mut ids) = (catalog(), ItemIds::default());
        let character = wearer();
        let vitals = Vitals::new(&character);
        let mut carried = Carried::default();
        let item = stack(&c, &mut ids, 4, 1);
        let id = item.id;
        carried.equipment[EquipSlot::Head] = Some(Equipped::new(item));
        let mut party = PartyInventory::default();
        let head = Slot::Equipment(EquipSlot::Head);
        assert_eq!(
            party.interact_slot(&character, &vitals, &mut carried, head, &c),
            Err(Refusal::Cursed)
        );
        assert!(carried.worn(EquipSlot::Head).unwrap().curse_known);
        carried.equipment[EquipSlot::Head].as_mut().unwrap().curse_released = true;
        party.interact_slot(&character, &vitals, &mut carried, head, &c).unwrap();
        assert_eq!(party.hand.held.as_ref().unwrap().id, id);
        party.interact_slot(&character, &vitals, &mut carried, head, &c).unwrap();
        assert!(!carried.equipment[EquipSlot::Head].as_ref().unwrap().curse_released);
        assert_eq!(
            party.interact_slot(&character, &vitals, &mut carried, head, &c),
            Err(Refusal::Cursed)
        );
        carried.equipment[EquipSlot::Secondary] =
            Some(Equipped { stack: stack(&c, &mut ids, 0, 18), curse_released: true });
        assert!(carried.receive(stack(&c, &mut ids, 0, 2), &c).is_none());
        assert!(carried.equipment[EquipSlot::Secondary].as_ref().unwrap().curse_released);
        carried.switch_weapon_sets(&c, false).unwrap();
        assert!(!carried.equipment[EquipSlot::AlternateSecondary].as_ref().unwrap().curse_released);
    }

    #[test]
    fn failed_compound_equipment_change_is_atomic() {
        let (c, mut ids) = (catalog(), ItemIds::default());
        let character = wearer();
        let vitals = Vitals::new(&character);
        let mut carried = Carried::default();
        carried.equipment[EquipSlot::Primary] = Some(Equipped::new(stack(&c, &mut ids, 1, 1)));
        carried.equipment[EquipSlot::Secondary] = Some(Equipped::new(stack(&c, &mut ids, 3, 1)));
        for place in &mut carried.backpack {
            *place = Some(stack(&c, &mut ids, 1, 1));
        }
        let mut party = PartyInventory::default();
        party.pool.stacks = (0..POOL_LIMIT).map(|_| stack(&c, &mut ids, 1, 1)).collect();
        party.hand.held = Some(stack(&c, &mut ids, 2, 1));
        let before = (carried.clone(), party.clone(), ids.next);
        assert_eq!(
            party.interact_slot(
                &character,
                &vitals,
                &mut carried,
                Slot::Equipment(EquipSlot::Primary),
                &c
            ),
            Err(Refusal::NoRoom)
        );
        assert_eq!((carried.clone(), party.clone(), ids.next), before);
        carried.backpack[3] = None;
        party
            .interact_slot(
                &character,
                &vitals,
                &mut carried,
                Slot::Equipment(EquipSlot::Primary),
                &c,
            )
            .unwrap();
        assert_eq!(carried.worn(EquipSlot::Primary).unwrap().item, ItemId(2));
        assert!(carried.worn(EquipSlot::Secondary).is_none());
        assert_eq!(carried.backpack[3].as_ref().unwrap().item, ItemId(3));
        assert_eq!(party.hand.held.as_ref().unwrap().item, ItemId(1));
    }

    #[test]
    fn consumption_distinguishes_physical_count_and_empty_charge_reservoir() {
        let (c, mut ids) = (catalog(), ItemIds::default());
        assert!(stack(&c, &mut ids, 0, 1).consume_use(&c.0[0], false).is_none());
        let wand = stack(&c, &mut ids, 5, 1);
        let id = wand.id;
        let empty = wand.consume_use(&c.0[5], true).unwrap();
        assert_eq!((empty.id, empty.count(), empty.remaining_uses()), (id, 1, Some(0)));
        assert!(empty.consume_use(&c.0[5], false).is_none());
        assert_eq!(stack(&c, &mut ids, 1, 50).consume_use(&c.0[1], false).unwrap().count(), 1);
        assert_eq!(stack(&c, &mut ids, 0, 0).count(), 1);
        let mut overfull = stack(&c, &mut ids, 0, 30);
        assert_eq!(overfull.absorb(stack(&c, &mut ids, 0, 3), &c.0[0]).unwrap().count(), 3);
        assert_eq!(overfull.count(), 30);
    }

    #[test]
    fn pool_anchors_do_not_retarget_after_compaction() {
        let (c, mut ids) = (catalog(), ItemIds::default());
        let mut party = PartyInventory::default();
        let anchor = stack(&c, &mut ids, 1, 1);
        let id = anchor.id;
        party.pool.stacks = vec![anchor, stack(&c, &mut ids, 3, 1)];
        party.take_from_pool(id).unwrap();
        let before = party.clone();
        assert_eq!(party.deposit_hand(PoolPosition::Before(id), &c), Err(Refusal::Nothing));
        assert_eq!(party, before);
        assert_eq!(party.deposit_hand(PoolPosition::End, &c), Ok(1));
        assert_eq!(party.pool.stacks[1].id, id);
    }

    #[test]
    fn member_receipt_fills_equipment_then_backpack_without_auto_equipping() {
        let (c, mut ids) = (catalog(), ItemIds::default());
        let mut carried = Carried::default();
        carried.equipment[EquipSlot::Secondary] = Some(Equipped::new(stack(&c, &mut ids, 0, 18)));
        assert!(carried.receive(stack(&c, &mut ids, 0, 5), &c).is_none());
        assert_eq!(carried.worn(EquipSlot::Secondary).unwrap().count(), 20);
        assert_eq!(carried.backpack[0].as_ref().unwrap().count(), 3);
        assert!(carried.receive(stack(&c, &mut ids, 1, 1), &c).is_none());
        assert!(carried.worn(EquipSlot::Primary).is_none());
        assert_eq!(carried_weight(&carried, &c), 240);
        assert_eq!(pool_shares(140, &[Some((0, 100)), Some((50, 100))]), vec![60, 10]);
    }

    #[test]
    fn partial_receipt_continues_to_the_next_owner_and_keeps_overflow_in_hand() {
        let (c, mut ids) = (catalog(), ItemIds::default());
        let mut party = PartyInventory::default();
        party.pool.stacks = (0..POOL_LIMIT).map(|_| stack(&c, &mut ids, 0, 20)).collect();
        party.pool.stacks[0] = stack(&c, &mut ids, 0, 19);
        let mut carried = Carried::default();
        party.hand.held = Some(stack(&c, &mut ids, 0, 7));
        let id = party.hand.held.as_ref().unwrap().id;
        assert_eq!(party.give_held(Some(&mut carried), true, &c), Some(Given::Member));
        assert!(party.hand.held.is_none());
        assert_eq!(
            (
                carried.backpack[0].as_ref().unwrap().id,
                carried.backpack[0].as_ref().unwrap().count()
            ),
            (id, 6)
        );
        for place in &mut carried.backpack {
            *place = Some(stack(&c, &mut ids, 1, 1));
        }
        party.hand.held = Some(stack(&c, &mut ids, 0, 7));
        party.pool.stacks[0] = stack(&c, &mut ids, 0, 19);
        let id = party.hand.held.as_ref().unwrap().id;
        assert_eq!(party.give_held(Some(&mut carried), false, &c), Some(Given::Pool));
        assert_eq!(
            (party.hand.held.as_ref().unwrap().id, party.hand.held.as_ref().unwrap().count()),
            (id, 6)
        );
        assert_eq!(party.give_held(Some(&mut carried), false, &c), Some(Given::NoRoom));
    }

    #[test]
    fn partial_equipment_extraction_retains_release_and_whole_extraction_retires_source() {
        let (c, mut ids) = (catalog(), ItemIds::default());
        let mut carried = Carried::default();
        let item = stack(&c, &mut ids, 0, 4);
        let source_id = item.id;
        carried.equipment[EquipSlot::Secondary] =
            Some(Equipped { stack: item, curse_released: true });
        let mut hand = Hand::default();
        carried
            .offer_slot(Slot::Equipment(EquipSlot::Secondary), |s| hand.take_units(s, 2, &mut ids));
        let remaining = carried.equipment[EquipSlot::Secondary].as_ref().unwrap();
        assert_eq!(
            (remaining.stack.id, remaining.stack.count(), remaining.curse_released),
            (source_id, 2, true)
        );
        let held_id = hand.held.as_ref().unwrap().id;
        let next = ids.next;
        carried
            .offer_slot(Slot::Equipment(EquipSlot::Secondary), |s| hand.take_units(s, 2, &mut ids));
        assert!(carried.worn(EquipSlot::Secondary).is_none());
        assert_eq!(
            (hand.held.as_ref().unwrap().id, hand.held.as_ref().unwrap().count()),
            (held_id, 4)
        );
        assert_eq!(ids.next, next);
    }

    #[test]
    fn gameplay_operations_revalidate_membership_and_stack_identity() {
        use super::operations::InventoryOperations;
        use crate::game::data::GameData;
        use crate::game::party::roster::{test_join, test_party};
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.init_resource::<Messages<ItemNews>>();
        let (c, mut ids) = (catalog(), ItemIds::default());
        let item = stack(&c, &mut ids, 0, 7);
        let id = item.id;
        let mut catalogs = Assets::<ItemCatalog>::default();
        let items = catalogs.add(c);
        world.insert_resource(catalogs);
        world.insert_resource(GameData {
            items,
            loot: default(),
            species: default(),
            spells: default(),
        });
        // Someone else is in the party; the actor is not, at first.
        let other = world.spawn((Vitals::new(&wearer()), wearer(), Carried::default())).id();
        let party = test_party(&mut world, &[other]);
        let character = wearer();
        let mut carried = Carried::default();
        carried.backpack[0] = Some(item);
        let actor = world.spawn((Vitals::new(&character), character, carried)).id();
        let source = ItemRef { owner: ItemOwner::Member(actor), id };
        let take = |world: &mut World| {
            world.run_system_once(move |mut op: InventoryOperations| op.take(source)).unwrap()
        };
        assert_eq!(take(&mut world), Err(Refusal::NotMember));
        test_join(&mut world, actor);
        take(&mut world).unwrap();
        assert!(world.get::<Carried>(actor).unwrap().backpack[0].is_none());
        assert_eq!(world.get::<PartyInventory>(party).unwrap().hand.held.as_ref().unwrap().id, id);
        world
            .run_system_once(|mut op: InventoryOperations| op.deposit(PoolPosition::Front))
            .unwrap()
            .unwrap();
        assert_eq!(take(&mut world), Err(Refusal::Nothing));
        assert_eq!(world.get::<PartyInventory>(party).unwrap().pool.stacks[0].id, id);
        // Clicking a full pool must report a refusal, not pretend a transfer happened.
        let c = catalog();
        {
            let mut inventory = world.get_mut::<PartyInventory>(party).unwrap();
            inventory.pool.stacks = (0..POOL_LIMIT).map(|_| stack(&c, &mut ids, 1, 1)).collect();
            inventory.hand.held = Some(stack(&c, &mut ids, 1, 1));
        }
        let before = world.get::<PartyInventory>(party).unwrap().clone();
        assert_eq!(
            world.run_system_once(|mut op: InventoryOperations| op.click_pool(0)).unwrap(),
            Err(Refusal::NoRoom)
        );
        assert_eq!(*world.get::<PartyInventory>(party).unwrap(), before);
    }
    #[test]
    fn owner_consumption_keeps_nonempty_remainders_and_rejects_equipment() {
        let definition = ItemDefinition { quantity: QuantityKind::Stack, ..default() };
        let mut ids = ItemIds::default();
        let stack = ItemStack::new(&definition, &mut ids, 5, true);
        let id = stack.id;
        let mut pool = ItemPool { stacks: vec![stack.clone()] };
        assert_eq!(pool.consume_units(id, 6), Err(Refusal::Nothing));
        assert_eq!(pool.stacks[0], stack);
        pool.consume_units(id, 2).unwrap();
        assert_eq!((pool.stacks[0].id, pool.stacks[0].count()), (id, 3));
        pool.consume_units(id, 3).unwrap();
        assert!(pool.stacks.is_empty());
        let mut carried = Carried::default();
        carried.equipment[EquipSlot::Primary] = Some(Equipped::new(stack.clone()));
        assert_eq!(carried.consume_backpack_units(id, 1), Err(Refusal::Nothing));
        carried.equipment[EquipSlot::Primary] = None;
        carried.backpack[3] = Some(stack);
        carried.consume_backpack_units(id, 2).unwrap();
        let rest = carried.backpack_stack(id).unwrap();
        assert_eq!((rest.id, rest.count()), (id, 3));
        carried.consume_backpack_units(id, 3).unwrap();
        assert!(carried.backpack[3].is_none());
    }

    /// Ammunition held over its gun loads it up to the shots it holds,
    /// spending units from the stack; a full gun takes none.
    #[test]
    fn ammunition_loads_its_gun_up_to_what_it_holds() {
        let gun = ItemDefinition {
            id: ItemId(0),
            kind: ItemKind::MUSKET,
            class: EquipClass::RangedWeapon,
            quantity: QuantityKind::Shots,
            initial_quantity: crate::content::items::Dice { count: 0, sides: 0, base: 10 },
            ..default()
        };
        let clips = ItemDefinition {
            id: ItemId(1),
            kind: ItemKind::POWDER_SHOT,
            class: EquipClass::Ammunition,
            quantity: QuantityKind::Stack,
            stack_limit: 99,
            ..default()
        };
        let catalog = ItemCatalog(vec![gun, clips]);
        let mut ids = ItemIds::default();
        let (character, mut carried) = (wearer(), Carried::default());
        let vitals = Vitals::new(&character);
        carried.backpack[0] = Some(ItemStack::new(&catalog.0[0], &mut ids, 4, true));
        let mut party = PartyInventory::default();
        party.hand.held = Some(ItemStack::new(&catalog.0[1], &mut ids, 9, true));
        let mut load = |party: &mut PartyInventory| {
            party
                .interact_slot(&character, &vitals, &mut carried, Slot::Backpack(0), &catalog)
                .unwrap();
            let shots = carried.backpack[0].as_ref().unwrap().remaining_uses();
            (shots, party.hand.held.as_ref().map(ItemStack::count))
        };
        assert_eq!(load(&mut party), (Some(10), Some(3)));
        assert_eq!(load(&mut party), (Some(10), Some(3)), "a full gun takes none");
    }

    /// Guns hold only what loads them; each omnigun tier what it has learnt
    /// to fire; bows their arrows.
    #[test]
    fn guns_and_omnigun_tiers_hold_only_what_loads_them() {
        let item = |id: u16, kind: ItemKind, class| ItemDefinition {
            id: ItemId(id),
            kind,
            class,
            ..default()
        };
        let musket = item(0, ItemKind::MUSKET, EquipClass::RangedWeapon);
        let (shot, cells) = (
            item(1, ItemKind::POWDER_SHOT, EquipClass::Ammunition),
            item(2, ItemKind::POWER_PACK, EquipClass::Ammunition),
        );
        assert!(can_hold_together(&musket, &shot) && can_hold_together(&shot, &musket));
        assert!(!can_hold_together(&musket, &cells));
        let arrows = item(3, ItemKind::ARROW, EquipClass::Ammunition);
        let (first, last) = (
            item(599, ItemKind::OMNIGUN, EquipClass::RangedWeapon),
            item(610, ItemKind::OMNIGUN, EquipClass::RangedWeapon),
        );
        assert!(!can_hold_together(&first, &arrows) && can_hold_together(&last, &arrows));
        assert!(can_hold_together(&item(4, ItemKind::LONGBOW, EquipClass::RangedWeapon), &arrows));
    }

    /// The party identifies an item when someone's Artifacts reaches its
    /// difficulty; well past it, they also tell whether it binds.
    #[test]
    fn the_party_identifies_by_its_best_artifacts() {
        use super::operations::InventoryOperations;
        use crate::game::data::GameData;
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.init_resource::<Messages<ItemNews>>();
        let mut c = catalog();
        c.0[1].identify_difficulty = 3;
        let mut ids = ItemIds::default();
        let mut item = stack(&c, &mut ids, 1, 1);
        item.identified = false;
        let mut catalogs = Assets::<ItemCatalog>::default();
        let items = catalogs.add(c);
        world.insert_resource(catalogs);
        world.insert_resource(GameData {
            items,
            loot: default(),
            species: default(),
            spells: default(),
        });
        let member = |world: &mut World, artifacts: u8| {
            let mut character = wearer();
            character.skill_points[Skill::Artifacts] = artifacts;
            world.spawn((Vitals::new(&character), character, Carried::default())).id()
        };
        let novice = member(&mut world, 12);
        let party = crate::game::party::roster::test_party(&mut world, &[novice]);
        world.get_mut::<PartyInventory>(party).unwrap().hand.held = Some(item);
        let identify = |world: &mut World| {
            world
                .run_system_once(move |mut op: InventoryOperations, mut rng: ResMut<GameRng>| {
                    op.identify(novice, ItemAt::Hand, &mut rng)
                })
                .unwrap()
        };
        world.init_resource::<GameRng>();
        assert_eq!(identify(&mut world), Ok(false));
        let expert = member(&mut world, 60);
        crate::game::party::roster::test_join(&mut world, expert);
        assert_eq!(identify(&mut world), Ok(true));
        let held = world.get::<PartyInventory>(party).unwrap().hand.held.clone().unwrap();
        assert!(held.identified && held.spell_known && held.curse_known);
        assert_eq!(identify(&mut world), Ok(false), "nothing left to identify");
    }

    /// A charged source loads a gun in the pool through the pool's click,
    /// keeping the charges it has left; a full gun takes none. Neither is
    /// refused, though no physical unit moves.
    #[test]
    fn a_charged_source_loads_a_pool_gun_and_keeps_its_rest() {
        use super::operations::InventoryOperations;
        use crate::content::items::Dice;
        use crate::game::data::GameData;
        use bevy::ecs::system::RunSystemOnce;
        let gun = ItemDefinition {
            id: ItemId(0),
            kind: ItemKind::MUSKET,
            class: EquipClass::RangedWeapon,
            quantity: QuantityKind::Shots,
            initial_quantity: Dice { count: 0, sides: 0, base: 10 },
            ..default()
        };
        let cell = ItemDefinition {
            id: ItemId(1),
            kind: ItemKind::POWDER_SHOT,
            class: EquipClass::Ammunition,
            quantity: QuantityKind::Charges,
            ..default()
        };
        let mut world = World::new();
        world.init_resource::<Messages<ItemNews>>();
        let mut ids = ItemIds::default();
        let (gun_stack, cell_stack) =
            (ItemStack::new(&gun, &mut ids, 4, true), ItemStack::new(&cell, &mut ids, 8, true));
        let mut catalogs = Assets::<ItemCatalog>::default();
        let items = catalogs.add(ItemCatalog(vec![gun, cell]));
        world.insert_resource(catalogs);
        world.insert_resource(GameData {
            items,
            loot: default(),
            species: default(),
            spells: default(),
        });
        let member = world.spawn((Vitals::new(&wearer()), wearer(), Carried::default())).id();
        let party = crate::game::party::roster::test_party(&mut world, &[member]);
        {
            let mut inventory = world.get_mut::<PartyInventory>(party).unwrap();
            inventory.pool.stacks.push(gun_stack);
            inventory.hand.held = Some(cell_stack.clone());
        }
        let click = |world: &mut World| {
            let result =
                world.run_system_once(|mut op: InventoryOperations| op.click_pool(0)).unwrap();
            let inventory = world.get::<PartyInventory>(party).unwrap();
            let held = inventory.hand.held.as_ref().map(|h| (h.id, h.remaining_uses()));
            (result, inventory.pool.stacks[0].remaining_uses(), held)
        };
        let rest = Some((cell_stack.id, Some(2)));
        assert_eq!(click(&mut world), (Ok(()), Some(10), rest));
        assert_eq!(click(&mut world), (Ok(()), Some(10), rest), "a full gun takes none");
    }
}
