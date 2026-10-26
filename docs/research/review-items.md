# Character review screen: the item page

Layout of the review ("camp") screen with its **Items** page selected, in retail
640x480 coordinates. Paths are relative to `/home/agluszak/code/personal/decomp/whizzardry-8`.
Abbreviations:

- `rcs.cpp` = `src/wiz8/local_screens/RCSItemsPage.cpp`
- `rcc.cpp` = `src/wiz8/local_screens/RCSCommon.cpp`
- `rvw.cpp` = `src/wiz8/local_screens/ReviewCharacterScreen.cpp`
- `cat.cpp` = `src/wiz8/local_code/CatalogArtwork.cpp`

The original decomp and Whizzardry are both available; the character panel and
its info-dialog targets were checked against `ReviewCharacterScreen.cpp` and
`RCSItemsPage.cpp` in the original decomp. The retail `wiz8.exe` string table lists exactly the `Data\Review\*.sti` files that the
catalog names, so the catalog matches what retail can load. Strings were decoded from
`data/strings/stringdata.dat`. Sprite sizes were read from the retail STI headers.
To check the coordinates, the page was composited from the retail art at the positions below.
The frames, slot rectangles, buttons and scroll arrows all line up. Open questions are
marked **Uncertain**.

## 1. Catalog numbers

`CatalogArtwork::Load(n, sub)` resolves `g_catalog_slots[n] = {first_entry, image_offset}`.
It then loads `g_catalog_entries[first_entry + sub]` and adds `image_offset` to every frame
index (`cat.cpp:762-812`). Every id used here has `image_offset` 0, so frame numbers
below are file frame numbers.

| Id | File (`data/...`) | Frames |
|---|---|---|
| 0x10f | `review/commoncorner.sti` | 0: 310x165 header backdrop; 1-8: 47x38 empty party cells; 9: 90x29 portrait overlay |
| 0x110 | `review/bottombuttonbar.sti` | 0: 640x30 |
| 0x111 | `review/reviewpagebuttons.sti` | 20 frames of 44x30 (4 buttons x 5) |
| 0x112 | `review/reviewitembuttons.sti` | 40 frames of 44x30 (8 buttons x 5) |
| 0x114 | `review/reviewitempage.sti` | 0: 330x165 info panel; 1: 113x285 carried panel; 2: 366x285 paper doll; 3: 161x285 pool panel; 4: 82x145 (not used) |
| 0x115 | `review/reviewitemhighlights.sti` | 52 frames: 12 sets of 4 slot rims, then one 46x54 set (48-51) |
| 0x116 | `review/portraitexit.sti` | 0: 90x73 |
| 0x117 | `review/smallportraithighlight.sti` | 0: hover, 1: selected (45x36) |
| 0x118 | `review/reviewgoldicon.sti` | 4 frames of 29x18, only 0 used |
| 0x11a | `review/itemusablebackground.sti` | 0: red 46x54 ("cannot use"); 1: green (not used) |
| 0x11b | `review/itemunidentifiedoverlay.sti` | 10 frames, one per slot shape (table in section 4) |
| 0x11c | `review/ac_ovals.sti` | 0: 17x14 |
| 0x11d | `review/leveling_bar.sti` | 0: 103x12 |
| 0x11e | `review/inventoryswapbuttons.sti` | 12 frames of 28x24 |
| 0x11f | `review/inventoryfilterbuttons.sti` | 45 frames of 24x24 (9 icons x 5) |
| 0x120 | `review/commod_button_bar.sti` | 0: 24x45 |
| 0x121 | `review/commod_buttons.sti` | 20 frames of 20x20 |
| 0x122 | `review/commod_back.sti` | 3 frames of 330x165 (alternative info panel) |
| 0x125-0x13f | `review/statue_*.sti` | 1 frame, 204x286 (`statue_mook_f` is 204x285) |
| 0x146 | `review/secondaryoccupied.sti` | 0: 34x72 |
| 0x106 | `char generation/cg_bottombuttons.sti` | 44x30 frames; 16-19 are Exit |
| 0x86 | `main interface/main_scroll.sti` | 0-3 up arrow, 4-5 thumb (16x10), 8-11 down arrow |
| 0x1b6 | `main interface/basic_fill_texture.pcx` | 420x340 indexed bitmap used as cell background |
| 0x193-0x198 | `spell casting/{fire,water,air,earth,mental,divine}_realm.sti` | 18x18 realm icons |

