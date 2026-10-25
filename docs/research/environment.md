# Wizardry 8 level environment: sky, fog, lighting, animated textures, filtering

Evidence: reference C++ at `/home/agluszak/code/personal/decomp/whizzardry-8` (paths are relative to it).
Retail data: `/home/agluszak/code/personal/decomp/wizardry-prepared`.
Scripts in this directory:
- `env_scan.py <retail root>` finds and dumps the PVL "fog options" section of every level.
- `pvl_materials.py <retail root>` dumps the PVL material table, including the animation fields.

Units: the C++ multiplies authored light, view and camera values by `kWorldScale = 500`
(`include/wiz8/gameplay_constants.h:32`) to get engine units. Level geometry is already in engine
units, the same space you divide by 600 to get metres. In the rest of this document "units" means
engine units.

## 1. Data: the PVL environment section

`ReadLevel` reads the sections in this order (`src/world/level_loader.cpp:935-1007`):
- lights, monsters, items, missiles, props, bitmaps, cameras;
- an `int count`;
- the environment, read only when `count != 0`; with `count == 0` the far clip is 42500 and the
  render range 37500 (`:956-962`);
- triggers, a sway byte, clip planes;
- `vec3 environment_offset` (`:997-1009`), then particles, named positions, automap.

Each PVL section ends with an `int -1` (`CHECK_PVL_OFFSET`, `:873-879`).

Environment layout (`ReadWorldEnvironment`, `level_loader.cpp:269-355`):

| field | type | meaning |
|---|---|---|
| fog_enabled | u8 | 1 turns on fog and the sky (section 3) |
| range.x | f32 | fog **start**, as a fraction of the far clip (`range_start`) |
| range.y | f32 | fog **end**, as a fraction of the far clip (`range_end`) |
| range.z | f32 | unused (always 1.0) |
| intensity | f32 | multiplies the ambient ramp (0 = no ambient or sun light, used for interiors) |
| view_distance | f32 | far clip = view_distance x 500 units |
| camera_mode | u8 | 0: nothing follows. 1: `Vec3f` start position. 2: `Vec3f` position, then `f32` angle and `f32[3]` axis |
| has_light_colours | u8 | if 1, 256 x RGB8 follow: the **fog/clear colour ramp** (the C++ calls it `light_colours`) |
| has_environment_colours | u8 | if 1, 256 x RGB8 follow: the **ambient (sun/sky light) ramp** (`colours`) |

Each table is converted to float as `byte/255`, clamped to [0,1] (`src/world/environment.cpp:212-232`).

Without a table the built-in default is used: grey `v=(i<128?i:255-i)/127` (`environment.cpp:234-242`).
Retail always ships both tables.

The render range is `1.5 x far` (or `2 x` when view_distance < 100) (`level_loader.cpp:346-351`).
It is only a culling distance.

### Verified retail values (`env_scan.py`)

Every one of the 34 retail PVLs has `fog_enabled=1`, `count=1`, `range.y=0.95`, both tables, and
camera_mode 0. The exceptions are `arnika_starterdungeon` and `seacave1`, which use camera_mode 2.

| level | fog start | intensity | view (x500 units) |
|---|---|---|---|
| monastery1 | 0.80 | 1.0 | 110 → far 55000 u, fog 44000-52250 u (~73-87 m) |
| monastery2 | 0.80 | 1.0 | 100 |
| arnika2 / trynnie1 / trynnie2 | 0.70 | 1.0 | 90 |
| swamp / ratkin | 0.30 | 1.0 | 90 |
| bayjin2, mtgigaswatercaves | 0.00 | 1.0 / 0.0 | 100 / 110 |
| circle | 0.70 | **0.0** | 260 |
| interiors: marten1, dungeon, rapaxcellar, rapaxmainfloor, mtgigas1/2, savanttower, mtgigaswatercaves | — | **0.0** | 90-110 |

