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

## 1. Establish the engine and content boundary

**Playable result:** load a representative room, walk the party through it,
interact with one prop, play one original-style animated actor and one voice
line.

Work:

- Compile one coherent engine/input/physics/UI dependency set on the intended
  native platforms and commit its lockfile.
- Decode enough retail content to obtain real geometry, collision, textures,
  anchors, an actor's animation and audio. Keep proprietary assets external.
- Test stairs, slopes, narrow doors, an elevator/platform and line-of-sight
  queries with the party's actual dimensions.
- Test navigation through doors, links and multiple levels. Resolve the
  Bevy 0.20 versus navigation-plugin compatibility decision before spreading
  plugin-specific types through gameplay.
- Prove vertex/frame animation and identified playback completion; do not
  assume every original animation becomes a skeletal glTF clip.
- Build a small native UI with keyboard/gamepad focus, text entry and input
  capture. Use it to prove map-note editing rather than building all screens.

**Exit evidence:** checked-in minimal code and a documented asset invocation;
observed behavior on the representative geometry; chosen versions and known
gaps recorded in [dependencies](dependencies.md). A flat test plane alone does
not settle the controller/navigation choice.

**Do not build yet:** a universal asset converter, an editor suite, a scripting
VM, interchangeable physics backends or a full UI framework.

**Status (25 October 2026):** in progress. Done:

- `retail::files` indexes an unmodified installation: base SLF libraries (more
  specific prefixes win), `patches/patch.NNN`, then loose files. It is exposed
  to Bevy as the `retail://` asset source.
- `retail::pvl` decodes the compiled level meshes and material table at the
  start of every retail `.pvl` (all 34 load). `retail::level_loader` turns them
  into Bevy meshes per octree mesh and material, repeat-sampled TGA textures
  and collision triangles.
- `retail::trigger` decodes type 4 trigger records. A new game enters the
  monastery at `MO100`, the retail new-game entrance.
- The party is one Avian body moved by Tnua, using the retail eye height,
  500-unit reach, walk/run/turn speeds, pitch limit and gravity. Its collider
  is raised 0.35 m (provisional) so steps meet Tnua's floating spring.
  Controls use an `Exploration` input context.
- A Rerecast navmesh is baked per level by Bevy's asset processor, cached,
  and used by Landmass. Stand-in walkers chase the party across both
  monastery levels. See [dependencies](dependencies.md#navigation).

Decisions made while doing this:

- Retail space is Y-up and left-handed. Bevy space mirrors Z, reverses
  triangle winding and uses 600 retail units per metre, which puts the eye
  1000 units (1.67 m) above the feet.
- Retail velocities are per 0.1 s tick; converted constants cite their source.

Known gaps, in rough order:

- The `.pvl` sections between the meshes and the triggers are not decoded:
  lights, monster paths, items, props, bitmaps, cameras and environment. Until
  they are, `find_item_trigger` locates an entrance by validating a type 4
  record found by its name. Replace it with a sequential section reader.
- Lighting is unlit vertex colour: static light, plus sun exposure at a
  provisional 0.75 until the environment's sun colour is decoded. There is no
  sky, fog or dynamic lighting. Animated `.ifl` textures show their first
  frame, and textures have no mipmaps.
- Collision and navigation use every fully opaque level polygon. Retail
  collision uses `.oct` surfaces with walkable flags and contact margins, and
  props collide too. Ground probing at entry is replaced by settling under
  gravity. Retail caps falls at 3000 units per tick; that cap is not applied.
- Navigation needs doors, teleporters and elevators as links, and retail
  collision surfaces; the party collider can catch on stairs. See the
  experiment results in [dependencies](dependencies.md#experiment-results-25-october-2026).
- The handedness conversion has only been checked visually. Compare it against
  a retail screenshot, e.g. readable text or a known landmark layout.
- No props, doors, actors, animation, UI, audio or navigation yet.

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

