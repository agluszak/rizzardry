# ECS architecture

## Choose component boundaries by how the game uses the data

Start with an actual reader and writer. For each proposed component ask who
queries its presence, who changes it independently, and what else must change
with it. An item can both cast and protect. Those capabilities compose because
different systems select them independently. A stack's count and limit belong
together: a merge needs both to determine and apply the transfer. Splitting a
number into its own component earns something when its presence selects a
meaningful population or it has a distinct writer or lifetime. Several fields
participating in one rule can simply be fields.

Use an enum when exactly one alternative applies and one decision interprets
it. A spell aimed at a cone or at one body has a targeting choice; adding a
shape should lead to the geometry and targeting code. It should not require
registering a fresh family of per-cast observers. Conversely, encoding every
combination of wearable, charged and spellcasting item in an enum would force
unrelated rules to enumerate combinations. Look at the *next plausible change*
before choosing the representation. [Target shapes](../../src/game/magic/aim.rs) ·
[Item capabilities](inventory.md)

## Separate the inputs to a rule from its derived components

Use Bevy's local and global transforms as the model. Gameplay writes a local
pose and a parent relationship. Propagation owns the world-space pose.
A system reading the global pose depends on when propagation last ran.
Apply the same distinction to equipment. Equipping a shield writes its
slot relationship. The item's protection and the actor's stance each have
their own writer. Armour calculation may publish a separate component on
the bearer giving protection by hit location. Consumers can borrow that
component without knowing how to walk the equipment.

Choose the scope of a derived component from its readers. Protection by
location can serve many blows against the same actor. A weighted average for
the sheet can be a method on that result taking the body's hit distribution.
A modifier specific to a particular attacker belongs in the strike. Only
materialize figures together when their inputs and required freshness agree.

Give a stored result one producer and specify when it is current. Moving a
shield between bearers affects both, removing it affects its former bearer,
and changing a worn item's protection affects its current bearer even if the
slot stays put. Rebuild from the sources on spawn and restore. If a strike can
follow a swap in the same action, the action must see the updated figure.
Place propagation between the operations, or evaluate the same armour rule
directly there if the operations must interleave. A frame-end refresh alone
cannot promise that ordering. Adding a source of protection should change
the armour rule and its inputs in one place; attacks and the sheet should
keep their existing reads.

### How Bevy actually synchronizes transforms

