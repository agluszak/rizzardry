//! The party's items: what each member carries, the shared pool, the item in
//! hand and the purse, and the moves between them.
//!
//! Items are owned values, not entities; each move returns what happened.
//! Rules follow retail as reconstructed by Whizzardry (see
//! `docs/research/inventory.md`). Whether a member can use an item needs
//! character rules that do not exist yet; callers pass it in.

use bevy::prelude::*;

use super::characters::{Character, DUAL_WEAPONS};
use super::rng::GameRng;
use crate::content::items::{EquipClass, ItemCatalog, ItemDefinition, ItemId, QuantityKind};

/// One item's identity, kept through moves; a split makes a new one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect)]
pub struct ItemInstanceId(pub u32);

/// Hands out item identities.
#[derive(Resource, Debug, Default)]
pub struct ItemIds {
    next: u32,
}

impl ItemIds {
    pub fn allocate(&mut self) -> ItemInstanceId {
        self.next += 1;
        ItemInstanceId(self.next)
    }
}

/// How much of an item there is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum Quantity {
    One,
    /// Identical things in one slot.
    Stack(u8),
    /// Charges, uses or shots left.
    Uses(u8),
}

#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct ItemInstance {
    pub id: ItemInstanceId,
    pub item: ItemId,
    pub quantity: Quantity,
    pub identified: bool,
    /// Its spell is known before it is fully identified.
    pub spell_known: bool,
    /// Whether it is cursed is known.
    pub curse_known: bool,
    /// Remove Curse let it go; lost whenever it moves.
    pub curse_released: bool,
    /// Its one-time "exceptional item" remark has been made.
    pub remarked: bool,
}

impl ItemInstance {
    /// Units of it: a stack's size, otherwise one.
    pub fn units(&self) -> u32 {
        match self.quantity {
            Quantity::Stack(n) => u32::from(n).max(1),
            _ => 1,
        }
    }

    /// What the party calls it (retail `GetItemDisplayName`): its name once
    /// identified, else its kind's; with `count`, a stack's size after it
    /// (`FormatItemDisplayName`, "Bullet Stones (16)").
    pub fn display_name(&self, definition: &ItemDefinition, count: bool) -> String {
        let name = if self.identified { &definition.name } else { &definition.unidentified_name };
        match self.quantity {
            Quantity::Stack(n) if count && n > 1 => format!("{name} ({n})"),
            _ => name.clone(),
        }
    }

    /// Every move goes through here: moving an item loses a released curse.
    fn moved(mut self) -> Self {
        self.curse_released = false;
        self
    }
}

/// A new item (retail `MakeItem`): its quantity rolled, or the most it can
/// start with when `maximum`; identified if forced or self-identifying.
pub fn make_item(
    definition: &ItemDefinition,
    ids: &mut ItemIds,
    rng: &mut GameRng,
    maximum: bool,
    identify: bool,
) -> ItemInstance {
    let mut amount = || {
        let n = if maximum {
            definition.initial_quantity.maximum()
        } else {
            rng.roll(definition.initial_quantity)
        };
        n.min(u32::from(u8::MAX)) as u8
    };
    let quantity = match definition.quantity {
        QuantityKind::None => Quantity::One,
        QuantityKind::Stack => Quantity::Stack(amount()),
        _ => Quantity::Uses(amount()),
    };
    ItemInstance {
        id: ids.allocate(),
        item: definition.id,
        quantity,
        identified: identify || definition.auto_identify,
        spell_known: false,
        curse_known: false,
        curse_released: false,
        remarked: false,
    }
}

/// Merge `source` into `into` (retail `MergeStack`): same item, same
/// identification, a stack, up to the definition's maximum. Returns the
/// units moved; the rest stays in `source`.
pub fn merge_stack(
    into: &mut ItemInstance,
    source: &mut ItemInstance,
    definition: &ItemDefinition,
) -> u8 {
    let (Quantity::Stack(have), Quantity::Stack(more)) = (&mut into.quantity, &mut source.quantity)
    else {
        return 0;
    };
    if into.item != source.item || into.identified != source.identified {
        return 0;
    }
    let moved = (*more).min(definition.maximum_quantity.saturating_sub(*have));
    *have += moved;
    *more -= moved;
    moved
}