**Not referenced:** Whizzardry does not use these files and `wiz8.exe` does not contain
their names. They look like leftovers of an earlier layout with different sizes, e.g.
`carriedbkgrnd` 166x282, `equippedbkgrnd` 314x282, 68x18 buttons.
- `carriedbkgrnd`, `equippedbkgrnd`, `partybkgrnd`, `nopartybkgrnd`
- `invitemhighlight`, `cursed{boots,cloak,...}`, `itemfx*`
- `itembuttonbar`, `pagebuttonbar`
- the per-button `*button{h,off,on,unav,n}.sti` files (identify, drop, split, merge, info, use, ...)

`invcursedhighlight.sti` (0x124) is named by the exe, but no Whizzardry code draws it
(**Uncertain** whether retail does).

## 2. Static layers

Draw order is top to bottom. Each block is repainted only when it is dirty
(`rcs.cpp:968-989`).

| What | Image, frame | Position |
|---|---|---|
| Header (shared by all pages) | `commoncorner` 0 | (0,0); the area behind it, (0,0)-(310,165), is first filled black (`rcc.cpp:495-500`) |
| Info panel (top right) | `reviewitempage` 0, or `commod_back` in character mode (section 6) | (310,0) (`rcs.cpp:1008`) |
| Carried panel | `reviewitempage` 1 | (0,165); caption "Carried" (0x92b) in Wiz_Text_Font_Bold, centred in (12,174,w93) (`rcs.cpp:1107-1111`) |
| Paper doll | `reviewitempage` 2 | (113,165) (`rcs.cpp:1160`) |
| Statue | `statue_<race>_<m\|f>` 0 | (194,165), drawn over the paper doll (`rcs.cpp:1161-1163`) |
| Pool panel | `reviewitempage` 3 | (478,165) (`rcs.cpp:1280`) |
| Gold icon | `reviewgoldicon` 0 | (514,422); the party gold, with comma separators, is centred in (553,425,w48) (`rcs.cpp:1281-1293`) |
| Pool caption | text | "Party Items" (0x92c), bold, centred in (486,174,w141) |
| Bottom bar | `bottombuttonbar` 0 | (0,450) (`rcc.cpp:828`) |

Text boxes `(x,y,w)` are 12 px tall and use `Wiz_Text_Font.sti` unless stated
(`DrawRcsText`, `rcc.cpp:165-177`).

**Statue choice:** `g_race_portrait_images[race * 3 + gender]` (`rvw.cpp:156-161`,
`rcs.cpp:1162`). Gender 0 is male and 1 is female. The third column repeats the male
image and is never reached by a valid gender. Races follow `W8Race` (`records.h:49-68`):
human, elf, dwarf, gnome, hobbit, faerie (`statue_fairy_*`), lizardman (`statue_lizard_*`),
dracon, felpurr, rawulf, mook. Trynnie, T'Rang, Umpani, Rapax and Android
(recruitable NPCs) have one genderless statue each (0x13b-0x13f).

## 3. Item cells

### Equipment slots (`g_camp_screen_regions`, `rvw.cpp:169-181`)

Index = equipment slot (`equipment_slots.h`). The names are the slot tooltips, strings
0x433-0x43e (`g_equip_slot_label_ids`, `rcs.cpp:124-126`). "Rim" is the first of four
`reviewitemhighlights` frames. "Unid" is the `itemunidentifiedoverlay` frame. "Label" is
the Ctrl name box (section 4).

