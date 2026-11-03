# Agent guidelines

Rizzardry rewrites Wizardry 8 in Rust and Bevy. Preserve its gameplay and
character, not its C++ architecture.

## Start with the behavior

- Prefer deletion, ordinary Rust and existing Bevy features. Fix the data model
  instead of layering flags, adapters and synchronization over a bad boundary.
- Before changing a feature, trace its authoritative state, writers, consumers and
  schedule. Read a nearby implementation, but check whether its ownership fits
  this use case. Existing code is evidence, not a template to copy unquestioningly.
- Establish behavior from the original game and decomp; modernized source is
  supporting evidence, not automatically equivalent. Record deliberate deviations
  in the PR. Check Bevy APIs against the version used by this repository.
- Name domain concepts by meaning. Decode categorical bytes and masks into named
  types; use shared named content IDs in rules. Keep raw encodings at the retail
  boundary. Investigate uncertain meanings instead of inventing numbered abilities
  or propagating unexplained values into gameplay/UI.
- Every abstraction or dependency must simplify a real use case. No speculative
  frameworks, one-use forwarding layers or compatibility glue. Good ECS code is
  measured by clear ownership and behavior, not component or system count.

## Model state and ownership

- Define state by meaning and lifetime: current fact, draft intent, accepted work,
  historical snapshot, derived value/index or presentation history. Give each fact
  one authority and explicit mutation operations; use cohesive structs/enums for
  coupled values and mutually exclusive alternatives.
- Use entities for things with independent identity/lifecycle or relationships,
  components for their state, assets for shared definitions, and resources for
  genuinely global state. Keep nested values as ordinary Rust structs/enums unless
  independent querying or lifetime makes them entities/components. A singleton
  domain actor can still be an entity; a resource must not become an actor registry
  that duplicates the ECS.
- Split components when behavior, access patterns or lifetime differ. Keep fields
  together when they express one invariant and change together. For example, health
  and stamina can belong to cohesive vitals; an accepted cast has a different
  lifetime from the actor's editable order. Avoid both one field per component and
  one component containing every subsystem's state.
- Share rule functions before storing their results. Caching needs a concrete cost,
  query or integration benefit. Every stored derivation/index needs named sources,
  a producer, all invalidation paths (including initialization, missing inputs,
  removal and restoration), and the boundary at which consumers may trust it.
  Consumers must not independently patch derived results. Snapshots preserve past
  facts; do not treat them as caches to refresh from current state.
- Use relationships for actual relationships; Bevy maintains their inverses, not
  dependencies on other gameplay facts. Required components ensure insertion,
  not value freshness; hooks observe lifecycle operations, not in-place mutation.
  Scene ownership may despawn visual children, never unrelated party members,
  targets or group members.
- PartyRoster owns membership/order. A Character query is not a party query.
  Retained NPC profiles do not imply active party membership or identical role stats.

## Write systems with explicit access and order

- Systems select ECS data and coordinate work; ordinary functions and methods
  express reusable rules and cohesive mutations. Give each system a clear domain
  operation. Do not split an invariant across systems merely to make them smaller.
- Prefer typed queries and resource parameters that expose actual reads/writes.
  Request mutable access only where needed; use filters to express the population
  and disjoint access. Use a SystemParam for a cohesive operation's dependencies,
  not to hide unrelated access or satisfy an argument-count preference.
- Use exclusive World access when a cohesive operation needs it, not as the default
  service interface. Resolve borrow conflicts through ownership and access design;
  do not clone live state into a second writable model to write it back later.
- Order systems for real producer/consumer dependencies. At each relevant boundary,
  distinguish direct mutations from deferred Commands and state when queued changes
  must be visible. Neither registration order nor conflicting mutable access is an
  ordering contract. Avoid chaining an entire plugin to fix one missing dependency.
- Gameplay operations validate authoritative state and commit required consequences
  together. Damage/death, item transfers and acceptance must not depend on loosely
  ordered observer chains. Use synchronous calls for required parts of one operation;
  events/messages for requests and notifications, with an explicit consumer and
  delivery lifetime. Pending state owns work that must survive delayed consumption.
- Use observers/hooks for local reactions and lifecycle maintenance with clear
  invariants. Change detection is a signal to inspect current state, not a history
  of domain transitions or a complete invalidation policy. Handle removal and
  restoration explicitly wherever consumers retain derived state.

## Preserve gameplay across lifetimes

- Distinguish editing, acceptance, execution and presentation. Accepted work keeps
  actor identity and paid costs; later roster or UI edits cannot redirect it.
  Before recomputing stored data, preserve whether it means now, at acceptance,
  at round start or after the last redistribution (including burden during combat).
- Retained entity references need a policy for despawn, removal and cancellation.
  Revalidate at execution as the rule requires; do not silently retarget a roster
  slot or default away a broken invariant. Handle expected absence explicitly.
- Level preparation is not activation. Reject missing mandatory content; preserve
  map-owned state in dormant records and restore it on revisit. Views cannot
  initialize campaign state or make gameplay readiness depend on artwork.
- Keep time domains explicit. Settle consequences under the originating
  policy/level and retain time remainders. Retail's order of random draws need not
  be reproduced. UI and cosmetics never consume gameplay RNG. System-local caches
  are not game history.
- Dependency direction: retail decodes into content; game consumes content and owns
  rules; presentation observes game state; app composes them. No reverse imports,
  giant Game object or second position authority alongside physics. Gameplay must
  run without rendering or UI plugins.
- Reuse native Bevy input, focus, widgets, assets and lifecycle. Keep views alive
  across value changes. UI state owns only presentation and uncommitted drafts;
  gameplay commits revalidate them. Explicitly gate shortcuts behind focused UI.

## Verification and workflow

- State refactor PRs include a short ownership note: what each representation means,
  its authority and mutation operations, why any derived storage is needed and
  when it is valid, and the fallback/cancellation/removal/restore paths checked.
  Judge simplification by remaining writable mirrors, mutation paths, invalidation,
  cleanup, ordering dependencies and opportunities for consumers to disagree.
- Write only meaningful tests: real regressions, important rules, or ownership and
  lifecycle boundaries. Not every helper, UI detail or change needs a test, and
  exhaustive permutations are not a goal. Do not build elaborate fixtures or
  change good production architecture merely to test it.
- Pick the cheapest convincing evidence: a pure rule test, a small real-schedule
  Bevy scenario, or an actual game check. Focus checks on the change; docs-only
  changes do not need an engine rebuild. For ordering/deferred-visibility changes,
  exercise the relevant production schedule boundary; calling functions in the
  desired order does not verify scheduling. Check the affected lifecycle failure
  path, not only initial spawn and the happy path.
- Do not claim checks you did not run or visual parity from headless tests.
- Keep usage instructions in README and development rules here. Put research,
  plans and temporary decisions in issues/PRs, not a growing docs archive.
- Always open a non-draft PR for completed changes. Merge only when the user requests it.
