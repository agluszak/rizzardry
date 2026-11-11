# Agent guidelines

Rizzardry is Wizardry 8's game, rebuilt as a Bevy game. It is not Wizardry 8's
program translated into Rust.

[docs/design.md](docs/design.md) is the design rulebook; read it before
touching `src/game`. [docs/rules/](docs/rules/) holds the game's rules as they
have been worked out, [docs/plan.md](docs/plan.md) the rewrite's slices, and
[docs/survey.md](docs/survey.md) what of the original is not here yet.
[README.md](README.md) says how to run the game. This file is about how to
work here.

## Where the game's rules come from

- Look a rule up before writing it: the decomp (`../wizardry-8-decomp`) first,
  then Whizzardry (`../whizzardry-8`). Never invent a stand-in rule, distance
  or constant. If the references do not answer, say so; do not guess silently.
- Take formulas, constants, tables and what the player can observe. Do not take
  procedures, record layouts, managers, schedulers or the order of unobservable
  work. No function exists because a retail function does.
- The original's order of random draws need not be reproduced. Presentation
  never draws from gameplay's random numbers.
- A comment names a retail routine only beside the formula or constant it is
  the source of.

## Design, not rearrangement

- Start from what the thing is in the game and what happens to it. Then fix the
  data model. Splitting a function into systems, threading a parameter through
  call sites, or renaming types is moving code around, and is not the job.
- Be ambitious. When the honest fix is a big rewrite, do the big rewrite. No
  half-conversions, wrapper systems or `*_in_world` helpers.
- Simplify deeply. When a better model makes something redundant, delete it:
  forks by actor kind, structs that mirror components, forwarding getters,
  gatherers that copy components into "facts" for a rule to read.
- One rule, one piece of code. Never a member version and a monster version of
  a rule; what differs between them is data, written where the actor is made or
  refitted.
- A rule reads the things it is about. Do not put a struct of copied values
  between an actor's components and the rule.
- A struct or `SystemParam` that bundles unrelated queries just to pass them
  around should not exist. A cohesive accessor (who stands where, what is laid
  on whom, what is held) is fine. A narrow `&mut World` interpreter is fine; a
  broad one that reaches into other domains is not.
- Content is read where things are made. A catalog record becomes components
  when its thing is spawned; rules do not look the record up again.
- Data is data. A table is a file under `assets/tables/`, not a `match` and
  not a list of ids in game code. Raw retail encodings (bit flags, magic ids)
  are decoded in `retail`, never tested in `game`.
- Judge a refactor by what it removed: lines, mechanisms, mirrors, places where
  two things can disagree.

## Use Bevy and Rust as they are meant

- Bevy's own types before hand-made ones: timers, quaternions and its math,
  colours as colours, the entity-keyed maps and sets, relationships, observers,
  scenes (BSN) instead of repeated spawn code.
- Enums for choices, never `Option<u8>` or a sentinel. No hand-kept `ALL` lists
  of an enum's variants.
- No type aliases of our own: a newtype, or the full type written out.
- Plain arithmetic. Never `saturating_*`, `checked_*` or `wrapping_*`; clamp
  where the rule clamps and say so.
- No home-made random-number generators. Gameplay and presentation each have
  their own resource.
- No test-only code in the main source.
- Files are split by what they are about, not by how the C++ was laid out.
  `clippy::too_many_arguments` is allowed; systems have many parameters.

## Do the work yourself

- Write the code yourself, and read every diff you commit. A green check and a
  subagent's report are not a review.
- Subagents are for narrow mechanical sweeps whose every edit is already
  decided. Never for surveys, reviews, design, or rebuilding a rule module.
- Fix forward on the current branch. No worktrees, and no building `main` to
  find out whether a failure was already there.
- Report what was actually run and what it showed. If something was skipped or
  is unverified, say that.

## Tests

- A test states a rule of the game by its outcome, pins a number or threshold
  taken from the original, or drives the real plugins through something the
  player would see. Nothing else earns its place.
- Delete, do not maintain: a test per term of a formula; a test that copies a
  table row back out of the table; a test that draws thousands of seeded rolls
  to check a rate; a test that pins a component layout, a schedule, an event
  count or an internal step.
- Spend little time on tests during a refactor. When a refactor removes the
  types a test was written against, delete the test unless it pins a real rule
  cheaply. Fewer tests is not no tests.
- Run the unit suite once per slice, not after every edit. Run the retail
  tests (`RIZZARDRY_RETAIL=<install> cargo nextest run --run-ignored only`)
  before committing a change to gameplay. The heavy tests are limited to two at
  a time in `.config/nextest.toml`; all at once run the machine out of memory.

## Git

- Commit and push only when asked. Check that the checks passed before the
  commit, not in the same command.
- Stage by explicit path. Never `git add -A`, `git add .` or `git commit -a`:
  the working tree holds private notes that must never be committed.
- Never send anything upstream: no issues, pull requests, comments or pushes to
  third-party repositories. Fixes to third-party code stay in `third_party/`.
