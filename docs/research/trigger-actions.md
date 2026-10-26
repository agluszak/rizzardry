# Retail trigger actions: behavioural spec (for the Monastery 2 bells)

Sources. Unless stated otherwise, every path is relative to `whizzardry-8/`. `TA` = `src/entities/trigger_actions.cpp`,
`TIO` = `src/entities/trigger_io.cpp`, `PR` = `src/entities/prop.cpp`, `TH` = `include/wiz8/entities/trigger.h`.
The original action catalogue below rests on Whizzardry's reconstruction; the activation-identity
section additionally checks the pinned decomp source.
Level data comes from `wizardry-prepared/levels/monastery/monastery2.pvl`. I decoded it with throwaway scripts that
re-implement `World::loadTrigger` type 4 (TIO:785-1002) and the prop header (PR:1034-1151).

## Activation identity (verified against decomp)

At decomp revision `075cbd0b853c4670e94f6ae4fe48d53b52b5e819`,
`engine_code/Trigger.cpp` (`CreateAndLoadLevelTrigger`, lines 1348–1370)
allocates each non-ambient record independently. `engine_code/Prop.cpp`
(`LoadProp`, lines 1691–1724) binds that record to its own prop; picking
uses that pointer directly (lines 363–380), not the trigger's name.
Equal names therefore do not imply shared activation state or animation.

`engine_code/ReadLevel.cpp` loads props, bitmaps, then standalone triggers
(lines 1077–1079, 1102–1106). Named recipients select the latest registered
match: `surrender/type_registry.cpp` prepends entries (lines 73–90) and
returns the first matching entry (lines 123–143). The importer preserves
record identity for prop bindings and uses names only for named links.
Prop-backed devices are colocated with their prop; neither needs a runtime
reference back to itself. Collision-door binding is a separate relationship.

Sound uses an explicitly positioned trigger first, otherwise its own prop
(`Trigger.cpp`, lines 1025–1057). Rizzardry still approximates a prop's sound
origin by its rest centre instead of retail's current-frame bounds centre.
No retail asset survey was available for this check; these conclusions are
from the recovered source, not a claim about duplicate-name frequency.

---

## 0. Data model that the Rust decoder currently drops

`src/retail/trigger.rs::read_item_trigger` skips most of the fields below. Several of them are needed for the bells.

| Type-4 field (TIO) | Meaning | Bell relevance |
|---|---|---|
| `packed_flags` (811, 853-859) | bit0 → **kind = item** (prop-backed; otherwise kind = "prop", meaning invisible/positioned). `searchable==1` also forces item kind. bit1 → `CAN_RUN_LINKED` (actually "show action message"; TH:199-201). bit2 → `ONCE`. bit3 → `EXCLUSIVE`. bit4 → `REACTIVATE_LINKED`. bit5 → 0x400000 (unknown). | yes |
| `enabled` (812, 869) | `W8_TRIGGER_ENABLED`. Gates **player picking** and proximity firing. It does **not** gate `Run` itself. | yes |
| `fire_linked` (813, 870) | `FIRE_LINKED`: fire recipients on commit. | yes |
| `link_on_deactivate` (814, 871) | Fire recipients when the animation ends instead of on commit. | no (all 0) |
| `keep_on_finish` (815, 872), `alt_toggles` (817, 876) | see §1.3 | no |
| `initial/alternate/fallback_action` (818-820, 865-867, 877) | `HAS_ALTERNATE` is set iff alternate != 0 | yes |
| `required_item` text (823, 864) | `atoi` → `required_item_id` (−1 if empty) | no |
| `consume_item`, `animate_action`, `sound` (824-830, 874-875) | `action_data = "data\sound\" + sound`. `ANIMATE_ACTION` enables the sound. | dial sound |
| `data[3]` (836-838) | `m_lData1..3`, meaning depends on the action (§2) | shake |
| `init_locvar` (840, 904-908) | if 0 or 1, ensures location var `<state_to_modify>` = value | minor |
| `flag0`, `action_data_mode`, `sound_volume` (841-845, 921) | alternate-sound selection (§4) | no |
| `state_count`, `animate_states`, `cycle_bounce` (879-902) | action 8 only | no |
| `required_states` (888, 893-895) | comma list of location vars, **all** must be non-zero (§3) | **yes** |
| `state_to_modify`, `state_mod_mode` (889-901) | var to set/clear/toggle on commit. Also the prefix of action 0x40's state names. | **yes** |
| `range_min/max` (911-917) | ×500 → retail units | pick range |
| `inline_action_data` (913) | item-table name for container props (action 3) | no |
| `alt_sound` (918-922) | `alternate_action_data` | no |

