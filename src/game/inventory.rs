//! Owned nonempty stacks, concrete containers, and synchronous inventory moves.
//! Equipment release belongs to a placement, never to the transferable object.
pub mod operations;

use super::characters::{Character, Vitals};
use super::rng::GameRng;
use crate::content::characters::Skill;
use crate::content::items::{EquipClass, ItemCatalog, ItemDefinition, ItemId, QuantityKind};
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
        let name = if self.identified { &definition.name } else { &definition.unidentified_name };
        if count && self.count() > 1 { format!("{name} ({})", self.count()) } else { name.clone() }
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
    /// Consumption owns its input: exhausted physical stacks never survive as values.
    pub fn consume_use(mut self, definition: &ItemDefinition, keep_empty: bool) -> Option<Self> {
        if let Some(uses) = &mut self.remaining_uses {
            *uses = uses.saturating_sub(1);
            if *uses == 0 && !keep_empty {
                return None;
            }
        } else if definition.quantity == QuantityKind::Stack {
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolPosition {
    Front,
    Before(StackId),
    End,
}

#[derive(Component, Clone, Debug, Default, PartialEq)]
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
    /// Worn stacks, backpack stacks, then the first empty backpack slot.
    pub fn receive(&mut self, incoming: ItemStack, catalog: &ItemCatalog) -> Option<ItemStack> {
        let rest = fill(
            self.equipment
                .values_mut()
                .flatten()
                .map(|e| &mut e.stack)
                .chain(self.backpack.iter_mut().flatten()),
            incoming,
            catalog,
        )?;
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
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct PartyInventory {
    pub pool: ItemPool,
    pub hand: Hand,
    pub gold: u32,
}
#[derive(Message, Clone, Debug, PartialEq)]
pub enum ItemNews {
    Takes { member: Option<String>, item: String },
    NoRoom,
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

/// Whether two items can share a weapon set (retail `CanHoldItemsTogether`).
pub fn can_hold_together(a: &ItemDefinition, b: &ItemDefinition) -> bool {
    use EquipClass::*;
    if a.two_handed || b.two_handed {
        return false;
    }
    let ranged = |d: &ItemDefinition| matches!(d.class, RangedWeapon | Ammunition);
    if ranged(a) || ranged(b) {
        // A launcher with the ammunition it shoots (retail
        // `CompatiblePartnerItems`).
        let fits = |launcher: &ItemDefinition, ammunition: &ItemDefinition| {
            launcher.shoots.is_some() && launcher.shoots == ammunition.ammunition
        };
        return fits(a, b) || fits(b, a);
    }
    let weapon =
        |d: &ItemDefinition| matches!(d.class, ShortWeapon | ExtendedWeapon | ThrownWeapon);
    !(weapon(a) && weapon(b)) || a.reach == b.reach
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
        use crate::game::{data::GameData, party::roster::PartyRoster};
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
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
        world.init_resource::<PartyRoster>();
        world.init_resource::<PartyInventory>();
        let character = wearer();
        let mut carried = Carried::default();
        carried.backpack[0] = Some(item);
        let actor = world.spawn((Vitals::new(&character), character, carried)).id();
        let source = ItemRef { owner: ItemOwner::Member(actor), id };
        let take = |world: &mut World| {
            world.run_system_once(move |mut op: InventoryOperations| op.take(source)).unwrap()
        };
        assert_eq!(take(&mut world), Err(Refusal::NotMember));
        world.resource_mut::<PartyRoster>().0.insert(actor);
        take(&mut world).unwrap();
        assert!(world.get::<Carried>(actor).unwrap().backpack[0].is_none());
        assert_eq!(world.resource::<PartyInventory>().hand.held.as_ref().unwrap().id, id);
        world
            .run_system_once(|mut op: InventoryOperations| op.deposit(PoolPosition::Front))
            .unwrap()
            .unwrap();
        assert_eq!(take(&mut world), Err(Refusal::Nothing));
        assert_eq!(world.resource::<PartyInventory>().pool.stacks[0].id, id);
        // Clicking a full pool must report a refusal, not pretend a transfer happened.
        let c = catalog();
        {
            let mut party = world.resource_mut::<PartyInventory>();
            party.pool.stacks = (0..POOL_LIMIT).map(|_| stack(&c, &mut ids, 1, 1)).collect();
            party.hand.held = Some(stack(&c, &mut ids, 1, 1));
        }
        let before = world.resource::<PartyInventory>().clone();
        assert_eq!(
            world.run_system_once(|mut op: InventoryOperations| op.click_pool(0)).unwrap(),
            Err(Refusal::NoRoom)
        );
        assert_eq!(*world.resource::<PartyInventory>(), before);
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
}
