# Native monster morphs and picking

Bevy 0.20 can represent the shipped cycles with shared meshes and per-instance
`MorphWeights`. The GOG 2001-12-23 (22306) installation contains 1,088 nonempty
monster cycles (maximum 100 frames; Bevy supports 256 targets). Each cycle has
fixed topology, but 179 of 203 usable species change vertex count between cycles.
Keep separate cycle meshes, not a species-wide frame bank. Savant Trooper has 199
source vertices for idle and 261 for death. Baruta fish WALK overrides the file's
30 FPS with 60.

The loader expands source indices once into native position deltas. Runtime
retains the existing request/restart clock, initial phase, FPS overrides and
last-frame hold (no last-to-first interpolation). Each part references its cycle
root's weights. The source frames and render-source index arrays are discarded.

A float32 reconstruction on Savant idle/walk/attacks/death and Dark Savant's
100-frame death compared 2,584,704 coordinates, including fractional frames,
loop boundaries and one-shot completion. Maximum discrepancy: 4.77e-7 metres.
The retail loader test exercises the actual conversion and conservative bounds.
A Vulkan/lavapipe comparison of Savant ATTACK_THRUST at frame 10.375, after
hiding/re-showing with unchanged weights, produced the same silhouette for CPU
reference versus GPU morph rendering. Of 614,400 corresponding pixels, 182 had
a one-level (out of 255) RGB difference; none differed more. This proves that
representative pose/path, not all-model visual parity or a frame-time gain.

## Picking uses the same deformation

Bevy 0.20 `MeshRayCast` does not apply morph weights. `morph_picking` filters those
meshes out of the static pass, rejects invisible/sky/out-of-ray bounds, and samples
the native mesh and weights into reusable scratch positions only for ray candidates.
Bevy's `ray_mesh_intersection` performs the triangle intersection; the nearest
animated or static hit wins. No second `Mesh`, persistent pose cache, geometry
upload or coarse collider substitutes for the drawn surface.

Bounds enclose every frame and use `NoAutoAabb`: Savant thrust reaches +1.947 m Z,
versus +0.916 m at frame zero. Linear adjacent-frame interpolation stays inside
the union. This is not a general bound for arbitrary additive/extrapolated morphs.

## Tradeoffs

Native `MorphAttributes` cost 48 bytes per expanded vertex per frame. Savant's
nine cycles need 4.34 MiB versus 0.61 MiB of source positions; Dark Savant's complete
set needs 23.66 MiB. CPU and GPU retain shared native morph data because accurate
picking needs CPU access. This removes per-instance meshes and position uploads,
but increases resident pose data; no unconditional memory or speed win is claimed.

A GPU picking replacement would require renderer/depth/readback integration and
stale-result handling. Bevy GPU picking proposals #6991/#8784 are unmerged; the
latter documents two-frame latency. The synchronous native triangle helper is the
smaller compatible boundary until Bevy supports morph-aware mesh picking itself.

Bevy [#25966](https://github.com/bevyengine/bevy/issues/25966) also affects hidden
cycles becoming visible: GPU-built instance uniforms can retain a missing morph
descriptor. Use `PbrPlugin::use_gpu_instance_buffer_builder = false` until the
engine fixes that dependency, and cap `GpuPreprocessingSupport` to `None` after
its `RenderStartup` initialization so opaque phase batching selects matching CPU
bins (including device recovery). Deformation still runs on the GPU; only instance
uniform construction uses the native CPU path. This is a global renderer
performance tradeoff, not a second animation representation or fake change tick.
