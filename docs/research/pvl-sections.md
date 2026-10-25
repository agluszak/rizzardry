# Wizardry 8 `.pvl`: sections after the compiled meshes

Verified against all 34 retail levels in `wizardry-prepared/levels/*/*.pvl` with
`pvl_sections.py` (same directory). Every level parses record by record, every
`-1` sentinel matches, and the parse stops **72 bytes before EOF**. Those 72
bytes are zero in all 34 files and nothing reads them (`ReadLevel` returns after
the automap sentinel). Treat them as an unread zero trailer.

C++ citations are to `whizzardry-8` (W8 = `/home/agluszak/code/personal/decomp/whizzardry-8`):
`LL` = `src/world/level_loader.cpp`, `TIO` = `src/entities/trigger_io.cpp`,
`TA` = `src/entities/trigger_actions.cpp`, `PR` = `src/entities/prop.cpp`,
`AN` = `src/entities/animation.cpp`, `AM` = `src/entities/animation_mesh.cpp`,
`ML` = `src/render/resources/mesh_loader.cpp`, `KT` = `src/entities/keyframe_track.cpp`,
`LS` = `src/world/level_session.cpp`.

## 0. Conventions

* Little-endian, packed (no alignment padding anywhere).
* `vec3` = 3 × f32. `cstr[N]` = fixed N-byte field holding a NUL-terminated
  string. The C++ rejects a field with no NUL when it says "checked" below.
* **World scale `S = 500.0`** (`kWorldScale`, `include/wiz8/gameplay_constants.h:32`).
  Every authored position, range and radius on disk is in "max units"; the loader
  multiplies by 500. The scale is listed per field. Rotations are `(angle: f32 radians,
  axis: vec3)` and are turned into `angleAxis(atan2(sin a, cos a), normalize(axis))`,
  with angle 0 giving identity.
* Y is up: ground probes trace along −Y (`src/spatial/level_spatial_index.cpp:866`).
* **Sentinel**: `i32 == -1`. Retail always has an `.oct`, so the sentinel after each
  section is present (`CHECK_PVL_OFFSET`, LL:873).
* Counts read through `ReadRecordCount` (LL:415) must be in `[0, 100000)`.

## 1. Section order (`ReadLevel`, LL:881-1046)

| # | Section | Reader | Sentinel after |
|---|---------|--------|----------------|
| – | mesh_count, root_count, materials, −1, compiled meshes, −1 | `ReadMultipleLevelMeshes` (ML:485) | (the two −1s are part of it) |
| 1 | Lights | `ReadWorldLights` LL:160 | yes (LL:936) |
| 2 | Monster paths | `ReadMonsterPaths` LL:549 | yes (LL:938) |
| 3 | Items | `ReadWorldItems` LL:478 | yes (LL:940) |
| 4 | Missiles: one `i32` count, no records | LL:942 | yes (LL:945) |
| 5 | Props | `ReadWorldProps(false)` LL:422 | yes (LL:947) |
| 6 | "Bitmaps" (props again, made unpickable) | `ReadWorldProps(true)` | yes (LL:949) |
| 7 | Camera paths | `ReadWorldCameras` LL:600 | yes (LL:951) |
| 8 | Environment: `i32 present`, then the body if it is non-zero | LL:957, `ReadWorldEnvironment` LL:269 | yes (LL:964) |
| 9 | Triggers: `i32 count` + records | LL:967, `Trigger::CreateAndLoadLevelTrigger` TIO:506 | yes (LL:980) |
| 10 | `i32 camera_sway` (≠0 starts camera sway) | LL:983 | **no** |
| 11 | Clip planes | `ReadWorldClipPlanes` LL:360 | yes (LL:993) |
| 12 | `vec3 environment_offset` (not scaled; added to unsunlit prop emissive) | LL:997, 1032 | **no** |
| 13 | Particle systems | `ReadWorldParticles` LL:644 | yes (LL:1003) |
| 14 | Named positions | `ReadNamedPositions` LL:836 | yes (LL:1005) |
| 15 | Automap cells | `Automap::readCells` src/world/automap.cpp:51 | yes (LL:1007) |
| – | 72 zero bytes, unread | – | – |

## 2. Shared sub-records

### 2.1 Keyframe path, `LoadPathAI` (KT:48-86)

| off | type | field |
|---|---|---|
| 0 | u8 | kind, must be 0 |
| 1 | u8 | version (2 = points carry a scale) |
| 2 | f32 | `position` (playback cursor). Overwritten with 0 after load (KT:84). 0.0 in retail |
| 6 | f32 | `unknown_08`. In retail it is always `1/point_count`. Stored but has no clear use |
| 10 | i32 | point_count (≥0; **0 → the path is "absent"**, `nullopt`) |
| 14 | point[n] | per point: `vec3 pos` (×S), `f32 angle`, `vec3 axis`, plus `vec3 scale` (not scaled) when version==2. 28 or 40 bytes |

