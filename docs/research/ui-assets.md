# Wizardry 8 UI image and font assets

Sources: reference C++ at `whizzardry-8` (cited as `path:line`), retail data at
`wizardry-prepared/data`. Every claim about file contents was checked with
`stci.py` (same directory) against the retail files.

## 1. Formats in use

| Format | Where | Notes |
|---|---|---|
| STCI (`.sti`) | 1327 files: all UI chrome, fonts, portraits, level-load screens, cursors | see section 2 |
| TGA | `automap/map_*_a.tga` (6), `icons/*.tga` (42), plus the 3D textures | automap markers are type 2 (uncompressed true-colour), 32 bpp BGRA, 64x64, descriptor 0x08 (8 alpha bits, bottom-up rows) |
| PCX | `main interface/basic_fill_texture.pcx`, one in `npc interaction/` | 8-bit, loaded through SDL_image (`src/canvas/himage.cpp:104-115`) |

The loader picks the decoder by extension: PCX/TGA/JPG go to SDL_image and STI to
`load_sti`. A path with no extension gets `.PCX` appended (`src/canvas/himage.cpp:43-56`, `286-298`).

## 2. STCI layout

Header struct: `include/wiz8/canvas/imgfmt.h:22-54`. It is 64 bytes, little-endian, and
`static_assert`ed at `imgfmt.h:71-77`.

| Off | Type | Field |
|---|---|---|
| 0 | char[4] | `"STCI"` |
| 4 | u32 | uiOriginalSize (decoded bytes; for ETRLE it is canvas w*h, not used) |
| 8 | u32 | uiStoredSize, the byte count of the pixel block |
| 12 | u32 | uiTransparentValue (0 in every retail file; unused by the loader) |
| 16 | u32 | fFlags: `0x04` RGB, `0x08` INDEXED, `0x10` ZLIB, `0x20` ETRLE (`imgfmt.h:17-20`) |
| 20 | u16 | usHeight |
| 22 | u16 | usWidth |
| 24 | union, 20 bytes | **RGB:** u32 R/G/B/A masks @24/28/32/36, u8 depths @40..43. **Indexed:** u32 numColours @24, u16 numSubImages @28, u8 R/G/B depth @30..32, 11 unused bytes |
| 44 | u8 | ubDepth: 8 (indexed), 16 or 24 (RGB) |
| 45 | u8[3] | padding |
| 48 | u32 | uiAppDataSize |
| 52 | u8[12] | unused |

The blocks after the header come in this order (`imgfmt.h:5-9`, `himage.cpp:228-233`):
1. Palette: `numColours * 3` bytes of (r,g,b), indexed files only. The loader insists on
   256 colours (`himage.cpp:236-246`) and makes every colour opaque.
2. Sub-image table: `numSubImages * 16` bytes, ETRLE files only (`imgfmt.h:56-63`):
   `u32 dataOffset, u32 dataLength, i16 offsetX, i16 offsetY, u16 height, u16 width`.
   `dataOffset` counts from the start of the pixel block.
3. Pixel block: `uiStoredSize` bytes.
4. App data: `uiAppDataSize` bytes. The native loader only bounds-checks it
   (`himage.cpp:250-253`).

Validation rules (`himage.cpp:213-227`): exactly one of RGB or INDEXED must be set, RGB
requires depth 16 or 24, indexed requires depth 8, and ETRLE cannot be combined with ZLIB.

### Pixel encodings
- **Indexed + ETRLE (flags 0x28): 1126 retail files.** The file decodes to a
  `SpriteSheet` of frames (`himage.cpp:151-207`). Each row is a series of control bytes:
  - `0x00` ends the row. The row must then be exactly `width` wide.
  - `c & 0x80`: skip `c & 0x7f` **transparent** pixels.
  - otherwise: copy `c` literal palette indices.
  - A run count of 0 or a run past the row width is an error.

  Transparent pixels are stored as a sentinel (`SpriteFrame::kTransparent`) and skipped
  when drawing (`src/canvas/sprite.cpp:63`). Palette index 0 written as a literal is
  **opaque**. A frame is drawn at `(x + offsetX, y + offsetY)` (`sprite.cpp:19-23`).
  For ETRLE files the header's width/height is only the authoring canvas size. For
  example, `map_cursors.sti` declares 640x480 but its frames are 23-26 px.
