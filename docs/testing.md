# Testing

Write only meaningful tests. Protect real regressions, important gameplay rules
and ownership/lifecycle boundaries, not every helper or implementation detail.
Not everything needs rigorous coverage. Prefer a few convincing checks to
exhaustive permutations, snapshot churn or large fixture frameworks. Never add a
second gameplay model or distort production architecture merely for testing.

## Choose evidence proportional to the change

- Ordinary Rust tests for rules, decoders and calculations on values.
- Small headless Bevy Apps for scheduling, input routing or entity lifetimes.
  Install the real systems/plugin when ordering matters; a test-only chain can
  hide the bug. Drive the relevant public request/input boundary.
- Real-game checks for rendering, movement feel, audio and platform input.
  Headless assertions cannot establish visual parity, clipboard or OS IME behavior.
- Focused checks during development; the broader suite for integrated changes.
  Documentation-only edits need link/content checks, not another engine build.

## Commands

```sh
cargo fmt --package rizzardry --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
RIZZARDRY_RETAIL=/path/to/wizardry8 cargo test --locked -- --ignored
RIZZARDRY_RETAIL=/path/to/wizardry8 cargo test --locked --test integration -- --ignored scenarios::
```

Retail tests are ignored by default and must fail if explicitly invoked without
valid assets. Retail scenarios share the navmesh cache and run serially. See
[the README](../README.md) for headless runs and offscreen screenshots; neither
all-map baking nor GPU screenshots are required for every PR.

## Avoid misleading results

- Advance the App normally for movement: Landmass, Tnua and Avian span
  FixedPreUpdate, FixedUpdate and FixedPostUpdate. Running only FixedUpdate is
  not a simulation step. Control time instead of sleeping; account for the first
  clock-initialization update, timestep and pause/time scaling.
- Seed gameplay RNG and define actor order. Assert discrete rules exactly, but
  physics/navigation by collisions, reachable regions and bounded progress.
- Use small Assets fixtures; bound waits for asynchronous readiness. Do not
  assume loading finishes after an arbitrary frame count.
- Keep repeated-frame systems scheduled: run_system_once resets their local
  state and readers. Add ordering only for actual dependencies.
- Verify the tested source tree, especially with shared targets across worktrees.
  Logs and screenshots are evidence to inspect, not automatic success. Report
  skipped checks and limitations; never reuse an earlier result as a new run.
