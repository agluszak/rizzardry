# Rizzardry — Agent Guidelines

Rizzardry is a from-scratch Rust and Bevy rewrite of Wizardry 8.

The goal is to preserve the original game's gameplay, content, and character while replacing its implementation with something substantially simpler, more maintainable, and extensible.

**We are rewriting the game, not translating the C++ code.**

Read [docs/README.md](docs/README.md) for the architecture, gameplay designs, and implementation plan. These documents describe the intended direction, not a list of abstractions that must be implemented.

## 1. Simplicity above all

**The best code is code we don't have to write.**

- Prefer removing code to adding it.
- Prefer ordinary functions, structs, enums, and collections.
- Use the standard library and existing Bevy functionality instead of reinventing them.
- Don't introduce an abstraction until a concrete problem requires it.
- Don't build infrastructure for hypothetical future requirements.
- Avoid one-use wrappers, forwarding methods, unnecessary indirection, and layers that merely rename things.
- Avoid generic frameworks when a few straightforward functions solve the problem.
- Don't preserve obsolete code or interfaces for compatibility. There is no external API to protect.

Judge complexity by how much a reader must understand, not merely by line count. A little duplicated, obvious code can be better than an elaborate abstraction. Conversely, fixing the underlying data model can eliminate entire categories of duplication.

**Every new abstraction must justify its existence.**

Being idiomatic Rust or following a design pattern is not sufficient justification.
Evaluate libraries against the actual code they replace: compare the complete
implementation, intermediate types, errors, and remaining custom parsing. Do not
adopt a dependency for a few shorter helper calls or keep two parsing mechanisms
without a concrete benefit.

## 2. Design the right system, not the smallest patch

Before changing an important subsystem, understand what it is supposed to accomplish.

If the implementation is complicated, don't assume its current structure is necessary. Consider how the subsystem would look if designed correctly from the beginning.

In particular:

- Look for duplicated representations of the same fact.
- Look for mutable state that must be manually synchronized.
- Look for unnecessary ownership and lifetime complications.
- Look for systems whose responsibilities overlap.
- Look for abstractions that compensate for a poor underlying model.

Fix these problems at their source.

Don't solve an ownership problem by adding more ownership machinery. Don't solve a synchronization problem by adding another synchronization mechanism. Don't move complexity into a helper and call it simplification.

Prefer making invalid states impossible to represent over adding runtime checks to detect them.

Choose data structures first. Once the data model is right, the operations should usually be straightforward.

## 3. Use the original game as evidence, not architecture

Two reference implementations are available:

- [wizardry-8-decomp](https://github.com/agluszak/wizardry-8-decomp): recovered original implementation.
- [whizzardry-8](https://github.com/agluszak/whizzardry-8): C++ modernization.

Consult both when behavior or requirements are unclear.

The decomp is the primary source for original behavior. Whizzardry can clarify recovered mechanics and provide useful architectural lessons, but its behavior may differ.

Neither implementation dictates our Rust design.

Do not reproduce C++ class hierarchies, global state, memory layouts, callback systems, or workarounds simply because they exist in the references.

Preserve meaningful game behavior, including ordering, timing, and unusual rules. Do not blindly preserve implementation accidents or undefined behavior.

When the sources disagree, investigate. When behavior remains uncertain, document the uncertainty instead of inventing a rule.

Deliberate gameplay changes are acceptable, but they must be explicit.

## 4. Let Bevy do its job

Bevy is our engine, not a rendering library underneath a second custom engine.

Use its ECS, assets, scenes, UI, input, rendering, audio, and scheduling facilities directly.

Don't recreate functionality already provided by Bevy or our selected dependencies.

For movement and navigation, our intended stack is:

- Avian for physics and spatial queries.
- Rerecast for navigation-mesh generation.
- Landmass for pathfinding, steering, and avoidance.
- Tnua if a dedicated character controller is needed.

Only write custom engine functionality when an actual requirement cannot reasonably be satisfied by these libraries.

Favor native Bevy components and relationships over parallel object hierarchies.

Do not introduce service locators, global getters, or manager classes to hide dependencies.

## 5. One authoritative representation of each fact

The ECS is authoritative for live gameplay state.

Use:

- **Entities** for things with independent identity, lifetime, or behavior.
- **Components** for state belonging to those entities.
- **Resources** for genuinely shared or session-wide state.
- **Ordinary values** for data that does not need an independent entity.
- **Assets** for immutable definitions and reusable content.
- **Save records** for persistent state, not as a second live game model.

An inventory item does not need to be an entity. Neither does every condition, attribute, spell, quest fact, or map marker.

Avoid duplicating state between resources, components, and UI models. Derive information when possible rather than storing another mutable copy.

Ownership should be unambiguous. An item belongs to exactly one inventory, container, world pile, or active transfer state.

Use stable domain identifiers where references must survive saving and loading. Bevy entity IDs are runtime identifiers.

Keep components reasonably cohesive. Do not split every field into its own component unless independent querying or behavior benefits from doing so.

- A level being decoded or its geometry being spawned does not mean it is
  playable. One activation boundary establishes readiness for gameplay;
  missing mandatory content is a load failure, never a silent no-op action.
- Map-owned state (killed/wounded local monsters, opened containers, trigger
  state, notes) survives departure in that map's dormant record. On revisit,
  restore it instead of respawning authored defaults. Do not persist runtime
  caches and temporary interaction state merely because they share a struct.
- `PartyRoster` is the authority for membership and order. A query over every
  `Character` is not a party query; a retained NPC character profile is not
  automatically a party member. Gameplay RNG consumers use a defined order.

## 6. Gameplay operations must be coherent

Gameplay rules should be ordinary, understandable Rust code.

Operations such as `apply_damage`, `move_item`, `consume_item`, `buy_item`, `recruit_npc`, and `commit_level_up` must leave their affected state consistent.

**Do not implement mandatory gameplay consequences through chains of loosely ordered observers or events.**

For example, dealing lethal damage must perform the relevant death bookkeeping before another combat action can act on that character. A UI notification about the death can happen afterward.

Use events and observers for notifications, interactions, and presentation when appropriate. Do not turn the entire game into a generic event bus.

The operation that changes gameplay state checks its own prerequisites and
permissions using the actual authoritative state. UI previews may reuse pure
eligibility functions, but an input-context gate or a caller-supplied
`can_use` boolean must not be the only validation. Fail explicitly if the
operation cannot run; do not partially commit because a catalog, actor, or
level is not ready. Keep required consequences and the state transition in
one ordered operation.

For operations spanning multiple frames, represent the pending work explicitly. Examples include executing combat actions, projectiles, cutscenes, conversations, and rest.

Keep these distinctions where relevant:

- Editing an action versus accepting it.
- Accepting an action versus executing it.
- Executing an action versus presenting its consequences.
- A temporary UI selection versus persistent game state.

Do not introduce a universal state-machine or workflow framework. Small, specific enums and functions are usually enough.

## 7. Keep presentation separate from game rules

Rendering, animation, sound, and UI should generally observe gameplay state rather than own it.

However, presentation timing sometimes affects gameplay. A spell may resolve at impact; a scripted scene may wait for dialogue or animation completion.

Model those dependencies explicitly. Avoid assuming that all animation is cosmetic or that gameplay must always be driven by animation callbacks.

Use the appropriate time domain for each operation. Combat rounds, game time, animation playback, and UI time are not interchangeable.
Record *why* an interval advances before changing activity (for example,
finishing combat), and settle its mandatory recovery/condition effects under
that policy. Do not advance the clock in one schedule and infer the old mode
from a resource that may already be gone in another. Preserve sub-tick
remainders; `Local` caches must not own gameplay history.

Gameplay randomness must be controlled and reproducible. UI previews and cosmetic effects must not consume gameplay random numbers.

Where ordering affects results, make it explicit rather than relying on incidental ECS iteration order.

## 8. Keep modules cohesive

Start with one Cargo package and ordinary Rust modules.

Do not split code into crates, plugins, traits, or interfaces without a demonstrated need.

A Bevy plugin is useful when a module registers systems, assets, or engine integration. Pure gameplay rules do not need a plugin merely for consistency.

Avoid circular dependencies. Prefer moving genuinely shared data types into a small common module over introducing an abstraction to route calls between two tightly coupled systems.

Do not create a giant `Game` object or `GameState` that duplicates the ECS and becomes a dependency of every function.

Dependencies should be visible. Functions should receive the data they need rather than retrieving arbitrary global state.

Keep dependency direction simple: `retail` decodes into `content`;
`game` consumes content and owns gameplay; `presentation` reads gameplay
and owns views; `app` composes them. `content` must not import `retail`,
and game rules must not depend on UI, meshes or playback. Place genuinely
shared types below both users instead of introducing mirrored DTOs or
reversing dependencies. Gameplay readiness and legality do not belong to
the presentation layer.

## 9. Use Rust idiomatically, but pragmatically

Prefer straightforward Rust over clever Rust.

- Use `Vec`, slices, `Option`, `Result`, and standard collections.
- Use enums to represent meaningful alternatives instead of magic numbers and combinations of flags.
- Use strong domain types when they prevent real mistakes, not as decoration around every primitive.
- Prefer explicit control flow to complex generic machinery.
- Keep mutable state scoped as narrowly as practical.
- Avoid unnecessary cloning, allocation, reference counting, and interior mutability.
- Don't use macros when ordinary functions are clearer.
- Don't introduce traits with only one implementation unless they serve a concrete purpose.

Make error handling appropriate to the boundary. Validate untrusted assets, external data, and user input. Internal invariants should be enforced by the data model, not hidden behind defensive fallbacks everywhere.

Comments should explain non-obvious decisions and gameplay quirks, not narrate what the code already says.

## 10. Write code that is easy to change

Optimize for the next developer who needs to understand or modify the subsystem.

A reader should be able to answer:

1. Where is this state owned?
2. Who is allowed to change it?
3. What happens when it changes?
4. What happens when its owner disappears?

without following a long chain of forwarding methods, callbacks, or synchronization hooks.

Prefer short, direct paths from intent to effect.

Avoid speculative extensibility. Modding matters, but that does not mean every subsystem needs a plugin interface, scripting language, registration framework, or dynamically dispatched abstraction.

Make content data-driven where it naturally belongs in data. Keep exceptional gameplay logic in ordinary Rust until there is evidence that a more general mechanism is useful.

## 11. Verify behavior, not implementation details

A successful compilation does not establish correct gameplay.

For meaningful changes, verify the observable result. Use the original game and reference implementations to establish expected behavior where necessary.

Tests should focus on important rules, invariants, and lifecycle boundaries. Prefer a few valuable tests to large amounts of scaffolding.

Use ordinary Rust tests for rules, headless Bevy `App` scenarios for system/lifecycle behavior, and real-game checks when rendering or movement matters. See [testing](docs/testing.md).

Do not add tests merely to protect an implementation detail or justify an abstraction. Refactoring should not require rewriting unrelated tests.

For rendering, movement, interaction, and timing problems, exercise the real game when possible.

Don't claim something works unless it has actually been checked.

## 12. Work toward complete, simple outcomes

Prefer coherent vertical slices over building large amounts of infrastructure before anything works.

Before implementing a substantial change:

1. Understand the required behavior.
2. Identify the authoritative data and its owner.
3. Consider the simplest design that satisfies the requirements.
4. Reuse existing library functionality.
5. Implement the behavior and remove any superseded code.
6. Verify the relevant result.

If a design keeps accumulating special cases, adapters, flags, or synchronization code, reconsider the design instead of continuing to patch it.

Keep the documentation in `docs/` consistent with significant architectural decisions, but do not let documentation become a substitute for working code.

**The objective is not to produce more architecture. It is to need less architecture.**