In the pinned Bevy 0.20.1, inserting a Transform requires GlobalTransform and
TransformTreeChanged. Required components establish presence. The transform
plugin establishes freshness. It runs an ordered propagation set in PostStartup
and PostUpdate. With static optimizations enabled, changes to local transforms
or parent links, newly added globals and removed parent links mark affected
trees. Marking runs toward ancestors so traversal can skip clean branches.
Composition runs from parent to child, so a moving parent updates a child whose
own local pose stayed unchanged. A separate pass handles standalone entities,
including newly detached leaves. Static optimizations can be disabled without
changing what the resulting world pose means. [Plugin](https://docs.rs/bevy_transform/0.20.1/src/bevy_transform/plugins.rs.html) · [Systems](https://docs.rs/bevy_transform/0.20.1/src/bevy_transform/systems.rs.html)

There are two legitimate reads. A consumer ordered after propagation reads a
shared component cheaply. A consumer that needs a pose after a local edit and
before propagation can call TransformHelper::compute_global_transform for that
entity. It walks the current ancestors and returns a value without publishing a
new global or updating siblings. Both reads require structural changes they
depend on to have been applied to the world. This is a useful contract even for
a domain small enough to rebuild all its outputs in one pass. Dirty tracking is
an implementation choice made after identifying the dependency graph.
[TransformHelper](https://docs.rs/bevy_transform/0.20.1/src/bevy_transform/helper.rs.html)

### Apply the contract to each domain

| Domain | Authoritative inputs | Derived result and freshness boundary |
| --- | --- | --- |
| Inventory burden | Stack weights and counts, locations, party roster and capacities | Pool distribution is a party-wide calculation. Rebuild the affected party together at the retail redistribution point. A stack change on one bearer can alter another member's share. Publish the resulting loads for movement and actor figures. |
| Effective attributes | Actor's underlying attributes and the active modifiers that affect them | Publish effective attributes on the affected actor for the many skill and combat readers. Applying, expiring or removing an effect must reach its subject. If a follow-up hit needs the new value within the action, resolve it from current inputs or publish before that hit. |
| Attack selection | Active gear, ammunition, training and natural attacks | A cached reach for broad AI selection can have a declared update point. A swing reads its actual weapon and available ammunition when it executes. A weapon accepted earlier in the round stays recorded on the order even after a gear change. |
| Magic | Learned spell, caster skills, realm pool, current activity and intended target | A book can build many previews in one read. A cast evaluates the chosen source when it executes. A projectile's released strength records an earlier decision and survives subsequent caster changes. |
| Actor fitness | Vitals, conditions and abilities | A published eligibility marker can select participants at round setup. During retaliation a hit may already have disabled one participant. Evaluate eligibility on the affected actor before another blow. Emit death and its cause when death occurs. |

The direction of a dependency differs by domain. Transforms fan out from a
parent, equipment aggregates from items to a bearer, pool distribution couples
the whole party, and casting combines a particular source with a particular
target. Let the domain that knows those edges own its calculation. A system
that orders load distribution before movement expresses a real dependency.
Making every immediate action advance a universal settlement schedule would
force unrelated domains to share a clock.

### Pick the Bevy mechanism by the guarantee the reader needs

| Reader's question | Engine precedent | Consequence here |
| --- | --- | --- |
| Who currently points to this holder? | Relationship hooks maintain a reverse collection when the structural operation is applied. | Inventory can enumerate the wearer's items immediately after the move executes. The exchange owns cross-slot rules and its outcome. |
| Should ordinary simulation discover this entity at all? | A disabling component changes the default query population while preserving identity and relationships. | Birth and script lookups can see dormant bodies deliberately. Every new targeting system gets the ordinary population by default. |
| What effective value do many readers share? | Transform propagation publishes a world pose. Visibility propagation publishes inherited visibility. | Name the sources and refresh point of effective actor figures. A consumer requiring a more recent answer can evaluate the same rule on current inputs. |
| Is this subject eligible for this particular reader or action? | Rendering still performs view-dependent checks after hierarchy visibility is determined. | Hostility between two actors and the legality of a cast against one target need the participants. A single marker on one participant cannot encode all those answers. |

When expanding a system, follow one mutation through removal as well as
insertion. Dropping a shield changes a reverse lookup when the edge is removed,
can change a published armour figure at its refresh point, and can generate a
story consequence at successful completion of the drop. Those three readers
have different timing requirements even though they concern the same item.
[Inventory ownership](inventory.md#what-the-relationship-guarantees) ·
[Dormant population](gameplay.md#monster-population) · [Hostility](gameplay.md#hostility)

### Work out whether a change needs history

Changed compares a component tick with the requesting system's previous and
current run ticks. Iterating the query still checks the matching population.
RemovedComponents yields the affected entity, with the old component data
already gone. Consequently neither mechanism alone answers “which bearer lost
this shield?” after a transfer. Bevy's own relationship discard hook can still
read the outgoing target before replacement and update its reverse collection.
Insert and discard hooks only cover structural writes, which is why Bevy pairs
this indexing pattern with immutable relationship components. [Change filter](https://docs.rs/bevy_ecs/0.20.1/src/bevy_ecs/query/filter.rs.html) · [Lifecycle hooks](https://docs.rs/bevy_ecs/0.20.1/src/bevy_ecs/lifecycle.rs.html)

Redressed already recovers former wearers from changed or removed reverse
collections. Three figure systems each collect their own set of wearers,
including item changes relevant to other readers. For a retained equipment
projection, collect its affected wearers once per publication, including the
old and new bearer and in-place changes to the capabilities it reads. Rebuild
that result from current active gear. A transfer A → B → A before publication
can coalesce for that calculation. The inventory action that owes story a
successful acquisition still reports its own outcome on each qualifying move.
The affected entities are scratch space belonging to the calculation. Give
them a longer lifetime only if invalidation and publication really occur at
different times. [Equipment changes](../../src/game/items.rs) · [Figure readers](../../src/game/actors/figures.rs)

## Give the reader a path through an action

For a system driven by a request, start at the entity the request identifies.
Resolve the relationships needed for this decision, read the capabilities of
the participants, apply the rule, and expose what actually happened. For a
system driven by elapsed time, start from the population whose time advances.
This makes the system signature explain both why the system runs and which
entities it can affect. A function called once for a known target should not
rediscover that target by scanning every possible bearer.

A named query is useful when it names a role with recurring semantics: an
attack's defender or the contents of a container. Destructure its borrowed
components where the rule uses them. A SystemParam is useful when one question
requires traversing several relationships. Places, for instance, resolves a
member through the party body and a monster through its own pose. Its current
of method substitutes the world origin when the participant has no place.
For a pending or vanished target, expose absence and let the targeting rule
decide what happens. For an actor guaranteed to be placed, enforce that
invariant at its selection point. A fabricated coordinate makes downstream
geometry legible while silently giving it the wrong subject.
[Spatial query](../../src/game/actors/place.rs)

Keep the sequence of an indivisible action readable in one function. Extract
a formula when it names a game decision and its inputs can be borrowed
directly. In melee, the caller should be able to read interception, application
of the blow, retaliation and the next eligibility check in their observable
order. The shared hit rule can remain substantial and independent. A missile
calls it when it arrives; a swing calls it immediately. This preserves one
implementation for both without forcing the immediate caller to manufacture a
temporary entity just to receive its result. [Hit resolution](../../src/game/hits/mod.rs)

## Extend at the point of variation

| Requested change | Where it belongs |
| --- | --- |
| Another weapon using existing rules | Its content definition supplies the same capabilities. Existing targeting and damage readers work for its bearer. |
| A new cause of an existing harm | The source determines when and whom it hits. The existing effect applies damage, resistance and resulting bodily changes wherever the same rules apply. |
| A genuinely new effect | Add its data and resolution at the rule that applies effects. Sources capable of delivering it pass it through. Consumers that display a new outcome can deliberately handle it. |
| A new reaction to a qualifying pickup | Consume the completed storage outcome. If retail requires the reaction to finish before the next story condition, invoke it at that ordered boundary. Independent announcements may observe it separately. |
| A new kind of dormant actor | Give the actor its own activation condition and lifetime owner. Ordinary combat and perception queries continue selecting participants that are active. |

Reading a rule should reveal which order matters. An observer for a second,
independent consequence of death is a good extension point. Two observers whose
relative order determines whether a corpse drops its gear or loses it need an
explicit owner for that sequence. Likewise, an exclusive world operation can
be appropriate for a multi-item exchange with structural writes; a read-only
target search or damage formula should keep its narrower borrows. These choices
follow the lifetime and causal dependencies of the operation.

## Replace the hand recalculation pipeline

The clearest candidate is actor arms. Equipment already relates item entities
to slots. Character actors acquire hand entities when Calling is added.
The grip system reads the equipment; successive systems write reach, modes,
rolls, effects, handling, hit rating, tempo and attack traits onto the hands.
Three more systems traverse the arms from the actor to reconstruct Wielding,
Tempo and reach summaries. They run as part of Settle on both the frame and
fixed schedules. The final three queries traverse all matching bearers each
time. Using set_if_neq avoids signalling unchanged outputs but still performs
the traversal. [Arms](../../src/game/actors/arm.rs) ·
[Settlement](../../src/game/actors/fitness.rs)

Start from the three times the data matters:

- **Available attacks now.** For an equipped actor, enumerate usable attacks
  from active slots, item capabilities, compatible ammunition and the actor's
  training. Unarmed attacks can come from the actor's capabilities. A species
  supplies its ordered natural attack definitions at spawn. One shared attack
  rule consumes the selected source. A hand slot has a stable name and a
  weapon already has an entity. Use those identities where callers currently
  navigate a synthetic hand entity. Preserve order wherever first and furthest
  attacks give different answers.
- **Accepted for this round.** The turn records which weapons were accepted and
  the scheduled attacks. This is genuine history: changing equipment afterwards
  must be distinguishable from having accepted the new weapon. At revalidation
  compare the recorded weapon entity with the one currently in the slot.
  [Turn acceptance and revalidation](../../src/game/combat/turn.rs)
- **Resolved for this swing.** Borrow the chosen source, actor and defender.
  Calculate values whose lifetime is this resolution as locals. Apply the blow,
  consume the actual ammunition and reevaluate live eligibility for the next
  swing. Keep an independent entity when something continues in the game,
  such as a missile still flying after its caster falls.

Remove the mirrored hand state, its actor summaries and the settlement chain
that keeps them aligned in the same rewrite as the consumers. A tactical query
that inspects every actor may justify a cached reach; establish the cost of
that query and identify its complete inputs before retaining the cache.
Preserve weapon-set changes, natural multiattack, last-ammunition consumption
and in-place weapon upgrades through the real turn and targeting paths.

## Choose a consistency boundary for actor conditions

The swing loop checks the target's Vitals directly and the attacker's CanFight
marker. CanFight is one of five markers that judge inserts or removes through
Commands after observing changed Vitals or Conditions. Both inputs can change
during an attack. Flushing the commands from a hit does not run judge. Thus a
single eligibility check reads two different points in game time. Whether a
specific retaliation currently exposes stale eligibility needs a gameplay
trace. [Swing loop](../../src/game/combat/swings/mod.rs) ·
[Fitness markers](../../src/game/actors/fitness.rs)

For a rule acting on an identified body, evaluate its relevant threshold
against its current vitals and conditions at the point of decision. For a
query selecting a population, a marker can earn its place if many systems
select that population and its update point is defined. Keep death attribution
and consequences explicit: health reaching zero and a death already processed
are different facts. Avoid using the next frame's change filter to repair
state that a second action in the same moment is entitled to read.

The same issue affects derived figures. Attributes and agility are in one
unordered inner tuple, even though agility reads Attributes. Attributes reads
Mastery while its change filter names Calling. That works only under an
extra assumption about all writers of Mastery. Delete cheap leaf projections
such as standalone agility where the caller can read it from attributes.
Retain a derived value when its repeated use warrants storage, and define
which inputs, including insertions and removals, make it stale.
[Figure writers](../../src/game/actors/figures.rs)

## Give suspension an owner

The aside plugin propagates Disabled through several relationship kinds,
including level ownership, transform parenting, equipment and arms. Removing
Disabled from a holder removes it from its dependents unconditionally.
Inserting a relationship to a disabled holder sets the flag; inserting one
to an active holder does not clear a previous inherited flag. The bit records
neither the cause nor the owner. Meanwhile waking an actor marks every mutable
component changed, waking unrelated readers as well.
[Propagation](../../src/game/world/aside.rs) · [Wake handling](../../src/game/actors/fitness.rs)

Choose a single gameplay lifetime owner for each dependent. A carried item
follows its bearer, a ground item follows its level, a model part follows its
body. Spatial parenting contributes lifetime only for parts actually owned by
that parent. Resolve effective activity from the new owner on transfer.
Represent independent dormancy on the object that owns that decision: a
dismissed companion and an unborn group have different activation rules.
Materialize Disabled at the engine boundary for normal query exclusion.
A level waking may clear inherited suspension; it cannot implicitly give
birth to a dormant group.

Persistent model derivations that must stay current while an actor is aside
can deliberately include disabled entities. Expensive state needed only on
activation can be initialized by that transition. Remove the archetype-wide
set_changed replay. Exercise transfer across active and suspended owners,
returning to a level with an unborn group, and waking a retained companion
with the same gear. These are design cases; no runtime failure was reproduced.

## Use change detection for the question it can answer

The story wants to know that automatic storage succeeded. The current Update
sweep observes which items are in party possession and adds Owned. End-of-frame
membership cannot recover a transfer into and back out of the party during the
same frame, or distinguish two qualifying reports while the item stays in the
party. Emit the storage outcome at the operation. Use a relationship query for
a different question: what does the party hold now? Likewise, a UI showing
current health can refresh from changed vitals while a death reaction needs
the actual transition and its cause. [Inventory rules](inventory.md) ·
[Retail acquisition paths](retail-evidence.md)

These choices eliminate structural changes made solely to carry control flow,
full-population maintenance passes and artificial dependencies between schedules.
Their value can be checked by reading which facts a gameplay decision now
requires. Performance gains require measurement.
