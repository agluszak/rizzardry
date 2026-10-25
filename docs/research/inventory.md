# Party inventory and equipment rules

Behaviour of the party's items: where they are stored, what may go where, how
stacks and charges change, and what each core move does and refuses. Paths are
relative to `/home/agluszak/code/personal/decomp/whizzardry-8` (commit
`3d046808`). Abbreviations:

- `inv.cpp` = `src/wiz8/inventory.cpp`, `inv.h` = `include/wiz8/inventory.h`
- `rinv.cpp` = `src/rules/inventory.cpp`, `rinv.h` = `include/wiz8/rules/inventory.h`
- `ir.cpp` = `src/rules/item_rules.cpp`, `pci.cpp` = `src/wiz8/local_code/PC Item.cpp`
- `rcs.cpp` = `src/wiz8/local_screens/RCSItemsPage.cpp` (the camp/review items page)

Notice texts were decoded from the retail `data/strings/stringdata.dat` by index.
Everything here comes from Whizzardry, not the original-binary decomp, so treat it
as reconstructed. Open questions are marked **Uncertain**.

## 1. Storage

| Store | Shape | Source |
| --- | --- | --- |
| Equipment | 12 fixed slots per character, indexed by slot (table below) | `character.h:269-271`, `equipment_slots.h:9-24` |
| Backpack | 8 fixed slots per character, can have gaps. The UI shows two columns of four | `character.h:273-275`, `rcs.cpp:640-645` |
| Party pool | One packed list, at most **500** entries (`kPoolLimit`), no gaps. Removing a row closes the gap (`erase`), and empty records are dropped on load | `inv.h:52-63`, `inv.cpp:133-141`, `inv.cpp:202-210` |
| Held item (hand/cursor) | Zero or one item, plus `origin` (the location it was picked up from) and `actor` (who picked it up, -1 if nobody) | `inv.h:23-28`, `inv.cpp:845` |
| Gold | One unsigned purse on the party. Spending clamps at 0. Adding also counts the gold toward the current level's `gold_collected` and can play `ChaChing.wav` | `party.h:19`, `party.cpp:38-41`, `inv.cpp:1543-1555` |

Equipment slots (`equipment_slots.h:9-24`):

| # | Slot | # | Slot |
| --- | --- | --- | --- |
| 0 | Head | 6 | Primary weapon (main hand, active set) |
| 1 | Misc 1 | 7 | Secondary weapon (off hand, active set) |
| 2 | Misc 2 | 8 | Alternate primary (main hand, stored set) |
| 3 | Cloak | 9 | Alternate secondary (off hand, stored set) |
| 4 | Torso | 10 | Legs |
| 5 | Hands | 11 | Feet |

Slots 8 and 9 have no paper-doll position (`ir.cpp:428-432`). Items there are not
"worn": they cannot be activated (`ir.cpp:689-703`), a curse never holds them
(`ir.cpp:567-573`), and they never bind (`rinv.cpp:310`).

**Pool UI.** The pool shows 8 cells at a time in rows of two, and it scrolls one
row (2 entries) at a time. The view can be filtered by usable-by-this-character,
by unidentified, and by one of the equip groups hand/body/accessory/other
(`rcs.cpp:1414-1439`, `rcs.cpp:726-727`, `pci.cpp:343-364`). An empty cell after
the end of the list addresses row `pool.len()`, so dropping onto it appends
(`rcs.cpp:540-551`). The filter is only a view: operations always use real pool
indices.

### Item instance

Disk layout is 12 bytes (`retail_records.h:510-521`). Runtime fields are in
`include/wiz8/rules/item_instance.h:4-23`.

| Field | Meaning |
| --- | --- |
| `item_id` (i32, -1 = empty slot) | Index into `items.dbs` |
| `stack_count` (u8) | Count for quantity kind 1 (stack). It is 0 for every other kind |
| `uses_or_charges` (u8) | Count for kinds 2, 3 and 4 (charges/uses/shots) |
| `identified` | The real name and properties are known |
| `spell_hint` | The spell is shown before full identification. Set together with `identified` (`pci.cpp:585`) |
| `unknown_08` | Set to 1 on identification (`pci.cpp:586`, `pci.cpp:489`). **Uncertain:** no reader was found |
| `effect_used` bit 0 | The one-time "exceptional item" remark has been spoken (`inv.cpp:405-448`) |
| `bind_announced` | **Curse released** (Remove Curse succeeded), see below |
| `bound` | **Curse status known** (see below) |