Retail: all prop part paths, animation paths and camera paths are version 2 or 1
(camera paths are 1). `point.rotation = angleAxis(angle, axis)`.

### 2.2 Parametric light animation (15 × 4 bytes = 60)

`u32 flags; f32 flicker_chance; vec3 color; vec3 color_to; f32 intensity;
f32 intensity_to; f32 period; f32 rate; f32 path_speed; i32 subcycle_min;
i32 subcycle_max` (LL:187-201, AN:131-143).
Flags (`include/wiz8/render/light.h:31`): mode = flags&3 (0 oscillate, 1 flicker,
3 ramp), 8 = colour, 0x10 = HAS_PATH (a `LoadPathAI` path follows), 0x20 = ping-pong path.

### 2.3 Material record (0x11a, or 0x12a with texture modes)

This is the same record the level geometry uses (`include/wiz8/render/material_loader.h:28`).
The texture names are `cstr[0x28]` × 4 at +0x29. A **material table** is `i16 count`;
the record bytes follow it. If `records[0].version >= 4`, every record is followed by
`f32[4] texture_modes`, so records are 0x12a bytes from then on (ML:232-261). The C++
checks the names for a NUL.

### 2.4 Level mesh record, `ReadSingleLevelMeshRecord` (ML:279-397)

| type | field | condition |
|---|---|---|
| i32 | version | |
| i32 | vertex_count (1..100000) | |
| i32 | face_count (1..200000) | |
| u8 | flags: 1 = LOD/vertex-frame vertices, 2 = short frames, 4 = compressed faces (`include/wiz8/render/mesh_loader.h:15`) | version > 2 |
| f32[10] | location vec3 (×S), angle, axis vec3, scale vec3 (read and then **discarded**) | version > 1 |
| i8 + n×(i16 key, i16 vertex) | vertex markers (key first, then the source vertex index) | version > 3 |
| vertices | if !(flags&1): vertex_count × vec3 (×S) | |
|  | else: `u8 compression_type`, `i16 frame_count`, `f32 compression_scale` (only if type==2, default 1000), then frame_count × vertex_count × (flags&2 ? 3×i16 : 3×f32) | |
| faces | face_count × (flags&4 ? 0x21-byte compressed face: u16 v[3], f32 uv[3][2], u16 material, u8 flags : 0x29-byte face: i32 v[3], f32 uv[3][2], i32 material, u8 flags) | |
| material table | §2.3 (count ≥ 1) | |

The runtime short-vertex scale is `500 / compression_scale` (ML:214). Face flag 4 means
two-sided, and the face is split (ML:91). Placement (ML:442): `T = location×S`,
`R = angleAxis(angle, axis) × Rx(π)`. **Props override this placement** (see §6).
Retail prop meshes are all version 4, with flags 0 or 7.

### 2.5 Animated mesh, `ReadAnimatedMesh` (AM:21-60)

`u8 count (≥1)`, `u8 first_index (<count)`, mesh record. If the first mesh has
more than one vertex frame (flags&1 and frame_count>1), that single record holds
the whole animation. Otherwise `count−1` more entries follow, each `u8 frame_index`
(unique and less than count) plus a mesh record.

### 2.6 Animation object, `AnimObjReadFromFile` (AN:26-257)

| type | field | cond |
|---|---|---|
| u8 | version | |
| u8 | group_count (≤3) | |
| u8 | animation_playing | |
| u8 | frame_method (0 none, 1 wrap, 2 ping-pong, 3 random) | |
| u8 | behaviour (0 none, 1 play once, 2 repeat, 3 never stop) | |
| u8 | cycle | |
| u8 | positioned_parts (bool) | |
| f32 | playback_scale (default 15) | v ≥ 3 |
| u8 | start_frame | v ≥ 5 |
| u8 | end_frame | v ≥ 11 |
| u8, f32 | random_play, play_chance | v ≥ 6 |
| 50 bytes | **discarded** | |
| u8 × group_count | channel bytes (read, unused) | |
| u8 n + n × (vec3 min, vec3 max) ×S | per-frame bounds | v > 6 |
| u8 light_count + lights | prop loads pass `light_list=NULL`, so **any light makes the prop fail to load**. Retail props have 0 | v > 7 |
| u8 has_path + path §2.1 | whole-animation path | !positioned_parts && v > 8 |
| u8 | ignored | v > 9 |
| groups | per group: if positioned_parts then `i8 count`, else 1. Per entry: `i8 channel (0..2)`, animated mesh §2.5, and if positioned_parts a path §2.1 that must not be empty | |

