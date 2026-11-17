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

Create the group entity when the level requests the arrival. Attach its origin
level and pending placement immediately. When assets are ready, populate that
same group. For authored births, let the group own its rolled placements until
birth consumes them. Resolve script group numbers within their level. Audit
readers of authored group numbers and monster locations. Where generated numbers
are required, reserve the whole range on the level when requesting the group,
across all creation paths. Preserve placement order and already rolled values.

Check two arrivals becoming ready together, travel while an asset is loading,
and repeated birth requests for the same authored group.

### Separate script addresses from selection order

Three current consumers explain why replacing every number with an entity
would change behavior. Birth resolves a level-authored group number. Magic
orders eligible monsters by placement location, and combat engagement also
uses location when building its list. Group recognition sorts by authored
group number, explicitly to order random draws. Preserve script addressing
and player-visible target or combat ordering. Reproducing the original random
draw order alone does not justify allocating synthetic script addresses.

Give only script-addressable groups an authored address. Preserve a level-local
placement order for bodies wherever it decides which target is selected. Assign
generated members their order when the group is requested so asset completion
cannot reorder them. Route story arrivals, encounters, combat summons and
point casts through the same level owner. Before deleting either numeric field,
trace selection through to the visible target, especially when more than one
eligible body is present. [Magic aim](../../src/game/magic/aim.rs) ·
[Engagement](../../src/game/combat/engage.rs) · [Recognition](../../src/game/world/identify.rs)

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

In the reviewed implementation, eight target-kind components installed per-cast observers for initial eligibility,
spread, straying, backfire and release. Several initial observers just choose a
`Takes` value. The caller creates an aim entity to request and receive a selection.
The working tree already has a targeting rewrite in progress. Check its coverage against this design. [Current aiming code](../../src/game/magic/aim.rs)

Represent target mode as a value on the spell. Query
caster, chosen entity or point, affiliations, positions and obstacles, and return
the bodies or point selected. Keep separate functions for initial eligibility,
spread once power is known, redirection and release geometry. Preserve a cast
or projectile entity where later impact or interruption gives it a lifetime.
Cover scatter and backfire in the same rewrite.

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

Find a shipped sequence with a dependent write and read and establish when
retail evaluates the condition. Encode that timing in one runner over the
authored sequence. If later reads see earlier writes, apply story-local writes
as the runner advances. If a trigger captures conditions at entry, preserve
that evaluation point explicitly. Carry the optional subject body in the
execution context. Issue domain operations such as device bids directly, keep
delayed consequences scheduled for their due time, and remove the generic
two-stage relay. The runner owns sequencing; recipient domains own their effects.

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
training available to spell eligibility: the rule distinguishes training before
this level's freely allocated points from the effective book skill and the new
caster levels. Computing every input from the final preview would erase that
distinction. Changing profession discards the old tentative allocations and
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
The group owns grudge expiry, including the leader and combat exceptions.

Replace propagation writers and their combat, targeting and dialogue readers
together. Provoking one group changes one fact, and a newcomer immediately
inherits the current stance. Verify how a turned monster in a provoked group is
treated when it dies and when it is raised, including which side counts for
faction attribution.

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
