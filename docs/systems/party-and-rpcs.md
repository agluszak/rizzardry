# Party members, recruitable NPCs, and actor identity

## Decision and evidence status

Use one spatial Party entity and independent character entities. The PartyRoster resource is the sole authority for membership and formation: six regular-character places and two recruitable-companion places. PartyInventory is a separate resource for the shared bag, purse, and physically held item. The Party entity owns spatial pose and body; party members do not each need a world collider or independent walking pose.

Party-wide actions, trap targets, recovery and incapacitation checks must
iterate `PartyRoster` and resolve those entities. Querying every entity with
an adventurer/Character component includes off-party named NPCs and is not a
membership test. Use roster order for gameplay draws that depend on order;
rendering order or ECS query iteration is not authoritative.

A named NPC keeps the same ActorId and campaign-lived entity when recruited, dismissed, moved by a quest, killed, or restored from a save. Recruitment changes membership and active role; it does not create a second identity. Named NPCs on unloaded maps retain their gameplay data but have no active body, AI, navigation, or per-frame condition simulation.

**One identity does not imply one numeric combat profile.** The inspected sources maintain a recruitable character profile and a separate world-monster representation with specific conversion rules. The proposal retains that semantic distinction on one actor while removing the copied full-character ownership protocol.

This document is a proposed Rust design, not an implemented feature. The comparison is pinned to [wizardry-8-decomp 075cbd0][decomp] and [whizzardry-8 9c433ec][modern]. Source bodies and callers were inspected; the acceptance scenarios below have not been run against a Rust implementation.

## What the two C++ implementations establish

| Concern | Original behavior | Modern C++ comparison | Rust consequence |
| --- | --- | --- | --- |
| Party capacity | Regular members use slots 2–7; NPC companions use slots 0–1. Adding copies a character, updates formation/order and derived statistics. [D1] | Optional membership replaces some occupancy flags, but character and row arrays remain. [M1] | Preserve six-plus-two rules without making the historical slot numbering the actor identity. |
| Recruitment | Copy the retained NPC character into a party slot; copy nonzero world-monster conditions; remove its world monster; mark it grouped. [D2] | The same transfer remains, with owned optional character storage and typed monster removal. [M2] | One named actor, explicit role transition, no new character entity. |
| Dismissal | Copy active party character back to NPC storage; remove party membership; create an NPC monster if appropriate; transfer conditions; return eligible non-wanted gear. [D3] | RAII replaces script cleanup and inventory owns store/drop operations; full-character copy-back remains. [M3] | Keep the complete operation and conversion rules, remove the duplicate full-character copy. |
| Persistent NPC identity | Story NPC state, party slot, monster location, and several presence/binding flags are coordinated. [D2][D3] | World construction prepares NPC bindings before installing them, but still connects story records to world monster records. [M4] | A stable actor and optional active-map body replace the binding lifecycle. |
| Region refusal | Awareness/condition, region restrictions, partner rules, and delayed post-entry consequences matter. [D4][D5] | The rules remain; some continuations use typed dialogue actions. [M5] | Preserve those stages in explicit party/story operations rather than a blanket travel ban. |
| Death | A dead member can remain in the party and be targeted by Resurrection; confirmed dismissal has its own dead/missing paths. [D6][D7][D16] | Death effects are centralized in live_party; they still notify NPC state and combat. [M6] | Death, dismissal, body removal, and permanent story removal are different operations. |

Whizzardry's retained character snapshot is not evidence of a second simultaneously active party member. It has a defined role and copy-back behavior. Removing its entire semantic content without checking the conversions would change gameplay, even if the resulting code looked cleaner.

## Owners and proposed components

These are domain groups; split them into smaller components only where queries or update frequency benefit. They are not a requirement to create a Rust type for every field.

