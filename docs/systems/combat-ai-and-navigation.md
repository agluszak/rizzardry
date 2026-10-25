# Combat, encounter AI, and navigation

## Status and scope

This is the proposed design for a fresh Rust and Bevy implementation. It records
which behavior the two C++ implementations establish, which ownership decisions
Rizzardry makes, and which details still need an original-game observation. No
runtime code or new tests are implemented by this document.

The main decision is to keep three things distinct:

- **World behavior:** perception, patrols, noise investigation, faction reactions,
  group coordination, and movement continue without a party fight.
- **Engagement:** a monster can become involved in an encounter before the party's
  `Battle` exists. Allegiance and engagement are different facts.
- **The party's fight:** `Battle` owns round progress, scheduled actor turns, the
  party movement action, and the action currently executing. Continuous combat
  advances this same model automatically.

The party has one spatial body. Characters are independent actor entities with
their own health, skills, conditions, equipment, and choices, but do not each
receive a movement controller. Monsters have spatial bodies. A group coordinates
monsters; it does not own a second copy of their health, position, or initiative.

The source baseline is pinned to:

| Repository | Commit | Use in this document |
| --- | --- | --- |
| `wizardry-8-decomp` | `075cbd0b853c4670e94f6ae4fe48d53b52b5e819` | Recovered rules and sequencing; comments and recovered names still require scrutiny. |
| `whizzardry-8` | `9c433ec9725877a963cb43bb1df13d5d084ed873` | The current C++ interpretation, useful ownership improvements, and concrete remaining coupling. |

Reading the same rule in both repositories is corroborating source evidence,
not an independent retail runtime test. This document does not claim that every
reconstructed branch has been exercised in the original executable.

## What the comparison tells us

| Concern | Recovered implementation | Current Whizzardry implementation | Rizzardry decision |
| --- | --- | --- | --- |
| Perception | `Sight.cpp` stores monster-to-party, party-to-monster, and monster-to-monster observations, with remembered visibility and last-seen times. | The same distinctions remain in `Sight.cpp`; world stepping now has an explicit sight interval. | Keep directional observations and memory. Make geometry queries independent of UI and rendering. [D-sight] [W-sight] [W-step] |
| Factions and provocation | Group defaults come from NPC disposition, faction reputation, or a species' proximity-hostility rule. Provocation can spread to visible same-faction groups. | The same rules remain in `MonsterGroup.cpp` and `combat/hostility.cpp`. | Keep campaign reputation, local group provocation, and actor relationship overrides separately. A single `team` number is insufficient. [D-disposition] [W-disposition] [D-alert] [W-alert] |
| Engagement | Group entry creates monster combat state; the party fight is a separate transition. | A monster owns optional `pCombat`, while `Game::combat` is a separate optional `Combat`. | Use actor `Engagement` independently of `Battle`; party-fight round counters have one owner in `Battle`. [D-group-entry] [W-group-entry] [W-monster-entry] |
| Turn execution | `UpdateCombat` switches on numeric action status and waits on camera, monsters, sound, spells, missiles, and scripts. | `CombatTurn` uses explicit idle/scheduled/resolving/finishing alternatives and `waitingOn`. | Keep the readable state machine and ordered readiness checks. Drop historical offsets, global lookups, and UI portrait fields from simulation state. [D-turn] [W-turn-types] [W-turn] |
| Movement and formation | One party movement phase changes which characters may still act. Formation changes reach and facing. | Movement is split into `combat/party_movement.cpp`; formation placement has one cell-grid authority in `rules/party_formation`. | One party mover and one scheduled party movement action; formation is logical placement, not eight character colliders. [D-party-move] [W-party-move] [W-formation] |
| Monster decisions | Real-time orders and round decisions are different functions. Decisions consider attacks, spells, protection, retreat, and valid targets. | Much of that decision logic remains in `local_code/MonsterAI.cpp`, still reaching `CurrentGame`, population, and spatial accessors. | Keep small decision functions over explicit inputs. Do not translate the global lookup network or add a general behavior-tree framework. [D-ai] [D-ai-round] [W-ai] [W-roam] |
| Navigation | Navigators combine body movement, routes, linked-group movement, and path probes through the octree. | World-bound `Navigator`, typed path results, and per-search `PathSearchScratch` clarify some ownership; the old flag-heavy route machinery remains. | Keep geometry, clearance, conditional links, desired separation, and blocked-path outcomes. Replace the navigator class hierarchy and magic flag combinations. [D-navigation] [W-navigation] [W-nav-search] |
| Long-lived magic | Hypnotic Lure controls movement outside combat; summons have identity, disposition, and explicit cleanup. | Active-lure lookup deliberately selects the newest unexpired effect even while old visuals are retiring. | `game::magic` owns control-effect lifecycle and calls AI/combat helpers. A world effect is not owned by the current `Battle`. [D-lure] [W-lure-lifetime] [W-lure-replace] [D-summon] |

