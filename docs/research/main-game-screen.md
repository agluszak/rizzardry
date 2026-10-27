# Main game screen

Layout of the exploration screen (the "MGS": roof, party portrait columns, bottom panels
and button bar around the 3D view), in retail 640x480 coordinates. Paths are relative to
`/home/agluszak/code/personal/decomp/wizardry-8-decomp`. Abbreviations:

- `mgs.cpp` = `src/wiz8/local_screens/MainGameScreen.cpp`
- `por.cpp` = `src/wiz8/local_screens/MGSPortraits.cpp`
- `btn.cpp` = `src/wiz8/local_screens/MGSButtons.cpp`
- `pc.cpp` = `src/wiz8/local_screens/MGSPortraitCombat.cpp` (button tables live here)
- `txt.cpp` = `src/wiz8/local_screens/MGSTextBox.cpp`
- `rad.cpp` = `src/wiz8/local_screens/MGSRadarMap.cpp`
- `frm.cpp` = `src/wiz8/local_screens/MGSFormation.cpp`
- `reg.cpp` = `src/wiz8/local_code/RegionManager.cpp` (`g_regions`)
- `cat.cpp` = `src/wiz8/local_code/VideoObjectManager.cpp` (`g_video_frames`, `g_video_slots`)

Everything comes from the decomp. Whizzardry's `MainGameScreen.cpp` and
`MGSPortraitCombat.cpp` have the same position tables (spot-checked: viewport modes,
bank, roof, layout-arrow and panel-button positions). Sprite sizes were read from the
retail STI headers. Strings were decoded from `data/strings/stringdata.dat` and are
`gppStringList` indices. Two layouts were composited from the retail art at the positions
below ("Closed" with portraits, and "Wide" with a combat popup). The columns, bars,
plugs, bottom panels and the button row all line up. Nothing was checked against a
running retail game. Open questions are marked **Uncertain**.

Rectangles are `(left,top)-(right,bottom)`. Region rectangles are inclusive (the portrait
region `(23,18)-(104,89)` is the 82x72 portrait art). Circle regions are `centre r radius`.

## 1. Catalog numbers

`DrawCatalogImage(target, id, frame, image, x, y)` resolves `g_video_slots[id] =
{first_frame, y_offset}` and draws image `image + y_offset` of file
`g_video_frames[first_frame + frame]` (`cat.cpp:786-830`). Every id below has
`y_offset` 0. "Frame" in this document is the STI image index. `frame` selects the
file only for portraits and item sheets. Sprites are drawn with their STI image offsets
(e.g. `healthbar` is 3x45 at offset (0,1)).

