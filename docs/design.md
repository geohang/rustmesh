# rustmesh design

## Goal

A permissively licensed (MIT OR Apache-2.0) mesh generator with a Rust core and
a Python wheel. It meshes 2D planar straight-line graphs with quality
constraints, and 3D triangulated complexes experimentally. Geophysical
inversion was the first use: for the geometries rustmesh covers it can stand in
for Triangle (2D; its licence restricts commercial redistribution), TetGen
(AGPL-3.0) and Gmsh (GPL) as the mesher behind pyGIMLi and PyHydroGeophysX.

Speed is not the reason. The reasons are the licence, robustness (no C++
crashes), one wheel that needs no C toolchain, and an API that works with
markers, regions and sensors.

## Dependencies and layout

- `spade` 2.15 (MIT OR Apache-2.0, pure Rust): constrained Delaunay
  triangulation and Ruppert/Chew refinement. A patched copy is vendored in
  `vendor/spade`; [PATCHES.md](../vendor/spade/PATCHES.md) lists the changes.
- `robust` for exact orientation and in-sphere predicates, `pyo3` 0.29 and
  `numpy` 0.29 for the Python extension. The desktop and web app uses `eframe`.

```
Cargo.toml                 workspace
crates/rustmesh/           the mesher, pure Rust, no Python
crates/rustmesh-py/        PyO3 extension `_rustmesh`
crates/rustmesh-app/       desktop and web app, command line, MCP server
python/rustmesh/           the Python package: arrays in and out
                           (`rustmesh.pygimli` imports pygimli lazily)
vendor/spade/              patched spade
xtask/                     `cargo xtask web`
tests/                     Python tests
examples/                  an ERT forward-modelling check (needs pyGIMLi)
```

Release wheels are built with maturin (`abi3-py310`); for local builds on
Windows without Visual Studio, `scripts/develop-windows-gnu.ps1` builds into a
Python environment with rustup's GNU toolchain.

## 2D mesher

### Input and output

`Plc` holds vertices with markers, segments with markers, region seeds (point,
marker, largest cell area, 0 for no limit) and hole seeds. `Options` holds the
smallest angle in degrees (default 28, allowed 0 to 40, 0 turns angle
refinement off), the largest area, a vertex budget, the points to refine
around (`refine_points`, `point_size`, `size_growth`), and the segments that
refinement must not split (`keep_segments`, `keep_boundary`).

`Mesh2D` has the nodes, counter-clockwise triangles, cell markers, the index of
the region seed that reached each cell (`cell_regions`, kept apart from the
marker because several regions can share one), the mesh edges that lie on input
segments (with the segment index and marker) and on the domain outline, node
markers, and `input_vertex_nodes`, which maps each input vertex to its node.

Input is checked before meshing: non-finite coordinates, duplicate vertices,
bad indices, zero-length segments and overlapping segments are rejected. A
segment end on another segment is allowed. Segments that cross are split at the
crossing, and the report says so.

### How it works

1. Insert the vertices and four frame vertices well outside the input, so that
   the domain's outline never lies on the convex hull. Without the frame, an
   electrode a rounding error off a sloping outline leaves a flat sliver
   between hull and outline, and refinement then cannot split the outline next
   to it.
2. Insert the segments. Each constraint edge is mapped to the input segment it
   belongs to, and the map is rebuilt after every refinement by following chains
   of new vertices between old ones. An edge that maps to no segment, or a
   segment that is lost, is an error.
3. Classify the faces by flood fill, not by spade's parity peel: the exterior
   from the hull edges that are not constraints, then holes, then region seeds
   in order (a later seed wins). Parity layers cannot represent a zone enclosed
   by another zone, or the layout pyGIMLi uses for ERT, where the inversion
   region touches the ground surface and lies inside the outer region.
4. Refine with one call of the patched `refine_with_kept_edges`: the excluded
   faces are the exterior and holes, the area limit of a face is the smaller of
   the global limit, its region's limit and the size the refine points ask for,
   and a callback enforces the vertex budget and cooperative limits.
5. Export the faces that belong to the domain and measure the result: smallest
   angle, cells below the requested angle (tolerance 1e-6 degrees) and cells
   over their area limit (relative tolerance 1e-9).

