"""Does an ERT forward run on a rustmesh mesh give the right numbers?

Builds pyGIMLi's ERT parameter-mesh geometry (``createParaMeshPLC``) and
meshes it with Triangle as PyHydroGeophysX calls it (34 degrees, smoothed),
also with 2 to 6 surface nodes between neighbouring electrodes (``addNodes``,
PyHydroGeophysX's ``surface_nodes``, which refine a Triangle mesh at the
electrodes), with Triangle at rustmesh's angle (28 degrees), and with rustmesh
(28 degrees, with and without electrode refinement). Each mesh is run twice:

- strict: ``ert.simulate(mesh, sr=False)``, the mesh as it is with no
  singularity removal, so the cells at the electrodes alone resolve the
  potential near each source. The hardest test of a mesh.
- as inverted: ``ert.simulate(mesh.createH2(), sr=True)``. pyGIMLi's
  ``ERTModelling()``, which PyHydroGeophysX inverts with, splits every cell
  of the forward mesh in four and removes the singularity by default.

Cases, all dipole-dipole (case 1 also Wenner):

1. flat ground, 100 ohm-m half-space, against the analytic values;
2. flat ground, 10 ohm-m over 100 ohm-m with the interface at 3 m in the
   geometry, against the exact two-layer solution (image series);
3. hilly ground, 100 ohm-m, against a reference: Triangle at 34 degrees on the
   same geometry with the inverted region's cells at most 0.1 m^2 and 4
   surface nodes between electrodes (about 21 000 cells), with singularity
   removal. On topography pyGIMLi computes the primary field of the
   singularity removal with quadratic elements, so this reference is within
   0.005 / 0.08 / 0.33 % (median / 95th / max) of the same run on that mesh
   split once more (84 000 cells; not run here, it peaks at 4 GB).

Memory: one mesh is alive at a time; the run peaks at about 1 GB.

Run:  python -u examples/ert_forward_check.py
"""

import gc
import time

import numpy as np
import pygimli as pg
import pygimli.meshtools as mt
from pygimli.physics import ert

import rustmesh.pygimli as rp
from rustmesh import __version__ as rustmesh_version

pg.setLogLevel(30)
N_ELEC, SPACING = 48, 1.0
INTERFACE_DEPTH, TOP_RES, BOTTOM_RES = 3.0, 10.0, 100.0
REFERENCE_AREA, REFERENCE_NODES = 0.1, 4


def sensors(topography=None):
    x = np.arange(N_ELEC) * SPACING
    z = np.zeros_like(x) if topography is None else topography(x)
    return np.c_[x, z]


def hill(x):
    return 1.5 * np.sin(x / 6.0)


def scheme_for(positions, name):
    """A measurement scheme with its analytic geometric factors set, so
    ``ert.simulate`` does not compute them (numerically on topography: one
    more forward run per call). The transfer resistances do not depend on
    them."""
    scheme = ert.createData(elecs=positions, schemeName=name)
    scheme.set("k", np.array(ert.createGeometricFactors(scheme, numerical=False), dtype=float))
    return scheme


def electrodes(scheme):
    return [np.array(scheme[c], dtype=int) for c in "abmn"]


# Geometries. ``add_nodes`` is ``createParaMeshPLC``'s ``addNodes``: 1 (the
# default) puts one node midway between neighbouring electrodes, k > 1 puts
# k nodes there. ``para_area`` caps the inverted region's cell area (0: none).

def flat_geometry(add_nodes=1, para_area=0.0):
    return mt.createParaMeshPLC(sensors(), addNodes=add_nodes, paraMaxCellSize=para_area)


def hilly_geometry(add_nodes=1, para_area=0.0):
    return mt.createParaMeshPLC(sensors(hill), addNodes=add_nodes, paraMaxCellSize=para_area)


