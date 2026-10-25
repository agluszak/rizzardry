# Level population: who stands where when the party arrives

How retail fills a level with monsters and NPCs, with enough detail to spawn
them as animated models. C++ paths are relative to the Whizzardry 8
checkout. Every layout below was checked by decoding all 34 retail `.sts` files,
`encountertables.dbs` and `levels.dbs` with a throwaway parser. Each parse
consumed its section exactly, and the 924 placed monsters cover every record.
Behaviour comes from Whizzardry only and is reconstructed, not proven.

## 1. Where placements live

| What | Where | Notes |
| --- | --- | --- |
| Fixed monster groups and named NPCs | `levels/<folder>/<level>.sts`, `MONS` chunk | One record per individual monster with its own position and yaw. No counts or dice. |
| Random-encounter generators | same `.sts`, `MONG` chunk | Points with an encounter-table index. |
| Encounter tables | `data/databases/encountertables.dbs` | Species, rarity, day/night and challenge per entry. |
| Per-level encounter budget | `data/databases/levels.dbs` +0x3C..0x54 | |
| Species → model | `monsters.dbs` `cycle_name` (+0x189) → `data/monsters/<cycle_name>.mls` | See [monsters-db](monsters-db.md) §1 and [monster-anim](monster-anim.md). |
| Script-spawn points | `.pvl` named positions ([pvl-sections](pvl-sections.md) §7.5) | Used by level code, story restores and trigger scripts. |
| Ground items | `.sts` `ITEM` chunk | Out of scope here. |

- The `.sts` path is `Levels\<folder>\<name>.STS`, with the folder table indexed
  by level id (`LoadSaveGame.cpp:1219-1226`).
- `.wpt` is the monster waypoint graph (`src/spatial/navigation.cpp:207-246`).
- `.rlk` holds region links (`src/spatial/level_spatial_index.cpp:607-620`).
- Neither `.wpt` nor `.rlk` holds placements.
- `.pvl` monster paths exist in the format but are 0 in every retail level
  ([pvl-sections](pvl-sections.md) §3.2).

**Named NPCs are ordinary `MONS` monsters.** Their species has `monsters.dbs`
flag `0x01` (NPC), and `npc_kind` (+0xCD, `0xFA` = none) is the `npc.dbs`
record index (`include/wiz8/retail_records.h:333-352`). The model is the species'
`cycle_name`, like any other monster. Binding to the NPC's persistent state
happens at load (`src/wiz8/npc_creation.cpp:145-196`). For example, Burz is
species 218, flags 0x11, `npc_kind` 80, which is npc.dbs record 80 "Burz", using
`trynnie_NPC.mls`. Across all levels, 241 of the 924 placed monsters have an
NPC species.

- **Trigger-bound NPCs have no body.** An NPC whose record is a merchant or
  voice-script NPC can be bound to a trigger named `_<source_name>`
  (`NPC Manager.cpp:1749-1780`).

## 2. Record layouts

### 2.1 Chunk file framing (`src/wiz8/local_code/chunk.cpp`)

- **Chunk header** (10 bytes): `char id[4]`, `u8 grouped`, `u8 at_end`,
  `i32 extent`, then `extent` payload bytes (`chunk.cpp:233-255`).
- **Group payload**: a group's payload starts with `i32 child_count`, followed by
  the child chunks (`chunk.cpp:165-182`).
- **The `grouped` byte is 0 on disk in retail `.sts` files**, even for the
  groups. The reader knows which chunks are groups.
- **`at_end` chunks** are skipped (`include/wiz8/chunk.h:124-139`).
- **`.sts` tree**:
  - Root `RIFF` (a group, 1 child).
  - `LVLS` (a group): `i32 child_count`, then `i32 level_id`, then the children
    (`LoadSaveGame.cpp:224-238`).
  - Monastery 1 has level id 8 and 7 children: `STAT MONS ITEM CUBE MONG LOCK TRES`.
  - The chunk ids are listed at `LoadSaveGame.cpp:117-145`.