| # | Slot | x | y | w | h | Rim | Unid | Label (x,y,right-aligned) |
|---|---|---|---|---|---|---|---|---|
| 0 | Head | 188 | 176 | 45 | 57 | 0 | 1 | 238,193 |
| 1 | Misc 1 | 431 | 198 | 40 | 40 | 40 | 8 | 426,222,R |
| 2 | Misc 2 | 431 | 241 | 40 | 40 | 36 | 8 | 426,241,R |
| 3 | Cloak | 376 | 176 | 38 | 73 | 44 | 9 | 371,176,R |
| 4 | Torso | 134 | 241 | 57 | 58 | 4 | 2 | 196,284 |
| 5 | Hands | 387 | 257 | 35 | 43 | 32 | 7 | 382,267,R |
| 6 | Primary weapon | 158 | 308 | 34 | 101 | 12 | 3 | 160,308 |
| 7 | Secondary weapon | 400 | 308 | 34 | 72 | 24 | 6 | 432,308,R |
| 8 | Alternate primary | 121 | 308 | 34 | 101 | 8 | 3 | 123,340 |
| 9 | Alternate secondary | 437 | 308 | 34 | 72 | 28 | 6 | 465,340,R |
| 10 | Legs | 203 | 359 | 34 | 77 | 16 | 4 | 242,399 |
| 11 | Feet | 362 | 378 | 26 | 58 | 20 | 5 | 357,415,R |

- The click rectangle is `(x, y, w+1, h+1)` (`rcs.cpp:873-880`).
- The four rim frames are about 4 px larger than the cell and are drawn at `(x-2, y-2)`.
- Slots 8/9 do have cells on the doll: the stored weapon set sits outside the active one,
  on both sides. `inventory.md`'s "no paper-doll position" is a rules statement
  (they are not worn), not a layout one.

### Backpack ("Carried") and party pool

| Grid | Cell `i` (0-7) draw rect | Click rect |
|---|---|---|
| Backpack | `x = 11 + (i&1)*49`, `y = 192 + (i>>1)*57`, 46x54 | 47x55 at the same origin |
| Pool | `x = 512 + (i&1)*49`, same `y`, 46x54 | 47x55 |

The grids are two columns by four rows (`rcs.cpp:1113-1117`, `1302-1306`, `861-871`).

**Pool rules:**
- The pool shows 8 entries of the filtered list `item_list`, starting at `item_scroll`.
- `item_scroll` is always even: one scroll step is one row of two
  (`rcs.cpp:726-733`, `1414-1440`).
- The cell just past the end of the list addresses the pool's append position.
- Cells past the end show only the fill texture, plus the orange rim while an item is
  held and hovered over them (`rcs.cpp:1330-1346`).
- The pool is not drawn or clickable before the game has started, i.e. when the screen
  is opened from party creation (`rcs.cpp:888-905`, `1284-1286`).

**Scroll bar:** `W8RangeControl` with bounds (611,193)-(629,417) (`rcs.cpp:725`).
`RangeControl.cpp:33-41`, `165-183` give the parts:
- Up arrow: `main_scroll` frame 0/1/2/3 (normal/hover/pressed/disabled) at (611,193).
- Down arrow: frames 8-11 at (611,401).
- Track: (611,209,16,192), filled black.
- Thumb: frame 4, or 5 when hovered, 16x10. It is drawn only when there are more than
  8 entries.
- The range is `0..ceil((n-8)/2)`. The mouse wheel over a pool cell scrolls it too.

## 4. Drawing one cell

### Backpack and pool cells (`rcs.cpp:1113-1146`, `1302-1329`)

1. **Background:**
   - empty, or usable by the reviewed character: the top-left 46x54 of `basic_fill_texture.pcx`;
   - otherwise: `itemusablebackground` frame 0 (red).
2. **Item picture:** item sheet frame **0**, drawn at `(left+1, top+1)`.
   - Frame 0 is pre-positioned inside a 44x52 box by its STI offsets, e.g.
     `chisel.sti` is 11x49 at offset (18,1). There is no runtime centring or scaling.
   - The sheet is `data/items/<video_object_name or kind fallback>.sti`
     (`PC Item.cpp:252-264`, see `inventory.md`).
3. **Quantity**, right-aligned and vertically centred in `(left+1, top+40, w44, h12)`
   (`rcs.cpp:1351-1388`). It depends on the record's `quantity_kind`:

   | Kind | Text | Palette |
   |---|---|---|
   | Stack | the count, only if above 1 | default |
   | Charges / uses | the count, or `?` while unidentified | blue |
   | Shots | the count | yellow |
4. **Unidentified:** `itemunidentifiedoverlay` frame 0 (a "?" mark) at `(left, top)`.
5. **Curse known:** if the record is cursed (`binds_on_equip != 0`) and the instance's
   `bound` (curse known) bit is set, draw `reviewitemhighlights` frame **50** (red rim) at
   `(left, top)`.
