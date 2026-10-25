# Dependencies

Research snapshot: 24 October 2026. These are proposed choices, not a tested
`Cargo.lock`. The first implementation spike must compile the selected set on
the native target platforms before the versions become project commitments.

## Proposed starting set

| Need | Choice | Reason and limit |
| --- | --- | --- |
| Engine | Bevy 0.20.0 | ECS, rendering, assets, UI, window/input, audio and schedules in one engine |
| Input actions | `bevy_enhanced_input` 0.27 | Contexts and remappable actions for exploration, combat targeting and modal UI |
| Collision/controller | `bevy_rapier3d` 0.37 | Scene/shape queries and a built-in kinematic controller for stairs, slopes and platforms |
| Definition/save serialization | `serde` + `ron` | Typed, inspectable native records; schema versioning remains our responsibility |
| Retail binary parsing | `binrw` | Declarative endian-aware records where it helps; ordinary parsing for irregular formats |
| Developer inspection | Optional `bevy-inspector-egui` 0.38 | Inspect ECS/assets during development; not the game's UI framework |
| Navigation | Unresolved; Rerecast + Landmass are candidates | Must pass a representative Wizardry map and match the chosen Bevy release |

Primary references:

- [Bevy 0.20 release](https://bevy.org/news/bevy-0-20/) (8 October 2026).
- [Enhanced Input compatibility table](https://github.com/simgine/bevy_enhanced_input#bevy-compatibility).
- [Rapier plugin documentation](https://docs.rs/bevy_rapier3d/0.37.0/bevy_rapier3d/)
  and [kinematic controller](https://rapier.rs/docs/user_guides/bevy_plugin/character_controller/).
- [Inspector package and Bevy 0.20 dependencies](https://docs.rs/crate/bevy-inspector-egui/0.38.0).
- [Serde](https://serde.rs/), [RON](https://docs.rs/ron/0.12.2/ron/),
  [binrw](https://docs.rs/binrw/0.15.2/binrw/).

Rapier is the first movement candidate because the controller replaces actual
code we otherwise have to write. This does not mean delegating combat rules to
rigid-body dynamics. The party remains kinematic; attack range, area membership,
initiative, conditions and damage are game rules. Controller defaults do not
prove matching Wizardry movement. Test narrow doors, steps, slopes, elevators,
terrain edges, collision dimensions and collision-query filters.

Avian remains a reasonable alternative if the real-map test gives it a better
integration story. Adding both engines, or a large physics abstraction to make
them interchangeable, would defeat the purpose. Tnua is an additional controller
candidate only if a floating-controller model solves a demonstrated problem.

## The navigation decision is still open

The published compatibility data reviewed for this plan gives:

| Package | Reviewed release | Bevy requirement |
| --- | --- | --- |
| `bevy_rerecast` | 0.5 | 0.19 |
| `bevy_landmass` | 0.12.1 | 0.19 |

Sources: [Rerecast compatibility](https://github.com/janhohenheim/rerecast#compatibility),
[Rerecast package](https://docs.rs/crate/bevy_rerecast/0.5.0),
[Landmass package](https://docs.rs/crate/bevy_landmass/0.12.1).

Do not list those versions as working Bevy 0.20 dependencies. Evaluate a small
compatible upstream update, a proven lower-level integration, or a coherent
Bevy 0.19 stack. Choose the smallest working stack after the map spike; do not
maintain a substantial engine-version bridge for the sake of using the newest
Bevy. If 0.19 wins, select its matching input, physics and inspector releases as
a set and record the result here.

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