/// Spend one use (retail `SpendItemUse`). Returns whether the item is used
/// up and gone. An emptied item with lasting properties is kept: those are
/// not decoded yet, so emptied items with uses are kept only when
/// `keep_empty`.
pub fn spend_use(item: &mut ItemInstance, keep_empty: bool) -> bool {
    match &mut item.quantity {
        Quantity::One => false,
        Quantity::Stack(n) => {
            *n = n.saturating_sub(1);
            *n == 0
        }
        Quantity::Uses(0) => !keep_empty,
        Quantity::Uses(n) => {
            *n -= 1;
            *n == 0 && !keep_empty
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect)]
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
    pub const ALL: [EquipSlot; 12] = [
        EquipSlot::Head,
        EquipSlot::Misc1,
        EquipSlot::Misc2,
        EquipSlot::Cloak,
        EquipSlot::Torso,
        EquipSlot::Hands,
        EquipSlot::Primary,
        EquipSlot::Secondary,
        EquipSlot::AlternatePrimary,
        EquipSlot::AlternateSecondary,
        EquipSlot::Legs,
        EquipSlot::Feet,
    ];

    fn index(self) -> usize {
        EquipSlot::ALL.iter().position(|&s| s == self).unwrap_or(0)
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
/// The most rows the shared pool holds.
pub const POOL_LIMIT: usize = 500;

/// What one party member carries: worn equipment and a backpack.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Carried {
    pub equipment: [Option<ItemInstance>; 12],
    pub backpack: [Option<ItemInstance>; BACKPACK_SLOTS],
}

impl Carried {
    pub fn worn(&self, slot: EquipSlot) -> Option<&ItemInstance> {
        self.equipment[slot.index()].as_ref()
    }

    fn slot_mut(&mut self, location: Slot) -> Option<&mut Option<ItemInstance>> {
        match location {
            Slot::Equipment(slot) => Some(&mut self.equipment[slot.index()]),
            Slot::Backpack(index) => self.backpack.get_mut(index),
        }
    }
}

/// A place on a member.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum Slot {
    Equipment(EquipSlot),
    Backpack(usize),
}

/// Where an item is: on a member (by party position) or in the pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum ItemLocation {
    Member(usize, Slot),
    Pool(usize),
}

/// The item in hand and where it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Held {
    pub item: ItemInstance,
    pub origin: Option<ItemLocation>,
}

/// What the party shares: the pool, the item in hand and the purse.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct PartyInventory {
    pub pool: Vec<ItemInstance>,
    pub held: Option<Held>,
    pub gold: u32,
}

/// What the main screen tells about items changing hands.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum ItemNews {
    /// "Ragnar takes %s", or "Party takes %s" for the pool.
    Takes {
        member: Option<String>,
        item: String,
    },
    NoRoom,
}

/// Where a given item went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Given {
    Member,
    Pool,
    /// Neither had room ("No room!").
    NoRoom,
}

/// Why a move did not happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Nothing,
    /// The item cannot be split or thrown away.
    Crucial,
    /// A curse keeps it on ("This item is cursed!").
    Cursed,
    /// It does not fit there, or the member cannot use it.
    DoesNotFit,
    /// The hand holds something else, or too much.
    HandFull,
    /// No room ("No room!").
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

/// Ammunition kinds that fit a ranged weapon kind, from retail's hard-coded
/// table (`CompatiblePartnerItems`). Only the plain pairs are here; the
/// Gadgeteer's tiers and the guns' range rule are not yet.
const PARTNERS: [(u16, u16); 4] = [(0x0b, 0x16), (0x25, 0x16), (0x0c, 0x53), (0x0d, 0x17)];

/// Whether two items can share a weapon set (retail `CanHoldItemsTogether`).
pub fn can_hold_together(a: &ItemDefinition, b: &ItemDefinition) -> bool {
    use EquipClass::*;
    if a.two_handed || b.two_handed {
        return false;
    }
    let ranged = |d: &ItemDefinition| matches!(d.class, RangedWeapon | Ammunition);
    if ranged(a) || ranged(b) {
        return PARTNERS.iter().any(|&(weapon, ammo)| {
            (a.kind, b.kind) == (weapon, ammo) || (b.kind, a.kind) == (weapon, ammo)
        });
    }
    let weapon =
        |d: &ItemDefinition| matches!(d.class, ShortWeapon | ExtendedWeapon | ThrownWeapon);
    !(weapon(a) && weapon(b)) || a.range == b.range
}

