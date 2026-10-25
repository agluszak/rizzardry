# Story scenes and environmental puzzles

## Design decision

Use ordinary Rust code for the few sequences with special staging, together with
typed dialogue/content and explicit story facts. A scene owns its current step
and the specific playback or conversation it is waiting for. Persistent facts
belong to the campaign or the relevant world object. A camera tween, dialogue
widget or animation callback is never the sole owner of a quest consequence.

The two examples below cover very different interactions: an authored encounter
that changes control and actor presence, and a puzzle assembled from several
independent discoveries. Neither needs a general workflow engine.

## Source comparison and limits

| Subject | Decomp evidence | Whizzardry comparison | Consequence for the rewrite |
| --- | --- | --- | --- |
| Monastery henchman arrival | [Fact dispatch](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Scripting%20Facts.cpp#L630-L635), [call action](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Scripting.cpp#L1779-L1792), [arrival callback](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Scripting.cpp#L2551-L2576) | [Typed story action](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/npc_story_events.cpp#L538-L545) and [arrival helper](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/npc_story_events.cpp#L339-L366) retain the same staging | Keep the order and actual completion point; replace globals and numeric callbacks with owned state |
| Henchman departure | [Departure action](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Scripting.cpp#L1692-L1705) starts the cycle and immediately clears the main-game target state; [completion](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Scripting.cpp#L2581-L2588) parks the actor | [Modern action and helper](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/npc_story_events.cpp#L488-L497) keep those distinct boundaries | Releasing control and removing the body are separate steps |
| Spaceport devices | [Ball slot and recorder callbacks](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/level_specific_code/Arnika.cpp#L1079-L1117) use location variables and NPC script notices | [ArnikaScript bindings](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/level_specific_code/Arnika.cpp#L198-L206) and [methods](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/level_specific_code/Arnika.cpp#L1031-L1067) give the level behavior an owner | Use typed interactable devices and a small Arnika module |
| Exact dialogue and trigger conditions | The callers delegate to external level and NPC script content | The modern wrappers do not supply the missing retail data | Verify the real content before declaring exact entry predicates, item debits, wording or fact order |

The official *Wizardry 8: Sybex Official Strategies & Secrets*, monastery
walkthrough p. 125 and spaceport walkthrough p. 227, corroborates the named
encounters and puzzle solution. A [readable copy](https://www.scribd.com/document/421808613/Wizardry-8-Sybex-Official-Strategies-Secrets)
supports those player-facing facts; it does not prove the hidden script
conditions. We should describe the monastery example as the **Aletheides/Savant
henchman scene**, rather than inventing a direct Dark Savant confrontation.

## Owners and module boundaries

| State | Owner | Lifetime |
| --- | --- | --- |
| Dialogue lines, choices, predicates and authored actions | Immutable dialogue assets | Content |
| Campaign facts, journal and NPC memory | CampaignState or the persistent NPC | Campaign |
| A device's repaired/activated state | Device component; dormant level record while unloaded | World object |
| Current dialogue continuation and offered item | Conversation | One conversation; item ownership follows the inventory operation |
| Current authored scene step and required completion | SceneSession | Until this scene settles |
| Playing clip, voice, camera motion and their request IDs | Presentation entities/state | Playback |
| Dialogue buttons and text entry | UI root and children | Visible UI |

The story module evaluates dialogue and applies explicit story operations.
The level module coordinates concrete actors, devices and camera cues. Inventory,
magic, trade and party modules perform their own mutations. Presentation reports
the completion of an identified request. None of those modules should call a
generic string-named script action to do ordinary item movement.

A terminal can reuse dialogue UI without being a living NPC with HP, hostility,
navigation and a monster body. Give it an interaction definition and device state.

## Example 1: the monastery encounter

### Accepted entry and calling the henchman

The external script decides when to raise
ALETHEIDES_CALLS_HENCHMAN. The recovered fact dispatch queues CALL_HENCHMAN.
The action ends the present dialogue, enters scripted world action, finds the
first group whose monster definition is 0x112, and starts its leader's special
cycle with callback 0x12. The
[lookup](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/MonsterGroup.cpp#L1174-L1192)
matches the monster definition, not a persistent group identity. Those source
IDs belong in the retail mapping, not throughout the new gameplay code.
[Source: fact/action paths above.]

Proposed operation sequence:

1. **Accept the authored action.** Resolve the intended actor and waypoint through
   content IDs, validate that this scene can start, and end the old Conversation.
   Capture the actor reference in the new SceneSession. Do not rely on whatever
   NPC happens to be selected later.
2. **Acquire the required control hold.** Disable party actions that would
   interfere with this staging. The scene owns that hold; it must release it on
   every exit path. Rendering, audio and the required playback continue.
3. **Request the special animation.** Presentation returns a PlaybackId.
   SceneSession becomes Calling with the actor and PlaybackId. No quest fact is
   committed merely because the renderer started drawing a clip.
4. **Await that clip's meaningful completion.** The original uses cycle 0x12.
   Match that milestone through the imported clip definition. An unrelated
   monster finishing a clip cannot advance this scene.
5. **Apply arrival once.** Resolve NP_HENCHMEN, direct the camera there, spawn or
   activate the henchman defined by the content mapping, and open its bound
   dialogue/script notice. The decomp spawns one monster of definition 0xdc.
6. **Remove the departing actor's world presence.** The original puts the callback
   actor at the origin and marks it parked. The new implementation removes or
   deactivates its body and presentation at the same logical milestone. Keep
   whatever persistent NPC/story identity the campaign still needs; there is no
   reason to hide a collidable actor at coordinate zero.
7. **Continue the henchman's dialogue.** The SceneSession can wait for a specific
   Conversation while dialogue owns line progression, voice and choices. Apply
   facts only at their authored locations.

Steps 5 and 6 belong to a single readable scene operation with deliberate
ordering, not independent unordered observers. Assets and named anchors should
be validated when loading the scene. If content is missing at runtime, release
the hold and return a visible failure; do not mark the scene complete to conceal
the missing actor.

### Leaving the scene

The HENCHMAN_DISAPPEARS fact dispatches HENCHMAN_LEAVES. This closes dialogue,
starts the henchman's special cycle, registers the departure callback, and
**releases the main-game target state immediately**. The actor is parked later,
when that callback runs. [Source: departure paths above.]

The [clear helper](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/NPCInteractionSubscreen.cpp#L4817-L4830)
clears the world-cursor gate and movement stop and resets the target cursor.
It does not wait for the departure callback. Other independently active control
holds still retain their own authority in the rewrite.

Represent that as Departing with the actor and PlaybackId, after releasing the
scene's party-control hold. The departure clip may still run while normal input
is available. When its identified milestone arrives, remove the body and finish
the scene. A single blanket “lock input until all animation ends” policy would
change this behavior.

A small scene-specific enum is sufficient:

~~~rust
enum MonasteryStep {
    Calling { actor: ActorId, playback: PlaybackId },
    Talking { conversation: ConversationId },
    Departing { actor: ActorId, playback: PlaybackId },
}
~~~

The actual transition functions can stay in
game::world::levels::monastery. This sketch describes the recovered staging;
the exact script entry and line-dependent facts remain content verification work.

### Completion, interruption and persistence

The initial native save policy refuses manual saving while this sequence has
an unresolved consequence. Autosave may wait for a settled boundary; there is no
general pending manual-save queue for scenes. It does not serialize an arbitrary
animation cursor or dialogue instruction pointer. Releasing movement before the
departure animation completes does not by itself make the remaining gameplay
removal safe to forget. See the [save policy](persistence-and-travel.md).

Completed story facts and final actor presence are saved. A load restores those
values without replaying the scene's arrival/departure operations. Reopening a
dialogue must not respawn another henchman.

On a failed asset load or explicit session teardown, clean up scene-owned camera
and control state. If skip is later supported, it must advance through the same
logical consequences exactly once; a generic “despawn cutscene entities” shortcut
cannot decide which persistent NPCs or facts should remain.

Do not use ALETHEIDES_DISAPPEARS as a guessed completion flag for this monastery
scene. Its name alone does not establish that relationship; the recovered handler
also participates in later-game content.

## Example 2: the Arnika spaceport puzzle

### Player-facing behavior and independent facts

The guide's spaceport walkthrough describes these connected discoveries:

- The useful Black Box from the wreck near Bayjin produces coordinates 112 at
  the analyzer.
- The Shiny Metal Ball repairs the Orbital Tracker.
- Entering 112 in the repaired tracker produces the reported value 10:24, which
  can be reported to Z'Ant.

Keep those as independent pieces of state. The tracker can be repaired before
the recorder is analyzed. Knowing the answer as a player does not automatically
require a “character has read the coordinates” predicate unless the actual
dialogue script supplies one.

A small native record might contain the tracker's repair state, the analyzer's
persistent state, and separately recorded discoveries. Avoid one linear
PuzzleStage whose variants prohibit a valid alternate order. Equally, avoid
copying the same repair flag into a quest component, a terminal component and
the journal; the journal is a consequence/readout of the relevant authority.

### What the callbacks do and do not prove

ArnikaBallSlot creates the BallInserted location variable only once, constructs
an item-0x240 payload and sends it to a kind-0x16 NPC script notice. It does not,
by itself, prove the identity validation and inventory debit for the player's
offered ball.

ArnikaFlightRecorder refuses to queue its notice while NPC dialogue is already
active; otherwise it forwards the held item, if any, to the kind-0x14 script.
It returns false even after queuing that notice. That boolean is a trigger
contract, not sufficient evidence that the player's interaction failed.
[Source: Arnika callbacks in the comparison table.]

There are also two distinct retail facts, ARNIKA_BLACKBOX_RECEIVED (0xc4) and
ARNIKA_BLACK_BOX_RECEIVED (0xc5), in the
[fact definitions](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/include/wiz8/fact_state.h#L209-L210).
Preserve the distinction in the importer until the actual scripts establish
their purposes. A spelling cleanup must not silently merge them.

### Ordered interaction flow

| Step | Proposed responsibility | State change |
| --- | --- | --- |
| Point at the device | World interaction query | Select the terminal/slot, not an arbitrary NPC |
| Choose Use or offer an item | UI/interaction intent | Keep the specific device and ItemInstanceId; selection does not consume |
| Validate availability | Arnika interaction function | Check repair/current conversation conditions and the real content predicate |
| Reject an offer | Same function | Explain rejection; ownership and quantity remain unchanged |
| Accept an offer | Inventory operation coordinated by the device function | Transfer or consume the specified item exactly as the content requires; commit the corresponding device state once |
| Play response | Conversation/presentation | Display the analyzer/tracker's lines and sounds |
| Record discovery | Story operation at the authored point | Update the relevant knowledge/fact/journal values |
| Enter coordinates | Terminal dialogue/input state | Validate the entered value and device state; do not invent a knowledge prerequisite |
| Produce the result | Arnika rule/dialogue operation | Commit the authored result and display it |
| Report to Z'Ant | His dialogue/story logic | Evaluate the appropriate facts, then apply his response/reward once |

The exact accepted-offer contract must be recovered from the retail trigger and
dialogue content before implementing it. Until then, the documentation specifies
the ownership boundary, not an invented “always consume any clicked item” rule.

After acceptance, closing a window cannot refund a consumed quest item while
leaving the tracker repaired. Before acceptance, cancelling merely removes the
offer/draft. An item held for an offer must still have one explicit owner and a
defined return path; see [inventory](inventory-trade-and-curses.md).

### Save, travel and revisit

Persist repair state, the relevant story facts and any still-owned quest item.
When the level reloads, derive its repaired appearance and available interactions
from that state. Do not replay BallInserted's side effects as a load-time event.

Ordinary travel is unavailable while an unresolved interaction owns a required
item transfer or scene consequence. Once settled, the terminal's level record
can become dormant while campaign knowledge remains available to Z'Ant elsewhere.

## Focused implementation checks

1. An old or unrelated animation completion cannot advance the monastery scene.
2. Calling the henchman twice cannot create two actors or repeat a one-time
   consequence.
3. Movement becomes available at the recovered departure boundary, while the
   body remains until its own completion.
4. A save/load after the scene restores actor presence without replaying it.
5. Repair-first and analyze-first puzzle paths both work.
6. An incorrect offered item remains owned by the player; an accepted item is
   consumed/transferred exactly once.
7. A busy Conversation rejects or defers a device request according to its own
   contract, without overwriting the current speaker.
8. The two similarly named Black Box facts remain distinct through conversion,
   saving and dialogue evaluation.
9. Revisit derives repaired visuals from saved state without re-awarding a
   discovery or replaying inventory mutations.

These checks verify the behavioral boundaries. They do not justify building a
general-purpose cutscene, quest or transaction framework first.

