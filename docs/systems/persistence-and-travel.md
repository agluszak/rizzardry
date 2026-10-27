# Persistence, travel, and dormant maps

## Decision and evidence status

Use the ECS as the authority for the running campaign. Write explicit, versioned native save records with Serde and RON initially. A save is a captured value, not a reflected Bevy World, an Entity-number dump, or a second database that gameplay edits alongside ECS components.

Keep the party and named/story NPC entities alive for the campaign. Load physics, navigation, rendering, and ordinary local actors for one active map. Move ordinary local actors into dormant map records when leaving. A dormant map may refer to a named NPC's ActorId, but must never contain a second copy of that NPC's character, stock, equipment, or world-role condition state.

**This is a design proposal, not implemented functionality.** The behavior comparison uses the original reconstruction at [075cbd0][decomp] and the modern C++ rewrite at [9c433ec][modern]. Source was inspected; no new runtime scenario was executed for this document. Inferred names in the decompilation are not treated as stronger evidence than their bodies and callers.

| Question | Evidence | Consequence for the Rust design |
| --- | --- | --- |
| Does leaving a map preserve it? | Original unload saves level state into CurrentGame.SAV; entry loads its matching level section before falling back to authored status. [D1] | Persist map changes across visits, but do not preserve the working-file mechanism. |
| Is an unloaded map continuously simulated? | Entry explicitly updates encounter budgets and invokes elapsed-time monster aging. [D2][D3] | Use bounded, rule-specific catch-up on activation; do not replay every missing AI/physics frame. |
| Are save/load and ordinary travel identical? | Ordinary entry clears continuing spell effects; save restoration reconstructs the supported saved monster-control effect. First-visit initialization is separate. [D1] | Give restore, first visit, and revisit different entry reasons. |
| What has modern C++ already improved? | Plain LevelState capture/build, in-memory LeftLevels, staged file replacement, and decode-before-install. [M1][M2][M3][M10] | Retain these ownership improvements without carrying over RIFF images or retail record layouts. |
| Can an arbitrary frame be saved? | Quicksave rejects combat and blocked/uninterruptible interaction; autosave has additional gates. [D4][D5] | Define supported save boundaries explicitly. Do not promise combat, falling, cutscene, or in-flight-action saves. |
| Does the modern save restore the RNG stream? | Its capture/install records omit the RNG; GameSession carries the stream forward when replacing a game. [M4][M5][M10] | Native saves will store RNG algorithm and state. This is an explicit native-save policy, not a claim about retail save semantics. |
| Is a recruited NPC just a monster copied into the party? | Both implementations retain an adventurer profile and use separate world-monster state, with specific condition-copy rules. [D6][M6] | One actor identity can own semantically distinct role state. See [Party and RPCs](party-and-rpcs.md). |

## Owners and lifetimes

The names below describe ordinary components and resources within the game crate, not a requirement for separate crates or a generic persistence framework.

| State | Authoritative owner while running | Lifetime / saved form |
| --- | --- | --- |
| Item, NPC, monster, map, and script definitions | Immutable content assets/catalogs | Content lifetime; save references stable definition keys and the required content set |
| Party membership and formation | PartyRoster resource | Campaign; PartyRecord references member ActorIds |
| Shared bag, purse, and physically held item | PartyInventory resource | Campaign; PartyRecord owns these item values exactly once |
| Party spatial pose and motion | Components on the single spatial Party entity | Active map; pose saved, controller contacts and collision caches rebuilt |
| Regular character and named/story NPC state | Their actor entities | Campaign; one ActorRecord per actor |
| Named NPC's off-party location | WorldPlacement on that actor | MapId plus pose; absence while recruited; never a competing map-resident copy |
| Named NPC world-role health/conditions | WorldCombatState on that actor | Exists outside the party; active-map simulation or defined revisit catch-up only |
| Ordinary local monsters, dropped items, mutable props/triggers | Active-map ECS entities | Moved into MapRecord on departure and reconstructed on activation |
| Dormant ordinary local actors and map changes | DormantLevels resource, keyed by MapId | Value records for unloaded maps only |
| Active map encounter budget, local script state, automap | ActiveLevel state and map-owned entities | Captured into the same MapRecord schema; active map is absent from DormantLevels |
| Facts, factions, journal, campaign deadlines | Story resources and named-actor components, according to actual owner | Campaign; references use stable IDs |
| Simulation clocks and gameplay RNG | Explicit clock/RNG resources | Campaign; algorithm version, state, and clock values saved |
| Open travel confirmation / destination load | TravelState resource | Only while the operation is pending; not a permanent second world |
| Save/load I/O request and progress | Persistence task state | Until completion; immutable captured records may outlive the frame that captured them |
| Entity lookup, nav paths, physics handles, render children, UI selection | Runtime indexes and presentation | Rebuilt; never authoritative save data |

