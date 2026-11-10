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
use crate::content::items::{EquipClass, ItemDefinition, ItemId, ItemKind, QuantityKind};
use crate::content::species::Reach;
use crate::game::characters::{Character, Vitals};
use crate::game::data::Catalogs;
use crate::game::inventory::{EquipSlot, ItemNews, Refusal};

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
#[derive(Component, Debug)]
pub struct KeepsEmpty;

/// One is used up each time it is used: a stack's, or a drink.
#[derive(Component, Debug)]
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
#[derive(Component, Debug)]
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
    if definition.requires_worn() {
        item.insert(Wearable {
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
        });
    }
    item.id()
}

/// The items there are and where they are, to read.
#[derive(SystemParam)]
pub struct Items<'w, 's> {
    items: Query<'w, 's, (&'static Item, Option<&'static Count>, &'static Known, &'static Weight)>,
    worn: Query<'w, 's, &'static Worn>,
    packed: Query<'w, 's, &'static Packed>,
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
        let (Ok((item, Some(count), known, _)), Ok((other, Some(_), theirs, _))) =
            (self.items.get(target), self.items.get(source))
        else {
            return 0;
        };
        if target == source || item != other || known.identified != theirs.identified {
            return 0;
        }
        count.limit.max(count.of) - count.of
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
}

/// An item is used once: a charge is spent, or one of a stack, or the
/// thing itself. Spent out, it is gone, unless it keeps when empty.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Spend {
    #[event_target]
    pub item: Entity,
}

fn spend(
    event: On<Spend>,
    mut items: Query<(Option<&mut Charges>, Option<&mut Count>, Has<SpentByUse>, Has<KeepsEmpty>)>,
    mut commands: Commands,
) {
    let Ok((charges, count, spent_by_use, keeps)) = items.get_mut(event.item) else { return };
    let gone = match (charges, count) {
        (Some(mut charges), _) => {
            charges.left -= charges.left.min(1);
            charges.left == 0 && !keeps
        }
        (None, Some(mut count)) if spent_by_use => {
            count.of -= 1;
            count.of == 0
        }
        (None, _) => spent_by_use,
    };
    if gone {
        commands.entity(event.item).despawn();
    }
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
    let Give { item, to, party, pool_first } = *event;
    let Ok((&Item(id), known)) = shown.get(item) else { return };
    let mut left = items.units(item) as u16;
    // Fill what is alike, in order; whether any went.
    let mut fill = |onto: Vec<Entity>, left: &mut u16, commands: &mut Commands| {
        let before = *left;
        for onto in onto {
            let moved = items.room(onto, item).min(*left);
            if moved > 0 {
                commands.trigger(Stack { onto, from: item });
                *left -= moved;
            }
        }
        *left < before
    };
    let mut taker = None;
    let pool = |left: &mut u16,
                taker: &mut Option<Option<Entity>>,
                commands: &mut Commands,
                fill: &mut dyn FnMut(Vec<Entity>, &mut u16, &mut Commands) -> bool| {
        let rows = items.pool(party);
        let mut took = fill(rows.to_vec(), left, commands);
        if *left > 0 && rows.len() < POOL_LIMIT {
            commands.trigger(Put { item, at: Place::Pool { party, row: 0 } });
            (*left, took) = (0, true);
        }
        if took {
            *taker = Some(None);
        }
    };
    if pool_first {
        pool(&mut left, &mut taker, &mut commands, &mut fill);
    }
    if let Some(member) = to.filter(|_| left > 0) {
        let carried = items.carried(member).filter(|&other| other != item).collect();
        let mut took = fill(carried, &mut left, &mut commands);
        let free = items.pack(member).iter().position(Option::is_none);
        if let Some(slot) = free.filter(|_| left > 0) {
            commands.trigger(Put { item, at: Place::Pack { member, slot } });
            (left, took) = (0, true);
        }
        if took {
            taker = Some(Some(member));
        }
    }
    if !pool_first && left > 0 {
        pool(&mut left, &mut taker, &mut commands, &mut fill);
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
    members: Query<(&Character, &Vitals)>,
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
                    for onto in there.into_iter().chain(others) {
                        let moved = items.room(onto, held).min(left);
                        if moved > 0 {
                            commands.trigger(Stack { onto, from: held });
                            left -= moved;
                        }
                    }
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
    members: &Query<(&Character, &Vitals)>,
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
            commands.trigger(Give { item: leaving, to: Some(member), party, pool_first: false });
        }
        return Ok(());
    }
    commands.trigger(Put { item: held, at });
    if let Some(there) = there {
        commands.trigger(Put { item: there, at: Place::Hand { party } });
    }
    Ok(())
}

pub(crate) fn plugin(app: &mut App) {
    app.add_message::<Refused>()
        .add_observer(stack)
        .add_observer(put)
        .add_observer(load)
        .add_observer(spend)
        .add_observer(give)
        .add_observer(swap);
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
        for observer in [
            Observer::new(stack),
            Observer::new(put),
            Observer::new(load),
            Observer::new(spend),
            Observer::new(give),
            Observer::new(swap),
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
        let member = world.spawn((sheet, vitals)).id();
        let party = world.spawn_empty().id();
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
        world.trigger(Give { item: quiver, to: Some(member), party, pool_first: false });
        world.flush();
        let more = new(&mut world, ARROWS, 30);
        world.trigger(Give { item: more, to: Some(member), party, pool_first: false });
        world.flush();
        let pack = world.run_system_once(move |items: Items| items.pack(member)).unwrap();
        assert_eq!(pack[..2], [Some(quiver), Some(more)]);
        let count = |world: &World, item| world.get::<Count>(item).unwrap().of;
        assert_eq!((count(&world, quiver), count(&world, more)), (100, 20));
        // A full pack sends the next thing to the pool.
        for _ in 2..PACK_SLOTS {
            let bow = new(&mut world, BOW, 1);
            world.trigger(Give { item: bow, to: Some(member), party, pool_first: false });
            world.flush();
        }
        let extra = new(&mut world, BOW, 1);
        world.trigger(Give { item: extra, to: Some(member), party, pool_first: false });
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
}
