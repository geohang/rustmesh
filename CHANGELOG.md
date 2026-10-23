# Changelog

## 0.1.0 (unreleased)

- 2D mesher: constrained Delaunay triangulation with quality refinement, regions
  with their own marker and largest cell area, holes, local cell sizes around
  chosen points, segments that refinement does not split, and segments that cross
  split at the crossing.
- Mesh reports that say whether refinement finished and whether every requested
  angle and area limit holds; strict entry points in Rust and Python that return
  an error carrying the mesh and its report.
- `Geometry2D`, a mesh input built from a drawing, and eleven templates.
- Experimental 3D tetrahedral meshing of triangulated complexes with regions,
  holes, volume limits and a minimum dihedral target.
- Exports: VTU, Gmsh 2.2 (signed markers kept in `$NodeData` and `$ElementData`),
  E4D mesh files, and `pygimli.Mesh`.
- Python package with a pyGIMLi adapter that can replace Triangle for 2D meshes.
- Resource limits and cancellation for meshing calls.
- Desktop and web app with a command line, a revisioned workspace, an MCP server
  and an optional agent panel (Sign in with ChatGPT).
- The patched triangulation library is vendored as `rustmesh-spade`.
- A website (`site/`) with Mesh Studio's web app under `app/`, published on
  GitHub Pages by `.github/workflows/pages.yml`.
- `THIRD_PARTY_NOTICES.md`; `cargo xtask web` writes the licence texts of the
  crates and fonts in the web app to `THIRD_PARTY_LICENSES.txt`, which the
  website publishes beside the app and Mesh Studio's About menu links to.
