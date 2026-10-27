# Content, assets and presentation

## Boundary

The new runtime consumes typed definitions and Bevy assets. `retail` understands
the old encodings and identifiers; normal gameplay does not understand SLF
directory entries, packed record offsets, Windows paths or raw script opcodes.

Start by loading the user's retail installation through a Bevy asset source and
small format loaders. A one-time converter can be added where processing is
expensive or native authored output is useful. Do not require conversion of the
entire game before one room works. Both direct loading and conversion, if used,
must produce the same typed definitions; they are not separate game models.

The decomp's [format evidence](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/docs/wiz8-data-formats.md)
documents archive, waypoint, level and database boundaries. Whizzardry's
[typed database loading](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/wiz8/loaded_game_data.cpp)
is a useful comparison for separating disk records from live values. Neither
requires copying its file-handle API into Rust.

## Import responsibilities

| Input | Native result | Checks |
| --- | --- | --- |
| SLF and loose files | One asset namespace with documented override order | Bounded entries, normalized paths, case handling, duplicate names |
| Item/spell/monster/profession databases | Immutable typed definitions | Counts, enum ranges, ID references, dice/range semantics |
| Level geometry and placements | Mesh/material assets and spawn definitions | Coordinates/scale, handedness, collision surfaces, named points and trigger links |
| Waypoints/links | Navigation input or explicit authored links | Directionality, connectivity, blocked/dynamic links; no discarded gameplay link semantics |
| Images and fonts | Native images/atlases/font assets | Palette/alpha/key color, frame offsets, dimensions and glyph metrics |
| Animation cycles | Shared geometry/clip data and per-instance playback | Frames, bounds, loop/end rules, semantic completion points |
| NPC/monster scripts and strings | Dialogue content and explicit supported operations | References, branches, side effects and unknown opcodes |
| Sound/music/video | Native audio assets or a contained decoder path | Formats, streaming, timing, loop points and completion |

Do not load an unknown enum or script operation as a silent no-op. Report the
source asset and record. Unknown bytes can remain in an import record until
understood, without contaminating the live component layout.

Importing should move in one direction: `retail` decodes and normalizes into
`content`; gameplay uses those content values and must not parse retail
bytes. Decode directly into a native definition when there is no meaningful
intermediate representation. Keep a separate raw record only for a genuine
format/native distinction (versioned disk fields, validation, unit conversion
or unresolved references), not to duplicate the same fields and copy them
one by one. `content` and shared presentation types must not depend on
`retail`; move a genuinely reusable type or font operation below both users
rather than adding another mirror representation.

Monster body metadata and visual cycles have independent readiness. The retail
adapter registers typed `.mls` loaders for `MonsterBody` and `MonsterModel`:
the body loader reads the species description without loading `.mon` cycles,
meshes or materials. Level activation waits for body metadata, while
presentation requests visual models separately. A failed visual cycle cannot
change gameplay dimensions or prevent an otherwise playable level activating.
Effective scale, reach radius, eye height and movement speed belong to the live
monster. Persistence captures those values and derives feet from gameplay's
controller offset; it never requires an attached visual model. Activation moves
owned dormant monsters, world items and group knowledge back into ECS.

## Stable content identity and overrides

Authored content uses stable namespaced keys. The retail adapter maps numbered
records and named triggers to those keys. Saves refer to content keys plus the
content/schema version needed to interpret them; a reordered catalog must not
turn a sword into a potion.

Native RON definitions are suitable for items, spells, encounters, dialogue and
level-specific parameters. Resolve and validate cross-references during load.
Define override precedence explicitly, for example base retail content followed
by an ordered list of mod packs. Duplicate keys without an intentional override
are load errors. Do not let filesystem enumeration order determine which mod wins.

Keep special rules in small Rust functions until enough examples justify a
declarative form. A spell can use an enum selecting a known effect function;
there is no need to invent a universal expression language for every formula.
Mod scripting is a later capability, not a prerequisite for data mods.

## Rendering

Bevy owns graphics resources and the render graph. World gameplay entities
carry or reference their native render representation. Child entities are
appropriate for mesh parts and attachment points, not a parallel scene graph.

The implementation branch already uses `StandardMaterial`, built-in
`DistanceFog`, `RenderLayers` for a sky camera, and retail environment tables
for time-of-day clear color and vertex lighting. These are the right primitives.
Preserve retail alpha rules, UV sampling, vertex illumination and view distance
instead of blindly replacing them with physically based defaults.

The current vertex relighting updates `Mesh::ATTRIBUTE_COLOR` when the time
phase changes. Keep it until a measured or correctness-driven need warrants a
`MaterialExtension` with a small WESL shader. Such a shader could calculate
time-dependent sunlight with per-vertex attributes on the GPU, but it can also
add custom pipeline complexity. Don't add one merely to be modern.
`StandardMaterial::uv_transform` is sufficient for simple scrolling clouds.

