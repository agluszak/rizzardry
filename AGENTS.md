# Working on Rizzardry

Rebuild Wizardry 8's observable gameplay in Bevy. Read [the design rules](docs/design.md)
before changing `src/game`. The [rules notes](docs/rules/) document established
mechanics. Verify formulas, constants and behavior in `../wizardry-8-decomp`
first, then `../whizzardry-8`. Record uncertainty when neither answers.
Choose the Rust representation according to what the game needs.

## Model the game

- Start with what exists, who owns it, and when it comes into being or ends.
  Give something an entity when it needs an identity of its own. Put state
  that lives and dies with one owner on that owner's entity. Reserve resources
  for state shared by the game as a whole.
- For an ECS design question, inspect the closest engine subsystem in the
  Bevy version pinned here. Follow who may write the data, when readers see
  it, and what happens on replacement or removal. Use the decomp to decide
  which game outcomes that design must preserve.

- Give each fact one authoritative representation. Use Bevy relationships for
  links such as membership or ownership. Let the relationship maintain its
  reverse lookup. Derive values where they are read, or update them when their
  inputs change.
- Model shared capabilities as components. A rule for actors who can take
  damage should query the components needed to take damage. Members and
  monsters can then use the same rule with different component values.
- Decode retail data at the content boundary. Apply catalog data when spawning
  or refitting an entity so gameplay reads the entity's current components.
  Keep rule tables in `assets/tables/`.

## Write systems around changes in the game

- Identify what causes a change and which gameplay domain owns the decision.
  Input expresses intent, gameplay validates it and changes state, and
  presentation shows the outcome. Keep gameplay usable headlessly and give
  presentation its own randomness.
- Query the components a rule reads or writes. Group queries when they answer
  one coherent question, such as who holds an item. Pass components directly
  to rule functions.
- Use events for occurrences and components for state that later readers need.
  React to the change that matters. Order systems explicitly when one needs
  another's result.
- Carry an ownership change through every caller. Remove superseded state,
  actor-specific branches and forwarding systems in the same refactor.

## Verify and deliver

- Test rules by their outcomes, including meaningful retail thresholds and
  player-visible behavior through the real plugins. Delete tests that only
  pin the old implementation.
- Use Clippy while developing. CI handles formatting, Clippy and unit tests.
  Run the local retail tests after completing a gameplay slice. Report what
  you ran and what remains unverified.
- Work on the current branch and fix failures there. Do the design and review
  yourself. Delegate only fully specified mechanical edits. Read every diff
  you commit.
- Commit and push only when asked. Keep private notes out of Git. Keep
  third-party fixes in `third_party/` and all activity off upstream repositories.
