# Wizardry 8 `.oct` (compiled octree + collision) and sibling files

Verified against all 34 retail `.oct` files in `wizardry-prepared/levels/*/` with
`oct.py` (each one parses to EOF, every `0xFFFFFFFF` sentinel checks out, trailing bytes = 0).
C++ citations are to `whizzardry-8` (W8 below). All integers and floats are little-endian; there is no padding (`#pragma pack(1)`).

Units: raw world units, the same frame as the `.pvl` compiled meshes (no ×500; the ×500
`kWorldScale` is applied only to the legacy `.wgd` path, `level_loader.cpp:ReadWGDList`).
Retail Y is up. Rizzardry's `(x, y, -z) / 600` mapping applies unchanged.

## 1. File layout

Reader: `LevelSpatialIndex::LevelSpatialIndex(path)`, `src/spatial/level_spatial_index.cpp:1419-1610`.

| # | Section | Size | Condition |
|---|---------|------|-----------|
| 1 | `W8OctFileHeader` | 0xF5 | always (`include/wiz8/spatial/octree_geometry.h`) |
| 2 | u32 `0xFFFFFFFF` | 4 | always; `version` must be `0x22` |
| 3 | branches `W8OctPreTreeBranch` | 36 × `branch_count` | u16 provisional_region, u16 region, u32 children[8] |
| 4 | leaves `W8OctPreTreeLeaf` | 40 × `leaf_count` | u32 flags, region_off, polygon_off, gd_polygon_off, kind_offsets[6] |
| 5 | leaf polygon-id stream | 4 × `leaf_polygon_stream_len` | count-prefixed runs |
| 6 | leaf lookup grid | 4 × gx·gy·gz | only if gx·gy·gz < 250000 |
| 7 | `aulPolyLookup` (render polygon table) | 4 × `polygon_count` | |
| 8 | region-id stream u16 | 2 × `region_list_len` | if `region_list_len` ≠ 0; 0-terminated runs |
| 9 | GD-surface id stream u32 | 4 × `gd_surface_stream_len` | if ≠ 0. Per-leaf lists of collision surfaces |
| 10 | trigger list u16 | 2 × `trigger_count` | if ≠ 0 (0 in every retail file) |
| 11 | region volumes `RegionVolume` | 0xE8 × `region_count` | if `region_count` > 1 (`retail_geometry_records.h:53`) |
| 12 | u32 `0xFFFFFFFF` | 4 | "region terminator" |
| 13 | submesh table `W8OctSubmesh` | 16 × (`submesh_count`+1) | if `submesh_count` ≠ 0: u32 flags, i32 mesh, u32 next_link, u32 polygon_count |
| 13a | alpha bits (packed bits) | var | inside 13, if `mesh_total` ≠ 0 |
| 13b | particle links: u16 × (`mesh_total`+1) lookup, u16 × `particle_len` records | var | inside 13, if `particle_count` ≠ 0 |
| 13c | prop links: same with `prop_len` | var | inside 13, if `prop_count` ≠ 0 |
| 14 | u32 `0xFFFFFFFF` | 4 | "mesh terminator" |
| 15 | path grid `W8FilePathNode` | 8 × `path_nodes` | if `path_nodes` ≠ 0 (`navigation.cpp:322 ReadPathNodes`) |
| 15a | `W8ConditionalPathHeader` | 16 | u32 path_count, frame_count, node_count, flags; if frame_count<2 or node_count<2 all three are treated as 0 |
| 15b | `GDPropCondPaths` × path_count (char name[64], u32 lookup_index); u32 × frame_count lookup; u16 × frame_count frames; u32 × node_count keys; u32 × node_count values | var | |
| 16 | prop sun bits (packed bits) | var | if `prop_count` ≠ 0 and `prop_sun_bits` ≠ 0 |
| 17 | u32 `0xFFFFFFFF` | 4 | "octree terminator" |
| 18 | processed GameData (collision), §2 | var | if `gd_surface_stream_len` ≠ 0 (all retail files have it) |
| 19 | u32 `0xFFFFFFFF` | 4 | only if 18 is present; this is EOF in every retail file |

Packed bits (`src/wiz8/packed_bits.cpp:203`): u32 `0xDEADD00D`, u32 bit_count, u32 packed_size,
`packed_size` bytes (Huffman-packed words), u32 `0xDEADD00D`. A parser can skip `packed_size` bytes.

### 1.1 Header (0xF5 bytes)