**A named entity remaining alive does not make it simulate off-map.** Movement, AI, ordinary poison/regen processing, perception, and collision require the appropriate active map presence. Campaign timers that the original evaluates globally or lazily, such as merchant cooldowns and delayed stock, retain those separate semantics. An off-map character does not receive one update per rendered frame just because Bevy can still query it.

The retained AdventurerState of an unrecruited RPC is also not a second active party member. Only the active role's pools and conditions are used by combat and aging. World attacks and loot are not implicitly derived from retained adventurer equipment.

### Avoiding duplicate authority

For an ordinary local monster, exactly one representation is authoritative:

1. An ECS entity in the active map, or
2. A value record in one dormant MapRecord.

Departure captures values and removes the live local entities at one ordered transition boundary. Activation consumes the dormant record into live entities. An immutable candidate decoded for validation is not published to gameplay.

Named actors are different because their persistent identity participates in quests, recruitment, stock, and cross-map references. Their entity remains authoritative. A map template that mentions Vi Domina, for example, resolves her ActorId and consults her current placement and party membership. It must not recreate the authored NPC when she is already recruited, moved, dead, or otherwise removed by story state.

Do not add a mutable ActorRepository that duplicates these components. A small ActorId-to-Entity index is acceptable: it is rebuilt and maintained as an index, contains no copied actor state, and cannot override the entities it indexes.

### Active level is a committed state, not an asset-load notification

A destination has an explicit preparation outcome: pending, ready to activate,
or failed. Geometry completion alone is insufficient. An activation operation
verifies all **required gameplay** dependencies (collision, valid local state,
resolved authored references and required shared catalogs), publishes the map,
then enables its actors, triggers and the party body together. Optional
presentation assets may finish later if gameplay does not depend on them.

A failed preparation leaves the source map alive, with input and physics
enabled and no speculative destination mutations. During the committed switch,
world actions stop, source-owned sessions are settled or cancelled, and the
persistent party's controller/collision is suspended until the new map is
active. A failed required dependency must not turn a trigger's loot, damage or
other mandatory consequence into a silent no-op.

Map records contain surviving ordinary actors (including health and death),
container contents, mutable devices, local scripts, explored cells and notes.
Notes belong to the map, not a global screen resource. Store only lasting
state, not proximity caches, currently playing animation, or pending UI drafts.
The authored population is a baseline for a **first visit**, never a fallback
to respawn previously killed or altered monsters on a revisit.

Use map/session identity to reject late asset, audio and action completions
from the departed level. None of this requires a generic loader framework or
two simultaneously authoritative levels.

## Stable identities and save records

Bevy Entity values are runtime handles. Neither their numeric parts nor vector positions are campaign identity. Use typed stable ActorId, StackId, MapId, and authored-object keys where a reference must survive save/load or an ownership move. Dynamic IDs can use a campaign counter; persist its next value. A UUID dependency is not required merely to allocate local single-player identities.

Do not use display names, party slot numbers, a monster species, or a sorted inventory row as identity. Two monsters can share a species; a party slot can be vacated and reused; identifying, sorting, or consuming an item can move its row.

Start with these explicit record groups:

| Record group | Contents |
| --- | --- |
| Save header | Format version, game-rule version, required content identifiers/versions, save metadata |
| Campaign record | World/simulation clocks, gameplay RNG algorithm and state, next dynamic IDs, facts/factions/journal, campaign deadlines |
| Party record | Member ActorIds and formation, shared inventory/purse, active map and party pose, held item if this save state is supported |
| Actor records | Regular characters and named NPCs, their role state, single inventories/equipment, stock and dialogue/story memory |
| Map records | One captured active map plus all dormant maps: ordinary local actors/items, mutable trigger/prop state, encounters, exploration/notes, map-local script values, departure timestamp |

Save primitive values and domain enums, not closures, query caches, assets' transient handle numbers, operating-system clocks, raw pointers, or serialized Rust memory layout. Effects and scripted state save only the domain state required to reconstruct a supported continuation. An opaque running Rust call stack is not a save format.

On load, validate the complete ID graph before publishing: unique actor/item IDs, roster capacity and uniqueness, valid content keys, valid map references, resolvable required references, and no named-actor payload hidden inside a dormant map. A reference allowed to outlive its target must be explicitly optional and have a defined missing-target result. A malformed required reference is a rejected save, not an excuse to silently spawn a replacement actor.

