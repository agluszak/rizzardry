# Implementation plan

## What this plan commits to

Build one Rust/Bevy application in small playable slices. The documentation
defines proposed behavior and ownership; it is not evidence that a dependency
set compiles, a retail asset renders correctly, or a reconstructed rule matches
the executable.

Use the [architecture](architecture.md) as the common ownership model. When a
source-backed edge case contradicts it, revise the relevant documents together.
Keep [revisions](revisions.md) as the short record of intentional differences
and remaining decisions.

No time estimate is useful until the first real-map and asset experiments
establish what Bevy handles directly.

## 1. Complete the real-world engine slice

**Playable result:** enter a representative retail room, traverse stairs,
open a door, use a scripted prop, observe one real animated actor, hear one
original voice line, and interact through a small native UI.

**Status (25 October 2026):** every playable result of this slice has been
shown on retail content; the gaps below remain. Done:

- **Files.** `retail::files` indexes an unmodified installation: base SLF
  libraries (more specific prefixes win), `patches/patch.NNN`, then loose
  files. It is the `retail://` Bevy asset source.
- **Levels.** `retail::pvl` decodes the compiled meshes and materials, and
  `retail::sections` the rest of every `.pvl` sequentially to its 72-byte
  trailer: lights, props, bitmaps, triggers (all four types, with door data),
  environment (fog and ambient ramps, view distance) and the environment
  offset. All 34 levels parse. Uncompiled `.lvl` worlds (skies) use the same
  section reader without sentinels.
- **Collision.** The level's `.oct` collision surfaces (solid flags; the
  pathfinding flag marks walkable floor) form the Avian collider and the
  navmesh input. Collidable props collide; non-collidable ones are sensors
  that can still be used.
- **Party.** One Avian body moved by Tnua on the fixed timestep, with retail's
  500-unit sphere, eye height, speeds, pitch limit and gravity. It climbs the
  upper monastery's `MO202` stairs. A new game enters at `MO100`.
- **Props and doors.** Props render with their animation paths; using a door
  plays its path, honours its lock data and opens or closes its navigation
  link. Teleport triggers move the party within the level or, naming
  another level's entrance (`MO102` → `MO204`), swap the level root and place
  the party at that entrance once the new level is ready.
- **Navigation.** Rerecast navmeshes are baked by Bevy's asset processor, one
  per walker size class (0.6, 1.0, 1.6, 3.0 m), and used by Landmass. Teleport
  and ladder triggers become animation links; doors are links only while open.
  All 34 levels × 4 classes validate.