| Data | Owner | Authority / lifetime |
| --- | --- | --- |
| ActorId and immutable definition reference | Character/NPC entity | Campaign identity; save references use ActorId |
| PartyRoster and formation/order | PartyRoster resource | The only membership/order authority; counts derive from roster |
| Shared purse, bag, and physically held item | PartyInventory resource | The single owner of those values; no second inventory attached to the Party body |
| Party pose, collision shape, movement | Party entity | Shared physical movement; member targeting uses formation and combat rules |
| AdventurerState | Regular character or recruitable NPC entity | Progression, attributes/skills, learned spells, and retained party-role pools/conditions; active only while that actor is a party member |
| Inventory and Equipment | Character/NPC entity | One set of adventurer item values; no copied backpack in an NPC snapshot |
| NpcState | Named NPC entity | Dialogue memory, disposition, quest flags/cooldowns and persistent story state |
| MerchantStock | NPC entity, when applicable | Separate from personal adventurer gear; persists across roles and maps |
| WorldCombatState | Off-party NPC entity | World-role pools/conditions/combat state based on its monster definition; does not duplicate progression, learned adventurer spells, or inventory |
| WorldPlacement | Off-party NPC entity | Map and pose; no separate world pose while traveling as a party member |
| Active-map body and AI/physics components | Actor or its owned presentation children | Only while placed in the loaded map and allowed by life/story state |
| Region discomfort / delayed RPC reaction | RPC state plus a campaign deadline or explicit pending action | Saved if it persists at supported save boundaries; not a UI-only timer |
| Pending recruitment/dismissal choice | Interaction state | ActorId, intended operation and needed continuation; expires with that interaction |

Do not keep a manually synchronized reverse MemberOf list as well as PartyRoster. This party is tiny; membership queries can inspect its authoritative roster. WorldPresence/active-body markers are derived runtime participation, not another membership ledger.

### The active role is explicit

A recruited RPC's AdventurerState is the party-combat/aging authority. Its WorldCombatState and WorldPlacement are absent. Its progression, items, NPC memory and stock remain on the actor.

An off-party recruitable NPC has a retained AdventurerState and an active or dormant WorldCombatState. These are semantically different profiles, not routinely synchronized mirrors. The world profile uses the monster/NPC content rules. The retained adventurer profile preserves what the NPC will bring when recruited again.

The shared actor-level Inventory/Equipment ownership is a **proposal**. It represents the retained adventurer possessions exactly once. It does not make world NPC attack damage, armor, spells, merchant stock, or loot automatically come from that gear. The original dismissal creates a world group from the NPC's monster species; it does not rebuild a monster from the adventurer's equipment. [D3]

The original conversion helpers copy nonzero conditions, including the poison argument. They do not proportionally copy ordinary HP/stamina between role maxima. Dismissal has a separate indefinite-unconsciousness stamina assignment. Exact retained-pool and condition-overlay behavior must therefore remain in the recruitment/dismissal functions and their comparison scenarios. A generic "switch role and keep all current stats" would be incorrect. [D8]

For all named actors, being retained in ECS is a lifetime choice. Only active-map WorldCombatState or party-member AdventurerState participates in the corresponding simulation. Revisit catch-up follows the map rules described in [Persistence and travel](persistence-and-travel.md); it does not also age the inactive adventurer profile.

## Ordered example: recruit a world NPC

Suppose a recruitable NPC is standing in the active map, the party has one free companion place, and dialogue reaches the join action.

1. **Resolve the actual NPC actor.** The dialogue holds ActorId, not the currently highlighted monster, a display name, or a party slot that may be reused. Resolve its current availability before applying the action.
2. **Evaluate the join rules.** The NPC must be recruitable. The original rejects a restricted current region and several story-specific cases: Glumph after the scuba mission, unpaid Sexus, Madras before the join fact, and Rodan/Drazic after peace. A positive minimum party level uses the average of occupied, positive-HP members whose highest condition is below Asleep. Script/dialogue prerequisites still run at their existing stage. [D9]
3. **Reserve a free companion place.** Lack of room refuses without removing the NPC body or modifying inventory. There are two companion places; an empty regular-character place is not implicitly an extra RPC place. [D1]
4. **Apply the role conversion.** Preserve AdventurerState's progression and inventory. Apply the source-defined nonzero world-condition overlay through the condition rules; activate the adventurer profile and settle its derived stats/skill availability. Do not copy a world monster's numeric HP as though it were a character with the same maximum. [D2][D8]
5. **Change participation in one operation.** Remove world AI, collider/body, current world targets and WorldPlacement/WorldCombatState as appropriate. Put the same ActorId in PartyRoster with its formation/order entry. Update script availability for a grouped NPC.
6. **Settle party consequences.** Recompute derived equipment/condition effects and shared load distribution, update legal action targets, and apply required story changes in the explicit operation.
7. **Present the outcome.** UI receives the settled membership change, refreshes portraits and closes/continues dialogue according to the script. Recruitment success cannot depend on a UI observer being installed.

