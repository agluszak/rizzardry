# Wizardry 8 monster animation: `.mls` + `.mon`

Evidence: whizzardry-8 C++ (paths are relative to `/home/agluszak/code/personal/decomp/whizzardry-8`)
and the retail data in `wizardry-prepared/data/monsters`. The parser is `monster_anim.py` in this
directory; its full output is in `monster_anim.out`.

Verified: `python3 -I monster_anim.py` decodes **1088 of 1089** `.mon` files and consumes every byte.
The one exception is `nessie_attack_speed.mon`, which is 0 bytes long. No `.mls` references it.

## 0. Units and axes

- `.mon` vertex positions come out in **retail world units**: the short value × `500/compression_scale`
  (`src/render/resources/mesh_loader.cpp:214`). Frame bounds, light positions and path points are
  stored as floats × 500 (`src/entities/animation.cpp:98,122`, `src/entities/keyframe_track.cpp:72`).
- `.mls` lengths are in authored metres × `kWorldScale = 500` (`include/wiz8/gameplay_constants.h:32`;
  `monster_loader.cpp:448-452`). `pathparms.txt` labels a radius in metres ("(m)").
  So retail itself uses 500 wu = 1 m, the scale Rizzardry adopted.
- The mesh is Y-up with the feet at y≈0. For example `savant_trooper_idle` has bbox y ∈ [-4, 901].
- Model forward is −Z. Evidence: the `savant_trooper_attack_thrust` bbox reaches z=-974 against +463.
  The runtime rotation is `Ry(yaw + π) · Rx(pitch) · Rz(roll)` (`src/entities/monster.cpp:1260-1283`,
  `g_monster_rotation_offset = π` at :144).
- The per-record placement (location plus angle/axis, `mesh_loader.cpp:299-307`) is **ignored for
  monsters**. `ShowFrame` copies it (`animation_mesh.cpp:69-75`), but `CyclePlayer::UpdateRepresentation`
  then overwrites location and rotation with the navigator pose (`cycle_player.cpp:684-690`).
  The data contains 405 distinct junk placements, so don't apply them.
- Scale is `(±s, s, s)`. s is the monster's scale (× `deathscale` during DIE), and x is negated for a
  left-handed roll (`monster.cpp:2376-2381`; `lefthanded` percent defaults to 45,
  `monster_loader.cpp:161,528`).

## 1. `.mls`: species definition (text)

The file is `data/Monsters/<cycle_name>.mls`. `cycle_name` is `monsters.dbs`+0x189, see
monsters_db.md. Parsing is in `LoadMonsterDefinition`, `src/entities/monster_loader.cpp:151-520`.
Lines are whitespace-tokenised, `#` starts a comment, and keywords are case-insensitive.

- **Cycle lines:** `<CYCLE>[n] <mon_basename> [fps_override]`.
  - CYCLE is prefix-matched (case-sensitive) against the 27 names in `g_cycle_names`
    (`monster.cpp:154-182`; enum in `include/wiz8/monster_cycles.h`). The legacy names `FLY`→BIRTH and
    `EXPLODE`→IDLE are also accepted (`monster_loader.cpp:601-637`).
  - Indices: 0 BIRTH, 1 IDLE, 2 SPICE (fidget), 3 TRANSITION (idle↔walk), 4 WALK, 5 DRAW,
    6 ATTACK_CLOSE, 7 ATTACK_RANGED, 9-16 SWING/THRUST/BASH/MELEE/THROW/PUNCH/KICK/LASH,
    17 SHOOT, 18 ATTACK_SPECIAL, 19 DODGE, 20 GET_HIT, 21 DIE, 22 TURN, 23 TALK_SPICE, 24 TALK,
    25 SPELL, 26 SPECIAL.
  - The file loaded is `data/monsters/<mon>.mon`, with textures from `data/monsters/Bitmaps/`
    (`cycle_player.cpp:258-260`). On disk that is `data/monsters/bitmaps`: 2174 .tga files and 91 .ifl files.
  - With the "additional animations" option off, SWING..LASH fold to ATTACK_CLOSE and THROW/SHOOT
    fold to ATTACK_RANGED (`monster_cycles.h:40-57`). Only the **first** file per cycle is kept;
    with the option on, later lines become sub-cycles, picked by `clock % count` (`monster.cpp:1452-1465`).
  - A third token > 0 overrides that cycle's fps (`monster_loader.cpp:343-360`). Example:
    `SPICE savant_trooper_spice 20`.
  - A cycle with no file falls back to a copy of IDLE (`monster.cpp:1440-1450`), and IDLE is mandatory.
  - DIE often points at a shared gib (`acid_vine`: `DIE gib_03`).
