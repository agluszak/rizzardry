# Locks, traps, and containers

## Status and scope

This is an implementation plan, based on source inspection of [wizardry-8-decomp at 075cbd0](https://github.com/agluszak/wizardry-8-decomp/tree/075cbd0b853c4670e94f6ae4fe48d53b52b5e819) and [whizzardry-8 at 9c433ec](https://github.com/agluszak/whizzardry-8/tree/9c433ec9725877a963cb43bb1df13d5d084ed873). Neither codebase is the architecture for the rewrite. Their behavior is evidence; the Rust structures below are proposed.

Use a world entity for a chest, door, or other interactive object. Keep its access mechanism and remaining contents as ordinary component values. Keep one active lock/trap interaction as gameplay session state. Native UI displays that state and submits actions. A successful pick, a trap discharge, and taking an item are separate commits.

The source repositories do not include the retail level, item-table, or dialogue assets. The general algorithms below are established by code; the exact lock, trap, key, loot table, and linked actions of a particular chest still require its content records.

## Evidence and comparison

| Concern | Original reconstruction | Current modernization | Consequence for Rust |
|---|---|---|---|
| Lock state | Trigger embeds type, difficulty, completion, pin configuration, practice budget, and last-interaction clock. [D1] | LockState has explicit reset/restore methods; decoded SavedLock values are separate from the live object. [M1] | Separate imported definition, saved device state, and active interaction. Do not copy the packed trigger record into ECS. |
| Picking and forcing | Screen code owns pin owners, temporary spell holds, RNG, stamina cost, and a one-second unlock delay. [D2] | Named interaction states and owned widgets improve lifetime management, but gameplay still runs inside W8LockInteraction through game/UI accessors. [M2] | Move decisions into game/world/interactions.rs; widgets become readers and intent producers. |
| Trap work | Screen code mixes inspection, device guesses, timers, spell selection, and discharge. [D3, D4] | W8TrapInteraction retains the same substantive algorithm with named states and separate active/suspended UI lifetime. [M3] | Use a small explicit session enum; call the ordinary spell/effect resolver for harm. |
| Container generation | GenerateItemGroup reseeds the global CRT RNG with a saved container seed, generates once, and retains gold/items. [D5] | Same generation algorithm; the seed now resets Game.rng. World-item ownership uses unique pointers/vectors. [M4] | Save actual rolled contents. Use a local container-seeded generator as an explicit native-RNG change. |
| Taking loot | A modal picker temporarily owns the container's linked items. [D6] | Picker moves owned world items out and returns leftovers on close; partial stack transfers preserve the remainder. [M5] | Leave contents on the container. The UI holds IDs/selections, never the inventory itself. |

## Data ownership

| Data | Owner and representation | Persistence |
|---|---|---|
| Authored key requirement, pick difficulty, trap definition, table reference, linked world action | Immutable content definitions addressed by stable content IDs | Definition IDs in saves; definitions in loaded content |
| Access mechanism | AccessDevice component on the world object: pickable lock, trap, or keyed mechanism, with resolved/armed state appropriate to that variant | Saved with the object's stable WorldObjectId |
| Lock pin configuration and remaining practice budget | Pickable-lock value inside AccessDevice | Saved; opening the panel does not reroll either |
| Selected trap kind when first interaction rolls one | Trap value inside AccessDevice | Saved after the first selection |
| Door/chest open state | World object's own state; independent of whether the device is solved | Saved |
| Container contents | ContainerContents component containing item values, their stable IDs, and gold | Saved with the world object |
| Raised pin owners, magical pin holds, selected actor, per-actor trap observations, disarmed cells, outstanding attempt | ActiveDeviceInteraction resource containing a LockSession or TrapSession | Session state; no UI pointers. Save at a resolved boundary as described below |
| Item-picker selection, hover, scroll, progress display, audio playback | UI/presentation | Recreated |
| Skill, stamina, conditions, spell points | Character entity's existing character/vitals values | Saved by character owner |
| Campaign time and ordinary gameplay randomness | Existing clock and RNG resources | Saved by their existing owners |

A lock's eight persisted pin bytes are randomized configuration, not the currently raised pins. The latter live in the interaction session. Similarly, lock_countdown in the C++ code is a limited skill-practice budget: it is spent only on a qualifying pick attempt, not decremented with wall time. Reset initializes it to three times the pin count, with the count clamped to 2–8. [D1, D2]

A compact contents value removes the separate items_generated flag:

    enum ContainerContents {
        Unrolled { table: LootTableId, seed: LootSeed, stored: Vec<ItemInstance> },
        Rolled { items: Vec<ItemInstance>, gold: Gold },
    }

An empty Rolled value remains rolled. The stored values in Unrolled are player deposits made before first opening. At materialization, append generated loot to those existing values. Do not overwrite a pending table with the player's deposit or generate it early: the party's level may change before opening.

## Common admission and action ordering

The game handler for InteractWithObject first resolves the stable target, checks that the party can interact with it, and checks the object's current access and world state. Repeated clicks cannot create a second session. Original and modernized code both refuse to enter lock/trap work during combat and prevent simultaneous lock and trap interactions. A spell/item detour suspends an existing session rather than creating another one. [D2, D3, M2, M3]

The order is:

1. Validate target, active mode, actor, action, and any required item.
2. Determine the applicable access mechanism or authored fallback.
3. Accept the attempt and record any cost/random outcome that must survive closing the UI.
4. Advance its specific session phase.
5. Commit success, failure, discharge, or world activation once.
6. Let UI and presentation observe the result.

For keys, distinguish a reusable key from an authored consume-on-use item. The original trigger selector has both required-item handling and a separate keyed-door path; matching a door key clears its locked state and can run a named linked trigger. A generic required item is only removed when its consumption flag is set. The rewrite should validate all relevant requirements before consuming the selected value. Do not infer that every key is consumed, that every locked object is pickable, or that solving a device necessarily opens every linked door. [D7]

## Walkthrough: picking, forcing, and Knock Knock

### Manual picking

| Step | Original behavior | Rust operation |
|---|---|---|
| Choose actor/pin | Actor must be eligible and have the Locks & Traps skill available. Changing picking actor drops the previous actor's nonmagical raised pins. | handle_pick_pin validates ActorId and pin index against the active LockSession. |
| Raise selected pin | UI starts a rising animation, then ResolvePick runs. A new pick is not a single success roll against that pin. | Store the pending pin action, then resolve at the chosen action boundary. |
| Retain earlier pins | For each earlier nonmagical pin held by the same actor, roll retention using skill and difficulty. Failing pins drop; the newly chosen pin becomes owned by the actor. | resolve_pick_pin applies the exact retention rule to value arrays, using gameplay RNG. |
| Practice | A 1-in-5 draw may spend one remaining practice opportunity and practice Locks & Traps. | Decrement the saved budget and call character skill practice once. UI refresh does neither. |
| Check completion | All required pins must have an owner. Success disables controls and starts a one-second delay. | Move to Unlocking with an explicit timer and no second accepted action. |
| Finish | CompleteItemInteraction marks the device solved, clears a door's locked flag where relevant, and reruns the trigger. | finish_unlock marks the access state solved and calls activate_unlocked_object once. |

The retention chance in the source is the actor's skill plus 5 minus five times the game difficulty setting, followed by the original clamping/conversion behavior. Preserve the normal-range formula; isolate any byte-wrap behavior as a documented compatibility decision rather than embedding casts throughout the Rust code. [D2, M2]

### Forcing

Forcing requires an eligible actor, at least 80 internal stamina units, and effective Strength above 50. With n tumblers and difficulty setting d, the source uses divisor = clamp(d - 1 + n, 2, 8) and integer chance = (Strength - 50) / 2^(divisor - 2). The accepted attempt spends 80 stamina before its random draw, including on failure. Success enters the same one-second unlock phase. Failure leaves the mechanism locked and refreshes eligibility. [D2, M2]

Implement force_lock as a complete operation over the actor, device, session, and RNG. Reject insufficient stamina before any draw or cost. An accepted failure is committed; closing the panel cannot refund it.

### Spells and usable items

The current interaction is retained while the spell or item picker is open. In the observed picking code, the selected actor's magically held pins are released before that detour; changing the manual picking actor has a different release rule. Preserve these distinctions when porting the session logic. Returning or cancelling the picker reuses the same target/session after revalidating that both still exist. [D2, M2]

Knock Knock shuffles pin order and tests at most power + 1 pins. The per-power table is expressed in thousandths; successful normal effects hold pins magically, while backfire releases selected raised pins. It then checks the same all-pins-held success condition. The spell module owns cast eligibility, resource expenditure, and backfire; interactions owns how that resolved effect changes this lock. Item charges are consumed by the item-use operation, not again by the lock UI. [D2, D4]

## Walkthrough: inspecting and disarming a trap

A trap can have a fixed kind or request a random kind on first interaction. The original selects among fifteen types until the type's difficulty falls within the permitted four-grade band and records the choice on the live trigger. Reopening its panel does not reroll it. The rewrite saves that choice explicitly; original lock record versions differ in which fields they include. [D1, D4]

The session needs actual trap kind, actor-specific observations, completed device cells, Divine Trap assistance, and a small phase enum. The player's selected trap-name hypothesis is UI/session input, never the authoritative type.

| Action | Resolution and consequences |
|---|---|
| Inspect | Compute the clamped inspection chance from Locks & Traps, difficulty, and Divine Trap assistance. Success practices the skill and independently reveals each of eight device cells or marks it uncertain for that actor. Failure springs the trap. |
| Choose a device cell | Choosing a cell absent from the actual trap immediately selects the sprung outcome. For a required cell, success compares a percentile draw with chance squared divided by 100. A success marks that cell completed and practices the skill. |
| Finish all required cells | CompleteTrapDisarm marks the mechanism resolved, announces success, then activates the underlying object. |
| Cast Divine Trap | The spell effect supplies assistance to the session. Despite its historical helper name SetKnockKnockTarget, this call originates in the Divine Trap spell case. |
| Cast Knock Knock at a trap | The source tests power × 5 + 50 - trap difficulty × 6; backfire supplies zero chance. After a 1.5-second resolution delay, success disarms and failure springs the trap. |
| Deliberately trigger | Enter the same discharge operation used by failed inspection/disarming. |
| Exit or suspend | Closing an idle session discards observations and unfinished cell work. A spell/item detour retains them. An accepted attempt must resolve before a new action or save. |

The inspection formula uses a table penalty for trap difficulty and clamps the final chance to 0–99. Direct cell work uses a different assistance amount before squaring the chance. Keep these as separate small rule functions with explicit inputs; one shared generic skill-check formula would erase real behavior. [D3, D4, M3]

### Discharge is not always disarm

ResolveSprungTrap has a 50% branch that completes the mechanism; the other branch leaves it armed. It derives effect power and target count from difficulty, chooses an effect source near the prop, and calls the normal spell resolver. For party-targeted effects, the selected actor is protected from random removal from the target list when eligible. Point-targeted effects use the party location. The UI waits two seconds and for portrait speech to finish before rerunning the trigger. [D3, D4]

The Rust discharge operation should:

1. Record whether the trap becomes disabled, its target selection, and resolved effect parameters.
2. Apply damage/conditions through the existing spell/effect code exactly once.
3. Run the short presentation/response wait.
4. Continue opening only when the access state now permits it. If still armed, leave the object inaccessible and end this attempt without recursively launching another one in the same system.

Conditions remain values on the affected character; no entity is needed for each trap-inflicted poison or other condition. A transient visible spell effect may have its own presentation entity.

The modernization retains the core discharge rules but changes how the source point is obtained: it uses the prop center and a spatial trace result instead of the old representation/bounds path. Treat effect origin/occlusion as a behavior check, not an assumed exact match. [D4, M6]

## Walkthrough: opening and taking contents

Solving a device and materializing its contents are different operations. The original opening path generates the table only once, attempts item identification, credits contained gold and clears that gold balance, then either hands out the lone item or opens a picker for several items. The picker waits for the prop to be in an acceptable animation state. Opening with an item already held can instead deposit that item. [D5, D6, M4, M5]

| Step | Proposed function/system | Commit and failure behavior |
|---|---|---|
| Activate unlocked object | activate_unlocked_object | Validate access and authored linked action. Record open/closed target state separately from animation progress. |
| Materialize authored loot | materialize_container | Resolve table, current party-level inputs, and saved seed. Build a local item-value vector and gold total; assign Rolled only after successful generation. Missing table is a content error, not an empty chest. |
| Apply opening effects | resolve_container_open | Run identification through item rules. Credit gold once and set container gold to zero in the same operation. |
| Display contents | open_container_view | UI reads the current contents by WorldObjectId and ItemInstanceId. It does not remove them from the component. |
| Take selected values | take_container_items | Revalidate each selected item and destination. Transfer only units that fit; leave remaining units in the container. Report partial success without duplicating the source stack. |
| Deposit | put_item_in_container | Validate source and destination; move the selected value once, preserving any pending authored loot table and its later generation boundary. |
| Close | close_container_view | Discard selection/scroll state. Items already taken and remaining contents retain their current owners. |

The loot-table algorithm is more specific than a conventional weighted picker: weight-zero entries are guaranteed, level-scaled candidates use party level and item value, insufficient candidates trigger a fallback, and random candidates are rejection-sampled and removed after selection. The original does not recompute the initial total weight after removal. Do not replace it with a weighted-choice library and assume equivalence. [D5, M4]

**Proposed native-RNG change:** use a local RNG for container generation instead of reseeding the shared gameplay RNG. Preserve the table algorithm and agreed container results, but explicitly give up the accidental effect of opening a chest on the subsequent global random stream. If exact seeded retail contents are required, the local generator can implement the original random algorithm; that still does not require resetting combat's RNG.

Do not pre-roll every chest on map load: generation currently reads party level at materialization. That would change content even with the same seed.

## Save, cancellation, and map changes

Persist the world object's solved/armed state, open state, randomized device configuration, practice budget, chosen trap kind, and unrolled seed or rolled contents. Original lock saves and modern SavedLock/SavedTriggerLoot records support this distinction. Do not serialize Bevy Entity handles, pointers, widgets, or sound handles. [D1, M1, M4]

An idle session may close before a save is attempted, discarding unfinished session progress in the same way as leaving the interaction. During an accepted timed attempt, a manual save request may remain as a transient UI intent scoped to this session. At the resolved boundary, consume that request, close the now-idle panel, and attempt the ordinary save gate once. If another gate still refuses the save, report refusal and discard the request. It is never serialized; cancellation, travel, load, or leaving the session also discards it. It must not produce an unexpected later save. An outcome already rolled, stamina already spent, spell resources already consumed, or harm already applied cannot be cancelled away by closing a widget.

This is a proposed explicit native behavior, not a claim that every retail UI cancellation edge was verified. Use a small session phase to prevent repeat acceptance and continue the pending outcome even if its view closes. Do not introduce a general transaction engine or a durable UI-command log.

When loading a dormant map, restore its container values and device state before enabling interaction. Reconstruct prop visuals from open state; do not rerun success callbacks or regenerate loot to reconstruct appearance.

## Module dependencies

- content loads and validates lock/trap definitions, key requirements, loot tables, and world-object links.
- game/world/interactions owns admission, lock/trap sessions, resolved device state, and activation.
- game/inventory owns item moves, stack remainders, identification, and operations on PartyInventory's shared pool, purse, and held item. PartyRoster owns actor membership/formation; the party body is only the shared spatial entity.
- game/characters owns stamina eligibility and skill practice; game/magic owns resolved spell effects both inside and outside combat.
- ui/interactions reads sessions and submits typed actions. presentation animates tumblers, props, sounds, and spell visuals.
- save captures canonical values after the relevant operation has finished.

Register these systems with the game's existing plugin. A separate crate, generic lock trait, per-pin ECS entities for gameplay, or puzzle scripting runtime is unnecessary.

## Finite unknowns and behavioral checks

Before implementing a representative retail chest, obtain its level/trigger record, loot-table row, and item records. Confirm required-key consumption, linked actions, trap type/grade, generation seed initialization, and the correct animation readiness point. These are missing content inputs, not reasons to redesign the system.

The focused acceptance cases are:

- Manual picking drops/retains the correct actor-owned pins; switching actor differs from a spell/item detour.
- Practice opportunities survive closing/loading and stop at zero; display refreshes consume no randomness.
- A force attempt at 79 stamina is rejected; an accepted failed attempt spends 80.
- Knock Knock normal/backfire and Divine Trap assistance reach the correct rule functions.
- A wrong trap cell, failed inspection, and deliberate triggering use the same discharge; disabled versus still-armed branches both work.
- Trap harm happens once even if the UI closes during its timer.
- The same unrolled seed and party level produce the agreed loot; reopening or loading a rolled empty chest produces nothing new.
- Gold, a single item, multi-item selection, full inventory, and partial stack transfer preserve total ownership.
- Depositing into an unrolled container retains both authored loot and the deposit.
- The local loot RNG leaves the next ordinary gameplay draw unchanged; this deliberate difference from the two C++ versions is recorded.

These are planned behavioral checks. No runtime implementation or new test suite is included in this documentation change.

## Sources

- [D1 — original lock reset, budget, and saved state](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/engine_code/Trigger.cpp#L179-L305).
- [D2 — original picking, forcing, suspension, spell resolution, and unlock](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/MainGameScreen.cpp#L987-L1359).
- [D3 — original trap interaction](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/MainGameScreen.cpp#L1962-L2441).
- [D4 — trap selection, disarm, and discharge](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Traps.cpp#L225-L369), [trap spell chance](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/MGSTextBox.cpp#L2130-L2169), and [Divine Trap call site](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Magic%20Effects.cpp#L3290-L3297).
- [D5 — original once-only generation](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/engine_code/Trigger.cpp#L2365-L2412), [loot-table selection](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/ItemManager.cpp#L215-L303), and [container opening](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/engine_code/Trigger.cpp#L2824-L2908).
- [D6 — original picker transfer](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/dialog_code/DialogFactoryDialogs.cpp#L1104-L1155) and [picker ownership](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/dialog_code/DialogFactoryDialogs.cpp#L1392-L1425).
- [D7 — original required-item, keyed-door, and device gate](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/engine_code/Trigger.cpp#L3546-L3635).
- [M1 — modern lock value](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/entities/locks.cpp) and [saved world records](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/include/wiz8/entities/saved_world_sections.h#L23-L80).
- [M2 — modern lock lifetime](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_screens/MainGameScreen.cpp#L353-L424) and [lock operations](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_screens/MainGameScreen.cpp#L710-L1069).
- [M3 — modern trap operations](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_screens/MainGameScreen.cpp#L1416-L1803).
- [M4 — modern generation](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/entities/trigger_actions.cpp#L916-L946), [table selection](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/ItemManager.cpp#L129-L212), and [saved contents](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/entities/trigger_io.cpp#L193-L201).
- [M5 — modern partial transfer](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/dialog_code/DialogFactoryDialogs.cpp#L452-L496), [picker ownership](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/dialog_code/DialogFactoryDialogs.cpp#L592-L611), and [return on close](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/dialog_code/DialogFactoryDialogs.cpp#L811-L818).
- [M6 — modern trap discharge](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/Traps.cpp#L92-L178).