The irreversible gameplay point is step 5 together with its required consequences, not the moment the player merely opens a dialogue option. Cancelled dialogue or rejected capacity leaves the existing world role in place. If presentation is closed after the operation committed, closing it cannot undo recruitment or return old NPC data.

A fresh NPC's adventurer template is initialized once from definition data: profession/level, attributes/skills, spells and equipment, followed by derived passes. Re-recruitment must not rerun starting-equipment generation or reset earned experience. [D10]

## Ordered example: dismiss an RPC

Normal living dismissal is an ordered party operation, not a render-entity despawn:

1. **Accept the dismissal choice for a specific ActorId.** Recheck membership. Resolve whether this is ordinary placement near the party, a scripted departure with no local spawn, or the dead/missing branch.
2. **Prepare ordinary placement.** Find a valid position and required world-monster definition before removing membership. A failed normal placement leaves the member in the party. Original callers probe placement, but some low-level failure paths mutate before returning failure; the rewrite should not retain those accidental half-completed states. [D3][D16]
3. **Retain adventurer state in place.** The actor already owns its latest progression, party-role pools, conditions and equipment. There is no full-character copy to an NPC repository.
4. **Remove membership and dependent party activity.** Remove the roster/formation entry, cancel that actor's outstanding party commands and modal confirmations, choose a valid remaining selected character, and reconcile combat/action references if this departure is permitted in that context.
5. **Build the world role when required.** Create WorldCombatState from the NPC's monster definition, apply the explicit nonzero condition-copy rules, and perform the original indefinite-unconsciousness stamina handling. Apply requested neutrality. Attach WorldPlacement/body only for a living NPC whose departure calls for a local world presence. [D3][D8]
6. **Return possessions by the actual rules.** For equipment, return only items that can be unequipped and are not wanted by that NPC. For backpack items, return those not wanted. Store/merge into the shared inventory and handle overflow through the item operation; preserve every unplaced remainder. Do not clear the inventory first and hope a UI callback stores it later. [D11]
7. **Commit dismissal history and reactions.** Set the relevant dismissal cooldown/event state and publish the settled outcome. Persist the actor at its new placement.

The wanted-item rule checks up to three definition entries, including one generic-name category. While recruited, carrying more than one of a wanted item can allow a duplicate to be relinquished. Dismissal clears grouped membership before the return-items pass, so do not assume that the duplicate exception remains active there. [D12]

Merchant stock is not emptied into the party during dismissal. A merchant's outstanding crafting output is also not another copy of its adventurer backpack. Both remain on the same NPC under their own stock/service rules.

### Cursed equipment and dead wearers

Known curse, cursed definition, current equipment location, and temporary curse release are separate facts. The original removal predicate blocks an unreleased cursed item in active worn slots while the wearer is alive; alternate weapon slots are exempt, and dead wearers are exempt. Therefore a living RPC can leave with otherwise unwanted cursed worn equipment because it cannot be removed. A dead member's equipment can be recovered under the dead-dismissal path. [D13][D19]

Dismissal uses the common item move operation. That operation must retain the earlier plan's identification and curse-release rules; moving an item clears the temporary release, rather than permanently removing its cursed definition. Do not implement a second item-copy path solely for recruitment/dismissal. [D20]

## Ordered example: a companion refuses a destination

The decompilation calls the relevant definition mask "service_flags," but this path uses it as a region restriction. This is not the merchant-services system.

1. A normal travel request records destination/entrance and checks the two companion places before ordinary "enter map" confirmation.
2. Only occupied, positive-HP RPCs with highest condition below Asleep participate in the initial warning check.
3. An RPC warns if the destination region is restricted and the current one is not. A separate Rodan/Drazic rule checks for a healthy partner when leaving region band 13. Region bands are not interchangeable with raw map numbers. [D4]
4. If warnings were queued, the initial travel request returns and restores the trigger position. A separate warning confirmation is shown. [D18]
5. Cancelling that warning does not dismiss anyone. Accepting it queues the applicable NPC dismissal actions. The callback itself does not immediately call BeginLevelTransition; the warning/dismissal sequence and the actual map transfer remain distinct. Do not reinterpret the first confirmation as an already-committed journey. [D14]
6. When travel does commit, roster and possessions are captured after required departures. A dismissed actor remains placed on the source map, unless its explicit story action says otherwise.