The prop record also has an **animation slot table** at version > 7 (PR:1134-1151). It holds `(frame:u16, tag:u16)` pairs,
and `src/retail/prop.rs:198` currently skips it. Action 0x40 needs it. So does `FindCurrentAnimationSlot`, which the bell
script calls.

Two more fields matter for animation. Each prop animation header carries `animation_playing, frame_method, behaviour, cycle`
(`src/entities/animation.cpp:45-53`; enums in `include/wiz8/entities/animation_types.h`). Behaviour 1 = PLAY_ONCE, 2 = REPEAT.
Frame method 1 = WRAP, 2 = PING_PONG.

---

## 1. Activation pipeline

### 1.1 Player use (click)
1. **Pick** (`src/world/world.cpp:336-368`). The prop must be `active`, and its trigger must satisfy all of these:
   - `ENABLED`.
   - not (`ONCE` and `FIRED`).
   - not in combat, unless it is a closed door.
   - the distance from the centre of the current frame's bounds to `party.eye()` is in `[range_min, range_max]`.
   The highlight test `IsTriggerInView` uses the same rules (PR:1007-1020). A prop that fails still occludes what is
   behind it.
2. **Click** → `Prop::activate` (`src/wiz8/local_screens/MainGameScreen.cpp:4974`, PR:1376-1381):
   `g_trigger_feedback = false; trigger->Run(-1); PrintNothingHappenedOrSpecialItemRequired()`.
   If nothing set feedback, the notice is `strings[0x964]` ("nothing happened"), or `strings[0x96b]` when an item is
   required (TA:939-949). Feedback is set at `commit_action` (TA:1854-1857) and on lock/trap UI.

### 1.2 `Trigger::Run(source)` (TA:979-1859)
1. `SelectAction()` (TA:1879-2011). If it returns false, stop.
2. If a level script installed `activation_callback` and it returns false, stop (TA:989-991).
   The callback runs **before** the action.
3. Dispatch on the selected `action`, in this order:
   - the prop-kind-only switch (TA:993-1111);
   - then the item-kind-only switch (TA:1113-1266);
   - then the common switch (TA:1268-1852).

   A `return` in any switch skips the commit. `break`/`goto commit_action` commits.
4. `commit_action` (TA:1854-1858) sets feedback, then calls `CommitActionResult(apply_state_changes)` (TA:757-818):
   1. `UpdateActionAnimation()`: plays the action sound (§4).
   2. **Recipients**, if `FIRE_LINKED` and not `LINK_ON_DEACTIVATE`. Each name in list order is looked up
      (case-insensitive; the newest trigger with that name wins, TA:105-111) and **synchronously** `Run(m_lData1)`.
      The caller's own `RUNNING` bit is forced on during each call and then restored (TA:762-773).
   3. `state_to_modify` (when `apply_state_changes`): mode 1 → 1, mode 2 → 0, mode 3 → toggle (TA:775-787).
   4. When `CAN_RUN_LINKED` (= packed bit1) is set, show a message from `Data\Messages\<levelfolder>.msg`
      (TA:789-814). The message id is `m_lData1`, `m_lData2` or `m_lData3`, chosen by `action_state` = initial,
      alternate or fallback; −1 means no message.
   5. `flags |= FIRED` (TA:817).

### 1.3 `SelectAction` (TA:1879-2011)
Checked in order:
- In combat, a trigger whose door is open fails (1885-1887).
- It fails if any of the following holds (1889-1893): `RUNNING` is set and the action is not 0x39; `ON` is clear; or
  `ONCE` and `FIRED` are both set.
  `ON` is set at construction (TA:618, TIO:526). Scripts and actions 0x2d/0x2e/0x2f can clear it.
- **required_states**: if any listed variable is 0, choose `fallback_action` (state = fallback) (1895-1904).
- When there is no door payload: `required_item_id >= 0` → the held item must match, or the fallback is chosen; the
  item is consumed when `CONSUME_ITEM` is set (1906-1921).
  When there is a door payload: block on a playing animation or busy recipients (`CanRunLinkedTriggers`), handle the key
  and `linked_trigger` (1922-1976).
- Pick/trap locks open their UI and fail (1978-1990).
- With no fallback chosen: if `USE_ALTERNATE` is clear, choose `initial`; `HAS_ALTERNATE` then sets `USE_ALTERNATE`.
  Otherwise choose `alternate`; `ALTERNATE_TOGGLES` then clears it (1992-2005).
  The result: with toggles the uses go initial, alt, initial, alt, …. Without toggles it is initial once, then alternate
  forever.
