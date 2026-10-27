# Revisions and open questions

## Status

This is the current design as of 24 October 2026. It consolidates the earlier
discussion, corrects overgeneralizations exposed by source comparison, and
identifies deliberate rewrite policies. The linked system documents contain
the evidence and full flows.

“Same gameplay” does not mean the two C++ repositories always agree, or that a
recovered name establishes behavior. When they differ, record the difference
and choose deliberately. Until characterized, use the decomp's observed behavior
as the starting reference rather than silently treating a modernization as a
rules change.

## Corrections and qualifications

| Topic | Current decision | Detail |
| --- | --- | --- |
| ECS and saved data | Live ECS is authoritative; saves are snapshots and dormant records own only absent ordinary objects | [Architecture](architecture.md), [persistence](systems/persistence-and-travel.md) |
| NPC identity and role stats | One persistent actor and one adventurer inventory, but distinct retained AdventurerState and active WorldCombatState where the original uses them | [Party/RPCs](systems/party-and-rpcs.md) |
| NPC world equipment | World-role attacks, armor and loot do not automatically derive from the retained adventurer's equipment; merchant stock is separate again | [Party/RPCs](systems/party-and-rpcs.md) |
| Unloaded named NPCs | Keeping an actor entity alive is a lifetime decision, not permission to run its world AI/recovery every frame | [Persistence](systems/persistence-and-travel.md), [time](systems/time-rest-and-conditions.md) |
| Party ownership | One Party entity holds Formation, PartyInventory and the spatial body; the PartyMember relationship's ordered PartyRoster is the one membership authority | [Architecture](architecture.md) |
| Monster engagement | A monster's engagement can exist before or without the party's Battle. This does not establish independently simulated fights in distant/unloaded areas | [Combat AI](systems/combat-ai-and-navigation.md) |
| Perception and disposition | Party detection, monster detection and perception history are directional; faction, group provocation and actor overrides are separate rule inputs | [Combat AI](systems/combat-ai-and-navigation.md) |
| Party combat movement | One scheduled movement action; the inspected initiative calculation uses the minimum living-member initiative, despite misleading comments | [Combat AI](systems/combat-ai-and-navigation.md) |
| Area bomb lifecycle | Selection does not consume; accepted execution can consume on success, fizzle or backfire. The effect retains a payload after the item is gone | [Combat items](systems/combat-items-and-magic.md) |
| Area targets and animation | The inspected radius path captures targets at casting and tests the shared party body. Do not invent per-member collision tests or a player hand-release callback | [Combat items](systems/combat-items-and-magic.md) |
| Merchant stock | Selling normalizes merchandise into stock lots/templates. It does not preserve every sold instance's charges and knowledge for later resale | [Inventory/trade](systems/inventory-trade-and-curses.md) |
| Identification and curse | Item definition, identification/curse knowledge, equipped restriction and removal permission are different. Remove Curse does not simply rewrite the item's definition as permanently noncursed | [Inventory/curses](systems/inventory-trade-and-curses.md) |
| Level-up | Commit progression choices, then apply derived-stat and equipment consequences. HP recalculation preserves the missing-HP difference; no invented random HP roll | [Progression](systems/progression-and-map.md) |
| Newly invalid equipment | Forced validity cleanup bypasses voluntary cursed-removal checks in both sources; place removed gear through explicit fallback rules, then settle derived stats again | [Progression](systems/progression-and-map.md) |
| Monastery scene | Model Aletheides and the Savant henchman. Control release and the henchman's final body removal happen at different milestones | [Scenes](systems/story-scenes-and-puzzles.md) |
| Spaceport puzzle | Repair, recorder analysis and knowledge are separate. Similar Black Box fact names are not aliases; callback bodies alone do not prove item consumption | [Puzzles](systems/story-scenes-and-puzzles.md) |
| Map notes | The original custom notes belong to the full automap; shared minimap annotations are a deliberate extension | [Map notes](systems/progression-and-map.md) |
| Lock picking | Persistent pin configuration differs from session-held pins. The historical lock_countdown is a skill-practice budget, not elapsed time | [Locks](systems/locks-traps-and-containers.md) |
| Triggered traps | Discharging a trap does not guarantee it is disarmed; the source can leave it armed | [Traps](systems/locks-traps-and-containers.md) |
| Container generation | Preserve player deposits without rolling loot early; opening later can use a different party level. An empty rolled chest stays rolled | [Containers](systems/locks-traps-and-containers.md) |
| Crafting | Manual combination is not a generic random success roll. Use original product placement and practice-before-creation as the baseline; Whizzardry differs in both | [Crafting](systems/crafting-and-services.md) |
| NPC commissions | Mature one delayed stock entry at the relevant refresh; do not replay ingredient-completion handlers during load | [Services](systems/crafting-and-services.md) |
| Time | Real time, simulation time, round aging and accelerated recovery are separate. Bulk aging is not automatically equivalent to repeated one-step calls | [Time](systems/time-rest-and-conditions.md) |