## State and its single authority

These are logical boundaries, not a requirement to create one Rust type for
every row of an algorithm. A component is useful when its presence or independent
mutation matters to systems. Small related values can stay in one component.

| State | Owner and representation | Important boundary |
| --- | --- | --- |
| Party position, heading, collision shape, movement intent | The single party entity: `Transform`, body/controller components, and party motion state. | The camera derives from party pose. Character formation does not create extra navigators. |
| Roster and live formation | `PartyRoster` resource, with an ordered roster and one formation grid referencing actor entities. | A formation editor holds a draft; the accepted/live layout remains authoritative. Facing and a remembered revival row have different meanings from cell occupancy. |
| Character or monster role statistics | Actor entity, with the applicable adventurer or world-combat profile, `Vitals`, `Conditions`, and derived statistics. | A named NPC can retain adventurer progression while using a distinct world-combat profile. Damage and targeting resolve its active role; they must not damage both profiles. |
| Active world presence | A level-presence/embodiment marker and spatial components for a world actor. | Campaign-lived NPC identity does not imply active AI. Absent and recruited NPCs are excluded from world movement, perception, collision, and monster-decision queries. |
| Group membership | `GroupMember` on each member, referring to the group entity. | This is the membership authority. Ordered member lists for a rule are derived; do not maintain a second mutable roster in the group. |
| Coordinated group state | A nonspatial group entity: leader, authored group relationships/orders, current local disposition, provocation time, and group engagement policy. | A leader reference is a role, not ownership of its followers. Leader death does not automatically delete the group. |
| Faction definitions and reputation | Immutable catalog data plus campaign reputation state. | Local provocation does not rewrite all campaign reputation. Turncoat or other actor overrides do not rewrite the base faction. |
| Perception memory | Actor-owned `Perception`: observations by subject, last-known positions, seen/recent/unseen state, and remembered noise. Party awareness can be stored on the observed monster as a distinct observation. | Opposite directions are separate observations. Current line-of-sight results are disposable; remembered observations are gameplay state. |
| Monster behavior | `AiState`: authored order, current roaming intent, cooldowns, current control response, and an accepted action choice where needed. | Authored patrol orders survive temporary pursuit/control. Do not replace them permanently with a movement target. |
| Encounter participation | Actor `Engagement`, referring to relevant opponents/group and its current encounter state. | May exist without `Battle`. It owns no duplicate party-fight phase counters. |
| Round and turn progress | Optional `Battle` resource: round, phase, ordered actor-round entries, current action, party movement progress, and combat result bookkeeping. | `Battle` references actors; it does not own their health, inventory, or bodies. Round counters live here once. |
| Chosen versus executing action | The character/AI action choice is accepted intent; `Battle.current_action` owns the snapshot that has begun executing. | A later UI/AI change cannot rewrite an in-flight spell or already-started attack. |
| Path and local motion | Spatial actor components containing movement profile, goal, owned waypoint vector/cursor, steering state, and completion reason. | A route is an owned value, not an entity. It is not another authority for the actor's current pose. |
| Geometry and path topology | Level resources and collision-world data, with versioned dynamic links for doors/bridges. | Queries return values. A path search does not mutate patrol history, reputation, or the gameplay RNG. |
| Lure or other spatial continuing effect | An effect entity owned by the active level, with source, position, power, remaining duration, and effect-specific state. | A monster's response refers to that effect's identity. A cosmetic old lure visual is not a second active lure. |
| Summon | A normal spatial monster actor plus summon provenance and the relevant caster dependency. | Current allegiance and the fact of being summoned are distinct. The summon is not a party roster member. |

Items remain values in inventory containers. Conditions remain values in an
actor's conditions component. Neither acquires an entity merely because AI or
combat reads it.

For observable ordering, iterate actors by stable spawn/order keys and party
members by roster order. Bevy's incidental query order must not determine which
actor wins an initiative tie or receives the first gameplay random draw.

## Runtime order and module boundaries

Use a small ordered simulation chain. Systems that share authoritative writes
must have an explicit order; unrelated rendering work can run independently.
The source's overall step already distinguishes combat stepping, sight/group
maintenance, and world motion, although current code still accesses them through
many globals. [W-step]