| Id | File (`data/...`) | Images |
|---|---|---|
| 0x12 | `portraits/medium/<portrait>.sti` (`frame` = portrait index) | 0: 90x72 face; 1-9: animation overlays (pose/mouth) |
| 0x1f-0x29 | `portraits/medium/mskull*.sti` | 90x72 dead portrait, per race (section 3) |
| 0x2f / 0x30 | `main interface/leftplug.sti` / `rightplug.sti` | 16x16 empty condition / enchantment socket |
| 0x31 | `main interface/pback.sti` | 90x72 empty-slot backdrop |
| 0x32 / 0x33 | `charhighlight.sti` / `charselect.sti` | 82x72 portrait rim: hover (white) / selected (blue) |
| 0x34 | `dragons.sti` | 82x72 empty-slot art |
| 0x4f-0x51 | `healthbar.sti`, `staminabar.sti`, `magicbar.sti` | 3x45 at offset (0,1) |
| 0x52-0x54 | `healthbarwide.sti`, `staminabarwide.sti`, `magicbarwide.sti` | 5x45 (numeric-HP option) |
| 0x5e / 0x5f | `edgehighlight.sti` / `edgeselect.sti` | 22x70 strip rim: hover / selected (Wide/Full) |
| 0x60 / 0x61 | `cornerselect.sti` / `condition_highlight.sti` | 18x18 socket rim: hover / has condition |
| 0x62 | `nameselect.sti` | 92x16 name hover |
| 0x63 / 0x73 | `weaponselect.sti` 18x27 / `weapon2select.sti` 18x51 | hand hover (one hand / two-handed) |
| 0x64 / 0x65 | `statsselect.sti` 17x51 / `statsselectwide.sti` 23x58 | vitals hover |
| 0x66-0x6f | `handleft/right`, `fhand*`, `paw*`, `claw*`, `rhand*` | 12x21 empty hands (section 3) |
| 0x70-0x72 | `popup_edge`, `popup_divider`, `popuphighlight` | info-plate frame pieces (out of scope) |
| 0x7e | `main_bottom_bar.sti` | 0,1: 23x92 side posts; 2,3: 30x30; 4: 262x30; 5: 51x30; 6: 185x30; 7: 82x30; 8,9: 96x30 |
| 0x7f | `main_submenu_bar.sti` | 4 widths (47/66/85/104)x28, combat popup background |
| 0x80 | `main_column_info_normal.sti` | 0: 20x73 hands; 1: 23x73 bars; 2: 20x73 two-handed; 3: 23x73 bars (numeric HP) |
| 0x82 | `main_columns_normal.sti` | 16 column pieces (0,3,4,7,..: 23x85; 1,2,5,6,..: 128x85); 16: 82x31 arch over the portrait |
| 0x83 | `small_formation_back.sti` | 105x92 |
| 0x84 | `main_radar_back.sti` | 105x92 |
| 0x85 | `main_roof.sti` | 640x18 |
| 0x86 | `main_scroll.sti` | 0-3 up arrow, 4-7 thumb (16x10), 8-11 down arrow |
| 0x87 | `main_textbox_back.sti` | 0: 384x92 (only frame used here) |
| 0x88 | `main_textbox_tabs.sti` | 3 frames of 28x66, one per selected tab |
| 0x89 | `icons_submenu.sti` | 196 frames of 18x18 (popup rows) |
| 0x8a | `icons_portrait.sti` | 393 frames of 20x20 (combat action icons) |
| 0x8b / 0x8c | `icons_portrait_shield.sti` 12x14 / `icons_portrait_jewels.sti` 8x 12x12 | combat badges |
| 0x8d | `progressbar.sti` | 286x13 combat round progress |
| 0x90-0x92 | `damage_splat_anim`, `damage_splat_anim2` (16x 82x72), `death_splat_anim` (30x 82x72) | portrait hit splats |
| 0x99 / 0x9a | `party formation/small_formation_cage.sti` / `small_form_cage_high.sti` | 91x91 board / hovered |
| 0x9b | `party formation/small_formation_cone.sti` | 91x91 view cone (both radar and formation board) |
| 0x9c / 0x9d | `small_formation_symbol.sti` 12x / `small_formation_jewels.sti` 24x 15x15 | board markers |
| 0xa1 / 0xa2 | `radar compass/compass_rim.sti` / `compass_rim_high.sti` | 91x91 rim / hovered |
| 0xa3 / 0xa4 | `radar compass/formation_dot_large.sti` / `formation_dot_small.sti` | 91x91 radar disc, zoomed in / out |
| 0xa5 | `radar compass/formation_jewels.sti` | 8 frames of 2x2 |
| 0xa7 | `leveling_button.sti` | 5 frames of 18x18 |
| 0xa8 | `cond_icon_rim_wide.sti` | 2 frames of 19x19 |
| 0xa9 | `paused_background.sti` | 286x20 |
| 0xaa-0xb6 | `spells/bitmaps/<realm>_<positive\|negative>.sti` | portrait spell-hit animation |
| 0xb7-0xc9 | `icons/conditions/*.sti` | 16x16, id = 0xb6 + condition |
| 0xca-0xd0 | `icons/enchantments/*.sti` | 16x16, id = 0xc9 + enchantment |
| 0xd1-0xe3 | `icons/travelingspells/*.sti`, `icons/partyspells/*.sti` | 19x19 party-spell icons |

Some buttons load their files by path, not by catalog id (`pc.cpp:51-90`):
`options_disk`, `attack_confirm`, `combat_stop`, `cont_start`, `cont_toggle`,
`cont_pending` (4x 30x30), `main_roof_buttons` (12x 28x16), `main_layout_arrows` (24x 23x24..26),
`main_scroll_arrows` (8x 13x18), `icons_standard` (45x 27x27) and `icon_combat_toggle`
(0-3: 89x26, 4-7: 28x26).

**Not used by retail.** These files are in the catalog (or only on disk), but no draw
call, `Controls` background or text control references them:
- `roof`, `bottom`, `bottomb`, `leftcorner`, `rightcorner`, `layout`, `inside`, `outside`, `outside2`;
- `lefttop`, `leftside1-4`, `righttop`, `rightside1-4`, `left/right/bottomstrip`, `left/rightwingtip`;
- `camp`, `party`, `search` and `option`, both the `*2[nhg]` (30x30) and `*[nhg]` (40x40) sets;
- `goldbar`, `textup/downarrow*`, `sliderbar`, `slidernut`;
- `compass`, `viewcone`, `formationsymbol`, `formationjewels(h)`;
- `main_columns_hit`, and the on-disk-only `main_compass_back`, `compassrim`,
  `combatactionicons`, `nameselectgreen`, `round switch`.

They are an older, pre-release layout. `layout.sti` (9 small 25x14 tabs plus one 80x14
tab) and `leftside1..4`/`rightside1..4` (127x87 side panels) belong to it. The
screen's "layout variants" in the shipped game are the roof buttons in section 6, not
these files. The compass is drawn as part of the radar (section 7).

## 2. Fixed frame and viewport

Draw order follows `DrawMainGameScreenChrome` (`mgs.cpp:3560-3770`) and the button
redraws it triggers. Each part is repainted only when its redraw bit is set.