## Deliberate native policies

These are proposed rewrite choices, not claims of byte-for-byte retail
compatibility.

### Native saves and RNG

Use explicit versioned save records, stable IDs, staged writes and reconstruction
of runtime references. Include the named gameplay RNG algorithm/version/state
and identity allocator state. The inspected C++ save capture does not serialize
the session RNG as this proposal does. Saving/loading our native state therefore
has a different continuation contract.

Generate a container's seeded loot through a local generator. Both inspected
C++ versions reseed a shared stream in that path. Preserve the agreed loot
algorithm and outputs, while deliberately avoiding the unrelated future-combat
RNG change caused by opening a chest. Exact seeded retail outputs, if required,
still need the corresponding algorithm and characterization.

### Stable save boundaries

Initial saves support settled gameplay state, not arbitrary execution frames.
Do not serialize animation handles, temporary UI drafts or unfinished combat
actions and imply that they can resume.

Lock/trap panels have one narrow convenience rule: an idle session can close
before saving; an accepted timed attempt must first apply its result, after
which the pending save is tried once against the ordinary save gates. The
pending request is transient UI intent, cleared on attempt, cancel, travel,
load or exit. This does not introduce a general queue for saving through any
cutscene or combat action.

### Native ownership and presentation

Do not copy the C++ service locator, global flags, raw pointers, callback queues,
screen router or parked-at-origin actor trick. Represent the required meaning
with owners, ordinary operations, a small running-state enum and identified
playback requests.

Keep native Bevy rendering, assets and UI. Add libraries when they eliminate a
demonstrated gap. The proposed versions are metadata-researched candidates,
not a compiled dependency set.

### Content and annotations

Keep retail decoding at the content boundary and preserve stable mappings for
external definitions/facts. Use readable native schemas and explicit overrides.
Do not distribute retail assets with the repository.

Map-note storage uses owned text and coordinates. Native text editing owns the
caret. Projecting the same note on the minimap is an intentional feature; any
change to count/text limits should be explicit UI policy rather than an
accidental consequence of old buffer sizes.

## Decisions that still require evidence

| Question | Evidence that will settle it | Work can proceed meanwhile |
| --- | --- | --- |
| Which Bevy/navigation combination? | Compile a coherent stack and navigate a representative multi-level retail map | Keep navigation intent/query types owned by the game |
| Exact monastery/spaceport script gates and item debits? | Load the relevant retail NPC/level scripts and observe the accepted interaction paths | Use the verified lifecycle/ownership boundaries without inventing content |
| Exact RPC role conversion edge cases? | Characterize the original nonzero condition overlays and HP/stamina exceptions | Keep role-specific state explicit |
| Crafting recipe and placement edge cases? | Characterize original slot-selection branches, practice draws, same-kind fallback and real recipe records | Implement the documented original baseline through shared inventory primitives |
| Complex magic rounding, geometry and effect ordering? | Small fixtures for each implemented spell family, plus real-map queries | Implement one complete effect path before the full catalog |
| Bulk rest/catch-up equivalence? | Record the source's batched condition/disease/recovery behavior and RNG consumption | Keep elapsed-time units and fractional accumulators explicit |
| Bink, voice, bitmap fonts and frame animation support? | Decode and render/play representative assets on target platforms | Keep presentation behind content IDs and playback requests |

Each system document lists more local acceptance cases. These are finite
implementation questions, not reasons to design a general framework first.
No runtime implementation or game-behavior test has been completed by this
documentation-only change.