| Off | Type | Field | Notes / retail observation |
|-----|------|-------|----------------------------|
| 0x00 | u16 | version | `0x22` |
| 0x02 | f32×3 | extent, cell_size, node_extent | |
| 0x0E | f32×18 | bounds[6] vec3 | [0],[1] = octree min/max (= pvl bounds ± 500 in 31/34 levels); [2],[3] = clipped min/max; [4],[5] not read |
| 0x56 | u32×3 | leaf grid dims x,y,z | |
| 0x62 | u16 | depth | |
| 0x64 | u16 | region_id_bound | |
| 0x66 | u32 | submesh_count | **= `.pvl` mesh_count** (all 34) |
| 0x6A | u32 | branch_count | |
| 0x6E | u32 | leaf_count | |
| 0x72 | u32 | polygon_count | **= sum of `.pvl` polygons + 1** (slot 0 is a sentinel; all 34) |
| 0x76 | u32 | vertex_count | render vertex count after welding; ≠ pvl vertex sum |
| 0x7A | u32 | surface_count | = GD `surface_count` (all 34) |
| 0x7E | u32 | path_nodes | path-grid entry count |
| 0x82 | u32 | leaf_polygon_stream_len | dwords |
| 0x86 | u32 | gd_surface_stream_len | dwords; also gates section 18 |
| 0x8A | u32 | trigger_count | always 0 |
| 0x8E | u32 | zero | |
| 0x92 | u32 | region_list_len | u16 count |
| 0x96 | u16 | region_count | |
| 0x98 | u16 | leaf_level | |
| 0x9A | u32 | root_mesh_count | differs from mesh_total in data (e.g. monastery1 192 vs 221), so the W8 comment saying they are equal is wrong |
| 0x9E | u32 | mesh_total | **= `.pvl` root_count** (all 34); sizes the link lookup tables |
| 0xA2 | u32 | kind1_submesh_count | |
| 0xA6 | f32 | region_grid_cell | |
| 0xAA | u16 | pad | |
| 0xAC | f32 | region_cell | **path-grid cell size** (500.0 in monastery1) |
| 0xB0 | u32 | edge_node_count | |
| 0xB4 | f32 | path_clearance | stack garbage in retail |
| 0xB8 | u8 | prop_sun_bits | presence flag for section 16 |
| 0xB9 | f32 | max_region_radius | |
| 0xBD | u32 | prop_count | |
| 0xC1 | u32 | particle_count | |
| 0xC5 | u16 | particle_len | |
| 0xC7 | u16 | prop_len | |
| 0xC9 | 44 bytes | pad | |

Sections 3-13 are the render/visibility octree (regions, PVS-style culling, per-mesh prop/particle
links). You don't need them for collision. Section 9 is only a spatial acceleration index over the §2 surfaces.

## 2. Processed GameData (collision) block

Reader: `LevelSpatialIndex::ReadProcessedCollision`, `src/world/level_loader.cpp:1414`.

Header `W8ProcessedGameDataHeader` (0x68 bytes, `level_loader.cpp:1364`):

| Off | Type | Field | Real meaning in retail data |
|-----|------|-------|-----------------------------|
| 0x00 | u32 | version | 1 |
| 0x04 | f32×3 | minimum | bounds of the GD vertices |
| 0x10 | f32×3 | maximum | |
| 0x1C | i32 | vertex_count | |
| 0x20 | i32 | surface_count | |
| 0x24 | i32 | "trigger_surface_base" | **= number of WALKABLE surfaces.** Surfaces are sorted walkable first (`surface_geometry.h compilationOrder`). W8's name is wrong. |
| 0x28 | i32 | "trigger_surface_count" | **= surface_count − walkable count** (the non-walkable run) |
| 0x2C | i32 | integrated_surface_count | = number of CROSSING (trigger) surfaces |
| 0x30 | i32 | interface_count | 0 in all retail files |
| 0x34 | i32 | state_count | 0 |
| 0x38 | i32 | trigger_count | trigger planes (= crossing surfaces / 2) |
| 0x3C | i32 | cond_poly_count | 0 |
| 0x40 | i32 | environ_count | 1 |
| 0x44 | 36 bytes | reserved | |

Then these arrays follow, with no separators:
- `vertex_count` × vec3 f32 (12 bytes)
- `surface_count` × `GDSurface` (0x4C bytes, below)
- `interface_count` × {i32 id, state_count, iStates}
- `state_count` × {i32 group, poly_count, poly_first}
- `cond_poly_count` × i32
- `environ_count` × `Environ` (0x44 bytes)

### 2.1 `GDSurface` (0x4C), `retail_geometry_records.h:27`

