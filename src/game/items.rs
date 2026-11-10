//! Items (`docs/rules/items.md`).
//!
//! An item is an entity. What kind of thing it is was read from its
//! definition as it was made and is its components; where it is, is a
//! relationship to what holds it. Moving an item changes that relationship
//! and nothing else, so an item is never in two places or none.

use bevy::ecs::relationship::RelationshipTarget;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use enum_map::EnumMap;
use enumset::EnumSet;

use crate::content::characters::{Attribute, Gender, Profession, Race, Skill};
use crate::content::items::{EquipClass, ItemDefinition, ItemId, ItemKind, QuantityKind};
use crate::content::species::Reach;
use crate::game::inventory::EquipSlot;

/// How many things a member's pack holds.
pub const PACK_SLOTS: usize = 8;
/// How many the party's pool holds.
pub const POOL_LIMIT: usize = 500;

/// An item, and the definition it was made from, which it is named and
/// shown by.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[require(Known)]
pub struct Item(pub ItemId);

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
    pub kind: ItemKind,
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
            kind: definition.kind,
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

/// Whether `other` may be held with the ranged weapon `ranged` (retail
/// `CompatiblePartnerItems`): bows, crossbows and slings with their
/// missiles, guns with what loads them, and each omnigun (by which one it
/// is) with what it has learnt to fire.
pub fn partners((ranged, which): (&Wearable, Item), other: &Wearable) -> bool {
    use ItemKind as K;
    let kind = other.kind;
    let dagger = other.class == EquipClass::ThrownWeapon && kind == K::DAGGER;
    let small = matches!(kind, K::SLING_STONE | K::POWDER_SHOT);
    let stars = matches!(kind, K::DARTS | K::SHURIKEN | K::THROWING_SHURIKEN);
    match ranged.kind {
        K::LONGBOW | K::GREATBOW => kind == K::ARROW,
        K::CROSSBOW => kind == K::QUARREL,
        K::SLING => kind == K::SLING_STONE,
        K::POWER_GLOVE | K::PHASER | K::MUSKET => {
            (u8::from(other.class) > u8::from(EquipClass::RangedWeapon)
                || ranged.reach == other.reach)
                && loaded_by(ranged.kind).is_none_or(|loads| loads == kind)
        }
        K::OMNIGUN => match which.0.0 {
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
pub fn hold_together(a: (&Wearable, Item), b: (&Wearable, Item)) -> bool {
    use EquipClass::*;
    if a.0.two_handed || b.0.two_handed {
        return false;
    }
    if a.0.class == RangedWeapon || b.0.class == Ammunition {
        return partners(a, b.0);
    }
    if b.0.class == RangedWeapon || a.0.class == Ammunition {
        return partners(b, a.0);
    }
    u8::from(a.0.class) >= u8::from(Ammunition)
        || u8::from(b.0.class) >= u8::from(Ammunition)
        || a.0.reach == b.0.reach
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

pub(crate) fn plugin(app: &mut App) {
    app.add_observer(stack).add_observer(put);
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
}
