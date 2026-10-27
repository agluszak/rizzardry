# Bevy 0.20: use the engine, not a second engine

Status: implementation guidance for Rizzardry, 25 October 2026. The
[`first-slice` branch](https://github.com/agluszak/rizzardry/tree/first-slice) is ahead of `main`;
implementations mentioned below are on that branch, not necessarily merged. The features and
examples refer specifically to **Bevy 0.20**, not an assumed future API.

The objective is less game-specific infrastructure and faithful gameplay, not maximum use of every
Bevy feature. Favor Bevy's ordinary assets, entities, materials, cameras, widgets and animation
machinery. Keep an explicit game-rule value when the engine's presentation state is not the
authority.

## Decisions by subsystem

| Job | Native feature | Choice and boundary |
| --- | --- | --- |
| Transform animation | `AnimationClip`, `AnimationPlayer`, animation curves and events | Prefer for nonphysical authored animation; keep collision-critical motion on the fixed simulation clock |
| Vertex-frame animation | Per-instance meshes from decoded frames | In use (CPU blend between frames); `MorphWeights` only if profiling demands it |
| Different mesh each frame | Shared `Handle<Mesh>` alternatives | Select the appropriate handle; do not force incompatible topology into morph targets |
| Retail lighting | `StandardMaterial`, `DistanceFog`, `RenderLayers` | Already in use on the implementation branch; preserve authored vertex light and fog |
| Specialized lighting | `ExtendedMaterial<StandardMaterial, ...>` and WESL shaders | Consider only if it simplifies correct runtime sunlight/material behavior |
| First-person interaction | Avian `SpatialQuery`; optionally Bevy picking | The existing center ray works for world props; don't maintain two authoritative targets |
| Integrated 3D viewport | `bevy::ui::widget::ViewportNode` | Strong candidate when building the framed Wizardry HUD; handles UI sizing and pointer forwarding |
| UI screens | Rust `bsn!`, native UI and headless widgets | Use for focus, buttons, text entry, dialogs and keyboard/controller input |
| Shared retail images | `AssetServer`, custom asset source, `CompressedImageSaver` | Make textures independently addressable before replacing manual mipmaps |
| Audio | `AudioPlayer`, `AudioSink`, `SpatialAudioSink` | In use with Bevy's `mp3` feature; voice completion is identified per line |
| Screen lifetime | Parent/child relations and `DespawnOnExit` | Use where natural; do not replace the game/save domain with global Bevy states |

## Animation: distinguish visual and physical timelines

The importer already decodes rigid part paths, frame-selected meshes and per-vertex frames. Convert
content to native representations rather than copying Whizzardry's animation class hierarchy.

**Rigid transform paths.** Bevy supports authored `AnimationClip` curves targeting ordinary
`Transform` fields; a skeleton or glTF intermediate is not required. Use `AnimationPlayer` and, when
blending is actually needed, `AnimationGraph` for actors and decorative props.
[Animation curves](https://github.com/bevyengine/bevy/blob/v0.20.0/crates/bevy_animation/src/animation_curves.rs)
and
[animation events](https://github.com/bevyengine/bevy/blob/v0.20.0/examples/animation/animation_events.rs)
are the relevant native APIs.

**Moving physical props are different.** Bevy's animation plugin advances players and applies target
curves in `PostUpdate`. Rizzardry's Avian/Tnua navigation and moving prop colliders run in
fixed-step schedules. The implementation branch already has a small `PropMotion` value and direct
transform sampling. **Do not replace it blindly with `AnimationPlayer`**, creating visual transforms
that lag or disagree with collision. Prefer one fixed-step authoritatively sampled pose for a
door/lift, reused by the visual parts and colliders. Bevy animation assets can still describe the
curves, but their player must not become a second physical clock. Only change this after proving
correct ordering at the same physical milestone.

**Vertex frames.** Test `MorphWeights` and `MeshMorphWeights` on one real monster with constant
topology and vertex order, including idle, walk, attack and death. Bevy supports up to 256 weights
per mesh, subject to texture/memory limits. Importing the retail format into Bevy morph-target data
may still need a small adapter. Evaluate memory, loading time, interpolation and the needed normals;
do not assume morph targets beat simple frame meshes for every actor. The
first-slice branch animates retail monsters by blending decoded frames into
each actor's own mesh copies on the CPU, which is cheap at retail vertex
counts (median about 200); move to morph targets only if profiling demands it.
[Bevy morph-target example](https://github.com/bevyengine/bevy/blob/v0.20.0/examples/animation/morph_targets.rs)
.

**Changed topology.** Retain immutable per-frame mesh assets and switch `Mesh3d` handles. No dynamic
GPU mesh edits or generalized playback framework are required without a concrete use.

**Gameplay milestones.** A clip event can announce a named animation point, but it must identify its
playback and its requesting action. Dialogue, combat or script state owns the durable pending
consequence; a Bevy event is not a save record. Visual death retirement and committed game death
remain separate.

## Rendering the retail world

The branch now decodes time-of-day environment tables, uses `DistanceFog` and clear colors,
maintains a sky camera with `RenderLayers`, and updates retail vertex colors from a game clock.
**Keep those built-ins**; do not replace them with a custom renderer or a physically based
interpretation of the old art.

The current vertex lighting works by updating `Mesh::ATTRIBUTE_COLOR` as the daylight phase changes.
This is a sensible baseline. A dedicated
[`ExtendedMaterial`](https://github.com/bevyengine/bevy/blob/v0.20.0/examples/shader/extended_material.rs)
plus WESL can be considered if it removes recurring CPU mesh updates *and* reproduces the original
equation and alpha behavior. A shader extension is not automatically smaller: it may require custom
vertex attributes, pipeline specialization, and extra testing. No shader work is justified solely by
wanting "modern" rendering.

Use `StandardMaterial::uv_transform` for scrolling clouds/textures where sufficient. Use Bevy's
built-in fog and ordinary cameras for fog and skies; no extra postprocessing graph is required.
Avoid Solari/path tracing, meshlet optimizations and elaborate atmospheric simulation unless a real
content requirement arises.
[Distance fog](https://github.com/bevyengine/bevy/blob/v0.20.0/examples/3d/fog.rs) .

## One selection model for world and UI

The implementation branch already casts an Avian ray against physical prop colliders, then resolves
the owning prop through parent relationships. Preserve that for the first-person use action if it
matches the original hit/range semantics.

Bevy's `MeshPickingPlugin` adds pointer hit testing on rendered meshes; it can be helpful for
mouse-selected items, portraits and world props. Bevy also coordinates pointer routing across UI and
3D targets. Don't run both mesh picking and Avian picking as competing sources of the same gameplay
target. Choose by interaction: **physical centered reach queries** for a first-person use action;
**Bevy picking and UI focus** for mouse widgets and 3D views. Rejected/noninteractive objects can
still occlude those behind them, as in the retail interaction rules.
[Mesh picking example](https://github.com/bevyengine/bevy/blob/v0.20.0/examples/picking/mesh_picking.rs)
.

## The Wizardry HUD: a real UI viewport

The original game embeds its world view in an interface. Bevy 0.20 has a native
[`ViewportNode`](https://github.com/bevyengine/bevy/blob/v0.20.0/examples/ui/widgets/viewport_node.rs)
: attach it to a UI `Node` with a camera rendering to an `Image`. The widget resizes the image with
the UI and forwards pointer input to the 3D camera, so prop picking can work inside a resizable
viewport.

Prefer `ViewportNode` when implementing the framed game interface. If the view is simply a fixed
window rectangle, `Camera.viewport` draws directly into the window without a separate offscreen
texture and can be simpler. Ordinary `RenderTarget::Image` and `RenderLayers` remain useful for item
previews, portraits and mirrors. **Do not create an additional scene-graph/UI composition engine.**
The viewport widget doesn't despawn its camera automatically; associate camera and UI lifetime
deliberately.

Compose screens with Rust `bsn!` where it is clearer than manual entity construction, and use native
UI/headless widgets for focus, scrolling, input capture, accessible controls and keyboard/gamepad
navigation. Use retail images/fonts for appearance, not Feathers' editor styling. Hot-reloadable
external `.bsn` scene files are **not** a shipped requirement of Bevy 0.20.
[BSN example](https://github.com/bevyengine/bevy/blob/v0.20.0/examples/scene/bsn.rs) .

Retail controls share `ControlSkin`, focus appearance and one discrete `step_scrollbar`
composition. `FocusHighlight` uses the hover frame where the artwork requires it;
other controls can use `FocusRing`. Screens set availability/selection and consume
native `SliderValue`; widgets own interaction, not a parallel state model. `Picture`/`BitmapText` own
layout/picking defaults, and `PictureCrop` clips source pixels without extra
bar hierarchies. Keep true content viewports as clipping nodes. HUD log scroll
remains a projection of `TextLog`, not another writable slider position.
Pool cells bubble wheel input to their pool's shared scrollbar. Menu actions use
enhanced input's per-binding reset for held keys; confirmation also guards native
button activation until Enter and Space have been released.

## Assets, sharing and mipmaps

`retail://` and `navmesh://` plus Bevy's asset processor are good boundaries. Keep the retail files
unmodified and the expensive navmesh bake cacheable. Cache invalidation must include algorithm
revisions, not only source bytes and settings.

Retail textures are shared assets addressed by retail path: materials load `<file>#image`, the
`image` sub-asset of a small `RetailTexture` asset whose loader decodes with retail sampling and
builds CPU mipmaps (`presentation/mipmaps.rs`). Its own asset type keeps Bevy's image loader the
only `Image` loader for `.tga`, so UI art loaded as a plain image is unaffected. Material blending
needs only whether the file carries alpha, which retail takes from the pixel format (a 32-bit TGA);
the loaders read that from the header. Bevy's
[`CompressedImageSaver`](https://github.com/bevyengine/bevy/blob/v0.20.0/examples/asset/compressed_image_saver.rs)
could later replace the CPU mipmapper inside one processing pipeline. Check target GPU format support
and fidelity of alpha-tested surfaces before enabling compression globally.

Animated `.ifl` textures need an ordinary shared image handle per frame plus the already-simple
playback selection. Do not add an animation-texture framework merely for these lists.

## Audio and lifetime

Retail scripts name voice lines as WAV, but the installation ships MP3, so enable Bevy 0.20's
`mp3` feature (the branch does). Start with
`AudioPlayer` and `AudioSink` or `SpatialAudioSink`, including native spatial positioning. Compare
actual voice completion, dialogue hold/pause and looping behavior before adding Kira. The game
dialogue/scene owns the outstanding request; presentation owns the Bevy playback entity/handle.

A level root owns its world objects and render children. Use Bevy relationships to despawn the
hierarchy; don't use a parallel object manager. Native `States`, `OnEnter`, `OnExit` and
`DespawnOnExit` help with menus or mutually exclusive UI screens but are not a replacement for
persisted party/campaign state or the explicit level transition.
[State-scoped entities](https://github.com/bevyengine/bevy/blob/v0.20.0/examples/ecs/state_scoped.rs)
.

## Relationships and notifications in the current implementation

Radar blips and travel dialogs use one-to-one Bevy relationships to their
subjects, with `linked_spawn` on the reverse target. The UI still owns its
layout through `ChildOf`. Removing a gameplay subject removes its view;
removing the UI removes the reverse reference without deleting gameplay.
Neither direction requires a polling cleanup system or a parallel entity map.
These presentation relationships are declared entirely in presentation modules.
View creation runs after level activation has committed its deferred despawns;
it cannot attach a view to a subject being removed in the same update.

Radar item visibility memory and glow timers are presentation components on
world items. They survive HUD reconstruction and disappear on level departure.
The existing `SearchNews::Spotted` message identifies its target, so glow follows
a committed discovery instead of comparing a cached copy of gameplay's hidden
flag. Messages suit these independent, scheduled notification readers. Mandatory
gameplay consequences still run in the operation itself, not observer chains.

Door navigation uses its existing `ChildOf`/`Children` hierarchy instead of a
second spawned-link list. Physical prop poses advance in `FixedPreUpdate`, then
door links update before Landmass's `SyncExistence` set. Navigation and physics
therefore consume the same doorway pose within each simulation step, including
frames with several fixed steps. Authored prop parts remain separate children
and are never removed when closing a navigation link.

## Next evaluation, in order

1. **One actual animated prop and one monster:** compare a fixed-step physical prop path to an
   ordinary Bevy clip; test morph targets versus mesh-frame selection for a real retail actor. Avoid
   bespoke animation machinery unless all native paths fail.
2. **One complete interaction:** finish an authored monastery mechanism (including sound, state,
   picking and an identified playback milestone). The implementation branch already has initial
   door/lever motion and links, so verify and extend it rather than replacing it wholesale.
3. **First HUD widget:** compose a small overlay with `bsn!`, native focus and a `ViewportNode`
   containing the moving party view; verify pointer/UI occlusion and gamepad focus.
4. **Image processing only when justified:** retail textures are now shared assets; replace the
   CPU mipmapper with asset processing only if that reduces code and work.
5. **Character, inventory and native saves:** retain explicit gameplay authority and records; Bevy
   world serialization is not the native save format.

Features are chosen to **remove** bespoke code. The existence of an engine facility is a reason to
evaluate it, not a reason to duplicate current working state or abandon original behavior.