6. **Hover:** frame **48** (orange rim) at `(left, top)` when the cell is hovered and either
   holds an item, or the hand holds an item and the Merge mode is off.

Frames 49 (green) and 51 (dark) of this set are not used for these cells.

### Equipment cells (`rcs.cpp:1150-1264`)

1. The fill texture crop, cell-sized, at `(x, y)`. Equipped cells never get the red
   background.
2. Then one of:
   - if occupied, item frame **1** (the large, slot-shaped picture, e.g. 34x101 for a
     staff), drawn at `(x, y)` with its offsets;
   - if slot 7 or 9 is empty and its partner (6 or 8) holds a two-handed item,
     `secondaryoccupied` 0 at `(x, y)`.

   Item sheets with only one frame draw nothing in an equipment cell, because
   `DrawSprite` ignores missing frames. **Uncertain** whether any such item can be
   equipped.
3. **Quantity**, as above, in `(x+2, y+h-13, w-4)`.
4. **Unidentified:** overlay frame `Unid` (table above) at `(x, y)`.
5. **Armour slots** (head, torso, hands, legs, feet): `ac_ovals` at `(x+2, y+1)`. The
   location AC `armor_class_by_location[k]` is centred in `(x+2, y+2, w17)`, with
   head=0, torso=1, legs=2, hands=3, feet=4.
6. **Rim** frame +3 (dark frame, always drawn) at `(x-2, y-2)`.
7. **Curse known** (same test as above): frame +2 (red).
8. Then one of:
   - frame +0 (orange), when the cell is hovered and the click would do something:
     the cell is occupied with nothing held, or the held item can be equipped here
     and used, or Merge mode is on and the cell is occupied;
   - frame +1 (green), when the cell is not hovered and the held item can be
     equipped and used here, the character is eligible, and Merge mode is off.

### Ctrl name labels (`rcs.cpp:632-721`)

While Ctrl is held and no dialog is open, every occupied cell gets a name box:
- The text is the item's display name in `font10arial.sti`, 1 px from the left edge.
- The box is the text width + 4 by the line count x font height + 2, filled black,
  with a 1-px outline of ARGB1555 0x99ED, about RGB (49,123,107).
- Anchors:
  - backpack: `(13 + (i&1)*49, 194 + (i>>1)*57 + (i&1)*16)`;
  - pool: `(556 + (i&1)*49, 194 + (i>>1)*57)`, plus 16 for even `i`;
  - equipment: the table's label column.
- "R" means the box is shifted left by its own width (right edge at the anchor). Pool
  labels are right-aligned too.

### Tooltips (`rcs.cpp:553-571`)

- Equipment: "Name (Slot)", or just the slot name when the slot is empty.
- Backpack and pool: the name, plus " (string 0x930)" for a spellbook or scroll whose
  spell the character already knows.

## 5. Buttons

Every button below is an image button. The five state frames are passed to
`ButtonGraphics::Load(grayed, off, off_hover, on, on_hover)` (`ButtonAppearance.h:48-53`):
- A disabled button draws `grayed`.
- Otherwise it draws `on`/`on_hover` while pressed or toggled, and `off`/`off_hover` otherwise.
- For a sheet base `f`, the item-page buttons use grayed `f+3`, off `f`, off-hover `f+1`,
  on `f+2`, on-hover `f+4`.
- In the art, `f` is the normal frame, `f+1` lit, `f+2` lit and pressed, `f+3` dark, and
  `f+4` pressed with hover.

### Bottom bar, y = 450, 44x30

Item actions use `reviewitembuttons`, created at `rcs.cpp:1785-1810`. Their order is the
enum `W8CampItemActionButton` (`RCSCommon.h:14-25`). Each one is a release-toggle that
selects a mode for the next cell click. If an item is already held, it acts on the held
item at once (`rcs.cpp:1985-2029`).