### 2.2 `MONS` (`LoadSaveGame.cpp:738-760`, records at `:387-676`)

```
u32 group_count, u32 monster_count
group_count × { u32 size (=0x12B), u8 image[size],
                u8 encounter  -- only if image.version (@0xC4) >= 2 }
monster_count × {
  u32 version (=7 in all retail), u32 size (=0x425), u8 image[size],
  u8 has_script; if set: char name[64], i32 wait, i32 line, i32 n, u8 cond[n]
  u8 unborn                         -- version >= 5
  u8 has_movement; if set:          -- version >= 2
     f32 a, f32 b, vec3 patrol_home, vec3 target
  -- version >= 3, orders:
  u8 defining_orders, i8 order_mode, u8 orders_finished, u8 deaf,
  f32 patrol_distance, f32 patrol_variation, i8 patrol_index,
  i32 n, vec3 points[n],
  vec3 direction (v>=4), u8 face_party (v>=6), u8 stay_home (v>=7)
}
```

- Short images are zero-extended (`ReadSizedImage`, `:261-267`).
- `order_mode` (`include/wiz8/monster_actions.h:35-42`): −1 none, 0 guard,
  1 patrol, 2 point patrol, 3 random point patrol, 4 face direction.
  - Corpus counts: −1: 723, 0: 87, 1: 105, 2: 1, 3: 3, 4: 5.
- The script name is an `.msf` under `data/monsters/scripts/`.
  - `Default.MSF` is `PATROL 30 300`, which matches the stored 15000/150000 ×500.
- When the orders are unused, the patrol floats hold garbage.

**Group image** (0x12B, `include/wiz8/retail_save_records.h:108-133`;
field notes in `include/wiz8/local_code/MonsterGroup.h:21-103`):

| Off | Field |
| --- | --- |
| 0x00 | i32 group_id |
| 0x04 | u32 member_count (a saved count; members are re-linked from the monster records) |
| 0x18 | i32 species (`monster_id`) |
| 0x1C | vec3 centre |
| 0x2A | u8 disposition: 0 neutral, 1 hostile, 2 friendly |
| 0x2C | u8 alternate_name |
| 0x9F | i32 leader_location_id |
| 0xA3 | i32 leader_group_id (0 = none) |
| 0xA7 | i32 allied_group_ids[4] |
| 0xB7 | vec3 `formation`: in practice the group's home point (equals centre in retail data; generators set it to their own position) |
| 0xC3 | u8 encounter_registered (0 in all retail `.sts` groups) |
| 0xC4 | u32 version (3 in all retail) |

**Monster image** (0x425, `retail_save_records.h:40-106`, asserts at `:223-227`):

| Off | Field |
| --- | --- |
| 0x000 | i32 location_id (unique per level; the identity triggers and groups use) |
| 0x004 | i32 group_id |
| 0x008 | u32 species (monsters.dbs index) |
| 0x014 | u8 fActive (1 on disk; ignored, the loader clears it) |
| 0x016 | u8 disposition |
| 0x017 | **vec3 position**: world units (not ×500), feet, see §3.2 |
| 0x023 | **f32 yaw** (`derived`), radians, heading `atan2(dx, dz)` |
| 0x027 / 0x02B | i32 hp_max / u32 hp (rerolled on first visit) |
| 0x24E | u8 fMotionless (0 in all retail) |
| 0x24F | f32 scale (rerolled on first visit) |
| 0x2F1 | i32 bound_npc_index (an NPC-slot index, −1 = none; see §5) |

### 2.3 `MONG` (`src/world/encounters.cpp:490-516, 639-669`)

```
i32 version (5 in retail)
i32 saved_budget (v>4), i32 random_budget, i32 culling_seconds (v>1)
v>2: u8 save_flag, i16 default_chance, i16 default_interval, i16 unused
i32 count
count × { u8 version (3);
          v>=3: char name[32], u8 enabled;
          u32 flags, u8 chance, u16 interval_s, u16 unknown, vec3 position,
          i32 table_index (into encountertables.dbs, −1 none);
          v>1: f32 timer_progress, f32 timer_scale }
```

