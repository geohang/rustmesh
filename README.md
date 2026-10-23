# rustmesh

A mesh generator written in Rust, with Python bindings and a model builder app.
It meshes planar straight-line graphs and 3D piecewise-linear complexes with
quality constraints: the geometry you draw (outlines, zones, holes, lines,
points) goes in, and triangles with the markers you set come out. The 2D mesher
is the main part; 3D tetrahedral meshing is experimental.

Pre-alpha. Licensed under MIT OR Apache-2.0.

Website: <https://geohang.github.io/rustmesh/>. Mesh Studio runs there in the
browser, with no install: <https://geohang.github.io/rustmesh/app/>.

## Features

- 2D meshing by constrained Delaunay triangulation and Delaunay refinement
  (Ruppert; Chew; references in [docs/design.md](docs/design.md)), on the
  vendored [spade](vendor/spade/PATCHES.md) kernel. Segments, vertices, regions
  with their own marker and largest cell area, and holes are kept, and segments
  that cross are split where they cross.
- Quality targets: a smallest angle and a largest area per region, cell sizes that
  grow away from chosen points, and segments that refinement must not split
  (`keep_segments`, `keep_boundary`).
- Every mesh comes with a report that says whether refinement finished and whether
  each requested limit holds. The strict entry points return an error that still
  carries the mesh and its report when a limit is missed.
- `Geometry2D` turns a drawing (a domain with marked edges, zone and hole shapes,
  lines, points) into a mesh input: pieces are cut where they cross, coinciding
  edges merge, and every enclosed area is seeded.
- Experimental 3D tetrahedral meshing of a triangulated complex, with material
  regions, holes, volume limits and a minimum dihedral target.
- Exports to VTU for ParaView, Gmsh 2.2 (2D and 3D), E4D mesh files and
  `pygimli.Mesh` objects.
- Node, cell, iteration, time and output limits with cancellation
  (`rustmesh::control`).
- A model builder for the desktop and the browser, with templates, a command line
  and an MCP server. See [docs/app.md](docs/app.md).
- The same input gives the same mesh every time, and the browser build gives the
  same mesh as the native build.

## Installation

The Python package builds from source with [maturin](https://www.maturin.rs/) and
needs a Rust toolchain (1.83 or newer) and Python 3.10 or newer:

```bash
git clone https://github.com/geohang/rustmesh
cd rustmesh
pip install .                      # build and install the package
# for development, in a virtual environment:
pip install maturin numpy
maturin develop --release
```

On Windows with rustup's GNU toolchain (no Visual Studio),
`scripts/develop-windows-gnu.ps1 -Python <path to python.exe>` builds the
extension into that environment.

The Rust crate is `crates/rustmesh`. Use it as a path or git dependency; enable
the `serde` feature to save and load geometries as JSON.

## Python

```python
import rustmesh

# A 10 x 5 box with a square zone on the left and a square hole on the right.
vertices = [[0, 0], [10, 0], [10, 5], [0, 5],        # box
            [1, 1], [3, 1], [3, 4], [1, 4],          # zone
            [6, 1.5], [8, 1.5], [8, 3.5], [6, 3.5]]  # hole
loop = lambda start: [[start + i, start + (i + 1) % 4] for i in range(4)]
mesh = rustmesh.triangulate_strict(
    vertices,
    loop(0) + loop(4) + loop(8),
    segment_markers=[1] * 4 + [2] * 4 + [3] * 4,
    regions=[[0.5, 0.5, 1, 0.5], [2, 2.5, 2, 0.02]],  # x, y, marker, largest area
    holes=[[7, 2.5]],
    min_angle=28,
)
print(len(mesh.nodes), len(mesh.triangles), mesh.report["complete"])
```

`mesh` holds NumPy arrays: `nodes`, `triangles`, `cell_markers`, `edges` with
`edge_markers`, and `input_vertex_nodes`, which maps your vertices to nodes.
`rustmesh.triangulate` returns the mesh even when a limit is missed and leaves
the check to `mesh.report`; `tetrahedralize` is the 3D counterpart.

## Rust

```rust
use rustmesh::model::{ellipse, rectangle};
use rustmesh::{Domain, Geometry2D, Shape, Zone};

let mut geometry = Geometry2D {
    domain: Domain::rectangle(0.0, 0.0, 10.0, 5.0),
    ..Geometry2D::default()
};
// A zone with its own marker and finer cells, and a round hole.
geometry.shapes.push(Shape {
    zone: Zone::new("Inclusion", 2, 0.02),
    outline: rectangle(1.0, 1.0, 3.0, 4.0),
    ..Shape::default()
});
geometry.shapes.push(Shape {
    zone: Zone::new("Hole", 0, 0.0),
    outline: ellipse([7.0, 2.5], 1.0, 1.0, 24),
    hole: true,
    ..Shape::default()
});
let mesh = geometry.mesh(100_000)?; // at most 100 000 vertices
println!("{} cells, complete: {}", mesh.triangles.len(), mesh.report.complete);
```

`rustmesh::triangulate` and `rustmesh::triangulate_strict` take a `Plc`
(vertices, segments, regions, holes) and `Options` directly.

## The app

The model builder is a desktop program and a web page
([online](https://geohang.github.io/rustmesh/app/)). The desktop app also has an
optional agent panel (Sign in with ChatGPT) that drives the same project through
MCP tools.

```bash
cargo run --release -p rustmesh-app --bin rustmesh-app   # desktop app
cargo xtask web                                          # web app into dist/web
python -m http.server 8000 -d dist/web                   # then open localhost:8000
```

`rustmesh --help` lists the command line (`templates`, `template`, `mesh`, `mcp`).
See [docs/app.md](docs/app.md) for the workspace, templates and files, and
[docs/agent.md](docs/agent.md) for the agent panel.

## Status and limits

- 2D meshing is the more mature part. Angle targets up to about 30 degrees are
  practical; spade does not promise termination above that, and cells next to
  small input angles or kept segments can miss the target. The report counts
  them.
- 3D meshing is experimental: constraint recovery is bounded, sliver removal is
  best effort, and there is no performance work yet. See
  [docs/mesh3d.md](docs/mesh3d.md).
- The design and its checks are in [docs/design.md](docs/design.md); E4D export
  is in [docs/e4d.md](docs/e4d.md).

## Used with

rustmesh was written for geophysical inversion. `rustmesh.pygimli.create_mesh`
can replace Triangle as pyGIMLi's 2D mesher, and
[PyHydroGeophysX](https://github.com/geohang/PyHydroGeophysX) can use rustmesh as
its mesher. The templates for electrical resistivity, crosshole, refraction,
gravity and magnetics, magnetotelluric, aquifer and watershed models follow
pyGIMLi's marker conventions.

## Tests

```bash
cargo test -p rustmesh --features serde
cargo test -p rustmesh-app --lib agent
pip install pytest && pytest tests      # after maturin develop
```

## Licence

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. The vendored copy of spade (`vendor/spade`) is under the same two
licences, with the changes listed in [vendor/spade/PATCHES.md](vendor/spade/PATCHES.md).
Third-party references are in [docs/design.md](docs/design.md#references).
