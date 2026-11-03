# Agent guidelines

Rizzardry rewrites Wizardry 8 in Rust and Bevy. Preserve its gameplay and
character, not its C++ architecture. See [README.md](README.md) for running the
game and [CI](.github/workflows/ci.yml) for exact automated check commands.

## Start with the behavior

- Trace authoritative state, writers, consumers and scheduling before changing a
  feature. Check nearby code for useful patterns, not templates to copy blindly.
- Establish behavior from the original game and decomp; modernized source is
  supporting evidence. Record deliberate deviations in the PR. Check Bevy APIs
  against this repository's version.
- Prefer deletion, ordinary Rust and existing Bevy features. Fix ownership instead
  of adding flags or synchronization. Abstractions and dependencies must simplify
  a real use case; avoid speculative frameworks and one-use forwarding layers.
- Decode categorical bytes/masks into named types and use shared content IDs in
  rules. Keep raw encodings at the retail boundary; investigate uncertain meanings.
- Implement shared rules against shared capabilities. Translate actor-specific
  source data at a clear boundary; separate implementations by actor kind only
  where the game rules differ.

## Model state and ownership

- Give each fact one authority and explicit mutation operations. Distinguish current
  state, draft intent, accepted work, historical snapshots and derived values.
  Use cohesive structs/enums for coupled invariants and mutually exclusive choices.
- Entities provide independent identity/lifetime or relationships; components hold
  their state; resources hold genuinely global state, not duplicate actor registries.
  Keep nested values ordinary Rust unless independent querying/lifetime warrants ECS
  representation. A singleton domain actor can still be an entity.
- Store shared definitions in immutable catalogs or assets. Use assets when loading,
  handles or lifecycle provide a concrete benefit.
- Split components by behavior, access or lifetime, not to make them smaller; do not
  combine unrelated state to reduce component count. Single-field components are
  appropriate for independently used capabilities. Keep coupled invariants together.
- Share rule functions before storing results. Derived storage needs a concrete
  cost, query or integration benefit; name its sources, producer, invalidation paths
  (initialization, missing inputs, removal, restoration) and consumer trust boundary.
  Consumers must not patch it independently. Historical snapshots stay historical.
- Use relationships for actual relationships. Bevy maintains inverses, not dependent
  gameplay facts. Required components ensure insertion, not freshness; hooks observe
  lifecycle operations, not in-place writes. Scene cleanup may despawn visuals,
  never unrelated party members, targets or group members.
- `PartyMember` represents membership; Bevy maintains `PartyRoster` as its ordered
  inverse. Change membership through relationship APIs, not a second list. Querying
  `Character` alone does not establish membership. Retained NPC profiles imply
  neither active membership nor identical role stats.

## Write systems with explicit access and order

- Introduce system boundaries for independently scheduled behavior or meaningful
  synchronization. Keep intermediate calculations and immediate sequences in local
  values/functions. Give work an entity when other systems need its identity,
  progress or independent lifetime; do not split an invariant just to shorten code.
- Prefer typed queries/resources exposing actual reads/writes; request mutable
  access only where needed. Filters express populations and disjoint access.
  Use `SystemParam` for cohesive dependencies, not to hide unrelated access.
- Use exclusive `World` access when a cohesive operation needs it, not as every
  rule's interface. Resolve borrowing through ownership/access design, not cloning
  live state into another writable model to write back later.
- Order real producer/consumer dependencies and specify when deferred `Commands`
  become visible. Registration order and conflicting mutable access do not define
  execution order. Do not chain an entire plugin to fix one missing dependency.
- Validate authoritative state and commit required consequences within one cohesive
  operation or explicitly ordered boundary. Damage/death, transfers and acceptance
  must not depend on loosely ordered observer chains. Use buffered `Message`s for
  scheduled consumers and observer `Event`s for triggered reactions. Specify timing
  and retention; store work explicitly when it must survive delayed consumption.
- Change detection signals current state, not transition history or complete
  invalidation. Handle removal/restoration explicitly. When publishing derived state,
  avoid marking unchanged results as changed; use equality-aware updates as appropriate.

## Preserve gameplay across lifetimes

- Separate editing, acceptance, execution and presentation. Accepted work retains
  actor identity and paid costs despite roster/UI edits. Preserve whether stored
  data means now, at acceptance, at round start or after redistribution (including
  combat burden). Retained entity references need removal/despawn/cancellation
  policies; revalidate as rules require without silently retargeting roster slots
  or defaulting away broken invariants.
- Preparation is not activation. Reject missing mandatory content; preserve dormant
  map state and restore it on revisit. Views cannot initialize campaign state or
  make readiness depend on artwork. Gameplay runs without rendering/UI plugins.
- Keep time domains explicit; settle under the originating policy/level and retain
  remainders. Retail RNG draw order need not be reproduced; UI/cosmetics never use
  gameplay RNG. System-local caches are not game history.
- Dependency direction: retail decodes into content; game owns rules; presentation
  observes game state; app composes them. No reverse imports, giant Game object or
  second position authority alongside physics.
- Reuse native Bevy input, focus, widgets, assets and lifecycle. Keep views alive
  across value changes. UI owns presentation and uncommitted drafts; gameplay
  revalidates commits. Gate shortcuts behind focused UI.

## Verification and workflow

- State refactor PRs include a short ownership note: meaning, authority/mutations,
  justification and validity of derived storage, and affected cancellation/removal/
  restoration paths checked. Judge simplification by remaining mirrors, mutation
  paths, invalidation, cleanup, ordering and opportunities for consumers to disagree.
- Test important rules, regressions and ownership/lifecycle boundaries. Choose the
  cheapest convincing evidence: a pure rule test, real-schedule scenario or game
  check. When ordering/deferred visibility matters, exercise observable behavior
  through the production schedule; calling functions in the desired order is not
  enough. Avoid assertions that merely freeze component layouts or scheduler stages.
- Check affected failure paths, not only initial spawn and happy paths. Keep testing
  proportional; avoid elaborate fixtures or production changes solely for tests.
  Docs-only changes need no engine rebuild. Report only checks actually run;
  headless tests do not establish visual parity.
- Keep usage in README, project rules here, and research/plans in issues or PRs.
  The ECS redesign skill supplies procedure/examples, not a second rulebook.
- Always open a non-draft PR for completed changes. Merge only when requested.