- **RGB 16-bit (flags 0x04): 201 retail files.** Every one uses masks
  `R=0xF800 G=0x07E0 B=0x001F A=0`, i.e. **RGB565**, stored row-major and uncompressed
  (`himage.cpp:277-282`). They live in `portraits/small` (79), `portraits/large` (80),
  `level load` (38) and `options` (4).
- Plain indexed (no ETRLE), ZLIB and 24-bit files are supported by the loader
  (`himage.cpp:262-276`) but **no retail file uses them**.
- **App data:** 9 files have it (`flics/compass.sti`, `flics/viewcone.sti`,
  `flics/formationsymbol.sti`, `flics/realms/*.sti`), 16 bytes per sub-image. The layout
  matches JA2's AuxObjectData: byte 8 is the frame count (e.g. 0x48 = 72 for the compass)
  and byte 9 is a flags byte (0x02, "animated"). It can be ignored, as the native port does.

Colour conversion to the 16-bit frame buffer: a non-black colour that packs to 0 is
nudged to 1 so it stays opaque (`himage.cpp:35-36`, `355-364`). In Bevy, convert the
palette to RGBA8 and map transparent runs to alpha 0.

## 3. Fonts (`data/fonts/*.sti`, plus `journal/journal_font.sti`, `level load/levelload_font.sti`)

- A font is an ordinary ETRLE sprite sheet with one sub-image per glyph
  (`src/canvas/Font.cpp:207-214`). It has no separate metrics table.
- **Glyph index to character** mapping (`Font.cpp:21-124`):
  - 0-25 `A-Z`, 26-51 `a-z`, 52-61 `0-9`.
  - 62-92 `! @ # $ % ^ & * ( ) - _ + = | \ { } [ ] : ; " ' < > , . ? /` then space at 92.
  - 93-148 are 56 accented Latin-1 code points, in this order:
    `193 192 193 196 195 197 199 201 200 202 203 205 204 206 207 209 211 210 212 214 213 216 218 217 219 220 221 225 224 226 228 227 229 231 233 232 234 235 237 236 238 239 241 243 242 244 246 245 248 250 249 251 252 254 255 223`.
  - 149-153 are the private-use targeting icons U+FFF0..FFF4
    (`include/wiz8/canvas/Font.h:20-24`).
  - 250 is `¿` (191) and 251 is `¡` (161).

  Lookup returns the first match, and an unknown character falls back to `?`
  (`Font.cpp:263-272`).
  Retail glyph counts: 93 (ASCII-only fonts), 144-154 (fonts with accents and icons,
  e.g. `wiz_text_font.sti` has 154), and 188-190 (`font10arial*`, `font12point1`).
- **Metrics:**
  - Advance = `frame.width + frame.offsetX` (`Font.cpp:216-221`).
  - Line height = `height + offsetY` of glyph 0 ('A') (`Font.cpp:248-259`).
  - Text is drawn left to right with no kerning (`Font.cpp:274-298`).

  Nearly all fonts have offsets of 0 and a constant cell height (e.g. wiz_text_font 11,
  largefont 17, dialog_font 18, embossed/engraved 29). `bfont3`/`sfont3` use non-zero
  offsetY.
- **Palette and colour:** glyph pixels index the sheet's own 256-colour palette.
  - Recolouring swaps the whole palette (`Font::setPalette`, `Font.cpp:134-138`).
  - The 15 "state palettes" come from `fonts/palette{red,green,purple,blue,orange,yellow,pink,brown,white,rust,bronze,gray,beige,optgreen,optwhite}.sti`
    (catalog 0x1e5 + n, `src/wiz8/startup_subsystems.cpp:110-114`; enum at
    `include/wiz8/rules/events.h:31-43`). Each of those files is a 2x2, 1-frame STI that
    exists only for its palette, e.g. palettered entries 1..n = (239,0,20).
  - Glow and shade tables are derived palettes (`Font.cpp:157-193`, `himage.cpp:338-353`).
    The glow is a luminance-times-colour mono palette. Shades are scaled by
    293/281/268/–/195/…/0 out of 256.
  - Mono blend: index 1 is the shadow colour and any other non-zero index is the
    foreground colour (`sprite.cpp:53-61`).
- The font roles used by the game are listed at `src/wiz8/startup_subsystems.cpp:74-98`.
  Body text uses `Wiz_Text_Font.sti`.

Sample: `font_wiz_text_font.png` is a contact sheet of 154 glyphs, 16 per row, in index
order.