`Options::point_size` is the target edge length at the refine points, growing by
`size_growth` per unit of distance from the nearest one; a cell is split while
its area exceeds that of an equilateral triangle with the target edge at its
centroid. The nearest point comes from a uniform grid whose search stops on the
distance to the part of the grid not yet searched.

### Robustness and reproducibility

- A geometry far from the origin (UTM coordinates) is meshed about its middle
  when it lies more than ten times its own size away, and the input vertices
  get their exact coordinates back.
- The domain's area is taken about its first corner, and the tolerance for
  merging points is at least 64 ulps of the coordinates.
- The angle limit's radius-to-edge ratio comes from a Taylor series instead of
  `sin`, and spade's `nearest_power_of_two` is computed exactly, so a browser
  and a native build mesh the same input identically.
- Panics inside the triangulation library are caught at the API boundary and
  returned as errors.

### Quality targets and the report

`Report::refinement_complete` says only that refinement stopped within its
budget; spade reports nothing else. rustmesh measures the exported mesh itself:
`quality_met` is true when no cell is below the angle or over the area limit,
and `complete` needs both. `issues` lists why a mesh is incomplete, one line
each, and `warnings` lists things that do not make it incomplete (seeds on a
segment, crossings).

`triangulate` returns `Ok` even when `complete` is false, and the caller reads
the report. `triangulate_strict` returns `Ok` only for a complete, nonempty,
positive-area mesh. Otherwise it returns `TriangulationError`: invalid input,
cancellation and backend errors carry no mesh, while a finished mesh that misses
a limit travels with the error (`mesh()`, `report()`, `into_mesh()`). The
Python counterparts are `triangulate(strict=True)`, `triangulate_strict` and
`Mesh2DQualityError`, which keeps `mesh` and `report`.

A smallest angle up to about 30 degrees is a practical target. spade does not
promise termination above it; for targets above 30 degrees refinement takes the
shortest edges first and may insert an off-center instead of a circumcenter, and
a target of 34 degrees is reached on some inputs. Cells next to a small input
angle cannot reach the target, and the same holds next to kept segments.
Violations are never waived. The report counts them, and
`angle_diagnostics` gives one entry per cell with the reason refinement left
it (for example `input_corner_triangle` for the triangle at a corner of the
input, or `shared_input_corner_guard`). A diagnostic explains the decision made
on that triangle; it does not say that no better mesh exists.

At a corner between two boundary segments, refinement tries to move away from
the corner: the triangle at the corner is left alone, but its short edge does
not force another split of a segment still incident to the corner. An acute
angle between two interior interfaces keeps the conservative guard, because
relaxing it there made refinement run far past a sensible vertex count.

The pyGIMLi adapter offers `angle_policy="cap30"`, which maps requests above 30
degrees to 30 and records the requested and the effective angle, with the
violations counted for both, in `report["angle_mapping"]`. The default,
`"exact"`, passes the request through.

`keep_segments` lists input segments that refinement must not split: no vertex
is added on them, except where other segments cross them. `keep_boundary` keeps
every piece with the domain on one side only (outside, or a hole on the other).
This is the behaviour of Triangle's `-Y` switch, which pyGIMLi's
`preserveBoundary` uses, and it lets a mesh that is joined along that boundary
to another mesh match it node for node. Cells next to a kept edge may miss the
angle or area target, and the report counts them.

### Resource limits

`control::with_control(MeshControl, closure)` runs 2D and 3D meshing with
cooperative cancellation and limits. The scope is thread-local, nests, and is
restored on unwinding; a `MeshControl` can be cloned and cancelled from another
thread. Stopping returns `MeshError::Stopped(StopReason)`. Checkpoints run in
2D input insertion and at every refinement queue visit, and in 3D insertion,
rebuilds, flips, local refinement and smoothing.

| Limit | Default | Meaning |
|---|---|---|
| nodes | 100 000 | vertices of the working triangulation; the four frame vertices are not counted |
| cells | 2 000 000 | cells of the working complex, including exterior cells |
| iterations | 5 000 000 | refinement queue visits and 3D rounds |
| time | 120 s | elapsed time from the clock the caller supplies |
| output | 256 MiB | serialized output (app workers) |