Incapacitating an RPC is not equivalent to removing the restriction. At ordinary level entry the game resets a pending check. After more than 60 world seconds, outside party combat/surprise, it evaluates the companions again. Asleep, paralyzed or unconscious members keep the check pending. An aware RPC in a restricted region can receive the source's bound-NPC penalty: minus 20 to all seven attribute adjustments and all 41 skill bonuses, with reactions and partner-specific departures. [D5][D15]

Use a named gameplay concept such as RegionDiscomfort for that penalty rather than the decompiled "bound" terminology, which is unrelated to cursed equipment. Store the pending deadline on gameplay state. Hiding a portrait, opening camp, unloading a render body, or rebuilding a dialogue panel cannot make it disappear.

The initial warning, later refusal, paired-companion rule, and forced/scripted exits have different predicates. Keep these as small explicit rules; a single "NPC.allowed_maps contains destination" check loses the original consequences.

## Death, resurrection, and removal are separate

Death changes the existing actor's active-role state. Modern C++ centralizes party death consequences: condition cleanup, HP/stamina zeroing, clearing targets, combat turn/round consequences, party reactions, and the NPC death marker. It does not remove the character from PartyRoster merely because HP reached zero. [M6]

The original Resurrection effect cures the Dead condition on its valid targets. In the successful character branch it uses floor((effective Piety + effective Vitality) / 4) as a percentage of the respective HP and stamina maxima, with a minimum restored value of one; the RPC death marker is cleared. The ordinary spell/item-use path remains responsible for costs and cast failure. This is resurrection of the same actor, not constructing a replacement from its definition. [D6]

Keep the character's equipment and progression through party death. Equipment interactions can change while dead because the curse removal predicate has a dead-wearer exception. Resurrection does not undo an item transfer already performed while dead; it re-evaluates the restrictions on whatever remains equipped.

Confirmed dismissal of a dead or missing RPC is a separate path. RCSCommon records the reviewed slot only after the dismiss confirmation succeeds; the main screen then calls UpdateNpcPartyMember once. That function stashes a dead member's items and removes membership, or removes a missing member. Its name/comment must not be mistaken for an automatic per-frame deletion of every dead RPC. [D7][D16][D17]

For the native model:

- Death does not destroy ActorId or automatically relinquish all possessions.
- Dismissal of a corpse does not create a living world NPC.
- A world corpse/fade is presentation with its own lifetime; it does not own the only record of whether a named actor is dead.
- Scripted restoration or permanent removal changes the existing actor's story/life/placement state. Re-entering an authored map cannot resurrect it accidentally.
- An unrecruited NPC's world-role death must use the world combat/story loot rules. Do not automatically spill retained adventurer gear or merchant stock unless the original path does so.

The unusual original dead-dismissal path does not run the same full-character copy-back as normal living dismissal. Its retained-state and eventual reappearance behavior needs a targeted fixture; the rewrite must not guess that it is simply living dismissal with zero HP.

## Persistence and delayed references

Native saves capture each named actor once, including its retained adventurer profile and world profile when semantically present, inventory, NPC memory, and merchant stock. PartyRoster saves ActorIds. WorldPlacement is the actor's location authority.

A dormant map stores ordinary local actors and map state. It may refer to a named actor in a trigger/script, but never stores an old copy of that actor's progression, gear, stock, or role state. When the map is visited, resolve the existing actor and attach a body only if its current placement and life/story state permit it.

Queued actions use ActorId, ItemInstanceId, and interaction/session identity where required. An old "dismiss companion in slot zero" callback must not dismiss the new occupant of that slot after a different NPC joined. On consumption, validate that the intended actor is still in the required role and that the operation belongs to the current interaction. Cancellation deletes the pending choice; completion changes ownership once.

Role changes are complete domain operations. Avoid a chain of independent observers for "remove world monster," "add party character," "return gear," and "update story"; a missing or reordered observer could leave two active roles or lose possessions. Optional notifications follow the completed operation.

## Module relationships

The party module owns roster/formation changes and coordinates recruitment/dismissal. The character/rules module calculates adventurer statistics and condition transitions. Items owns all inventory movement and equipment restrictions. Story owns NPC-specific prerequisites, reactions, region restrictions and scripted moves. World owns placement and active-map body attachment. Combat owns accepted/executing actions and is asked to retire or reject actor references at the defined membership boundary.

The party module does not own renderer objects or serialized retail character images. The world module does not decide whether an NPC may join. UI supplies choices and cancellation, never the only copy of the character that is about to leave.

