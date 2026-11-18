# Gameplay redesigns

## Monster population

A story action creates a `Coming` request without its destination level. While
waiting for the body asset, the request depends on whichever level is in play.
If the party travels, the named spot could resolve on the wrong level or the
request could be discarded. Within one invocation of `come`, two ready requests
scan the same live monsters and groups for the largest numeric identifiers.
Deferred spawns from the first request are invisible to the second. Encounter,
summon and point-cast code also allocate numbers this way.
[Population](../../src/game/world/population.rs) · [Story requests](../../src/game/npcs/story.rs)

Create the group entity on its origin level when requested. Allocate a body
entity before birth wherever another actor or script can already refer to it.
In the Sea Caves, a script starts with a named NPC, inspects its body's birth
cycle, follows its group and wakes that group. A vector of anonymous placements
cannot answer that query. Keep dormant bodies addressable through their
relationships while excluding them from combat, perception and normal picking.
Birth activates the existing group and members once; an asset becoming ready
satisfies a prerequisite on those same entities. Travel cannot redirect the
request. A species-wide birth command visits every matching group on its level.

Use Bevy's default query filters for bodies whose whole gameplay presence is
dormant. Register the birth marker as a disabling component at plugin setup.
Combat, picking and perception then omit dormant bodies by default. The named
NPC lookup opts in with Allow for that marker, and birth selects With that
marker. Entity identity and group relationships survive. Removing the marker
from the existing body makes normal queries include it once the structural
change is applied. Disabling a group alone leaves its bodies individually
queryable. Apply the marker to each body that must be excluded. An actor
who merely sleeps or is unconscious must still be targetable under the
applicable rules and belongs in the conditions model.
[Bevy entity disabling](https://docs.rs/bevy_ecs/0.20.1/src/bevy_ecs/entity_disabling.rs.html)

### Identity and ordering

Keep stable entity relationships for script bindings, membership and leadership.
Preserve authored addresses at the script boundary. A body's placement number
also contributes to its leadership rank: higher location breaks a tie at equal
species level. Preserve the value needed by that rule, including for generated
bodies. Reserve any needed range once on the origin level, across creation paths.

Give the origin level one **placement number allocator** for generated bodies
and groups. Bevy's Commands::spawn_empty allocates an Entity immediately and
queues its insertion. That lets the caller refer to the same entity before any
query can see it. Follow that separation for the retail numbers that gameplay
actually uses. Reserve the group's number and the range of body locations in
one mutable borrow on the level, after accepting the spawn count. Two arrivals
in one system invocation see successive reservations even with every spawn
still queued. Populate authored numbers first. On restore, recover the next
reservation from saved allocation state if previous reservations must survive
despawn; establish retail number reuse before choosing that persistence rule.
Use Entity for runtime links. This replaces max scans in story arrival, summon,
encounter and point casting and the encounter's private advancing pair.
[Bevy command implementation](https://docs.rs/bevy_ecs/0.20.1/src/bevy_ecs/system/commands/mod.rs.html) · [Population allocation](../../src/game/world/population.rs)

Each targeting rule owns its ordering. Normal cursor picking takes the nearest
selectable live model, retaining the first encountered at equal distance.
Automatic party target selection compares candidate state, then reach, then
distance. Its comparator leaves exact ties equal. Neither establishes a
universal placement-number sort for spells. Verify each spell selector before
changing its observable ordering. Check repeated birth, an NPC referenced before
birth, and equal-level leader succession with distinct location numbers.
[Decomp paths and boundary cases](retail-evidence.md#monster-groups-and-targeting)

## Melee swings

An attack spawns a swing, queues aiming, fate, guarding, landing, hitting and
settlement, despawns the swing, then queues the next swing. A damage deed links
back through `BlowOf`; its observer writes `Blew` onto the swing. Razor Cloak
uses a second return path through `AnswerTo` and `Answered`.
[Swing resolution](../../src/game/combat/swings/mod.rs)

Keep turn scheduling, target selection and the next attack moment. When an
attack comes due, resolve its swings in sequence. For each, read the live actor
and target, calculate aim, select the actual defender after interception, apply
the blow, resolve retaliation, and accumulate what happened. Give the shared
hit rule a synchronous entry point returning the effects actually dealt. A
redirected target and returned damage belong in local variables. Emit outcomes
for reporting and other domains after the corresponding changes.

Rewrite the whole path from `swing_again` through settlement. Recheck the
attacker, target and ammunition between swings. Preserve a guarded hit, the
separately reported retaliation, and termination when the target dies or the
last shot is spent.

## Spell targeting

Commit `f092ad9` moved initial eligibility, spread and backfire into the release of a deed spell. A local target selection now carries those decisions through release. Preserve the boundary at delayed impact. The remaining work includes casting eligibility, power calculation and immediate utility spells. [Magic design](magic.md) · [Deed casting](../../src/game/magic/casting/deed.rs)

## Actor fitness and derived figures

`Settle` runs in both Update and FixedUpdate. Its chain includes collapse,
inventory weight, hostility, character creation, repertoire, derived figures
and party tending. Reactivating a companion marks every mutable component
changed to replay derivation. Within one inner tuple, agility reads attributes
without an explicit dependency on the system that writes them. This creates a
possible one-pass lag. [Fitness schedule](../../src/game/actors/fitness.rs) ·
[Derived figures](../../src/game/actors/figures.rs)

Eliminate leaf values such as agility that readers can compute directly from
attributes. Give draft picks and refunds to character creation, equipment and
weight dependencies to their owners, and death, exhaustion and recovery to
actor physiology. Specify when consequential transitions become visible to the
next action. For retained caches, cover every real input and initialize explicitly
when an actor becomes active. Retire blanket change replay once those paths
cover retained companions. Preserve combat timing when moving burden updates.

## Story execution

`follow` unfolds nested conditionals before queuing selected rows. A later
condition therefore sees the state preceding an earlier selected write.
`Befalls` then dispatches a cloned row as `Befell<T>`. Some recipients immediately
translate it again, for example into a device `Bid`. Body-local actions take
a separate path. [Story interpreter](../../src/game/npcs/story.rs)

Evaluate each condition when execution reaches it. An unsuppressed write
stores its value, records a journal change if appropriate, then runs its
consequences before the enclosing sequence resumes. A same-value write can
still run consequences. A suppressed write only stores the value. In the
Astral Dominae acquisition path, the possession write clears the NPC's
scheduled item before the acquisition rule asks whether the retrieval quest
is still active. A delayed event explicitly begins a new sequence when due.

Implement the sequence as a narrow story interpreter. Execute immediate fact
writes and reads with fresh access to the owning state. Resolve each domain
operation whose effects a subsequent condition can see before advancing.
Keep the subject body with the sequence and keep genuinely delayed work on
its own schedule. Remove eager conditional expansion and the generic event
relay. Queue presentation after deciding what happened. [Retail evidence and
worked scenarios](retail-evidence.md)

The missing primitive here is a **story continuation**. The current unfold
already has a stack, but empties it into a list of selected rows before any
of them executes. Keep the stack local to the execution. Each frame identifies
the authored sequence, its next row, and the subject and triggering fact needed
by its conditions. Advance one row, perform its domain operation, then evaluate
the next row against the resulting world. A nested fact write pushes its
consequences above the caller's remaining rows. When a delay genuinely outlives
the call, retain the authored work and binding required at resumption. Ordinary
nested execution needs neither an entity per row nor a global interpreter queue.

Bevy's scheduler can insert ApplyDeferred between ordered systems. It cannot
interpose between two loop iterations inside unfold. Adding more ordering edges
between fact observers therefore cannot fix the time at which a branch was
already chosen. The completion boundary belongs inside the interpreter.
[Bevy schedule build pass](https://docs.rs/bevy_ecs/0.20.1/src/bevy_ecs/schedule/auto_insert_apply_deferred.rs.html)

## Dialogue endpoints

`NpcActors` bundles name lookup, time, level, party membership, monster
conditions, position and device speakers. Its availability check can succeed
through a member, body or device but returns only a boolean. Subsequent code
has to discover the relevant endpoint again. [NPC access](../../src/game/npcs/access.rs) ·
[Conversation actions](../../src/game/npcs/conversation/acts.rs)

At admission, resolve both the persistent person and the endpoint through which
the party reached them. Apply admission rules for that member, body or device
and retain the endpoint on the conversation. Disposition, script identity and
trade history belong to the person. Turning toward the party concerns a body.
A word reply needs the conversation and its script. Use the existing person
relationships and give endpoint disappearance an explicit dialogue outcome,
verified against the interruption rules. Rewrite admission and the ongoing dialogue operations together, removing
the superseded accessor.

## Character advancement

Opening a level-up draft clones eight components and copies learned spells onto
a second entity. Changing picks rebuilds draft spell entities. Acceptance copies
progression values back and reparents the selected spells. A separate system
compares change ticks to discard stale drafts. A choice about one level has
acquired the maintenance cost of a second actor. [Draft implementation](../../src/game/characters/draft.rs)

Keep the draft entity for the lifetime of the player's choice. Link it to the
member and store the chosen profession, allocations and spell picks. Use one
progression rule to evaluate those choices against the member's current state.
The UI previews its result. Acceptance validates the choices again, applies the
earned changes once, teaches the spells and announces the advancement so that
equipment such as the omnigun can respond. If the underlying member changed,
acceptance must preserve that newer state. Cancellation simply ends the draft.

Rewrite profession switching, allocation, spell selection, preview and acceptance
together. Remove the cloned spellbook, reparenting and tick-based reconciliation.
New-character creation owns a character with no prior member. Share progression
formulas where they agree. Check a profession switch after points are spent,
cancellation after picking a spell, and acceptance of the paired portal spells.

### What preview and acceptance must agree on

Evaluate the profession choice first. Continuing a profession pays down
attribute debt; switching applies the new minimums and seeds profession skills.
Crossing the first caster level can grant book training. Keep this intermediate
training available to spell eligibility: the rule distinguishes book and realm skills before this level's freely allocated points, including
their effective levels, from the profession and caster levels being previewed.
Evaluate learning on that basis and refund tentative picks that lose eligibility.
Cap available picks at the number of eligible spells. Changing profession discards the old tentative allocations and
picks and evaluates the new choice from the member again.

Acceptance checks whether the member can still advance, the profession is
available, the allocations are legal and every chosen spell is still eligible
and payable. Remaining points block acceptance only while a legal increase is
available. If a choice has become invalid, keep the member untouched and return
the reason to the draft. On success, teach each earned spell once, preserve
current health and stamina as maxima change, and announce advancement after
writing the result. A preview needs values to render; it never needs to spawn
a learned-spell entity. [Profession and acceptance rules](../../src/game/characters/draft.rs)

## Hostility

A group's temporary stance is written to its members as Side, recomputed on
membership or faction changes, and combined with Loyalty and Conditions into
Standing. Provocation uses underlying allegiance while targeting needs effective
standing. Death preserves the side an actor was on when it died. These are
three different questions. [Hostility rules and propagation](../../src/game/world/hostility.rs)

Keep allegiance on the actor, reputation with the faction and the timed grudge
on the group. A cohesive read-only query follows membership and applies the
group override or faction default to answer which side an actor belongs to.
For the disposition between two actors, apply their personal conditions and
loyalties. Record the historical side at death where subsequent rules need it.
The group owns its stance timestamp and uses its species Intelligence for
expiry. Preserve an individual disposition where the body resists group changes.
A leader that preserves hostility can prevent the group change itself.
A successful group change can reach its encounter leader and allied groups.
Represent these links in the group model and retain the scope of the retail
operation. Apply sight and target consequences to groups actually affected.

Replace propagation writers and their combat, targeting and dialogue readers
together. A newcomer can read its group stance directly. Distinguish the group change
from each affected body’s combat response. Verify how a turned monster in a provoked group is
treated when it dies and when it is raised, including which side counts for
faction attribution.

Bevy visibility separates the authored visibility choice, the result of
hierarchy propagation and the outcome of checks by views. In particular,
visibility to *some* view cannot answer whether one specific camera sees an
object. Apply that distinction to hostility. A group's stance and a member's
individual override can produce one effective allegiance per member. Whether
actor A may attack B still requires both subjects. A combatant changing sides
can change who may attack it even if the would-be attackers have changed no
components. Keep pairwise decisions at target selection. If broad AI searches
eventually need indexed sides, publish effective allegiance with a defined
refresh point. The death rule reads the current side at death and records it
once. Subsequent group changes must leave that attribution intact.
[Bevy visibility implementation](https://docs.rs/bevy_camera/0.20.1/src/bevy_camera/visibility/mod.rs.html)

## Turn interruption

At the start of a combat moment, observers pass an action through nausea, web,
blindness, insanity, fear, panic, lure and Turncoat checks. Each recovers the
actor through ActionOf. Breaking a web queues the report, condition removal and
loss of the action separately. A berserk monster can strike even when it loses
its intended action. [Action execution](../../src/game/combat/action.rs)

Put precedence in one ordered resolver for the actor's due moment. Read the
intended action and live conditions, roll the relevant checks, apply immediate
changes and return what the actor actually does or that the moment was spent.
Keep a dedicated rule for decisions such as selecting a friendly target under
insanity. Berserk state lasts through the round and belongs on the actor.
Remove the Weighs events and temporary action entities used only to advance
through the checks. Preserve announcements and the timing of the next moment.

Combine this with the melee rewrite. Turn resolution chooses the attempted
action and intended target. Attack resolution handles interception and the blow.
A webbed actor who breaks free must lose the web, spend the moment and leave
the original order unexecuted. [Combat turn state](../../src/game/combat/turn.rs)

## Party activity

Freedom inspects camp, conversation, travel, device interaction, path flight
and other ongoing activities. Advancement and travel each check their own
subsets. The subsets encode genuine exceptions, including utility magic during
camp and the world continuing while the party is occupied. [Freedom](../../src/game/freedom.rs) ·
[Travel admission](../../src/game/world/level.rs) · [Advancement admission](../../src/game/characters/draft.rs)

Before implementation, record admission by player action and activity, including
the point when a closing conversation releases the party. Resolve uncertain
differences against the game rules. Then make each activity entity carry the
restrictions that share its lifetime. One gameplay query answers whether the
requested action is available across the current activities. Travel retains
destination validation and advancement retains member eligibility. Replace the
existing scattered occupancy checks in the same rewrite.

Check that closing dialogue releases input at the intended moment, device work
blocks the relevant party actions while world time continues, and camping still
permits the applicable utility magic. These outcomes require separate action
permissions even when the same activity owns them.

### Admission cases the rewrite must distinguish

The following are observations of the current entry points. Where two paths
disagree, establish the intended behavior before merging their checks.

| Request | Current decision and consequence for the rewrite |
| --- | --- |
| Ordinary world action through Freedom | Requires a played level and excludes entry, active dialogue, travel questions, path flight, device work, member selection, game ending and camp. Closing dialogue already releases this particular check. |
| Utility magic | Its occupancy check permits camp. Spell eligibility also distinguishes combat, device, dialogue, camp and exploration in that priority order. Preserve both the ability to attempt an action and the spell-specific decision. |
| Advance a member | Independently checks battle, entry, travel question and any conversation. In particular it sees closing dialogue differently from Freedom. Verify that timing before assigning a shared restriction. |
| Accept travel | Checks battle, camp and pending entry. On success it closes travel questions and clears movement. The question that initiated travel must be able to authorize its own transition. |

Casting currently derives context and party markers in the actor settlement
schedule. Have the spell view and cast admission use the same live context
decision when this schedule is removed. Keep footing as a physical input and
whether dialogue is awaiting a reply as a dialogue input. Neither is a general
party restriction. Looting a previously locked device currently counts as lock
work for casting; the source explicitly records the retail behavior as unknown.
[Spell context](../../src/game/magic/occasion.rs) · [Party freedom](../../src/game/freedom.rs)
