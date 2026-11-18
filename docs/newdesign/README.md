# Proposed gameplay architecture

These are design proposals from a source review. The [design rules](../design.md)
govern implementation, and the [game rules](../rules/) govern observable behavior.
The Rust review identifies architectural risks. Retail behavior requires
independent evidence. The [retail comparison](retail-evidence.md) records the
verified paths and the limits of the review.

## Working principles

Give an entity to something with an identity or lifetime of its own. Put a fact
on the actor, item, group, or level that owns it. Use relationships when gameplay
needs to follow an association in both directions. Give members and monsters
the same capabilities where the same rule applies. Decode content when making
or refitting the entity.

Use queries to read current state and ordinary calls to resolve one indivisible
action. Use events when another domain cares that something happened. Keep
scheduled state when the game can advance before the work finishes. Input asks,
gameplay decides, and presentation displays the outcome using its own randomness.
Test consequences a player could observe.

## Rewrite scope

Choose a complete gameplay responsibility and replace its model, writers and
readers together. Remove superseded mechanisms in the same rewrite. Existing
unit tests carry little architectural weight. Delete tests tied to obsolete
types or internal steps. Retain cheap checks of meaningful game outcomes.

## Priority

- Start with population because simultaneous arrivals can collide. Character
  advancement is the clearest independent rewrite and removes draft work from
  actor settlement.
- Rewrite inventory before changing ammunition handling. Replace the derived hand model, turn
  interruption chain and melee swing pipeline together. Spell targeting can proceed
  independently.
- Hostility affects targeting, AI and dialogue. Give its readers explicit
  queries for underlying and effective allegiance. Rewrite actor settlement
  around the owners of the remaining derived state.
- Replace eager story evaluation with ordered execution. Dialogue endpoints can
  be rewritten independently. Establish party activity admission rules before
  replacing the occupancy checks in dialogue, travel and advancement.

[Gameplay proposals](gameplay.md) · [Inventory and integration](inventory.md)

[Code architecture for gameplay rewrites](code-architecture.md)

[Magic, casting lifetimes and derived combat figures](magic.md)
