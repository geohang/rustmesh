# rustmesh Mesh Studio (desktop and web)

A general 2D geometry and mesh builder: draw a domain, the zones and holes
inside it, the lines the mesh must follow and the points that must be nodes,
and get a quality triangle mesh with every part marked. Templates start common
problems, from a plain rectangle or a plate with a hole to geophysical surveys,
a pumped aquifer and a watershed.

One app, two ways in:

| | Web app | Desktop app |
|---|---|---|
| Start | open a web page | run one program, no install |
| Meant for | trying things, teaching, designing a geometry, small and medium meshes | large meshes, long runs, batches (command line) |
| Where it computes | in the browser on the user's computer (WebAssembly in a Web Worker); nothing is uploaded | natively, in an isolated worker process |
| Stop | terminates the worker | terminates the worker process and waits for its exit |
| Default node budget | 500 000 (can be raised) | 5 000 000 |
| Files | browser file picker and downloads | system file dialogs |

Both run the same Rust code: the same mesher, the same geometry builder, the
same window. A project saved in one opens in the other.

The browser is not slow at this: the surface ERT template refined to about 1.9
million cells meshed in 2.5 s in the web app against 2.2 s on the desktop (see
"Measured" below). The web app's limits are memory (WebAssembly has at most
4 GB) and handing the finished mesh back to the page, which is done as text and
pauses the page briefly for meshes over a million cells. The mesher is serial in
both; the desktop's edge is memory, file access and batches, not speed.

## Workspace

- **Templates** chooses an example and its parameters. **Use template** replaces
  the geometry and opens the geometry editor.
- **Geometry** holds the domain, zones, holes, lines and point tables.
- **Mesh** holds the resolution settings and the quality report of the last
  build.
- **Build mesh** stays at the bottom of the inspector, outside the scrolling
  form. Invalid coordinate text disables it until the entry is corrected.
- **Open**, **Save**, **Export** and **Appearance** (light, dark or the system
  setting) are in the top toolbar.
- The canvas header shows the mesh counts and whether edits need a rebuild.
  **View options** has edges, outlines and vertical exaggeration. Scroll to
  zoom, drag to pan, double-click to fit the geometry.

## 3D workspace

Use the **2D / 3D** switch beside the app name. Each workspace keeps its
geometry, settings and last mesh while you switch. The 3D workspace offers
**Block**, **Layered ground**, **Sphere** and **Torus** templates. Change the
dimensions or surface resolution, then choose **Use 3D template**. Layered
ground has two marked volumes and a conforming internal interface.

**Build 3D mesh** calls the Rust tetrahedral mesher in a background thread or a
browser Web Worker. The mesh settings are the maximum cell volume, the minimum
dihedral target, the node budget and the pass budget. The report lists unmet
volume and angle targets, slivers, volume and elapsed time. The 3D mesher is
experimental (see [mesh3d.md](mesh3d.md)); its warnings are under **Mesher
notes**.

- Drag to rotate, right-drag to pan, scroll or pinch to zoom, double-click to
  fit.
- **Camera** gives isometric, top, front and side views.
- **Cut X** keeps whole tetrahedra by their centroids and shows their boundary
  faces. This is a stepped cell cut, not an interpolation on a plane.
- Colour the surface by cell region or minimum dihedral angle; toggle edges.
- **Geometry, Edit geometry (JSON)** edits vertices, triangles, region seeds,
  cavity seeds and their markers. Apply valid edits before building or saving.
  Triangle indices are zero-based and outer shells must close.
- **Open** and **Save** use a versioned `rustmesh-project-3d` JSON document with
  `plc`, `options`, `name` and `units`. Opening either project format selects its
  workspace. The command line project commands work on 2D projects.
- **Export** writes tetrahedral `.vtu` or Gmsh 2.2 `.msh` files, including the
  recovered boundary triangles. VTU keeps signed node, cell and triangle markers;
  Gmsh physical tags use absolute marker values, as in 2D.

Stop terminates the browser worker. On the desktop it discards the request while
the thread finishes in the background. Changing settings during a build marks its
result as needing a rebuild, and loading a 3D project discards a pending result.

