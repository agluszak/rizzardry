# Rizzardry

A from-scratch Rust and Bevy rewrite of Wizardry 8.

The [design documentation](docs/README.md) describes the architecture,
dependency choices, gameplay flows and open questions. Implementation is on
the first slice of the [implementation plan](docs/implementation-plan.md): the
game loads a retail level directly from your installation with its props,
doors, sky, fog and time of day; the party walks it, uses doors and
teleporters, and is chased over baked navmeshes by animated retail monsters;
a voice line plays; and the automap takes notes.

## Running

You need your own retail copy of Wizardry 8 (e.g. the GOG release). Assets are
read in place from its SLF archives; nothing is converted or copied.

```sh
RIZZARDRY_RETAIL=/path/to/wizardry8 cargo run
```

Options:

- `--retail <dir>`: the installation, instead of `RIZZARDRY_RETAIL`.
- `--level <path>`: another level, e.g. `levels/arnika/arnika2.pvl`.
- `--entrance <name>`: an entrance trigger, e.g. `MO101`. The default level
  starts at the new-game entrance `MO100`.
- `--start x,y,z`: an explicit eye position in metres; `--yaw` and `--pitch`
  (degrees) override the facing.
- `--hour <0-24>`: start at this time of day instead of noon.

Developer options:

- `--screenshot <file.png>` (with `--screenshot-after <s>`): capture the view
  after entering, then exit.
- `--walk <s>`, `--interact-at <s>`, `--face-prop <name>`: scripted movement
  and prop use.
- `--ui-script "key m; click X Y; type TEXT; key Enter; wait S"`: drive the UI
  through real input messages; clicks use the 640×480 screen layout.
- `--say <line>`: speak a retail voice line, e.g. `npc_aletheides_000`.
- `--test-agents <n>`, `--test-agent-radius <m>`, `--test-monster <species>`:
  stand-in monsters that chase the party, optionally dressed as a retail
  species such as `savant_trooper`.
- `--nav-debug`: draw the navmeshes, paths and avoidance.
- `--overhead <metres>`: view the party from straight above.

Run with `--help` for the full list.

The first launch bakes every level's navmeshes, one per monster size class,
with Bevy's asset processor (several seconds); results are cached in
`imported_assets/`.

Controls: WASD or arrow keys to walk, Q/E or Left/Right to turn, hold Shift to
run, hold the right mouse button to look around, Space or left click to use
the prop in front of you, M to open the automap (Escape or M closes it).

`cargo run --example level_info -- [level.pvl] [--props] [--links]`
summarises decoded level geometry, props and links; `cargo run --example
navmesh_bake -- [level.pvl]` bakes navmeshes and reports repairs and
connectivity. Both read `RIZZARDRY_RETAIL`. Retail-dependent tests are ignored
by default; run them explicitly against your installation:

```sh
RIZZARDRY_RETAIL=/path/to/wizardry8 cargo test --locked -- --ignored
```

See [testing](docs/testing.md) for the lightweight Bevy testing approach.

Behavioral research compares
[Wizardry 8 Decomp](https://github.com/agluszak/wizardry-8-decomp) and
[Whizzardry 8](https://github.com/agluszak/whizzardry-8). Their implementations are
evidence for game behavior, not constraints on the new architecture.
