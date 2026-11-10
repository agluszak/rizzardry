# Levels and who is in them

This page is the specification for slice 4 of [the plan](../plan.md). Where
the code differs, the code is wrong.

## A level that is left stays

A level is an entity, and everything in it belongs to it: its ground and
props, its devices and what they remember, its monsters and their groups, the
items lying in it and stowed in its containers, its map notes.

When the party leaves, none of that is copied anywhere. The level and
everything it owns are **set aside**: still there, with every component they
had, but taking no part in anything until the party comes back. Coming back
**wakes** them. A level the party has never been to is built from its
definition, once.

Nothing is rebuilt from a record, so nothing can be forgotten by a record:
a monster keeps its health, its conditions, the spells laid on it and the side
its group was made to stand on; a door keeps how far it had swung; an item
keeps what the party knows of it.

Setting aside is Bevy's own: an entity that is `Disabled` is skipped by every
query that does not ask for it. A level left is its entities disabled; a
level woken is that removed.

## What time does to a level that was away

A level does not run while it is away. What should have happened in the
meantime happens as it wakes, from how long it was away: wandering encounters
come or go, a stance set on a group wears off, a spell laid runs out. Each is
the same rule that runs while the party is there, given the time that passed.

## Who is not in the party

A character is an entity whether or not they are in the party. One who is
dismissed, or has not yet joined, is a character without a seat: they keep
their sheet, their health and what ails them, and what they wear and pack
stays worn and packed, as item entities. Joining gives them a seat; nothing is
unpacked.

What the player can see of an NPC who may join (retail `NPC Manager.cpp`,
`DismissNpcFromParty` and the join beside it):

- Standing in a level they fight and are hurt as a monster of the species that
  is their likeness, with the health and stamina that species rolls, rolled
  anew each time they come to stand there.
- Seated in the party they have their sheet's health and stamina, as they left
  them when last seated.
- What ails them goes with them both ways. Dismissed out cold without end,
  they stand with no more stamina than they had seated.
- Dismissed dead, they do not stand at all.

That is all the original is evidence of. That it keeps a character record and
makes a separate monster record is its layout, and decides nothing here.

An NPC who stands in a level is two entities here, for a reason of our own:
the body has a lifetime the person does not. It is made when they come to
stand in a level, set aside with that level, and gone when they take a seat,
and with it go its collider, its navigation and what is drawn of it. The
person (what was said to them, their sheet, their things, their stock) outlives
every body and is never set aside. The body is an actor like any monster, and
ends; what ails it is handed to the person as it ends, and to a new body as
one is made. Whether an NPC is dead is one fact in each case: one killed where
they stood stands nowhere again (`Npc::killed`), and one who dies seated is
dead as any member is.

One who is somewhere the party is not is set aside with that level, or on
their own if they are nowhere.

## What goes

`LevelMemory`, `LevelRecord`, `LevelContents::remember`, `KeptMonster` as a
record of a monster that exists (making a new monster from its species stays,
as a spawn), `KeptItem`, `TriggerMemory`, `KeptDevice`, `DeviceRecords`,
`NpcPopulation::depart` and the placement records it keeps, `Profile`,
`StoredCharacter`, and with them the last plain item values: `ItemStack`,
`unpack`, `Items::stack`.

## What has to be found out before this is built

Setting a level aside must also take it out of physics, navigation and
drawing. Whether disabling its entities does that with the physics and
navigation libraries in use, or whether their own handles must be removed and
restored too, is not known yet and is the first thing to try.