Memory is not a limit. A caller that needs one runs meshing in a separate
process, which is what the desktop app does for every job (`rustmesh
--mesh-worker`), and the web app does with a terminable Web Worker.

### Geometry builder and templates

`Geometry2D` (`model.rs`) builds the PLC from a drawing and has nothing
application-specific in it: a domain outline with a marker per edge, shapes
(zones or holes; the later one wins where they overlap), lines and points. It
cuts pieces where they cross or touch, merges coinciding edges (the first
piece's marker wins, a zero marker gives way), drops what lies outside the
domain, and seeds every enclosed area with the zone its largest triangle's
centroid lies in.

The templates in `examples.rs` are built from these pieces: four general ones
(rectangle, plate with a hole, channel past a cylinder, L-shape) and seven for
geophysics and hydrology. The geophysics templates use pyGIMLi's markers: sensor
nodes -99, ground surface edges -1, other outer edges -2.

The surface ERT template sizes cells at `0.25 * spacing` around the electrodes,
growing by 0.3 per unit of distance. For strict forward calculations without
singularity removal, `0.15 * spacing` with growth 0.3 is more accurate and adds
about 45 % more cells; with singularity removal and a split forward mesh it
does not consistently help. The generic defaults are no point refinement and a
growth of 0.5.

## 3D mesher

See [mesh3d.md](mesh3d.md). It is experimental and independent of the 2D code.

## pyGIMLi

`rustmesh.pygimli.create_mesh(plc, quality, area, ...)` replaces
`pygimli.meshtools.createMesh` for a 2D PLC. pyGIMLi's `createParaMeshPLC`
still builds the geometry and `to_pygimli(mesh)` returns a `pg.Mesh` with the
same markers, so zones embedded as PLC edges keep working. pyGIMLi's usual
settings are cells marked 1 (outer region) and 2 and above (inversion region),
boundary markers -1 (ground surface, Neumann) and -2 (mixed), sensors -99.

Triangle's smoothing (`smooth=[2, 10]`) is not done; other keywords are
ignored. `electrode_size` and `size_growth` refine around the PLC's sensor
nodes. pyGIMLi's forward modelling expects the cell of an outline edge on its
left, so `to_pygimli` calls `fixBoundaryDirections`.

## Checks

### Tests

`cargo test -p rustmesh` and `pytest tests` check:

- the unit square meets its regional area and angle limits, with all markers in
  place;
- a concave surface with an enclosed zone and a hole meshes with no vertex in
  the air or the hole and the zone's area exact;
- refinement reaches a domain that touches the surface (pyGIMLi's ERT layout);
- cells grade from the refine points;
- crossing segments are split at the crossing, and kept segments get no new
  vertex;
- 120 random geometries (star-shaped, layered, L-shaped, hilly, sliver-thin,
  with overlapping zones, holes, crossing lines and points near edges, some at
  millimetre scale on UTM coordinates) mesh as drawn: every sample point lies in
  the zone the drawing says, every point is a node with its marker, every line
  runs along mesh edges, no cell is degenerate, every area limit is met, and a
  second run gives the same mesh;
- strict mode keeps the mesh and report on a budget failure or an unmet angle,
  and 32 and 34 degrees are met on a real ERT geometry;
- accepted refinement leaves the mesh locally constrained Delaunay;
- 3D meshes keep constraints, markers, material interfaces, electrodes and
  determinism, and reject invalid input without panicking;
- the Gmsh writers keep signed markers apart in 2D and 3D;
- the Python entry points: strict errors, rejected arguments, VTU and pyGIMLi
  conversion, the E4D files.

### ERT forward modelling

`examples/ert_forward_check.py` (needs pyGIMLi) builds pyGIMLi's ERT geometry for
48 electrodes 1 m apart, meshes it with Triangle and with rustmesh, runs
dipole-dipole (and for the half-space Wenner) readings, and compares them with
analytic or reference values. Every mesh is run two ways:

- strict: `ert.simulate(mesh, sr=False)`, the mesh as it is with no singularity
  removal, so the cells at the electrodes alone resolve the potential near each
  source;
- as inverted: `ert.simulate(mesh.createH2(), sr=True)`, which is what
  `ERTModelling()` does by default: every cell split in four, singularity removed.

Cases: flat ground over a 100 ohm-m half-space (analytic); 10 ohm-m over
100 ohm-m with the interface at 3 m (exact image-series solution); hilly ground
(z = 1.5 sin(x / 6)) against a Triangle mesh refined to about 21 000 cells with
singularity removal, which is within 0.005 / 0.08 / 0.33 % of the same run on a
mesh split once more.

Each entry below is the median / 95th percentile / maximum relative error in %,
dipole-dipole unless stated. "Triangle 34" is Triangle at 34 degrees with
`smooth=[2, 10]` and k nodes between neighbouring electrodes on the surface;
"rustmesh 28 e, g" is rustmesh at 28 degrees with a target edge of e electrode
spacings at each electrode, growing by g per unit of distance.

Strict:

| Mesh | Cells: flat / layered / hilly | Half-space | Wenner | 10 over 100 ohm-m | Hilly ground |
| --- | --- | --- | --- | --- | --- |
| Triangle 34, k = 1 | 1550 / 4240 / 1370 | 0.63 / 1.9 / 6.6 | 0.17 / 1.6 / 3.0 | 0.16 / 1.5 / 7.2 | 0.41 / 2.1 / 15 |
| Triangle 34, k = 4 | 2836 / 5815 / 2579 | 0.40 / 1.0 / 2.0 | 0.17 / 0.62 / 1.2 | 0.099 / 0.70 / 2.6 | 0.72 / 1.4 / 2.5 |
| Triangle 34, k = 6 | 3742 / 6844 / 3583 | 0.31 / 0.82 / 2.0 | 0.13 / 0.34 / 1.1 | 0.12 / 0.64 / 1.7 | 0.51 / 1.2 / 2.5 |
| Triangle 28 | 475 / 920 / 492 | 1.6 / 7.3 / 44 | 0.58 / 8.4 / 21 | 0.68 / 8.9 / 46 | 1.8 / 7.9 / 51 |
| rustmesh 28 | 788 / 1333 / 809 | 1.3 / 8.0 / 51 | 0.90 / 12 / 24 | 0.61 / 8.3 / 55 | 1.3 / 6.6 / 61 |
| rustmesh 28, 0.25, 0.5 | 1458 / 2137 / 1458 | 0.39 / 2.2 / 12 | 0.19 / 3.5 / 6.3 | 0.23 / 2.6 / 13 | 0.43 / 2.0 / 11 |
| rustmesh 28, 0.25, 0.3 | 2698 / 3177 / 2718 | 0.13 / 0.69 / 3.4 | 0.099 / 0.61 / 1.4 | 0.098 / 0.65 / 2.5 | 0.14 / 0.57 / 2.9 |
| rustmesh 28, 0.15, 0.3 | 3870 / 4427 / 3976 | 0.11 / 0.42 / 1.8 | 0.13 / 0.34 / 0.81 | 0.089 / 0.36 / 2.3 | 0.094 / 0.37 / 1.8 |

As inverted (cells before the split):

| Mesh | Cells: layered / hilly | 10 over 100 ohm-m | Hilly ground |
| --- | --- | --- | --- |
| Triangle 34, k = 1 | 16960 / 5480 | 0.072 / 0.22 / 0.37 | 0.012 / 0.16 / 0.38 |
| Triangle 34, k = 4 | 23260 / 10316 | 0.050 / 0.21 / 0.31 | 0.007 / 0.071 / 0.37 |
| Triangle 28 | 3680 / 1968 | 0.34 / 1.4 / 2.2 | 0.019 / 0.30 / 1.5 |
| rustmesh 28 | 5332 / 3236 | 0.25 / 0.73 / 1.2 | 0.013 / 0.12 / 1.3 |
| rustmesh 28, 0.25, 0.3 | 12708 / 10872 | 0.070 / 0.16 / 0.25 | 0.010 / 0.14 / 0.41 |
| rustmesh 28, 0.15, 0.3 | 17708 / 15904 | 0.071 / 0.19 / 0.26 | 0.009 / 0.14 / 0.39 |

On the flat half-space every mesh gives the same 0.001 / 0.24 / 0.30 %
(dipole-dipole) as inverted: the primary field of the singularity removal is
then the analytic answer, and what remains is pyGIMLi's 2.5D wavenumber
integration.

Reading the tables, at about the same number of cells:

- Strict, dipole-dipole: rustmesh's median error is about 3 times smaller on
  flat ground and 5 times smaller on hilly ground than Triangle with 4 to 6
  surface nodes, and its 95th percentile 1.5 to 3.4 times smaller. Its maximum
  is larger at about 2 700 cells (3.4 against 2.0 % on flat ground) and smaller
  at about 3 900 cells.
- Strict, Wenner: about equal.
- Strict, layered: rustmesh needs fewer cells, because the interface adds more
  cells to Triangle's meshes.
- As inverted, the difference at the electrodes is gone: every mesh refined at
  the electrodes stays within 0.42 % at its worst reading, and on hilly ground
  the meshes cannot be ranked within the reference's own uncertainty.
- At the cell count of Triangle's default (k = 1) the strict test fails for
  both meshers, with worst readings above 6 %. The worst readings sit in the
  shortest dipoles, whose potential is dominated by the few cells around each
  electrode.
- Without electrode refinement, rustmesh and Triangle at the same angle are in
  the same class.

### Convergence on manufactured solutions

On the unit square, four meshes per problem (largest area 0.02, 0.005, 0.00125
and 0.0003125, 28 degrees), a P1 Galerkin solver gives these results for the
Poisson problem `-Δu = 2π² sin(πx) sin(πy)` and for diffusion with
`k = 1` for `x < 0.5` and `k = 10` for `x > 0.5` (cell markers 11 and 22, a
fitted interface):

| Measurement | Poisson | Two-material diffusion |
|---|---:|---:|
| L2 error reduction, coarsest to finest | 62.8 | 70.2 |
| H1 seminorm error reduction | 7.95 | 8.35 |
| Observed L2 slopes | 1.87 to 1.95 | 1.87 to 2.07 |
| Observed H1 slopes | 0.94 to 0.98 | 0.94 to 1.04 |
| Finest relative L2 error | 0.081 % | 0.173 % |

Swapping the two conductivities raised the finest L2 error 469 times, so the
setup does detect a wrong material mapping.

### Speed

Meshing a surface ERT line (release build, one Windows laptop):

| Electrodes | Cells | Time |
|---|---|---|
| 500 | 23 046 | 0.02 s |
| 1 000 | 45 188 | 0.04 s |
| 2 000 | 89 712 | 0.09 s |
| 4 000 | 177 864 | 0.21 s |
| 8 000 | 354 996 | 0.55 s |

## Limits

- Cells below the angle target remain possible next to small input angles and
  next to kept segments. In some of the random geometries a share of the cells
  below the target lie away from small input angles: refinement can insert a
  circumcenter on the far side of an interior constraint edge when that point
  does not encroach the edge, and the skinny cell then stays. Shewchuk's
  Terminator and Chew's second algorithm are the known remedies and are not
  implemented.
- A small feature next to a high angle target can use up the vertex budget
  (angle-driven over-refinement); the report says the budget ran out.
- Smoothing is not done. A prototype that moves free vertices towards the
  circumcentres of their cells cut the cells under 30 degrees by about 90 % but
  made the worst strict reading worse in some cases, so it is not included.
- 3D is experimental.

## References

- J. Ruppert, A Delaunay refinement algorithm for quality 2-dimensional mesh
  generation, Journal of Algorithms 18(3), 548-585, 1995.
- L. P. Chew, Guaranteed-quality triangular meshes, Technical Report TR-89-983,
  Cornell University, 1989.
- J. R. Shewchuk, Adaptive precision floating-point arithmetic and fast robust
  geometric predicates, Discrete & Computational Geometry 18, 305-363, 1997
  (the `robust` crate).
- J. R. Shewchuk, Delaunay refinement algorithms for triangular mesh generation,
  Computational Geometry 22, 21-74, 2002.
- A. Rand, Where and how Chew's second Delaunay refinement algorithm works,
  23rd Canadian Conference on Computational Geometry, 2011.
- A. Üngör, Off-centers: a new type of Steiner point for computing size-optimal
  quality-guaranteed Delaunay triangulations, LATIN 2004, 152-161; Computational
  Geometry 42(2), 109-118, 2009.
- S. Altmayer, spade, https://github.com/Stoeoef/spade.

The 3D references are in [mesh3d.md](mesh3d.md#references).