Animation lights (only for non-prop callers): `u8 ver, vec3 pos ×S, vec3 colour,
f32 intensity, f32 range ×S`, then `u8 kind` if ver ≥ 3 (otherwise kind = (ver==2)).
kind 1 is followed by §2.2. kind 2 is followed by `u8 ignored, i32 start, i32 end,
6×(i32 frame, f32 value, vec3)`.

Retail: animation versions are 7 and 9; group_count is 1; channel is always 2.
2226 animations are positioned_parts with 1-10 parts. 554 are non-positioned, and 5 of
those have a whole-animation path.

## 3. Sections 1-4

### 3.1 Lights (LL:114-267)

`i16 count`, then per light a 40-byte header:

| off | type | field |
|---|---|---|
| 0 | i16 | version |
| 2 | u8 | create |
| 3 | u8 | visible |
| 4 | u32 | flags (bit 1 = parametric definition follows; bit 0 unknown, seen as 3) |
| 8 | vec3 | location (×S) |
| 20 | vec3 | colour |
| 32 | f32 | intensity |
| 36 | f32 | range (×S) |

If version ≥ 2: `cstr[20] name` (not checked; uppercased). If also flags&2:
§2.2, plus a path if def.flags&0x10. A name starting with "Sun" makes the light
ambient-only. Lights with create=0 and visible=1 are baked static point lights.
Retail: all 3039 lights are version 2; 89 have flags&2.

### 3.2 Monster paths (LL:549): `i32 count`, then per path `cstr[20] "id[:opts]"` + path §2.1. **0 in every retail level.**

### 3.3 Items (LL:478-547): `i32 count`, then per item:
`cstr[20] name` (a number or an item name), `vec3 pos ×S`, `4×i32` (discarded),
`u8 has_trigger` [+ trigger §5], `3×u8` (discarded), `4×i32` (discarded).
**0 in every retail level.**

### 3.4 Missiles: one `i32` count. No records are read. 0 in retail.

## 4. Props and bitmaps (sections 5-6), `Prop::LoadProp` (PR:1047-1295)

Each section is `i32 count` + prop records. The bitmap section uses the same layout;
its props are made unpickable (LL:468). Retail: all 2780 props (2510 + 270) are version 9.

| type | field | cond |
|---|---|---|
| i8 | version | |
| *v < 4*: u8 playing, u8 frame_method, u8 behaviour, [f32 playback_scale if v ≥ 2], animated mesh §2.5 | | v < 4 |
| u8 | frame_count (used when positioned_parts) | v ≥ 4 |
| u8 | face_camera (billboard about Y) | v > 4 |
| vec3 | location (×S) → `Prop::location` | v > 4 |
| u32 | prop flags: 0x01 COLLIDABLE, 0x02 ACCUMULATE_PATH_FRAMES (`include/wiz8/entities/prop.h:41`). Retail values: 0, 1, 2 | v > 5 |
| cstr[64] | name (checked) | v > 6 |
| i8 n + n×(u16 frame, u16 tag) | animation segments/slots (frame < frame_count). The C++ keeps (i8)frame and (u8)tag | v > 7 |
| animation | §2.6 | v ≥ 4 |
| u8 has_trigger [+ trigger §5] | attached trigger (m_bRepType = PROP) | v > 2 |
| u8 has_footstep [+ u8 surface, u8 material] | | v ≥ 9 |

The engine adds flag 0x40 NO_DIRECT_SUN from the .oct sun bits, and 0x20/0x80 at
runtime. Neither is on disk.

## 5. Trigger record, `CreateAndLoadLevelTrigger` (TIO:506-1012)

