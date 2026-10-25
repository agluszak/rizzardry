# Monsters.DBS, monster sizes and movement classes, and level placement

The parser is `monsters_db.py`, which imports `monster_anim.py`. Its full output is in
`monsters_db.out`. C++ paths are relative to `/home/agluszak/code/personal/decomp/whizzardry-8`.

## 1. monsters.dbs layout

- The file is a `u32 count` (595) followed by count × 0x297-byte packed records. The file size
  matches exactly.
- Reader: `src/wiz8/loaded_game_data.cpp:292-309`. Layout and offsets:
  `include/wiz8/retail_records.h:333-472` (`MonsterRecord`), with field notes in
  `include/wiz8/data/records.h:87-111,415-555`.
- `record_id` (0x187) equals the index for every record.

| Off | Field | Off | Field |
|---|---|---|---|
| 0x000-0x0BF | name0..3, 4 × 48 bytes **UTF-16LE**, cut at `#` | 0x181 | u32 experience |
| 0x0C0 | can_open_doors | 0x187 | i16 record_id |
| 0x0C1 | group_size_dice (i16 base, u8 n, u8 sides) | **0x189** | **char[48] cycle_name → `data/monsters/<cycle_name>.mls`** |
| 0x0C5/0x0C8 | companions[2] (i16 species, u8 chance) | 0x1B9 | prefer_ranged_actions |
| 0x0CB | kind (body/creature class) | 0x1BA | f32 combat_move_range |
| 0x0D0 | flags: 1 NPC, 2 vulnerable from behind, 4 bodyguard, 0x10 alternate name | 0x1BF/0x1C0 | combat_behavior / morale |
| 0x0D6/0x0DA | HP / stamina dice | 0x1C3 | char[48] remains model (creaturegoo/plantgoo) |
| 0x0E0-0x0E6 | spell / flee / advance %, special kind, initiative, attacks / swings per round | 0x1F3 | treasure block |
| 0x0E7 | 3 × attack (0x22 bytes: has, weapon name, type, range, score, …, modes @0x15, damage dice @0x17) | 0x24F/0x251 | effective / display level |
| 0x14D | spells[10] | 0x253/0x257 | model_index / alternate (570 of 595 are 0) |
| 0x15E | constitution (hit-location label row) | 0x25B | i32 hostility_radius |
| 0x15F/0x166/0x16D | hit-location %, AC by location, AC by attack mode | 0x25F/0x263 | faction / material |
| 0x176 | resistances[6] | 0x267/0x26A | deleted / unborn |

**The database has no size, radius, height, speed or movement class.**

- The only spatial fields are `combat_move_range` (0.0-8.0, mostly 1.0-1.4; a combat movement
  multiplier) and `hostility_radius`.
- Everything physical comes from the `.mls` named by `cycle_name`:
  - `Monster::ReadAllCycles(world, record->cycle_name, …)`
    (`src/wiz8/local_code/MonsterManager.cpp:235`)
  - `LoadMonsterDefinition` (`src/entities/monster_loader.cpp:151-520`).
- 205 distinct `.mls` files are used. 11 records name `death06`…`death16`, which have no `.mls`.

## 2. Size and movement (from .mls, per species)

- **Collision radius:**
  - `walkradius × 500` wu (`monster_loader.cpp:448`), default 500 when absent or 0 (`monster.h:380`).
  - Multiplied by the actor's scale through `Navigator::SetScale` (`navigator.cpp:1133-1147`), which
    also scales height offsets and `movement_scale`.
  - Scale is `scalefactor`, or a uniform roll within `scalerange*` (`monster_loader.cpp:440-445,530-534`).
- **Alternate radius:** `fightradius` gives the combat spacing and broad-phase radius
  (`navigator.cpp:429-433`).
- **Heights:** `targetheight` is the aim point (at least 250 wu, `monster_loader.cpp:450-451`).
  `cameraheight` is used for the camera.
- **Speed:** `movementrate` × 500 wu/s × scale, i.e. authored m/s. Turn rate is
  `rotationrate × 2π` (`monster_loader.cpp:501-503`).
- 46 distinct `walkradius` values exist (0.25-16). The full histogram is in `monsters_db.out`.
  Effective radius (walkradius × mid-scale, in authored metres; × 0.833 for your /600 units):

| Bucket | Species | DB records | Largest r at max roll | Examples |
|---|---|---|---|---|
| ≤ 0.6 m | 10 | 41 | 0.70 | humanoid females, digithead, hellazoid |
| ≤ 1.0 m | 61 | 150 | 1.28 | most humanoids (savant_trooper 0.84), mosquito, baruta fish |
| ≤ 1.6 m | 74 | 254 | 1.95 | acid_vine, ants, apus, bats |
| ≤ 3.0 m | 35 | 100 | 3.45 | crab, crocodile, djinn, ant_queen |
| > 3.0 m | 14 | 39 | 32 | bat_giant 13, elemental_fire_large 3.3, tentacle_boss 5.3, nessie 32 |

- **Movement class** (`monster_loader.cpp:505-514`, precedence flies > swims > quadruped > crawls):