| Module | Systems at the boundary | Ordinary functions inside them |
| --- | --- | --- |
| `game::world::perception` | Refresh due observations and age memory. | `assess_detection`, `trace_sight`, `hear_noise`, `remember_observation`. |
| `game::story` faction rules | Apply authored reputation changes or explicit provocation, coordinating affected world actors. | `relationship_between`, `group_default_disposition`, `provoke_group`, `alert_visible_allies`. |
| `game::world::ai` | Update active roaming actors; choose a combat action when requested. | `choose_roaming_goal`, `choose_combat_action`, `collect_attack_options`, `choose_spell_target`. |
| `game::world::navigation` | Plan/replan invalid routes and advance permitted motion. | `find_route`, `can_reach`, `find_attack_position`, `steer`, `move_with_collision`. |
| `game::combat` | Enter/leave the party fight and advance its round/action state. | `begin_round`, `schedule_actor`, `begin_action`, `finish_action`, `settle_party_movement`, `finish_round`. |
| `game::magic` | Advance deliveries and continuing effects in and out of combat. | `resolve_spell`, `apply_lure`, `release_lure`, `spawn_summon`, `expire_effect`. |
| Presentation | Read results and advance the relevant sounds/animations/camera. | Format notices, choose animation clips, and report readiness for the originating action. |

`game::combat` asks AI for a choice and navigation for a route; neither navigation
nor a rendered mesh decides to start a fight. `game::magic` can change AI control
and actor state without a `Battle`. Spatial queries cannot call back into arbitrary
gameplay owners.

The state-changing step is deliberately ordered:

1. Apply accepted player/script choices and explicit world changes.
2. Advance due world-time consequences, then refresh observations needed by this
   decision step. Expiration happens before a decision reads the affected state.
3. Refresh group reactions and engagement; enter the party fight if its entry
   conditions are satisfied.
4. Advance the permitted battle action or roaming decision. Build a route only
   when that decision needs one.
5. Advance motion, projectile delivery, and semantic animation milestones in
   their required order. Resolve mandatory consequences directly.
6. Settle completed actions, invalidate affected observations/routes, and publish
   presentation results.

This is a proposed Bevy schedule, not a claim that all legacy updates may be
reordered without effect. During implementation, pin the concrete milestone order
with the scenarios below. The old every-fifth-frame group maintenance is not a
reason to make Rust gameplay depend on frame rate. Use a stable simulation cadence
and explicit work budgets; preserve meaningful delays and random decisions.

Do not turn this sequence into a queue of tiny commands replaying ordinary calls.
For example, `provoke_group` changes disposition, updates affected choices and
engagement, and returns its result before presentation receives a notice. Required
combat entry does not depend on a UI observer receiving that notice.

## Walkthrough 1: a patrol hears the party and starts an encounter

### Source behavior

The roaming AI distinguishes authored guard/patrol orders, hearing a noise,
investigating its location, facing toward it when a route/hearing test fails, and
pursuing or retreating from a detected party. Hearing uses a measured path and
closed-door hops, not just straight-line distance. A group can share its leader's
orders, and a group with an incapacitated member can suppress the ordinary roaming
choice. [D-ai] [W-roam] [W-nav-noise]

Seeing a monster on screen and that monster detecting the party are different
events. The source evaluates observer senses, facing, blindness, camouflage or
scouting, distance, and prior awareness. Loss of direct sight can leave recent
memory. Monster-to-monster perception matters once friendly creatures, hostile
creatures, and control effects coexist. [D-sight] [W-sight]

Group engagement also needs more than a hostility flag: the code considers
visible targets, attack/spell feasibility, movement range, and a route probe.
Reinforcements entering an existing party fight have another admission path.
Both implementations retain these distinctions. [D-engage] [W-engage]

### Ordered Rust flow

| Step | Reads and writes | Commit or cancellation rule |
| --- | --- | --- |
| 1. The party moves or produces an authored noise. | Read the party pose and noise parameters; update the relevant actors' remembered noise through `hear_noise`. | The noise is already an occurrence. Cancelling later movement does not erase that memory. |
| 2. Refresh a patrol's observation. | Query geometry and hearing paths, then write that observer's `Perception`. | A UI preview of sight/hearing is read-only and does not repeat the decision's random draw. |
| 3. Choose a roaming goal. | Read authored orders, group constraints, conditions, and memory; write `AiState` intent such as investigate, face noise, chase, or return. | Temporary investigation retains the patrol's original order and index. |
| 4. Plan and move. | Navigation produces an owned route with desired separation, or returns a concrete failure. Motion updates the actor pose. | A blocked route can lead to facing the noise, waiting/retrying, or returning according to the decision rule; failure does not teleport the actor. |
| 5. Refresh local disposition and relationships. | Read campaign faction/NPC disposition and local provocation. Actor conditions affect pairwise relationship evaluation. | A local hostile reaction can expire or be superseded by an authored faction change. It is not an unconditional campaign-wide war. |
| 6. Admit the group to an encounter. | Add/update `Engagement` for eligible active members and linked allied groups. | Group/actor existence does not depend on membership in the party's `Battle`. |
| 7. Start the party fight. | If active party members can fight and entry is not deferred by the relevant script/conversation, create `Battle`, record surprise, initialize choices/formation, and stop unrestricted party motion. | An NPC's combat-entry script can defer this transition. Do not enter the battle halfway and wait for a UI callback to repair it. |
| 8. Admit later reinforcements. | Reassess visible/reachable groups and register qualifying actors in the current round under explicit insertion rules. | An actor that dies or loses active presence before admission is skipped; it does not leave a dangling scheduled entry. |

