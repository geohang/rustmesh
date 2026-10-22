# Experimental 3D mesher

The Rust `tetrahedralize(Plc3D, Options3D)` and Python
`rustmesh.tetrahedralize(...)` generate unstructured tetrahedra from a
triangulated piecewise-linear complex. This is not an extrusion of a 2D mesh,
and it has no TetGen, Gmsh or C/C++ meshing dependency. It is a research
baseline: constraint recovery is bounded, quality improvement is best effort,
and it has not been tuned for large models.

## Input and output

- Vertices have three coordinates. The optional `facets` are triangular index
  triples (the input triangles) with material-interface or boundary markers.
  Without them, the domain is the convex hull of the points.
- Closed outer shells, closed internal interfaces, cavity seeds and interfaces
  that meet the outer shell conformingly are supported. Intersections must
  already be represented by shared vertices and edges; there is no triangle-soup
  repair and no general intersection stage.
- Region seeds are `(x, y, z, marker, max_volume)`. The last seed in a connected
  region wins; zero volume means no regional limit, and the global and regional
  limits combine as the smaller positive one.
- Hole seeds remove a connected compartment. A seed on a constraint or outside
  the domain is an error. Nested regions are classified by flood fill, not by
  shell parity.
- Input vertices that lie on an input triangle or an edge (surface electrodes,
  for instance) are incorporated before recovery, and their coordinates are kept
  exactly.
- The output has positive-volume tetrahedra, cell neighbours (slot `k` is
  opposite local vertex `k`; `-1` is the exterior), boundary and interface
  triangles with the input triangle each one came from (`face_facets`), material
  markers, the mapping from input vertices to nodes, and volume and dihedral
  statistics. Exterior faces wind outwards; internal faces use their first
  incident cell.

## How it works

1. Check finite coordinates, indices, duplicates, degenerate triangles, marker
   counts and open edges. This is not a complete intersection test.
2. Scale by an exact power of two, which keeps the binary coordinates of the
   input intact. Translating by a rounded centre and dividing by an arbitrary
   extent can change the orientation of nearly coplanar tetrahedra.
3. Incremental Bowyer-Watson insertion with `robust::orient3d` and `insphere`.
   An explicit convex hull stands for the exterior cells at infinity, so there is
   no finite super-tetrahedron. Exactly cospherical ties follow insertion order.
4. Recover absent constraint edges by subdivision and missing triangle interiors
   with extra points. All input triangles that share a split edge are updated
   together, and every piece keeps the index of the input triangle it came from.
5. Flood from unconstrained hull faces to remove the exterior, then classify
   holes and material regions without crossing recovered faces.
6. Insert the centroids of cells that exceed a volume limit and repeat recovery.
7. With `min_dihedral > 0`, improve the quality (below).
8. Audit the output: cell orientation, shared-face incidence, neighbour
   reciprocity, domain-boundary constraints, and the recovered area of every
   input triangle.

All constructions use floating-point coordinates. The adaptive predicates decide
signs for those coordinates; they do not make generated midpoints or centroids
exact. The area covered by each input triangle is checked with a relative
tolerance of 1e-8. There is no random jitter, and results are deterministic for
a fixed input order.

### Quality improvement (`min_dihedral > 0`)

- Flips. Edge removal (3-2, 4-4, 5-6 ... flips, with the best retriangulation
  of the edge's ring chosen by Klincsek's dynamic programme, after Shewchuk's
  topological improvement of tetrahedral meshes) and 2-3 face flips, applied to
  the cells below the target, worst first. A flip is kept only if the worst
  dihedral of the cells it replaces strictly improves. Constraint faces and
  edges are never removed, a flip stays inside one region, new cells need a
  positive exact orientation, and before a replacement is committed the cavity
  boundary, face incidence, duplicate cells, total volume and each cell's volume
  limit are checked.
- Insertion, where flips leave cells below the target: a vertex at the
  circumcentre of the sliver, when that point lies inside the domain, encroaches
  no constraint triangle and keeps a quarter of the cell's shortest edge from
  every vertex. A circumcentre that encroaches a constraint triangle splits its
  longest edge instead, following the encroachment rule of Shewchuk's Delaunay
  refinement papers; this also refines skinny boundary triangles. The next pass
  rebuilds and recovers as usual.
- Rollback. The state before each insertion is kept. If recovery cannot
  follow (it fails, runs past its allowance of new vertices, or the domain
  volume changes, which means classification leaked through a gap), that state is
  restored and insertion stops. The best state seen is selected by volume
  violations, then angle violations, then worst angle.