Native save compatibility is the initial contract. Retail import, export, unknown retail bytes, and the old CurrentGame.SAV cache are separate work if requested; they do not determine the native data layout.

## When saving is allowed

The source has several different gates, rather than one universal "can save" function:

- Normal quicksave rejects blocked input, uninterruptible movement, ordinary combat, and Iron Man's manual-save restriction. The development-mode combat shortcut is not a player feature to reproduce. [D4]
- The blocked-input predicate includes spell targeting/casting, lock/trap/item-selection interactions, and a conditional conversation restriction. [D7]
- Autosave additionally waits for no world-cursor lock, no dying monster, no combat, no sight override, interruptible motion, and no dialogue/camp. The modern implementation expresses part of this through Activity. [D5][M7]
- Interruptible movement is a grounded/falling-state rule, not merely "velocity is zero." [D8]
- The modern snapshot includes a held inventory item. Its existence alone does not prove that every modal or action involving that item is saveable. [M4]

**Initial native policy:** support manual saves at a settled, out-of-combat exploration boundary and from a paused inventory/options view whose only pending ownership is a serializable held item. Reject manual saving during travel publication, unresolved world casts/impacts, uninterruptible movement, dialogue/script continuations without a defined checkpoint, and uncommitted character-edit/level-up drafts. Autosave waits for the quieter exploration boundary.

Lock/trap work has one narrow deferral rule. An idle panel closes before capture, discarding session-only raised pins, observations, and unfinished disarm work as ordinary closure does. If a timed attempt has already been accepted, it finishes applying its result exactly once; only then is the pending save attempted against the ordinary save gates. The cost, rolled outcome, and damage are never refunded to make a save possible. No unfinished interaction is serialized.

That pending manual save is transient UI intent, not durable campaign state. It is cleared when attempted, cancelled by the user, or invalidated by travel, loading another session, or exiting the game. If the next resolved boundary still fails another gate, report the refusal rather than leaving a request to fire later. This exception does not authorize arbitrary queues for combat actions, spells, or cutscenes.

This is deliberately a stated supported-state policy. Exact parity for every original manual-save route, especially dialogue and paused inventory submodes, remains a finite characterization task below. Do not broaden it by assuming all components can be serialized.

An autosave intent may wait until the next eligible boundary. A manual refusal reports why. The lock/trap deferral above is the only proposed pending manual-save route; it cannot survive a map or session change. Loading an existing save is a different operation from creating a save: modern tests explicitly load while combat is suspended, preserve that combat on malformed input, and replace it on a successful load. [M8]

## Ordered example: save, alter the world, then load

1. **Accept the save request.** Check the current save policy and finish the already-authorized simulation operation. Pending damage/removal must not be half-applied. If preparation has gameplay consequences, make it an explicit simulation step; the encoder never invokes gameplay. Original SaveGame itself performs cleanup, which is a reason to isolate preparation. [D9]
2. **Capture at one schedule boundary.** Read party, actor, story, clock/RNG, and active-map values. Copy dormant map records into the save value. Do not independently snapshot subsystems on different frames.
3. **Resume only after capture is complete.** Encoding can use the immutable captured value. A screenshot is optional metadata and cannot advance the gameplay RNG or invalidate an otherwise valid state capture.
4. **Write and publish.** Write to a temporary file beside the destination, finish encoding, flush/close it, then replace the destination using the platform's supported file-publication operation. Report success only after publication. A failure preserves the previous save. Modern C++ already follows the staged-write pattern. [M1]
5. **Later, read a load request.** Parse into plain records; check versions, bounds, content, all map records, and references. Prepare the destination assets before releasing the current session. Malformed data or a missing required asset leaves the existing session usable.
6. **Commit the replacement once.** An exclusive transition system stops old gameplay, cancels old interaction/async-result owners, removes the old campaign entities, installs resources, and reconstructs entities from the validated records. No gameplay system runs halfway through reconstruction.
7. **Resolve references in two passes.** Create actors and map-local entities, build the ID index, then connect roster, script, effect, and target references. Install presentation/navigation/physics from definitions and current state.
8. **Resume with Restore entry semantics.** Restore the saved pose, clocks, realized timers, and RNG state. Do not grant first-visit rewards, recreate starting inventories, run ordinary revisit catch-up, or roll fresh encounters merely because the process loaded a file.

Predictable fallible work belongs before step 6. This does not require a generalized rollback engine or two continuously running games. A late programming error during activation is not silently handled by merging half of the new state into the old campaign.

Old callbacks carry their original interaction/session identity and become inapplicable after replacement. A screenshot completion, voice completion, stale item confirmation, or old load result must not act on whichever Party entity happens to exist afterward.