Current Whizzardry makes the party transition a single `Game::beginCombat`
operation, but it still dismisses screens and reaches world/global state inside
that operation. Keep the complete transition; move presentation requests to its
result and pass the actual game data explicitly. [W-begin]

### Faction rules that must stay separate

`relationship_between(a, b)` must be able to account for base disposition,
faction identity, neutral actors, Turncoat, and special authored exceptions.
Two monsters with different party-relative dispositions do not automatically
ignore their shared faction. Likewise, a turned party member changes who regards
that individual as hostile. [D-hostility] [W-hostility]

On deliberate provocation, `provoke_group` updates the target group, linked allies
per the authored propagation rules, and actor states whose choices became invalid. Same-faction
notification is limited by the source's visibility rule; it is not a broadcast to
all faction members in every zone. Preserve this as a direct, bounded operation.
Whether a faction reputation change is also due is a separate story rule.
[D-alert] [W-provoke] [W-alert]

## Walkthrough 2: move the party around a corner during a round

### Round and continuous mode

The two combat modes share initiative, phases, action execution, and round-end
consequences. Continuous mode controls when the next round starts and some pacing
behavior; it does not replace phases with one independent cooldown per actor.
The recovered implementation waits on the same kinds of active work before
scheduling the next actor. Whizzardry's typed `CombatTurn` is a useful expression
of this existing lifecycle. [D-turn] [W-turn]

At round setup, actor initiative includes a small random term, with the ordinary
weapon-action adjustment restricted to the appropriate attack actions. A chosen
action is copied to an executing action when the turn begins, then revalidated
against current actor conditions and target availability. Changing a later choice
does not alter that executing snapshot. [D-initiative] [D-action] [W-turn]

### Party movement is a competing scheduled action

The original party movement phase uses the **minimum** living member initiative
after each member's random term. Both files misleadingly describe this as the
“best” initiative, but their bodies compute the minimum. Its phase is aligned to
the movement step schedule and participates alongside actor turns. [D-party-move]
[W-party-move]

| Step | Rust owner and operation | Behavior to preserve |
| --- | --- | --- |
| 1. Choose walk/run. | UI changes the pending party movement choice in `Battle`. | This is one party action, independent of individual attack/cast choices. |
| 2. Admit the choice to the round. | `start_party_movement` computes its phase, movement limit, and phase alignment. | Planning and executing are separate. Cancelling a merely pending choice clears it. |
| 3. Reach its scheduled phase. | `Battle` permits movement of the party's one body and starts progress accounting. | Monsters and character phases still belong to the same round. This is not free exploration with combat paused. |
| 4. Walk or run around the corner. | The party controller performs collision-constrained movement. `Battle` accounts for consumed movement and character stamina costs. | Running changes speed and the cost to remaining action opportunities. Encumbrance and disease affect member fatigue. |
| 5. Stop or exhaust the move. | `settle_party_movement` freezes the achieved pose, refreshes sight/engagement, and computes which characters can still act. | Already moved distance and spent stamina remain spent. A stop is not rollback to the original position. |
| 6. Revalidate following actions. | `begin_action` reads the current shared pose, actor formation cell, current sight, and target state. | The corner can invalidate a previously chosen shot/spell even though the UI accepted it earlier. |
| 7. Finish the round. | `finish_round` commits the accepted formation edit, advances the environment clock and relevant effects, performs recovery/practice, and refreshes engagement/targets. | Continuous mode can begin the next round automatically after readiness checks. |

The remaining-action rule is not simply “moving costs one action.” The original
computes spent movement from the gauge, multiplies that spent fraction by `1.5`
for running, and rolls each eligible, not-yet-finished member against the remaining
chance. Those that pass catch up to the current phase; others finish the round.
In continuous mode, the movement gauge can also advance by bounded synthetic
steps when the party remains stationary. Preserve or explicitly revise that
behavior; otherwise waiting without moving can stall the round. [W-move-progress]
[W-party-move]

There is another cancellation boundary: a movement action already admitted but
not started uses the phase-settling interruption path. The original catches
unfinished characters up to the combat clock. Do not treat all three operations
as the same button implementation: cancel pending choice, interrupt scheduled
movement, and stop an active move. [W-move-cancel]

### Formation and reach

Use the party body for world distance and collision, then apply logical formation
and facing rules in `assess_action_target`. The source shortens close-range reach
according to intervening formation rows. It also chooses different line-of-sight
tests for different action types. Monster-to-monster attacks use the two monster
bodies and their own observation, not the party-formation rule. [D-range]
[W-range]

