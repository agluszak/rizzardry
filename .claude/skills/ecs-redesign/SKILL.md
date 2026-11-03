---
name: ecs-redesign
description: Refactor gameplay around explicit ownership, independent lifetimes, shared actor capabilities, and cohesive operations. Use when reviewing or simplifying gameplay architecture and its ECS boundaries.
---

# Gameplay ownership and execution

Start with the game rules and the state that must survive an operation.
Read the existing code and retail evidence to inventory behavior before
changing representation. Preserve coupled invariants and same-step
settlement; ECS is an ownership and composition model, not a requirement
to turn each instruction into a system.

## Choose representation by lifetime

| Concept | Representation |
| --- | --- |
| Actor capabilities and authoritative state | Cohesive components |
| Participation, guarding, ownership of pending work | Relationships with explicit cleanup |
| Editable intent versus accepted work | Separate values with distinct lifetimes |
| Independently delayed projectiles or impacts | Entities with progress and ownership |
| Shared spell/effect descriptions | Immutable catalog values or assets |
| Immediate arithmetic, branches, sequence cursors | Local values and ordinary functions |

Several occurrences can be a collection owned by one component. Give an
occurrence its own entity when it has independent identity, behavior, or
lifetime that another system actually needs. Do not create an entity only
because a loop has several iterations.

Mutually exclusive actions usually belong in an enum. Separate components
are appropriate for facts that can coexist or have independent lifetimes.
A `match` is often the clearest way to express a finite domain choice.

Keep coupled state together: damage, absorption, fatigue, conditions, and
death preparation may legitimately be one `Vitals` operation. Components
should expose the vocabulary consumers need, not mirror individual fields
or terms of a formula.

## Make operations cohesive

A system boundary should represent independently scheduled behavior or a
meaningful synchronization boundary. Calculate aim, penetration, and damage
with ordinary values. Do not publish partially calculated arithmetic into
the world or repeatedly traverse relationships to add individual terms.

Immediate effects can be a local ordered sequence with a local previous
outcome. Settle an operation's consequences before advancing to the next
actor or swing. Delayed impacts retain entities until their actual arrival.
Do not spread synchronous work across frames for architectural neatness.

A focused `SystemParam` can express a useful access boundary. Judge it by
its responsibilities, coupling, and borrowing requirements, not its syntax.
Avoid passing an entire gameplay world facade to every formula. Plain
rules should receive only the values they need. Function length is a reason
to inspect cohesion, not an automatic instruction to split systems.

## State and freshness contracts

For every stored value document its owner, writer, readers, and lifetime.
For a projection also identify authoritative inputs and its validity:

- An accepted snapshot intentionally ignores later input edits.
- A live capability must refresh before a consequence uses changed inputs.
- An historical result (such as the side at death) stays frozen.

Prefer deriving cheap local facts on demand. Keep reusable projections when
they give consumers a common actor vocabulary; skip unchanged inputs and
avoid emitting changes when the result is equal. Removal and membership
changes are inputs too. Definitions must not be selectable by executors.

Battle participation owns fight state for every participating actor. Party
membership still owns seating and formation. Leaving a fight cleans that
actor; ending a fight follows its participation relationships, without a
global scan for stray temporary components.

## Design and verification

Describe the intended ownership and execution boundaries before substantial
changes. An approved task or review can supply the design; routine choices
within that scope do not need repeated approval. Record material behavior
changes and deviations from retail in the PR.

Inventory and preserve the rules: target selection, costs, rolls, conditional
follow-ups, death, interruption, movement, reflection, and cleanup. Test
observable gameplay outcomes and external contracts. Remove tests that pin
component layouts, change ticks, scheduler stages, temporary markers, or
catalog representation. Do not reproduce the implementation to calculate an
expected result. A representation change with unchanged behavior should not
require rewriting expected outcomes.

Check that removed machinery is actually gone. Review both code size and
conceptual complexity; fewer lines are useful evidence, not a correctness
criterion. Run formatting, Clippy, unit tests, and the retail scenarios:
`RIZZARDRY_RETAIL=... cargo test --lib -- --ignored`.