- **Scalars:** `movementrate` (default 3, below 0.1 becomes 2), `rotationrate` (0.7 turns/s),
  `scalefactor`, `scalerangestart/end` (rolled per actor), `walkradius` (collision radius),
  `fightradius` (alternate radius), `targetheight`, `cameraheight`, `deathscale`,
  `hoverrangestart/end`, `bobrangestart/end`, `missilestart`, `spellstart`,
  `randomidlefps a b` (default -3..3), `loddistance`, `opacity`, `glow`, `shadow w [d]`,
  `lefthanded`. Flags: `flies`, `swims`, `crawls`, `quadruped`, `spicemonster`, `fulltransition`.
  Sources: `monster_loader.cpp:214-307,439-517`.
- **Skins:** `SKIN`/`SKINSWAP old.tga new.tga` build damage stages, which are texture-table clones
  (`monster_loader.cpp:308-326`, `:92-145`).
- **Events:** `SOUND_FRAME|SOUND_CYCLE <CYCLE> <frame> <wave> [LOOP]`, `SOUND_FOOTSTEP <CYCLE> <frame>`,
  and `SHAKE_FRAME <CYCLE> <frame> <intensity> <duration> <dist>` (`monster_loader.cpp:371-427`).

## 2. `.mon`: one animation cycle (binary, little-endian, packed)

| Part | Layout | Source |
|---|---|---|
| File prefix | `u8 file_version` (must be 1); `u8 has_path` → **Path**; `u8 has_particles` → **Particles** | `cycle_player.cpp:205-288` |
| Animation header | `u8 ver` (data: 5/8/10/11), `u8 group_count` (≤3), `u8 playing`, `u8 frame_method` (1 wrap, 2 ping-pong, 3 random), `u8 behaviour` (1 once, 2 repeat, 3 never-stop), `u8 default_cycle`, `u8 positioned_parts`, [`f32 fps` if ver≥3, else 15], [`u8 start_frame` if ≥5], [`u8 end_frame` if ≥11], [`u8 random_play, f32 play_chance` if ≥6], `u8[50]` discarded, `u8[group_count]` channel bytes | `animation.cpp:26-90` |
| Frame bounds | ver>6: `u8 n`, then n × (vec3 min, vec3 max) ×500. All retail files have n=0 | `animation.cpp:91-102` |
| Lights | ver>7: `u8 n`, each `u8 lv, vec3 pos, vec3 rgb, f32 intensity, f32 range, [u8 kind if lv≥3]`. kind 1 adds 60 bytes, kind 2 adds 129 bytes | `animation.cpp:104-194` |
| Animation path | when not positioned and ver>8: `u8 has` → Path. ver>9: `u8` ignored | `animation.cpp:196-212` |
| Parts | for each group: [`i8 count` if positioned, else 1] × (`i8 lod_channel` 0..2, **AnimatedMesh**, [Path if positioned]) | `animation.cpp:214-239` |
| AnimatedMesh | `u8 frame_count, u8 first_index`, then a **MeshRecord**. If it holds more than 1 vertex frame, that is the whole animation. Otherwise frame_count−1 × (`u8 idx`, MeshRecord) | `animation_mesh.cpp:21-60` |
| MeshRecord | `i32 ver, i32 nverts, i32 nfaces`, [`u8 flags` if ver>2: 1 = framed vertices, 2 = i16 frames, 4 = compressed faces], [10 × f32: loc3, angle, axis3, scale3 if ver>1; ignored for monsters], [ver>3: `i8 nmap` × (`i16 key, i16 vertex`)]. If flags&1: `u8 ctype, i16 nframes, [f32 cscale if ctype==2, default 1000]`, then nframes × nverts × 3 i16, else nverts × 3 f32 ×500. Faces: flags&4 gives 0x21 bytes each (`u16 v[3], f32 uv[3][2], u16 mat, u8 flags`), otherwise 0x29 bytes (`i32 v[3], f32 uv[3][2], i32 mat, u8 flags`). Then **Materials** | `mesh_loader.cpp:279-397`, `include/wiz8/render/mesh_loader.h:14-37` |
| Materials | `i16 n`. The first record is 0x11A bytes; if its version is ≥4, add 16 bytes (`texture_modes`) and the rest are 0x12A each, otherwise all are 0x11A. Texture = the first non-empty of 4 × 40-byte `texture_names` at +0x29. Ambient at +0xC9, diffuse +0xD5, opacity +0xFD, emission +0x101, surface_flags +0x116 (bit 0 = two-sided). **Material 0 is always replaced by a white default** | `mesh_loader.cpp:232-275`, `include/wiz8/render/material_loader.h:28-49`, `material_loader.cpp:49-177` |
| Path | `u8 kind=0, u8 ver, f32, u32, i32 n`, n × (vec3 pos ×500, f32 angle, vec3 axis, [vec3 scale if ver==2]) | `keyframe_track.cpp:48-86` |
| Particles | `i32 n`, n × (`u8 ver`, then 0x216/0x218/0x21D/0x225 bytes for ver 1-4). Name at +0, embedded material at +0xEC, `i16 attachment_key` at +0x216 | `src/world/level_loader.cpp:644-680`, `include/wiz8/world/level_loader.h:26-67` |

