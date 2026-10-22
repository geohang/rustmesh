"""3D meshing through the Python entry points."""
import tempfile
from pathlib import Path
from xml.etree import ElementTree

import numpy as np
import pytest

from rustmesh import MeshQualityError, tetrahedralize


def test_regions_and_shared_interface(layers):
    vertices, facets, markers = layers
    mesh = tetrahedralize(vertices, facets, facet_markers=markers,
                          regions=[[.23, .31, .1, 2, .015], [.23, .31, .8, 3, .04]])
    volumes = mesh.cell_volumes
    assert volumes.sum() == pytest.approx(1., abs=1e-12)
    assert volumes[mesh.cell_markers == 2].sum() == pytest.approx(.4, abs=1e-12)
    assert volumes[mesh.cell_markers == 3].sum() == pytest.approx(.6, abs=1e-12)
    assert np.all(volumes[mesh.cell_markers == 2] <= .015 * (1 + 1e-12))
    # The interface z = 0.3 + 0.2 x over the unit square has area sqrt(1.04).
    p = mesh.nodes[mesh.faces[mesh.face_markers == 12]]
    areas = np.linalg.norm(np.cross(p[:, 1] - p[:, 0], p[:, 2] - p[:, 0]), axis=1) / 2
    assert areas.sum() == pytest.approx(np.sqrt(1.04), abs=1e-12)
    cells = mesh.face_cells[mesh.face_markers == 12]
    assert np.all(cells >= 0)
    assert np.all(mesh.cell_markers[cells[:, 0]] != mesh.cell_markers[cells[:, 1]])


def test_strict_budget_and_bad_python_arguments(cube):
    vertices, facets = cube
    with pytest.raises(RuntimeError):
        tetrahedralize(vertices, max_volume=1e-9, max_vertices=8)
    mesh = tetrahedralize(vertices, max_volume=1e-9, max_vertices=8, strict=False)
    assert not mesh.report["complete"]
    for kwargs in (dict(facets=facets.astype(float)), dict(facets=[[0, 1, -1]]),
                   dict(vertex_markers=[2**40] * 8), dict(max_passes=1.5),
                   dict(regions=[[.2, .3, .4, np.nan]]), dict(vertices=vertices.ravel())):
        args = dict(vertices=vertices)
        args.update(kwargs)
        with pytest.raises((ValueError, OverflowError)):
            tetrahedralize(**args)


def test_strict_quality_error_retains_recovered_mesh(cube):
    vertices, _ = cube
    kwargs = dict(max_volume=1e-9, max_vertices=8)
    with pytest.raises(MeshQualityError) as raised:
        tetrahedralize(vertices, **kwargs)
    error = raised.value
    assert isinstance(error, RuntimeError)
    assert not error.mesh.report["complete"]
    assert error.mesh.report["volume_violations"] > 0
    relaxed = tetrahedralize(vertices, strict=False, **kwargs)
    np.testing.assert_array_equal(error.mesh.tetrahedra, relaxed.tetrahedra)
    assert error.mesh.report == relaxed.report
    # A facet that cannot be recovered is a plain error, not a quality error.
    with pytest.raises(ValueError):
        tetrahedralize(vertices, [[0, 1, 2]], strict=False)


def test_vtu_export(layers):
    vertices, facets, markers = layers
    mesh = tetrahedralize(vertices, facets, facet_markers=markers,
                          regions=[[.2, .3, .1, 2], [.2, .3, .9, 3]])
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "mesh.vtu"
        mesh.write_vtu(path)
        piece = ElementTree.parse(path).find(".//Piece")
        assert int(piece.attrib["NumberOfCells"]) == len(mesh.tetrahedra)
        assert int(piece.attrib["NumberOfPoints"]) == len(mesh.nodes)


def test_pygimli_conversion(layers):
    pytest.importorskip("pygimli")
    from rustmesh.pygimli import to_pygimli
    vertices, facets, markers = layers
    mesh = tetrahedralize(vertices, facets, facet_markers=markers,
                          regions=[[.2, .3, .1, 2], [.2, .3, .9, 3]])
    converted = to_pygimli(mesh)
    assert converted.dim() == 3
    assert converted.cellCount() == len(mesh.tetrahedra)
    np.testing.assert_allclose(np.asarray(converted.cellSizes()), mesh.cell_volumes)
    assert any(b.marker() == 12 and b.leftCell() is not None and b.rightCell() is not None
               for b in converted.boundaries())