- **Ambient ramp:** all 34 levels share one ambient ramp.
  Samples by phase: 0 `2d3f58`, 32 `221f26`, 64 `3e3433`, 96 `7b7572`, 128 `9d9b9c`, 160 `81777a`,
  192 `655258`, 224 `494553`, 255 `2d3f58`.
- **Fog ramps:** there are five distinct fog ramps.
  - The standard outdoor ramp is used by 23 levels, monastery included.
    Samples by phase: 0 `00000c`, 32 `6a483e`, 64 `6a483e`, 96 `c19d73`, 112 `7794bc`, 128 `69a2ef`
    (noon sky blue), 160 `bc9b75`, 192 `775846`, 224 `4a322f`, 240 `171015`.
  - Interiors use all-black ramps.
  - mtgigas1/2 and savanttower use a constant `120b05`.
  - bayjin2 and mtgigaswatercaves use a cyan ramp.
  - swamp has its own ramp.

## 2. Day clock and phase (time of day: YES, it changes everything)

- **Clock:** `game_time_ms` is in [0, 86 400 000). A new game starts at `0x2932E00` = 43 200 000
  = **noon** (`src/wiz8/local_code/Gameplay Init.cpp:69`).
- **Clock rate:** each frame adds real elapsed time x `g_view_distance` (`environment.cpp:150-158`,
  `src/wiz8/game_advance.cpp:57`). That global is confusingly named: it is the game-clock
  multiplier, **12** by default (`src/wiz8/local_code/Sight.cpp:60`) and 2880 while resting
  (`GameplayTime.cpp:307`). So one game day is 2 real hours.
- **Phase:** `phase = ((game_time_ms/1000) << 8) / 86400`, giving 0..255. 128 is noon and 0 is
  midnight (`environment.cpp:143,253,328`).

When the phase changes:
- **Ambient:** ambient = `clamp(colours[phase] * intensity)`. It goes into the scene ambient and into
  the `ambient` of every PVL light whose name starts with "Sun" (`ApplyEnvironmentColour`,
  `environment.cpp:578-591`; Sun lights are registered at `level_loader.cpp:245-249`).
  `RefreshEnvironment` is at `:323-333`.
- **Fog colour:** fog colour = `light_colours[phase]`. `PublishLightDirection` is misnamed: it
  calls `scene->setFogColor` on both the level scene and the sky scene (`src/render/frame.cpp:555-563`;
  `environment.cpp:247-258`).
- **Clear colour:** the frame clear colour is that fog colour while the sky or fog is enabled
  (`frame.cpp:437-445`).

## 3. Sky

**What it is.** The sky is a **separate world** loaded from its own `.LVL` file, a small uncompiled
level with dome meshes and props (`src/world/level_session.cpp:212-269`). It is not a skybox. It is
enabled at level entry when `fog_enabled` is set (`src/wiz8/local_screens/MainGameScreen.cpp:2260-2264`,
`EnableSky` at `environment.cpp:313-319`). The "Mesh sky" option can hide it
(`src/render/quality.cpp:62`).

**Which file.**
- Each level picks a sky through `g_level_folders[level].sky_index` into `g_sky_names`
  (`level_session.cpp:86-134, 286-291`):
  `DefaultSky, RapaxSky, MountainPassSky, RiftSky1, TrynnieSky1, TrynnieSky2, RatkinSky1, CampSky, BluffSky1, BluffSky2, CircleSky`.
- The path is `Levels/<Folder>/<SkyName>.LVL`. If that file is missing it falls back to
  `Levels/Test/DefaultSky.LVL` (`level_session.cpp:334-345`).
- Only two sky files exist in retail: `levels/test/defaultsky.lvl` (15 210 B) and
  `levels/circle/circlesky.lvl`. So every sky except Circle's is DefaultSky.
- Monastery (sky_index 0) gets `test/defaultsky.lvl`.
- Levels with `sky_index -1` have no sky: Bayjin, Marten1, the MtGigas caves and floors 1/2, Swamp,
  Trynnie, SavantTower, Ratkin, Dungeon. They show only the fog-coloured clear.