## Ordered example: leave a map and return later

1. **Request a destination.** A trigger or script supplies MapId and entrance. Story access rules remain with that trigger/script. Do not invent a new universal reputation, quest, or "all members healthy" gate.
2. **Resolve travel confirmation and RPC consequences.** Normal entry can ask for confirmation. Cancelling restores the trigger's prior party position and makes no departure snapshot. RPC warnings use their own sequence; accepted warnings can dismiss companions before a separate transition proceeds. [D10][D13] See [Party and RPCs](party-and-rpcs.md).
3. **Prepare the destination.** Resolve content and entrance, decode an existing dormant map or instantiate a first-visit baseline, and load required geometry/assets. Do not run entry scripts, roll encounter catch-up, or mutate named actors during speculative asset preparation.
4. **Commit departure.** Stop accepting source-map actions; finish or explicitly cancel the source-owned operations according to their rules. Capture level-script variables, settle membership/placement changes, stamp departure time, capture ordinary local actors and persistent map changes, and remove their runtime bodies/entities. The named actors and party stay.
5. **Publish the destination.** Consume its dormant record, spawn ordinary local entities, and attach bodies only to named actors currently placed there. Apply entrance pose to the single Party entity. There is never a window in which both maps' AI can act.
6. **Run exactly one entry pass.** First visits initialize authored populations/items; revisits run the elapsed-time rules below; a save restore uses its separate path. Rebind script references by authored keys and actor IDs, then enable simulation and presentation.
7. **Return later.** The previous map's doors, loot, explored areas, notes, local script values, and surviving local actors come from its record. An NPC recruited there is resolved from the existing actor, not restored from the map's old character image.

Travel preparation does not roll back a dismissal already completed by the separate RPC warning sequence. If asset preparation then fails, the party remains on the source map with those completed membership and item changes intact. The transition itself has not committed. Keeping these boundaries separate preserves the fact that accepting a warning is not yet a successful map transfer.

Map unloading releases assets and bodies only after departure work. Despawning a render child must never secretly save the game, return RPC equipment, or advance a story fact.

Modern C++ already prepares replacement map/sky resources before leaving the current map and separates departure bookkeeping from resource release. Its entry function also distinguishes ordinary travel from restore. Keep that separation. The native proposal additionally makes the ordinary-local-record transfer and the named-actor exclusion explicit. [M11]

### Dormant catch-up is a rule, not a faster frame loop

The original encounter update adds elapsed time divided by the level's period to its budget, clamps budget/population bounds, rerolls random encounters when the absence exceeds 36,000 world seconds, and otherwise shifts existing encounter spawn timestamps. First visit takes the reset branch. These are explicit entry effects, not proof that off-map groups keep walking or fighting. [D2]

The misleadingly named AgeAllMonsterSight computes elapsed whole 120-second steps and calls AgeMonsterSight on living restored monsters. The latter also handles damage, regeneration, stamina, condition/enchantment/effect aging, and movement/look bookkeeping. Calling only a perception update would miss actual gameplay. Conversely, applying ordinary per-frame poison to all campaign-lived NPCs and also running this catch-up would double-age them. [D3][D11]

Represent each elapsed-time owner once. Named NPC world-role state is aged on its actor at the same map-entry boundary as other restored world actors. Its inactive adventurer profile is not also aged. Global NPC dismissal/dialogue cooldowns have their own clock-based pass; merchant stock and delayed crafting outputs retain their own evaluation rules. [D12]

Store enough timestamps and remainder state to apply each rule once. Do not "clean up" integer quantization into a different clock without a comparison scenario. Native loading restores already-realized timers rather than consuming RNG to rebuild them.

## Randomness and failure boundaries

Keep the tiny original gameplay generator/range mapping for rule comparisons unless a separate gameplay decision changes it. Modern C++ documents and implements the MSVC 32-bit stream, and separates presentation randomness. [M5][M9]

For native saves, persist the generator algorithm/version and current state, not only the campaign's initial seed. Restore it before any gameplay work; constructing meshes, UI, physics, or cached derived stats must not draw from it. A save itself makes no draw. An ordinary revisit may draw through its encounter/aging rules in an explicit stable order.

This makes native save/load reproduce the next gameplay draw. It intentionally differs from modern C++'s inspected session-stream continuation across replacement; it is not advertised as a retail-load RNG trace. Avoid inventing per-character or per-map gameplay streams until a demonstrated need outweighs the lost comparison simplicity.

## Module relationships