| Off | Type | Field | Notes |
|-----|------|-------|-------|
| 0x00 | u32 | flags | §3 |
| 0x04 | u32 | index | = its own position (all 34) |
| 0x08 | i32 | trigger_index | trigger number for CROSSING; environment index for ENVIRONMENT |
| 0x0C | i32×3 | edge_link | neighbour surfaces, −1 = open edge |
| 0x18 | i32×3 | vertex_indices | into the GD vertex array |
| 0x24 | f32×4 | plane | unit normal + w (n·p + w = 0) |
| 0x34 | f32 | distance | runtime scratch (garbage/0 on disk) |
| 0x38 | u32 | hit_plane | runtime pointer slot (0 on disk) |
| 0x3C | u8 | footstep_surface | acoustic class (`AmbientSound.h:105`): 7 outdoors flat, 8 outdoors canyon, … |
| 0x3D | u8 | footstep_material | 4 = SHALLOW_WATER, 3 stone, 10 rough stone, 12 mud, … (`AmbientSound.h:122`) |
| 0x3E | u8×2 | positional | garbage |
| 0x40 | f32 | contact_margin | already ×500: 500 normal, 50 steep walkable, 550 trigger |
| 0x44 | u32 | chance | mostly 0 |
| 0x48 | f32 | slope | motion factor for walkable faces (1.0 flat; 0 if EXPLICIT_SLOPE) |

The surface order is `[walkable run (including horizontal triggers)][non-walkable run (including vertical triggers)]`.
Triggers sit at the end of the walkable run, or at the very end of the file.

`Environ` (0x44): i32, u8 ground_latch, u8 airborne, u8[2], i32, then 14 f32: scale, gravity xyz
(0, −187.5, 0), motion_step 0.05, motion_factor 1, vector xyz, world_height **1000**,
forward_scale 1500, motion_limit 112.5, momentum_scale 0.3, value_40 1. It is the same in every retail level.

## 3. Surface flags (`include/wiz8/geometry.h:22`)

| Bit | Name | Meaning | Retail count |
|-----|------|---------|--------------|
| 0x3 | AXIS | dominant normal axis (0=x, 1=y, 2=z) for 2-D point-in-triangle | all |
| 0x4 | WALKABLE | floor: contact sets the walkable-contact flag, footstep sounds, slope factor | e.g. monastery1 4817/24706 |
| 0x8 | COLLISION_PROCESSED | runtime only (per movement step) | 0 |
| 0x10 | CONDITIONAL_DISABLED | switch-interface surface currently off (not collidable) | 0 |
| 0x20 | EXPLICIT_SLOPE | authored slope < 0: slope forced to 0 (slides) unless the slope-override script+worn item | e.g. ascension 7180 |
| 0x40 | PATHFINDING | authored walkable (`.wgd` face type == 1). The path grid is built only on these. | |
| 0x80 | CROSSING | trigger plane: not solid, fires trigger `trigger_index` when crossed | 2-36 per level |
| 0x800 | PROP_GEOMETRY | prop surfaces only (never in `.oct`) | 0 |
| 0x1000 | ENVIRONMENT | environment-swap plane (not solid) | 0 |
| 0x8000 | SKIP_FILTERED_TRACE | prop only | 0 |

No `.oct` uses 0x10, 0x800, 0x1000 or interfaces. Doors and switches are not part of the level GD.

How the flags are derived at build time (`ClassifySurfacePlane`, `level_loader.cpp:1532`, together with
`ReadWGDList`). The values are already baked into `.oct`, so you don't need to recompute them:
- Face type 1 → WALKABLE|PATHFINDING.
- Otherwise, if normal.y > 0.996 (`kSurfaceFlatNormalThreshold`) → WALKABLE with slope 1 ("auto-flat": flags 0x4 without 0x40).
  The data confirms this: every non-walkable surface has normal.y ≤ 0.996.
- If a walkable surface has normal.y ≤ 0.5 and margin ≈ 1.0 → margin 0.1 (50 units). These are authored-walkable steep faces and ceilings
  (monastery1 has 220, 93 of them with normal.y < 0).
- If slope < 0 → EXPLICIT_SLOPE with slope 0. If a walkable surface has slope ≈ 0 and no EXPLICIT_SLOPE → slope = normal.y if ≤ 0.75, else 1.
- contact_margin is multiplied by 500. Trigger planes (`AddTriggerTriangle`) get margin 1.1 → 550, plus CROSSING, and WALKABLE if they are horizontal.

## 4. How party collision uses the surfaces (`src/spatial/collision.cpp`)

