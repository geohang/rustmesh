"""Shared geometry for the Python tests."""
import numpy as np
import pytest

import rustmesh


@pytest.fixture
def cube():
    """Vertices and triangular facets of the unit cube."""
    vertices = np.array([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0],
                         [0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]], dtype=float)
    facets = np.array([[0, 1, 2], [0, 2, 3], [4, 6, 5], [4, 7, 6], [0, 4, 5], [0, 5, 1],
                       [1, 5, 6], [1, 6, 2], [2, 6, 7], [2, 7, 3], [3, 7, 4], [3, 4, 0]])
    return vertices, facets


@pytest.fixture
def layers():
    """A unit cube split by an inclined interface (facet marker 12).

    Returns vertices, facets and facet markers. The interface is the plane
    z = 0.3 + 0.2 x, so the lower layer has volume 0.4 and the upper 0.6.
    """
    xy = np.array([[0, 0], [1, 0], [1, 1], [0, 1]], dtype=float)
    heights = (np.zeros(4), 0.3 + 0.2 * xy[:, 0], np.ones(4))
    vertices = np.vstack([np.column_stack((xy, z)) for z in heights])
    facets = [[0, 1, 2], [0, 2, 3], [8, 9, 10], [8, 10, 11], [4, 5, 6], [4, 6, 7]]
    markers = [-2, -2, -1, -1, 12, 12]
    for layer in range(2):
        for k in range(4):
            a, b = layer * 4 + k, layer * 4 + (k + 1) % 4
            facets.extend([[a, b, b + 4], [a, b + 4, a + 4]])
            markers.extend([-2, -2])
    return vertices, facets, markers


@pytest.fixture
def layered_mesh(layers):
    """The two-layer cube meshed, with region markers 7 below and 42 above."""
    vertices, facets, markers = layers
    return rustmesh.tetrahedralize(vertices, facets, facet_markers=markers,
                                   regions=[[.2, .3, .1, 7], [.2, .3, .9, 42]])