Retail data in practice (from `monster_anim.out`):

- 1190 of 1196 meshes are flags=7: i16 vertex frames, ctype 1, scale 1000 (so world = short × 0.5),
  with compressed faces.
- 1192 parts store all frames in one record; only 4 use one mesh per frame.
- Every file has behaviour = never-stop and method = wrap.
- fps values: 30 (538 files), 24 (214), 20 (202), 10 (73), 15 (39).
- 1034 files have only LOD channel 2; 54 have LODs 0, 1 and 2.
- No retail file uses positioned_parts.
- 228 files carry particles, 72 carry lights.
- Vertex counts per mesh are 4-717 (median 202).

**Building a renderable mesh** (`BuildMeshPart`, `mesh_loader.cpp:97-227`):

- Split the faces by material shader. Unique render vertices are keyed by (material, two-sided,
  source vertex), and a UV seam adds a new UV index.
- A face with flags&4 or a two-sided material is emitted twice (`:142-155`, `:88-92`).
- Normals are **not stored**. Per frame, compute face normals and then summed vertex normals
  (`mesh_frames.cpp:423-449`).
- Faces whose texture name starts with "blank" are hidden (`mesh_frames.cpp:155-185`).
- Textures:
  - Retail tries the name with `.tga`, then with `.jpg` (`material_loader.cpp:236-261`).
  - `.ifl` is a CRLF list of frame textures (`:207-232`).
  - The slot-3 texture or a texture alpha channel selects blending (`:73-120`).
  - Case-insensitive lookup is needed.
  - All textures used by .mls-referenced cycles exist, apart from the unused `temp2_*` set.
- Reverse winding under your z-flip as you do for levels. I did not check UV v-orientation against
  a render.

## 3. Playback: time → frame

`CyclePlayer::TickAnimation`, `cycle_player.cpp:382-439`, driven by `Monster::TickAnimation`
(`monster.cpp:2443-2453`):

- `rate = fps × time_scale`.
  - fps is the cycle's `playback_scale`: the file f32, or the `.mls` override.
  - IDLE is re-rolled per actor as `max(fps + U(randomidlefps), 1)` (`monster_loader.cpp:536-545`).
  - `time_scale` is 1. For WALK it is the navigator `movement_speed`, the actual speed ÷
    (movementrate·scale·500), clamped to at least 0.5 (0.9 when flying or swimming)
    (`navigator.cpp:1770-1795`). Slowed monsters get ×0.5 (`monster.cpp:637-645`).
- Each tick: `elapsed_ms = min(now − timer, 1000)`, `progress = elapsed × rate / 1000`, and the
  integer part becomes whole frame steps. `timer += steps × 1000/rate`. The remainder becomes
  `frame_fraction`.
- For each step:
  - `advanceFrame` (`:533-588`): play-once clamps at the last frame and clears `animation_playing`.
    Wrap goes to first_frame and sets `wrapped`. Ping-pong reverses.
  - The frame-event callback, then SOUND_FRAME/FOOTSTEP and SHAKE_FRAME for the new frame.
- The cycle covers frames 0..N−1. A one-shot lasts (N−1)/fps; a loop period is N/fps.
  `start_frame` and `end_frame` from the file are not used for monsters: `SetCycle` sets first=0
  and last=N−1 (`monster.cpp:1514-1515`).