- Probe point = camera.y + (500 − world_height) = **eye − 500 = feet + 500**, because the eye is feet + 1000
  (`g_default_world_height`, `level_session.cpp:280`; `collision.cpp:441`).
- The probe is swept, not a true sphere. `W8GDSurface::segmentContact` (`collision.cpp:800`) stops the point
  `limit` units in front of each plane:
  - Walkable: limit = contact_margin (500 → a 500-radius "sphere" centred 500 above the feet, so the feet rest on the floor).
  - Non-walkable: limit = margin − (1 − |n.y|)·150 (`kCollisionSlopeMarginScale`). A vertical wall gives **350**.
  - Single-sided: the surface is ignored if the probe starts behind the plane or moves away from it (dot(n, dir) ≥ 0).
  - Edges and corners: `edgeContactLimit` rounds them to sqrt(limit² − d²), so they behave like a sphere.
- Solid set = every surface without CROSSING|ENVIRONMENT and without COLLISION_PROCESSED|CONDITIONAL_DISABLED
  (`collision.cpp:528`). Walls and walkable surfaces are both solid.
- Second pass: CROSSING/ENVIRONMENT surfaces get a crossing test along the frame's motion.
  If the plane was crossed, `ProcessCrossedSurface` (`collision.cpp:757`) runs the trigger with direction ±1 (from the sign).
  When the party has no sound environment, crossing tests lift the probe 165 units (`kCollisionHeightLift`).
- Walkable contact (`ResolveCollision`, `collision.cpp:995`): sets motion_factor = max(factor, slope), raises
  `W8_LEVEL_FLAG_WALKABLE_CONTACT`, and selects footstep surface/material. Gravity is −187.5 per tick-scale.
  Low slope on a walkable face means sliding. A non-walkable face gives no ground contact, so you fall and slide.
- Up to 5 slide iterations, 100 contacts. Monsters, props and doors are traced separately in the same loop
  (`ProbeMonstersAlongMotion` and `ProbePropsAlongMotion`). Prop surfaces are built by `initializePropSurface`
  (`surface_geometry.h`): margin 500, WALKABLE if n.y ≥ 0.5.
- Ground snap: `settleToGround(pos, true, 500)` drops onto walkable GD or props.

## 5. Triggers and doors

- Trigger planes: each `.pvl` trigger quad is two GD triangles with CROSSING. `trigger_index` is the order of
  the `.pvl` triggers (`AddTriggerPlane`, `level_loader.cpp:1302`: in compiled mode only the Trigger pointer table is filled).
  `.oct` already contains these triangles. They are never solid.
- Doors, gates, bridges and trapdoors are props. Their collision comes from the prop mesh at runtime, not from `.oct` GD.
  In `.oct` they appear only as named conditional path sets (§15b: monastery1 has `big_door_1`, `bridge`,
  `trapdoor`, `Box85`, …). Each set flips path-grid cells between DOOR, BLOCKED and INACTIVE per prop frame
  (`navigation.cpp:SetConditionalPathFrame`).

## 6. Path grid (monster navigation), §15

- Entries are a multimap from key `(cell_z << 16) | cell_x` to a u32 value.
  cell = floor((p.xz − octree_bounds.min.xz) / region_cell), with region_cell = 500.
- Value bits:
  - low 16 bits: height level h. World y = (h − 1) · span + min.y, with span = (max.y − min.y) / 65535 (`g_path_span_scale`).
  - bits 16-23: neighbour mask (when HAS_DIRECTIONS 0x01000000 is set), otherwise clearance depth.
  - 0x02000000 blocking frame, 0x04000000 conditional, 0x08000000 door, 0x10000000 inactive, 0x20000000 blocked
    (`navigation_flags.h`).
- Verified (`pathgrid.py`): 99% of monastery1/dungeon cells lie within 100 units of a PATHFINDING GD surface.
  Several cells per XZ key mean several floors.

## 7. Sibling files

- **`.rlk`, region links (render visibility).** u32 count; u32 keys[count]; u16 values[count]
  (`ReadRegionLinkFile`, `level_spatial_index.cpp:606`). It is a multimap from region id (or packed camera-cell key
  `0x1F......`) to the visible submesh/region. Used only to cull meshes. Several files have unread trailing bytes
  (e.g. monastery1: 19326). southeastwilderness has count 0 but 39 KB, and four files are 0 bytes.
  It looks like stale data from files overwritten without truncation, and the reader ignores it. Not needed for collision.
