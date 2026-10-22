use super::{rows, to_py_err};
use numpy::ndarray::{Array1, Array2};
use numpy::{IntoPyArray, PyReadonlyArray1, PyReadonlyArray2};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

#[pyfunction]
#[pyo3(signature = (vertices, facets, vertex_markers, facet_markers, regions, holes, max_volume, max_vertices, max_passes, min_dihedral))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn tetrahedralize<'py>(
    py: Python<'py>,
    vertices: PyReadonlyArray2<'py, f64>,
    facets: PyReadonlyArray2<'py, i64>,
    vertex_markers: PyReadonlyArray1<'py, i32>,
    facet_markers: PyReadonlyArray1<'py, i32>,
    regions: PyReadonlyArray2<'py, f64>,
    holes: PyReadonlyArray2<'py, f64>,
    max_volume: f64,
    max_vertices: usize,
    max_passes: usize,
    min_dihedral: f64,
) -> PyResult<Bound<'py, PyDict>> {
    let f = facets.as_array();
    if f.ncols() != 3 {
        return Err(PyValueError::new_err("facets must have 3 columns"));
    }
    let mut facet_list = Vec::new();
    for r in f.rows() {
        if r.iter().any(|&v| v < 0) {
            return Err(PyValueError::new_err("facet indices must be non-negative"));
        }
        facet_list.push([r[0] as usize, r[1] as usize, r[2] as usize]);
    }
    let mut region_list = Vec::new();
    for (i, r) in rows::<5>(&regions, "regions")?.into_iter().enumerate() {
        if !r[3].is_finite()
            || r[3].fract() != 0.0
            || r[3] < i32::MIN as f64
            || r[3] > i32::MAX as f64
        {
            return Err(PyValueError::new_err(format!(
                "region {i}: marker must be an int32 integer"
            )));
        }
        region_list.push(rustmesh::Region3D {
            point: [r[0], r[1], r[2]],
            marker: r[3] as i32,
            max_volume: r[4],
        });
    }
    let plc = rustmesh::Plc3D {
        vertices: rows::<3>(&vertices, "vertices")?,
        vertex_markers: vertex_markers.as_array().to_vec(),
        facets: facet_list,
        facet_markers: facet_markers.as_array().to_vec(),
        regions: region_list,
        holes: rows::<3>(&holes, "holes")?,
    };
    let options = rustmesh::Options3D {
        max_volume,
        max_vertices,
        max_passes,
        min_dihedral,
    };
    let mesh = py
        .detach(|| rustmesh::tetrahedralize(&plc, &options))
        .map_err(to_py_err)?;
    let out = PyDict::new(py);
    out.set_item(
        "nodes",
        Array2::from_shape_vec(
            (mesh.nodes.len(), 3),
            mesh.nodes.iter().flatten().copied().collect(),
        )
        .unwrap()
        .into_pyarray(py),
    )?;
    out.set_item(
        "tetrahedra",
        Array2::from_shape_vec(
            (mesh.tetrahedra.len(), 4),
            mesh.tetrahedra
                .iter()
                .flatten()
                .map(|&v| v as i64)
                .collect(),
        )
        .unwrap()
        .into_pyarray(py),
    )?;
    out.set_item(
        "neighbors",
        Array2::from_shape_vec(
            (mesh.neighbors.len(), 4),
            mesh.neighbors.iter().flatten().copied().collect(),
        )
        .unwrap()
        .into_pyarray(py),
    )?;
    out.set_item(
        "faces",
        Array2::from_shape_vec(
            (mesh.faces.len(), 3),
            mesh.faces.iter().flatten().map(|&v| v as i64).collect(),
        )
        .unwrap()
        .into_pyarray(py),
    )?;
    out.set_item(
        "face_cells",
        Array2::from_shape_vec(
            (mesh.face_cells.len(), 2),
            mesh.face_cells.iter().flatten().copied().collect(),
        )
        .unwrap()
        .into_pyarray(py),
    )?;
    out.set_item(
        "node_markers",
        Array1::from(mesh.node_markers).into_pyarray(py),
    )?;
    out.set_item(
        "input_vertex_nodes",
        Array1::from(mesh.input_vertex_nodes).into_pyarray(py),
    )?;
    out.set_item(
        "cell_markers",
        Array1::from(mesh.cell_markers).into_pyarray(py),
    )?;
    out.set_item(
        "cell_regions",
        Array1::from(mesh.cell_regions).into_pyarray(py),
    )?;
    out.set_item(
        "face_markers",
        Array1::from(mesh.face_markers).into_pyarray(py),
    )?;
    out.set_item(
        "face_facets",
        Array1::from(mesh.face_facets).into_pyarray(py),
    )?;
    let r = mesh.report;
    let report = PyDict::new(py);
    report.set_item("complete", r.complete)?;
    report.set_item("issues", r.issues)?;
    report.set_item("warnings", r.warnings)?;
    report.set_item("passes", r.passes)?;
    report.set_item("steiner_points", r.steiner_points)?;
    report.set_item("volume_violations", r.volume_violations)?;
    report.set_item("dihedral_violations", r.dihedral_violations)?;
    report.set_item("forced_dihedrals", r.forced_dihedrals)?;
    report.set_item("sliver_passes", r.sliver_passes)?;
    report.set_item("flips", r.flips)?;
    report.set_item("interior_collapses", r.interior_collapses)?;
    report.set_item("smoothing_moves", r.smoothing_moves)?;
    report.set_item("smoothing_sweeps", r.smoothing_sweeps)?;
    report.set_item("local_edge_splits", r.local_edge_splits)?;
    report.set_item("local_cell_splits", r.local_cell_splits)?;
    report.set_item("local_refinement_passes", r.local_refinement_passes)?;
    report.set_item("min_dihedral_p05", r.min_dihedral_p05)?;
    report.set_item("min_dihedral_median", r.min_dihedral_median)?;
    report.set_item("min_dihedral_p95", r.min_dihedral_p95)?;
    report.set_item("sliver_count", r.sliver_count)?;
    report.set_item("radius_edge_median", r.radius_edge_median)?;
    report.set_item("radius_edge_p95", r.radius_edge_p95)?;
    report.set_item("radius_edge_max", r.radius_edge_max)?;
    report.set_item("mean_ratio_min", r.mean_ratio_min)?;
    report.set_item("mean_ratio_median", r.mean_ratio_median)?;
    report.set_item("shape_metric_failures", r.shape_metric_failures)?;
    report.set_item("sizing_edge_violations", r.sizing_edge_violations)?;
    report.set_item("sizing_max_edge_ratio", r.sizing_max_edge_ratio)?;
    report.set_item("min_dihedral", r.min_dihedral)?;
    report.set_item("min_volume", r.min_volume)?;
    report.set_item("max_volume", r.max_volume)?;
    report.set_item("total_volume", r.total_volume)?;
    out.set_item("report", report)?;
    Ok(out)
}
