# Dependencies

Implementation status: 25 October 2026. Bevy 0.20, Avian 0.8,
Tnua, Landmass and the vendored Rerecast core are compiled and exercised on
Linux (Wayland, Vulkan/Mesa). The [`first-slice` branch](https://github.com/agluszak/rizzardry/tree/first-slice)
is ahead of `main` and additionally contains retail `.oct` collision,
animated props, doors/links, teleports, fog, lighting and skies. Those code
changes are not merged by this documentation update. macOS and Windows are
not verified; `serde` is present but native saves/definitions are not built.

## Chosen set

| Need | Choice | Status |
| --- | --- | --- |
| Engine | Bevy 0.20.0 (`mp3`, `tga`, `wayland`) | In use; UI, picking and audio native |
| Input actions | `bevy_enhanced_input` 0.27 | Exploration and screen contexts |
| Collision and physics | Avian 0.8 | Level and prop trimeshes, party and walker bodies |
| Grounded movement | Tnua 0.33 + `bevy-tnua-avian3d` 0.13 | Party and walkers; walk basis only, no actions |
| Navmesh generation | `rerecast` (core), vendored from PR #63 | Baked from the level's collision triangles |
| Pathfinding, steering, avoidance | `bevy_landmass`, PR #195 branch pinned by git rev | Party is a `Character3d`, walkers are `Agent3d` |
| Command line | `clap` 4 | Game and examples |
| Definition/save serialization | `serde` + `ron` | Not yet needed |
| Retail binary parsing | Small bounds-checked cursor; optional `binrw` | Existing cursor is working; no library needed without a reduction in code |
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
- `rerecast` is vendored in `third_party/` with two fixes restoring
  recastnavigation behaviour: region holes are merged into their outlines
  (upstream omits Recast's `mergeRegionHoles`, so obstacles inside a region
  were either a failed bake or covered by walkable polygons), and a failed
  contour triangulation keeps its partial result instead of failing the
  bake. See `third_party/README.md`.
- Navmeshes are baked on request, not by Bevy's asset processor. A virtual
  `navmesh://` source exposes every retail `<level>.pvl` as
  `<level>_class<n>.navmesh`, one per walker size class (agent radius 0.6 and
  1.0 m; a walker uses the smallest class that fits it, wider ones the 1 m
  class, and the party joins every class's archipelago). The navmesh loader
  decodes its collision triangles, bakes them off the main thread (0.1 to 2 s
  each) and caches a small binary file under `imported_assets/navmesh`, keyed
  by the input, the bake settings and the format version. Only levels entered
  are baked; later entries only load.
- The bake input is the level's `.oct` collision mesh (the same one the
  Avian collider uses), compacted to referenced vertices, plus collidable
  props in their rest pose as obstacles. Walkable area comes from the
  surfaces' designer-marked pathfinding flag, not a slope test.
- Teleport and ladder triggers are Landmass animation links that walkers
  cross by teleporting; a door is a link only while it is open, crossed by
  walking.
- The conversion fixes only numerical artifacts Landmass rejects:
  duplicate or spike vertices are removed, polygons with no area in the
  walking plane are dropped, flipped detail triangles are rewound
  (`navigation::bake::repair`). Inverted polygons are dropped and counted
  with their area (one in the swamp's class 1); a concave polygon with area
  fails the bake. All 136 bakes validate.
- Moving actors never trigger a rebake.

`cargo run --example navmesh_bake` bakes outside the game and reports
repairs, connected components and (with `--gaps`) where the largest
components come closest. `--climb`, `--radius` and `--height` override
settings.

### Experiment results (25 October 2026)

- **Step height.** At 0.35 m the upper monastery split into 22 islands, the
  largest covering 61% of the walkable area, and most walkers reported
  `NoPath`. At 0.5 m one island covers 98% and every walker finds a path.
  Raising it to 1 m merges nothing more. `game::world::STEP_HEIGHT` is now
  retail's 300 units (0.6 m) and sets the bake and the test walkers.
- **Overlapping floors.** The upper monastery's balconies and floors bake as
  separate layers; walkers path between them.
- **Crowds.** 30 walkers on the upper monastery ran at about 110 fps (dev
  build, debug gizmos on), all in `Moving`, and closed to a median of 2.2 m
  around the party, queueing around each other instead of overlapping.
- **Retail collision surfaces.** Baking from the `.oct` collision instead of
  opaque render polygons joined most splits: the lower monastery's largest
  component went from 58% to 85% of the walkable area, the upper monastery's
  to 99.6%, `marten1`'s from 38% to 94%. It also makes riverbeds under
  translucent water walkable, as in retail, and adds the invisible walls.
  Remaining splits are mostly doors, drops and links (teleporters,
  elevators, ladders) still to be modelled.
- **Party on stairs.** A wide, flat-bottomed cylinder (0.83 m radius, 0.5 m
  clearance) caught on stairs rising inside its radius and never left the
  `MO202` entrance. The party now uses retail's sphere (slightly smaller so
  it clears the ground), which climbs those stairs.
- **Doors.** With a door closed, walkers behind it report `NoPath`; opened,
  5 of 6 reach the party through it.
- **Size classes.** Walkers of radius 0.35 m closed to a median 1.4 m from
  the party; of 1.4 m, to 2.8 m (body contact). All stayed `Moving`.
- **Flying.** Nothing in retail truly flies: flying monsters are snapped to
  the ground, their altitude is a mesh offset plus a cosmetic bob, and they
  path like walkers. Surface navmeshes suffice.

A navmesh generator only builds navigation data. It does not supply Wizardry's
group orders, target choice, moving-party pursuit, doors/links, avoidance or
combat movement budget. The [navigation design](systems/combat-ai-and-navigation.md)
separates those decisions. No library choice is justified solely by a flat-floor
pathfinding demo.

Retail fonts are STCI sprite sheets; `retail::stci` draws text from them into
images for Bevy UI, which needs no helper crate.

## Use Bevy 0.20 facilities before adding libraries

The detailed decisions are in [Bevy 0.20 integration](bevy-features.md).
Use the engine's existing features selectively; do not rewrite working code
just to fit a fashionable API.

| Feature | Intended use and gate |
| --- | --- |
| `AnimationClip`, `AnimationPlayer`, events | Visual rigid-part animation and identified milestones; **do not desynchronize fixed-step colliders** |
| `MorphWeights` | Not used: monsters animate their own mesh copies from decoded vertex frames on the CPU (median about 200 vertices); revisit only if profiling demands it |
| `DistanceFog`, `RenderLayers`, `StandardMaterial` | Already used on the implementation branch for retail environment and sky |
| `ExtendedMaterial`, WESL | Add only for verified visual fidelity or less code than CPU mesh relighting |
| Picking + Avian `SpatialQuery` | Keep one authoritative target per world interaction; native UI picking/focus for widgets |
| Rust `bsn!`, headless UI widgets | Native inventory/dialogue UI; external `.bsn` files are not required in 0.20 |
| `ViewportNode` | Candidate for the game HUD's framed 3D view: resizable UI surface with 3D pointer forwarding |
| `Camera.viewport`, render-to-texture | Use the simpler direct camera viewport when adequate; offscreen view for UI-integrated viewport/previews |
| Image asset processing and `CompressedImageSaver` | Retail textures are shared `RetailTexture` assets; consider replacing the CPU mipmapper only if it removes code |
| `AudioPlayer`, `SpatialAudioSink` | Built-in audio with Bevy's `mp3` feature; voice-line completion is identified per line |
| `States`, `DespawnOnExit` | Scope UI screens or modes, not persisted character or campaign state |

Retail scripts name voice lines as WAV, but the installation ships MP3, so
the project enables Bevy's `mp3` feature. External
`bevy_rerecast`, `avian_rerecast`, `landmass_rerecast`, a new animation
framework or an audio mixer are not needed merely because they exist.

## Add only for a demonstrated gap

| Candidate | Gate |
| --- | --- |
| Kira/Bevy Kira integration | Built-in audio cannot express a required verified playback/mixing behavior economically |
| FFmpeg integration | Actual Bink/cinematic playback needs it; keep decoder/version/platform packaging at this boundary |
| A deterministic RNG crate | Pin a named algorithm and serializable state for native saves; do not depend on an unspecified default RNG |
| Particle plugin | A real spell effect would otherwise require substantial custom GPU machinery |
| Scripting runtime such as Rhai/Lua | Several real mods require behavior beyond data definitions and explicit native handlers |
| Additional collections (`enum-map`, small vectors, arenas) | A concrete type/measurement makes them simpler than standard collections |

**Binary parsing (`binrw`), evaluated and not adopted.** A prototype read
`monsters.dbs` with `binrw` 0.15 and produced exactly the same 595 species as
`retail::species`, but took 126 non-blank lines against 80. It needed raw
mirror structs (`RawSpecies`, `RawAttack`, `RawDice`) and conversions into
the content types, replaced the research tables' absolute offsets with
relative `pad_before` arithmetic, and added three crates. Deriving on the
content types instead would put retail layout into `content`. The retail
layer keeps its small `Bytes` cursor and decodes straight into content
types. `items.dbs` was not prototyped: it reads fields in order and maps
into a 40-field definition, so a mirror struct would cost more still.
Reconsider for formats where structure, not field offsets, dominates
(counted, versioned or conditional records), and only if the result is
smaller.

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