/// Whether what is in a slot may come off (retail `CanUnequipSlotItem`): a
/// curse holds worn items until released, unless the wearer is dead.
pub fn can_take_off(
    slot: EquipSlot,
    item: &ItemInstance,
    definition: &ItemDefinition,
    dead: bool,
) -> bool {
    !definition.cursed
        || item.curse_released
        || matches!(slot, EquipSlot::AlternatePrimary | EquipSlot::AlternateSecondary)
        || dead
}

/// Give an item to a member (retail `PlaceWithCharacter` during play): merge
/// into matching stacks worn, then in the backpack, then take the first
/// empty backpack slot. Never equips. Whatever does not fit comes back.
pub fn place_with_member(
    carried: &mut Carried,
    item: ItemInstance,
    catalog: &ItemCatalog,
) -> Result<(), ItemInstance> {
    let mut item = item.moved();
    if let (Some(definition), Quantity::Stack(_)) = (catalog.get(item.item), item.quantity) {
        for slot in carried.equipment.iter_mut().chain(carried.backpack.iter_mut()).flatten() {
            merge_stack(slot, &mut item, definition);
            if item.quantity == Quantity::Stack(0) {
                return Ok(());
            }
        }
    }
    match carried.backpack.iter_mut().find(|slot| slot.is_none()) {
        Some(empty) => {
            *empty = Some(item);
            Ok(())
        }
        None => Err(item),
    }
}