| What | Image, frame | Position |
|---|---|---|
| Roof | `main_roof` 0 | (0,0) (`btn.cpp:531`) |
| Party columns | section 3 | x 0-128 and 512-640, y 18-358 |
| Left post (layout arrows) | `main_bottom_bar` 0 | (0,358) |
| Right post | `main_bottom_bar` 1 | (617,358); decoration only |
| Radar panel | `main_radar_back` 0 | (23,358); when hidden, (23,358)-(128,450) is cleared to the 3D view |
| Text box | `main_textbox_back` 0, then `main_textbox_tabs` frame 0/1/2 | (128,358), tabs at (138,360); hidden: (128,358)-(512,450) cleared |
| Formation panel | `small_formation_back` 0 | (512,358); hidden: (512,358)-(616,450) cleared |
| Options socket | `main_bottom_bar` 2 | (0,450) |
| Bank background | `main_bottom_bar` 4 | (30,450) |
| Combat-toggle socket | `main_bottom_bar` 8 (exploring) / 9 (combat) | (196,450) |
| Prev/next socket | `main_bottom_bar` 5 | (292,450) |
| Character caption | `main_bottom_bar` 6 | (343,450) |
| Panel-button socket | `main_bottom_bar` 7 | (528,450) |
| Combat stance socket | `main_bottom_bar` 3 | (610,450) |

### Viewport

The 3D view is `g_viewport_modes[mode]` (exclusive right/bottom; `mgs.cpp:4784`), set
by `SetViewportMode`, which also moves the world-view region 0xe6. The panels and
columns are 2D art drawn over the frame buffer, so hiding one exposes the 3D view behind
it.

| Mode | Rect | Used when (`GetMainGameViewportMode`, `mgs.cpp:7404`) |
|---|---|---|
| 0 | (23,18)-(617,450) | Wide or Full with at least one bottom panel hidden |
| 1 | (23,18)-(617,358) | Wide with all three bottom panels |
| 2 | (128,18)-(512,358) | Closed with all panels; also the fallback in spell, item, lock, trap and dialogue modes |
| 4 | (128,18)-(512,450) | Closed with the text box hidden |
| 3, 5, 6 | (128,18)-(512,279), (128,18)-(512,416), (23,18)-(617,416) | **Uncertain**: never returned by `GetMainGameViewportMode` |

## 3. Party columns

Slot `s` (0-7) sits in column `s&1` (0 left, 1 right) and row `s>>1`. Anchors come
from `GetPartySlotMenuAnchor` (`por.cpp:420-524`). With `row_y = 18 + 85*(s>>1)`:

| | Left (even) | Right (odd) |
|---|---|---|
| `menu_x` (portrait block) | 0 | 512 |
| Hands column `bme` | 2 | 619 |
| Bars column `bpe` | 105 | 512 |
| Column piece | `main_columns_normal` 1/5/9/13 at (0,row_y) | 2/6/10/14 at (512,row_y) |

In Wide and Full mode (unless the slot is slid out, below) the block collapses to a
23-px strip:
- `menu_x` becomes -105 (left) or 617 (right), and `bpe` becomes 0 or 617;
- the column piece is `main_columns_normal` grid-1 (0/4/8/12) at (0,row_y), or grid+1
  (3/7/11/15) at (617,row_y);
- the portrait, hands, name and sockets are not drawn, which leaves just the bars.

### Draw order for one slot (`RedrawPartyPortraitOverlay`, `por.cpp:686-1070`)

1. **Portrait** at (`menu_x`+20, `row_y`):
   - Empty slot: `pback` 0, then `dragons` 0.
   - Dead (`hp == 0`, once a death splat has reached frame 13): the race's medium skull.
     It is mirrored for odd slots. `g_dead_portrait_catalog_ids[race][1]`: human, elf,
     dwarf, gnome, hobbit and faerie use `mskull`; then lizardman `_liz`, dracon `_drac`,
     felpurr `_felp`, rawulf `_raw`, mook `_mook`, trynnie `_tryn`, T'Rang `_trang`, Umpani
     `_ump`, Rapax `_rap`, Android `_and`.
   - Otherwise the **medium** portrait (catalog 0x12), image 0 drawn with
     `VO_BLT_SHADOW`, then the current animation images (`QuoteManager.cpp:1900-2010`).
     - It is mirrored when `g_portrait_descriptors[p].render_mode` is 1 in an even slot or
       2 in an odd slot.
     - The 89x71 rect is shadowed while the character is dead in combat, Missing, or a
       surprise is pending.
   - Small portraits (`data/portraits/small`) are not used on this screen; they belong to
     the review screen header.
2. Column piece (table above), then `main_columns_normal` 16 (the arch) at
   (`menu_x`+23, `row_y`).