The names `bound` and `bind_announced` are misleading. The assay dialog decides
the "Cursed Status" line like this (`src/wiz8/dialog_code/AssayDialog.cpp:209-226`,
strings 0x8f9-0x8fd):

- `!bound` shows "?".
- `bound` and the record is not cursed (`binds_on_equip == 0`) shows "Not Cursed".
- `bound`, cursed and `!bind_announced` shows "Cursed (Unremovable)".
- `bound`, cursed and `bind_announced` shows "Uncursed (Removable)".

So the record's `binds_on_equip` byte means **cursed**. Remove Curse sets
`bind_announced` (`pci.cpp:528-551`).

Invariants:

- Exactly one of the two counts is meaningful, chosen by the record's
  `quantity_kind`. `NormalizeItemQuantityKind` repairs instances whose count is in
  the wrong byte (`ir.cpp:27-40`).
- Non-stack items count as 1 unit for weight and `count` (`ir.cpp:83-93`,
  `rinv.cpp:69-71`).
- **Every transfer clears `bind_announced`**, so a released curse is lost
  whenever the item moves. This applies to exchange, character placement, pool
  insertion, drops to the hand, weapon-set swaps and hand moves
  (`inv.cpp:153`, `rinv.cpp:184`, `rinv.cpp:250`, `rinv.cpp:272`, `inv.cpp:360`,
  `inv.cpp:483`, `inv.cpp:1264`).

## 2. Definitions used (items.dbs)

- **Equip classes** (`include/wiz8/data/item_records.h:37-70`): 0 short weapon,
  1 extended weapon, 2 thrown, 3 ranged, 4 ammunition, 5 shield, 6 torso, 7 legs,
  8 head, 9 hands, 10 feet, 11 misc, 12 cloak, 13 instrument, 14 gadget, 15 misc
  magic, 16 potion, 17 bomb, 18 powder, 19 spellbook, 20 scroll, 21 food,
  22 drink, 23 key, 24 writing, 25 other. Classes 26-31 have names but no retail
  item uses them (checked: 819 records, highest class is 25).
- **Flags** (`item_records.h:73-82`): 0x01 auto-identify, 0x02 no-discard
  ("crucial"), 0x04 two-handed, 0x08 off-hand allowed, 0x10 merchant cannot
  sell, 0x20 container, 0x40 must equip to use, 0x80 never depletes.
- **Quantity kinds** (`item_records.h:86-92`): 0 none, 1 stack, 2 charges,
  3 uses, 4 shots. Retail check: there is no kind 5. Stack maximums are 1, 3, 5,
  10, 20, 25 or 100. Every non-stack record has `maximum_quantity` 0 except one,
  so the maximum only matters for stacks. Charge-like counts are capped at
  `initial_quantity.max()` (`ir.cpp:126-133`, `inv.cpp:1177`). Shots are used
  only by ranged weapons such as the Musket, Zip Gun and Rocket Launcher.
- **Kind** (`unidentified_name_index`, rizzardry `unidentified_name`) is the
  generic-name kind. Rules also use it for pairing and class items. 0x83 is the
  Gadgeteer class item family (`inv.cpp:688`, `ir.cpp:255`). The generic name
  string is looked up through a 147-entry table (`ir.cpp:364-385`). It is not a
  direct string id.
- **Image**: `video_object_name` (0xcd, 0x18 bytes) names the item's picture
  under `data/items`; when it is empty (most records), a 145-entry table
  indexed by the kind supplies it (Whizzardry `ItemSprites`,
  `PC Item.cpp:250-264`). Checked: every one of the 819 records resolves to an
  existing file.
