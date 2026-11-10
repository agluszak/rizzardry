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

**Rules.** [rules/tables.md](rules/tables.md).

Professions, races, traits, spell effects, special attacks and trap types are
data in `content`, loaded like the rest. The code that reads them is written
once per kind of rule, not once per row.

**Gone.** `profession_row`, `race_attributes`, the trait lists, `define_spell`,
the special-attack and trap `match`es, `spells::bonus`/`personal`/
`combat_only`/`OFFENSIVE`, `LOADED_BY`, `partners`, and every arm that names a
spell, profession, race or item kind outside the loader.

**New types, and no others.** One row type per table, deserialised from its
file; the catalogs gain one field per table.

**Order of work.** Spell effects first (the largest, and `EffectSpec` is
already nearly a row), then spells laid, then professions and races, then the
small ones.

### 6. A round

What a round of combat is, in the game's terms: who acts when, and what the
player sees. Designed from that, not from the original's phase counter.

**Rules.** [rules/round.md](rules/round.md).

**Gone.** `RoundPhase` and the loop that steps a counter from 1 to 100 looking
for someone due; the messages `Called` and `Perform` and the `Chosen` enum
that carry one actor's moment between four systems chained on the fixed step
(`call_actor`, `take_turn`, `perform`/`swings::attack`, `casting::cast`,
`defend_instead`, `summon`); `Release` as a message drained by a later system;
`Turn::phase == 0` as "done"; turn fields named for the original's where a
plainer fact says the same.

**New types, and no others.** `Moment { actor }`, an event at the actor whose
time has come. One event per thing an actor can do with it, each with one
observer that does all of it: `Strike`, `Cast`, `Breathe`, `Guard`, `Retreat`,
`Hold`. What a deed does lands as it is released, in the same chain, so a
moment is decided, done and settled before the round looks for the next. On
the fight: the round's timeline as data (who is still to act and when), from
which the earliest is taken.

**Not allowed.** A system that runs every step to see whether a message
arrived. A marker component whose only job is to hand an actor from one system
to the next. A function that exists because the original has one of that name.

**Order of work.** The timeline and `Moment`; then deeds landing as they are
released; then each act as its own event, the attack last because it is the
largest; then the end of the round as what follows the last moment rather than
a chain gated on a marker.

### 7. Presentation reads the new model

The screens are rebuilt over the model above as each slice lands, keeping views
alive and changing what changed. Visual parity is checked in the running game;
headless tests do not establish it.

## Status

