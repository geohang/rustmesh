"""E4D export: file contents, boundary conditions and rejected input."""
import copy
import json
import tempfile
from pathlib import Path

import numpy as np
import pytest

from rustmesh.e4d import write_e4d


def test_files_and_boundary_conditions(layered_mesh):
    m = layered_mesh
    with tempfile.TemporaryDirectory() as folder:
        result = write_e4d(m, folder, conductivity=.01, translation=[.5, .5, .5])
        paths = result["files"]
        node = np.loadtxt(paths["node"], skiprows=1)
        ele = np.loadtxt(paths["ele"], skiprows=1, dtype=int)
        neigh = np.loadtxt(paths["neigh"], skiprows=1, dtype=int)
        faces = np.loadtxt(paths["face"], skiprows=1, dtype=int)
        np.testing.assert_allclose(node[:, 1:4] + np.loadtxt(paths["trn"]), m.nodes)
        np.testing.assert_array_equal(ele[:, 1:5] - 1, m.tetrahedra)
        np.testing.assert_array_equal(neigh[:, 1:], np.where(m.neighbors >= 0, m.neighbors + 1, -1))
        np.testing.assert_array_equal(faces[:, 1:4] - 1, m.faces)
        np.testing.assert_array_equal(ele[:, -1], np.where(m.cell_markers == 7, 1, 2))
        # Surface faces get flag 1, Dirichlet faces 2, the interface its own id.
        np.testing.assert_array_equal(faces[m.face_markers == -1, -1], 1)
        np.testing.assert_array_equal(faces[m.face_markers == -2, -1], 2)
        np.testing.assert_array_equal(faces[m.face_markers == 12, -1], 3)
        for f in m.faces[m.face_markers == -2]:
            np.testing.assert_array_equal(node[f, -1], 2)
        np.testing.assert_allclose(np.loadtxt(paths["sig"], skiprows=1), .01)
        assert paths["sig"].read_text().splitlines()[0] == f"{len(m.tetrahedra)} 1"
        meta = json.loads(paths["metadata"].read_text())
        assert meta["face_facets"] == m.face_facets.tolist()
        assert meta["node_markers"] == m.node_markers.tolist()
        assert meta["zone_of_marker"] == {"7": 1, "42": 2}
        with pytest.raises(FileExistsError):
            write_e4d(m, folder)
        write_e4d(m, folder, overwrite=True)


@pytest.mark.parametrize("kwargs", [dict(name="../escape"), dict(surface_markers=[-2]),
                                    dict(dirichlet_markers=[]),
                                    dict(translation=[float("nan"), 0, 0]),
                                    dict(conductivity=-1), dict(conductivity=[.1])])
def test_bad_arguments_are_rejected_before_writing(layered_mesh, kwargs):
    with tempfile.TemporaryDirectory() as folder:
        path = Path(folder) / "out"
        with pytest.raises(ValueError):
            write_e4d(layered_mesh, path, **kwargs)
        assert not path.exists()


def test_inconsistent_or_incomplete_mesh_is_rejected(layered_mesh):
    altered = copy.deepcopy(layered_mesh)
    altered.neighbors[:] = -1
    with tempfile.TemporaryDirectory() as folder, pytest.raises(ValueError):
        write_e4d(altered, folder)
    altered = copy.deepcopy(layered_mesh)
    altered.report["complete"] = False
    with tempfile.TemporaryDirectory() as folder:
        with pytest.raises(ValueError):
            write_e4d(altered, folder)
        write_e4d(altered, folder, allow_incomplete=True)