| x | Enum | Tooltip (string) | Base frame |
|---|---|---|---|
| 0 | IDENTIFY | Inspect Item (2369) | 0 |
| 44 | MOVE | Merge Items (2370) | 10 |
| 88 | SPLIT_STACK | Split Items (2371) | 5 |
| 132 | USE | Use Item (2372) | 15 |
| 176 | DROP | Drop Item (2373) | 20 |
| 220 | USE_ON_ITEM | Cast Identify (2375) | 30 |
| 264 | USE_ON_CHARACTER | Cast Remove Curse (2376) | 35 |
| 308 | CAST_SPELL | Uncurse All Items (2374) | 25 |

The enum names are misleading:
- "MOVE" is the Merge mode, handled by `MergeItemStacksWithHeld`/`combineHeld`.
- "CAST_SPELL" is the "Uncurse All Items" trait action.
- "IDENTIFY" opens the inspect (assay) dialog after an identify attempt.

When each button is enabled is in `UpdateControls`, `rcs.cpp:1816-1960`.

Page buttons use `reviewpagebuttons` and Exit uses `cg_bottombuttons` (`rcc.cpp:783-820`).
Page buttons use `on = f+4` and `on_hover = f+2`, the reverse of the item buttons.

| x | Button | Frames |
|---|---|---|
| 420 | Items (2364) | 0-4 |
| 464 | Magic (2367) | 10-14 |
| 508 | Skills (2366) | 15-19 |
| 552 | Stats (2365) | 5-9 |
| 596 | Exit (2368) | grayed 19, off 16, hover 17, on-hover 18, no "on" |

### Weapon-set buttons under the doll (`inventoryswapbuttons`, 28x24, `rcs.cpp:751-756`)

| Pos | Button | Frames (grayed, off, hover, on, on-hover) |
|---|---|---|
| (125,412) | Swap Weapons (2385), push button | 3, 0, 1, 2, 2 |
| (158,412) | All Swap Lock/Unlock (2386), toggle | 11, 8, 9, 4, 5 |

The lock toggle is green-dotted when unlocked (frames 8-11) and red-dotted when locked
(frames 4-5). Frames 6, 7 and 10 are not used.

### Pool filter column (`inventoryfilterbuttons`, 24x24, x = 485, `rcs.cpp:757-769`)

| y | Button (tooltip) | Base frame | Kind |
|---|---|---|---|
| 193 | Weapons and Shields (2378), group "hand" | 0 | toggle, the four groups are exclusive |
| 218 | Armor (2379), group "body" | 15 | toggle |
| 268 | Non-equippable (2380), group "other" | 20 | toggle |
| 243 | Equippable (2381), group "accessory" | 5 | toggle |
| 301 | Usable (2382) | 10 | independent toggle |
| 335 | Unidentified (2383) | 40 | independent toggle |
| 369 | Sort By Type (2384) | 30 | push button (the P key does the same) |

The rows are listed in creation order, which is not screen order: Non-equippable
(index 2) sits below Equippable (index 3). Frames 25-29 (a red potion) and 35-39 (blank)
are not used. The group mapping is `RebuildCampItemList`, `rcs.cpp:1414-1428`.

### Info-panel tabs (`commod_buttons`, 20x20, `rcs.cpp:770-777`, `840-854`)

- `commod_button_bar` 0 is drawn at (316,7).
- Tab 0 (frames 0-4) is at (318,9). Its tooltip is "Show Character Info" (0x95d) and it
  selects mode ITEMS.
- Tab 1 (frames 5-9) is at (318,30). Its tooltip is "Show Equipment Info" (0x95e) and it
  selects mode CHARACTER.

## 6. Info panel (top right, 310,0 - 640,165)

**Mode ITEMS** (`rcs.cpp:993-1094`): `reviewitempage` frame 0, then:
- Title "EXPERIENCE POINTS", centred in (345,10,w206).
- "Earned" and "Next Level" at (350,24) and (350,38). Their values are centred in
  (448,24/38,w103).
- XP bar: `leveling_bar` at (448,38), clipped to the width
  `103 * (xp - prev_goal) / (goal - prev_goal)`.
- Hit Points, Stamina and Load labels at (324, 58/72/86). The values ("cur/max") are
  centred in (397,...,w44).
- Load uses the font palette for the load category: 1 blue, 2 green, 3 yellow, 4 red
  (`rvw.cpp:153`).
- Armor Class heading centred in (324,114,w117). Base AC and Average AC at (324,128) and
  (324,142), with values at (397,...,w44).