- With a fallback chosen and `fallback_action == 0`, return false (2006-2008). **This is how `required_states` gates a
  trigger off.**

Our `use_prop` (`rizzardry/src/game/world/props.rs:60-64`) chooses the alternate whenever `uses > 0`. That is right only
when `ALTERNATE_TOGGLES` is clear.

### 1.4 When `RUNNING` clears
- **Item kind** (prop-backed): when a PLAY_ONCE animation reaches its end, `AdvanceAnimationValue` calls
  `trigger->RunLinkedTriggers()` (PR:501-526). That clears `RUNNING` and, if `FIRE_LINKED` and `LINK_ON_DEACTIVATE` are
  both set, runs each recipient with `Run(-1)` (TA:547-560).
- **Prop kind**: `FinishAction()` (TA:658-755). It runs when the party leaves a positioned trigger's range
  (TA:248-251), and when a shake ends (TA:320-324).
  Its other effects:
  - When `KEEP_ON_FINISH` is clear, it undoes 4 (lights) and 0x22 (environment).
  - It cascades `FinishAction` to recipients under `FIRE_LINKED`.
  - It re-runs recipients when `REACTIVATE_LINKED` is set and the trigger was running.

### 1.5 Proximity triggers (`UpdateWorldTriggers`, TA:217-265)
Every step it considers triggers that are prop kind, `ENABLED`, `POSITIONED` and not `PLANE`:
- Inside `[min, max)` and not running → `Run(-1)`. When `EXCLUSIVE` is set, only one such trigger fires per step.
- Beyond max while running → `FinishAction`.

Step order (`src/world/world.cpp:105-129`): triggers, then the level script's masters, …, then
`UpdateTimedTriggerEvents`, then `camera_shakes.advance`.

### 1.6 Timed actions (`TriggerEvent`, TA:267-538)
- There is at most one live timed action per trigger. Adding a new one marks the old one completed (TA:274-282).
- Shake events take their own path (TA:438-441).
- Every other event fires only once its timer progress is > 1 **and no monster or party member is within 5000 units of
  the trigger or prop** (TA:451). That is the "don't close the door on someone" rule, and it applies to *all* delayed
  actions, including the 0x3f delayed relay.
- Delayed effects per action (TA:455-533):
  - 0x23: 1d6+2 damage to the party.
  - 0x25-0x2b: recharge.
  - 0x0c: missile.
  - 0x3f: `Run(-1)` the trigger named by the full recipient text.
  - 0x47: particles off.
  - 2: re-run (auto-close the door).

---

## 2. Action codes

The "kind" column says which switch handles the code: P = prop kind only, I = item kind only, C = common. `d1..d3` are
`m_lData1..3`. "Toggle anim" means `SetRepresentationActive(!playing, true)` (§2.1).

