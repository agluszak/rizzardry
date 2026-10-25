# Rizzardry

A from-scratch Rust and Bevy rewrite of Wizardry 8.

The [design documentation](docs/README.md) describes the architecture,
dependency choices, gameplay flows and open questions. Implementation has
started with the first slice of the [implementation plan](docs/implementation-plan.md):
the game loads a retail level directly from your installation, lets you walk
the party around it, and bakes a navmesh that test monsters can follow you on.

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
- `--start x,y,z` and `--yaw <degrees>`: an explicit eye position in metres.
- `--screenshot <file.png>`: capture the view shortly after entering, then exit.
- `--walk <seconds>`: hold "walk forward" after entering.
- `--test-agents <n>`: spawn stand-in monsters that chase the party over the
  navmesh.

- `--nav-debug`: draw the navmesh, paths and avoidance.
- `--overhead <metres>`: view the party from straight above.

Run with `--help` for the full list.

The first launch bakes every level's navmesh with Bevy's asset processor
(several seconds); results are cached in `imported_assets/`.

Controls: WASD or arrow keys to walk, Q/E or Left/Right to turn, hold Shift to
run, hold the right mouse button to look around.

`cargo run --example level_info -- --retail <dir> [level.pvl]` summarises
decoded level geometry; `cargo run --example navmesh_bake -- --retail <dir>
[level.pvl]` bakes navmeshes and reports repairs and connectivity. Both read
`RIZZARDRY_RETAIL` too. Tests that need retail data run when `RIZZARDRY_RETAIL` is set
and skip otherwise.

Behavioral research compares
[Wizardry 8 Decomp](https://github.com/agluszak/wizardry-8-decomp) and
[Whizzardry 8](https://github.com/agluszak/whizzardry-8). Their implementations are
evidence for game behavior, not constraints on the new architecture.