A trigger record appears in three places: the trigger section (§1 #9), after a prop
(§4), and after an item (§3.3). Every record starts with `u8 record_version` (1 in
retail; read and ignored), `u8 type` (1..4; anything else throws).
Names and recipient lists are uppercased. Recipients are split on `,` (TA:85). If the
action (initial, alternate or fallback) is 9, 0x0e, 0x34 or 0x3f, the whole recipient
text is also kept as `recipient_name` (TIO:1003).

### Type 1, legacy item trigger (TIO:528-616). **Not in retail.**
`u8 ver; i32 cycle_bounce; i32 state_count; f32 animate_states; i32 range (×S);
i32 action; i32 (discarded); i32 animate_action; u8 packed_flags (1 fire linked,
2 link on deactivate); u8 enabled; cstr[0x80] name; cstr[0x100] recipients;
cstr[0x80] sound`, then:
- ver>1: `f32 min_range (×S); 64 bytes surface_id` (if byte 0 is NUL, `atoi(bytes[1..5])` is the
  collision surface id, else −1)
- ver>2: `u8 has_data` [+ `u8 kind`, kind 1 → door §5.5]
- ver>3: `i32 action_value`.
Kind = item.

### Type 2, legacy positional trigger (TIO:618-714). **Not in retail.**
`u8 ver; f32 range (×S); vec3 pos (×S); i32 action; i32 searchable; u8 fire_linked;
u8 enabled; cstr[0x80] name; cstr[0x100] recipients`, then:
- ver>1: `u8 plane; vec3[4] quad (×S)`
- ver>2: `f32 angle; vec3 direction; u8 (discarded); cstr[0x80] action_string`
  (kept only when action==17)
- ver>3: `u8 keep_on_finish; i32 action_value`
- ver>4: `u8 has_geom` [+ `u8 kind`, kind 2 → `u8 n, 36×vec3, 2×u8`, all discarded].
Kind = prop, POSITIONED.

### Type 3, ambient sound (TIO:716-781). No Trigger object is created.
`u8 ver; i32 vol_min, vol_max, speed_min, speed_max, time_min, time_max; i32 unbounded;
f32 radius (×S); vec3 pos (×S); vec3 region_u (×S); vec3 region_v (×S); cstr[0x80] wave`, then:
- ver>1: `u8 has_position (discarded), u8 looping`
- ver>2: `vec3 region_center (×S); f32 region_angle; vec3 region_min; vec3 region_max` (min/max not scaled)
- ver>3: `cstr[0x80] name`
- ver>4: `u8 shared`.
Retail: 2339 records, all ver 5.

### Type 4, current trigger (TIO:783-999). 2472 retail records, all ver 3.

| type | field | notes |
|---|---|---|
| u8 | version | |
| cstr[0x80] | name | uppercased |
| u8 | packed_flags | 1 = "item" kind and **no representation block**; 2 CAN_RUN_LINKED (shows a level message m_lData1/2/3 from `Data\Messages\<folder>.msg`); 4 ONCE; 8 EXCLUSIVE; 0x10 REACTIVATE_LINKED; 0x20 → flag 0x400000 |
| u8 ×6 | enabled, fire_linked, link_on_deactivate, keep_on_finish, flag(0x1000), alternate_toggles | |
| i32 ×3 | initial_action, alternate_action, fallback_action | stored as u16 |
| cstr[0x100] | recipients | |
| u8 | searchable | 1 → item kind as well |
| cstr[0x100] | required item (`atoi`, −1 if empty) | |
| u8 | consume_item | |
| i32 | action_value | |
| u8 | animate_action | |
| cstr[0x80] | sound → `action_data = "data\sound\" + sound` | |
| *ver > 1:* i32 m_lData1, i32 m_lData2, i32 m_lData3, f32 representation_scale (discarded), u8 initial_location_value, u8 alternate_flag (→ ALTERNATE_ACTION, applied only if ver > 2), i8 action_data_mode, i8 sound_volume, 4×i32 (discarded, 0 in retail) | | |
| u8 | state_count | |
| u8 | animate_states | |
| u8 | cycle_bounce | |
| cstr[0x100] | required_states (comma list of location variables) | |
| cstr[0x100] | state_to_modify (location var; created with initial_location_value if that is 0/1) | |
| u8 | state_mod_mode (0 none, 1 set, 2 clear, 3 toggle) | |
| f32 | range_min (×S) | |
| f32 | range_max (×S) | |
| cstr[0x100] | inline_action_data (uppercased) | |
| i32 | discarded (always 0x3f800000 = 1.0f in retail) | |
| cstr[0x80] | alternate sound → `alternate_action_data` | ver > 2 |
| **representation**, only if `(packed_flags & 1) == 0`: `u8 kind`; kind 1 → `vec3 position (×S), f32 angle, vec3 direction (axis)` + flag POSITIONED; kind 2 → `vec3[4] quad (×S)` (plane trigger); then `u8 has_legacy` [+ `u8, f32, cstr[0x80]`, discarded] | | |
| `u8 has_action_data` [+ `u8 kind`; kind 1 → door §5.5; kind 2 → `u8 n, 36×vec3, 2×u8`, discarded] | | |

Post-load fix-ups (TIO:974-997):
- action 0x34 with empty recipients → range_min = range_max = 0 (arrival-only marker).
- action 0x0c (missile) checks m_lData1 and m_lData2. If m_lData2 < 0 it queues a timed event.
- actions 0x25..0x2b set uses_remaining = m_lData1.

Plane triggers (kind 2, or type 2 with plane=1) are registered with
`AddTriggerPlane` (LL:1306). With an .oct, the n-th plane trigger **in load order**
(prop-attached triggers first, because props load before the trigger section) binds
to slot n of the .oct trigger-surface table. The quad in the .pvl is not used then.

Retail observations: representation kind 0 = 1854, 1 = 471, 2 = 147; packed_flags
values 0/1/2/3/4/5/6/7; action_data_kind 1 (door) = 326; no legacy blocks.

### 5.5 Door action data, `ReadDoorTriggerActionData` (TIO:472-504)

`u8 version (1); 9 × u8 flags (low bit): open, lockable, locked, auto_locking, one_way,
secret, found, jammable, jammed; u16 key item (0xffff = none); u8 has_position;
vec3 position (×S); cstr[0x80] linked_trigger` = 0x9a bytes.
Loading also sets `trigger.state_index = open`. Retail: has_position is always 0, and
no linked_trigger is set.

## 6. Prop world transform

All positions are already ×S. Transform = `T · R · diag(scale)`
(`include/wiz8/render/transform.h:53`).

* **positioned_parts** (2226 retail; all bitmaps). Each part's mesh vertices are in
  part-local space (the mesh record location is 0). The part's transform is the
  keyframe path point at the current subcycle: `T = point.pos`,
  `R = angleAxis(point.angle, point.axis)`, `scale = point.scale` (v2 paths)
  (`sampleKeyframeTrack` KT:318, `ApplyAnimationFrame` PR:704, `AttachAnimationInstances`
  PR:784-834). The mesh's own placement and its Rx(π) are replaced. At load,
  subcycle = 0, so **path point 0 is the initial (closed/rest) pose**. Retail:
  `prop.location == part path point[0].pos`. Animating moves through the points
  (lerp/slerp between `subcycle` and the next frame). `Prop::GetPosition()` is the last
  part's animated position.
* **non-positioned** (554; all have mesh flags 7, i.e. short vertex-frame meshes).
  If the animation has a whole-animation path: `T/R/scale` come from that path at the
  current subcycle. Otherwise `T = prop.location`, `R = prop.rotation` (**identity**; nothing
  sets it), scale 1 (PR:851-862). The mesh record's angle/axis is non-zero but
  ignored. The data agrees: vertex-frame vertices are already Y-up (for example, the
  "mookbanner" mesh, whose record has −X 90°, is already vertical).
* Collision: props with COLLIDABLE (0x01) go into `collidable_props` and get
  bounds in the octree (LL:456-465). The navigation representation is rebuilt from the
  animated pose (`BuildOrRefreshPathingRepresentation`), so moving platforms carry
  their collision with them.

## 7. Sections 7-15

### 7.1 Camera paths (LL:600-642)
`i32 count`, then per path: `i32, i32` (discarded), `u8 has_speed`,
`cstr[0x14] name` (checked), `f32 speed` (only if has_speed, default 15), path §2.1
(must not be empty). Played in timed mode. Action 9 activates a path by name. While it
plays it **drives the party camera position and rotation** each frame. When it
ends, the party is wherever the last point is (src/world/world.cpp:230-247).

### 7.2 Environment (LL:269-358)
`i32 present` (LL:957). If present ≠ 0 (all 34 retail levels):
`u8 fog_enabled; vec3 range (x = range_start, y = range_end, z unused);
f32 intensity (ambient = clamp(intensity)); f32 view_distance (×S → far clip);
u8 camera_mode`.
- camera_mode 1: `vec3 camera pos (×S)`
- camera_mode 2: `vec3 camera pos (×S); f32 angle; vec3 axis` → initial camera rotation
  (in 2 retail levels).

Then `u8 has_light_colours` [+ 256×RGB8 = 768 bytes], `u8 has_env_colours`
[+ 768 bytes] (`ReadEnvironmentColours`, src/world/environment.cpp:219). If present == 0,
far clip is 42500 and render range 37500.

### 7.3 Clip planes (LL:360-412)
`i32 count`. If count > 0: `u8 version` (**one byte for the whole section**), then per
plane `cstr[64] name` (not checked), `f32 x, y, z, w` (xyz ×S = location; w discarded).
The plane equation is fixed at (0,1,0,0). **0 in retail.**

### 7.4 Particle systems (LL:644-834, struct `include/wiz8/world/level_loader.h:26-67`)
`i32 count`, then per system `u8 version` plus a body of 0x216 (v1), 0x218 (v2), 0x21d (v3)
or 0x225 (v4) bytes:

| off | type | field |
|---|---|---|
| 0x000 | cstr[64] | name (checked; "CLOUD*" = camera-relative) |
| 0x040 | vec3 | location ×S |
| 0x04c | f32 | rotation_angle |
| 0x050 | vec3 | rotation_axis |
| 0x05c | 12 bytes | unused |
| 0x068 | u32 | particle_count |
| 0x06c | vec3 | spread (x,y ×250, z ×S) |
| 0x078 | i32 | has_acceleration |
| 0x07c | vec3 | acceleration ×S |
| 0x088 | i32 | expiry_mode |
| 0x08c | i32 | bounds_mode (0 none, 1 box, 2 sphere) |
| 0x090 | vec3 | bounds_origin ×S (a non-zero x sets replace_when_full) |
| 0x09c | f32 | bounds_radius ×S |
| 0x0a0 | vec3 | bounds_extent ×250 |
| 0x0ac | u32 | lifetime ms |
| 0x0b0 | i32 | emission_mode |
| 0x0b4 | u32 | emission_interval |
| 0x0b8 | i32 | los_check |
| 0x0bc | i32 | speed_mode |
| 0x0c0 | f32 ×3 | initial_speed, speed_min, speed_max (×S) |
| 0x0cc | f32 | particle_size |
| 0x0d0 | i32, f32, f32 | flutter_mode, flutter_value, flutter_period |
| 0x0dc | i32, f32, f32 | direction_mode, direction0/1 (degrees) |
| 0x0e8 | i32 | initially_active |
| 0x0ec | 0x12a | material record (always with texture modes) |
| 0x216 | i16 | attachment_key (v ≥ 2) |
| 0x218 | i32, u8 | emission_limit, requires_sorted_renderer (v ≥ 3) |
| 0x21d | i32, i32 | start_frame, end_frame (v ≥ 4) |

Retail: 397 systems, all v4.

### 7.5 Named positions (LL:836-871)
`i32 count`, then per position `u8 version (must be 1)`, `cstr[0x80] name` (checked),
`vec3 position (×S)`, `f32 angle`, `vec3 direction (rotation axis)` = 157 bytes.
`FindEntityByName` (src/world/world.cpp:331) resolves "PARTY" or these names. They are
NPC/monster spawn points ("NP_*"), teleport destinations, and so on.

### 7.6 Automap (src/world/automap.cpp:51-94)
`f32 cell_size`, `u32 count`, then `count × u32 key`. Cell centre =
`(key>>21, key&0x3ff, (key>>10)&0x7ff) × cell_size + cell_size/2 + grid_min`, where
grid_min is the .oct clipped bounds minimum. Cell index is 1-based through the key map.

## 8. Retail counts (all 34 levels)

| total | value |
|---|---|
| lights | 3039 |
| monster paths / items / missiles / clip planes | 0 / 0 / 0 / 0 |
| props / bitmaps | 2510 / 270 |
| camera paths | 64 |
| triggers in the trigger section | type 3: 2339, type 4: 580 |
| triggers attached to props | 1892 (type 4) |
| all type-4 triggers | 2472 (doors: 326) |
| particle systems | 397 |
| named positions | 187 |
| automap cells | 114051 (per level 318 … 18228) |

The per-level table is printed by `python3 -I pvl_sections.py <levels dir>`.

## 9. Trigger semantics for navigation and level structure

### 9.1 Trigger kinds and activation
* `trigger_kind = item` if `packed_flags&1 || searchable==1`. These are **use/click**
  triggers, normally attached to a prop and activated by clicking the prop (`Prop::activate`
  PR:1380). `trigger_kind = prop` otherwise. Those are **world triggers**:
  * representation kind 1 (POSITIONED): proximity sphere. It runs when the camera is
    within `[range_min, range_max)` of `position`, the trigger is ENABLED and not RUNNING.
    When the camera leaves, `FinishAction` runs (`UpdateWorldTriggers` TA:214-262).
  * representation kind 2: plane/quad. It runs when the party crosses the .oct trigger
    surface (src/spatial/collision.cpp:640-660, 759-770).
* Action selection (`SelectAction` TA:1898). It uses initial_action. If `alternate_action≠0`
  (HAS_ALTERNATE), the next run uses alternate; alternate_toggles flips back. It falls back
  to fallback_action when required_states are unmet or the required item/key is not held.
  Level scripts can veto a run through `activation_callback` (`LevelScript::bindTrigger`,
  src/wiz8/level_specific_code/MasterFunctionList.cpp:92).
* After a run (`CommitActionResult` TA:783): if FIRE_LINKED, every recipient trigger is
  `Run(m_lData1)`. state_to_modify is then applied, and with CAN_RUN_LINKED a level
  message is shown.

### 9.2 Action codes (`Trigger::Run` TA:1001-1878)

| code | meaning | data |
|---|---|---|
| 0x00 | nothing (commit only: linked triggers and state vars still fire) | |
| 0x01 | **door open** (item kind, prop rep). Key check is door.item. Plays the prop animation, sets state_index 1 and the collision surface interface state (`surface_id`, type-1 only). Auto-closes after m_lData1 s (−1 → 10 s) if m_lData1 ≠ 0 | door data §5.5 |
| 0x02 | **door toggle** (open↔close). Usually the alternate_action of a 0x01 door | |
| 0x2c | door/prop toggle with key check | |
| 0x03 | toggle prop animation (start if stopped, stop if playing). Used for **lifts/elevators, levers, gates and chests**. With non-empty inline_action_data it is a container (item group) | prop |
| 0x32 / 0x33 | start / stop prop animation | prop |
| 0x08 | step prop state (state_count, cycle_bounce). Animates paths by one node | prop |
| 0x37 | select animation slot by tag | m_lData1 |
| 0x40 | advance animation segment, set `<state_to_modify><n>` vars | prop slots |
| 0x48 / 0x49 / 0x4a | set subcycle / speed of own prop / recipient props | m_lData1 |
| 0x3a–0x3e | activation state of the prop | m_lData1 = source tag |
| 0x09 | **play camera path** named by recipients. Used for **ladders/ropes/climbing**: the camera path carries the party to its end point | recipients = camera name |
| 0x0a | Monastery switch-puzzle camera selection ("Camera0N") | location vars Switch0..6 |
| 0x0f | run every recipient trigger with source m_lData1 (relay) | recipients |
| 0x2d / 0x2e / 0x2f | recipient triggers ON / OFF / toggle | recipients |
| **0x34** | **destination: teleport or level transition**, `RunDestination(recipients)` (TA:856-910) | recipients, m_lData1 |
| **0x4b** | random destination among recipients (`clock % n`) | recipients |
| 0x04 / 0x30 / 0x31 | light toggle / on / off | recipients = lights |
| 0x41 / 0x42 / 0x43 | ambient sound position / stop / toggle | recipients |
| 0x44 / 0x45 / 0x46 / 0x47 | particles on / off / toggle / burst for m_lData1 ms | recipients |
| 0x0b | wake monster group `atoi(recipients)` | |
| 0x0c | fire missile m_lData1 (m_lData2: 0 once, <0 periodic) | position, angle/direction |
| 0x0e | reaction condition on a random character (recipients "party") | |
| 0x10 | camera sway start/stop (water planes) | |
| 0x11 | set RUNNING | |
| 0x22 | environment intensity → 0 (saved and restored on finish) | |
| 0x23 | pain plane: 1d6+2 every 0.5 s while inside | |
| 0x24 | damage party m_lData1 d m_lData2 + m_lData3 | |
| 0x25–0x2b | fountains: stamina, heal, spell points, hex/sleep/noxious/heal-all spells. m_lData1 = uses, m_lData2 = recharge in hours×720 s, m_lData3 = amount | |
| 0x36 | play the trigger's sound (action_data or alternate_action_data) | |
| 0x39 | camera shake (m_lData1 intensity, m_lData2 period ms, m_lData3 duration ms) | |
| 0x3f | after m_lData1 ms, run trigger `recipient_name` | |
| 0x1a / 0x1f / 0x21 | not handled in `Run`; they only fire linked triggers and level-script callbacks (rift pool, poison pool/gas trap, fire trap) | |

### 9.3 Level transitions and teleports (0x34 / 0x4b)
`RunDestination(dest)`:
1. If `dest` is "PARTY" or a **named position** in the current level (§7.5), the party is
   placed at it in this level, with rotation `angleAxis(angle, direction)`.
2. Otherwise `dest` must be ≥5 chars: `dest[0..3]` = **location code**
   (`g_level_folders`, LS:86-134, e.g. ARN = Arnika2, MO1 = Monastery1, CT1 = Arnika_Trynton,
   DUN = Dungeon). `atoi(dest[3..5])` is the **entrance number**.
   * In a different level: `RequestLevelTransition(level, entrance, m_lData1<0 ? 0 : m_lData1)`.
     m_lData1 ≠ 0 shows an "Enter <level>?" confirmation (e.g. CMP01, ARN02 have d1 = 1).
   * In the same level: the party is placed at the trigger named `dest`
     (`position`, **no ground probe**), and the camera turns to `angleAxis(angle, direction)`.
3. Scripts call RunDestination directly, e.g. the Marten's Bluff teleporter MR109 →
   CT611/CT107/RIF01/MR207 (src/wiz8/level_specific_code/MartensBluff1.cpp:406), and Arnika
   → ARN11.

Retail data: 318 type-4 triggers have initial action 0x34 (one more 0x34 is an
alternate). 148 have empty recipients (arrival-only markers with range forced to 0).
TY100 and TY106 have recipients `"  "` (two spaces): their range is not zeroed, but
RunDestination ignores names shorter than 5. 125 name another level's "CCCnn"
entrance, and all of those resolve to an existing trigger in that level's .pvl.
43 name a trigger in the same level (DUN, RAM, ASC, RIF teleports). None name a named
position or PARTY. Example: `ARN02 → "CT104"`.

### 9.4 Level entrances and the arrival position
* An entrance is a **trigger named `sprintf("%3s%02d", location_code, entrance)`**, for
  example "MO100" = Monastery1 entrance 0 (the new-game start, `ChooseNewGameStartLocation`,
  `src/wiz8/local_code/NPC Manager.cpp:981`). Lookup is case-insensitive and the
  newest trigger wins (TA:105).
* In retail every one is a **type-4 trigger, packed_flags 0, representation_kind 1
  (POSITIONED), initial_action 0x34**. It is either an arrival-only marker (no
  recipients) or a two-way portal whose recipients lead elsewhere. It is found in the
  trigger section, not attached to a prop.
* Arrival (`EnterLevel`, LS:348-401). If the trigger is POSITIONED:
  `p = trigger.position`; `p.y = probeGround(p).y + 1000` (`g_default_world_height`, LS:280),
  where `probeGround` = `settleToGround(p, test_props=true, limit=500)`. That traces down
  from `p.y+500` to the first walkable level surface or prop top
  (src/spatial/level_spatial_index.cpp:866). If `|p.y − trigger.y| > 2500`
  (`g_position_height_epsilon`, LS:282), it keeps the trigger's y.
  `party.place(p)` sets the **eye/camera position**. If trigger_kind == prop (always for
  entrances), the camera rotation is `angleAxis(angle, direction)` (angle 0 = identity).
  With no trigger, the party goes to `(0, 1000, 0)`.
* `InsideDestinationTrigger` (TA:179) keeps spawned monsters out of 0x34 trigger spheres.

### 9.5 Doors, lifts, ladders: how to recognise them
* **Door**: a prop whose attached trigger has door action data (`action_data_kind 1`).
  Usually initial 0x01 + alternate 0x02. Lock/key/secret flags are in §5.5, and the
  auto-close delay is in m_lData1. `FindTriggerForProp` (TA:132) also finds a door that is
  driven through up to 5 recipient hops. Opening plays the prop's positioned-part
  animation (path point 0 = closed), and the prop's collision moves with it.
* **Elevator/lift/platform**: there is **no prop flag** for it. It is a COLLIDABLE (flags&1)
  prop with positioned_parts paths, toggled by action 0x03 or 0x32/0x33 on itself or
  from lever/control-panel props that list it as a recipient (e.g. "Lift control panel" →
  "Lift-00", "ElevatorLever" → "LoftElevator", "RatElevator"). The vertical travel is the
  path from point 0 to point N−1. Lifts that change level (Marten "Lift2Marten1/2",
  Gigas "elevatorToLevel2") end in a 0x34 trigger and/or level script logic.
* **Ladder/rope/climb**: a click trigger with 0x0f (relay) → a world trigger with
  action **0x09** naming a camera path ("ladderCamera", "Camera_LadderUp", "rope01" →
  "Camera01", "CamUp"). The climb is the camera path in section 7. It flies the party
  camera, and the party ends at the path's last point.
* **Teleport pads / portals**: 0x34/0x4b, and in some levels a level script.
* `W8_PATH_EDGE_TELEPORTAL` (`include/wiz8/navigation_flags.h:19`) belongs to the
  monster navigation graph (.wpt), not the .pvl.

## 10. Read but discarded, and unresolved

* The 72-byte zero trailer.
* Mesh record scale vec3. The 50-byte block and the channel bytes in animations. Item
  `4×i32 + 3×u8 + 4×i32`. Camera `i32, i32`. Type-4 `representation_scale`, `4×i32`, trailing
  `i32` (1.0f), legacy action. Type-1/2/4 legacy geometry. Type-2 unused byte. Type-3
  has_position. Clip plane w. Particle `positional[12]`. Path `unknown_08`
  (= 1/point_count).
* Light header flags bit 0 (seen in 8 lights with value 3): meaning unknown.
* Types 1 and 2 and the monster-path, item and clip-plane layouts are taken from the C++.
  Retail has no instances, so they are **not verified against data**.
* Prop `slots` frame/tag are on disk as u16 but used as i8/u8.
* Door key `item` u16: 0xffff = none (−1 as short).