| Slice | State |
| --- | --- |
| Groundwork: combat, effects and magic resolved through messages and plain functions | Done on `refactor/event-driven-combat-and-magic`. An improvement by the old standard only: the functions are still retail routines. Slices 1 and 6 replace them. |
| 1 and 2. One kind of actor, and what is on it | Mostly landed. Gone: the per-step judges, `place_bodies`, `Footing`/`Eyes`/`Conspicuous`, `judge_sides`, `sync_member_maxima`, `notice_changes`, `Vitals::enchantments`, `PartyEffects`. Spells laid are entities related to their bearer; place and sight are computed on read; side is stored and changed by `Stand`, faction changes and joining. Where the contract was changed while designing, the rule page says why: conditions stay in `Vitals`; Turncoat is read where the side is read, so there are no `Afflicted`/`Recovered` events; `Loyalty` and `Allegiance` are written when a monster is made. Deaths settle from `Damaged`; practice is one `Practise` event whose `Improved` refits the member, and advancement refits when it is accepted. **Still owed:** `combat/attack.rs` is still the port of `CalcAttacks`/`CalcArmorClasses` (`Grip`), and refit after gear or load changes is still noticed by a write to `Carried`/`Burden`: both go with slice 3, where items are things. The party's own sight (`world/perception.rs`) still reads species records; `engage`, `leave_fight` and the monster AI are redesigned only as far as place and side, their shape is slice 6's. |
| 3. Items are things | First phase landed: everything the party holds is item entities placed by relationship (`game/items.rs`), changed by events with one observer each; `Carried`, `ItemPool`, `StackId`, `ItemIds` and the addressing enums are gone; wearing refits and holdings reweigh by the relationships changing. Then what lies in the world, sits in a container or falls as loot became item entities too (`Lying`, `Stowed`), taken by `Give` and thrown by `Throw`. Then a member's arms and armour: what an item is to a hand is components set when it is made (`Weapon`, `Launcher`, `Missile`, `Shield`, `Protection`), and refit reads the two hands ([rules/arms.md](rules/arms.md)); `Grip` and the member half of `combat/attack.rs` are gone. Then what the rules read of an item: what it casts and who may use it (`Casts`), how hard it is to identify (`Obscure`), whether heat takes it (`Flammable`), what a ranged weapon takes and what reloads it (`Takes`, `LoadedBy`) are written as it is made. **Still owed:** recipes and trade values are still read from the definition; a monster's loot is made from its species record as it dies; no test yet for `Identify`, burning, or the omnigun. The screens were not checked in the running game. |
| 4. Levels keep what is in them | First phase landed: a level that is left is set aside (`SetAside`, Bevy's `Disabled` on everything that belongs to it) and woken on return (`Woke`, carrying how long it was away); `LevelMemory`, `LevelRecord`, `KeptItem`, `TriggerMemory`, `KeptDevice`, `DeviceRecords` and `NpcPopulation::depart` are gone. Physics and navigation honour `Disabled` on their own, which a test establishes. Then recruits: a recruitable NPC is their character from the start, wearing and packing item entities, and being in the party is having a seat; `Profile`, `StoredCharacter`, `ItemStack`, `unpack` and `Items::stack` are gone, so no plain item value is left. Then a level that was away owes those in it the time that passed: what ails them and the spells laid on them age as it wakes. **Still owed:** an NPC who stands in a level is two actors, a character and a body with a `Vitals` each, and conditions are copied between them on joining and leaving ([rules/levels.md](rules/levels.md) says what the player sees and why this is wrong). One actor means either stripping a body's physics, navigation and drawn parts from the entity when it takes a seat, or keeping the body as a thing of its own that holds presence only; and telling a member by their seat wherever a rule now tells one by `Character` being present (some thirty places). `wake` and `set_aside` walk what belongs to a level through `&World`; drawing and sound of a level set aside and woken were not checked in the running game. |
| 5. Tables are content | First phase landed: what each spell does is `assets/tables/spell_effects.ron`, 101 spells as rows, read into the catalogs; `define_spell` and its 44 arms are gone, after a test showed the table and the code made the same definitions for every spell. A row names its spell's own numbers symbolically (its dice, rounds, realm). Then spells laid: what a spell adds while laid is an amount written as data (`assets/tables/spells_laid.ron`), with whether it is personal, lasts only a fight, pulses and on whom, may backfire, and its death floor; backfires and pulses are rows beside a spell's effects; `bonus`'s match, `personal`, `combat_only`, `OFFENSIVE`, `define_backfire` and `define_pulse` are gone, each after a test found old and new equal for every spell. Then professions and races: each is a row (`assets/tables/professions.ron`, `races.ron`) holding its numbers and traits; `profession_row`, `race_attributes`, the trait arms and fourteen tests of a particular profession or race inside rules are gone, after tests found old and new equal for every profession, race and level 1 to 50. Then special attacks (`special_attacks.ron`) and the kinds of trap (`traps.ron`), each proven equal to the code it replaced. Then the ranged weapons (`weapons.ron`): what each bow, gun and omnigun tier takes, what reloads a gun, and the omnigun's tiers by level. Then what a laid spell does for its bearer beyond numbers (warning, finding, breath, answering a blow, absorbing, answering a slowing) is found by its row, not its name. An NPC's posts and what it waits on before joining are decoded in `retail`. **Still owed:** a breath is aimed by the record of one named cone spell; Set Portal and Return to Portal are paired by name where spells are picked; a fountain heals by the record of Heal All. |
| 6. A round | First phase landed: the round takes the earliest moment from a timeline (`Moment`); an actor's moment is one chain of events (`Strike`, `Cast`, `Breathe`, `Guard`, `Retreat`, `Hold`); deeds land as they are released (`Release` is an event); `RoundPhase`, `Called`, `Perform`, `Chosen` and the scheduled `resolve` are gone. Then the end of the round is what follows the last moment (`RoundEnds`, its steps in the rule page's order as events), time passing is an event (`Elapsed`, and `Aged` for spells laid), deaths settle where the hurt was dealt (`Settle`), and provocation, utility casts and refused casts are events. No system on the fixed step polls for a message or a marker any more: the step is `fly`, `stop_party_move`, `begin_round`, `next_moment`. Then the attack ([rules/attack.md](rules/attack.md)) and a monster's choices ([rules/monsters.md](rules/monsters.md)) were rebuilt as rules over structs of facts with a gatherer for each, and then rebuilt again without them: each rule takes the fighters, or the monster and what it makes out (`Sight`), and the attack under way. The facts modules and some forty mirror structs are gone. Casting ([rules/casting.md](rules/casting.md)) likewise as far as who may cast, the failure roll, the cost and the auto power, which read the caster, the means of the cast and the spell. A provoked group carries a mark that engagement reads once. **Still owed:** whom a cast reaches is still worked out over a list of targets taken as the cast begins (`magic/reach.rs`, `Stage`), and `casting::Around` still bundles seventeen unrelated things a cast looks at; `monster/close.rs` and `noticing.rs` keep a few plain-value rules; what the casting, monster and attack pages record as confirmed but unfixed or missing; `Settle` finds the dead by looking over every body rather than being told who was hurt. Most unit tests of the monster's and the cast's rules went with the fact structs and were not rebuilt: those rules are covered by the combat scenarios and the retail fight only. |
| 7. Presentation | Adapted as each slice landed; never checked in the running game. |
