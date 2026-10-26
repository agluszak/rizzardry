# Design documentation

Status: active design and implementation, updated 26 October 2026. Implementation
notes describe the integrated code on `main` unless a historical branch is
explicitly identified. Gameplay documents are sourced designs, not claims of
completed mechanics. See known gaps for validation limits and unfinished work.

## Start here

1. [Architecture and ownership](architecture.md): authoritative state, entities,
   resources, values, scheduling and module relationships.
2. [Dependencies](dependencies.md): chosen Bevy/Avian/Landmass/Rerecast stack
   and when additional libraries are warranted.
3. [Bevy 0.20 integration](bevy-features.md): concrete animation, rendering,
   picking, UI viewport, asset and audio choices before writing custom engine code.
4. [Content and assets](content-and-assets.md): retail loading, native content,
   animation, audio, UI and modding boundaries.
5. [Implementation plan](implementation-plan.md): remaining playable outcomes, without completed-task history.
6. [Known gaps](known-gaps.md): unfinished behavior, deliberate departures and validation limits.
7. [Revisions and open questions](revisions.md): corrections to the discussion,
   deliberate native-format policies and remaining evidence gaps.
8. [Testing](testing.md): ordinary rule tests, small headless Bevy scenarios,
   and focused real-game checks.
9. [Retail research notes](research/README.md): file formats and retail
   behaviour the decoders and rules follow, with their sources.

## Gameplay systems

| Document | Worked flows |
| --- | --- |
| [Combat items and magic](systems/combat-items-and-magic.md) | Prepare an area bomb, choose a point, execute, consume it, deliver the effect, apply damage/conditions and finish the action |
| [Inventory, trade and curses](systems/inventory-trade-and-curses.md) | Item ownership and cursor transfers, buy/sell, identification, cursed equipment, Remove Curse and Hex |
| [Story scenes and puzzles](systems/story-scenes-and-puzzles.md) | Aletheides/Savant henchman monastery scene; Black Box and Orbital Tracker at Arnika |
| [Progression and map notes](systems/progression-and-map.md) | Level-up draft/commit and custom automap/minimap annotations |
| [Combat AI and navigation](systems/combat-ai-and-navigation.md) | Perception, encounter/group decisions, party movement, combat scheduling, navigation and continuing magic |
| [Persistence and travel](systems/persistence-and-travel.md) | Save/load, active and dormant levels, departure, revisit and stable references |
| [Party and recruitable NPCs](systems/party-and-rpcs.md) | Recruitment, dismissal, role conversion, region restrictions and death/resurrection |
| [Locks, traps and containers](systems/locks-traps-and-containers.md) | Lock attempts, tools/magic, trap inspection/disarm/firing, opening and generated loot |
| [Crafting and services](systems/crafting-and-services.md) | Ingredient combinations, gadgets, skill practice and NPC custom orders |
| [Time, rest and conditions](systems/time-rest-and-conditions.md) | Clock domains, recovery, timed effects, camping/ambush and revisit catch-up |

## How to read the evidence

Source comparisons use these fixed revisions:

- [Wizardry 8 Decomp, `075cbd0b853c4670e94f6ae4fe48d53b52b5e819`](https://github.com/agluszak/wizardry-8-decomp/tree/075cbd0b853c4670e94f6ae4fe48d53b52b5e819).
- [Whizzardry 8, `9c433ec9725877a963cb43bb1df13d5d084ed873`](https://github.com/agluszak/whizzardry-8/tree/9c433ec9725877a963cb43bb1df13d5d084ed873).

Each system distinguishes observed source behavior, the proposed implementation,
and unresolved details. Recovered names and comments are not automatically
correct: several are contradicted by their function bodies. A modernized function
is not automatically behaviorally equivalent to the decomp either.

Retail databases, level files and dialogue scripts are not present in these
source trees. A code path can establish lifecycle and ordering without proving
the exact content that activates it. The official Wizardry 8 strategy guide
corroborates the named monastery and spaceport examples; it does not establish
their hidden script predicates.

Code snippets are type sketches. System names describe proposed responsibilities;
one row of a walkthrough does not require one Bevy system, plugin or Rust type.
Behavioral checks are suggested implementation acceptance cases, not tests that
have already been written or executed.