The formation grid is the single record of occupied cells, as it already is in
current Whizzardry. A death can unseat a character while retaining the row used
for revival; that memory is not a competing live cell. Use a small pure formation
helper for packing, reseating, and row/facing calculations. [W-formation]

An area effect that includes the party still follows the shared-party spatial
test and per-character eligibility described by the magic design. Do not infer
individual blast colliders from the fact that melee reach uses formation.

## Walkthrough 3: a monster chooses a spell or a reachable attack

### Keep the decision policy readable

The existing AI is a collection of conditional policies rather than a universal
planner. It considers incapacity, a species' advance behavior, neutral return
behavior, range preferences, injury, special attacks/flee behavior, spell
availability, and finally valid ordinary attack options. Attack candidates can
include a particular attack mode and target. Protection, waiting, stealth-based
target avoidance, and follow-up attacks also influence the candidate set.
[D-ai-round] [W-ai] [W-ai-options]

Implement those rules as ordinary functions returning an action value. A small
local vector of candidates is enough when the rule needs enumeration. Do not
introduce a persistent blackboard, task scheduler, utility graph, or framework of
one-node commands merely to express these branches.

### Ordered decision and execution

1. **Establish eligibility.** The scheduled actor must still have active presence,
   a live world-combat profile, and conditions that permit the action. Resolve
   the actor by its actual identity, not whichever NPC currently occupies a
   party slot.
2. **Read the decision inputs.** Use the actor's effective profile, its observation
   memory, group policy, current caster dependencies, target relationships, and
   collision/navigation queries. Do not request every possible route every frame.
3. **Choose an action and target.** Run gameplay random draws in an explicit order
   and record the accepted choice. Spell eligibility and target checks are shared
   with `game::magic`; UI target previews call non-random assessment functions.
4. **If movement is needed, choose an attack position.** Request a route toward an
   acceptable distance/facing/sight region around the target. Melee approach,
   ranged positioning, backing off, and returning to an authored start point
   have different goals. A route to the target's exact center is usually wrong.
5. **Advance motion within its action budget.** Follow waypoints with local body
   avoidance and collision checks. The route can complete, become blocked, lose
   its target, or consume its permitted movement before reaching the goal.
6. **Reassess at execution.** A target may move behind a door, die, become neutral,
   or be replaced by a control effect after the decision. Revalidate the chosen
   action. Replan or select the rule's fallback only at the proper decision point.
7. **Resolve the action at its milestone.** Monster attack animation release can
   be a real gameplay milestone. The semantic action owns that milestone and the
   animation reports it to the originating action. Mandatory hits, projectiles,
   spell effects, stamina, and death consequences are applied in order.
8. **Settle the round entry.** `Battle` records any remaining attacks, next phase,
   or completion. A later attack may have different permitted target-locking
   rules; it is not an arbitrary reroll of the entire turn.

The physical attack, item-use, and spell-delivery walkthroughs define their
particular release and consumption boundaries. In particular, the player area
consumable path creates its missile during casting; this document's mention of
monster animation milestones must not invent a player hand-release callback.

### Route data and dynamic obstacles

The current pathing code exposes concrete needs beyond a flat shortest path:
conditional links controlled by props, path length rather than straight-line
distance, body radius/clearance, target separation, height constraints, linked
group motion, and route steering. Its hearing path also counts closed-door hops.
The current C++ improvements include owned search scratch and typed path results;
those are useful contracts even though Rizzardry need not keep its exact graph or
search algorithm. [D-navigation] [W-navigation] [W-nav-search] [W-nav-noise]

Keep an immutable level navigation asset plus a small dynamic link state. A door
or bridge operation changes its collision/link state together, then invalidates
affected route results. Each moving actor owns its path and goal. Search scratch
belongs to that search or a reusable scratch allocation, not to shared mutable
monster state.

Movement profiles must distinguish ground, flying, and swimming requirements.
Do not assume a ground navmesh alone implements all authored mover modes.
Initially, keep route planning synchronous and bounded. If profiling later
requires background searches, stamp the request with actor, goal, level, and
navigation revision; reject obsolete results before they replace a current path.
No asynchronous job layer is needed to begin the rewrite.

Group followers may share a route corridor or a leader goal, but each keeps its
own collision-aware motion. Removing a leader changes the coordination role;
it must not leave followers dereferencing the leader's route allocation. A
blocked follower does not move through another monster merely because the
leader's route succeeded.

## Walkthrough 4: Hypnotic Lure redirects a patrol, then is replaced

This is a useful boundary case because it joins magic, resistance, group rules,
navigation, clocks, and saving without requiring the party to be in combat.

