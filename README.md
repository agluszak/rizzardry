# Rizzardry

A from-scratch Rust and Bevy rewrite of Wizardry 8.

The game loads a retail level directly from your installation with its props,
doors, sky, fog and time of day; the party walks it, uses doors and
teleporters, and is chased over baked navmeshes by animated retail monsters;
a voice line plays; and the automap takes notes.

## Running

You need your own retail copy of Wizardry 8 (e.g. the GOG release). Assets are
read in place from its SLF archives; nothing is converted or copied.

```sh
RIZZARDRY_RETAIL=/path/to/wizardry8 cargo run
```

Interactive play opens the main menu. Choose **New Adventure** to assemble a
party and create characters before entering the monastery.

Options:

- `--retail <dir>`: the installation, instead of `RIZZARDRY_RETAIL`.
- `--level <path>`: another level, e.g. `levels/arnika/arnika2.pvl`.
- `--entrance <name>`: an entrance trigger, e.g. `MO101`. The default level
  starts at the new-game entrance `MO100`.
- `--start x,y,z`: an explicit eye position in metres; `--yaw` and `--pitch`
  (degrees) override the facing.
- `--hour <0-24>`: start at this time of day instead of noon.
- `--party <file.ron>`: skip the main menu and start with this party. Headless
  runs use `assets/party.ron` unless overridden.
- `--continuous`: fight continuously, as retail's setting does.

Developer options:

- `--headless`: no window or sound; updates run as fast as the machine
  allows, with what happens written to the log. `--run-for <s>` stops after
  that much play, `--seed <n>` fixes gameplay's random numbers.
- `--screenshot <file.png>` (with `--screenshot-after <s>`): capture the view
  after entering, then exit; with `--headless`, rendered offscreen.
- `--say <line>`: speak a retail voice line, e.g. `npc_aletheides_000`.
- `--nav-debug`: draw the navmeshes, paths and avoidance.
- `--overhead <metres>`: view the party from straight above.

Run with `--help` for the full list.

Entering a level the first time bakes its navmeshes, one per monster size
class (up to a few seconds); results are cached in `imported_assets/`.
`cargo run --release --example navmesh_prebake` bakes every level ahead of play.

Controls: arrow keys to walk and turn, hold Shift to run, Q to quick-save and
Shift+Q to quick-load (`saves/quick.ron`; `--load <file>` starts from a save), hold the right mouse button to look around, Space or left click to use
the prop in front of you, Tab to open the automap (Escape closes it; Tab navigates its controls).
On the map, use the magnifying glasses to zoom, arrows to pan, and the centre
button to recenter on the party. Keyboard arrows also pan; clicking empty map
space zooms in and right-clicking zooms out. Use the quill buttons to add or
erase notes; Enter accepts typing and Escape cancels it. The layer buttons above the pan
controls switch floors; note text and picking follow the selected layer.
F6 enters HUD keyboard navigation; Tab/Shift+Tab navigate and Escape returns
focus to the world.

`cargo run --example level_info -- [level.pvl] [--props] [--links]`
summarises decoded level geometry, props and links; `cargo run --example
navmesh_bake -- [level.pvl]` bakes navmeshes and reports repairs and
connectivity. Both read `RIZZARDRY_RETAIL`. Retail-dependent tests are ignored
by default; run them explicitly against your installation:

```sh
RIZZARDRY_RETAIL=/path/to/wizardry8 cargo nextest run --locked --run-ignored only
```

Behavioral research compares
[Wizardry 8 Decomp](https://github.com/agluszak/wizardry-8-decomp) and
[Whizzardry 8](https://github.com/agluszak/whizzardry-8). Their implementations are
evidence for game behavior, not constraints on the new architecture.