- The second header field, which the C++ calls `interval_min`, is actually used
  as the default spawn chance (`encounters.cpp:366-368`).
- Flag bits (`:100-105`):
  - 1: disabled.
  - 4: armed (always cleared on load).
  - 8: use the header's default chance and interval instead of the generator's own.
  - 0x20: story-gated. These are HARASSMENT tables: they spawn only if the
    species' faction is hostile and a Destinae, Astral or Chaos "possess" fact is
    set.

### 2.4 `encountertables.dbs` (`src/wiz8/loaded_game_data.cpp:250-279`)

- **File header**: `i32 n_names`, `n_names × char[256]` (unused category names),
  `i32 n_tables`.
- **Each table** begins with a 0x108-byte header: `u8 kind`, `char name[256]`,
  `u32 ?`, `u16 version`, `u8 n`.
- **Entries follow as columns**: `u16 species[n]`, `u8 rarity[n]` (3/7/20/70),
  `u8 time[n]` (0 day, 1 night, 2 any), `u8 challenge[n]`, `char script[64][n]`.
- **Trailer**: one extra byte if `version != 1`.
- **Retail**: 72 tables.

### 2.5 `levels.dbs` (`include/wiz8/retail_records.h:312-332`)

- **File layout**: `u32 count` (60), then `count × 0xD8` records, indexed by
  level id.
- **Encounter fields**, all i32:
  - +0x3C max simultaneous random groups
  - +0x40 min simultaneous random groups
  - +0x44 max budget
  - +0x48 min budget
  - +0x4C budget period
  - +0x50 culling seconds
- **+0x54** is an f32 time scale.

## 3. What happens at level entry

### 3.1 Choosing the state (`src/world/level_session.cpp:491-525`)

- If the party has been here before, the **saved image of the level as it was
  left** is used (`left_levels`, `:515`). Otherwise the asset `.sts` is used (`:517`).
- On a revisit, a saved level takes only locks, cursor nodes, the generator list
  and trigger actions from the `.sts` (`ApplyLevelBase`, `LoadSaveGame.cpp:1291-1321`).
  - The `.sts` monsters are never reloaded, so fixed monsters that have died stay dead.
- Building (`LevelState::build`, `LoadSaveGame.cpp:1469-1508`):
  - Every monster of a non-deleted species gets a body from its `.mls`
    (`:1493`). No retail placement uses a deleted species.
  - Groups and members are linked, with the leader taken from `leader_location_id`
    (`:1336-1400`).

### 3.2 Placing a body (`src/wiz8/local_code/MonsterManager.cpp:122-185`)

- **All** loaded monsters are placed immediately. There is no proximity
  activation.
- Position: the stored position is settled onto the ground (`SetAdjustedPosition`,
  `:138`, i.e. `Navigator::AdjustPosition`).
  - **y is the feet.** Saving writes the ground-probed y (`LoadSaveGame.cpp:640-643`).
  - In Monastery 1, a roach group and a bat group share y = 10022, while group
    homes and generators sit at 10047. So snap to the ground below the point.
  - Not yet checked against the `.oct`.
- **Facing**: `SetFacing(yaw)` (`:160`). The yaw is a heading: retail direction
  `(sin yaw, 0, cos yaw)`.
  - In Rizzardry this is `retail_yaw(yaw, [0,1,0])` for the actor; the
    model-root π turn in `src/presentation/monsters.rs` already handles the mesh.
  - Position is `retail_position(position)`.
- **Animation**:
  - A normal monster plays cycle 1 (IDLE), never stopping, from a **random start
    frame** (`:152-156`).
  - An `unborn` monster is frozen on frame 0 of cycle 0 (BIRTH), inactive, until
    a script births it (`:140-150`).
  - The corpus has 8 unborn records: mtgigasouter 6, rift1 1, seacave1 1.