3. Hands column, only in Closed mode or when slid out:
   - Background: `main_column_info_normal` 0, or 2 when the primary weapon is two-handed,
     at (`bme`, `row_y`).
   - Upper hand (primary weapon): the item sheet's image **2** at (`bme`+3, `row_y`+23).
     An empty hand draws the race's upper hand at (`bme`+4, `row_y`+23).
   - Lower hand (secondary): the same at `row_y`+47. It is omitted for two-handed weapons.
   - Empty-hand ids (`g_empty_hand_catalog_ids`, upper/lower):
     - `handright`/`handleft`: human, elf, dwarf, gnome, hobbit, Umpani;
     - `fhand*`: faerie;
     - `claw*`: lizardman, dracon, T'Rang, Rapax;
     - `paw*`: felpurr, rawulf, mook, trynnie;
     - `rhand*`: Android.
   - Stack count, when non-zero: `smfnt`, at (`bme` - (w+1)/2 + 8, `row_y`+38) for the
     upper hand and (`bme` - (w+1)/2 + 11, `row_y`+62) for the lower.
4. Bars column: `main_column_info_normal` 1 (3 for numeric HP) at (`bpe`, `row_y`), then
   the bars (below).
5. Text (Closed or slid out), all in `smfnt` except the name:
   - Average AC, centred in 13 px at (`bme`+3, `row_y`+3). Its palette depends on the load
     category.
   - Profession abbreviation (string `g_profession_name_message_ids[prof+16]`), centred in
     18 px at (`bpe`+2, `row_y`+3). Its palette is the character's party-order colour.
   - Name in `wiz_text_font`, centred in 84 px from (`menu_x`+21, `row_y`+73). Shade 1
     while the name is hovered, otherwise 4.
6. Selection rim:
   - Closed or slid out: `charhighlight` (hover, while the left button is up) or
     `charselect` (selected character), at (`menu_x`+23, `row_y`).
   - Wide/Full strips: `edgehighlight` or `edgeselect` at (`bpe`+1, `row_y`+2).
7. Sockets at `row_y`+3. Left slots have the condition at `menu_x`+25 and the
   enchantment at `menu_x`+88; right slots swap them.
   - A condition draws catalog `0xb6 + highest_condition`; an empty socket draws a
     plug. The socket on the screen-left side always gets `leftplug` and the screen-right
     one `rightplug`, whichever of the two it holds.
   - An enchantment draws `0xc9 + enchantment_top`.
   - At (x-1, `row_y`+2), `cornerselect` is drawn while the socket is hovered, otherwise
     `condition_highlight` if the character has a condition.
   - Retail quirk: the enchantment socket also gets `condition_highlight` when the
     character has a *condition* (`por.cpp:975-981`).
8. Hover overlays:
   - `nameselect` at (`menu_x`+19, `row_y`+72);
   - `weaponselect` at (`bme`+1, `row_y`+21) for the upper hand and `row_y`+45 for the
     lower; `weapon2select` at `row_y`+21 for a two-handed weapon;
   - `statsselect` at (`bpe`+3, `row_y`+21); with numeric HP, `statsselectwide` at
     (`bpe`, `row_y`+14).
9. Damage splat, then spell-hit icon (below), then any held info plate.

### Bars (`SyncPartyPortraitVitalsBars`, `RedrawPartyPortraitBars`, `ShadeStatusBarGap`)

| Option | Images | x (from `bpe`) HP / stamina / SP | y | Dark lines |
|---|---|---|---|---|
| Normal | `healthbar`, `staminabar`, `magicbar` (3x45, offset y 1) | 6 / 10 / 14 | `row_y`+23 | 3 px wide |
| Numeric HP | `*barwide` (5x45) | 3 / 9 / 15 | `row_y`+17 | 5 px wide |

**Fill** (0-45 px, integer division):
- HP: `hp * 45 / hp_max`, at least 1 while alive.
- Stamina: `max(stamina, 0) * 45 / stamina_max`, at least 1 if stamina is non-zero.
- SP: the sum of current points over all six realms, times 45, divided by the sum of
  maximum points. At least 1 if any points are left.
- The SP bar is not drawn at all for characters with no spell points.
- When `hp == 0`, no bars are drawn.

**Drawing:** the full bar is drawn first. Then the empty part, `45 - fill` long, is
painted from the top with near-black (RGB 1,1,1) vertical lines (`por.cpp:1074-1088`),
so the bars drain downward. The lines start at the bar's `y` (one row above the art in
normal mode).

Numeric HP shows `hp_current` in `smfnt`, centred in (`bpe`+2, `row_y`+62)-(`bpe`+20,
`row_y`+70) on a black fill. The option is `g_settings.numeric_hit_points`.

### Effects on portraits

- **Damage splat** (`RecordCharacterDamage`, `por.cpp:113-245`):
  - `damage_splat_anim` or `_anim2`, chosen 50/50, at (22 or 533, `row_y`). 8 frames,
    100 ms each.
  - The accumulated damage is drawn in `wiz_text_font_bold`, centred in the 82x72 box
    from `y`+4, for the first 6 frames.
  - When the hit kills, `death_splat_anim` plays its 30 frames instead. The skull
    portrait replaces the face at frame 13.
  - The 50/50 pick calls the game's `Random`, a cosmetic use of the gameplay RNG; the
    rewrite should not copy it.
- **Spell hit** (`StageMonsterCastIcon`): the `<realm>_<positive|negative>` animation at
  (23 or 535, `row_y`), 100 ms per frame, with the spell's sound.
