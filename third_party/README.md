# Vendored dependencies

## rerecast

`rerecast/` is the `rerecast` crate from the Bevy 0.20 upgrade branch,
[janhohenheim/rerecast PR #63](https://github.com/janhohenheim/rerecast/pull/63)
(`andriyDev/rerecast` at `0b41a3e0ff`), MIT OR Apache-2.0. Its tests and
workspace inheritance were removed; `Cargo.toml` lists the workspace's
dependency versions directly.

One behavioral change, in `src/poly_mesh.rs` `triangulate`: when no diagonal
can be found, upstream returns `PolygonNavmeshError::InvalidContour` and the
whole navmesh fails. recastnavigation's `triangulate` instead returns the
triangles found so far (`return -ntris`), and `rcBuildPolyMesh` logs a
warning and keeps them. The patch restores that. Without it, 18 of the 34
retail Wizardry 8 levels cannot be baked at all.

Drop this copy once an upstream release contains an equivalent fix.
