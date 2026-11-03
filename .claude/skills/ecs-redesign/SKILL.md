---
name: ecs-redesign
description: Refactor gameplay around explicit ownership, independent lifetimes, shared actor capabilities, and cohesive operations. Use when reviewing or simplifying gameplay architecture and its ECS boundaries.
---

# Gameplay ownership and execution

Apply the project rules in [AGENTS.md](../../../AGENTS.md). This skill supplies a
refactoring procedure and examples; ownership and verification policy live there.

## 1. Inventory behavior and ownership

Read the implementation and retail evidence. Trace the affected target selection,
costs, rolls, conditional follow-ups, death, interruption, movement, reflection and
cleanup. Identify state that survives each operation and consequences that must
settle before the next actor or swing.

For substantial changes, describe the intended ownership/execution boundaries in
the PR. An approved task or review can supply the design; routine choices within
that scope do not need repeated approval.

## 2. Choose representation and execution boundaries

Use these examples to apply the criteria in AGENTS.md:

| Concept | Candidate representation |
| --- | --- |
| Independently used actor capabilities | Components, including single-field ones |
| Coupled damage, absorption, fatigue and death preparation | A cohesive vitals operation |
| Editable order versus accepted cast | Separate values with distinct lifetimes |
| Independently delayed projectiles or impacts | Entities with progress and ownership |
| Immediate aim, penetration, damage or sequence cursors | Local values and ordinary functions |
| Mutually exclusive actions | An enum and a local `match` |

Inspect [fighter projection](../../../src/game/combat/fighters.rs):
`judge_fighters` translates character/monster inputs into common capabilities;
`Fighter::write` publishes them with equality-aware updates. Compare those choices
to the consumers' needs rather than copying the component layout.

Inspect [impact resolution](../../../src/game/hits/resolve.rs): `impacts` runs
immediate effects in a local sequence, with explicit capability/consequence
boundaries; impacts waiting for flight retain entities. Several occurrences can
remain a collection owned by one component when none needs independent identity.

## 3. Trace freshness and cleanup

For changed representations, identify owner, writer, readers and lifetime. Follow
source changes through publication to the first dependent consequence. Check which
values intentionally freeze and which must refresh:

- An accepted snapshot ignores later intent edits.
- A live capability reflects changed inputs before a consequence consumes it.
- A historical result, such as side at death, stays frozen.

Include removal and membership changes in the trace. For combat, distinguish battle
participation from party membership/seating: leaving a fight cleans the participant;
ending it follows participation relationships. Check cancellation and delayed work
without introducing a global scan for stray temporary components. Ensure executors
cannot mutate shared definitions to select their next effect.

## 4. Verify behavior and remove obsolete machinery

Choose checks using AGENTS.md's proportional verification policy. Assert observable
outcomes and external contracts, including production schedule behavior when order
matters. Replace assertions that merely pin internal layouts/stages, while keeping
meaningful freshness and lifecycle regressions. Derive expected results from game
rules rather than reproducing implementation arithmetic.

Check that replaced state, synchronization and cleanup paths are gone. Review
remaining ownership complexity as well as code size. Use
[CI](../../../.github/workflows/ci.yml) for exact automated check commands and
[README.md](../../../README.md) for game/retail checks. Run applicable checks for the
change; retail scenarios require game data and are useful when affected behavior
needs integration evidence, not as a blanket requirement for every edit.
