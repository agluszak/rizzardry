# Items

An item is a thing: a sword, a stack of arrows, a potion with charges left.
This page is the specification for slice 3 of [the plan](../plan.md). Where the
code differs, the code is wrong. Numbers and formulas are the original's and
are cited beside the code that holds them.

## What an item is

An item is an entity. Its identity is the entity; nothing allocates ids for
items and nothing addresses one by owner and index.

What kind of item it is, is read from its definition **once, when it is
made**, and becomes components. A rule reads the components of the item in
front of it and never looks a definition up again.

| Component | The fact it holds | On which items |
| --- | --- | --- |
| `Item` | Which definition it was made from: its name, picture and model are shown from it | all |
| `Count` | How many of it this one entity stands for | what stacks |
| `Charges` | Uses or shots left | what is used up by use |
| `Weight` | What one of it weighs | all |
| `Known` | What the party knows of it: what it is, its spell, its curse | all |
| `Wearable` | Where it may be worn, whether it takes both hands, whether it may go in the off hand, who may use it | what can be worn or wielded |
| `Weapon` | Its skill, reach, dice, the ways it strikes, what its blows do besides damage, what it slays, its bonuses to hit, damage and swings | what strikes |
| `Launcher` | The kind of ammunition it shoots | bows, slings, the omnigun |
| `Ammunition` | Its dice and what it does, and whether it is used up when thrown or shot | arrows, bolts, stones, thrown weapons |
| `Protection` | Armour it adds where it is worn, resistances, attribute and skill bonuses, regeneration | what protects or bestows |
| `Casts` | The spell it casts when used, at what power, by which skill | what casts |
| `Cursed` | It cannot be taken off while its curse holds | cursed items |
| `Crucial` | The party may not part with it | quest items |

A definition field no rule reads does not become a component.

## Where an item is

An item is in exactly one place, and the place is a relationship from the
item to what holds it:

| Relationship | Held by | Carries |
| --- | --- | --- |
| `Worn` | a member | the slot it is worn in |
| `Packed` | a member | the backpack slot |
| `Pooled` | the party | — the party's pool keeps its order |
| `InHand` | the party | — at most one |
| `Stowed` | a container in the world | — |
| lying | the level, by a position | — |
| `Loot` | a monster, until it dies | — |

Moving an item is changing that relationship. There is no "remove from here,
insert there" pair to keep in step, and an item cannot be in two places or
none.

The stored weapon set is two slots like any other; an item worn there is not
in use, so its curse does not hold and it adds nothing.

## Stacks

An item with a `Count` stands for that many alike. Taking some from it makes a
second item entity with the rest of the facts copied; putting two alike
together adds the counts and one entity goes. Two are alike when they were
made from the same definition and the party knows the same of each.

## What happens to items

Each is one operation that says what it did. None of them knows which screen
asked.

- **take**: an item lying, stowed or dropped as loot comes to a member, to the
  pool if they have no room, and stays where it is if neither has.
- **move**: an item goes to a slot, a pack slot, the pool or the hand. What was
  there goes to the hand. A slot refuses what does not fit it, what its wearer
  may not use, and what the other hand rules out; a cursed item worn in use
  refuses to leave.
- **drop**: an item leaves the party and lies where the party stands; a
  crucial one refuses.
- **use**: its spell is cast and a charge or one of the count is spent; at
  none it is gone unless it keeps when empty.
- **load**: ammunition goes to the other hand of its launcher, charges into
  what holds them.
- **merge**: two items a member knows how to combine become a third.
- **identify**: the party comes to know what it is.

## What follows from where items are

- **Refit.** What a member wears is one of the sources of their figures: a
  thing coming onto them or leaving them, or a worn thing changing, leaves
  the figures stale, and they are written anew as bodies are settled
  ([actors.md](actors.md), "Refit"). Nothing that moves an item says so.
- **Burden.** What a member carries weighs on them: what they wear and pack,
  and their share of the pool. It is weighed as bodies are settled, from
  what is carried then; a fight keeps the loads it began with.
- **A member's arms.** What a member strikes with is read from the weapons in
  the two hands of the set in use: a weapon in a hand is an arm; an empty hand
  is a fist by Martial Arts; a launcher needs its ammunition in the other
  hand; the off hand strikes only beside a main hand of the same kind (close
  or ranged), never as a shield or ammunition. The arm's numbers come from the
  weapon's components and the member's attributes and skills.

## What goes

`StackId`, `ItemIds`, `ItemStack`, `Equipped`, `Carried`, `ItemPool`, `Hand`,
`Slot`, `MemberSlot`, `ItemOwner`, `ItemRef`, `ItemAt`, `PoolPosition`,
`content.items[..]` in any rule, `Grip` and `combat/attack.rs` as a port of the
original's hand table, `distribute_burden` as a system that scans for changes,
and the watch on a member's gear that fires refit.
