# Rewrite plan

`game/` is being rewritten from the game's rules up, by the rules in
[design.md](design.md). The current code is a port of the original's procedures
and record layouts onto an ECS; cleaning it does not change that. Each slice
below replaces one part of the model. A slice is done when the names it lists
are gone, the game plays, and its rules are written in `docs/rules/`.

Every slice starts with a contract: what must be gone, the only new types
allowed, and its expected size. A type outside the contract means stopping to
ask.

## Where the port shows

| Symptom | What it is really |
| --- | --- |
| `judge_defences`, `judge_fighters`, `judge_sides`, `place_bodies` | Members and monsters are two record types, reconciled every step |
| `StackId`, `ItemIds`, `ItemRef`, `ItemAt`, `Slot`, `MemberSlot` | Items are values in arrays with hand-made ids and addresses |
| Conditions and enchantments as `Vec`s inside `Vitals` | Things with their own lifetime stored as fields |
| `content.species[..]`, `content.items[..]` in rules | Entities hold foreign keys; rules join by hand |
| `KeptMonster`, `LevelRecord`, `LevelMemory`, NPC `Registry`, `StoredCharacter` | Entities serialised into records and rebuilt |
| `profession_row`, `race_attributes`, trait lists, `define_spell` | Tables written as code |
| Phases 1–100, `Turn` flags, `Pause` | The original's scheduler |
| Functions named and shaped after retail routines | The procedure was the unit of design |

## Slices

### 1. One kind of actor — partly done

**Rule.** An actor has attributes, a level, health and stamina, things it can
strike with, armour, resistances and a side. A member's come from the sheet and
what they wear; a monster's from its species. After it exists, its numbers
change only when something changes them: gear put on or taken off, a level
gained, a spell laid or lifted.

**Gone.** `judge_defences`, `judge_fighters`, `judge_sides`, `place_bodies` and
every per-frame or per-step re-derivation; `CharacterStats`, `.stats(`,
`.base_stats()`; `member_fighter`/`monster_fighter`,
`character_defences`/`species_defences` and any other member/monster pair;
`content.species[..]` in any rule under `game/combat`, `game/hits`,
`game/magic`; `Monster { species }` as something rules read.

**New types, and no others.** `Attributes`, `Level`, `Resistances`, `Armour`,
`Attacks`: components every actor has, written when it is made. `Modifier`: one
stat change with its source, added and removed with what causes it. `Refit`: the
one event fired at an actor when its gear, level or modifiers change, and the
one observer that answers it.

**Where member and monster formulas differ** in the original (initiative,
armour, to-hit), the difference lives at creation and at `Refit`, as the source
of the numbers. No formula is unified where the original differs; that would be
a gameplay change.

**Size.** `fighters.rs`, `defences.rs`, `footing.rs`, `attack.rs` and the stat
half of `characters.rs` are about 3,300 lines. The replacement should be under
1,500.

### 2. Spells and conditions on an actor are things

An enchantment or a condition is laid, lasts, and is lifted. Each is an entity
related to its bearer, with its own duration; laying and lifting it are the
events that change its bearer. `Vitals` is health and stamina.

### 3. Items are things

An item is an entity. It is in a pack, worn in a slot, lying in the world or in
a container, by relationship. `StackId`, `ItemIds` and the addressing enums go.
An item's definition becomes components when it is made.

### 4. Levels keep what is in them

A level that is left stays, dormant, with its monsters, items and devices. The
mirror records and the code that copies entities into and out of them go. The
same for NPCs who are not present and members who are not in the party.

### 5. Tables are content

Professions, races, traits, spell effects, special attacks and trap types are
data in `content`, loaded like the rest. The code that reads them is written
once per kind of rule, not once per row.

### 6. A round

What a round of combat is, in the game's terms: who acts when, and what the
player sees. Designed from that, not from the original's phase counter.

### 7. Presentation reads the new model

The screens are rebuilt over the model above as each slice lands, keeping views
alive and changing what changed. Visual parity is checked in the running game;
headless tests do not establish it.

## Status

| Slice | State |
| --- | --- |
| Groundwork: combat, effects and magic resolved through messages and plain functions | Done on `refactor/event-driven-combat-and-magic`. An improvement by the old standard only: the functions are still retail routines. Slices 1 and 6 replace them. |
| 1. One kind of actor | Stats done: `Attributes`, `Level`, `Resistances`, `Armour`, `Attacks` on every actor, written by one `Refit` observer; `judge_defences`, `judge_fighters`, `CharacterStats` and the member/monster stat pairs are gone; rules read no species record. Still to do: `Side` (`judge_sides`) and where an actor stands (`place_bodies`) are re-derived each step; `Refit` is fired by watching components for writes, and any write to `Vitals` counts until slice 2 moves spells out of it; a few readers outside combat still rebuild `Attributes` from the sheet instead of reading the component (burden, recovery, the spell screens). `Modifier` turned out not to be needed and was not added. |
| 2–7 | Not started. |
