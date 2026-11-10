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

### 1 and 2. One kind of actor, and what is on it — in progress

Done together: the stats cannot be changed by events until spells are apart
from health, and side cannot until conditions are.

**Rules.** [rules/actors.md](rules/actors.md).

**Gone.**
- `judge_defences`, `judge_fighters`, `judge_sides`, `place_bodies`,
  `sync_member_maxima`, `notice_changes` and every other per-frame or per-step
  re-derivation of something about an actor.
- `Footing`, `Eyes`, `Conspicuous` as components copied onto actors.
- `CharacterStats`, `.stats(`, `.base_stats()`, and `Attributes::with_spells`
  or `Attributes::of_sheet` anywhere an actor exists to be read.
- `member_fighter`/`monster_fighter`, `character_defences`/`species_defences`
  and any other member/monster pair outside refit.
- `content.species[..]` in any rule under `game/combat`, `game/hits`,
  `game/magic`.
- `Vitals::enchantments`, `Vitals::conditions`, `Vitals::poison`,
  `Vitals::cheated_death_in`, `Vitals::fell_from`; `PartyEffects`.
- `Stances`, `monster_own`, `monster_stance`.
- `game/combat/attack.rs` as a port of `CalcAttacks`/`CalcArmorClasses`: the
  member's numbers are derived in refit from the rule page, with the formulas
  cited, not from a `Grip` that mirrors the original's hand table.

**New types, and no others.** Components every actor has: `Attributes`,
`Level`, `Resistances`, `Armour`, `Attacks`, `Vitals`, `Conditions`, `Spells`,
`Side`. Events: `Refit`, `Afflicted`, `Recovered`, and the existing `Damaged`
and `Dead`. One accessor that computes an actor's place when asked.

**Not allowed.** A component that mirrors another. A system that scans actors
for changes. A helper that rebuilds a stored fact from its sources outside
refit.

**Size.** `characters.rs`, `attack.rs`, `footing.rs`, `hostility.rs`,
`enchantments.rs`, `recovery.rs` and the actor parts of `hits/` are about 5,500
lines. The replacement should be under 3,000.

**Behaviour that changes, on purpose.** Conditions and spells are lifted from
the dead at death rather than at the next settlement. A member's trait
immunities apply to every affliction that allows a save. Nothing else; where
the original's member and monster formulas differ, both are kept.

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
| 1 and 2. One kind of actor, and what is on it | Mostly landed. Gone: the per-step judges, `place_bodies`, `Footing`/`Eyes`/`Conspicuous`, `judge_sides`, `sync_member_maxima`, `notice_changes`, `Vitals::enchantments`, `PartyEffects`. Spells laid are entities related to their bearer; place and sight are computed on read; side is stored and changed by `Stand`, faction changes and joining. Where the contract was changed while designing, the rule page says why: conditions stay in `Vitals`; Turncoat is read where the side is read, so there are no `Afflicted`/`Recovered` events; `Loyalty` and `Allegiance` are written when a monster is made. **Still owed:** `combat/attack.rs` is still the port of `CalcAttacks`/`CalcArmorClasses` (`Grip`); refit after a sheet or load change is noticed by a write to `Character`/`Burden` rather than fired by the operation; deaths are settled by scanning changed `Vitals` rather than from `Damaged`; the party's own sight (`world/perception.rs`) still reads species records; `engage`, `leave_fight` and the monster AI are redesigned only as far as place and side, their shape is slice 6's. |
| 3–7 | Not started. |