## What you build

A 2D geometry in `(x, y)`, in a length unit you name (it labels the axes and is
saved with the project; the mesh is not scaled). For a cross-section through the
ground, y is elevation. The geometry is made of:

- **Domain**: a closed outline, each edge with its own marker (for a boundary
  condition, say). Make a rectangle, edit the outline as rows
  `x, y, edge marker`, or import a table.
- **Zones**: every part with cells has a name, a cell **marker** and a largest
  cell area. The background zone fills the domain. Each **shape** (a closed
  outline: rectangle, circle or any polygon) is a zone of its own or a **hole**
  with no cells; where shapes overlap, the one listed later wins. A shape's
  outline can carry an edge marker too.
- **Lines**: open polylines the mesh follows without them changing any zone:
  cracks, faults, boreholes, well screens, a path to sample along. Their edges
  carry a marker.
- **Points**: always mesh nodes, with a node marker, and by default with cells
  refined round them: sensors, sources, corners where a solution is singular.
  Add a row of evenly spaced points, edit them as rows
  `x, y[, marker[, refine]]` (refine 0 makes a node without refining), or import
  a table.
- **Mesh size**: smallest cell angle, cell size at the refined points and how fast
  it grows away from them, largest cell anywhere, most nodes.

Pieces that cross or touch are cut where they meet and coinciding edges merge:
the domain's edges keep their marker over a shape's, a shape's over a line's, and
a zero marker gives way to any other. Anything outside the domain is dropped
(points outside it get a warning), and every enclosed area gets the marker and
cell size of the zone it lies in.

Before meshing, the view shows each zone tinted and holes empty; hovering shows
the coordinates and the zone under the pointer. "Fit details" zooms to the points
and the small pieces when the domain is much larger than them, as for the
magnetotelluric boundary 100 km away.

## Templates

A template builds a whole geometry from a few parameters (the command line name
and the parameters are in brackets). Pick the field, then the template; the
geometry can be edited freely afterwards.

| Field | Template | What it shows |
|---|---|---|
| General | Rectangle (`rectangle`: width, height) | an empty rectangle, edges marked 1 to 4: a start for anything |
| General | Plate with a hole and inclusions (`plate-with-hole`: width, height, radius) | a round hole and two inclusions of other materials, as for heat or stress problems |
| General | Channel past a cylinder (`channel`: length, height, radius) | inlet, outlet, walls and cylinder marked apart, cells refined round the cylinder, as for flow problems |
| General | L-shaped domain (`l-shape`: size, corner) | cells refined at the inner corner, where solutions are singular |
| Geophysics | Surface ERT line (`surface-ert`: electrodes, spacing, hill) | electrodes on a low hill, inversion region and outer region as pyGIMLi lays them out |
| Geophysics | Crosshole tomography (`crosshole`: sensors, spacing, distance) | sensors in two boreholes (ERT, seismic or radar), inversion region between |
| Geophysics | Seismic refraction line (`refraction`: geophones, spacing) | geophones on sloping ground, mesh only under the line (traveltime tomography) |
| Geophysics | Gravity or magnetics body (`gravity-magnetics`: stations, spacing, overburden) | stations over overburden, bedrock and a dipping ore body in a wide domain |
| Geophysics | Magnetotelluric profile (`magnetotelluric`: stations, spacing, air) | stations on a ridge, air above, conductor and basement, boundary far away |
| Hydrology | Aquifer with a pumping well (`aquifer-well`: aquifer, clay, screen_top, screen_bottom) | soil, aquifer and clay layers, a well with its screen marked, an ERT line |
| Hydrology | Watershed (`watershed`: width, length, river_size) | plan-view catchment, marked divide and outlet, branching streams and a riparian zone with local refinement |

The watershed is an illustrative plan-view catchment, not a delineation from an
elevation model and not a hydrology solver. Width and length are in metres, and
`river_size` sets the local stream refinement, limited to 0.2 to 10 % of the
smaller basin dimension. Cell markers are 1 (uplands) and 2 (riparian zone); edge
markers are 1 (divide), 2 (outlet), 10 (main stream), 11 (tributaries) and 12
(riparian boundary); the outlet node is 20. The markers are labels for the
receiving solver; no boundary conditions or discharge values are assigned.