The minimap and full map remain projections of exploration/annotation data;
camera images are not the source of map discovery. See the specific
[Bevy 0.20 rendering decisions](bevy-features.md#rendering-the-retail-world)
and [UI viewport plan](bevy-features.md#the-wizardry-hud-a-real-ui-viewport).

## Animation: do not assume skeletal glTF

Whizzardry's [AnimationMesh](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/include/wiz8/entities/animation_mesh.h)
and [reader/player](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/entities/animation_mesh.cpp)
handle vertex frames and separate meshes per frame. The
[animation loader](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/entities/animation.cpp)
also uses rigid positioned parts and keyframe paths. There is no evidence that
all retail animation can be converted to skeletons.

Choose native Bevy representations **per actual format**:

- Rigid transform paths: `AnimationClip` with transform curves and
  `AnimationPlayer` for visuals; `AnimationGraph` only when blending is needed.
- Fixed-topology vertex frames: evaluate Bevy `MorphWeights` on a real monster,
  considering memory and GPU limits. Do not assume compressed retail frames
  are automatically usable as morph targets.
- Whole-mesh frame changes: switch among shared `Handle<Mesh>` values.
- Collision-relevant doors and lifts: retain one authoritative fixed-step
  `PropMotion`/pose shared by the visible parts and Avian colliders. Bevy's
  standard animation plugin runs in `PostUpdate`, not the physics fixed step;
  migrating the pose directly to an `AnimationPlayer` can desynchronize it.

First compare a real monster's idle/walk/attack/death and one moving prop. Use
animation events for presentation milestones only with playback identity; game
operations remain authoritative for committed effects and outstanding waits.
Details and Bevy references: [animation integration](bevy-features.md#animation-distinguish-visual-and-physical-timelines).

## UI and input

Use Rust `bsn!`, native Bevy UI layout, headless widgets, focus and picking.
Preserve retail art and decoded fonts rather than adopt editor-styled Feathers
as the game's appearance. External hot-reloadable `.bsn` assets are not a
Bevy 0.20 dependency; prefer native Rust composition.

For the original **framed 3D game viewport**, evaluate Bevy 0.20
`ViewportNode`. It sizes an offscreen render target from its UI node and
forwards pointer input to the 3D camera. A bare `Camera.viewport` is simpler
when the camera merely occupies a fixed rectangle; render-to-texture is useful
for UI previews and portraits, not obligatory for the main world.

Use the existing Avian center-ray for physical world interactions unless
Bevy's 3D pointer picking replaces it cleanly. Avoid competing world
selection models. UI overlays and modals must block underlying pointer
events and global shortcuts as appropriate. Suppressing an input context is
not gameplay authorization: direct world, inventory, camp and combat
operations still validate their own current activity and prerequisites.

Screen roots own their view entities and drafts. Values such as items,
conditions, journal facts and map annotations stay in game state. Native
`States` and `DespawnOnExit` can scope screens where useful, without a
catch-all `UiState` or widget interpreter. Input contexts distinguish
exploration, targeting, dialogue, map editing and text entry. See
[Bevy UI and picking](bevy-features.md#one-selection-model-for-world-and-ui).

## Audio and cinematics

Speech playback owns an audio handle and completion/hold state. Dialogue owns
quote progression and its consequences. Mouth animation can use decoded voice
gap data. The original script system holds a completed voice before advancing;
its [voice handling](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Scripting.cpp#L432-L455)
and [completion hold](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Scripting.cpp#L582-L594)
are behavior to model, not a reason to reproduce the old speech queue globally.

Use separate gameplay and presentation randomness for barks/visual variation.
Make music state, spatial sound placement and pause behavior explicit. Enable
the `wav` feature on Bevy for retail WAV playback (the default `audio`
collection includes Vorbis, not WAV). Start with `AudioPlayer`,
`AudioSink` and `SpatialAudioSink`, and characterize actual completion and
hold behavior before adding a separate audio library. Evaluate FFmpeg only at
the video boundary if the shipped cinematic formats require it.

## First proof and unresolved content

The first imported room must exercise geometry, collision, an authored
door/trigger, a real actor's animation, one alpha texture, a UI viewport and
one voice. The `first-slice` branch already demonstrates world/prop meshes,
collisions, door/lever motion, links, teleports, fog, sky and animated textures;
it does not yet demonstrate the full actor/voice/UI loop. Prefer completing
that loop using Bevy's facilities over adding new engine code.

That proof now exists (see the [implementation plan](implementation-plan.md)
status). What it established:

- Direct loading suffices: every format so far is a Bevy `AssetLoader` over
  the `retail://` source (`.pvl`, `.lvl`, `.mls`, `.sti`, MP3, TGA). The
  navmesh loader also bakes, which is expensive, so it caches its results.
- Monster cycles are vertex frames, not skeletons. A model asset holds each
  cycle's frames; every actor animates its own mesh copies on the CPU, which
  is cheap at retail vertex counts (median about 200).
- Material alpha follows retail's texture slots: slots 0-1 blend, slots 2-3
  add. Facing props are full billboards.
- Retail fonts are STCI sprite sheets with no metrics table; text is drawn
  into an image for Bevy UI.

Retail `.NSF`/`.MSF` dialogue and level/database payloads are absent from the
research trees. This prevents proving the monastery's precise entry/quote gates,
the spaceport's complete terminal tree and a named bomb's complete definition.
Those gaps are tracked in [revisions](revisions.md), not filled with invented
content. The repository contains design/source references, not redistributed
retail assets.