- **First visit only** (`level_session.cpp:393-401, 444-446`):
  - HP, stamina and scale are rerolled for every monster
    (`World::initializeMonsterVitals`, `MonsterManager.cpp:569-582`).
  - The scale comes from HP within the `.mls` `scalerange`, with ±20 % jitter
    (`:184-206`).
  - Group scripts restart.
  - The `.sts` HP and scale values are therefore irrelevant at a new game.

### 3.3 Random encounters (`encounters.cpp`)

- **When the rolls happen** (`level_session.cpp:423-430`; `UpdateRandomEncounterBudget`,
  `encounters.cpp:391-414`):
  - **First visit**: the budget is set to the level maximum and
    `RollRandomEncounters` runs.
  - **Revisit after more than 10 h of world time**: the same reroll happens.
  - **Shorter revisit**: the budget grows by `elapsed / period` and the live
    random groups age.
  - **Imported Umpani/Trang start**: random groups are despawned instead.
- **RollRandomEncounters** (`:706-733`) despawns the current random groups,
  shuffles the generators, then makes one **forced** attempt per generator.
  - A forced attempt skips the distance, line-of-sight, occupancy and chance
    tests.
  - It keeps the global gates (`:321-325`) and the simultaneous-group limit
    `clamp(budget, min_groups, max_groups)` (`:341-348`). Only main groups with
    no leader count towards that limit.
- **Afterwards**, each generator ticks every `interval ± 20 %` s (`:605-623, 685-691`).
  - An attempt succeeds with `chance` % (`:366-368`), and only when all of these hold:
    - The camera is between 35 000 and 200 000 wu away.
    - There is no line of sight.
    - The ±5000 wu box around the point is empty.
    - The group limit is not reached (`:327-370`).
- **Culling** (`:438-468`): random groups older than `culling_seconds` and more
  than 50 000 wu from the party are despawned.
- **Choosing the species** (`SelectEncounterCandidate`, `:117-176`):
  - Rarity is rolled: ≤3 → class 3, ≤10 → 7, ≤30 → 20, otherwise 70 (on `below(100)`).
  - Candidates must have that class (scaled by level mismatch), satisfy
    `|1 − challenge / party_level| < 0.5` and pass the time test. The party level
    is `floor(avg + 0.49)`.
  - If no entry qualifies, any entry of the class that passes the time test is used.
  - **Night** is game time outside (05:00, 22:00].
- **Group size** (`RollEncounterGroupSize`, `:184-219`): `group_size_dice`, biased
  by difficulty when `display_level / party_level ≥ 1.2` or when the dice midpoint
  is ≤ 0.8.
- **Companions**: each of the 2 companions joins with its own chance, as a linked
  group (`:255-305`).
- **Group creation and placement** (`createMonsterGroup`,
  `src/wiz8/local_code/MonsterGroup.cpp:567-610`):
  - At most 9 members, all spawned at the generator point.
  - Members face the party eye.
  - They scatter in a grid at spacing 2 × leader radius: 3 columns (0, +1, −1),
    or 5 when there are more than 10 members. Rows run away from the party, with
    ±0.2-cell jitter (`placeMonsterGroup` `:59-140`, `scatterPositions`
    `level_spatial_index.cpp:2092-2170`).
  - The generator point is the group's home, with `Default.MSF` patrol unless
    the table entry names a script.

### 3.4 Story and NPC conditions

- **Fixed `.sts` placements have no day/night or fact conditions.** Time of day
  matters only through the encounter `time` column.
- **NPC binding**: a monster with an NPC species is bound to the NPC's persistent
  state through `npc_kind` (`npc_creation.cpp:145-196`).
- **Removal and restoring**: NPC state and story events can remove a bound body
  later, or restore an NPC at a `.pvl` named position:
  - `restoreNamedNpc` / `restoreNpcMonster`, `NPC Manager.cpp:1523-1616`.
  - The pending restores run at entry (`ClearPendingNpcLevelFlags`, `:1542-1553`).
