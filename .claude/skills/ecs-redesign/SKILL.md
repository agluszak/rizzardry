---
name: ecs-redesign
description: Redesign C++-shaped Rust (routines ported from retail, context bags, giant interpreters, member/monster forks) as Bevy entities, components and small systems. Use before refactoring any gameplay subsystem whose code passes bundles of queries around, switches on a "kind" inside one big function, or mirrors retail call structure.
---

# Redesigning C++-shaped code as ECS

Retail Wizardry 8 is a C++ program of routines that take a world pointer and
do everything an action needs. Porting it routine by routine produces Rust
with the same shape: context structs, interpreters taking a dozen queries,
`match kind` switches, and helper functions threading the same five queries
everywhere. Moving those routines into systems, or slicing one routine into
stages joined by carrier components, does not fix the shape. Only changing
the primitives does.

**Do not start from the existing code.** Start from the game: what exists,
what happens, what follows from what. Read the existing code only to learn
the rules it encodes and the cases it covers.

## 1. Inventory the rules, not the routines

Before designing, list every case the current code handles, with retail
evidence (decomp, AGENTS.md references). For each, note:

- **What happens** in game terms ("a missile flies to each victim", "a
  victim who saves against Insanity is frightened anyway").
- **Multiplicity.** Can there be several at once? (A blow can trigger many
  conditions and drains; a spell can do damage then two conditions.) A fact
  that can occur more than once on one thing cannot be a component on that
  thing: it is an entity of its own.
- **Order and conditionality.** Does it happen after something else, only
  if that landed, only if it was saved against, until the victim dies?
- **Lifetime.** Current fact, draft intent, accepted work, in-flight work,
  historical record, derived value (see AGENTS.md "Design").

## 2. Find the entities

Things that exist and things that happen are entities:

- Actors and objects (bodies, the fight, the party).
- **Acts**: anything that happens over time or produces a record: a cast,
  an attack, a breath, a cloud's pulse, a sprung trap.
- **Parts of acts**: one entity per thing the act does to one target (an
  effect on a victim, a swing). If an act can do several things to the same
  target, each is an entity.

Ask of every struct in the old code: *is this an entity with a lifetime,
or just locals of a routine?* `Prepared`, `Rolled`, `Situation` and
`Field` were locals; making them components or parameters changes nothing.

## 3. Components are facts; relationships are links

- A component is one fact with one meaning and one writer. Name it by
  meaning (`InFlight(Timer)`, `Afflict { condition, rounds }`), never by
  stage (`Prepared`, `Rolled`, `Landing`, `Finale`).
- Links between entities are relationships (`By(actor)`, `Strikes(body)`,
  `HitOf(act)`, `Follows(previous)`), so Bevy keeps the inverse and cleans
  up after despawns. Never store an `Entity` field when it is really a link.
- Order between parts is a relationship too: an effect that `Follows` an
  earlier one waits until that one is gone. Conditional follow-ups are a
  marker on the follower (`OnlyIfSaved`) that the predecessor's system
  honours.
- Do not mirror state. Where something is, is where its `Position` or
  `Transform` says, or where whatever it belongs to is, through the existing
  relationship (a member stands where its party does, via `PartyMember`);
  never a copied position.

## 4. Systems are single steps

A system does one step to every entity in one state:

- Its query is the state it handles (`With<Afflict>, Without<InFlight>,
  Without<Follows>`).
- It changes components on those entities, spawns or despawns entities,
  and writes notifications. Nothing more.
- If a system needs a `match` over kinds, the kinds should be components,
  each with its own system.
- If a system body is more than a screen, or takes queries it uses for
  only one branch, split it along the branches.
- Rules (formulas, saves, resistance) are plain functions over plain
  values or one query item, shared by the systems that need them.

Work that must finish within one simulation step (an attack's swings, a
spell's effects and the reflections they set off) runs as a schedule of
small systems that an exclusive system repeats until nothing is pending
(`run_swings`, `run_hits`). Do not spread it over frames.

## 5. Smells that mean "still C++"

- A struct of queries/resources passed into functions (`Field`,
  `MagicContext`, `BattleMagic`, `Situation`). Even when it is a
  `SystemParam`.
- A function taking a tuple of five or more queries or catalogs, or helper
  functions that all take the same queries.
- `match kind { ... }` inside a system or helper that applies effects.
- Member-versus-monster forks for the same fact (position, side, level).
- Components named after pipeline stages, or holding a copy of another
  entity's data.
- Worklists and loops inside a system that re-implement scheduling.
- A system named after a retail routine.

## 6. Deliver a design before code

Write the design down (in chat, then in the PR description) and get it
approved before writing code:

1. Entities, with the components each carries and what each component
   means, who writes it, and when it goes away.
2. Systems, each with the query it runs over and its one step, in order.
3. Every case from the inventory, walked through the design.
4. What is deleted (names that must be gone, checkable with grep).
5. Recorded deviations from retail.

During implementation, new types not in the design mean the design was
incomplete: stop and revise it with the user rather than improvising.

## 7. Verify

- `rg` for the deleted names: none left.
- `git diff --stat` against the starting commit: the touched area should
  shrink.
- Clippy, unit tests and retail tests
  (`RIZZARDRY_RETAIL=... cargo test --lib -- --ignored`).
- Re-read the smells list against the new code.