- **Combat action icon** (`RedrawCombatPortraits`, `por.cpp:331-386`):
  - `icons_portrait` image at (25 or 595, 68 + 85*row). The image is chosen by
    `GetCombatPortraitImage` (action and status). It uses +10 while hovered, and pulses
    ±1 every 100 ms for the acting character.
  - Above it, at y 55 + 85*row: `icons_portrait_shield` (at x 25/603) when the character
    cannot act, or `icons_portrait_jewels`[target's party order] (y 56) when protecting a
    party member.
- **Level-up button**: a text control with `leveling_button` frames 0-4, at (25 or 596,
  70 + 85*row) and 18x18 (`por.cpp:1226-1252`). It is active while the character can
  advance. A click opens the character screen in level-up mode.
- **Wide/Full condition button**: `cond_icon_rim_wide` 19x19 at (23 or 598, 19 + 85*row).
  - The icon (highest condition, else top enchantment) is drawn at +2,+2.
  - Holding the left button shows the info plate (`por.cpp:1300-1336`, `1385-1432`).

### Slide-out (Wide and Full)

- Clicking a strip (regions 0x5a-0x61: (0,18+85r)-(21,102+85r) and
  (617,18+85r)-(639,102+85r)) calls `RefreshSelectedPartyPortrait` (`mgs.cpp:4137`).
- The slot is then drawn with the full Closed layout at its Closed position, over the
  viewport (`portrait_refresh_pending`, x shift 105), and its Closed hit regions are
  enabled.
- It retracts when the cursor leaves it, unless it is pinned or an animation (damage
  splat, spell icon, quote) is playing (`UpdateFormationPortraitRefresh`, `mgs.cpp:5040`).
- Damage, spell hits and quotes slide the portrait out automatically.
- **Uncertain**: whether retail animates the slide; the code jumps straight to the
  105-px offset.

## 4. Roof

| What | Image | Position | Notes |
|---|---|---|---|
| Layout buttons | `main_roof_buttons` 28x16 | (10,1), (39,1), (68,1) | section 6 |
| Combat progress | `progressbar` 0 | (177,2), clipped to right = 177 + 286*pct/100 | only while a round is executing (`mgs.cpp:3617-3628`) |
| Party spell icons | catalog icon 19x19 per active party effect | first at (622,0), each next 19 px to the left, up to 12 | hover tooltip "name: amount, duration" (`MGSSpellIcons.cpp:150-178`) |
| Combat effect strips | 20x20 icons | left (129,20)-(317,40) from x 129 rightward, step 21, up to 9; right (386,20)-(511,40) from x 491 leftward, up to 6 | combat only. **Uncertain** which side is which (`g_combat_state->effect_slots` vs `effect_slots0`) |
| Monster group list | text, 11-px rows | from (250,25) | combat, region 0xe5 |
| Pause banner | `paused_background` 0 | (177,319) | "Paused" (0x77f) or "Combat Paused" (0x780), `wiz_text_font`, centred in (177,319)-(463,339) |

## 5. Bottom button row (y 450-480)

All are `W8DialogButton`s with frames given as `Configure(file, grayed, off, off_hover,
on, on_hover)` (`DialogButton.h:74-79`). Toggle buttons stay pressed (`on`).

| Pos | Tooltip (string) | File, frames (grayed, off, hover, on, on-hover) | Action |
|---|---|---|---|
| (0,450) | Options (0x41) | `options_disk` 3,0,1,2,2 | opens the options screen. Disabled in dialogue, lock, trap and camp |
| (40,452) | Journal (0x42) | `icons_standard` 0x2b,0x28,0x29,0x2a,0x2c | journal screen |
| (70,452) | Camp (0x43) | `icons_standard` 3,0,1,2,4 | `RequestCamp` |
| (100,452) | Search (0x44) | `icons_standard` 8,5,6,7,9, toggle | `ToggleSearchMode`, pressed while searching |
| (130,452) | Use/Equip Item (0x45) | `icons_standard` 0x17,0x14,0x15,0x16,0x18, toggle | use-item view; in combat opens the Use popup |
| (160,452) | Cast Spell (0x46) | `icons_standard` 0x1c,0x19,0x1a,0x1b,0x1d, toggle | spell view. Enabled only if the character has a castable spell |
| (199,452) | Start Combat (0x47) | `icon_combat_toggle` 3,0,1,2,2 (89x26) | `ToggleCombatMode`. Visible outside combat |
| (200,452) | End Combat (0x48) | `icon_combat_toggle` 7,4,5,6,6 (28x26) | the same. Visible in combat |
| (232,452) | Defend (0x4a) | `icons_standard` 0x12,0xf,0x10,0x11,0x13 | combat only: Defend popup |
| (262,452) | Attack (0x49) | `icons_standard` 0xd,0xa,0xb,0xc,0xe | combat only: Attack popup |
| (300,456) | Prev Character (0x4b) | `main_scroll_arrows` 3,0,1,2,2 | select previous character |
| (323,456) | Next Character (0x4c) | `main_scroll_arrows` 7,4,5,6,6 | select next character |
| (541,452) | Party Move (0x4d) | `icons_standard` 0x21,0x1e,0x1f,0x20,0x22 | combat only: Run/Walk popup |
| (571,452) | Party Formation (0x4e) | `icons_standard` 0x26,0x23,0x24,0x25,0x27, toggle | opens/closes the formation panel; pressed while it is open |
| (610,450) | (see below) | 30x30 | combat round control |

Positions: `pc.cpp:59-90`. Frames: `pc.cpp:113-161` and `btn.cpp:255-470`.

**Bank enablement** (`UpdateSubMenuButton`, `pc.cpp:170-292`): the bank is grayed when any
of these holds:
- NPC dialogue, except Journal when allowed;
- a lock, trap or review overlay;
- an item or spell view, except its own button;
- a pending surprise, or camp;
- combat between rounds;
- an open popup.

Journal, Camp, Search and Start Combat are disabled in combat. Use and Cast require an
eligible character.

**Character caption** (`DrawSubMenuCharacterAction`, `btn.cpp:953-1060`), `smfnt` in the
character's party-order palette, centred in 185 px from x 343:
- Line 1 at y 454: "Name - Profession".
- Line 2 at y 465, exploring: the highest condition's name, if any.
- Line 2 in combat, the queued action:
  - "Cast Spell - spell (power)";
  - "Use Item - spell";
  - "Attack - weapon[/weapon]", with "Unarmed" (0x5b8) for an empty hand;
  - "Protect - target";
  - "Nothing" (0x7e1);
  - otherwise the action name.

**Combat stance button** (`UpdateCombatStanceButtons`, `btn.cpp:362-410`). It is hidden
outside combat. Exactly one of the five files is visible, all with frames 3,0,1,2,2:

| Index | File | Tooltip | Shown when |
|---|---|---|---|
| 0 | `attack_confirm` | Start Round (0x4f) | phased combat, round not executing |
| 3 | `cont_toggle` | Go Continuous (0x51) | phased combat, round executing |
| 1 | `combat_stop` | Go Phased (0x50) | continuous, may advance (or surprised), round active |
| 4 | `cont_pending` | Go Phased (0x50) | continuous, may advance, round not active |
| 2 | `cont_start` | Start Round (0x4f) | continuous, waiting |

- Left click: switches stance while a round is executing, otherwise starts the round.
- Right click: always switches stance between phased and continuous.
- The button is disabled during spell, item or review overlays.

**Combat popups** (`BuildSubMenuPanel`, `btn.cpp:1310-1460`):
- Geometry: `Controls` at (button x - 1, 451)-(+w, 479), background `main_submenu_bar`
  frame `entries - 2`, widths 47/66/85/104.
- Rows: `icons_submenu` 18x18 at local (5 + 19i, 5), followed by a Cancel row (frames
  0xbd/0xbf).
- Entries: Attack (Attack, Berserk, Breathe, Turn Undead, Pray); Defend (Defend,
  Protect); Use (Equip, Use Item, Use Last Item); Spells (Cast, Cast Last); Party Move
  (Run, Walk). Only available entries are listed.
- Right-click on the popup background closes it.

## 6. Layout modes and layout arrows

`g_settings.main_ui_mode` (decomp enum `PORTRAITS/FORMATION/RADAR` = 0/1/2; the roof
tooltips 0x38-0x3a name them **Closed / Wide / Full**). New games start in Wide with all
three bottom panels shown (`Gameplay Init.cpp:483-490`).

| Mode | Party | Bottom panels | Roof button frames (grayed, off, hover, on, on-hover) |
|---|---|---|---|
| Closed | full portrait columns | radar and formation forced on; text box per `portraits_action_panel_preference` | 9,0,6,3,6 |
| Wide | 23-px bar strips, slide-out on click | each per its `formation_*_preference` | 10,1,7,4,7 |
| Full | strips | all off | 11,2,8,5,8 |

`ApplyMainGameModeFlag` (`mgs.cpp:5125-5240`) applies a mode.

**Roof buttons:**
- Full is hidden during dialogue, spell, item, lock and trap overlays. Closed and Wide are
  disabled in dialogue and camp.
- Backspace cycles Closed → Wide → Full; Shift+Backspace cycles backwards. While input is
  blocked it only toggles between Closed and Wide (`MGSKeyboard.cpp:381-400`).

**Layout arrows**, all at x 0 on the left post (`pc.cpp:93-96`, `btn.cpp:640-787`):

| y | Visible when panel shown | Visible when hidden |
|---|---|---|
| 371 | Hide Radar (0x3e): frames 6,0,1,-,2 | Show Radar (0x3b): 7,3,4,-,5 |
| 394 | Hide TextBox (0x3f): 14,8,9,-,10 | Show TextBox (0x3c): 15,11,12,-,13 |
| 417 | Hide Formation (0x40): 22,16,17,-,18 | Show Formation (0x3d): 23,19,20,-,21 |

- In Closed mode, Hide Radar and Hide Formation are disabled; only the text box toggles.
- Hiding the last visible panel in Wide switches to Full.
- Showing a panel in Full switches to Wide with only that panel on.
- Each toggle stores its preference for the next time the mode is entered.
- The whole bank is hidden during dialogue, spell, item, lock and trap overlays.

## 7. Radar and compass (`rad.cpp`)

- **Panel:** `main_radar_back` at (23,358). The three sprites are 91x91 and centred at
  (30,359) + (45,45).
  - Disc: `formation_dot_small`, or `formation_dot_large` when zoomed in. The zoomed-in
    disc has a 2x2 `formation_jewels`[party order] chip per occupied formation cell, at
    `g_radar_cell_offsets` (`rad.cpp:23-26`).
  - Rim: `compass_rim`, or `compass_rim_high` while hovered.
  - View cone: `small_formation_cone`.
- **Compass rotation:** each update, the rim is rotated by `party_facing` degrees, so its
  north mark tracks the world. The cone is rotated by `party_facing - party_heading + 360`.
  **Uncertain**: the exact meaning of facing vs heading (body facing vs camera yaw), and
  the pivot (assumed to be the sprite centre). The creation order is cone, rim, map;
  **Uncertain** whether that is also the draw order.
- **Hit circle** `(75,402) r46` (region 0x62, tooltip "Automap/Radar Zoom" 0x1f):
  - left click opens the automap;
  - right click toggles zoom;
  - entering and leaving swaps the rim art.
- **Zoom:**
  - Out is the default outside combat: scale 2, inner radius = party radius, outer =
    Extreme range + radius.
  - In is forced when the radar opens in combat: scale 13, inner = Touch range + radius,
    outer = Long range + radius.
  - Key Z (316) toggles zoom.
- **Blips:** 2x2 coloured squares. Their delta from the party is rotated by
  `360 - facing` about Y, and only x/z are used. With `d` the horizontal distance:
  - Inside the inner radius, monsters snap to `scale` px from the centre, while items and
    missiles scale linearly by `scale / inner`.
  - Beyond it, the radius in px is `scale + (d - inner) * (38 - scale) / (outer - inner)`,
    capped at 38.
  - Screen position: (75 + x*k, 404 - z*k).
  - Monsters beyond the outer radius are clamped onto the 38-px rim.
- **Colours** (`g_radar_blip_colors`): class × height ring.
  - Ring: dim if the object is more than 1500 units below the party, bright if more than
    1500 above, middle otherwise.
  - Classes: 0 yellow (disposition 0 or 3), 1 red (disposition 1), 2 green
    (disposition 2), 3 grey (a monster no longer visible, at its last seen position), 4
    white/grey (items), 5 blue (missiles).
  - In combat, monsters targeted by the selected character glow.
- **What is shown:**
  - world items that can be picked up, once seen or in line of sight;
  - active monsters within viewing distance;
  - missiles in flight.
  - Party condition 0x40 shows everything (**Uncertain** which spell; likely Detect).
  - Nothing is shown while a surprise is pending.

## 8. Formation board (`frm.cpp`)

- `small_formation_back` at (512,358). The cage is `small_formation_cage`, or
  `small_form_cage_high` while hovered, at (519,359).
- The view cone `small_formation_cone` at (519,359) is rotated like the radar cone.
- Each occupied cell draws `small_formation_symbol`[facing*3] and
  `small_formation_jewels`[party_order*3] at (519,359) + `g_formation_marker_offsets[cell]`
  (`frm.cpp:4-9`). The highlighted slot uses +1 on both.
- Hit circle `(564,402) r40` (region 0x63, tooltip "Party Formation" 0x20): a left click
  opens the formation panel. A 10x10 hit box per marker gives that character's tooltip.

## 9. Text box (`txt.cpp`)

| Part | Where |
|---|---|
| Channel tabs | regions (140,362)-(163,377) General (0x21), (140,383)-(163,398) Combat (0x22), (140,404)-(163,419) Dialog (0x23). The tab art frame is 0/1/2 for the selected channel (`GetTextBoxMode` 0/1/3) |
| Body | region 0x55 (166,362)-(456,445). Up to 7 lines, 11 px apart, `wiz_text_font` |
| Up arrow | `main_scroll` at (461,363): 0 enabled, 1 hovered, 3 disabled at the top. Region (461,363)-(476,378) |
| Down arrow | `main_scroll` at (461,429): 8 enabled, 9 hovered, 11 disabled at the end. Region (461,429)-(476,444) |
| Thumb | `main_scroll` 4 at (461, 379 + scroll*40/(lines - visible)), only when the text overflows. Track region (465,379)-(472,428) |

- Lines are centred in the body, except in the Dialog channel, which is left-aligned
  (`txt.cpp:1955-1972`).
- Selecting the Dialog tab scrolls to show the last 7 lines.
- PgUp, PgDn, Home, End and Del (500-504) scroll or clear the box.
- The mouse wheel over the body scrolls by its line delta; right-clicking an
  arrow scrolls seven lines (`TextBoxBodyRegionEvent`, `TextBoxScrollUpRegionEvent`,
  `TextBoxScrollDownRegionEvent`). Dragging the thumb and repeated arrow presses
  are still unimplemented.
- **Uncertain**: `GetTextBoxVisibleLineCount` returns 1 outside spell, item, camp and
  dialogue modes, which makes the thumb range "lines - 1" there.

## 10. Hit regions and mouse behaviour

Portrait regions in the Closed layout (`reg.cpp`, regions 0x22-0x51). These are slot 0 /
slot 1; add 85 to y per row.

| Region | Slot 0 | Slot 1 | Tooltip | Behaviour |
|---|---|---|---|---|
| Portrait | (23,18)-(104,89) | (535,18)-(616,89) | "Select/Slide Out Character" or "Take Item" | see below |
| Hands | (4,42)-(19,90) | (621,42)-(635,90) | "Swap Weapons" (0x1c) | hover: upper half (y < top+25) or lower; left click swaps weapon sets (Shift: whole party); right click opens item info for the hovered weapon |
| Vitals | (109,42)-(123,90) | (517,42)-(531,90) | "View Stats" (0x18) | info plate while the left button is held |
| Condition socket | `(32,27) r8` | `(608,27) r8` | "View Conditions" (0x19) | plate while held, if the character has a condition |
| Enchantment socket | `(95,27) r8` | `(544,27) r8` | "View Enchantments" (0x1a) | the same, for enchantments |
| Name strip | (21,91)-(105,101) | (539,91)-(612,101) | none | swallows clicks |
| Combat action | (24,68)-(42,85) | (595,68)-(613,85) | none | combat: opens the action menu for that character |

**Portrait clicks** (`PortraitSelectRegionEvent`, `mgs.cpp:5387-5630`):
- Left click:
  - while choosing an ally target (or with Ctrl when the queued action needs one), aims at
    that character;
  - with an item in the cursor, gives it to the character (to the party pool in combat or
    if ineligible);
  - otherwise selects the character, and in Wide/Full also slides the portrait out.
- Double click: opens the character (review) screen.
- Right: a release, or holding for 1000 ms, opens the character screen. With an item held
  and no Ctrl, the release gives the item instead.
- Hover: highlights the monsters this character targets.

Other regions:
- World view: region 0xe6, tooltip "Viewport" (0x17), always the current viewport rect.
- Panel backgrounds (radar 0x59, text 0x59, formation 0x64) swallow clicks.

### Keys (`data/strings/mgskeyboard.ini`, UTF-16; command ids from `include/wiz8/mgs_commands.h`)

| Key | Command |
|---|---|
| Esc | cancel |
| Alt+X | quit |
| Ctrl+F12 | fullscreen |
| Arrows | move/turn; Shift runs |
| Num0 / Num. | strafe left/right (Shift runs) |
| Num8 / Num2 / Num5 | look up/down/level |
| Pause | pause |
| Backspace / Shift+Backspace | next / previous layout |
| Tab | automap |
| O / J / I | options / journal / inventory |
| U / Shift+U | use item / use last item |
| M / Shift+M | cast / cast last spell |
| C / S / \` | camp / search / combat toggle |
| Q / Shift+Q | quick save / quick load |
| Z | radar zoom |
| ' | replay last quote |
| W / Shift+W | swap weapons / swap all |
| 1-6 | select PC 1-6 |
| 7, 8 | select recruited NPC 1, 2 |
| PgUp / PgDn / Home / End / Del | text box |
| Enter | start round |
| \\ | continuous combat |
| Space | camera lock |
| N | cycle target |
| A, K, B, T, Y, D, P, E | Attack, Berserk, Breathe, Turn Undead, Pray, Defend, Protect, Equip |
| R / Shift+R | party walk / run |
| L | repeat action |
| Ctrl+L / Ctrl+S | load / save options |
| Ctrl+, / Ctrl+. | combat delay up / down |
| Ctrl+[ / Ctrl+] | text delay up / down |
| Ctrl+V | show version |

Actions 1000 and above in the file are debug/cheat bindings and are not listed.
**Uncertain**: which party slots "PC 1-6" and "NPC 1-2" map to.

## Uncertain

- Viewport modes 3, 5 and 6 (section 2) are defined but never selected.
- The rotation pivot and draw order of the radar sprites, and facing vs heading.
- `g_portrait_descriptors[].render_mode` (portrait mirroring) was not decoded.
- `icons_portrait` frame selection (`GetCombatPortraitImage`) and the `icons_submenu`
  frame table (`g_submenu_entry_message_ids` + state offsets) are not tabulated here.
- `main_textbox_back` frames 1-3 (384x92, 308x84, 330x84) are not used by the code
  read here; spell, item or dialogue panels may use them.
- The info plates (`popup_edge`/`popup_divider`/`popuphighlight`), the party-movement
  panel and the combat keyboard menu (`combatportraitpanel`) are out of scope.
- No coordinate here was checked against a running retail game. The composite renders
  agree with the art.