- **Rendering.** Retail baked lighting (static light, sky light by phase,
  environment offset, emission), linear distance fog and view distance from
  the environment ramps, a game clock at 12× real time, `.ifl` texture
  animation, retail textures as shared assets (`<file>#image`, CPU mipmaps,
  alpha from the file's pixel format as in retail), the sky world on its own camera with time-of-day
  gradients, shaded scrolling clouds and the Sun and Moon on retail's orbit.
  Facing props turn with the camera. Slot 0-1 alpha materials blend; slot 2-3
  materials add (retail's packed shader `SRC_ALPHA, ONE`).
- **Monsters.** `.mls` species and `.mon` vertex-frame cycles decode (all
  1088 retail cycles). Each actor animates its own mesh copies, blending
  between frames without wrapping last-to-first as retail does; play-once
  cycles report completion. Walkers play WALK or IDLE.
- **Audio.** A voice line is an entity; `SpeechFinished` names it when its
  audio ends. Retail ships MP3 where scripts name WAV.
- **UI.** STCI images and bitmap fonts decode (all 1327 retail `.sti`). The
  automap is native Bevy UI from retail art: frame, buttons with picking
  hover/click, a plan of the floors, and notes pinned, typed
  in `Wiz_Text_Font` and erased, within retail's limits. Retail zoom and
  movement buttons provide three zoom steps, bounded panning and party
  recentering; arrow keys pan, and right-click zooms out. Geometry, markers,
  notes and picking use one aspect-preserving projection. Bounds currently
  cover all walkable geometry, not just explored cells. This native orthographic
  view uses full-span, half-span and 50-metre zoom; retail's terrain-dependent
  camera-height calculation is not reproduced. Numbered clip planes and
  `MapFilters.txt` supply the layer cuts; the layer buttons reveal the topmost
  walkable surface below each cut, including slopes crossing the cut. The map
  initially selects the party's floor band. Note labels and selection are
  confined to their layer; note markers and the party remain visible across
  layers, as in retail. The window backend displays the original tool cursors
  and hotspots.

Decisions made while doing this:

- Retail space is Y-up and left-handed. Bevy space mirrors Z, reverses
  triangle winding and uses retail's own scale of 500 units per metre
  (authored values are metres × 500), which puts the eye 1000 units (2 m)
  above the feet. The orientation was checked against the retail game.
- Retail velocities are per 0.1 s tick; converted constants cite their source.
- Nothing in retail truly flies: flying monsters snap to the ground, their
  altitude is a mesh offset plus a cosmetic bob, and they path like walkers.
  They use the surface navmeshes.
- Movement, navigation, prop animation and the clock step on the fixed
  timestep; look, camera aiming and presentation run per frame.

Known gaps, in rough order:

- Monsters take sky light while they can see the level's `SUN` light,
  fading as retail does, plus the level offset and nearby static lights; but
  those lights reach the whole actor by their falloff at its centre, not per
  vertex with N·L, and their runtime attenuation is not confirmed. Monsters
  use only the nearest detail level and ignore particles, sounds and skins.
- Elevators/lifts, random teleports and camera-path triggers are not
  implemented. Travel does not yet ask retail's "Enter <level>?" question,
  and arriving does not probe for the ground (the party settles).
- Map notes are not saved; the automap does not yet track explored cells or
  project notes onto a minimap. Only the map screen exists.
- The screenshot tool intermittently omits all or some unchanged UI artwork,
  even while map/note assertions pass. Complete map frames have also been
  captured; the cause is still unresolved.

### Next concrete implementation work

1. **Validate one complete authored prop interaction.** Use a monastery
   mechanism such as the Monastery 2 `bell_button`/`dial1` combination:
   source-backed pick/range gating, persistent trigger effect, prop frame
   changes, the correct sound and final disabled state. Extend the existing
   prop and door code instead of replacing it.
2. **Check the world features against retail:** authored door
   geometry/navigation while closing, two-level loading without spawning
   a second party, original entrance ground placement, fog/sky/light
   appearance and animation/contact ordering. The ability to bake all
   34 levels doesn't prove each location's traversal.

Outstanding work should follow the [Bevy-native feature decisions](bevy-features.md)
instead of creating another engine layer.

### Exit evidence

- An interaction completes from the *same* decoded trigger and moving prop
  that the player sees; closing/opening changes usable collision and pathing.
- A monster's actual retail animation plays without per-frame CPU mesh
  reuploads unless that is the simplest proven representation.
- A real voice plays and its specific completion is observable by its owner.
- The 3D viewport and native UI don't send the same click to two targets.
- Travel leaves one surviving party and no orphaned prior-level entities.
- The render result is compared against a known retail location, not only a
  flat demo map.

**Do not build yet:** a universal asset converter, interchangeable physics
backends, a scripting VM, a generic animation-action workflow engine, a
separate scene graph or an editor suite.

## 2. Make party and inventory state useful

**Playable result:** create a small party, inspect characters, move/stack/equip
items, place an item on the cursor, drop/pick it up, and save/load this settled
state.

Work:

- Establish typed content IDs, persistent ActorId/ItemInstanceId allocation and
  a derived runtime entity lookup.
- Implement the spatial Party entity, PartyRoster and PartyInventory resources,
  and actor-owned AdventurerState/Inventory.
- Give item moves, splitting, merging, consumption and overflow explicit
  results. Protect the held item from accidental loss.
- Separate item definition properties, identification knowledge, active
  equipped-curse restrictions and temporary removal permission.
- Add native save capture/restore and staged file writing now, while the state
  graph is small. Include RNG algorithm/version/state and ID allocator state
  from the first gameplay save.
- Keep importer/definition records and save records distinct from live
  components; do not build a mirrored mutable GameState.

**Exit evidence:** the item identity graph survives save/load without duplicates;
equipment changes recalculate the relevant stats; closing an inventory screen
does not destroy an owned item. The few tests should cover conservation and
actual rule boundaries, not merely constructor getters.

## 3. Complete one combat and magic slice

**Playable result:** a monster detects the party, a fight starts, the party
chooses attacks and an area bomb, and the encounter reaches victory or defeat.

Work:

- Establish directional perception, group disposition/provocation and actor
  engagement separately from the party Battle.
- Implement accepted choices, initiative and one executing action; AI produces
  choices through the same rule boundary as player input.
- Implement targeting, cast validation, accepted consumption, projectile/effect
  delivery, resistance/damage/conditions and immediate death consequences.
- Include the area-bomb walkthrough, fizzle/backfire consumption and a target
  moving after victim capture.
- Include the party's one scheduled movement action, its initiative and the
  resulting individual action availability.
- Add explicit round time advancement without also aging the same interval
  through the ordinary exploration clock.
- Keep gameplay target iteration and RNG order defined.

**Exit evidence:** the full [combat item flow](systems/combat-items-and-magic.md)
works, including invalidation before execution and delivery after the source
item disappears. Combat can end cleanly; rendering speed does not alter damage
or spend another item. No broad spell catalog is required before one difficult
effect path is correct.

## 4. Add persistent world interaction and character progression

**Playable result:** open a trapped chest, identify its contents, combine items,
trade with an NPC, then level up and retain those changes after a reload.

Work:

- Implement lock/trap sessions separately from persistent device state.
- Generate container loot at the established opening boundary; preserve
  deposits made before the first roll.
- Handle a sprung trap that remains armed through the shared magic pipeline.
- Implement merchant stock normalization, pricing, buy/sell and full-inventory
  overflow with the held item protected.
- Implement manual recipes, item knowledge and skill practice in the chosen
  source order.
- Implement the level-up allocation draft, profession changes, spell selection,
  derived-stat consequences and unusable-equipment handling.
- Add full-map notes and the deliberate minimap projection feature.

**Exit evidence:** an interrupted UI does not undo an accepted roll or debit;
reopening a chest does not reroll it; save/load does not duplicate stock or class
items. Characterize the chosen original crafting output-slot/RNG order against
representative inputs before calling the result faithful; retain the documented
Whizzardry differences for comparison.

## 5. Prove travel, RPC identity and elapsed time

**Playable result:** recruit a companion, travel between two maps, wait/rest,
return to the original map and dismiss/recruit the companion without duplicating
their identity or possessions.

Work:

- Add campaign-lived named actors and ordinary local actors that become dormant
  records. Unloaded named actors retain state but do not run world AI per frame.
- Implement explicit adventurer/world combat-profile conversion at recruitment
  and dismissal; do not carry HP and equipment behavior across roles by guesswork.
- Make map preparation and activation separate so a failed load leaves the
  current map usable. Preserve any separately committed RPC dismissal.
- Implement region warnings, delayed refusal/discomfort checks and saved deadlines.
- Add simulation-time accumulators, ordinary camp and incapacitation recovery,
  interruption/ambush, recovery fractions and finite/indefinite conditions.
- Apply the source's dormant catch-up rules and encounter-budget rules on revisit;
  do not replay thousands of missed AI frames.
- Add a delayed Antone/Ferro-style commission to prove off-map NPC stock,
  deadlines and once-only completion.

**Exit evidence:** one actor identity exists before and after every role/map/save
transition; condition recovery and commissions use the intended clock; a failed
travel preparation cannot duplicate returned gear or resurrect a dismissed NPC.

## 6. Complete authored content and continuing effects

**Playable result:** run the monastery scene, solve the Arnika spaceport puzzle
in either valid order, and use a continuing effect such as Hypnotic Lure outside
party combat.

Work:

- Recover the relevant retail dialogue/level data and verify exact entry
  predicates, item handling and fact order.
- Implement small concrete level modules and owned scene continuations.
- Bind each meaningful animation/voice completion to its identified request.
- Preserve the monastery's separate control-release and body-removal milestones.
- Model the spaceport's repair and discoveries independently.
- Ensure continuing effects can exist without Battle and restore only through
  their explicitly supported saved state.
- Extend content coverage after these ownership/timing cases work.

**Exit evidence:** replay/load does not respawn a scene actor, repeat a reward,
consume a second quest item or drop an outstanding consequence. An old animation
completion cannot advance a different scene.

## Working module layout

Start with ordinary source modules; the table indicates responsibility rather
than a commitment to create every file before its first feature.

| Path | Purpose |
| --- | --- |
| src/main.rs, src/app.rs | Entry point, plugin composition and startup |
| src/content/ | Typed definitions, IDs and content validation |
| src/retail/ | Archive/format decoding and retail-to-native conversion |
| src/game/characters/ | Stats, progression, active-role conditions and practice |
| src/game/time/ | Simulation clock, periodic delivery, pause policy and camp |
| src/game/party/ | Roster, formation and RPC transitions |
| src/game/inventory/ | Ownership, equipment, consumption and item knowledge |
| src/game/magic/ | Targeting, cast/effect execution and continuing effects |
| src/game/combat/ | Party Battle, action sequencing and combat rules |
| src/game/trade/ | Stock, pricing, exchanges and commissions |
| src/game/world/ | Local objects, movement, perception, AI, devices and level-specific behavior |
| src/game/story/ | Dialogue, facts, faction relationships and journal |
| src/presentation/ | Mesh/material/animation/audio/camera and map projection |
| src/ui/ | Screen-local widgets, focus, drafts and player intents |
| src/save/ | Explicit records, versioning, capture and restore |

Add a module or plugin when the corresponding slice needs it. A pure rule
function does not need a plugin, message type and scheduling stage of its own.

## How to verify behavior without copying the old architecture

For each implemented slice:

1. Record the small source paths and content fixtures that determine behavior.
   Treat a comment as a hypothesis when the function body disagrees.
2. Pick boundary cases from its system document: ownership transfer, rejected
   action, accepted cost, target invalidation, repeated interaction, save/load
   and interruption where applicable.
3. Compare concrete inputs/outcomes and random-draw order against the chosen
   reference. If decomp and Whizzardry differ, say which behavior is intended.
4. Use unit tests for meaningful pure rules and integration checks for lifecycle
   boundaries. Use a real rendered scene where geometry, timing, audio or input
   makes the difference.
5. Update the document when a decision changes. Passing source-link checks is
   not behavioral verification.

The initial compatibility promise is native content/save schemas plus the
explicitly chosen gameplay behavior. Retail save import/export and arbitrary
mid-combat or mid-cutscene saves are separate future work. They should not
silently determine the first Rust data model.

