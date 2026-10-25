# Testing

Tests should establish meaningful game behavior without shaping the architecture
around tests. Prefer a few checks at ownership, scheduling and gameplay boundaries
to elaborate test scaffolding or implementation snapshots.

## Pick the right scope

- **Ordinary `#[test]`:** rules and computations that already work on Rust
  values, format decoders, asset conversion and explicit invariants. Do not move
  ECS state into a second game model just to write unit tests.
- **Headless Bevy scenarios:** behavior spanning systems or entity lifetimes.
  Construct an `App` with `MinimalPlugins`, install the relevant real systems or
  plugins, arrange a small world, call `app.update()`, and assert ECS state.
  Use the production plugin registration where its scheduling matters; do not
  replace it with a test-only `.chain()` that masks ordering bugs.
- **Real-game checks:** rendering, movement feel, animation timing, input
  integration, and retail geometry that tiny synthetic worlds cannot represent.
  Exercise a representative scene and check the actual result.
- **Headless game runs:** gameplay on retail content (fights, loot, travel)
  without a window, rendering, sound or UI. `--headless` runs the real game
  plugins that way and logs what happens; do not open a window to check a
  rule.

No custom harness, fixture DSL, mock engine, second gameplay model or separate
testing crates unless a concrete problem requires one. Keep fixtures local to
the tests that use them.

## Headless Bevy details

- Use the same input/request boundary as the game. Gameplay rule tests need not
  simulate a mouse, while actual control-binding tests should exercise input.
- For full movement scenarios, advance the app normally: Landmass runs in
  `FixedPreUpdate`, Tnua controls in `FixedUpdate`, and Avian plus velocity
  feedback in `FixedPostUpdate`. Calling `run_schedule(FixedUpdate)` alone is
  not a simulation step.
- Control time instead of sleeping. After the first clock-initialization update,
  `TimeUpdateStrategy::FixedTimesteps(1)` advances one fixed tick per update
  at normal time scale. Use the actual simulation timestep and account for
  pause/time scaling. Test long-running sequences by advancing a bounded number
  of updates.
- Seed gameplay RNG, keep its target order defined, and test exact results for
  discrete rules. For physics/navigation, assert reachable regions, collisions
  and bounded progress rather than exact positions or waypoints every tick.
- Put tiny, immutable test assets directly in `Assets<T>` where appropriate.
  Test asynchronous loading separately by waiting for readiness with a bound,
  rather than assuming an asset finishes after a fixed number of frames.
- If ordering becomes suspect, use Bevy's schedule ambiguity diagnostics or
  seeded schedule shuffling. Add explicit ordering only when the rules require
  it, not to silence all warnings.
- Avoid `run_system_once` for repeated-frame behavior: it resets system-local
  state and reader cursors. A scheduled system in an `App` retains them.

## Retail and graphical verification

Retail assets remain external. Tests requiring `RIZZARDRY_RETAIL` are
`#[ignore]` by default and **must fail rather than silently pass** if invoked
without a valid installation:

```sh
cargo test --locked
RIZZARDRY_RETAIL=/path/to/wizardry8 cargo test --locked -- --ignored
```

The real-game `--walk`, `--test-agents`, `--nav-debug`, `--screenshot` and
`navmesh_bake` tools are useful diagnostics. Logging positions, writing a
screenshot or printing "INVALID" does not by itself pass a test. A check used
as a gate must assert an outcome and exit unsuccessfully on failure.

### Headless runs

`--headless` builds the game without a window or event loop, without audio,
and with a renderer that has no GPU backend (Bevy's headless setup: meshes and
images still load as assets, nothing is drawn). Presentation is not installed;
gameplay news (combat, items, search, identification, camping, level
activation) goes to the log. With it:

- `--continuous` fights without waiting for the player between rounds;
- `--speed N` runs virtual time N times faster (the fixed step, the fight's
  pace and the game clock follow it);
- `--run-for SECONDS` exits after that many real seconds;
- `--seed N` gives another reproducible run of gameplay's random numbers.

```sh
cargo run -- --headless --continuous --speed 8 --run-for 10 --seed 4 \
  --start=69.5,10,-24 --yaw 0     # the Monastery's Green Slimes
```

`--ui-script` runs headless too, for its steps that need no window (`wait`,
`travel`, `hurt`, `tire`, `use`, `log-party`, map assertions); key, typing,
pointer and cursor steps are skipped with a warning. Screenshots need the
window and are refused with `--headless`.

```sh
cargo run -- --headless --speed 4 --run-for 15 --start=138.2,2.5,-214 \
  --ui-script "wait 2;travel MO204;wait 12;travel MO102;wait 12"
```

Do not require GPU screenshots or all-map baking on every PR. Run focused
retail/visual checks when the changed feature warrants them.

## High-value scenarios

The first slice should prove that an invalid collider never activates the
level, a second ready level does not duplicate the party, and a tiny traversable
scene works through Rerecast, Landmass, Tnua and Avian together. Supplement
these with real retail stairs, doors and level links when modifying movement.

As gameplay grows, cover item conservation across owners, accepted-cost versus
cancelled actions, death bookkeeping before the next combat action, interrupted
interactions, save/load identity and cross-level lifetimes. In particular,
exercise failed/delayed level activation with a previously active party,
a container trigger with required catalogs unavailable, map-local notes and
killed monsters across travel, the last combat round's recovery policy, and
party effects when an off-roster Character exists. Derive expected
behavior from the original game when faithfulness matters. Do not lock tests
to an incidental internal representation.
