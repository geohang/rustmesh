# E4D mesh export

`rustmesh.e4d.write_e4d` writes a recovered tetrahedral mesh in the
[PNNL electrical mesh-file formats](https://e4d-userguide.pnnl.gov/e4d_guide/elec/e4d_e4d-mesh-files.html).
It does not invoke TetGen, Gmsh or E4D, and needs only NumPy in addition to
rustmesh. The files are mesh inputs, not a complete survey or inversion run
folder.

```python
from rustmesh.e4d import write_e4d

# mesh is a Mesh3D. In this example the PLC uses -1 for the ground surface
# and -2 for far-field walls/bottom. Supply your own marker sets if different.
export = write_e4d(
    mesh, "e4d_mesh", name="site",
    surface_markers=(-1,),
    dirichlet_markers=(-2,),
    conductivity=0.01,  # S/m; scalar or one value per cell; optional
)
print(export["node_file"])  # site.1.node, relative to e4d_mesh/
```

## Conventions

- Nodes, cells, faces and neighbor references are numbered from 1. Exterior
  neighbors remain -1. Neighbor slot k is opposite tetrahedron vertex k.
- `.node` includes one constant attribute and a boundary flag. Exterior
  markers map to no-flux flag 1 or zero-potential flag 2, as the marker sets you
  pass say. A node touching both gets flag 2. Interior nodes get 0. Geometry is
  not used to guess a boundary condition. Unknown exterior markers are rejected,
  as is a domain without any Dirichlet boundary.
- `.ele` preserves cell order and maps sorted material markers to consecutive
  zone IDs starting at 1. The returned `zone_of_marker` records this mapping.
- `.face` includes exterior and marked interface triangles. Exterior flags
  are 1/2; interface marker IDs are mapped to consecutive IDs from 3. Their
  mapping and original markers are recorded in the sidecar.
- `.neigh` is reconstructed independently from the cells and checked against
  the mesh's neighbor table before writing.
- Node coordinates are written minus `translation` (default: mean node
  position); `.trn` contains that translation. Coordinates use 17 significant
  digits. Cells must remain positive after translation.
- Optional `.sig` stores positive finite conductivity, in original cell
  order. It is not a resistivity file; 100 ohm-m corresponds to 0.01 S/m.
- `.rustmesh.json` preserves original node markers (including electrodes),
  input-node mapping, facet ancestry, face markers, region IDs, zone mapping,
  and the mesh report. Do not use E4D's boundary flags as electrode IDs.

Names must begin with an ASCII letter and contain at most 26 letters, digits,
underscores or hyphens. This avoids E4D's filename length/path/dot ambiguity.
Existing files require `overwrite=True`. A mesh that misses its quality limits
needs `allow_incomplete=True`, which does not skip the exporter's geometry checks.
Argument checks and payload preparation come before any file is created. An I/O
failure can still leave partial files; a multi-file export is not a filesystem
transaction.

## Tests and open checks

`pytest tests/test_e4d.py` checks the file indices and layouts, neighbour slots,
boundary flags, zones, conductivity, the provenance file, and that bad arguments
are rejected before any file is written.

No E4D forward solve has been run on these files. A mesh round trip does not
validate the accuracy of an ERT calculation, electrode interpolation, convergence
or reciprocity. With E4D available, the checks to make are:

1. Add a survey, conductivity and ERT2 forward-run configuration to an export.
2. Solve a homogeneous half-space and compare the transfer resistances with the
   analytic solution, noting the errors from the finite domain and the
   discretization.
3. Swap current and potential electrode pairs and check reciprocity.
4. Repeat with refinement and larger outer domains, then add layered and
   topographic cases.

The numeric marker `-2` has different meanings across adapters: in pyGIMLi it is
a mixed boundary condition, whereas the E4D example above maps it to zero
potential (Dirichlet). Equal marker values do not imply equal physics, and the
outer domain must be large enough for the comparison to converge.
