"""Draw the website's figures from meshes made by the rustmesh Python package.

usage: python site/figures.py      (after maturin develop; writes site/images/)
"""
import math
import pathlib

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np
from matplotlib.collections import PolyCollection

import rustmesh

OUT = pathlib.Path(__file__).resolve().parent / "images"
EDGE = "#3a3a3c"


def draw(mesh, colors, path, xlim=None, ylim=None, points=None, width=8.0):
    tri = mesh.nodes[mesh.triangles]
    x0, x1 = xlim or (mesh.nodes[:, 0].min(), mesh.nodes[:, 0].max())
    y0, y1 = ylim or (mesh.nodes[:, 1].min(), mesh.nodes[:, 1].max())
    fig = plt.figure(figsize=(width, width * (y1 - y0) / (x1 - x0)))
    ax = fig.add_axes([0, 0, 1, 1])
    faces = [colors.get(int(m), "#ffffff") for m in mesh.cell_markers]
    ax.add_collection(PolyCollection(tri, facecolors=faces, edgecolors=EDGE, linewidths=0.18))
    if points is not None:
        ax.plot(points[:, 0], points[:, 1], "o", ms=2.2, color="#1d1d1f")
    ax.set_xlim(x0, x1)
    ax.set_ylim(y0, y1)
    ax.set_aspect("equal")
    ax.axis("off")
    fig.savefig(path, dpi=200)
    plt.close(fig)
    # a 64-colour palette keeps the files small; the drawings use few colours
    from PIL import Image
    Image.open(path).convert("RGB").quantize(colors=64).save(path, optimize=True)


def smallest_angles(mesh):
    p = mesh.nodes[mesh.triangles]
    a = np.linalg.norm(p[:, 1] - p[:, 2], axis=1)
    b = np.linalg.norm(p[:, 2] - p[:, 0], axis=1)
    c = np.linalg.norm(p[:, 0] - p[:, 1], axis=1)
    cos = lambda u, v, w: np.clip((v**2 + w**2 - u**2) / (2 * v * w), -1, 1)
    return np.degrees(np.arccos(np.stack([cos(a, b, c), cos(b, c, a), cos(c, a, b)]))).min(axis=0)


def surface_section():
    """A resistivity line: 48 electrodes on topography, two layers and a body."""
    left, right, bottom = -40.0, 134.0, -50.0
    ground = lambda x: 2.5 * math.sin(2 * math.pi * (x - left) / 120.0)
    layer = lambda x: -9.0 + 0.04 * x
    electrodes = np.array([[2.0 * i, ground(2.0 * i)] for i in range(48)])
    xs = sorted(set(np.r_[np.linspace(left, right, 30), electrodes[:, 0]].round(6)))
    surface = [[x, ground(x)] for x in xs]
    vertices = surface + [[right, layer(right)], [right, bottom], [left, bottom], [left, layer(left)]]
    n = len(vertices)
    segments = [[i, (i + 1) % n] for i in range(n)]
    markers = [-1] * (len(surface) - 1) + [-2] * (n - len(surface) + 1)
    # the layer boundary, from the left side to the right side
    inner = [[x, layer(x)] for x in np.linspace(left, right, 12)[1:-1]]
    first = len(vertices)
    vertices += inner
    chain = [n - 1] + list(range(first, first + len(inner))) + [len(surface)]
    segments += [[a, b] for a, b in zip(chain[:-1], chain[1:])]
    markers += [0] * (len(chain) - 1)
    # an elliptic body below the layer boundary
    first = len(vertices)
    k = 32
    vertices += [[55 + 8 * math.cos(2 * math.pi * i / k), -15 + 3.5 * math.sin(2 * math.pi * i / k)]
                 for i in range(k)]
    segments += [[first + i, first + (i + 1) % k] for i in range(k)]
    markers += [0] * k
    regions = [[left + 1, -2, 1], [left + 1, bottom + 1, 2], [55, -15, 3]]
    mesh = rustmesh.triangulate(vertices, segments, segment_markers=markers, regions=regions,
                                min_angle=30, refine_points=electrodes, point_size=0.5,
                                size_growth=0.3)
    colors = {1: "#e3ecf7", 2: "#f2ebe0", 3: "#f4cdb9"}
    draw(mesh, colors, OUT / "surface.png", xlim=(-12, 106), ylim=(-30, 4), points=electrodes,
         width=9.0)
    return mesh


def zones_and_holes():
    """The README example: a box, a zone with finer cells and a hole."""
    vertices = [[0, 0], [10, 0], [10, 5], [0, 5],
                [1, 1], [3, 1], [3, 4], [1, 4],
                [6, 1.5], [8, 1.5], [8, 3.5], [6, 3.5]]
    loop = lambda start: [[start + i, start + (i + 1) % 4] for i in range(4)]
    mesh = rustmesh.triangulate_strict(
        vertices, loop(0) + loop(4) + loop(8),
        segment_markers=[1] * 4 + [2] * 4 + [3] * 4,
        regions=[[0.5, 0.5, 1, 0.5], [2, 2.5, 2, 0.02]],
        holes=[[7, 2.5]], min_angle=28)
    draw(mesh, {1: "#eef0f3", 2: "#d6e6fa"}, OUT / "zones.png", width=7.0)
    return mesh


if __name__ == "__main__":
    OUT.mkdir(exist_ok=True)
    for name, mesh in [("surface", surface_section()), ("zones", zones_and_holes())]:
        print(f"{name}: {len(mesh.nodes)} nodes, {len(mesh.triangles)} cells, "
              f"smallest angle {smallest_angles(mesh).min():.1f} deg, complete {mesh.report['complete']}")