/// Give a new character an item (retail `PlaceWithCharacter` with `equip`,
/// used only during party creation): wear it in its creation slot when the
/// character can use it and the slot takes it, otherwise as
/// [`place_with_member`].
pub fn place_at_creation(
    carried: &mut Carried,
    item: ItemInstance,
    catalog: &ItemCatalog,
    member: &Character,
) -> Result<(), ItemInstance> {
    let mut item = item.moved();
    if let Some(definition) = catalog.get(item.item).filter(|d| member.can_use(d))
        && let Some(slot) = creation_slot(definition, carried, catalog, member)
        && allowed_slots(definition, carried, catalog, None).contains(&slot)
    {
        match &mut carried.equipment[slot.index()] {
            empty @ None => {
                *empty = Some(item);
                return Ok(());
            }
            Some(worn) => {
                merge_stack(worn, &mut item, definition);
                if item.quantity == Quantity::Stack(0) {
                    return Ok(());
                }
            }
        }
    }
    place_with_member(carried, item, catalog)
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
            if member.skill_points[DUAL_WEAPONS] > 0 && empty(EquipSlot::Secondary) =>
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

/// An item's weight in tenths of a pound: each unit of a stack counts.
pub fn item_weight(item: &ItemInstance, catalog: &ItemCatalog) -> u32 {
    catalog.get(item.item).map_or(0, |d| u32::from(d.weight) * item.units())
}

/// What a member wears and carries in the backpack weighs.
pub fn carried_weight(carried: &Carried, catalog: &ItemCatalog) -> u32 {
    carried
        .equipment
        .iter()
        .chain(&carried.backpack)
        .flatten()
        .map(|i| item_weight(i, catalog))
        .sum()
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
    /// Put an item in the pool (retail `placeInPool`): merge into its stacks
    /// in order, then put the rest at the front. Comes back if the pool is
    /// full.
    pub fn place_in_pool(
        &mut self,
        item: ItemInstance,
        catalog: &ItemCatalog,
    ) -> Result<(), ItemInstance> {
        let mut item = item.moved();
        if let (Some(definition), Quantity::Stack(_)) = (catalog.get(item.item), item.quantity) {
            for row in &mut self.pool {
                merge_stack(row, &mut item, definition);
                if item.quantity == Quantity::Stack(0) {
                    return Ok(());
                }
            }
        }
        if self.pool.len() >= POOL_LIMIT {
            return Err(item);
        }
        self.pool.insert(0, item);
        Ok(())
    }

    /// Give an item to the party: the member first, then the pool.
    pub fn receive(
        &mut self,
        carried: &mut Carried,
        item: ItemInstance,
        catalog: &ItemCatalog,
    ) -> Result<(), ItemInstance> {
        match place_with_member(carried, item, catalog) {
            Ok(()) => Ok(()),
            Err(rest) => self.place_in_pool(rest, catalog),
        }
    }

    /// Take `amount` units of a stack into the hand (retail `pickUp`). The
    /// part taken is a new item unless the whole stack is.
    pub fn pick_up(
        &mut self,
        member: Option<(&Character, &mut Carried)>,
        location: ItemLocation,
        amount: u8,
        ids: &mut ItemIds,
        catalog: &ItemCatalog,
    ) -> Result<(), Refusal> {
        let held = self.held.as_ref().map(|h| (h.item.item, h.item.units()));
        let dead = member.as_ref().is_some_and(|(character, _)| character.is_dead());
        // Split `amount` off `source`; true when that is all of it.
        let mut split = |source: &mut ItemInstance| -> Result<(bool, ItemInstance), Refusal> {
            let definition = catalog.get(source.item).ok_or(Refusal::Nothing)?;
            let Quantity::Stack(count) = source.quantity else { return Err(Refusal::Nothing) };
            if amount == 0 || amount > count {
                return Err(Refusal::Nothing);
            }
            if definition.crucial {
                return Err(Refusal::Crucial);
            }
            if let Some((item, units)) = held
                && (item != source.item
                    || units + u32::from(amount) > u32::from(definition.maximum_quantity))
            {
                return Err(Refusal::HandFull);
            }
            if let ItemLocation::Member(_, Slot::Equipment(slot)) = location
                && amount == count
                && !can_take_off(slot, source, definition, dead)
            {
                return Err(Refusal::Cursed);
            }
            let mut taken = source.clone();
            taken.quantity = Quantity::Stack(amount);
            if amount < count {
                taken.id = ids.allocate();
                source.quantity = Quantity::Stack(count - amount);
            }
            Ok((amount == count, taken.moved()))
        };
        let taken = match location {
            ItemLocation::Pool(row) => {
                let (whole, taken) = split(self.pool.get_mut(row).ok_or(Refusal::Nothing)?)?;
                if whole {
                    self.pool.remove(row);
                }
                taken
            }
            ItemLocation::Member(_, slot) => {
                let place = member.and_then(|(_, m)| m.slot_mut(slot)).ok_or(Refusal::Nothing)?;
                let (whole, taken) = split(place.as_mut().ok_or(Refusal::Nothing)?)?;
                if whole {
                    *place = None;
                }
                taken
            }
        };
        match &mut self.held {
            Some(held) => {
                if let Quantity::Stack(n) = &mut held.item.quantity {
                    *n += amount;
                }
            }
            None => self.held = Some(Held { item: taken, origin: Some(location) }),
        }
        Ok(())
    }

    /// Exchange what is at a member's slot with the hand (retail
    /// `swapWithHand`): picks up, puts down or swaps. Into equipment only
    /// what the member can use and the slot takes; out of it only what a
    /// curse lets go (the dead keep nothing).
    pub fn swap_with_hand(
        &mut self,
        character: &Character,
        carried: &mut Carried,
        member: usize,
        slot: Slot,
        catalog: &ItemCatalog,
    ) -> Result<(), Refusal> {
        let current = carried.slot_mut(slot).ok_or(Refusal::Nothing)?.clone();
        if current.is_none() && self.held.is_none() {
            return Err(Refusal::Nothing);
        }
        if let Slot::Equipment(equip) = slot {
            if let Some(item) = &current
                && let Some(definition) = catalog.get(item.item)
                && !can_take_off(equip, item, definition, character.is_dead())
            {
                return Err(Refusal::Cursed);
            }
            if let Some(held) = &self.held {
                let definition = catalog.get(held.item.item).ok_or(Refusal::DoesNotFit)?;
                // Judged with this slot emptied for it.
                if !character.can_use(definition)
                    || !allowed_slots(definition, carried, catalog, Some(equip)).contains(&equip)
                {
                    return Err(Refusal::DoesNotFit);
                }
            }
        }
        let location = ItemLocation::Member(member, slot);
        let put = self.held.take().map(|held| held.item.moved());
        *carried.slot_mut(slot).ok_or(Refusal::Nothing)? = put;
        self.held = current.map(|item| Held { item: item.moved(), origin: Some(location) });
        Ok(())
    }

    /// Take a pool row into the empty hand (retail `swapWithHand` on the
    /// pool); the rows after it close up.
    pub fn take_from_pool(&mut self, row: usize) -> Result<(), Refusal> {
        if self.held.is_some() || row >= self.pool.len() {
            return Err(Refusal::Nothing);
        }
        let item = self.pool.remove(row).moved();
        self.held = Some(Held { item, origin: Some(ItemLocation::Pool(row)) });
        Ok(())
    }

    /// Give the held item to a member clicked on the main screen (retail
    /// `GiveHeldItemToCharacterOrParty`): to them, else the pool; the pool
    /// first when `pool_first` (in a fight); only the pool when they cannot
    /// take items (`member` is `None`). Refused, it stays in hand. Retail
    /// also lets everyone try to identify it; that waits for the skill.
    pub fn give_held(
        &mut self,
        member: Option<&mut Carried>,
        pool_first: bool,
        catalog: &ItemCatalog,
    ) -> Option<Given> {
        let held = self.held.take()?;
        let item = held.item;
        let to_member = |carried: &mut Carried, item| place_with_member(carried, item, catalog);
        let given = match member {
            Some(carried) if !pool_first => to_member(carried, item)
                .map(|()| Given::Member)
                .or_else(|rest| self.place_in_pool(rest, catalog).map(|()| Given::Pool)),
            Some(carried) => self
                .place_in_pool(item, catalog)
                .map(|()| Given::Pool)
                .or_else(|rest| to_member(carried, rest).map(|()| Given::Member)),
            None => self.place_in_pool(item, catalog).map(|()| Given::Pool),
        };
        Some(match given {
            Ok(given) => given,
            Err(item) => {
                self.held = Some(Held { item, origin: held.origin });
                Given::NoRoom
            }
        })
    }

    /// Put the held item away with a member, or in the pool when they have
    /// no room. When neither can take it, it stays in hand.
    pub fn put_away_held(&mut self, carried: &mut Carried, catalog: &ItemCatalog) {
        if let Some(held) = self.held.take()
            && let Err(item) = self.receive(carried, held.item, catalog)
        {
            self.held = Some(Held { item, origin: held.origin });
        }
    }

    /// Merge the held stack into the stack at `location` (retail
    /// `mergeHeld`); returns the units moved, the rest stays in hand.
    pub fn merge_held(
        &mut self,
        carried: Option<&mut Carried>,
        location: ItemLocation,
        catalog: &ItemCatalog,
    ) -> u8 {
        let Some(mut held) = self.held.take() else { return 0 };
        let moved = {
            let target = match location {
                ItemLocation::Pool(row) => self.pool.get_mut(row),
                ItemLocation::Member(_, slot) => {
                    carried.and_then(|c| c.slot_mut(slot)).and_then(Option::as_mut)
                }
            };
            match (target, catalog.get(held.item.item)) {
                (Some(target), Some(definition)) => merge_stack(target, &mut held.item, definition),
                _ => 0,
            }
        };
        if held.item.quantity != Quantity::Stack(0) {
            self.held = Some(held);
        }
        moved
    }

    /// Put the held item into the pool at `row` (retail `depositHeld`): a
    /// stack merges into that row, then every pool stack; the rest goes in
    /// at `row`.
    pub fn deposit_held(&mut self, row: usize, catalog: &ItemCatalog) -> Result<(), Refusal> {
        let Some(mut held) = self.held.take() else { return Err(Refusal::Nothing) };
        if row > self.pool.len() {
            self.held = Some(held);
            return Err(Refusal::Nothing);
        }
        if let (Some(definition), Quantity::Stack(_)) =
            (catalog.get(held.item.item), held.item.quantity)
        {
            let rows = self.pool.len();
            let order = std::iter::once(row).chain((0..rows).filter(|&r| r != row));
            for r in order.filter(|&r| r < rows) {
                merge_stack(&mut self.pool[r], &mut held.item, definition);
                if held.item.quantity == Quantity::Stack(0) {
                    return Ok(());
                }
            }
        }
        if self.pool.len() >= POOL_LIMIT {
            self.held = Some(held);
            return Err(Refusal::NoRoom);
        }
        self.pool.insert(row, held.item.moved());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::items::Dice;

    fn definition(id: u16, class: EquipClass) -> ItemDefinition {
        ItemDefinition {
            id: ItemId(id),
            name: format!("item {id}"),
            class,
            profession_mask: 0xffff,
            race_mask: u32::MAX,
            gender_mask: 3,
            weight: 10,
            ..default()
        }
    }

    /// Arrows (stack of 20), a sword, a two-handed axe, a shield, a cursed
    /// helm, a crucial key.
    fn catalog() -> ItemCatalog {
        let mut arrows = definition(0, EquipClass::Ammunition);
        arrows.quantity = QuantityKind::Stack;
        arrows.maximum_quantity = 20;
        arrows.initial_quantity = Dice { count: 2, sides: 6, base: 0 };
        let mut sword = definition(1, EquipClass::ShortWeapon);
        sword.weapon_skill = Some(0);
        let mut axe = definition(2, EquipClass::ExtendedWeapon);
        axe.weapon_skill = Some(1);
        axe.two_handed = true;
        let shield = definition(3, EquipClass::Shield);
        let mut helm = definition(4, EquipClass::Head);
        helm.cursed = true;
        let mut key = definition(5, EquipClass::Key);
        key.quantity = QuantityKind::Stack;
        key.maximum_quantity = 5;
        key.crucial = true;
        ItemCatalog(vec![arrows, sword, axe, shield, helm, key])
    }

    fn item(ids: &mut ItemIds, id: u16, quantity: Quantity) -> ItemInstance {
        ItemInstance {
            id: ids.allocate(),
            item: ItemId(id),
            quantity,
            identified: true,
            spell_known: false,
            curse_known: false,
            curse_released: false,
            remarked: false,
        }
    }

    #[test]
    fn a_given_item_goes_to_the_member_or_in_a_fight_the_pool() {
        let (catalog, mut ids) = (catalog(), ItemIds::default());
        let mut carried = Carried::default();
        let mut party = PartyInventory {
            held: Some(Held { item: item(&mut ids, 1, Quantity::One), origin: None }),
            ..default()
        };
        assert_eq!(party.give_held(Some(&mut carried), false, &catalog), Some(Given::Member));
        assert!(party.held.is_none() && party.pool.is_empty());
        party.held = Some(Held { item: item(&mut ids, 1, Quantity::One), origin: None });
        assert_eq!(party.give_held(Some(&mut carried), true, &catalog), Some(Given::Pool));
        assert_eq!(party.pool.len(), 1);
        // A full pool and no member to take it: it stays in hand.
        party.pool = vec![item(&mut ids, 3, Quantity::One); POOL_LIMIT];
        party.held = Some(Held { item: item(&mut ids, 1, Quantity::One), origin: None });
        assert_eq!(party.give_held(None, true, &catalog), Some(Given::NoRoom));
        assert!(party.held.is_some());
    }

    #[test]
    fn stacks_merge_up_to_their_maximum_and_only_alike() {
        let (catalog, mut ids) = (catalog(), ItemIds::default());
        let mut a = item(&mut ids, 0, Quantity::Stack(15));
        let mut b = item(&mut ids, 0, Quantity::Stack(8));
        assert_eq!(merge_stack(&mut a, &mut b, &catalog.0[0]), 5);
        assert_eq!((a.quantity, b.quantity), (Quantity::Stack(20), Quantity::Stack(3)));
        let mut unknown = item(&mut ids, 0, Quantity::Stack(3));
        unknown.identified = false;
        let mut c = item(&mut ids, 0, Quantity::Stack(1));
        assert_eq!(merge_stack(&mut c, &mut unknown, &catalog.0[0]), 0);
    }

    #[test]
    fn new_items_roll_their_quantity_reproducibly() {
        let catalog = catalog();
        let (mut ids, mut rng) = (ItemIds::default(), GameRng::seeded(3));
        let rolled = make_item(&catalog.0[0], &mut ids, &mut rng, false, false);
        let Quantity::Stack(n) = rolled.quantity else { panic!() };
        assert!((2..=12).contains(&n));
        let full = make_item(&catalog.0[0], &mut ids, &mut rng, true, false);
        assert_eq!(full.quantity, Quantity::Stack(12));
        assert_ne!(rolled.id, full.id);
    }

    #[test]
    fn members_merge_stacks_then_fill_the_backpack_and_the_pool_takes_the_rest() {
        let (catalog, mut ids) = (catalog(), ItemIds::default());
        let mut carried = Carried::default();
        carried.equipment[EquipSlot::Secondary.index()] =
            Some(item(&mut ids, 0, Quantity::Stack(18)));
        let mut party = PartyInventory::default();
        party.receive(&mut carried, item(&mut ids, 0, Quantity::Stack(5)), &catalog).unwrap();
        // Two topped up the worn stack; three took a backpack slot.
        assert_eq!(carried.worn(EquipSlot::Secondary).unwrap().quantity, Quantity::Stack(20));
        assert_eq!(carried.backpack[0].as_ref().unwrap().quantity, Quantity::Stack(3));
        for _ in 1..BACKPACK_SLOTS {
            party.receive(&mut carried, item(&mut ids, 1, Quantity::One), &catalog).unwrap();
        }
        // The backpack is full: new things go to the front of the pool.
        party.receive(&mut carried, item(&mut ids, 3, Quantity::One), &catalog).unwrap();
        party.receive(&mut carried, item(&mut ids, 2, Quantity::One), &catalog).unwrap();
        assert_eq!(party.pool.iter().map(|i| i.item).collect::<Vec<_>>(), [ItemId(2), ItemId(3)]);
    }

    #[test]
    fn the_pool_has_a_limit() {
        let (catalog, mut ids) = (catalog(), ItemIds::default());
        let mut party = PartyInventory::default();
        for _ in 0..POOL_LIMIT {
            party.place_in_pool(item(&mut ids, 1, Quantity::One), &catalog).unwrap();
        }
        assert!(party.place_in_pool(item(&mut ids, 1, Quantity::One), &catalog).is_err());
    }

    #[test]
    fn hands_follow_two_handed_and_pairing_rules() {
        let catalog = catalog();
        let mut ids = ItemIds::default();
        let (sword, axe, shield, arrows) =
            (&catalog.0[1], &catalog.0[2], &catalog.0[3], &catalog.0[0]);
        let mut carried = Carried::default();
        assert!(allowed_slots(axe, &carried, &catalog, None).contains(&EquipSlot::Primary));
        carried.equipment[EquipSlot::Secondary.index()] = Some(item(&mut ids, 3, Quantity::One));
        // A shield in the off hand bars a two-handed weapon from that set.
        assert_eq!(allowed_slots(axe, &carried, &catalog, None), [EquipSlot::AlternatePrimary]);
        carried.equipment[EquipSlot::Primary.index()] = Some(item(&mut ids, 2, Quantity::One));
        assert!(!allowed_slots(shield, &carried, &catalog, None).contains(&EquipSlot::Secondary));
        assert!(!can_hold_together(axe, shield));
        assert!(can_hold_together(sword, shield));
        // Ammunition only pairs with its weapon kind.
        assert!(!can_hold_together(sword, arrows));
    }

    #[test]
    fn splitting_a_stack_makes_a_new_item_and_crucial_ones_stay_whole() {
        let (catalog, mut ids) = (catalog(), ItemIds::default());
        let mut party = PartyInventory::default();
        party.place_in_pool(item(&mut ids, 0, Quantity::Stack(10)), &catalog).unwrap();
        let original = party.pool[0].id;
        party.pick_up(None, ItemLocation::Pool(0), 4, &mut ids, &catalog).unwrap();
        let held = party.held.clone().unwrap();
        assert_eq!(
            (held.item.quantity, party.pool[0].quantity),
            (Quantity::Stack(4), Quantity::Stack(6))
        );
        assert_ne!(held.item.id, original);
        // Taking the rest adds to the hand and closes the pool row.
        party.pick_up(None, ItemLocation::Pool(0), 6, &mut ids, &catalog).unwrap();
        assert!(party.pool.is_empty());
        assert_eq!(party.held.as_ref().unwrap().item.quantity, Quantity::Stack(10));
        party.deposit_held(0, &catalog).unwrap();
        party.place_in_pool(item(&mut ids, 5, Quantity::Stack(3)), &catalog).unwrap();
        assert_eq!(
            party.pick_up(None, ItemLocation::Pool(0), 1, &mut ids, &catalog),
            Err(Refusal::Crucial)
        );
    }

    #[test]
    fn a_worn_curse_holds_until_released_and_moving_loses_the_release() {
        let (catalog, mut ids) = (catalog(), ItemIds::default());
        let mut wearer = Character::new(
            "Wearer",
            crate::game::characters::Race::Human,
            crate::game::characters::Gender::Male,
            crate::game::characters::Profession::Fighter,
        );
        let head = Slot::Equipment(EquipSlot::Head);
        let mut carried = Carried::default();
        let mut party = PartyInventory {
            held: Some(Held { item: item(&mut ids, 4, Quantity::One), origin: None }),
            ..default()
        };
        party.swap_with_hand(&wearer, &mut carried, 0, head, &catalog).unwrap();
        assert!(party.held.is_none());
        assert_eq!(
            party.swap_with_hand(&wearer, &mut carried, 0, head, &catalog),
            Err(Refusal::Cursed)
        );
        carried.equipment[EquipSlot::Head.index()].as_mut().unwrap().curse_released = true;
        party.swap_with_hand(&wearer, &mut carried, 0, head, &catalog).unwrap();
        assert!(!party.held.as_ref().unwrap().item.curse_released, "moving loses the release");
        // Something that does not belong there is refused.
        let feet = Slot::Equipment(EquipSlot::Feet);
        assert_eq!(
            party.swap_with_hand(&wearer, &mut carried, 0, feet, &catalog),
            Err(Refusal::DoesNotFit)
        );
        // Put it back on: the curse holds again, but not on the dead.
        party.swap_with_hand(&wearer, &mut carried, 0, head, &catalog).unwrap();
        wearer.hp = 0;
        assert_eq!(party.swap_with_hand(&wearer, &mut carried, 0, head, &catalog), Ok(()));
    }

    #[test]
    fn using_spends_stacks_and_uses() {
        let mut ids = ItemIds::default();
        let mut stack = item(&mut ids, 0, Quantity::Stack(1));
        assert!(spend_use(&mut stack, false));
        let mut wand = item(&mut ids, 1, Quantity::Uses(1));
        assert!(!spend_use(&mut wand, true));
        assert_eq!(wand.quantity, Quantity::Uses(0));
        assert!(spend_use(&mut item(&mut ids, 1, Quantity::Uses(1)), false));
    }

    #[test]
    fn pool_weight_goes_to_whoever_has_most_room() {
        // Half of 40 is shared: the roomier member first, then evenly by
        // free fraction; the dead carry nothing.
        let shares = pool_shares(40, &[Some((0, 100)), Some((50, 100)), None]);
        assert_eq!(shares, vec![20, 0, 0]);
        let shares = pool_shares(140, &[Some((0, 100)), Some((50, 100))]);
        assert_eq!(shares.iter().sum::<u32>(), 70);
        assert_eq!(shares, vec![60, 10]);
    }

    #[test]
    fn a_pool_row_comes_into_the_hand_and_goes_back_with_a_member() {
        let catalog = ItemCatalog(vec![definition(0, EquipClass::Misc)]);
        let mut ids = ItemIds::default();
        let mut rng = GameRng::seeded(1);
        let mut party = PartyInventory::default();
        for _ in 0..2 {
            party.pool.push(make_item(&catalog.0[0], &mut ids, &mut rng, true, true));
        }
        let first = party.pool[0].id;
        party.take_from_pool(0).unwrap();
        assert_eq!(party.pool.len(), 1);
        assert_eq!(party.held.as_ref().map(|h| h.item.id), Some(first));
        assert_eq!(party.take_from_pool(0), Err(Refusal::Nothing), "the hand is full");
        let mut carried = Carried::default();
        party.put_away_held(&mut carried, &catalog);
        assert!(party.held.is_none());
        assert_eq!(carried.backpack[0].as_ref().map(|i| i.id), Some(first));
    }
}