## 4. Automap / map-notes assets

Screen layout is 640x480 (`src/wiz8/local_screens/AutomapScreen.cpp`):
- Map viewport rectangle: x 12..467, y 32..467 (`:72-73`).
- Background: catalog 0x14a = `automap/map_border.sti` (`:346`).
- Buttons: 4 frames per button, used as grayed=base+3, off=base, hilite=base+1,
  on=base+2 (`:271-281`; signature in `include/wiz8/dialog_code/ButtonAppearance.h:48-53`).
- Catalog ids were resolved through `src/wiz8/local_code/CatalogArtwork.cpp:36-759`.

| Asset | Size / frames | Use (catalog id, screen pos) |
|---|---|---|
| `automap/map_border.sti` | 1 frame, 640x480; the viewport is opaque black and the button wells are transparent | full-screen frame (0x14a) |
| `automap/map_notebuttons.sti` | 8 frames of 40x40. 0-3 = **Add note**, 4-7 = **Erase note** | 380 at (490,70), 384 at (541,70) |
| `automap/map_markerbuttons.sti` | 12 frames of 40x40, 3 buttons | page cycle (348/352/356) at (591,70) |
| `automap/map_zoombuttons.sti` | 12 frames of 40x40 | zoom in (490,124), out (541,124), reset (591,124) |
| `automap/map_layerbuttons.sti` | 8 frames of 40x40 | layer down (490,164), up (591,164) |
| `automap/map_movementbuttons.sti` | 20 frames of 40x40 | pan N (540,218), S (540,326), W (486,272), E (594,272), centre (540,272) |
| `automap/map_exitbutton.sti` | 4 frames of 42x34 | exit at (588,441) |
| `automap/map_cursors.sti` | 5 frames: 26x26, 25x25, 26x26, 26x26, 23x24 | tool cursors, catalog 0x14b + tool. Hotspots `{0,0},{8,7},{1,24},{1,24},{8,7}` (`:75`). Tool 2 = AddNote, 3 = EraseNote (`include/wiz8/local_screens/AutomapScreen.h:24`) |
| `automap/map_selectedbuttonrim.sti` | 1 frame, 34x34 | selected-tool rim |
| `automap/map_textmarker_a.tga` | 64x64 BGRA32 | **note marker blip**. Scaled by zoom; glows black to orange (1,0.4,0) with a 2000 ms period (`:912-926`) |
| `automap/map_partymarker_a.tga`, `map_monster{friendly,neutral,hostile,marker}_a.tga` | 64x64 BGRA32 | party and monster blips (`:722-738`) |
| `automap/automaplarge.sti` / `automapsmall.sti` | 640x480 / 100x72, 1 frame | not referenced by the reference AutomapScreen |
| `automap/mapfilters.txt` | INI-like `[LEVEL_X]` sections with `LAYER=<height>` lines and texture names | layer split heights per level (`:361`) |

Note text is drawn with `g_wiz_text_font_secondary` (Wiz_Text_Font.sti):
- It sits to the right of the marker (marker width + 2 px) and is vertically centred
  (`AutomapScreen.cpp:826-856`).
- While a note is being edited, a 1-px box in RGB(239,155,86) surrounds the text.
- A note holds at most 40 UTF-16 units (`:70`) and a map holds at most 200 notes
  (`src/world/automap.cpp:30`).
- Save format, in the level's AUTO chunk (`src/world/automap.cpp:173-221`):
  - optional packed visited bits
  - `u32 0xF00DF00D`
  - `u32 count`
  - count x {`f32 x, f32 z, i32 layer, u32 len (UTF-16 units incl. NUL, 1..40), u16[len] text`}
  - `u32 0xF00DF00D`

Samples in this directory:
- `automap_map_border.png` (transparent areas shown dark purple)
- `automap_map_notebuttons.png`
- `main_compassrim.png`
- `levelload_savanttower_rgb565.png` (an RGB565 sample)

## 5. `stci.py`

Stdlib-only decoder and PNG writer. Commands:
- `scan <root>`
- `info <f>`
- `png <f> <out>`: contact sheet for ETRLE, bitmap for RGB
- `frame <f> <i> <out>`

`python3 -I stci.py scan wizardry-prepared` gives **1327/1327 decoded, 0 failures**
(1126 indexed8+ETRLE, 201 RGB565). There are no trailing bytes in any file and app data
appears only in the 9 flics files.