| Class | Species | Records |
|---|---|---|
| GROUNDED | 133 | 389 |
| CRAWLING | 24 | 70 |
| QUADRUPED | 22 | 68 |
| FLYING | 9 | 44 |
| SWIMMING | 6 | 13 |

- No burrowing class exists.
- Plants (acid_vine, blood_vine) are QUADRUPED walkers with `movementrate` 2.84, not stationary.
- "Stationary" is a per-instance runtime state, not a species class: `fMotionless`, and
  `StartMonsterCycle` refuses non-idle cycles for motionless monsters (`MonsterManager.cpp:1074-1083`).
- What the classes change (`navigator.cpp:395-422`, `:1401-1410`, `:1529-1535`, `:1784-1788`):
  - FLYING, SWIMMING and QUADRUPED enable pitch.
  - CRAWLING enables pitch and roll (the body follows slope).
  - FLYING and SWIMMING pitch toward their target and use a 0.9 minimum walk-animation rate.
  - SWIMMING additionally scales the turn step (`navigator.cpp:1489`).

### Flying verdict: you are right, nothing truly flies

- **Flying species:** bat, bat_vampire, dark_savant_orb(_slow), flying_serpent, giant_mosquito,
  screaming_head, tree_sprite, wasp.
- **Swimming species:** baruta_fish(_slow), manta_ray, psi_shark, snapper, squid.
- **Hovering but GROUNDED:** flying_serpent_giant, morax, mook_screg_hologram, nessie.
- Evidence:
  1. `Navigator::AdjustPosition` settles **every** navigator except PATH_ONLY onto the ground probe
     (`settleToGround`, with gravity when above it) every move (`navigator.cpp:740-809`). It has no
     FLYING exemption.
  2. Altitude is a cosmetic render offset. `vertical_offset = hover_base + sin(phase) ×
     bob_amplitude` is rolled from `hoverrange*`/`bobrange*` (×500) per actor
     (`monster_loader.cpp:547-562`) and animated at 0.25 cycles/s (`navigator.cpp:1604-1615`).
     It is added only to the drawn model, lights and target heights (`cycle_player.cpp:668,686`,
     `monster.cpp:1257-1258`).
  3. Most of the visible height is **baked into the mesh**. `wasp_idle` vertices span y = 388..2336 wu
     above the actor origin, while hover adds only 100-300 wu.
  4. Pathing and AI never read `navigation_mode`. The only readers are navigator.cpp, and
     `special_movement` only affects the walk→idle transition test (`monster.cpp:553`).
- So flyers use the same ground navigation as walkers, with a ground-snapped origin. Treat them as
  ground agents of their radius, plus a visual Y offset and pitch.

## 3. How retail places monsters

Most of this section comes from a code-reading pass. I spot-checked `level_loader.cpp:549-575` and
`encounters.cpp:313-330`.

- **Fixed placements are in the level's `.STS` status file.**
  - `Levels\<folder>\<name>.STS` uses the save-chunk format (`src/world/level_session.cpp:523`,
    `src/wiz8/local_code/LoadSaveGame.cpp:1236`).
  - The MONS chunk holds monster groups (0x12B-byte images: species id, centre, leader, home) and
    per-monster 0x425-byte `MonsterInfo` images (species, position, yaw), plus optional
    script / patrol / orders sections (`LoadSaveGame.cpp:479-560,609-676,764-779`;
    `include/wiz8/retail_save_records.h:40-127`).
  - Each monster is individually positioned; there is no count or dice.
- **Random encounters come from generators** (the MONG chunk of the `.STS`, `src/world/encounters.cpp:504-529,653-685`).
  - A generator record is: a name, flags, chance, interval, **vec3 position**, an encounter-table index, and timer/scale.
  - The species is rolled from `EncounterTables.dbs` by rarity, ±50% party level, and day/night
    (`encounters.cpp:115-176`; `loaded_game_data.cpp:258-282`).
  - Group size comes from `group_size_dice`, with up to 2 companion groups (`encounters.cpp:182-308`).
  - The spawn point is always the generator position. A spawn requires the camera to be
    35 000-200 000 wu away, no line of sight, and an empty ±5000 box (`encounters.cpp:313-366`).
  - The budget comes from `levels.dbs` +0x3C..0x50.
- **`.pvl` monster paths:** `ReadMonsterPaths` (`level_loader.cpp:549-597`).
  - Each record is a count plus a 20-byte numeric species id, with optional `:` flags
    (shown/active/animated/looping), followed by a LoadPathAI path.
  - Each record creates a one-member group that follows the path.
- **Formation:** members scatter around the leader at 2 × leader radius, facing the camera yaw
  (`src/wiz8/local_code/MonsterGroup.cpp:62-140,688-777`). There are at most 9 per group.
  Patrols come from `.msf` orders (`PATROL dist var`, `POINTPATROL`, …,
  `src/entities/monster_script.cpp:648-699`) around the group's home point.
- For a Bevy proof:
  - Spawn from a generator-like point.
  - Snap to the navmesh.
  - Scatter members on the navmesh at 2r spacing.