- Sky textures come from `levels/test/bitmaps/` (for Circle, `levels/circle/bitmaps/`).

**DefaultSky contents** (strings in the file):
- meshes "Sky gradient" (`SkyGrad0000.ifl`), "Skytop" (`Skytop0000.ifl`) and "Horizon Blue"
  (`Horizon0000.ifl`);
- props `Clouds01..08` (`TRY2-Clouds_a.tga`), "Clouds Bottom" (`nightSky-clouds.tga`), `Sun`
  (`Sunflare.tga`) and `Moon` (`moon.tga`, `Moonflare.tga`).
- The three IFLs have 256 frames each: `skygrad*.tga` 64x64, `skytop*.tga` and `horizon*.tga` 16x16.
  All are 24-bit **RLE TGA (image type 10)**, so your TGA loader must handle RLE.

**Day-phase behaviour** (`InitializeLevelEnvironment` at `environment.cpp:661-702`,
`AdvanceEnvironmentTime` at `:90-148`):
- **Gradients:** the three gradient IFLs are found by name. Their playback is set to MANUAL and
  `frame = phase` every update, so one frame per 1/256 of a day.
- **Celestial origin:** the origin is the midpoint of the authored `Sun` and `Moon` prop positions,
  and the orbit radius is half their distance.
- **Moving body:** by day (06:00 < t ≤ 22:00 exclusive bounds: 18 000 000 < t ≤ 79 200 000) the Sun
  moves and the Moon sits at the origin. At night it is the reverse.
- **Orbit position:** `pos = origin + Rz(angle - 40°) * (0, radius, 0)`.
  - Day: `angle = (t - 18e6) * 80° * 1.634e-8`.
  - Night: `angle = (t + 7.2e6 or t - 79.2e6) * 80° * 3.968e-8`.
  - Rotation is about the sky's local Z.
- **Clouds** (`level_session.cpp:212-257`, `environment.cpp:57-66`):
  - Every `CloudsN` prop gets a shared "AnimatedCloudMaterial". It has ambient 1 and diffuse 0, and
    its UV scrolls at **0.002 texture units/s on U**.
  - Cloud brightness is `b = 1 - |t - noon| / noon`. Material ambient = `b`, alpha = `0.75 b + 0.25`.
- **Sky camera:** it stays at the sky's origin and copies only the party camera's **rotation** and
  view plane (`src/world/world.cpp:251-265`). The sky has no parallax.
- **Draw state:** all sky meshes get depth func ALWAYS (`SetWorldSceneMeshShaderLowBits(sky,0)`).
  Depth write stays on except for sorted meshes (`SetWorldSceneMeshShaderBit3`,
  `level_session.cpp:253-254`, `src/world/update.cpp:325-360`). The sky's camera light is removed.
- **Order:** each frame draws the clear (fog colour), then the sky scene with its camera, then the
  level scene (`frame.cpp:455-458`).
- **Bevy recipe:** a second camera with lower `order` renders the sky layer from the origin with the
  rotation only. Clear the depth buffer, or disable depth on sky materials, before the level camera.

## 4. Fog

- **Fog objects:** when the sky is enabled, two `FogParams` (static and dynamic) are created with
  density 1.0, `fog_start = far * range_start`, `fog_end = far * range_end` (`environment.cpp:262-297`,
  `RefreshFogRanges` at `:503-515`).
- **Packing:** each fog is packed as `{start, 1/(end-start), density, end}` (`src/render/vsg_material.cpp:259-268`).
- **Fog is linear, per vertex, on eye distance** (`src/render/shaders/wizardry_vertex.glsl:226-238`):
  `f = clamp((|eye| - start)/(end - start), 0, 1) * density`.
- **Layer combining:** layers combine as `f = f*(1-d)+d`. The port attaches both fog objects to the
  one level scene, which compounds them (`2f - f^2`). Retail attached one fog per scene, so
  **implement a single linear fog**. This is an inference from the static/dynamic naming.
