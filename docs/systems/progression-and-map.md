# Progression and map notes

## Design decision

These screens edit bounded values and submit ordinary operations. A level-up
draft is not a second live character. A map marker is not a world actor. Use
native Bevy UI entities to present them, with screen-local input and draft state,
and keep committed gameplay data under its existing owner.

## Source comparison

| Subject | Decomp | Whizzardry | Proposed boundary |
| --- | --- | --- | --- |
| Level-up editing | [Screen constructor](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/CharacterScreen.cpp#L127-L147) copies the entire character | [CharacterDraft](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/include/wiz8/local_code/CharGeneration.h#L30-L75) owns editing operations, but [commit](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_screens/CharacterScreen.cpp#L429-L463) still copies the character back | Draft only the progression choices and preview inputs; commit to the existing actor |
| Page sequence | [AdvancePage](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/CharacterScreen.cpp#L358-L409) handles profession confirmation, optional spells, skills and the next earned level | [Modern screen](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_screens/CharacterScreen.cpp) retains this flow with a draft object | Preserve the meaningful gates without duplicating the old screen hierarchy |
| Derived stats | [Level initialization](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/CharGeneration.cpp#L149-L170) and [HP recalculation](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Health%20Stamina%20Mana.cpp#L1203-L1241) | [CharacterDraft implementation](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/CharGeneration.cpp#L151-L173) uses the same rule boundary | Preview and commit call shared rule functions |
| Notes | [Creation](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/AutomapScreen.cpp#L834-L849), [editing](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/AutomapScreen.cpp#L2175-L2253), [save/load](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/AutomapScreen.cpp#L1673-L1755) | [Automap](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/include/wiz8/world/automap.h#L21-L98) owns UTF-8 notes and captured records; [UI](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_screens/AutomapScreen.cpp#L928-L975) edits them | Persist map-relative values; UI holds an ID and a draft, not a pointer into a rendered blip |

## Example 1: level-up from eligibility to commit

### Owners

| Data | Owner | Persistence |
| --- | --- | --- |
| XP, total/profession levels, base attributes, skills, spell knowledge and debts | Actor's AdventurerState | Saved |
| Profession requirements, skill availability, XP and derived-stat rules | Immutable content/rule functions | Content |
| Intended profession, allocated points and chosen spells | LevelUpDraft on the screen root | Discarded on cancel; not saved as character progress |
| Current page, confirmation and focus | UI state | Transient |
| Actual inventory/equipment | Existing actor inventory | Saved; affected only by commit consequences |

Keep a baseline of the progression inputs needed to build the draft. Recheck
against the live actor when committing. This can be a small value/revision check;
it does not require a generic optimistic-transaction framework.

### 1. Decide whether the action is available

The source eligibility helper checks an occupied party slot, positive HP, the
condition gate and sufficient XP. The button adds combat, camp and portrait-mode
gates. See [IsCharacterReadyToAdvance](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/GameplayCode.cpp#L307-L320)
and [UpdateRcsLevelUpPanel](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/RCSCommon.cpp#L509-L521).

A proposed can_begin_level_up query returns eligibility and a useful reason.
The UI uses it for the button; begin_level_up rechecks it when accepting the
request. Do not turn a stale enabled button into authorization to level up a
dead or no-longer-present actor.

This uses the current party member's active AdventurerState, including for an
RPC. The same actor's off-party WorldCombatState is not a progression input, and
an inactive retained adventurer profile is not eligible merely because the named
NPC entity still exists. Recheck PartyRoster at commit; see
[party roles](party-and-rpcs.md).

The level-up screen's world hold follows the shared advancement policy.
Opening a screen should not give its UI code permission to stop or resume every
clock independently.

### 2. Build one next-level draft

Build the projected next level from the actor's retained progression. Increment
the projected total level and chosen profession-level counter once. Compute the
available pools and any debt handling from the rules.

The ordinary non-creation point pools are six attribute points and nine skill
points before the relevant adjustments. They come from the
[pool initializer](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/CharGeneration.cpp#L90-L118),
not a random HP/skill award on every page visit. Spell choices depend on the
profession and magic rules.

Going back to a page reuses or deterministically recomputes this same draft; it
must not increment the projected level again. The original
[SyncCharacterForPage](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/CharacterScreen.cpp#L474-L490)
has a specific guard around initialization. In Rust, express the lifecycle
directly rather than reproducing a fragile page-index convention.

### 3. Allocate attributes and consider profession change

The attribute page edits point allocations and previews qualification for
professions. Use the same limits and eligibility calculation for enabled
controls and final validation. Changing profession may change skill
availability, spell options, debt and derived stats, so recompute those from the
draft's baseline and current choices.

Do not append a new profession level, grant starting gear, or overwrite the
live character merely because the player clicked a profession button.
Profession changes have their own confirmation and realm-skill-debt response in
AdvancePage. Preserve the source's requirements and confirmations rather than
substituting a generic “class = selected enum” assignment.

The original allocation clamp takes excess points back in a defined order;
see [attributes](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/CharGeneration.cpp#L275-L310).
Recalculation should remain repeatable and must not consume campaign RNG.

### 4. Choose spells, then allocate skills

The spell page appears only when the draft awards spell choices. Validate each
selection against profession, spellbook, realm and level rules. Removing a
selection refunds only that draft choice. Advancing with unspent spell choices
uses the source's confirmation path.

The skill page displays currently available skills, limits and remaining points.
A profession or attribute change can invalidate earlier allocations; refund or
clamp them using the same explicit rule functions. Preserve point budgets and
debt across backward navigation.

For level-up, the original sequence is attributes/profession, optional spells,
then skills. Character-creation name/personality/portrait choices are not another
mandatory level-up step.

### 5. Commit the progression and its consequences

commit_level_up performs one bounded operation:

1. Resolve ActorId and check that the actor is still eligible and that the
   baseline progression has not changed.
2. Validate all allocation totals, limits, profession eligibility and selected
   spells; ensure the needed confirmations were accepted.
3. Apply the next-level progression and learned spells to AdventurerState.
4. Recalculate derived stats using the same rules as the preview.
5. Apply profession-item changes and resolve equipment that is no longer usable.
   Those are inventory operations, not a copy-back of a draft inventory. Settle
   derived stats and affected action choices after those equipment consequences,
   so removed gear cannot leave its old modifiers or weapon choices active.
6. Publish the outcome for UI, sound and portrait refresh, then close this draft.
7. If the actor still has enough XP for another level, open a fresh next-level
   draft from the newly committed state.

The original [commit](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/CharacterScreen.cpp#L546-L580)
runs finalization, upgrades the Gadgeteer class item and calls
UnequipUnusableItems. Whizzardry routes the class-item change through Inventory
and uses RecalculateCharacterDerivedStats after copying the draft. That is a
useful consolidation, but copying the whole character is not necessary for the
new design.

The modern [party settlement](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/live_party.cpp#L214-L245)
removes unusable equipment, updates party/character stats, and reconciles
affected hand choices in that order. Final settlement belongs inside the commit,
before UI observes its result.

The original [UnequipUnusableItems](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/PC%20Item.cpp#L3372-L3439)
checks usability across the twelve equipment slots, announces the unusable item,
moves it out, then tries the character's backpack followed by the shared pool.
It does not call the voluntary CanUnequipItem curse predicate. This forced
invalid-equipment path is therefore distinct from asking to remove otherwise
usable cursed gear.

The modern [unusable-item search](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/rules/inventory.cpp#L279-L291)
and [placement operation](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/inventory.cpp#L1354-L1384)
preserve that distinction and add an explicit overflow path: character storage,
shared pool, then a ground drop; if all placement fails, the item returns to its
equipment slot and is skipped for that pass.

The native operation must preserve every item exactly once. A full backpack is
not permission to delete gear or leave a renderer holding the only copy. Verify
the unusable cursed-item case and full backpack/pool/drop-failure case directly;
do not substitute the ordinary voluntary-removal gate for this consequence.

### 6. Preserve the actual health rule

HP recalculation adds the maximum-HP difference to current HP, bounded by the
source's rules. For example, 20/30 becomes 25/35 when the new maximum is 35.
It does not automatically become 35/35, and the inspected path does not roll a
new random HP gain. [Source: HP recalculation above.]

Preview must show the same result that commit applies. Vitals, conditions,
equipment and skill modifiers interact, so “new max HP” is not an independent
widget formula.

### Cancel and save boundaries

Cancelling before commit discards the draft. It cannot award a level, keep a
selected spell or modify an item. Once commit succeeds, closing the next-level
screen cannot undo the already-earned previous level.

Initial saves contain committed progression only. Finish or cancel a level-up
draft before using the ordinary save path; serializing an unfinished wizard is
outside the initial persistence policy. A save/restore after commit must not
repeat class-item upgrades or grant starting equipment.

## Example 2: placing a custom automap/minimap marker

### Original behavior versus the proposed extension

The recovered custom-note implementation is in the **full automap**. It stores
a two-dimensional location, layer and text; creation accepts fewer than 200
notes and a text length below 40 wide characters. The edit path uses a literal
underscore as its caret, allows moving the pending note, strips the caret on
Enter, and removes the pending note on Escape or empty submission.
[Source: note creation/editing above.]

Whizzardry gives the level's Automap ownership of note text and save capture,
and adds proper native text input handling. It still carries the trailing caret
marker in the edited string. Keep the ownership improvement; the new native
UI's caret is presentation, never part of saved note text.

Showing and creating these same annotations on the minimap is an **explicit
rewrite feature**, not a claim that the original radar already supported it.

### Representation

~~~rust
struct MapNote {
    id: MapNoteId,
    position: Vec2,
    layer: MapLayerId,
    text: String,
}

struct NoteDraft {
    map: MapId,
    position: Vec2,
    layer: MapLayerId,
    text: String,
}
~~~

MapNote values belong to the map's exploration/annotation record under
ActiveLevel or DormantLevels. The draft belongs to the active map UI. A
MapNoteId lets the UI refer to a note across view rebuilding without keeping
pointers to vector elements.

An icon rendered in the full map or HUD can be a Bevy UI entity. It refers to
the same note; it does not create a second note owner. A note does not need
Transform, HP, collision, physics or a world actor Entity.

Use UTF-8 storage and native Unicode text editing. Retail limits can remain an
initial presentation policy; they must not dictate fixed-size string buffers in
the save schema. Decide any changed count/length policy explicitly when building
the UI.

### Ordered placement flow

| Step | System or operation | Result |
| --- | --- | --- |
| Enter annotation mode | Map UI input | Show placement feedback; capture relevant pointer/keyboard input |
| Click a map location | Map projection helper | Invert the actual map viewport, pan, scale and rotation to a map coordinate |
| Check the location | Map UI/model query | Reject outside-map clicks and identify the selected layer |
| Begin typing | NoteDraft on the UI root | Store text and position only in the draft |
| Move the pending note | Map UI input | Update the draft position using the same inverse transform |
| Confirm nonempty text | add_map_note | Validate policy and insert one MapNote in the map record |
| Cancel or submit empty text | UI | Discard the draft |
| Render full map and minimap | Presentation | Project the same committed note into each view |
| Edit/delete a committed note | update_map_note or remove_map_note by ID | Change the authoritative value and refresh both views |
| Travel/save/load | Map persistence | Preserve committed note coordinates, layer, ID and text |

Do not save screen pixels. For a rotating minimap, the inverse transform must
include rotation as well as scale and center. Keep the party's current altitude
separate from the map's explicit layer selection; overlapping floors must not
collapse into one marker.

Typing a note captures gameplay bindings, so a letter cannot also move the party
or activate a combat hotkey. When editing ends, release only this UI's input
context and retain any independently active pause/modal state.

A player-created note is player knowledge. Placing one should not mark nearby
cells explored, reveal enemies, or alter quests. Visibility and clipping are
presentation decisions over the same stored annotation.

### Persistence and verification

Save only committed note values. An unfinished draft can be discarded when its
screen closes; it does not masquerade as a saved empty marker with a caret.
Revisit restores notes even if the full-map and minimap render entities have
never been instantiated in this session.

Committed-only capture does not make every automap or note-editing mode
saveable. A save request still follows the [ordinary save gates](persistence-and-travel.md);
this screen adds no separate deferred-save queue.

Focused checks:

1. Forward/backward level-up navigation never adds an extra level or duplicates
   a spell choice.
2. Cancelling a profession change restores the previous draft budgets and leaves
   live equipment unchanged.
3. Preview and commit agree on HP, skill limits, spell eligibility and debt.
4. Full inventory plus newly unusable gear never loses or duplicates an item.
5. Multiple earned levels are committed one at a time.
6. Map click-to-coordinate-to-screen round trips under pan, zoom and rotation.
7. Overlapping map layers preserve separate markers.
8. Escape creates no saved note; Enter saves text without an artificial caret.
9. Unicode editing operates on user-visible text correctly and cannot trigger
   exploration/combat input.
10. Travel and save/restore preserve one note, while both map views can recreate
    their own temporary rendering entities.