| Code | Kind | Effect | data | Commits? | Cite |
|---|---|---|---|---|---|
| 0x00 | P/C | nothing (commit only: sound, recipients, message, state) | message ids (with packed bit1) | yes | TA:995-996, 1302-1304 |
| 0x01 | I | door open: needs `state_index==0` and anim idle; key check; anim to end; state 1; `RUNNING`; door.open. Optional auto-close. | d1 = auto-close s (<0 → 10 s, 0 none) | yes | TA:1115-1156 |
| 0x02 | I | door toggle (anim idle); state ^= 1; door.open = state | — | yes | TA:1158-1178 |
| 0x03 | C | **toggle anim**. Item kind needs a prop. With `inline_action_data` it acts as a container instead (loot table, gold, item picker). | — | yes (fails if no prop) | TA:1332-1425 |
| 0x04/0x30/0x31 | C | lights in recipients: toggle / on / off. Fails if none found. | — | yes | TA:1269-1300 |
| 0x08 | I | multi-state cycle (`state_count`, `cycle_bounce`); with `ANIMATE_STATES` it drives paths or a subcycle | — | yes | TA:1211-1247 |
| 0x09 | P | play the camera path named by the recipient text | — | yes | TA:998-1004 |
| 0x0a | P | camera path `Camera0N` chosen from `Switch0..6` vars (one level's puzzle) | — | yes | TA:1006-1060 |
| 0x0b | C | wake the monster group `atoi(recipients)` | — | no | TA:1504-1506, 958-977 |
| 0x0c | C | missile: d2==0 fires once; d2≠0 repeats every \|d2\| ms (d2<0 starts at load) | d1 missile, d2 period | yes | TA:1638-1674, TIO:981-996 |
| 0x0e | P | random party speech when recipient == "party" | — | yes | TA:1062-1070 |
| **0x0f** | C | **relay**: for each recipient, `Run(d1)`. Fails if none found. | d1 = source passed on | yes → **recipients run again** if `FIRE_LINKED` (§2.2) | TA:1448-1467 |
| 0x10 | C | camera sway start (source>0) or stop | — | no | TA:1469-1472 |
| 0x11 | P | set `RUNNING` (string payload, TIO:665-667) | — | yes | TA:1072-1077 |
| 0x22 | P | environment intensity to 0; restored by FinishAction | — | yes | TA:1079-1085, 722-728 |
| 0x23 | P | damage zone: 1d6+2 every 0.5 s | — | yes | TA:1087-1099, 456-462 |
| 0x24 | C | party health change by dice | d1 count (def 1), d2 sides (def 6), d3 bonus (def 2) | yes | TA:1483-1502 |
| 0x25-0x2b | C | fountains: stamina, heal, SP (`fountain_magic.wav`), Hex, Sleep, Noxious, Heal All | d1 uses (−1 = ∞), d2 recharge (×720 calendar), d3 power | yes | TA:1581-1636 |
| 0x2c | I | toggle anim with key check | — | yes | TA:1180-1209 |
| 0x2d/0x2e/0x2f | C | recipients' `ON` set / clear / toggle | — | yes | TA:1306-1330 |
| 0x32/0x33 | C | start anim if idle / stop if playing | — | yes | TA:1427-1446 |
| 0x34 / 0x4b | P / C | teleport to recipient / to a random recipient | d1 = transition arg | yes | TA:1101-1106, 1843-1848, 824-890 |
| 0x36 | C | play sound only (alternate sound in fallback mode) | — | **no** | TA:1474-1481 |
| 0x37 | I | `SelectAnimationSlot(source)`, then play to that slot's frame. With source −1 the tag is 0xFF (a reconstruction quirk). | d1 = default tag | yes | TA:1249-1261 |
| 0x38 | C | nothing | — | yes | TA:1302-1304 |
| **0x39** | C | **camera shake** (§4.3). Bypasses the `RUNNING` check. | d1 intensity, d2 ms, d3 ±duration ms | yes | TA:1508-1550 |
| 0x3a/0x3b/0x3c | C | missile hit with `source == d1`: deactivate / stop / toggle anim | d1 = missile index | yes | TA:1552-1579, PR:972-979 |
| 0x3d/0x3e | C | prop `active` on / off | — | yes | TA:1676-1688 |
| 0x3f | C | delayed run of the trigger named by the recipient text, after d1 ms (subject to the 5000-unit rule) | d1 ms | yes | TA:1690-1708, 507-511 |
| **0x40** | C | **dial**: advance to the next animation segment (§2.3) | — | yes, but no state-mod | TA:1710-1731 |
| 0x41/0x42/0x43 | C | named ambient sound: start at camera / stop / toggle | — | yes | TA:1733-1752, `src/audio/AmbientSound.cpp:547-573` |
| 0x44/0x45/0x46 | C | named particles on / off / toggle | — | yes | TA:1754-1779 |
| 0x47 | C | particles on for d1 ms (default 10 s) | d1 | yes | TA:1781-1806 |
| 0x48 / 0x49 / 0x4a | C | set prop subcycle to d1 / own anim speed d1 / recipient props' speed d1 | d1 | yes | TA:1808-1841 |
| anything else | C | `return`: no commit | | | TA:1850-1851 |

### 2.1 Driving a PLAY_ONCE prop animation
Every prop in the bell puzzle has behaviour PLAY_ONCE.

**Load** (PR:1179-1191, 1242-1252, 1280-1285). The prop starts idle with `first_frame=0`, `last_frame=frames-1` and
`subcycle=0`. `frame_direction` starts as `REVERSE_COMPLETE` for PING_PONG and `FORWARD_COMPLETE` otherwise.

**`SetRepresentationActive(1, true)`** (PR:342-367) picks the direction:
- Forward from `first_frame` when either (direction = `FORWARD_COMPLETE` and method ≠ PING_PONG) or
  (direction ≠ `FORWARD_COMPLETE` and method = PING_PONG).
- Otherwise reverse from `last_frame`.

The result for repeated uses:

| Frame method | Successive uses |
|---|---|
| WRAP | play 0→end every time (restarts from 0) |
| PING_PONG | 0→end, then end→0, then 0→end, … (alternates) |

**`SetRepresentationActive(1, false)`** starts playback without changing the direction or the subcycle.

**Advance** (`UpdatePropAnimation`, PR:376-453, `AdvanceAnimationValue`, PR:489-606).
- Each frame lasts `1/fps` seconds.
- When the run reaches the endpoint it stops, sets `*_COMPLETE`, and calls `RunLinkedTriggers()`, which clears
  `RUNNING` (§1.4).
- PLAY_ONCE ignores ping-pong and wrap *within* a run (PR:501-526).

### 2.2 Relay double-fire
Action 0x0f runs every recipient once (TA:1453-1462). Then `CommitActionResult` runs them **again** when `FIRE_LINKED` is
set (TA:762-773). All three bell relays have `fire_linked = 1`.

The second call is normally a no-op, because the first call left the recipient `RUNNING`. That does not hold for:
- a 0x39 recipient, which bypasses `RUNNING`, so its timer and countdown simply restart (TA:1531-1538);
- an action that does not set `RUNNING`.

Preserve this behaviour, or prove that dropping it is equivalent.

### 2.3 Action 0x40 (dial) and the animation slots
- **Load** (PR:1218-1227). For an initial action of 0x40, the game ensures location variables `<STATE><i>` for
  `i in 0..slots.len()`, with `<STATE>0 = 1` and the rest 0. `ensureLocationVariable` creates a variable only if it is
  missing (`src/wiz8/story.cpp:51-58`).
- **Run** (TA:1710-1731):
  1. Set `<STATE><old state_index>` to 0.
  2. `state_index = prop.AdvanceAnimationSegment()`.
  3. `SetRepresentationActive(1, false)`.
  4. Set `<STATE><new>` to 1.

  `apply_state_changes = false`, so the generic state-mod step does not run.
- **`AdvanceAnimationSegment`** (PR:297-326):
  - Returns 0 when there are fewer than 3 slots.
  - Otherwise it finds the slot whose `frame == first_frame` and moves to the next segment, wrapping from `count-2` to
    0. Segment `k` plays `slots[k].frame → slots[k+1].frame` forward, starting from `subcycle = slots[k].frame`. The
    function returns `k`.
  - So N slots make N−1 segments, and the last slot is only an end frame.
- It does **not** check or set `RUNNING`, so the dial can be turned again mid-animation. The new segment then starts
  immediately; this is an inference from the code.
- **`FindCurrentAnimationSlot`** (PR:285-295) returns the index of the slot whose `frame == first_frame`, which equals
  the current segment.
- **`SelectAnimationSlot(tag)`** (PR:251-280), used by 0x37: finds the slot with this `tag` and plays from the current
  `subcycle` to `slot.frame`, forward or reverse.

---

## 3. Gates, one-shots, persistence

**Gates on use:**
- Pick distance `[range_min, range_max]` from the prop's bounds centre to the eye (world.cpp:361-365).
- `ENABLED` controls only picking and proximity. Scripts can still `Run` a trigger that is not enabled, and so can
  recipients: **every bell-chain recipient has `enabled = 0`**.
- `ON` controls every `Run` (TA:1889). Clearing it is how retail "disables after use"; the Monastery 2 script does
  exactly that (§5). A disabled-but-enabled prop stays pickable, and clicking it gives the "nothing happened" notice
  (§1.1).
- `ONCE` + `FIRED`: never runs again, and is no longer pickable (TA:1890, world.cpp:358).
- `RUNNING` (§1.4); `required_states` (§1.3); required items and keys; locks and traps (TA:1978-1990).

**Persisted** (`Trigger::Capture`/`Restore`, TIO:160-203, 279-323):
- Trigger: `flags` (ON, FIRED, RUNNING, USE_ALTERNATE, …), `state_index`, `state_direction`, `action`, `action_state`,
  `required_item_id`, door state with any pending auto-close progress, loot, seed, gold, `uses_remaining`.
- Prop animation state: `subcycle`, `first_frame`, `last_frame`, `frame_direction`, `animation_playing`, `active`,
  keyed by prop name (PR:1302-1328). `first_frame` is what makes the dial state survive a save.
- Location variables (`RING0..3`) live in the story save.
- Lock progress (TIO:65-151).

---

## 4. Sounds, shakes, breakables

### 4.1 Trigger sound
- **Where the sound comes from:**
  - Type 4 reads the per-trigger sound from the record: `action_data = "data\sound\<sound>"` (TIO:827-830), with an
    optional `alternate_action_data` (TIO:918-922).
  - Type 1 has its own sound field (TIO:557-561).
  - There is no lookup table by action or by name, except the fixed `fountain_magic.wav` for 0x25-0x27
    (TA:1591-1598) and script-played sounds.
- **When it plays:** at commit, via `UpdateActionAnimation` (TA:629-656), and only if `ANIMATE_ACTION` (`animate_action`
  byte) or `ALTERNATE_ACTION` (`flag0`) is set.
- **Which sound** (the `action_data_mode` field):

  | Mode | Sound played |
  |---|---|
  | 0 with `ALTERNATE_ACTION` | alternates between primary and alternate on each commit |
  | 1 | alternate when `action == alternate_action` |
  | 2 | alternate when `action == fallback_action` |

- **Playback** (`PlayActionSound`, TA:405-431): `volume = sound_volume`, where 0 means 100.
  - A prop-backed trigger plays 3D at the centre of the prop's frame bounds.
  - A positioned trigger plays 3D at its position.
  - Otherwise the sound plays 2D.
- **Files:** retail paths are case-insensitive. The extracted data has `wizardry-prepared/data/sound/<lowercased path>`,
  for example `data/sound/ambients/lock open small.wav`.

### 4.2 Breakables
Retail has no special "breakable" action. `GLASS1..7` are ordinary action-0x03 props:
- Each has a 3-frame PING_PONG PLAY_ONCE animation at 60 fps, played 0→2 once (≈33 ms).
- Their triggers have **no sound** (`sound ''`, `animate_action 0`), so breaking is silent from data. `glass impact
  02.wav` is used only by `micro_door1`.
- `enabled = 0`, so the player cannot click them.
- Some carry `PROP_COLLIDABLE` (`glass_payne_01`, `Box13`, `glass_payne_05`), and their collision follows the animation.

### 4.3 Camera shake, action 0x39 (TA:1508-1550, 301-329; `src/render/effects/camera_shake.cpp`)
- **The first Run creates a timed event:**
  - `intensity = d1` (−1 → 800);
  - `timer = d2` ms (−1 → 70 ms);
  - if `d3 != -1`, a countdown of `|d3|` ms, with `d3 < 0` meaning fade-out.
  - A prop-kind trigger also sets `RUNNING`.
  - A non-prop trigger with d3 == −1 throws.
- **Later Runs** restart the timer and countdown (TA:1531-1538).
- **Each step** `UpdateShake` builds the effect once, as `ShakeDefinition(duration = countdown or 1 s, intensity =
  max(d1/250, 1))`, non-continuous, with `fade_out` when reversed.
- **When it ends**, `FinishAction` runs (it clears `RUNNING` and completes the event; TA:696-702).
- **Output:** `ShakeSystem::advance` sums the active shakes and applies a per-axis random offset of ±`min(sum, 1)·250`
  retail units, scaled by `1 − progress` for fade-out (camera_shake.cpp:60-87, 113-125).
- It uses `PresentationRng`, not gameplay randomness.
- **UNCERTAIN, two points:**
  - As reconstructed, `intensity` is clamped with `max(…, 1)` and then `min(…, 1)`. That makes the authored `d1` (90)
    irrelevant: the amplitude is always 250 units. It looks like a min/max inversion in the reconstruction.
  - The `d2` timer (40 ms) is never read on the shake path. Its retail meaning is probably the jitter period, but that
    is unverified.

---

## 5. Monastery 2 bell mechanism: concrete trace

### 5.1 Data (decoded from `monastery2.pvl`)
In the table, `item` and `prop` are the trigger kinds of §0. Fps and frame counts are each prop's own (animation header).
Every trigger range below is `[0, 5000]` retail units.

| Trigger (prop) | Kind | en | fire_linked | packed | Action | Recipients | Gate | Sound | Anim |
|---|---|---|---|---|---|---|---|---|---|
| BELLRINGSWITCH (`dial1`) | item | 1 | 0 | 1 | 0x40 | — | — | `ambients\lock open small.wav` (anim_action 1, vol 100); state `Ring`, mode 1 | 21 fr, 16 fps, PING_PONG, PLAY_ONCE; **slots (0,0),(1,1),(10,2),(20,3)** |
| BELL_BUTTON (`bell_dial_button`) | item | 1 | 1 | 1 | 0x03 | BELLTRIGGER01, BELLTRIGGERC, BELL_ACTIVATE_2, BELL_ACTIVATE_3 | script callback (§5.2) | none | 12 fr, 16 fps, PING_PONG |
| BELLTRIGGER01 (`belltrigger01`) | item | 0 | 1 | 5 (= item + ONCE) | 0x00 | GLASS1..7, MONACAMSHAKE | `Ring2` | none | 6 fr, 3 fps (never played) |
| BELLTRIGGERC (level trigger, positioned) | prop | 0 | 1 | 0 | 0x0f | BELL_1 | `Ring0` | none | — |
| BELL_ACTIVATE_2 (level) | prop | 0 | 1 | 0 | 0x0f | BELL_2 | `Ring1` | none | — |
| BELL_ACTIVATE_3 (level) | prop | 0 | 1 | 0 | 0x0f | BELL_3, MONACAMSHAKE | `Ring2` | none | — |
| BELL_1/2/3 (`Bell_1..3`) | item | 0 | 0 | 1 | 0x03 | — | — | none | 37 fr, 10 fps, **WRAP**, PLAY_ONCE |
| GLASS1..7 | item | 0 | 0 | 1 | 0x03 | — | — | none | 3 fr, 60 fps, PING_PONG |
| MONACAMSHAKE (`MonaCamShakeBox`) | prop (packed 0, attached to a prop) | 0 | 0 | 0 | 0x39 | — | — | none | data (90, 40, −2400) |

- All fallback and alternate actions are 0.
- No other trigger names these as recipients, except `GLOWBALL`, which also fires MONACAMSHAKE (microdial puzzle).

### 5.2 Level script (`src/wiz8/level_specific_code/Monastery2.cpp`)
- On level enter, the script installs a callback on `bell_button` (54-58; `bindTrigger` sets `activation_callback`,
  `src/wiz8/level_specific_code/MasterFunctionList.cpp:94-99`).
- **Every** button Run that passes `SelectAction` calls the callback first (64-85). It reads
  `slot = dial1.FindCurrentAnimationSlot()`:

  | Slot | Effect |
  |---|---|
  | 0 | play `Data\Sound\Ambients\Mon2Bell1.wav` (2D) |
  | 1 | play `Mon2Bell2.wav` |
  | 2 | play `Mon2Bell3.wav`, then **clear `ON`** on `bellringswitch` and `bell_button` (28-32) |

- It always returns true, so the button's own action still runs, including on the slot-2 press.
- The files are `wizardry-prepared/data/sound/ambients/mon2bell{1,2,3}.wav`. The bell sounds come **only** from the
  script; no record references them.
- Caveat: the file header (13-24) says neighbouring callback blocks were not recovered. Whether this callback is
  complete is unverified.

### 5.3 Dial states
- **Initial state:**
  - The load creates `RING0=1, RING1=0, RING2=0, RING3=0`. `RING`=0 is also created by `init_locvar` and is unused.
  - The dial sits at `first_frame=0`, `subcycle=0`, so its slot is 0.
  - The trigger's `state_index` is 0.
- **Each use of the dial** (if in range, `ON`, not `RUNNING`; 0x40 never sets `RUNNING`) plays `lock open small.wav`
  in 3D at the dial and advances the segment:

| Use | Segment, anim | state_index / slot | Vars after |
|---|---|---|---|
| start | — (frame 0) | 0 | RING0=1 |
| 1st | 1: frames 1→10 (9/16 s) | 1 | RING0=0, RING1=1 |
| 2nd | 2: frames 10→20 (10/16 s) | 2 | RING1=0, RING2=1 |
| 3rd | 0: snap to frame 0, then 0→1 (1/16 s) | 0 | RING2=0, RING0=1 |
| … | cycles 1, 2, 0, … | | |

The `Ring%d` variables use `state_index`, and the script uses `FindCurrentAnimationSlot`. Both always equal the current
segment, so they agree. `RING3` is never set.

### 5.4 Pressing the button
Preconditions: in range, `ON`, and the button not `RUNNING`; the button stays `RUNNING` for 11/16 s ≈ 0.69 s after each
press.

1. `SelectAction`: initial action 0x03.
2. The **callback** plays `Mon2Bell<slot+1>.wav`. In slot 2 it also disables the dial and the button.
3. Action 0x03 toggles the button animation (0→11 on odd presses, 11→0 on even presses; PING_PONG, §2.1), `state_index ^= 1`,
   and sets `RUNNING`.
4. Commit: no sound. The recipients run in order with `Run(-1)`:
   1. **BELLTRIGGER01**:
      - If `RING2 == 0`, the fallback is 0 and it does nothing.
      - If `RING2 == 1` and not yet `FIRED`, action 0 commits and runs `GLASS1`…`GLASS7` in order (each plays 0→2), then
        `MONACAMSHAKE` (starts the shake). Then `FIRED` is set, and `ONCE` blocks it forever.
   2. **BELLTRIGGERC** (`RING0`): relay → `BELL_1.Run`. Bell 1 swings 0→36 (3.6 s) and is `RUNNING` until the swing
      ends. The second fire (§2.2) is a no-op.
   3. **BELL_ACTIVATE_2** (`RING1`): relay → `BELL_2` swings, the same way.
   4. **BELL_ACTIVATE_3** (`RING2`): relay → `BELL_3` swings, and `MONACAMSHAKE` runs again. If BELLTRIGGER01 already
      started the shake this press, this only restarts its timer. The second relay fire restarts it again.
5. The button gets `FIRED` (no `ONCE`, so that has no effect).

Only the relay whose `RINGn` is 1 fires, so exactly one bell swings per press.

**Re-pressing while a bell is still swinging** (button free again after 0.69 s) plays the script sound again. The bell
does **not** restart, because it is `RUNNING`. This follows from the code and has not been seen in game.

**Shake** (same press as glass and bell 3):
- `ShakeDefinition` lasts 2.4 s and fades out linearly.
- Its amplitude is 250 units as reconstructed (§4.3, uncertain).
- `RUNNING` stays set until the shake ends.

### 5.5 Outcomes by dial position
| Dial slot when pressed | What the player sees and hears | Persistent change |
|---|---|---|
| 0 (initial) | button moves, `mon2bell1.wav`, Bell_1 swings | none (repeatable) |
| 1 | `mon2bell2.wav`, Bell_2 swings | none (repeatable) |
| 2 | `mon2bell3.wav`, Bell_3 swings, all 7 glass panes break (silent), the camera shakes about 2.4 s with fade-out | BELLTRIGGER01 `FIRED` (ONCE); `ON` cleared on BELLRINGSWITCH and BELL_BUTTON |

### 5.6 Final state (after the slot-2 press)
- **Variables:** `RING2=1` (others 0).
- **Dial:** at segment 2 (frame 20), `ON` clear. Clicking it gives the "nothing happened" notice (TA:948); it stays
  pickable because `ENABLED` is still set.
- **Button:** `ON` clear, with the same click behaviour as the dial.
- **Bells:** Bell_3 ends `FORWARD_COMPLETE` at frame 36.
- **Glass:** GLASS1..7 at frame 2, `FORWARD_COMPLETE`. Their collision follows the final frame.
- **Triggers:** BELLTRIGGER01 is `ONCE`+`FIRED`. MONACAMSHAKE is idle again (`RUNNING` cleared).
- **Save:** all of the above, through the trigger flags, the prop animation state and the location variables (§3).

---

## 6. Other data mentioned in the brief

**COMPDELAY03/04, PHONSIGN, TRIGGER2910** (`Box98`, `Box24`, `Box40`, `Box88`):
- Each is item kind, packed 3 (item + `CAN_RUN_LINKED`), `enabled` 1, action 0, data `(17|17|36|15, −1, −1)`.
- Using one commits action 0 and shows message `d1` (17, 17, 36 or 15) from `Data\Messages\monastery.msg` (TA:789-814).
  The file is `wizardry-prepared/data/messages/monastery.msg`.
- So `data[0]` is a **message id here, not a delay**. "CompDelay" is only a name. I did not decode the message text.

**What `data[0]` means for each action:** see the "data" column in §2.
- Message id for any action when packed bit1 is set.
- Seconds for 0x01.
- Milliseconds for 0x0c (d2), 0x3f and 0x47.
- Missile index for 0x0c and 0x3a-0x3c.
- Intensity for 0x39.
- Use count for 0x25-0x2b.
- Dice count for 0x24.
- Frame or speed for 0x48-0x4a.
- Forwarded `source` for 0x0f, and for every commit-time recipient run (TA:769).

---

## 7. Uncertain / not found
- Everything in this spec comes from the Whizzardry reconstruction; I could not check it against the original decomp.
- The shake intensity clamp and the unused `d2` timer (§4.3).
- Whether the bell button's 12 frames are a full press-and-release or only the press: retail alternates the direction on
  every press (PING_PONG + PLAY_ONCE). I did not inspect the keyframes.
- Whether the frame-20 → frame-0 snap on the third dial turn looks seamless; that depends on the authored frames.
- `belltrigger01`'s own 6-frame animation is never started by any trigger. Its purpose is unknown; it may be a hidden
  helper.
- Packed bit5 (0x400000), type-4 `flag` (0x1000) and `searchable`: their meanings were not traced.
- The completeness of the Monastery 2 script (§5.2 caveat).