- Interior smoothing. A bounded pattern search moves only generated interior
  vertices. Input points, constrained faces, the hull and material interfaces
  stay fixed. A move is accepted only if every affected cell stays positive,
  volume limits still hold, the local worst angle improves, and the number of
  angle violations does not rise. Up to eight sweeps run.
- Local conforming refinement. After classification, excluded cells are
  removed from the working complex and the whole star of a long edge is bisected,
  updating every incident constraint piece and keeping the input-triangle index.
  This avoids another Delaunay rebuild. Each child must be positive and the split
  must conserve volume; a rejected split leaves the old state intact.
- Constrained smoothing and insertion. Flips and smoothing alternate. A
  generated boundary or interface point may slide only on the plane of its
  original input triangle or along a common original edge; folding triangles and
  inverted tetrahedra are rejected. After up to 12 rounds of edge refinement, the
  best state is restored and up to four rounds of interior 1-to-4 cell
  subdivision are tried. A protected acute wedge is not refined again and again
  at its fixed edge.
- Selection. Local trials may temporarily raise the number of cells below the
  target. The best state is exported, ranked by volume violations, angle
  violations, then worst angle.

Dihedral angles use `atan2`, which is stable near 0 and 180 degrees. A violation
at an edge where two boundary or interface faces of the cell meet is counted in
`forced_dihedrals`; that is a local diagnostic, not a proof that the target is
unreachable.

A torus surface with planar, cocircular quads shows why flips matter: floating
point input leaves near-degenerate cells of tiny positive volume there, a vertex
placed in one only makes more flat cells, and removing the quad's other diagonal
removes them.

## Other Rust entry points

These are opt-in, experimental, and not exposed in Python.

- `tetrahedralize_sized(plc, options, &Sizing3D)`: after recovery and the quality
  stage, bisects the star of the edge with the largest length-to-target ratio
  until the sizes hold or `max_splits` or `max_vertices` is reached, then runs
  protected flips (`flip_passes` in total, alternating with at most three
  contraction passes that remove generated interior vertices). The target size is
  `h(x) = min(far_size, near_size + growth * distance(x, points))`, and an edge
  must be no longer than `min(h(a), h(b))`. The report adds
  `sizing_edge_violations` and `sizing_max_edge_ratio`, and unmet sizing makes
  `complete` false.
- `prepare_surface_patches(plc, protected_edges)` and
  `tetrahedralize_patched(plc, options, sizing, protected_edges)`: groups
  edge-connected, exactly coplanar input triangles with the same marker into
  patches, so that coplanar triangulation seams are not treated as features.
  Creases, marker changes, junction edges and protected edges stay features. The
  result (`PatchedMesh3D`) adds `face_patches` and `face_sources`, which lists for
  every exported face the original input triangles it covers and their areas.
  Use `face_sources`, not `mesh.face_facets`, which is -1 where a face has
  several contributors.
- `tetrahedralize_quality(plc, options, sizing, &Quality3D, protected_edges)`:
  Delaunay refinement towards a radius-edge limit (`max_radius_edge`, default 2;
  `max_insertions`, default 2000) with local constrained Bowyer-Watson cavities.
  Segments and surface triangles are repaired before cells, a proposed
  circumcentre that encroaches a surface is redirected to boundary repair, and a
  cavity replacement is committed only after its children, volumes, vertices,
  coverage, feature chains and adjacency check out. `sizing` supplies only the
  spatial size field here. The result is a `PatchedMesh3D` whose `report.quality`
  counts the insertions, rejected transactions and remaining violations. A
  radius-edge bound does not imply a dihedral bound.

## Report and failure contract

Failure to recover any required input triangle, a degenerate or inverted exported
cell, or an invalid cavity is an error, and `strict=False` does not change that.

`report.complete` means the geometry was recovered and the requested volume and
minimum-dihedral checks passed. It does not say the mesh is good enough for a
simulation. `min_dihedral = 0` switches quality improvement and the angle check
off: the conforming Delaunay cells come back unchanged, and a point cloud gives
exactly its Delaunay tetrahedralization. A target above zero drives flips,
insertion, local subdivision and constrained smoothing, best effort, and is then
checked.

Python defaults to `strict=True`, so unmet limits raise `MeshQualityError` (a
`RuntimeError`) whose `mesh` attribute keeps the recovered mesh and report. With
`strict=False`, a mesh that misses its limits comes back with `complete=False`
and the reason in `issues` (budget, stalled, no placeable vertex, or a
rolled-back insertion). Severe slivers add a warning whatever the target.