The recovered effect searches active nearby hostile monsters around the lure,
checks sight to the lure and eligibility, and makes a resistance attempt. Outside
party combat, the leader/group response can control the group; during combat,
responses are individual. The current code retains that distinction. A resisted
response prevents treating each later scan as a fresh unconditional opportunity
to reroll the same effect. A currently lured actor remains eligible for the
source's later scan; successful attraction is not a permanent immunity to a
subsequent resistance check. [D-lure] [W-lure-apply]

| Step | Ordered operation | Ownership consequence |
| --- | --- | --- |
| Cast succeeds. | `game::magic` creates the world lure effect with source, position, power, and duration. | It is level-lived, not a `Battle` field. |
| A previous lure exists. | Expire the old active lure and release its control before installing the replacement as active. | Old fading visuals may remain presentation-only. The current effect identity is unambiguous. |
| Apply or rescan control. | Collect eligible active actors, check line of sight, and resolve resistance in stable order. Outside combat apply the appropriate group response; inside combat apply the individual response. | Store the response with the effect identity in AI/control data; do not overwrite base faction or authored patrol orders. |
| Follow the lure. | AI's effective movement goal becomes the lure position with stopping separation. If pathing fails, the source can face toward the point instead. | The effect's point is a navigation input, not a second monster pose. |
| Time advances. | `game::magic` ages the duration and applies the required nearby-control scan at the world-time cadence. | This continues without the party's combat scheduler. |
| The effect expires or is replaced. | Release only responses belonging to that effect, restore effective ordinary behavior, and invalidate its movement goals. | Ending an old visual cannot clear the new lure's control. |
| Save and restore an allowed exploration state. | Save effect identity/source/point/power/duration and relevant actor responses; rebuild the visual and route on load. | Do not recast, replay the cost, or draw resistance again merely to reconstruct presentation. |

The world-time and following-goal paths are separate from the initial cast:
`GameplayTime.cpp` ages/rescans the effect, while roaming AI requests movement
toward its point. [W-lure-clock] [W-lure-goal]

This is a place where current Whizzardry is clearer than the recovered source:
`FindMonsterControlSpellEffect` searches newest-first for an unexpired lure;
replacement expires previous live lures before releasing control. An existing
native test specifically checks newest-unexpired selection. Keep this lifetime
rule rather than copying the older first-match scan. [W-lure-lifetime]
[W-lure-replace] [W-lure-test]

The source has a dedicated `HYPN` save record for this effect, and world entry
distinguishes ordinary travel from save restoration: ordinary entry clears
world-local spell effects, while restoration rebuilds lure visuals. This proves
that “leave combat,” “leave level,” and “load save” cannot share one blanket
cleanup function. [W-lure-save] [W-level-entry]

The original scan size is tied to world far clip. Rizzardry should give this a
gameplay range independent of graphics settings. The exact range to use is an
explicit compatibility decision, listed below; it is not a reason to let render
culling own magic.

## A summon is another actor, with an explicit dependency

Summon Elemental illustrates why an ally is not automatically another party
character. The spell checks an existing caster dependency, chooses an elemental
kind using the relevant weights, creates a group/body at the target, determines
disposition from caster state and backfire, and records the caster-to-summon
dependency. Summoned origin remains distinguishable from current disposition.
Both source implementations express these steps. [D-summon] [W-summon]

In Rust, `spawn_summon` performs placement validation, creates the spatial actor
and group if required, sets summon provenance, installs the dependency, and then
admits it through ordinary engagement rules. One readable function performs this
sequence. Failure to place must not leave the caster permanently marked as owning
a nonexistent summon.

When a summon dies or normal combat-end cleanup removes it, clear the dependency
before another summon can be chosen. Do not award normal monster-death benefits
merely because a friendly summon disappears. The source has distinct normal and
forced cleanup branches; forced departure must follow the documented world
transition rather than relying on a fading death animation. [W-summon-cleanup]
[W-dependencies]

Do not infer a universal rule that every effect or summon dies with its caster.
Dependencies are effect-specific. The directly verified rule here is clearing
the caster's summon reference when the summon is removed, together with the
normal combat-end summon cleanup.

## Persistence and unload

Active ECS state is authoritative. A saved record is a capture or an unloaded
level's state, never a second mutable copy of a living actor.

