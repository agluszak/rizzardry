# Implementation priorities

Build complete playable outcomes, not infrastructure for every proposed system.
[Known gaps](known-gaps.md) tracks what is missing; [architecture](architecture.md)
sets ownership boundaries. Detailed system documents are designs and evidence,
not claims of implementation or mandatory test matrices.

1. **Finish existing player interactions.** Complete inventory actions and the
   container picker, character review/level-up, and missing HUD actions. Keep
   controls native where possible and preserve retail-specific presentation.
2. **Complete a combat/magic loop.** Add missing orders and targeting, accepted
   costs, spell delivery, conditions and recovery as one usable flow. Keep
   mandatory outcomes synchronous; explicit execution state handles real waits.
   Expand monster decisions and round-based movement rather than adding detached
   AI plumbing. Verify a representative retail encounter.
3. **Complete persistent world interaction.** Locks/traps, searchable triggers,
   crafting/services, dialogue and authored sequences need evidence-driven rules
   plus usable interaction. Do not generalize every exception into a script engine.
4. **Prove campaign continuity.** Disk saves/load, stable identity, RPC role changes
   and dormant-level elapsed-time catch-up must preserve ownership and accepted
   work. Session records already exist; do not replace live ECS with a parallel
   persistence model. Implement settled-state saves before arbitrary mid-action saves.
5. **Close content and presentation gaps.** Lifts, missing trigger actions,
   particles/audio/skins, lighting and remaining frontend options. Compare actual
   outcomes to retail; keep uncertain behavior explicitly unresolved.

For each change: establish the expected behavior, identify its owner, implement
its shortest coherent path, delete superseded machinery, and collect focused
[verification](testing.md). A new crate, plugin, event or abstraction is not an
outcome by itself. Update the gap entry when the behavior actually works, not
when scaffolding compiles.