- Seven attributes: labels at (450, 58+14k, w76) and values centred in (530,...,w20).
  The click rects for their info dialogs are (450, 58+14k, 77, 13).
- Six realm icons at (557, 13+24r):
  - Each uses frame `initial_frame`, or the last frame `frame_count` when the character
    has no points in that realm.
  - "cur/max" spell points are centred in (578, 13+24r, w50, h18).

**Mode CHARACTER** uses the stats page's `W8CampCharacterInfo` panel: `commod_back`
frame 0, 1 or 2 at (310,0) (`rvw.cpp:432-447`). It is out of scope here.

## 7. Header (shared with the other pages, `rcc.cpp:445-605`)

- The reviewed character's medium portrait is at (164,12). `commoncorner` frame 9 is
  drawn over it.
- On hover, `portraitexit` 0 is drawn at (164,12). Clicking the big portrait,
  rect (164,12,91,73), exits.
- Party cells: rect `(6 + (s&1)*48, 5 + (s/2)*39, 46, 37)`.
  - An empty party slot shows `commoncorner` frame `s+1` at `(5+(s&1)*48, 4+(s/2)*39)`.
  - Otherwise the slot shows the small portrait. A slot that cannot be selected is
    shaded twice.
  - `smallportraithighlight` frame 0 marks hover and frame 1 the reviewed character.
- The name, race and class are centred in x 116-302 at y 96, 111, 124 and 137.
- Vitals bars are at x 268. The two hand icons (item frame **2**, or the race's empty
  hand) are at (132/133, 34) and (132/133, 58).

## 8. Opening and switching characters

The screen is `W8_SCREEN_CAMP`. It always opens on the Items page (`rvw.cpp:614-640`):
`OpenCharacterScreenForPartySlot(slot, identify)` sets the place to Camp and requests the
screen for that character (`MainGameScreen.cpp:5546-5561`). Callers:

| Trigger | Where |
|---|---|
| Inventory key, default **I** (command 306, `data/strings/mgskeyboard.ini` line 76), for the selected character. Refused during scripted dialogue, a set-aside conversation, the transcript layout, or with no selection | `MGSKeyboard.cpp:385-392` |
| Double left click on a main-screen portrait | `MainGameScreen.cpp:4323-4332` |
| Holding the right button on a portrait for 1000 ms | `MainGameScreen.cpp:4337-4352` |
| Right-click release on a portrait with nothing held, or with Ctrl held. With an item held and no Ctrl, the item is given to that character instead | `MainGameScreen.cpp:4406-4420` |
| In combat, each character with a queued Equip action, one after another | `combat/turn.cpp:369-381` |
| The Identify spell, or an item that identifies: opens in item-target mode (`parameter_4`) | `MGSSpellCasting.cpp:896`, `PC Item.cpp:813` |
| The Review button on the party-selection screen; the pool is hidden before the game starts | `PartySelectionScreen.cpp:883` |

**Switching** (`rcc.cpp:225-286`, `79-102`):
- Left-click a party cell in the header. While a character-targeting mode is active, the
  click targets that character instead.
- Switching keeps the current page. On the item page it cancels the item action, except
  Identify, Cast Identify and use-on-item, and rebuilds the pool list if "Usable" is on.
- Right-click a party cell while holding an item to give it to that character.
- No keyboard shortcut for next/previous character was found. `CampScreen::frame` only
  handles three keys (`rvw.cpp:697-716`):
  - Esc cancels the item action, or exits if there is none;
  - P sorts the pool;
  - Alt+X quits.

**Leaving:** the Exit button, the big portrait, or Esc all call
`DismissSelectedPartyCharacter` (`rvw.cpp:750-756`). It first settles a held item owed to
an NPC member, then returns to the previous screen and starts a fresh combat round if
in combat.

## Uncertain

- Whether retail draws `invcursedhighlight.sti` (0x124) anywhere. Whizzardry uses the
  red rim frames instead.
- The purpose of `reviewitempage` frame 4 (82x145, a two-slot box). It is not used.
- Equipping an item whose sheet has only frame 0 (see section 4).
- All coordinates come from Whizzardry's reconstruction; none were checked against a
  running retail game. The composite render agrees with the art's own frames and wells.