## Markers

Every marker is yours to set: zone markers become cell markers, domain edges,
shape outlines and lines give edge markers, points give node markers.

The general templates mark edges with small positive numbers (the channel: inlet
1, outlet 2, walls 3, cylinder 4). The geophysics and aquifer-well templates
follow pyGIMLi: sensors are nodes with marker -99, the ground surface edges -1
(no-flow, homogeneous Neumann), the sides, bottom and air top -2 (mixed). The
surface ERT and crosshole templates lay out pyGIMLi's outer region (cell marker 1)
and inversion region (2); in the others each material has its own cell marker,
and the magnetotelluric air is 0.

## Files

- **Project** (`*.rmesh.json`): the geometry, mesh sizes, node budget and the
  rustmesh version that wrote it, not the mesh itself. Open it in either app or
  mesh it on the command line. Projects saved before the general geometry
  (format version 1) are not read; create them again from a template.
- **ParaView** (`.vtu`): cells with marker, region, area and smallest angle;
  marked edges as lines; node markers. Every marker is kept as it is.
- **pyGIMLi and Gmsh** (`.msh`, Gmsh 2.2): read with
  `pygimli.meshtools.readGmsh`. Gmsh numbers must be positive, so the file follows
  pyGIMLi's reading of them. A node with a negative marker is written as a point
  with the number negated (sensors as 99, read back as -99); a positive node
  marker is written as a point with its absolute number, which pyGIMLi reads as a
  negative node marker. Edges are written as lines with the absolute value of
  their marker, and pyGIMLi reads lines 1 to 4 back as -1 to -4 (its
  boundary-condition markers) and other numbers as they are, except tags of 10000
  and above, which its CEM rule also negates. So for pyGIMLi, give sensor-like
  points negative markers, and use edge markers from 5 for edges that are not
  boundary conditions.

  Both the 2D and the 3D writer also store the original signed markers in
  standard `$NodeData` (`rustmesh:node_marker`) and `$ElementData`
  (`rustmesh:element_marker`), indexed by the file's node and element IDs, so
  opposite signs and large values stay recoverable. Readers that only use
  physical tags, `readGmsh` among them, do not restore these values; use `.vtu`
  or read the scalar datasets for exact labels. The layout follows the
  [Gmsh MSH specification](https://gmsh.info/doc/texinfo/gmsh.html#MSH-file-format-version-2-_0028Legacy_0029).

## Command line

The desktop build also has `rustmesh`, for scripts and batches:

```bash
rustmesh templates
rustmesh template surface-ert electrodes=21 spacing=2 -o ert.rmesh.json
rustmesh mesh ert.rmesh.json -o ert.msh
```

`templates` lists every template with its field and its parameters and their
defaults. `template` writes a template as a project, with the parameters given as
`key=value` and the defaults for the rest. `mesh` prints the node and cell
counts, the smallest angle and anything not met, and exits with an error when a
limit was not met (the mesh is written anyway).

A mesh is complete when refinement finished within its budget and no cell is
below the angle or over the area limit (angles with a 1e-6 degree tolerance,
areas with a 1e-9 relative one); see [design.md](design.md). Small input angles
can force cells below the target, and they are not excused. A mesh can be
returned successfully and still be incomplete, which also affects Python's
`create_mesh(strict=True)` and the app's report.

## Building

Desktop app and command line:

```bash
cargo run --release -p rustmesh-app --bin rustmesh-app
cargo build --release -p rustmesh-app --bin rustmesh
```

Web app, into `dist/web`, then serve that folder (any static server; module
workers need it served over http, not opened as a file):

```bash
rustup target add wasm32-unknown-unknown
cargo xtask web
python -m http.server 8000 -d dist/web
```

`cargo xtask web` runs wasm-bindgen as a library, so no wasm-bindgen command-line
tool and no C compiler is needed.

On Windows with rustup's GNU toolchain and no full MinGW (no Visual Studio),
three things matter; the MSVC toolchain needs none of them.

- A `dlltool` for the windows crates. LLVM's archiver is one when called by that
  name: `rustup component add llvm-tools`, copy `llvm-ar.exe` from `rustc --print
  sysroot`'s `lib/rustlib/x86_64-pc-windows-gnu/bin` to a folder as `dlltool.exe`,
  and put that folder on `PATH` for the build.
- The LLVM linker, which `.cargo/config.toml` selects for this target. With GNU
  ld, the import libraries several crates bring for the same DLL end up partly
  behind another one's terminator, the loader leaves those imports unbound, and
  the app crashes before `main` (an access violation inside `.idata`). A
  `RUSTFLAGS` variable replaces the config's flags, so leave it unset for the
  app. Cargo reads `.cargo/config.toml` from the current folder upwards, not from
  the manifest's, so run cargo from inside the repository: `cargo build
  --manifest-path` from elsewhere links with GNU ld and gives the same crash.
- If the linker reports `unable to find library -lshlwapi`, put a `libshlwapi.a`
  import library in a folder of its own and set `RUSTMESH_WINDOWS_LIB_DIR` to that
  folder; `crates/rustmesh-app/build.rs` adds it to the link search path. Adding a
  whole MinGW library folder instead shadows the bundled C runtime and fails.

## Measured

On a Windows laptop (release builds, Chrome-based browser):

| | Desktop | Web |
|---|---|---|
| Download | `rustmesh-app.exe` 8.9 MB, `rustmesh.exe` 1.0 MB | 6.0 MB of WebAssembly (before compression) |
| Each template (700 to 4 000 cells) | under 0.01 s | about 0.01 s |
| Surface ERT refined to 1.9 million cells | 2.2 s meshing | 2.5 s meshing, plus about 0.5 s handing the mesh to the page |

The same project file gives the same mesh in both: the same node and cell counts
and the same coordinates, triangles and cell markers to the last bit (checked on
the 1.9 million cell ERT mesh, and on the magnetotelluric and channel templates).
Floating-point functions whose last bits differ between a browser and a native
build are kept out of the meshing (see [PATCHES.md](../vendor/spade/PATCHES.md)).
The templates themselves use `sin` and `cos` for topography and outlines, so build
a template once and save it as a project when meshes must match exactly.

The pyGIMLi export was checked end to end: the surface ERT template with flat
ground (`hill=0`), written as `.msh` by the command line and read with
`pygimli.meshtools.readGmsh`, has its 41 electrodes as nodes with marker -99, cell
markers 1 and 2, surface edges -1 and outer edges -2. Over a 100 ohm-m half-space
it gives apparent resistivities within 0.14 % (median), 0.9 % (95th percentile)
and 3.1 % (worst) of the analytic values for dipole-dipole, and within 1.6 %
(worst) for Wenner, without singularity removal (the strict test in
[design.md](design.md#ert-forward-modelling)).

## Distribution

The web app is a static site (the `dist/web` folder), so GitHub Pages can host
it; no server computes anything. The desktop programs are built per platform with
cargo. Unsigned programs get a SmartScreen warning on Windows and are blocked by
Gatekeeper on macOS until they are signed and notarized, and managed computers
may not run downloaded programs at all; the web app works there.

## Shared workspace, MCP and agent panel

The native app keeps the project in a revisioned workspace that the canvas, the
command line and 12 MCP tools share. A tool edit reaches the canvas; undo creates
a new revision, and earlier meshes stay tied to the geometry they were built from.
`rustmesh mcp --workspace <directory>` serves the tools over stdio, and
`rustmesh workflow` and `rustmesh replay` save and replay a workflow without any
model login. The **Agent & workspace** panel connects Codex to the same workspace
after Sign in with ChatGPT; see [agent.md](agent.md).

Set `RUSTMESH_WORKSPACE` to choose the workspace directory before launch;
otherwise the app uses its local data directory. One process owns each workspace,
and other clients attach to it with an authenticated address instead of opening
its files. Version 2 project files can still be imported. The web app meshes on
its own and does not use the workspace.
