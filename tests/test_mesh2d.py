"""2D meshing through the Python entry points: strict mode and the pyGIMLi adapter."""
import warnings

import numpy as np
import pytest

import rustmesh

VERTICES = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]]
SEGMENTS = [[0, 1], [1, 2], [2, 3], [3, 0]]


def test_strict_matches_default_for_a_complete_mesh():
    strict = rustmesh.triangulate_strict(VERTICES, SEGMENTS, max_area=.05)
    default = rustmesh.triangulate(VERTICES, SEGMENTS, max_area=.05)
    assert strict.report["complete"]
    np.testing.assert_array_equal(strict.nodes, default.nodes)
    np.testing.assert_array_equal(strict.triangles, default.triangles)


def test_budget_error_keeps_mesh_and_report():
    with pytest.raises(rustmesh.Mesh2DQualityError) as raised:
        rustmesh.triangulate(VERTICES, SEGMENTS, max_area=.001, max_vertices=4, strict=True)
    error = raised.value
    assert error.report is error.mesh.report
    assert not error.report["refinement_complete"]
    assert error.report["area_violations"] > 0
    assert np.all(error.mesh.cell_areas > 0)
    assert "vertex budget" in str(error)
    default = rustmesh.triangulate(VERTICES, SEGMENTS, max_area=.001, max_vertices=4)
    np.testing.assert_array_equal(error.mesh.triangles, default.triangles)


@pytest.mark.parametrize("kwargs", [dict(segments=[[0, 99]]),
                                    dict(segments=SEGMENTS, holes=[[.5, .5]])])
def test_invalid_and_empty_input_raise_without_a_mesh(kwargs):
    with pytest.raises(ValueError) as raised:
        rustmesh.triangulate_strict(VERTICES, **kwargs)
    assert not hasattr(raised.value, "mesh")
    assert not hasattr(raised.value, "report")


@pytest.mark.parametrize("kwargs", [dict(segments=[[0., 1.5]]),
                                    dict(segments=SEGMENTS, max_vertices=4.5),
                                    dict(segments=SEGMENTS, vertex_markers=[0, 0, 0, .5]),
                                    dict(segments=SEGMENTS, segment_markers=[0, 0, 0, 2**31])])
def test_strict_rejects_integer_inputs_it_would_truncate(kwargs):
    with pytest.raises(ValueError) as raised:
        rustmesh.triangulate_strict(VERTICES, **kwargs)
    assert not hasattr(raised.value, "mesh")


def test_pygimli_strict_error_carries_mesh_and_report():
    mt = pytest.importorskip("pygimli.meshtools")
    from rustmesh.pygimli import create_mesh
    plc = mt.createRectangle(start=[0., 0.], end=[1., -1.], marker=1)
    with pytest.raises(rustmesh.Mesh2DQualityError) as raised:
        create_mesh(plc, area=.001, max_vertices=4)
    error = raised.value
    assert error.report is error.mesh.report
    assert len(error.mesh.triangles) > 0
    assert not error.report["complete"]
    with pytest.warns(RuntimeWarning, match="incomplete"):
        assert create_mesh(plc, area=.001, max_vertices=4, strict=False).cellCount() > 0


def test_pygimli_angle_mapping_reports_requested_and_effective_angle():
    mt = pytest.importorskip("pygimli.meshtools")
    from rustmesh.pygimli import create_mesh, map_quality
    plc = mt.createRectangle(start=[0., 0.], end=[1., -1.], marker=7)
    reference, report = create_mesh(plc, quality=30, area=.02, return_report=True)
    assert report["angle_mapping"]["policy"] == "exact"
    with warnings.catch_warnings(record=True) as observed:
        warnings.simplefilter("always")
        for requested in (32, 34):
            mesh, report = create_mesh(plc, quality=requested, area=.02,
                                       angle_policy="cap30", return_report=True)
            # cap30 meshes with a 30 degree target, so it matches the 30 degree mesh.
            assert mesh.cellCount() == reference.cellCount()
            np.testing.assert_array_equal(np.asarray(mesh.positions()),
                                          np.asarray(reference.positions()))
            assert report["complete"]
            contract = report["angle_mapping"]
            assert contract["requested_min_angle"] == requested
            assert contract["effective_min_angle"] == 30
            # Recount the cells below the requested angle with the law of cosines.
            p = np.asarray(mesh.positions())[:, :2]
            triangles = np.array([[n.id() for n in c.nodes()] for c in mesh.cells()])
            a = np.linalg.norm(p[triangles] - np.roll(p[triangles], 1, axis=1), axis=2)
            b, c = np.roll(a, 1, axis=1), np.roll(a, 2, axis=1)
            angles = np.degrees(np.arccos(np.clip((b**2 + c**2 - a**2) / (2 * b * c), -1, 1)))
            below = int(np.sum(angles.min(axis=1) < requested - 1e-6))
            assert contract["requested_angle_violations"] == below
            assert contract["requested_complete"] == (below == 0)
        assert len(observed) == 2
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", RuntimeWarning)
        with pytest.raises(rustmesh.Mesh2DQualityError) as raised:
            create_mesh(plc, quality=34, area=.001, max_vertices=4, angle_policy="cap30")
    error = raised.value
    assert error.report is error.mesh.report
    assert error.report["angle_mapping"]["requested_min_angle"] == 34
    assert not error.report["angle_mapping"]["requested_complete"]
    # A thin triangle cannot reach 34 degrees; the shortfall is reported.
    acute = mt.createPolygon([[0., 0.], [1., 0.], [1., -.1]], isClosed=True, marker=7)
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", RuntimeWarning)
        with pytest.raises(rustmesh.Mesh2DQualityError) as raised:
            create_mesh(acute, quality=34, angle_policy="cap30", max_vertices=10000)
    report = raised.value.report
    assert report["refinement_complete"]
    assert report["angle_violations"] > 0
    assert not report["quality_met"]
    assert not report["angle_mapping"]["requested_complete"]
    for requested in (0, 20, 28, 30):
        assert map_quality(requested, "cap30")["effective_min_angle"] == requested
    assert map_quality(34)["effective_min_angle"] == 34
    for requested in (float("nan"), float("inf"), -1, 41):
        with pytest.raises(ValueError):
            map_quality(requested, "cap30")
    with pytest.raises(ValueError):
        map_quality(30, "unknown")