The initial supported save boundaries are defined in
[persistence and travel](persistence-and-travel.md#when-saving-is-allowed): settled
out-of-combat exploration and specified paused inventory/options states. An
already-established lure can be a supported continuing effect; an unresolved cast
or impact is not made saveable by that example. The clock and catch-up owner is
defined in [time, rest, and conditions](time-rest-and-conditions.md).

| State | Save/unload treatment |
| --- | --- |
| Party pose, roster, live formation, character state | Persist through ordinary travel. Clear transient UI drafts; settle or refuse an incompatible active action according to the transition policy. |
| Named NPC identity and its retained profiles/progression | Keep the campaign actor identity. Remove active world presence and its AI/physics/render participation when absent; serialize persistent role state and optional world placement, which is absent while recruited. |
| Ordinary level monsters and groups | Capture durable pose, vitals, conditions, authored orders, local provocation, membership, and necessary observation memory into that level's dormant records; despawn their active representation on unload. |
| Current line-of-sight and proximity query results | Discard and rebuild. They are not memories and are not independent authorities. |
| Observation history, cooldowns, and patrol progress | Save the facts whose loss changes the next decision; use named clock domains. Re-entry aging follows an explicit policy rather than running hidden movement off-map. |
| Waypoint path, search scratch, steering candidates | Discard and replan from saved pose/goal. Saving never mutates patrol visits or draws gameplay RNG. |
| `Battle` and an in-flight action | Follow the project's save availability policy. Existing Whizzardry does not serialize the party fight. This design does not silently promise arbitrary mid-action saving. |
| Hypnotic Lure in an allowed save | Save the continuing effect and control response; reconstruct delivery/presentation without a second cast or resistance roll. Ordinary level departure ends the level-local effect. |
| Summons and caster dependencies | Settle the relevant combat/departure cleanup first. Persist only states allowed by the chosen save policy, with both ends of each surviving dependency resolvable. |
| Temporary animation/sound waits | Presentation handles are never serialized as gameplay identity. A restored semantic phase needs a defined resume/finish rule if that phase is ever allowed in saves. |

Translate runtime entity references to persistent IDs at the save boundary. On
restore, instantiate actors/groups/effects before resolving cross-references.
Missing optional targets cause a defined fallback; a missing required owned
dependency is a load error or a documented repair, not an accidental reference to
the next entity occupying that index.

Level departure is ordered: prevent new decisions, settle/cancel the applicable
current work, release departing control and summon dependencies, capture the
remaining authoritative state, and remove level-local entities/resources.
Despawn hooks release engine resources; they do not invent reputation changes,
combat rewards, or story consequences after the world has already gone away.

## Concrete decisions still requiring evidence or product choice

1. **Activation and range versus graphics settings.** Both source trees use far
   clip/renderability in some group and lure decisions. Choose stable gameplay
   distances and verify representative encounters before removing this coupling.
2. **Decision cadence.** Replace frame-modulo maintenance with simulation time,
   but first characterize patrol response, hearing cooldowns, and reinforcement
   timing. A faster new loop must not provide extra random attempts.
3. **Party movement formula and stationary continuous progress.** The source is
   precise enough to implement, including its minimum-initiative calculation and
   probabilistic remaining turns. If the rewrite changes that policy, document
   it as a gameplay change rather than an ECS simplification.
4. **Group leadership loss.** Pin the authored leader/replacement rule for mixed
   and allied groups. Membership ownership is decided; successor selection is
   still a gameplay rule, not “first Bevy query result.”
5. **Off-party combat.** The evidence proves monster engagement can exist without
   the party's `Battle`. It does not by itself specify a fully simulated,
   independently scheduled monster-versus-monster fight in unloaded or distant
   zones. Do not build that larger feature by implication.
6. **Navigation assets and mover modes.** Confirm which imported waypoint,
   conditional-link, flying, swimming, and scripted paths are needed before
   selecting a single replacement path representation. A library's ground-route
   demo is insufficient evidence for all these cases.
7. **Possible save extensions.** The initial exploration policy is decided in the
   persistence document. Round-boundary or arbitrary mid-action saves would be
   additional features requiring their own resumed-phase rules; they are not an
   implementation obligation of this combat design.

## Acceptance scenarios for the implementation

These are specifications to exercise when code exists, not a request to create
tests for this documentation change.

| Scenario | Observable result that matters |
| --- | --- |
| A patrol hears the party through a closed door. | Hearing/path attenuation can differ from direct sight; the patrol investigates or faces the noise and retains its authored patrol destination. |
| Party and monster have asymmetric perception. | Showing the monster to the player does not automatically grant it knowledge of the party; recent memory can outlast direct sight. |
| A neutral faction group is attacked near two allies. | Visible qualifying allies react; a blocked/distant same-faction group is not unconditionally alerted. Campaign reputation changes only through its own rule. |
| A group becomes engaged before party combat begins. | Its actor state is valid without a `Battle`; an NPC entry script can defer the party transition without duplicate combat initialization. |
| The same fight runs in phased and continuous modes. | Action order and effects follow the same rules for the same accepted choices and RNG inputs; automatic next-round pacing is the intended difference. |
| Walk, run, and stop at different gauge fractions. | One party body moves; member fatigue and remaining action chances follow the explicit movement policy; stopping does not restore spent movement. |
| Continuous movement is selected but no motion occurs. | The round follows the chosen stationary-progress rule and cannot remain stuck accidentally. |
| A back-row melee actor and a ranged actor target the same nearby monster. | Formation modifies close reach appropriately without separate character movement bodies. |
| A target moves behind a closing door after action choice. | Execution revalidates sight/range and navigation; an old route or accepted UI choice cannot hit through the changed obstruction automatically. |
| A leader dies while followers are routing. | The group resolves its next coordination role without dangling route/body references; followers still obey individual collision. |
| An absent named NPC retains its campaign entity. | It receives no world AI tick, sight scan, path request, or collision while absent or recruited into the nonspatial party role. |
| Lure A is replaced by lure B while A's visual is fading. | B is the active control source. A's retirement cannot clear B's responses; normal orders remain available after B expires. |
| An allowed save contains a lure and a resisted monster. | Load restores duration/control identity without spending spell resources again or rerolling solely for visual reconstruction. |
| A summon dies, changes allegiance, or is removed at combat end. | Dependency cleanup and rewards follow the specific rule; allegiance changes do not erase summoned origin or add it to the party roster. |
| Debug overlays and extra cameras are toggled during a seeded encounter. | They do not alter gameplay RNG, decision count, route authority, or turn order. |

## Pinned source references

D-prefixed links refer to recovered source; W-prefixed links refer to the current
C++ implementation. Every link names the complete immutable commit and the
relevant file/line range.

[D-sight]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Sight.cpp#L366-L790
[W-sight]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/Sight.cpp#L362-L670
[W-step]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/game_advance.cpp#L33-L111
[D-disposition]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/MonsterGroup.cpp#L198-L319
[W-disposition]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/MonsterGroup.cpp#L182-L291
[D-hostility]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Combat%20Hostility.cpp#L68-L137
[W-hostility]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/combat/hostility.cpp#L69-L235
[D-alert]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Combat%20Hostility.cpp#L912-L927
[W-alert]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/combat/hostility.cpp#L867-L883
[W-provoke]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/combat/hostility.cpp#L337-L411
[D-group-entry]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/MonsterGroup.cpp#L1373-L1491
[W-group-entry]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/MonsterGroup.cpp#L917-L1026
[W-monster-entry]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/MonsterManager.cpp#L909-L955
[D-turn]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Combat.cpp#L3113-L3315
[W-turn-types]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/include/wiz8/combat/combat.h#L31-L78
[W-turn]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/combat/turn.cpp#L38-L310
[D-initiative]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Combat.cpp#L1729-L1785
[D-action]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Combat.cpp#L2125-L2343
[D-party-move]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Combat%20Party%20Movement.cpp#L238-L330
[W-party-move]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/combat/party_movement.cpp#L241-L390
[W-move-progress]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/combat/party_movement.cpp#L95-L175
[W-move-cancel]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/combat/party_movement.cpp#L393-L519
[W-formation]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/include/wiz8/rules/party_formation.h#L12-L63
[D-range]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Combat%20Range.cpp#L856-L950
[W-range]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/combat/range.cpp#L198-L249
[D-ai]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/MonsterAI.cpp#L315-L648
[D-ai-round]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/MonsterAI.cpp#L817-L1307
[W-roam]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/MonsterAI.cpp#L399-L578
[W-ai]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/MonsterAI.cpp#L738-L866
[W-ai-options]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/MonsterAI.cpp#L987-L1240
[D-engage]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/MonsterAI.cpp#L948-L1037
[W-engage]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/MonsterAI.cpp#L2472-L2561
[W-begin]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/combat/fight.cpp#L108-L222
[D-navigation]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/engine_code/Navigator.cpp#L1546-L1652
[W-navigation]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/spatial/navigator.cpp#L1191-L1295
[W-nav-search]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/include/wiz8/spatial/navigation.h#L184-L344
[W-nav-noise]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/spatial/navigation.cpp#L692-L791
[D-lure]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Magic%20Effects.cpp#L2477-L2549
[W-lure-apply]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/Magic%20Effects.cpp#L2173-L2244
[W-lure-lifetime]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/Magic.cpp#L373-L408
[W-lure-replace]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/Magic.cpp#L2911-L2937
[W-lure-clock]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/GameplayTime.cpp#L589-L607
[W-lure-goal]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/MonsterAI.cpp#L691-L710
[W-lure-test]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/tests/native/save_records_test.cpp#L1291-L1325
[W-lure-save]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/save_codec.cpp#L1221-L1260
[W-level-entry]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/world/level_session.cpp#L425-L451
[D-summon]: https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/Magic%20Effects.cpp#L702-L815
[W-summon]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/Magic%20Effects.cpp#L695-L794
[W-summon-cleanup]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/MonsterManager.cpp#L497-L521
[W-dependencies]: https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/local_code/ConditionsAndEnchantments.cpp#L68-L109