The save module depends on domain record conversions from party/items, story, world, and time. Those domains do not depend on RON or filesystem I/O. The world/travel module owns ordered departure and entry, calls the party module for RPC decisions, and invokes explicit story hooks. The time module owns elapsed-time rules; world entry supplies the measured absence. UI requests operations and displays their results. It does not own the only copy of travel destination state, a held item, or a load candidate required to complete an accepted transition.

## Acceptance scenarios

These are implementation gates with concrete failure modes, not a request for a broad framework:

1. Open a door, empty a container, annotate the automap, leave and return. State persists; loot and first-visit initialization do not repeat.
2. Recruit an NPC on map A, travel to B, save/load, dismiss on B, and revisit A. Exactly one ActorId exists; no old NPC or inventory respawns on A.
3. Leave a wounded/conditioned ordinary monster and a named NPC, then revisit at elapsed values immediately below and above a catch-up step and the encounter stale threshold. Each active role receives the source-defined catch-up once.
4. Save with a supported held stack, load, and place it. Total units and unique ownership are preserved; no item remains duplicated in its old slot.
5. Save immediately before a known gameplay random draw. Loading yields the same next draw; screenshot/UI activity changes neither result nor saved state.
6. Reject malformed nested map data, duplicate actor IDs, unresolved required references, and missing required content before session replacement.
7. Fail file writing before publication. The previous save remains loadable; the UI does not report success.
8. Cancel a travel warning before accepting any departure, or fail destination preparation. Cancellation commits no dismissal. Failed preparation preserves source-map authority and makes no speculative gameplay draw; it does not reverse an RPC dismissal and item returns already completed by a separate accepted warning.
9. Attempt manual save during combat, falling, pending impact, and an uncheckpointed scripted scene. The supported-state policy is enforced; autosave waits without serializing a partial action.
10. Request a save from an idle device panel and during an accepted trap/lock timer. Idle closure discards only session progress; an accepted attempt applies its costs and result once before the save is attempted. Travel/load/exit cancels the pending save, and a later unrelated interaction cannot reactivate it.

## Remaining decisions and characterization work

1. Pin the exact manual-save policy for dialogue and inventory submodes; compare keyboard, options, and autosave entry points instead of assuming one gate.
2. Characterize pending deaths, long-lived lure/control effects, and moving-platform contact at save/travel boundaries. Add supported continuation records only when the gameplay state is understood.
3. Pin revisit catch-up ordering and remainder behavior, including named NPC world-role conditions and encounter rerolls. Avoid interpreting decompiled "minutes" names as units without checking callers.
4. Confirm NPC travel-warning acceptance/retry timing and scripted exits that bypass normal confirmation. Preserve established consequences without making every scripted exit a user prompt.
5. Define the first native format version and content compatibility policy. Compression or a binary codec is a later size/performance decision; it does not change ownership.

## Primary sources

Evidence links above point to the pinned source revisions and relevant line ranges. They support the original/modern behavior; the Rust ownership and save policies are explicitly proposed.

[decomp]: https://github.com/agluszak/wizardry-8-decomp/tree/075cbd0b853c4670e94f6ae4fe48d53b52b5e819
[modern]: https://github.com/agluszak/whizzardry-8/tree/9c433ec9725877a963cb43bb1df13d5d084ed873
[D1]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/engine_code/Levels.cpp#L512-L688
[D2]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/engine_code/MonGen.cpp#L508-L550
[D3]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Sight.cpp#L363-L385
[D4]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/MGSKeyboard.cpp#L454-L499
[D5]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/LoadSaveGame.cpp#L1733-L1859
[D6]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Manager.cpp#L452-L622
[D7]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/MainGameScreen.cpp#L6368-L6384
[D8]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/engine_code/GameData.cpp#L2252-L2264
[D9]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/LoadSaveGame.cpp#L382-L507
[D10]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/MainGameScreen.cpp#L4854-L4891
[D11]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/GameplayTime.cpp#L898-L1250
[D12]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Manager.cpp#L871-L900
[D13]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/MainGameScreen.cpp#L2996-L3021
[M1]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/LoadSaveGame.cpp#L204-L220
[M2]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/LoadSaveGame.cpp#L1051-L1107
[M3]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/LoadSaveGame.cpp#L1245-L1297
[M4]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/LoadSaveGame.cpp#L1031-L1048
[M5]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/game.cpp#L57-L97
[M6]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/NPC%20Manager.cpp#L452-L601
[M7]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/LoadSaveGame.cpp#L847-L860
[M8]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/tests/runtime/saves_test.cpp#L59-L119
[M9]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/include/wiz8/rules/random.h#L1-L75
[M10]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/LoadSaveGame.cpp#L1774-L1908
[M11]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/world/level_session.cpp#L348-L532
