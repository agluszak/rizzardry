# Tables

This page is the specification for slice 5 of [the plan](../plan.md).

## What a table is

Much of the game is rows: what each profession gains a level, what each race
starts with, which traits a profession or race has, what each spell does to
whom, what each special attack does, what each kind of trap does, what loads
which gun, what each omnigun can fire. The original keeps these in its
program, not in its data files, so they come with this project, not with the
installation.

A table is **data**: a file of rows under `assets/tables/`, read with the rest
of the content into the catalogs. It is not a `match` with one arm per row.
A row can be read, compared with the original and corrected without reading
any code.

## What code is left

For each kind of row there is one piece of code that knows what such a row
*means*, written once: how a spell's effects are applied, how a profession's
row turns into a level's gains. It never names a particular spell, profession
or item. Where the original truly has a rule for one thing only (Ring of Fire
pulsing on those around the party; the rocket launcher swinging once), that is
a fact in that thing's row — a flag, a kind of effect — not a test of its id
in the code that reads rows.

## Where rows are read

When a thing is made, as everywhere else: a spell's effects when its
definition is loaded, a weapon's facts when the item is made, a profession's
row when a character gains a level. A rule in play reads components and
catalog entries it was handed, and does not look a row up by id to decide how
to behave.

## The tables

| Table | Rows | Replaces |
| --- | --- | --- |
| professions | attribute and skill gains, hit points, stamina, spell books, traits | `profession_row`, the trait lists in `Character::traits` |
| races | starting attributes, resistances, traits | `race_attributes`, race tests in resistance and armour |
| spell effects | per spell: each effect, whom it lands on, its roll, save and resistance | `define_spell` |
| special attacks | per kind: each effect as a row like a spell's, whether a silence stops it, what it summons | the special-attack `match`, and the code that spelled out its five effect words |
| traps | per kind: what it does | the trap `match` in `world/interaction` |
| spells laid | per spell: what it adds while laid, whether it is personal, lasts only a fight, pulses | `spells::bonus`, `personal`, `combat_only`, `OFFENSIVE` |
| ammunition | what loads which gun, what each launcher and each omnigun fires, what strength does not help | `LOADED_BY`, `partners`, the kind lists in `items::shape` |

## What goes

Every function that returns a different constant for each profession, race,
spell, special attack, trap or item kind; every `SpellId::X =>` and
`ItemKind::X =>` arm outside the code that reads the tables into the catalogs.