- **Scripted spawns** happen at named positions:
  - Level code, e.g. Monastery 1's roaches, spiders and coffin head
    (`level_specific_code/Monastery1.cpp:52-105`).
  - Trigger action 0x0b, which wakes a group by id
    ([pvl-sections](pvl-sections.md)).

## 4. Worked example: Monastery 1 at a new game

The level is `levels/monastery/monastery1.sts`, level id 8. The party enters at
trigger `MO100` (eye ≈ x 76 000, z 52 500).

- **New party (fact VIRGIN)**: no story NPC is added.
- **Imported party** (not Umpani or Trang): Vi Domina (npc kind 0x18, species
  395, `vi_domina.mls`) is restored at named position `NP_ViMon`, (71 069, 1 377,
  54 214) in world units after the ×500 (`NPC Manager.cpp:892-931`).

### 4.1 Fixed placements (`MONS`: 13 groups, 28 monsters)

All `.mls` files exist in `data/monsters/`. Positions are retail world units:
x, y = feet, z. Yaw is in radians. Dispositions: H hostile, N neutral, F friendly.

| Group (home) | Species | `.mls` | Disp | Members: position / yaw | Script / orders |
| --- | --- | --- | --- | --- | --- |
| 5 (74899, 11701, −313) | 109 Noxious Slime | slime_yellow | H | (77122, 11676, 68) / 0.09 | — |
| 46 (35813, 3875, 15807) | 108 Green Slime | slime_green | H | (34738, 3861, 14601) / 1.21; (34634, 3850, 17647) / 1.29; (35935, 3850, 16427) / 1.29 | — |
| 67 (55242, 10043, −15748) | 209 King Crab | crab | H | (55242, 10018, −15748) / 3.82 | — |
| 80 (102394, 10047, −51955) | 312 Roach | water_scorp_small | H | (102337, 10022, −52748); (102535, 10022, −51103); (102394, 10022, −51955), all 1.64 | — |
| 102 (58773, 404, 48075) | 253 Crab | crab_small | N | (57423, 387, 46776) / 1.53; (58137, 381, 49517) / 2.61; (58773, 379, 48075) / 1.53 | — |
| 156 (120107, 17579, −19263) | 17 Screaming Head | screaming_head | H | (120107, 17498, −19263) / 5.19 | — |
| 157 (120007, 17579, −15691) | 17 Screaming Head | screaming_head | H | (120007, 17518, −15691) / 4.38 | — |
| 242 (112996, 17428, 34959) | **218 Burz (NPC, npc.dbs 80)** | trynnie_NPC | F | (112152, 17354, 34302) / 5.18 | `proximityNPC.msf`, guard |
| 244 (−3964, 10047, 12214) | 106 Bat | bat | H | (−10675, 10022, −59992) / 2.75; (−10218, 10022, −61123) / 2.75; (−9366, 10022, −63092) / 2.70 (leader) | leader `Default.MSF` patrol |
| 251 (42097, 18436, −44954) | 414 Gregor | gregor | H | (42524, 18436, −46219) / 5.85 | `gregorpatrol.msf` (deaf, PATROL 1 20) |
| 262 (96322, 10868, 1709) | 467 Tanika Fish | baruta_fish_shallow | N | (105370, 10534, 11926) / 5.83; (105100, 10555, 11435) / 0.13; (104677, 10559, 12250) / 5.86; (102922, 10655, 13334) / 5.17 (leader) | leader `Default.MSF` patrol |
| 296 (85794, 5919, 96205) | 309 Softshell Crab | crab_small | H | (86648, 5856, 97499) / 5.67; (84397, 5765, 96374) / 0.35; (84612, 5899, 94652) / 0.18; (86888, 6066, 94695) / 5.97; (85794, 5882, 96205) / 6.16 | — |
| 298 (17437, 9936, −30835) | 109 Noxious Slime | slime_yellow | H | (17437, 9911, −30835) / 0.17 | `guard.msf` |