- **Fragment** (`wizardry_fragment.glsl:24-37`): `rgb = mix(rgb, fog_colour, f * material.fog_scale)`.
  `fog_colour = light_colours[phase]`. Level materials use shader `0x0100a51b`
  (`src/render/resources/material_loader.cpp:62`), whose fog mode (bits 8-9) is 1 = blend toward the
  fog colour.
- **Monastery at noon:** fog runs from 73 m to 87 m toward `#69a2ef`, and the far clip is 91.7 m.
- **Depth cue:** a camera-range "depth cue" also exists (`setEnvironmentRange`, `frame.cpp`/`camera.cpp:53-90`).
  It only applies to materials with the DEPTH_CUE operation, which level materials do not have.

## 5. Final vertex colour (level geometry)

### CPU bake per vertex

From `Mesh::updateBakedLighting`, `src/render/resources/mesh_frames.cpp:87-133`. Inputs come from
`SetAmbientColor` (`mesh_frames.cpp:388-397`) and `mesh_instance.cpp:423`.

```
A    = scene ambient = clamp(colours[phase] * intensity)       // 0 for interiors
sun  = PVL per-vertex sunlight weight (0..1; 1.0 if mesh version 0)
mA   = material.ambient (PVL record .ambient; textured mats: usually 1.0, sometimes 0.1 or 0)
L    = PVL static vertex light (RGB, already includes material diffuse)
E    = PVL environment_offset (vec3 after clip planes; level_loader.cpp:997-1009)
k    = brightness = g_monster_light_scale (1.0 except during fade-to-black transitions)
baked = ((A == 0 ? 0 : A * sun * mA) + L + E) * k
```

### GPU per vertex

From `wzShadeVertex`, `wizardry_vertex.glsl:96-306`. Instance ambient is 0 for baked meshes
(`vsg_material.cpp:296-299`).

```
diffuse = clamp(material.emissive + baked + Σ dynamic lights)
emissive = (emission, emission, emission) for textured level materials (material_loader.cpp:157-165)
```

Fragment: `colour = texel * diffuse` (gradient MODULATE), then fog (section 4).

### Your provisional constant

Your `0.75` should be `colours[phase] * intensity * material.ambient`. At noon in the monastery that
is ≈ (0.616, 0.608, 0.612) x mA. At midnight it is (0.18, 0.25, 0.35), bluish.

### Dynamic lights at runtime

Light records are in `level_loader.cpp:160-262`.

- **Static PVL lights:** group mask 1. Level meshes have exclusion mask 1
  (`SetModelInstanceExclusionMask(...,1)`, `level_loader.cpp:916,926`), so these lights **do not**
  light level geometry at runtime because they are already in L. They do light props and monsters.
- **"Sun" lights:** ambient-only, group mask 1|4. Their ambient follows the day ramp. Props flagged
  NO_DIRECT_SUN get exclusion mask 4 and a material with ambient 0 and emissive + E
  (`level_loader.cpp:1011-1036`).
- **Camera ("torch") light:** group mask 2, diffuse (1, 0.85, 0.39), intensity 1, range 4000 u
  (`level_loader.cpp:1677-1684`). It is hidden on level entry (`SetSkyNodeVisible(false)` despite the
  name, `level_session.cpp:361`, `environment.cpp:595-606`). Light-spell modifiers enable it and set
  its intensity (`src/wiz8/local_code/Gameplay Mods.cpp:23-65`), up to 50 in 0.5 steps.
- **Attenuation:** dynamic point lights (missiles, spells, camera) use OpenGL-style attenuation
  `1/(1 + k d)`, `k = (1-a)/(range*a)`, `a = 0.0019569` (`update.cpp:275-282`,
  `src/render/effects/light.cpp:84-95`). They add `diffuse * intensity * k_brightness * att * max(0, N·L)`.
- **Bake falloff:** the bake-time falloff for non-Sun lights on props is linear,
  `intensity * (1 - d/range) * facing` (`update.cpp:103-157`).