Related system contracts: [Persistence and travel](persistence-and-travel.md) and [Time, rest, and conditions](time-rest-and-conditions.md).

## Acceptance scenarios

1. Recruit with both companion places occupied. NPC state, body, stock and gear remain unchanged; an empty regular-character place does not bypass the cap.
2. Recruit, earn XP/change profession or equipment, dismiss, save/load, and recruit again. Progression and retained gear survive once under the same ActorId; starting items do not regenerate.
3. Use different world and adventurer maximum HP, wound/condition the NPC in each role, then cross roles. Results match the characterized conversion rather than a guessed percentage copy.
4. Dismiss a living RPC with wanted gear, unwanted gear, an unreleased cursed worn item, and a nearly full party inventory. Each item ends in exactly one valid owner or explicit overflow destination.
5. Kill an RPC in the party and resurrect it without dismissing. Membership, identity and progression persist. Recover cursed equipment while dead, resurrect, and verify the moved item is not restored by a stale snapshot.
6. Confirm and cancel dismissal separately for living, dead and missing members. A placement failure or cancellation never creates a second world actor or loses the roster entry.
7. Cancel a region-refusal warning. No RPC is dismissed and no map snapshot commits. Accept it and verify departures/gear handling occur before the eventual travel.
8. Carry an incapacitated RPC into a restricted region, recover it, and advance past the delayed check. Apply the original discomfort/partner consequences once; do not skip them because the initial warning was bypassed.
9. Recruit on A, dismiss on B, visit A, save/load, and revisit B. Only the actor's current placement can produce its world body; A's authored NPC record cannot restore an old copy.
10. Reuse a companion roster place while an old confirmation is pending. The stale operation is rejected by actor/interaction identity and cannot affect the new occupant.

## Remaining decisions and characterization work

1. Pin exact world/adventurer pool and condition-overlay behavior through wounded, poisoned, unconscious, dismissed and re-recruited fixtures. Preserve semantically distinct state without reinstating two full Character copies.
2. Characterize dead/missing dismissal, later story restoration, and loot/equipment recovery for named NPCs killed outside the party. Do not infer these from the normal living-dismissal function.
3. Verify region-warning acceptance/retry timing, the paired Rodan/Drazic routes, and forced/scripted travel. Keep first-warning and post-entry rules distinct.
4. Decide the ordinary placement-failure user experience. The proposed invariant is to keep the actor in the party until placement can succeed; intentionally unsafe legacy partial-mutation behavior is not a native ownership contract.

## Primary sources

Evidence links above point to the pinned source revisions and relevant line ranges. They support the original/modern behavior; the Rust identity and role ownership are explicitly proposed.

[decomp]: https://github.com/agluszak/wizardry-8-decomp/tree/075cbd0b853c4670e94f6ae4fe48d53b52b5e819
[modern]: https://github.com/agluszak/whizzardry-8/tree/9c433ec9725877a963cb43bb1df13d5d084ed873
[D1]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/GameplayCode.cpp#L1025-L1141
[D2]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Manager.cpp#L562-L622
[D3]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Manager.cpp#L451-L516
[D4]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Manager.cpp#L2627-L2777
[D5]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Manager.cpp#L902-L1035
[D6]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Magic%20Effects.cpp#L1693-L1752
[D7]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/RCSCommon.cpp#L556-L592
[D8]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/ConditionsAndEnchantments.cpp#L819-L866
[D9]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Manager.cpp#L738-L791
[D10]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Manager.cpp#L1416-L1478
[D11]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Manager.cpp#L416-L449
[D12]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Manager.cpp#L382-L413
[D13]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/PC%20Item.cpp#L1289-L1303
[D14]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Scripting.cpp#L2033-L2039
[D15]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Manager.cpp#L2223-L2263
[D16]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Manager.cpp#L519-L559
[D17]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/MainGameScreen.cpp#L3171-L3174
[D18]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_screens/MainGameScreen.cpp#L4860-L4891
[D19]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/PC%20Item.cpp#L220
[D20]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/PC%20Item.cpp#L2770-L2806
[M1]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/rules/party.cpp#L1-L38
[M2]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/NPC%20Manager.cpp#L547-L601
[M3]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/NPC%20Manager.cpp#L403-L504
[M4]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/NPC%20Manager.cpp#L1147-L1224
[M5]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/NPC%20Scripting.cpp#L694-L770
[M6]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/live_party.cpp#L179-L211