- **Interpolation:**
  - With "smooth monster animations" on (the default), `frame_interpolation = frame_fraction`
    (`monster.cpp:1326`).
  - Positions are linearly interpolated between frame f and f+1, and **never wrap** from N−1 to 0
    (`mesh_frames.cpp:371-386`).
  - Normals come from frame f only.
- **Behaviour comes from the caller, not the file.** `StartMonsterCycle(cycle, PLAY_ONCE)` is used
  for attacks, dodge, spell and special (`combat/attack.cpp:1354,1700`, `combat/actions.cpp:125,654`).
  Idle and walk are queued with NEVER_STOP (`monster.cpp:541-620`). Frame direction resets to
  forward unless KEEP_FRAME_DIRECTION is set. TRANSITION plays **reversed** when going walk→idle
  (`monster.cpp:550-575`).
- **Completion:** `Query(CYCLE_COMPLETE)` (`monster.cpp:1797-1813`).
  - NEVER_STOP: true once playback has stopped.
  - Otherwise: true at the last frame going forward, or frame 0 going in reverse.
  - The manager polls it each frame (`local_code/MonsterManager.cpp:1093-1150`). Examples:
    - DIE complete starts loot and the fade/removal.
    - ATTACK_SPECIAL complete sets `special_ready`.
    - In the default branch, a non-motionless monster with no pending cycle is sent back to its
      resting cycle (`MonsterManager.cpp:1138-1150`, partially read).
- **Milestones:**
  - Ranged cycles (7 ATTACK_RANGED, 13 THROW, 17 SHOOT) release the missile when a step crosses
    `missilestart`, i.e. `prev < f ≤ cur`; when `missilestart` is unset (0) that point is frame 1
    (`monster.cpp:1821-1833`).
  - SPELL (25) does the same with `spellstart` (`:1840-1856`).
  - The launch point is mapped vertex key 5 (missile) or 6 (spell) of the cycle mesh
    (`monster.cpp:666-700`). `mapped_vertices` key→vertex comes from the record (`mesh_loader.cpp:309-326`).
  - Particle `attachment_key`s look up the same table (`cycle_player.cpp:772-804`).
  - **Melee has no hit-frame marker.** `dispatchAnimationFrame`/`HandleAnimationFrame`
    (`monster.cpp:1821-1856`) are the only gameplay frame hooks, and they cover only cycles
    7/13/17/25. Melee resolution is not tied to any frame: `attack.cpp:1353-1354` starts the cycle.
    Only sounds and shakes are frame-keyed. I did not fully trace when melee damage is displayed.

## 4. LOD and bounds

- LOD 2 is the near model. The thresholds are 3500/8500 wu × the detail slider, and the code falls
  back to whichever LOD exists (`cycle_player.cpp:815-867`).
- Navigator bounds are the idle frame-0 bounds × scale (`monster_loader.cpp:493-500`).
- `AnimationMesh.bounds` is the union over all frames, and `radius` is the maximum half-diagonal
  (`animation_mesh.cpp:44-52`).

## 5. Example: `savant_trooper` (recommended first actor)

- `.mls` values: scalefactor 1.25, movementrate 1.969, walkradius 0.675, targetheight 1.65.
- Mesh: 199 vertices, 386 faces, one mesh, LOD 2 only, 12 `rider*.tga` textures.
- Frames, fps and duration per cycle:

| Cycle | Frames | fps | Duration |
|---|---|---|---|
| IDLE | 30 | 20 | 1.50 s |
| SPICE | 30 | 20 | 1.50 s |
| TRANSITION | 6 | 30 | 0.20 s |
| WALK | 32 | 30 | 1.067 s |
| ATTACK_THRUST | 30 | 30 | 1.00 s |
| ATTACK_MELEE | 38 | 30 | 1.267 s |
| DODGE | 30 | 30 | 1.00 s |
| GET_HIT (pain) | 30 | 30 | 1.00 s |
| DIE (death) | 32 | 24 | 1.333 s |

- The DIE mesh has 261 vertices and 489 faces, plus 3 mapped vertices, particles and 1 light.
- Idle bbox: (-364,-4,-469)..(216,901,201) wu, before the 1.25 scale.
- Run `python3 -I monster_anim.py <dir> <species>` for any other species. `wasp` is a flying example.