def layered_geometry(add_nodes=1, para_area=0.0):
    plc = flat_geometry(add_nodes, para_area)
    line = mt.createLine(start=[plc.xmin(), -INTERFACE_DEPTH], end=[plc.xmax(), -INTERFACE_DEPTH])
    plc = mt.mergePLC([plc, line])
    # The interface cuts the inverted region in two and the outer one in
    # three; seed the new parts too (the inverted one with the same area limit).
    plc.addRegionMarker(pg.Pos(0.5 * (N_ELEC - 1) * SPACING, -0.5 * INTERFACE_DEPTH), 2, para_area)
    plc.addRegionMarker(pg.Pos(plc.xmin() + 1.0, -1.0), 1)
    plc.addRegionMarker(pg.Pos(plc.xmax() - 1.0, -1.0), 1)
    return plc


def layered_res(mesh):
    z = np.array(mesh.cellCenters())[:, 1]
    return np.where(z > -INTERFACE_DEPTH, TOP_RES, BOTTOM_RES)


# Truths.

def halfspace_transfer(scheme, res=100.0):
    return res / np.array(scheme["k"], dtype=float)


def two_layer_transfer(scheme):
    """Exact transfer resistances on a two-layer earth (unit current, surface
    electrodes): the potential at distance r from a source is
    rho1 / (2 pi) (1 / r + 2 sum_n q^n / sqrt(r^2 + (2 n h)^2)) with
    q = (rho2 - rho1) / (rho2 + rho1)."""
    q = (BOTTOM_RES - TOP_RES) / (BOTTOM_RES + TOP_RES)
    n = np.arange(1, 1000)[:, None]  # q^1000 < 1e-80

    def potential(r):
        images = q ** n / np.sqrt(r ** 2 + (2.0 * n * INTERFACE_DEPTH) ** 2)
        return TOP_RES / (2.0 * np.pi) * (1.0 / r + 2.0 * images.sum(axis=0))

    x = np.arange(N_ELEC) * SPACING
    a, b, m, nn = electrodes(scheme)
    d = lambda i, j: np.abs(x[i] - x[j])
    return potential(d(a, m)) - potential(d(a, nn)) - potential(d(b, m)) + potential(d(b, nn))


def hilly_reference(scheme):
    t0 = time.perf_counter()
    mesh = mt.createMesh(hilly_geometry(REFERENCE_NODES, REFERENCE_AREA), quality=34)
    r = transfer(mesh, scheme, 100.0, sr=True)
    print(f"  reference: {mesh.cellCount()} cells (Triangle 34, area {REFERENCE_AREA:g}, "
          f"addNodes {REFERENCE_NODES}, singularity removal), {time.perf_counter() - t0:.0f} s", flush=True)
    del mesh
    gc.collect()
    return r


# Meshers: each takes a geometry function and returns a pyGIMLi mesh.

def triangle(quality, smooth=None, add_nodes=1):
    def build(geometry):
        return mt.createMesh(geometry(add_nodes=add_nodes), quality=quality, smooth=smooth)
    return build


def rustmesh(electrode_size=0.0, size_growth=0.5):
    """``electrode_size`` in electrode spacings (0: no electrode refinement);
    the target edge grows by ``size_growth`` m per m away from the nearest
    electrode. No smoothing."""
    def build(geometry):
        return rp.create_mesh(geometry(), quality=28, electrode_size=electrode_size * SPACING,
                              size_growth=size_growth)
    return build


MESHERS = [
    ("triangle-34-smooth", triangle(34, [2, 10])),  # PyHydroGeophysX's default
    *[(f"triangle-34-smooth-n{k}", triangle(34, [2, 10], add_nodes=k)) for k in (2, 3, 4, 6)],
    ("triangle-28", triangle(28)),
    ("rustmesh-28", rustmesh()),
    ("rustmesh-28-e0.25-g0.5", rustmesh(0.25, 0.5)),
    ("rustmesh-28-e0.25-g0.3", rustmesh(0.25, 0.3)),
    ("rustmesh-28-e0.15-g0.3", rustmesh(0.15, 0.3)),
]


def min_angle(mesh):
    worst = 180.0
    for c in mesh.cells():
        p = np.array([[n.pos()[0], n.pos()[1]] for n in c.nodes()])
        for i in range(3):
            u, v = p[(i + 1) % 3] - p[i], p[(i + 2) % 3] - p[i]
            worst = min(worst, np.degrees(np.arctan2(abs(u[0] * v[1] - u[1] * v[0]), u @ v)))
    return worst


