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

**Rules.** [rules/items.md](rules/items.md).

An item is an entity. It is in a pack, worn in a slot, lying in the world or in
a container, by relationship. `StackId`, `ItemIds` and the addressing enums go.
An item's definition becomes components when it is made.

This slice also takes what slices 1 and 2 could not finish without it: a
member's arms are read from the weapons in their hands, which replaces `Grip`
and `combat/attack.rs`; wearing and removing refit by the relationship
changing, which removes the watch on a member's gear; and burden is worked
out when what is carried changes.

**Gone.** `StackId`, `ItemIds`, `ItemStack`, `Equipped`, `Carried`, `ItemPool`,
`Hand`, `PartyInventory` (but for the purse), `Slot`, `MemberSlot`,
`ItemOwner`, `ItemRef`, `ItemAt`, `PoolPosition`, `WorldItem`/`GroundLoot`
holding stacks, `Contents` holding stacks; `content.items[..]` in any rule
outside where an item is made or shown; `Grip`, `Holds`, and
`combat/attack.rs`'s member half; `distribute_burden` and `refit_regeared`.

**New types, and no others.** On an item: `Item`, `Count`, `Charges`, `Known`,
and the components its definition becomes (`Weight`, `Wearable`, `Weapon`,
`Launcher`, `Ammunition`, `Protection`, `Casts`, `Cursed`, `Crucial`). Where it
is: the relationships `Worn`, `Packed`, `Pooled`, `InHand`, `Stowed`, `Loot`
and their targets. One read accessor for "what does this member wear and
pack". The operations of the rule page as events an observer carries out:
`Take`, `Move`, `Drop`, `Use`, `Load`, `Merge`, `Identify`, each saying what it
did with `ItemNews`. A destination is one enum (`Place`): a slot of a member, a
pack slot, the pool before some item or at its end, the hand.

**Not allowed.** An id or index that stands for an item. A value copy of an
item moved between collections. An operation that takes an item out of one
place and puts it in another as two steps another rule could see between.

**Order of work.** The item components and relationships and the operations
over them, with their rules tested through the world; then the party's
screens over them; then world items, containers, loot and trade; then a
member's arms and refit from what is worn, which deletes `Grip`; then burden.
Members out of the party and levels that were left still copy what they hold
into records until slice 4; that boundary converts to and from item entities
and is the only place a value copy of an item survives.

### 4. Levels keep what is in them

**Rules.** [rules/levels.md](rules/levels.md).

A level that is left stays, set aside, with its monsters, items and devices. The
mirror records and the code that copies entities into and out of them go. The
same for NPCs who are not present and members who are not in the party.

**Gone.** `LevelMemory`, `LevelRecord`, `LevelContents`, `KeptMonster` as a
record, `KeptItem`, `TriggerMemory`, `KeptDevice`, `DeviceRecords`, `Profile`,
`StoredCharacter`, `NpcPopulation::depart`, `ItemStack`, `unpack`,
`Items::stack`.

**New types, and no others.** None for the levels themselves: setting aside is
`Disabled`. One event for a level waking, carrying how long it was away, which
the rules that owe it time react to.

**First step.** An experiment, not a design: disable a level's entities with
the party elsewhere and find out what physics, navigation and drawing still
see of it.

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
| 1 and 2. One kind of actor, and what is on it | Mostly landed. Gone: the per-step judges, `place_bodies`, `Footing`/`Eyes`/`Conspicuous`, `judge_sides`, `sync_member_maxima`, `notice_changes`, `Vitals::enchantments`, `PartyEffects`. Spells laid are entities related to their bearer; place and sight are computed on read; side is stored and changed by `Stand`, faction changes and joining. Where the contract was changed while designing, the rule page says why: conditions stay in `Vitals`; Turncoat is read where the side is read, so there are no `Afflicted`/`Recovered` events; `Loyalty` and `Allegiance` are written when a monster is made. Deaths settle from `Damaged`; practice is one `Practise` event whose `Improved` refits the member, and advancement refits when it is accepted. **Still owed:** `combat/attack.rs` is still the port of `CalcAttacks`/`CalcArmorClasses` (`Grip`), and refit after gear or load changes is still noticed by a write to `Carried`/`Burden`: both go with slice 3, where items are things. The party's own sight (`world/perception.rs`) still reads species records; `engage`, `leave_fight` and the monster AI are redesigned only as far as place and side, their shape is slice 6's. |
| 3. Items are things | First phase landed: everything the party holds is item entities placed by relationship (`game/items.rs`), changed by events with one observer each; `Carried`, `ItemPool`, `StackId`, `ItemIds` and the addressing enums are gone; wearing refits and holdings reweigh by the relationships changing. Then what lies in the world, sits in a container or falls as loot became item entities too (`Lying`, `Stowed`), taken by `Give` and thrown by `Throw`. Then a member's arms and armour: what an item is to a hand is components set when it is made (`Weapon`, `Launcher`, `Missile`, `Shield`, `Protection`), and refit reads the two hands ([rules/arms.md](rules/arms.md)); `Grip` and the member half of `combat/attack.rs` are gone. **Still owed:** rules still look definitions up for the spell an item casts (`magic/casting.rs`), recipes, identify difficulty, burning by class, trade values and the omnigun's tiers; plain item values (`ItemStack`) survive only in the records of levels that were left and of characters out of the party, which slice 4 removes; no test yet for `Identify`, burning, or the omnigun. The screens were not checked in the running game. |
| 4. Levels keep what is in them | Rules and contract written; not started. |
| 5–7 | Not started. |
