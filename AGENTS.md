# Agent guidelines

Rizzardry rewrites Wizardry 8 in Rust and Bevy. Preserve its gameplay and
character, not its C++ architecture.

## Design

- Prefer deletion, ordinary Rust and existing Bevy features. Fix the data model
  instead of layering flags, adapters and synchronization over a bad boundary.
- Every abstraction or dependency must simplify a real use case. No speculative
  frameworks, one-use forwarding layers, mirrored models or compatibility glue.
- Establish behavior from the original game and decomp; modernized source is
  supporting evidence, not automatically equivalent. Record deliberate deviations.
- Define state by meaning and lifetime: current fact, draft intent, accepted work,
  historical snapshot, derived value/index or presentation history. Give each fact
  one authority and explicit mutation operations; use cohesive structs/enums for
  coupled values and mutually exclusive alternatives. Assets hold shared definitions;
  entities hold live actors/devices; resources hold genuinely global state;
  ordinary values hold drafts and records. Split components by independent
  behavior/lifetime, not one field per component or old class boundaries.
- Share rule functions before storing their results. Caching needs a concrete cost,
  query or integration benefit. Every stored derivation/index needs named sources,
  a producer, all invalidation paths (including initialization, missing inputs,
  removal and restoration), and the boundary at which consumers may trust it.
  Consumers must not independently patch derived results.
- Use relationships for actual relationships; Bevy maintains their inverses, not
  dependencies on other gameplay facts. Required components ensure insertion,
  not value freshness; hooks observe lifecycle operations, not in-place mutation.
  Scene ownership may despawn visual children, never unrelated party members,
  targets or group members.
- PartyRoster owns membership/order. A Character query is not a party query.
  Retained NPC profiles do not imply active party membership or identical role stats.
- Gameplay operations validate authoritative state and commit required consequences
  together. Damage/death, item transfers and acceptance must not depend on loosely
  ordered observer chains. Define deferred visibility and schedule boundaries when
  they affect dependent readers. Events/messages suit requests and notifications,
  not a second authority. Explicit pending state owns work that outlives a frame.
- Distinguish editing, acceptance, execution and presentation. Accepted work keeps
  actor identity and paid costs; later roster or UI edits cannot redirect it.
  Before recomputing stored data, preserve whether it means now, at acceptance,
  at round start or after the last redistribution (including burden during combat).
- Level preparation is not activation. Reject missing mandatory content; preserve
  map-owned state in dormant records and restore it on revisit. Views cannot
  initialize campaign state or make gameplay readiness depend on artwork.
- Keep time domains and RNG consumption explicit. Settle consequences under the
  originating policy/level, retain time remainders and define target order. UI and
  cosmetics never consume gameplay RNG. System-local caches are not game history.
- Dependency direction: retail decodes into content; game consumes content and owns
  rules; presentation observes game state; app composes them. No reverse imports,
  giant Game object or second position authority alongside physics.
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
  changes do not need an engine rebuild.
- Do not claim checks you did not run or visual parity from headless tests.
- Keep usage instructions in README and development rules here. Put research,
  plans and temporary decisions in issues/PRs, not a growing docs archive.
- Always open a non-draft PR for completed changes. Merge only when the user requests it.
