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

Bevy owns graphics resources and the render graph. World gameplay entities carry
or reference their render representation. Native children are appropriate for
owned mesh parts, effects and attachment points; they are not a second gameplay
scene graph.

Preserve material alpha behavior, texture sampling, billboards, fog, sky,
lighting and animation timing through representative comparisons. A modern PBR
default does not automatically preserve the old art. Start with the simplest
materials that reproduce it, then add a shader only for a demonstrated effect.
The retained scene/culling machinery should come from the engine, while game
visibility/detection rules remain separate from render culling.

The minimap and full map are views of the same level exploration/annotation
data. A render-to-texture camera is an option for world imagery, not the authority
for map discovery or marker coordinates.

## Animation: do not assume skeletal glTF

Whizzardry's [AnimationMesh](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/include/wiz8/entities/animation_mesh.h)
and [reader/player](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/entities/animation_mesh.cpp)
handle both vertex frames in one mesh and separate meshes per frame. The
broader [animation loader](https://github.com/agluszak/whizzardry-8/blob/9c433ec9725877a963cb43bb1df13d5d084ed873/src/entities/animation.cpp)
also contains positioned parts and paths. A conversion to ordinary skeletal
animation is not established by this evidence.

First prove one real monster's idle, movement, attack and death cycles and one
animated prop. A small retail-frame playback component can select/interpolate
decoded frame geometry. If profiling warrants it, move vertex-frame interpolation
to a shader. Shared frame assets remain immutable; instances own clip/time state.

Use semantic clip requests and identified completion waits. Where gameplay
requires a cycle milestone, the timeline reports it to the originating action.
Death is already authoritative before its visual retirement; the monastery
handoff explicitly waits for departure animation. These are different contracts.

## UI and input

Use native Bevy UI composition and stock interaction/focus behavior. Preserve
retail art with decoded image/font assets and appropriate sizing, while making
layout usable at modern resolutions. The initial game UI is not an egui editor.

A screen owns its view entities and drafts. Inventory slots display references
to item values; buttons invoke semantic operations. Refresh the affected view
from authoritative state, with direct retained entity handles where useful.
Do not construct an intermediate widget catalog, UI interpreter or general
binding framework just to update text and images.

Input contexts distinguish exploration, targeting, map editing, text entry,
dialogue and modal confirmation. Opening a modal blocks underlying pointer
picking and relevant global shortcuts, including edge movement. Keyboard focus,
controller navigation and accessibility labels belong to the actual UI nodes.
A map click must not also fire a combat action.

## Audio and cinematics

Speech playback owns an audio handle and completion/hold state. Dialogue owns
quote progression and its consequences. Mouth animation can use decoded voice
gap data. The original script system holds a completed voice before advancing;
its [voice handling](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Scripting.cpp#L432-L455)
and [completion hold](https://github.com/agluszak/wizardry-8-decomp/blob/075cbd0b853c4670e94f6ae4fe48d53b52b5e819/src/wiz8/local_code/NPC%20Scripting.cpp#L582-L594)
are behavior to model, not a reason to reproduce the old speech queue globally.

Use separate gameplay and presentation randomness for barks/visual variation.
Make music state, spatial sound placement and pause behavior explicit. Evaluate
FFmpeg only at the video boundary if the shipped cinematic formats require it.

## First proof and unresolved content

The first imported room must exercise actual geometry, collision, a door/trigger,
one monster's frame animation, one texture with alpha, a UI image/font and voice.
That is the useful test of whether the proposed adapters remove the old engine.

That proof now exists (see the [implementation plan](implementation-plan.md)
status). What it established:

- Direct loading suffices: every format so far is a Bevy `AssetLoader` over
  the `retail://` source (`.pvl`, `.lvl`, `.mls`, `.sti`, MP3, TGA). Only the
  navmesh bake, which is expensive, goes through the asset processor.
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