- No record is unborn, and no record has movement except the two patrol leaders
  and Gregor.
- Gregor (species 414) has `npc_kind` 0xFA, so he is a plain monster, not an NPC.

### 4.2 Random encounters rolled on arrival

- **Level settings**:
  - `levels.dbs` row 8: groups 1..5, budget 0..15, period 2440, culling 180 s.
  - `MONG` header: default chance 30 %, default interval 180 s.
- **Generators**:

| Generator | Position | Table | Own chance / interval |
| --- | --- | --- | --- |
| MonGen001 | (52044, −702, 76335) | 5 "Crabs" | 10 % / 120 s |
| MonGen002 | (8548, 10047, 24286) | 7 "Monastery 1 General" | defaults (flag 8) |
| MonGen003 | (−16632, 10047, 10775) | 7 | defaults |
| MonGen004 | (−3359, 10047, −33697) | 7 | defaults |
| MonGen005 | (31115, 10047, −80674) | 7 | defaults |
| MonGen006 | (50008, 10047, −67336) | 7 | defaults |
| MonGen007 | (19097, 9936, −32874) | 6 "Slimes" | 40 % / 60 s |
| MonGen008 | (85538, 11954, −23772) | 7 | defaults |
| MonGen009 | (78099, 18458, 16922) | 7 | defaults |

- **How many spawn**: the first visit forces an attempt at all 9 generators in
  shuffled order, capped at **5 main groups**. All entries are time 2 (any time).
- **What spawns with a level-1 party**, where only challenge-1 entries qualify:
  - Crabs: always 253 Crab (`crab_small`), 1d3.
  - Slimes: always 108 Green Slime (`slime_green`), 1d2.
  - General, by rarity class:
    - class 70: 258 Rat (`giant_rat`, 2d3), 108 Green Slime, or 106 Bat (`bat`, 1d2)
    - class 20: 258 Rat or 107 Roach (`water_scorp_small`, 1d2)
    - class 7: 106 Bat
    - class 3: 253 Crab
- **Companions**:
  - Crab: 30 % chance of a second 253 group.
  - Green Slime: 15 % chance of 313 Green Slime.
  - Bat: 10 % chance of 270 Bat.
- **Placement**: the groups appear at the generator points, scattered as in §3.3
  and facing the party.

## 5. Uncertainties

- **Reconstruction only**: all behaviour is from Whizzardry. The original decomp
  was not available.
- **`bound_npc_index`**: on disk it is an NPC-slot index (69 for Burz) that does
  not match `npc_kind` (80). The loader binds through it only if that slot
  already exists (`LoadSaveGame.cpp:1486-1488`), and `prepareNpcBinding`
  re-resolves through `npc_kind` anyway. For a new game, ignore it and use
  `npc_kind`.
- **The two floats before `patrol_home`** in the movement block are named
  min/max height, but in retail they equal the patrol distance and variation.
  Their meaning is unverified.
- **The y = feet claim** rests on the save code and on in-level consistency. It
  has not been checked against `.oct` ground heights, and the editor that wrote
  the `.sts` files may have stored heights differently.
- **What an unborn monster looks like** before its birth (hidden, or frozen on
  BIRTH frame 0) depends on the renderer path. It was not traced.
- **Positions taken after `createMonsterGroup`**: the scatter grid's exact
  orientation and whether a probe is accepted depend on navmesh snapping
  (`SnapWaypointPosition`). The positions above for random groups are therefore
  approximate by design.
- **`NP_GuardMon`** exists in Monastery 1 but nothing in Whizzardry references it.
- **Monastery1.cpp coordinates**: `Monastery1.cpp:63-65` raises the roach spawn
  points along **z**, while the comment says "above". Either the comment or the
  axis is wrong.
- **Unread fields**: NPC scripts (`.nsf`) and the fact-driven NPC movement over
  the story were not followed. Only the level-entry passes were.