The report also has `smoothing_moves`, `smoothing_sweeps`, `flips`,
`interior_collapses`, `local_edge_splits`, `local_cell_splits`,
`local_refinement_passes` (local trial rounds, including discarded ones, kept
apart from `passes`, the global rebuilds), `sliver_count` (cells with a minimum
dihedral under 1 degree), and the quantiles `min_dihedral_p05`,
`min_dihedral_median` and `min_dihedral_p95` of each cell's minimum dihedral
(linear interpolation, not all six angles pooled). Shape diagnostics are
`radius_edge_median`, `radius_edge_p95`, `radius_edge_max`, `mean_ratio_min` and
`mean_ratio_median`. The mean ratio is `12 (3V)^(2/3) / sum(edge_length^2)`: one
for a regular tetrahedron and near zero for a sliver whose radius-edge ratio is
still modest. If a cell cannot be measured finitely, these values are `None`
and `shape_metric_failures` is nonzero. These fields describe the mesh; they are
not limits and do not change `complete`. In Python, `mesh.cell_min_dihedrals`
recomputes the per-cell values from the current arrays.

## Limits

Budgets default to 10 000 vertices and 100 rebuilding passes. They limit
vertices and iterations, not wall time or the number of tetrahedra. The local
stage adds at most 12 edge-refinement and 4 cell-insertion rounds, sharing
`max_vertices`. Full conflict scans, rebuilding and surface matching are
deliberately simple and can be slow. There is no general termination guarantee,
small-angle protection, sliver exudation, cavity-based recovery of input
triangles or production-scale acceleration, and near-degenerate input may fail.
Coordinate extents must lie between 1e-90 and 1e90. Next steps are local
cavities for recovering input triangles with protection of acute features, exact
or implicit constructions for generated points, adjacency maintained during
insertion, and a broader set of test geometries.

## Python example

```python
import rustmesh

mesh = rustmesh.tetrahedralize(
    vertices=[[0, 0, 0], [1, 0, 0], [0, 1, 0], [0, 0, 1]],
    facets=[[0, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]],
    facet_markers=[-1, -2, -2, -2],
    regions=[[0.1, 0.1, 0.1, 2]],
)
mesh.write_vtu("tetrahedron.vtu")
from rustmesh.pygimli import to_pygimli
pg_mesh = to_pygimli(mesh)
```

`write_vtu` writes cells, material and region ids, node markers and the per-cell
volume and minimum dihedral (leave the last two out with
`include_quality=False`). `write_boundary_vtu` writes a separate triangle mesh
with boundary and interface markers, input-triangle indices and both incident
cells. Open both files in ParaView to find bad cells and inspect interfaces. E4D
mesh files come from `rustmesh.e4d.write_e4d`; see [e4d.md](e4d.md).

## Tests

```text
cargo test -p rustmesh
pytest tests
```

The Rust tests cover a concave L-shaped volume, an inclined interface with
regional volume limits, an internal cavity, surface and edge electrodes, marker
and determinism checks, a cospherical grid, a very thin tetrahedron, invalid
input, and a dihedral target that is met or reported as unmet. The Python tests
check regional volumes (0.4 and 0.6) and the interface area (sqrt(1.04)), strict
errors that keep the mesh, rejected arguments, and the VTU and pyGIMLi exports.

## References

- G. T. Klincsek, Minimal triangulations of polygonal domains, Annals of
  Discrete Mathematics 9, 121-123, 1980.
- J. R. Shewchuk, Adaptive precision floating-point arithmetic and fast robust
  geometric predicates, Discrete & Computational Geometry 18, 305-363, 1997.
- J. R. Shewchuk, Two discrete optimization algorithms for the topological
  improvement of tetrahedral meshes, unpublished manuscript, 2002.
- J. R. Shewchuk, Delaunay refinement algorithms for triangular mesh generation,
  Computational Geometry 22, 21-74, 2002.
- Y. Hu, T. Schneider, B. Wang, D. Zorin, D. Panozzo, Fast tetrahedral meshing in
  the wild, ACM Transactions on Graphics 39(4), 2020. The use of a shape
  objective (the mean ratio) to schedule and accept topology changes, and the
  removal of generated interior vertices, follow its ideas.
- L. Diazzi, D. Panozzo, A. Vaxman, M. Attene, Constrained Delaunay
  tetrahedrization: a robust and practical approach, ACM Transactions on Graphics
  42(6), 2023. The reference for robust constraint recovery; rustmesh does not
  implement its exact constructions.
- The E4D mesh file formats: PNNL, E4D user guide, "Electrical Methods Mesh Files".

The 2D references are in [design.md](design.md#references).