def transfer(mesh, scheme, res, sr=False):
    data = ert.simulate(mesh, scheme=pg.DataContainerERT(scheme), res=res, sr=sr,
                        noiseLevel=0, noiseAbs=0, verbose=False)
    # Copy while `data` is alive: np.asarray would be a view into the
    # DataContainer's memory, which is freed with it (garbage values or a
    # crash later).
    return np.array(data["r"], dtype=float)


def describe(name, mesh, seconds):
    para = sum(1 for c in mesh.cells() if c.marker() >= 2)
    return (f"  {name:23s} cells {mesh.cellCount():6d} (inverted {para:5d})  "
            f"nodes {mesh.nodeCount():6d}  min angle {min_angle(mesh):5.1f}  "
            f"meshing {seconds * 1e3:7.1f} ms")


def error_metrics(values, truth):
    values, truth = np.asarray(values), np.asarray(truth)
    if values.shape != truth.shape or values.ndim != 1 or values.size == 0:
        raise ValueError("response and reference must be equal nonempty vectors")
    if not np.all(np.isfinite(values)) or not np.all(np.isfinite(truth)) or np.any(truth == 0):
        raise ValueError("nonfinite response/reference or zero reference resistance")
    rel = np.abs(values / truth - 1.0) * 100.0
    if not np.all(np.isfinite(rel)):
        raise ValueError("nonfinite relative error")
    return dict(median_percent=float(np.median(rel)), p95_percent=float(np.percentile(rel, 95)),
                max_percent=float(rel.max()), worst_reading=int(np.argmax(rel)), readings=len(rel))


def errors(values, truth):
    m = error_metrics(values, truth)
    return (f"median {m['median_percent']:6.3f} %   95th {m['p95_percent']:6.3f} %   "
            f"max {m['max_percent']:6.3f} %")


def check(geometry, runs, res=100.0):
    """Mesh ``geometry`` with every mesher, one mesh alive at a time, and print
    the error of each run ``(label, scheme, truth)`` on the mesh as it is
    without singularity removal (strict) and split once with singularity
    removal (as inverted)."""
    for name, build in MESHERS:
        t0 = time.perf_counter()
        mesh = build(geometry)
        seconds = time.perf_counter() - t0
        print(describe(name, mesh, seconds))
        for mode, sr in (("strict", False), ("inverted", True)):
            fwd = mesh.createH2() if sr else mesh
            rho = res(fwd) if callable(res) else res
            for label, scheme, truth in runs:
                r = transfer(fwd, scheme, rho, sr=sr)
                print(f"    {mode:8s} {label:4s} {errors(r, truth)}", flush=True)
            del fwd, rho
        del mesh
        gc.collect()


def case_halfspace():
    print("1. flat ground, 100 ohm-m half-space, against analytic values")
    runs = []
    for name in ("dd", "wa"):
        scheme = scheme_for(sensors(), name)
        runs.append((name, scheme, halfspace_transfer(scheme)))
        print(f"  scheme {name}: {scheme.size()} readings")
    check(flat_geometry, runs)


def case_layered():
    print(f"2. flat ground, {TOP_RES:g} ohm-m over {BOTTOM_RES:g} ohm-m (interface at "
          f"{INTERFACE_DEPTH:g} m in the geometry), against the exact two-layer solution")
    scheme = scheme_for(sensors(), "dd")
    check(layered_geometry, [("dd", scheme, two_layer_transfer(scheme))], layered_res)


def case_topography():
    print("3. hilly ground (z = 1.5 sin(x / 6)), 100 ohm-m, against a reference")
    scheme = scheme_for(sensors(hill), "dd")
    check(hilly_geometry, [("dd", scheme, hilly_reference(scheme))])


if __name__ == "__main__":
    print(f"rustmesh {rustmesh_version}, pyGIMLi {pg.__version__}, "
          f"{N_ELEC} electrodes {SPACING} m apart")
    case_halfspace()
    case_layered()
    case_topography()
