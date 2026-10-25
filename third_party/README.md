# Vendored dependencies

## rerecast

`rerecast/` is the `rerecast` crate from the Bevy 0.20 upgrade branch,
[janhohenheim/rerecast PR #63](https://github.com/janhohenheim/rerecast/pull/63)
(`andriyDev/rerecast` at `0b41a3e0ff`), MIT OR Apache-2.0. Its tests and
workspace inheritance were removed; `Cargo.toml` lists the workspace's
dependency versions directly.

Two behavioral changes restore recastnavigation's behaviour:

1. **Region holes are merged into their outlines** (`src/contour_holes.rs`,
   called at the end of `build_contours`): a port of `mergeRegionHoles` from
   `RecastContour.cpp`, which upstream omits (rerecast issue #64). Without
   it, a region surrounding an obstacle yields a backwards-wound hole
   contour; triangulating it fails, and the outline is triangulated across
   the obstacle as if it were walkable. The port sorts stably and skips the
   diagonal's own outline vertex in the intersection test, where the C++
   passes the hole index.
2. **Partial triangulation is kept** (`src/poly_mesh.rs` `triangulate`):
   when no diagonal exists, upstream fails the whole navmesh with
   `InvalidContour`; Recast keeps the triangles found so far and warns.
   Since holes are merged, no retail level reaches this path.

Drop this copy once an upstream release contains equivalent fixes.
