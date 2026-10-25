# Dependencies

Research snapshot: 24 October 2026. These are proposed choices, not a tested
`Cargo.lock`. The first implementation spike must compile the selected set on
the native target platforms before the versions become project commitments.

**Compiled and running (25 October 2026)** on Linux (Wayland, Vulkan/Mesa),
with `Cargo.lock` committed. macOS and Windows are untested. `serde`/`ron`,
`binrw` and the inspector are not used yet: the irregular retail records so far
read cleanly with a small bounds-checked cursor.

## Chosen set

| Need | Choice | Status |
| --- | --- | --- |
| Engine | Bevy 0.20.0 (`tga`, `wayland`) | In use |
| Input actions | `bevy_enhanced_input` 0.27 | Exploration context in use |
| Collision and physics | Avian 0.8 | Level trimesh, party and walker bodies |
| Grounded movement | Tnua 0.33 + `bevy-tnua-avian3d` 0.13 | Party and walkers; walk basis only, no actions |
| Navmesh generation | `rerecast` (core), vendored from PR #63 | Baked from the level's collision triangles |
| Pathfinding, steering, avoidance | `bevy_landmass`, PR #195 branch pinned by git rev | Party is a `Character3d`, walkers are `Agent3d` |
| Command line | `clap` 4 | Game and examples |
| Definition/save serialization | `serde` + `ron` | Not yet needed |
| Retail binary parsing | `binrw` | Not yet needed |
| Developer inspection | Optional `bevy-inspector-egui` 0.38 | Not yet needed |

Primary references:

- [Bevy 0.20 release](https://bevy.org/news/bevy-0-20/) (8 October 2026).
- [Enhanced Input compatibility table](https://github.com/simgine/bevy_enhanced_input#bevy-compatibility).
- [Avian](https://github.com/Jondolf/avian), [Tnua](https://github.com/idanarye/bevy-tnua).
- [Rerecast PR #63](https://github.com/janhohenheim/rerecast/pull/63) (Bevy 0.20,
  Avian 0.8; all CI checks passing, still open).
- [Landmass PR #195](https://github.com/andriyDev/landmass/pull/195) (draft, Bevy
  `0.20.0-rc.1`, which Cargo resolves to 0.20.0; `landmass_rerecast` excluded).
- [Inspector package and Bevy 0.20 dependencies](https://docs.rs/crate/bevy-inspector-egui/0.38.0).
- [Serde](https://serde.rs/), [RON](https://docs.rs/ron/0.12.2/ron/),
  [binrw](https://docs.rs/binrw/0.15.2/binrw/).

### Why Avian rather than Rapier

The first spike used `bevy_rapier3d` 0.37, whose kinematic controller worked on
the monastery. Avian 0.8 replaced it for integration, not performance:
Rerecast's Bevy 0.20 branch integrates with Avian, Tnua supports Bevy 0.20 +
Avian 0.8, and together they cover navigation and grounded movement without
our own stair-climbing controller. Only one physics engine is used. Physics
does not decide combat rules: range, area membership, initiative, conditions
and damage remain game rules.

Tnua is a floating controller, so the party and walkers are dynamic bodies
held above the ground by a spring, with rotation locked. Facing is game state,
not body rotation. Avian's `MoveAndSlide` remains an option if Tnua's floating
model causes problems on retail geometry.

## Navigation

The movement loop for a navigating actor:

1. AI (not yet written) chooses a destination or target entity.
2. Landmass computes the desired velocity, including path following and local
   avoidance of other agents and the party.
3. Tnua and Avian move the body under collision.
4. The body's resulting velocity goes back to Landmass on the next step.

Integration choices:

- `bevy_rerecast` and `avian_rerecast` are not used. The navmesh is baked with
  the `rerecast` core pipeline directly from the same triangles as the level
  collider, which are already in hand. `landmass_rerecast` is not ported to
  Bevy 0.20; its 65-line mesh conversion is adapted in
  `game::world::navigation`.
- `rerecast` is vendored in `third_party/` with one fix: when contour
  triangulation finds no diagonal, upstream fails the whole navmesh, while
  recastnavigation keeps the partial triangulation and warns. Without the fix,
  18 of 34 retail levels fail to bake. Report it upstream and drop the copy
  once fixed.
- Navmeshes go through Bevy's asset processor (`AssetMode::Processed`,
  `asset_processor` feature). A virtual `navmesh://` source exposes every
  retail `<level>.pvl` as `<level>.navmesh`. `LoadTransformAndSave` reads its
  collision triangles, bakes them and writes a small binary file to
  `imported_assets/navmesh`. The game loads that file as a Landmass
  `NavMesh3d`. The bake settings are in the asset meta, so changing a default
  re-bakes automatically. The first launch bakes all 34 levels in parallel
  (0.1 to 2 s each); later launches only load. The retail source has a
  processed reader but no writer, so it is never processed or copied.
- The conversion repairs Recast precision artifacts that Landmass rejects:
  duplicate or spike vertices are removed, polygons that stay concave are
  dropped, and flipped detail triangles are rewound
  (`navigation::bake::repair`). All 34 levels validate. Most need no repair;
  the worst, `trynnie2`, drops 49 of about 1600 polygons at the old 0.35 m
  step height and none at 0.5 m.
- Moving actors never trigger a rebake.

`cargo run --example navmesh_bake` bakes outside the game and reports
repairs, connected components and (with `--gaps`) where the largest
components come closest. `--climb`, `--radius` and `--height` override
settings.

### Experiment results (25 October 2026)

- **Step height.** At 0.35 m the upper monastery split into 22 islands, the
  largest covering 61% of the walkable area, and most walkers reported
  `NoPath`. At 0.5 m one island covers 98% and every walker finds a path.
  Raising it to 1 m merges nothing more. `game::world::STEP_HEIGHT` (0.5 m)
  now sets the bake, the party collider clearance and the test walkers.
- **Overlapping floors.** The upper monastery's balconies and floors bake as
  separate layers; walkers path between them.
- **Crowds.** 30 walkers on the upper monastery ran at about 110 fps (dev
  build, debug gizmos on), all in `Moving`, and closed to a median of 2.2 m
  around the party, queueing around each other instead of overlapping.
- **Remaining splits are structural.** Several levels still have large
  separate pieces (e.g. the monastery beach and monastery, 26,600 and 14,300
  m², 2.2 m apart across a 1.3 m drop). Looser climb and height limits do
  not merge them. They need retail data we do not decode yet: doors,
  teleporters, elevators and ladders as Landmass links, and the `.oct`
  collision surfaces with walkable flags. Those surfaces also matter because
  translucent water is currently not solid, so riverbeds under water are
  walkable navmesh.
- **Party on stairs.** A wide, flat-bottomed cylinder (0.83 m radius, 0.5 m
  clearance) caught on stairs rising inside its radius and never left the
  `MO202` entrance. The party now uses retail's sphere (slightly smaller so
  it clears the ground), which climbs those stairs.
- **Not tested.** Doors (no props yet), per-size navmeshes, and flying
  monsters. Landmass avoidance is horizontal, so flying monsters need separate
  handling rather than pretending a surface navmesh is 3D navigation.

A navmesh generator only builds navigation data. It does not supply Wizardry's
group orders, target choice, moving-party pursuit, doors/links, avoidance or
combat movement budget. The [navigation design](systems/combat-ai-and-navigation.md)
separates those decisions. No library choice is justified solely by a flat-floor
pathfinding demo.

## Use native Bevy features first

- Native UI layout, picking, focus, accessibility and headless widgets. The game
  supplies its art and behavior binding. Feathers can help development tools,
  but its editor styling does not define the game's visual identity.
- Rust `bsn!` composition where it simplifies UI/scene construction. The 0.20
  release notes still put the external `.bsn` asset format under "What's Next?";
  do not require hot-reloadable `.bsn` files for the initial UI. The release post
  includes a future-looking file example, so the prose feature status matters.
- Standard asset loading, meshes, materials, textures, cameras, render layers
  and render-to-texture. No replacement C++-shaped scene graph.
- Built-in audio initially, with the codecs needed by actual assets enabled.
  Verify voice completion, pause/resume, spatial attenuation and music behavior
  before adding another audio plugin.
- Native diagnostics and tracing rather than a second logging system.

## Add only for a demonstrated gap

| Candidate | Gate |
| --- | --- |
| Kira/Bevy Kira integration | Built-in audio cannot express a required verified playback/mixing behavior economically |
| FFmpeg integration | Actual Bink/cinematic playback needs it; keep decoder/version/platform packaging at this boundary |
| A deterministic RNG crate | Pin a named algorithm and serializable state for native saves; do not depend on an unspecified default RNG |
| Bitmap font helper | Needed to render the retail fonts faithfully; inspect the decoded format and current Bevy support first |
| Particle plugin | A real spell effect would otherwise require substantial custom GPU machinery |
| Scripting runtime such as Rhai/Lua | Several real mods require behavior beyond data definitions and explicit native handlers |
| Additional collections (`enum-map`, small vectors, arenas) | A concrete type/measurement makes them simpler than standard collections |

The RNG algorithm and serializable state are required from the first gameplay
and save implementation. Only the choice to use an external RNG crate is optional.

No database server, event-sourcing framework, generalized ability framework,
behavior-tree framework, extra ECS or network stack is needed for the initial
single-player game. Existing libraries should remove implementation work while
leaving game rules readable.

## Build implications

Use ordinary Cargo tooling and commit the lockfile when the implementation
exists. Keep developer inspector/dynamic-linking features optional. Avoid a
permanent code-generation pipeline for UI or gameplay that hand-written native
Bevy composition already expresses. Retail conversion should be a reproducible
asset operation, not a second way to build the executable.