- **`.wpt`, waypoint graph (monster long-range patrol/pathing).** It is v2 everywhere. Layout
  (`NavigationMesh::ReadWaypointFile`, `navigation.cpp:234`):
  - Header: u32 version, u32 edge_node_count (unused), u32 waypoint_count, u32 link_count.
  - Waypoints (16 bytes): u16 flags, u16 first_edge, f32 x, y, z. Index 0 is a zero sentinel.
  - Edges (14 bytes): u32 flags (capability mask, e.g. 0x003FFFFF = all groups + walk/fly/swim + sizes;
    0x01000000 teleportal, 0x20000000 conditional, 0x80000000 disabled), u16 source (absent in v1),
    u16 destination, f32 distance, u16 next (chain from first_edge). Index 0 is a sentinel.
  - Waypoint flags seen: 0x1000, 0x2000, plus low bits 0x10 and 0x1. If (flags & 0xF000) == 0 the reader ORs in 0x2000.
  - Some files carry ignored trailing bytes too.
- **`.sts`, initial level status.** A RIFF-style `W8Chunk` container: 4CC + size stored as two LE u16 halves,
  high half first; 10-byte heads. A `LVLS` group holds `STAT` and other sections. It is loaded as the
  first-visit baseline `LevelState` (monster groups, props, level flags) by `BuildLevelStatusPath`
  (`LoadSaveGame.cpp:1236`) and `level_session.cpp:523`. It holds no geometry.

## 8. Water (checked with `water.py` and `water3.py` against the translucent `.pvl` water polygons)

Retail has **no water flag and no collision at the water surface**:
- Under most visible water there is a walkable GD floor 100-800 units below: the riverbed.
  Its footstep_material is usually 4 (SHALLOW_WATER), so the party wades along it.
  monastery1: 272 of 437 flat translucent polygons have walkable GD 0-800 units below and only 26 have GD at water level.
  Those 26 are marble floors with translucent decals.
- Deep or decorative water has **no GD floor underneath at all**. Examples: monastery1 125, swamp 128, mtgigas1 314 polygons.
  It is either far from any walkable GD (unreachable) or bordered by GD walls, i.e. invisible collision walls.
  The GD contains collision-only geometry: 8% (monastery1) to 36% (swamp) of GD vertices match no render vertex.
- Swimming exists only as a monster navigation mode (`W8_NAV_SWIM`, `W8_NAVIGATOR_MODE_SWIMMING`). It is not in party collision.

## 9. Recommendation for rizzardry

1. **Collision**: replace `collision_triangles` (pvl opacity ≥ 1) with the `.oct` GD surfaces. Use vertices as-is (raw units),
   then `(x, y, -z) / 600`. The stored plane equals cross(v1−v0, v2−v0) normalised (all 24706 monastery1 surfaces, |n·p+w| < 0.02). The normal faces the open side, and collision is single-sided. The Z mirror reverses handedness, so swap two indices to keep the normal pointing at the open side.
   - **Solid** for party and monsters: every surface with `flags & 0x1090 == 0`, i.e. not CROSSING, ENVIRONMENT or
     CONDITIONAL_DISABLED. That covers both walls and floors. Render polygons are not used for collision at all. That fixes riverbeds and also
     gives you invisible walls.
   - **Trigger volumes**: surfaces with 0x80. Index by `trigger_index` in `.pvl` trigger order and fire on plane crossing. They are not solid.
   - **Party shape**: sphere radius 500 (500/600 ≈ 0.833 Bevy units) centred 500 above the feet. To match retail more closely,
     shrink it against non-walkable faces by (1 − |n.y|)·150, so walls hold you 350 away. Walkable faces with margin 50 hold you only 50 away.
     Ground contact only counts on WALKABLE faces. Steep non-walkable faces are slide-only.
2. **Walkable floor / navmesh**: use `flags & 0x40` (PATHFINDING), excluding 0x80. This is exactly what retail's monster path grid covers.
   `flags & 0x4` (WALKABLE) is the wider set: it adds auto-flat tabletops and similar, plus authored steep or ceiling faces.
   For a navmesh also require n.y > 0.5. Easier and more faithful: use the **path grid in the `.oct` directly** (§6), which is a
   500-unit grid of walkable cells with heights and door/conditional flags. Use the `.wpt` graph for long routes.
3. **Water**: not solid, not blocking. Render it as decoration only. Shallow water is walkable via the GD riverbed.
   Deep water is excluded by GD walls or by having no floor.
4. **Doors and props**: collide with prop meshes at runtime (margin 500, walkable if n.y ≥ 0.5). For monster nav, toggle the
   named conditional path sets per door state.