## 6. IFL animated textures

**File format.**
- Plain text, CRLF, one frame file per line, relative to the `.ifl`'s folder.
- Lines may repeat a frame to hold it longer, e.g. `candle-0000.ifl`, `glow_torch.ifl`.
- **Parsing stops at the first line of 2 characters or fewer**, i.e. a blank line
  (`material_loader.cpp:208-232`). Some retail IFLs have trailing blank lines.
- There is no timing syntax.

**Playback state.** It comes from the PVL material record (`include/wiz8/render/material_loader.h:28-45`):
- `animation_mode` u8 @0x10d: 0 LOOP, 1 PING_PONG, 2 PLAY_ONCE, 3 MANUAL (`include/wiz8/texture_animation.h`);
- `animation_frame` i32 @0x10e: start frame;
- `animation_rate` f32 @0x112: **frames per second** (default 15, `include/wiz8/render/texture.h:32`);
- `texture_modes[4]` f32 @0x11a (version ≥ 4): for the selected slot, a value in (0,1] means
  RANDOM_FRAME with that probability, and a value > 1 means RANDOM_START with probability `v - 1`
  (`material_loader.cpp:100-109`).

**Frame advance** (`Texture::UpdateFrame`, `src/render/resources/texture.cpp:302-378`):
- `n = floor(elapsed_ms * rate / 1000)` since the animation started, where `count` is the number of
  frames.
- LOOP: `frame = n % count`.
- PING_PONG: forward on even cycles, `count-1-(n%count)` on odd cycles.
- PLAY_ONCE: clamps at `count-1`.
- MANUAL: never advances. Used by the sky gradients, which are driven by phase.
- RANDOM_FRAME: each update, with probability p, jump to a random frame.
- RANDOM_START: idle until a roll succeeds, then play from frame 0.
- The clock is the world/visual clock, so it pauses with the game.

**Verified across 4 353 retail level materials** (`pvl_materials.py`):
- All records are version 4 with `texture_modes = -1`. **Random modes are never used by level
  geometry.**
- IFL materials:
  - mode 0 (loop) at 1-30 fps, mostly 10/12/15;
  - mode 2 (play once) for the `BSmoke0006` and `Part-0004` smokes at 1-5 fps;
  - mode 1 (ping-pong) for camp coals, monastery2 `COMP_01` at 7 fps and `Candle-0000` at 16 fps,
    `Caustic0010`, and `Fireflyblink-00`.
- Monastery1: Candle 15, CausticD 8, Water 12, WaterFlow 30, Slime 10, Glow_torch 10 (all loop).
- The same IFL can appear in several materials with different rates.
- The animation object is shared per IFL **load**, i.e. per material (`material_loader.cpp:213-217`).

## 7. Texture filtering and mipmaps

- **Defaults:** `mag = min = FILTER_GOOD` (linear) and `mipmap = MIPMAP_FASTEST`, which means mip
  levels are generated but chosen by nearest selection, not trilinear blending
  (`texture.cpp:42-43, 192-208`; `computeNumMipMapLevels`, `maxLod 16`). Wrap mode is REPEAT.
- **Options:** the Advanced Graphics "Mip mapping" option switches between `MIPMAP_BEST` (trilinear)
  and `NONE` (`src/render/quality.cpp:53-61`, `OptionsScreen.cpp:944`). The mag/min options switch
  between linear and nearest.
- **"High texture detail" off:** this halves world textures (`texture_reduction=1`, `quality.cpp:65`,
  `texture.cpp:177-179`). The automap renders with reduction 3 (`frame.cpp:540`).
- **UI and overlays:** UI and overlay textures are native size, with no mips and clamped
  (`frame.cpp:589-596`).
- **For Bevy:** generate mipmaps for level TGAs and use a linear mag/min sampler with
  `mipmap_filter: Nearest` to match the retail default (or `Linear` to match "Mip mapping on"), with
  address mode Repeat.