- `merge_component_a/b`, `merge_skill` and `merge_skill_level` (recipes) and
  `attack_flags`, `resistance_bonus` and the regen bonuses (used by "has hidden
  properties") are not yet decoded by `src/retail/items.rs`.

## 3. Equipment rules

### Which slots an item can occupy: `GetItemEquipSlotMask` (`ir.cpp:477-522`)

A hand counts as "free" when it is empty. For the main-hand test it also counts
as free when its item is not two-handed (`ir.cpp:524-542`).

| Item | Allowed slots |
| --- | --- |
| Weapon (classes 0-3) with no weapon skill | None (data error) |
| Two-handed weapon | 6 if slot 7 is empty, and 8 if slot 9 is empty |
| One-handed weapon | 6 and 8. With the off-hand flag it can also go in 7 if slot 6 is free, and in 9 if slot 8 is free |
| Ammunition, shield | 7 if slot 6 is free, and 9 if slot 8 is free |
| Torso / legs / head / hands / feet / cloak | Its own slot |
| Misc | 1 or 2 |
| Anything else | None (not equippable) |

`CanEquipItemInSlot(character, item, slot, ignore_worn)` tests the slot bit
against this mask using the character's current hands. With `ignore_worn` it
answers as if every hand were empty (`ir.cpp:526-542`).

**Usability** is a separate check, `CanCharacterUseItem` (`ir.cpp:603-631`):

1. The profession bit, `profession_mask & (1 << profession)`, and the race bit,
   `race_mask & (1 << race)`, must both be set.
2. If the gender mask is not 3, the gender bit must be set.
3. Every attribute requirement (effective attribute >= minimum) and every skill
   requirement (skill level >= minimum) must hold. Id 0xff means unused. The
   first one missed names the message (`ir.cpp:580-601`).
4. Category 3 (spell source/spellbook): usable only if that spell is not already
   learned.
5. Category 6 (Bard) or 8 (Gadgeteer) casting items: usable only if the
   Bard/Gadgeteer level is at least `MinimumCasterLevelForSpellLevel(spell level)`.
   The table is level 1 for spell levels 1 and other unlisted levels, then
   2→3, 3→5, 4→8, 5→11, 6→14, 7→18 (`ir.cpp:633-651`).

Every move into an equipment slot requires both usability and the slot rule
(`inv.cpp:946-949`).

**Pairing in one weapon set**: `CanHoldItemsTogether(a, b)` (`ir.cpp:547-565`):

1. Either side empty: allowed.
2. Either side two-handed: refused.
3. A ranged weapon or ammunition on either side: `CompatiblePartnerItems(ranged,
   other)` decides (see below).
4. If the first item is not a weapon (class 4 or higher): allowed.
5. Two weapons must have the same `range_category`. Otherwise allowed.

`CompatiblePartnerItems` (`ir.cpp:203-296`) is a hard-coded table by kind. Kinds
0x0b/0x25 take kind 0x16, 0x0c takes 0x53, 0x0d takes 0x17. Kinds 0x68/0x6e/0x72
pair by range category or through the table at `ir.cpp:75-77`. The Gadgeteer
class item tiers (ids 599-0x262) accept a growing list of ammunition and thrown
kinds. Retail bows are not flagged two-handed (checked), so bow plus arrows
pairs.

### Default slot when an item is equipped automatically: `GetItemDefaultEquipSlot` (`ir.cpp:434-466`)

| Class | Slot |
| --- | --- |
| Short/extended weapon | 6 |
| Ranged | 8 once play has started, 6 during party creation |
| Ammunition | 9 once play has started, 7 during party creation |
| Thrown | 8 |
| Shield | 7 |
| Misc | 1 |
| Body classes, cloak | Their own slot |
| Other | None |

Two more rules depend on the default slot:

- "Equippable": an item whose default slot is not None must be worn (slots 0-7)
  to be activated (`ir.cpp:670-675`).
- The pool filter groups (`pci.cpp:343-364`).

Party creation (`game_started == false`) adjusts the hand choice in
`ChooseCharacterEquipSlot` (`rinv.cpp:163-229`):

- A melee weapon for a character with Dual Weapons skill points goes into the
  free off hand (7, or 9) next to a one-handed melee weapon. If the new weapon
  cannot go off-hand, the existing main-hand weapon is moved to the off hand
  instead.
- A ranged weapon takes the first completely empty pair (6, then 8).
- Ammunition goes beside a ranged weapon (7, or 9). With no ranged weapon it is
  not equipped.

### Curses (move permission only)

`CanUnequipSlotItem` (`ir.cpp:567-573`) allows removal when:

- the slot is empty, or
- the record is not cursed, or
- the curse is released (`bind_announced`), or
- the slot is 8 or 9, or
- the wearer is dead.

Knowing about the curse is not part of the rule. A cursed item in the backpack
or pool moves freely.

- Clicking an equipped item first calls `claimEquipped` (`inv.cpp:1195-1204`,
  `rcs.cpp:1640-1645`):
  - If the item cannot come off, `BindEquippedItem` marks the curse as known.
    The first time this happens it announces "The %s can't be removed!"
    (0x1ea), because that only fires while `bound` is still unset
    (`rinv.cpp:306-314`). The click is refused with "This item is cursed!"
    (0x90b), and every later click is refused the same way.
  - Otherwise `bound` is set: the status is now known to be "not cursed" or
    "released".
- Placing into a hand whose partner is cursed reveals the partner's curse and
  refuses with "Can't swap out cursed weapon!" (0x916) (`inv.cpp:1035-1040`).
- A weapon-set swap first reveals the curses on slots 6 and 7. If either is
  cursed the swap is refused with "CAN'T swap weapons due to a curse!" (0x1ee)
  (`inv.cpp:1279-1303`).
- New items get `bound` (curse status known) when they are made with
  `mark_special`, which covers starting equipment and class items
  (`ir.cpp:360`, `src/wiz8/local_code/CharGeneration.cpp:1085`). They also get it
  when an identification attempt beats the difficulty by 3 or more
  (`pci.cpp:587-589`), or from a magical identify (`pci.cpp:480-492`).

### What equipment changes recompute

Every operation ends with `settle(changed)` (`inv.cpp:271-300`), which calls
`SettleActiveParty` (`src/wiz8/live_party.cpp:253-284`). It does these things in
order:

1. For every member, `unequipUnusable` removes each item the member can no
   longer use, one at a time. The member's inputs are re-evaluated before each
   check, because removing one item can remove a bonus another item needs
   (`rinv.cpp:279-291`). A removed item goes to the member's own slots, else the
   pool, else the floor or hand. If none of those take it, it stays equipped.
   Each removal announces "can no longer use…" (0x1ef/0x1f0)
   (`inv.cpp:1506-1537`).
2. `UpdatePartyStats` re-evaluates every member's inputs (equipment bonuses),
   redistributes the pool's weight, and re-derives HP/stamina/SP ceilings,
   regeneration, damage reduction, resistances, capacity, load, initiative,
   attacks and AC (`src/rules/party_stats.cpp:96-117`,
   `src/rules/character_stats.cpp:828-846`).
3. If a member's hand changed during play: the berserk choice is reconciled and
   the member's combat hand choices are adjusted (`live_party.cpp:266-273`).
4. "Became overloaded" is presented, and so is a death caused by a lowered HP
   ceiling (`live_party.cpp:274-283`).
5. A committed Use Item action whose item has moved or vanished is reset during
   play (`inv.cpp:290-297`).

Changing any hand slot also clears a weapon swap queued for the round
(`inv.cpp:958-963`).

### Weight and load (computed here, used by stats)

- A stack weighs `weight × max(stack_count, 1)` (`ir.cpp:83-93`). Weight is in
  tenths of a pound.
- A character's own weight is the sum over the equipment and the backpack
  (`character_stats.cpp:789-795`).
- Capacity is `(Vitality + 2 × Strength) × 12`. With the Faerie trait it is
  `× 8` instead (`character_stats.cpp:778-786`).
- **Pool weight is shared out**: half the pool's total weight (`>> 1`) is handed
  out one unit at a time. Each unit goes to the living member with the highest
  remaining-capacity percentage, with ties going to the lowest slot
  (`party_stats.cpp:21-62`). Before play shares are zero. During combat the
  shares are frozen (no redistribution) (`live_party.cpp:67-78`).
- Load band is (own weight + share) / capacity: <50% none, <70% light, <85%
  medium, ≤100% heavy, otherwise extreme (`character_stats.cpp:804-813`). The
  stamina cost penalty by band is +0/25/50/100/200% (`character_stats.cpp:852`).

## 4. Quantities

**Creation, `MakeItem(id, maximum, force_identified, mark_special)`**
(`ir.cpp:337-362`):

- The quantity is a roll of `initial_quantity` dice (`count`d`sides`+`base`), or
  the dice maximum (`base + count × sides`) when `maximum` is set
  (`include/wiz8/data/dice.h:8`).
- Stacks put the quantity in `stack_count`. Kinds 2-4 put it in
  `uses_or_charges`. Kind 0 keeps no count.
- `identified` is set if forced or if the record has the auto-identify flag.
  `bound` is set if `mark_special`.
- Rolls use gameplay RNG. Starting equipment uses the maximum
  (`CharGeneration.cpp:1085`).

**Merge, `MergeStack(dest, src)`** (`rinv.cpp:94-108`):

- It only merges when the item id matches, both are identified or both are
  unidentified, and the kind is stack.
- It moves `min(src, max(0, maximum_quantity - dest))` units. A stack already
  above its maximum takes nothing.
- It returns the number of units moved and leaves the remainder in `src`.

**Spending a use, `SpendItemUse(item, remove_exhausted)`** (`rinv.cpp:110-124`):

- Kind 0 items do nothing, except drinks.
- A stack loses one unit and is gone at 0.
- Otherwise:
  - If the count is already 0, the item goes only when `remove_exhausted` is set.
  - Otherwise it loses one use, and is gone at 0 unless the record has hidden
    properties. A drained wand with properties stays (`ir.cpp:398-408`).

**Recharge** adds uses up to `initial_quantity.max()` (`rinv.cpp:126-133`).

**Pouring uses** (reloading a shot weapon), `pourUses(into, from)`
(`inv.cpp:1167-1183`):

- The source supplies its own count: `stack_count` for stacks, otherwise
  `uses_or_charges`.
- `moved = min(available, into.max - into.current)`. All of it is added to
  `into.uses_or_charges`.
- The source is then spent one use at a time with `remove_exhausted = false`,
  which removes it when it empties.
- Partner kinds come from `g_compatible_partner_pairs`, for example kind 114
  (gun) ← 113 (`ir.cpp:75-77`, `ir.cpp:321-333`).

Automatic uses:

- In combat, an empty shot weapon reloads from the first related-kind item on
  the member, checking equipment then backpack (`inv.cpp:1386-1405`,
  `inv.cpp:1478-1484`).
- Before an attack, `readyPrimaryWeapon` equips an off-hand partner and reloads
  (`inv.cpp:1489-1502`).
- Ammunition spends from the paired hand. When it runs out, a compatible item
  is pulled from slot 9 or found by id (`inv.cpp:1338-1372`,
  `inv.cpp:1451-1462`).
- Thrown weapons that run out are re-split across the two hands. The primary
  hand gets the larger half (`inv.cpp:1411-1440`, `inv.cpp:1468-1477`).

**Uncertain:** the items page calls `pourHeldUses` only when the held item's kind
is the clicked item's partner kind (`rcs.cpp:1477-1482`, `rcs.cpp:1649-1650`).
However, `pourHeldUses` returns early unless both item ids are equal
(`inv.cpp:1188-1190`), so in Whizzardry a manual reload by click does nothing.
Check this against the decomp.

## 5. Placement into a character or the pool

**`PlaceWithCharacter(item, equip, stack)`** (`rinv.cpp:239-275`), in this order:

1. If `equip` is set and the character can use the item: choose the slot
   (default slot, or the creation adjustments above). If the slot is empty, put
   the item there. That acquires nothing and announces nothing (`inv.cpp:334`).
   If the slot is occupied, try to merge all of the item into it.
2. If `stack` is set and the item is a stack: merge into the first matching
   **equipment** slot that takes **all** of it, then the first backpack slot
   that does. A merge that only takes part of the stack is still kept: the
   remainder moves on to the next candidate.
3. Otherwise, use the first empty backpack slot. If there is none, the
   placement fails and the remainder stays with the caller.

`equip = true` is used only at character creation, import and similar
(`CharGeneration.cpp:1086`, `Party Import.cpp:477-579`). Items received during
play are never auto-equipped.

**`placeInPool(item, stack)`** (`inv.cpp:341-368`):

1. A stack merges into existing pool rows in order. Partial merges are kept.
2. Whatever is left is **inserted at row 0** (the front). This fails if the pool
   already holds 500 entries.

Announcements: "takes %s" (0x1e8) for a character, "Party takes %s" (0x1e9) for
the pool.

**`store(recipient, item, order)`** (`inv.cpp:370-380`, `inv.cpp:510-517`) tries
the character, then the pool (`CharacterFirst`), or the reverse (`PoolFirst`).

After an actual acquisition (not a direct equip), `acquire` runs
(`inv.cpp:458-465`):

- Plot-item facts are applied (`acquireItemFacts`).
- `reactToItem` runs once per instance, guarded by `effect_used`
  (`inv.cpp:393-449`). A few ids have fixed speech. Otherwise the receiving
  character remarks when they can use the item, it is identified, and
  `value / level > 500`. When the pool receives an item, a random member is
  picked (gameplay RNG), but the generic remark is skipped.
- Creation and import pass `Suppressed`, which skips the reaction
  (`rinv.h:71-72`).

## 6. Core operations

All of these are in `inv.cpp`. "Refuse" means nothing changes.

| Operation | Semantics | Refusals |
| --- | --- | --- |
| **pickUp(src, n, actor)** `872-933` | Splits `n` units of a stack into the hand, keeping the src location as `origin` and the actor. If the hand already holds the same item id, the units are added to it. A pool row emptied by the split closes up | src empty or invalid; `n` is 0 or more than the count; not a stack; **no-discard**; taking the whole stack out of equipment that cannot be unequipped; hand holds a different id, or the result would exceed `maximum_quantity` |
| pickUp with src = hand | Keeps `n` in the hand and returns the rest: to `origin` if that slot is empty and is a backpack slot or an equipment slot that accepts it, else to the actor's first free backpack slot, else appended to the **end** of the pool. If `n` is the whole held stack, nothing happens | No destination at all |
| **swapWithHand(loc, actor)** `935-955` | Exchanges the slot and the hand (either may be empty). The new held item's origin is `loc`. A pool row that becomes empty closes up. Pairing is not checked | loc is the hand or invalid; both empty; equipment loc that cannot be unequipped; held item not usable or not allowed in that slot |
| **placeHeld(actor, loc)** `1026-1051` | Equipment or backpack only. First `reactToItem`. If the hand's partner cannot share a set with the held item: refuse if the partner is cursed, else `stowPartner`. Otherwise `swapWithHand`, whose own refusal is silently ignored. Afterwards, a held item put into an occupied backpack slot has a 5% (gameplay RNG) chance of character event 14 | "Can't swap out cursed weapon!" (partner revealed); "No room!" (0x90c) when stowPartner fails |
| stowPartner `968-1024` | The held item goes into the hand slot and the partner slot is cleared. If the target was occupied, the old target item goes to the hand and the partner goes to the first free backpack slot, else to the front of the pool. If the target was empty, the partner goes to the hand. The displaced partner gets `bound` | Either slot cursed; held item not usable or not allowed with the partner removed; no room; an RPC (members 0-1) whose NPC wants the partner (`NpcWantsItem`) |
| **mergeHeld(loc)** `1053-1063` | `MergeStack(loc, held)`. The remainder stays in the hand. Returns the units moved | Nothing merged |
| **depositHeld(row, insert_rest)** `1073-1097` | A stack merges into `row` first, then every pool stack in order. Whatever is left is inserted **at `row`** if `insert_rest` is set. Returns Complete, Partial or Refused | Pool full, `row > len`, or `!insert_rest` leaves Partial or Refused, and the rest stays held. "No room!" |
| **combineHeld(actor, target)** `1103-1162` | Recipe: some record lists the two items as components, in either order. The quantity is the smaller of the two counts if both are stacks, else 1. That many are taken from each. The product is `MakeItem(recipe)` (rolled, not forced identified, not bound) with its stack set to the quantity, and goes into the target slot if the target emptied, else into the hand. Then a learn sound plays and `merge_skill` is practised by `level / 10`. Without a recipe, two items with the same id fall back to `mergeHeld` | Either item unidentified ("Unidentifed items cannot be merged!" 0x163); `merge_skill` level too low ("%s can't merge those items." 0x162); no recipe and different ids ("Those items cannot be merged!" 0x161) |
| **giveHeld(member, order)** `547-573` | Dropping on a portrait. The UI first lets the party try to identify the item (`MainGameScreen.cpp:4305`). It uses `CharacterFirst` outside combat for an eligible member, otherwise `PoolFirst` (`MainGameScreen.cpp:4298-4308`). Right-click always uses `PoolFirst` (`:4416-4417`). Whatever cannot be placed stays held | Ineligible member with `CharacterFirst`: "Can't give items to someone in this condition" (0x1f5). Nowhere to put it: "No room!" |
| storeHeld(member) `533-545` | Merges or puts into the backpack only | No room |
| **dropHeld(cursor)** `850-865` | Creates a world item at the drop position under the cursor | No-discard: "On second thought, you decide you might need this item later." (0x4ef). If geometry finds no position, returns `false`, the hand keeps the item and `CantDrop.WAV` plays |
| hold(item) / replaceHeld(item) `835-848` | Puts an item into an empty hand / replaces the held item. Used for world pickup (`ItemManager.cpp:335-336`) and scripts | `hold` refuses when the hand is full. **Uncertain:** world pickup still removes the world item when `hold` fails (`ItemManager.cpp:338-345`); presumably the UI does not allow that click while the hand is full |
| **storeOrDrop(item)** `519-526` | Pool, else a world drop at the party's feet unless no-discard, else into an empty hand | All three fail: the caller keeps the item |
| **receiveAll(items, commit)** `575-625` | For purchases. Plans everything on a copy of the pool: merge into stacks, then insert at the front. Overflow is dropped in the world, or held if it is a single no-discard item and the hand is empty. Only once every item has a place are the changes applied; then `commit` (payment) runs, followed by acquisition | Any item with no place: nothing changes (`Refused`) |
| Container picker `DialogFactoryDialogs.cpp:458-485` | Each selected item goes to the pool or a chosen character (`storeInPool`, `storeWithCharacter`). A partial stack stays in the container | Destination full |
| **removeById(id, all)** `788-819` | Search order is hand, then each member (equipment then backpack), then the pool. With `all = false` it stops at the first hit. With `all = true` it still clears **only one copy per member** but every matching pool row | None |
| **removeQuantity(loc, n)** `821-831` | Subtracts `n` from the stack. If `n >= max(1, count)` the whole item is removed | Empty slot or `n == 0` |
| take(loc) `176-184` | Removes and returns the item | Empty slot |
| **use(actor, use)** `1740-1747`, `1567-1738` | Checks usability, then whether the item can be activated (`ir.cpp:653-687`: must be worn if equippable or must-equip, has a spell, has charges). Casts the item's spell, and spends a use **only if the cast succeeded** | Stale location (a different item is there now); not usable; spent charges/shots |
| **sortPool()** `1206-1212` | Explicit only: the sort button or the `P` key (`rcs.cpp:501-507`, `ReviewCharacterScreen.cpp:706-709`). Never automatic | Before play |
| stashDeparting(member) `630-657` | A member leaving moves every item to the pool, except worn items whose curse holds (cursed, not released, on the paper doll, wearer alive). Overflow is dropped ("The %s has been dropped." 0x7d3) or held | None |

**Pool sort order** (`rinv.cpp:137-155`). The header comment says "descending",
which is wrong for the first two keys. Keys in order:

1. Equip class, **ascending**.
2. Kind, **ascending**.
3. Identified before unidentified. Two unidentified items compare equal from
   here on.
4. Value, **descending**.
5. Stack count, descending.
6. Uses, descending.

The sort is not stable (`std::sort`).

**Items-page click** (`rcs.cpp:1459-1742`) chooses the operation:

- Shift-click takes one unit (`pickUp(1)`).
- Clicking equipment first goes through `claimEquipped`.
- With an item held:
  - If the clicked item's partner kind matches the held item, `pourHeldUses`.
  - A pool cell does `depositHeld(row, true)`.
  - A same-id stack does `mergeHeld`.
  - Otherwise `placeHeld`.
- With an empty hand, `swapWithHand` picks the item up.

Restrictions:

- Before play: "Can't manipulate items yet!" (0x900).
- A missing, webbed or turncoat character cannot touch their own slots
  (`rcs.cpp:1499-1527`).
- In combat, a click that touches equipment or the backpack spends the
  character's action allowance. The exception is moving their own thrown weapon
  or ammunition into a slot it fits beside (`rcs.cpp:1529-1551`).

**Identification at the interaction points**:

- Right-click/info runs `PartyAttemptsToIdentifyItem` on the item in place
  (`rcs.cpp:163-176`).
- Dropping on a portrait identifies first (`inv.cpp:528-531`).
- Search identifies world items before they are acquired
  (`src/wiz8/local_code/Search.cpp:189`).

Attempt rule (`pci.cpp:455-475`, `pci.cpp:568-596`):

- Each living, sane member tries in slot order. An already identified item is
  skipped.
- Strength is `Artifacts / 6`. Success at `difficulty <= strength`. The curse
  status is also revealed at `difficulty + 3 <= strength`.
- If the margin is 0-2, Artifacts is practised by 2.
- Insertion never identifies anything by itself. The only exceptions are
  auto-identify records and forced creation.

## 7. Notable rules (keep these)

- Received items are **never auto-equipped during play**. Placement order is:
  merge into equipment stacks, then backpack stacks, then an empty backpack slot
  (`rinv.cpp:260-274`). Pool placement merges first, and new rows go at the
  **front** (`inv.cpp:359`).
- Rows returned when a held split comes back, and new rows from `pickUp`, go to
  the **end** of the pool (`inv.cpp:911-912`, `inv.cpp:928`). A deposit goes
  **at the clicked row** (`inv.cpp:1093`).
- `pickUp` adding to the held stack does **not** compare `identified`
  (`inv.cpp:888-890`), but `MergeStack` does. **Uncertain** whether this is
  retail behaviour or a reconstruction slip.
- No-discard items can be neither split nor dropped. They can still be swapped
  and placed.
- Every move resets a released curse (`bind_announced`). An equipped cursed item
  still blocks after its curse has been revealed.
- `CountCharacterItem` counts the **equipment** slot with the same index for
  each matching backpack slot. This is a retail bug that Whizzardry keeps
  (`rinv.cpp:76-79`). **Uncertain** whether any quest check depends on it.
- `placeHeld` into a backpack slot draws gameplay RNG (`inv.cpp:1046`). Pool
  receipt picks a random member for the reaction (`inv.cpp:399`).

## 8. Notes for the Rust model

- Item values belong to their owners. The stores are `[Option<Item>; 12]`,
  `[Option<Item>; 8]`, a pool `Vec<Item>` capped at 500, `Option<Held>` and the
  purse. The quantity should be an enum chosen by `quantity_kind` instead of two
  bytes.
- Rename the knowledge fields: `curse_known` (`bound`) and `curse_released`
  (`bind_announced`). Clear `curse_released` inside the single move primitive,
  not at each call site.
- Pool indices shift on insert at the front, insert at a row, and erase. Choices
  that must survive operations (queued item use, identify target) need an
  instance identity. Whizzardry instead re-checks "same item id still at this
  location" and drops stale uses (`inv.cpp:253-257`, `inv.cpp:290-297`).
