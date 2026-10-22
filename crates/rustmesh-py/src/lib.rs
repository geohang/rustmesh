mod mesh3d;

use numpy::ndarray::{Array1, Array2};
use numpy::{IntoPyArray, PyReadonlyArray1, PyReadonlyArray2};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use rustmesh::{MeshError, Options, Plc, Region, TriangulationError};

pyo3::create_exception!(_rustmesh, Mesh2DQualityError, PyRuntimeError);

fn to_py_err(e: MeshError) -> PyErr {
    match e {
        MeshError::Internal(msg) => PyRuntimeError::new_err(msg),
        other => PyValueError::new_err(other.to_string()),
    }
}

fn rows<const N: usize>(a: &PyReadonlyArray2<'_, f64>, what: &str) -> PyResult<Vec<[f64; N]>> {
    let a = a.as_array();
    if a.ncols() != N {
        return Err(PyValueError::new_err(format!(
            "{what} must have {N} columns, not {}",
            a.ncols()
        )));
    }
    Ok(a.rows()
        .into_iter()
        .map(|r| std::array::from_fn(|k| r[k]))
        .collect())
}

fn flat<T: Copy>(rows: &[[T; 2]]) -> Vec<T> {
    rows.iter().flat_map(|r| r.iter().copied()).collect()
}

/// Mesh a planar straight-line graph. Use `rustmesh.triangulate`, which
/// converts its arguments and wraps the result.
#[pyfunction]
#[pyo3(signature = (vertices, segments, vertex_markers, segment_markers, regions, holes, min_angle, max_area, max_vertices, refine_points, point_size, size_growth, strict=false, keep_segments=None, keep_boundary=false))]
#[allow(clippy::too_many_arguments)]
fn triangulate<'py>(
    py: Python<'py>,
    vertices: PyReadonlyArray2<'py, f64>,
    segments: PyReadonlyArray2<'py, i64>,
    vertex_markers: PyReadonlyArray1<'py, i32>,
    segment_markers: PyReadonlyArray1<'py, i32>,
    regions: PyReadonlyArray2<'py, f64>,
    holes: PyReadonlyArray2<'py, f64>,
    min_angle: f64,
    max_area: f64,
    max_vertices: usize,
    refine_points: PyReadonlyArray2<'py, f64>,
    point_size: f64,
    size_growth: f64,
    strict: bool,
    keep_segments: Option<PyReadonlyArray1<'py, i64>>,
    keep_boundary: bool,
) -> PyResult<Bound<'py, PyDict>> {
    let mut kept = Vec::new();
    if let Some(keep) = &keep_segments {
        for (i, &s) in keep.as_array().iter().enumerate() {
            if s < 0 {
                return Err(PyValueError::new_err(format!(
                    "keep_segments[{i}] is negative"
                )));
            }
            kept.push(s as usize);
        }
    }
    let seg = segments.as_array();
    if seg.ncols() != 2 {
        return Err(PyValueError::new_err("segments must have 2 columns"));
    }
    let mut segment_list = Vec::with_capacity(seg.nrows());
    for (i, r) in seg.rows().into_iter().enumerate() {
        if r[0] < 0 || r[1] < 0 {
            return Err(PyValueError::new_err(format!(
                "segment {i} has a negative vertex index"
            )));
        }
        segment_list.push([r[0] as usize, r[1] as usize]);
    }
    let mut region_list = Vec::new();
    for (i, r) in rows::<4>(&regions, "regions")?.into_iter().enumerate() {
        if !r[2].is_finite()
            || r[2].fract() != 0.0
            || r[2] < i32::MIN as f64
            || r[2] > i32::MAX as f64
        {
            return Err(PyValueError::new_err(format!(
                "region {i}: the marker must be an int32 integer"
            )));
        }
        region_list.push(Region {
            point: [r[0], r[1]],
            marker: r[2] as i32,
            max_area: r[3],
        });
    }
    let plc = Plc {
        vertices: rows::<2>(&vertices, "vertices")?,
        vertex_markers: vertex_markers.as_array().to_vec(),
        segments: segment_list,
        segment_markers: segment_markers.as_array().to_vec(),
        regions: region_list,
        holes: rows::<2>(&holes, "holes")?,
    };
    let options = Options {
        min_angle,
        max_area,
        max_vertices,
        refine_points: rows::<2>(&refine_points, "refine_points")?,
        point_size,
        size_growth,
        keep_segments: kept,
        keep_boundary,
    };
    let result = py.detach(|| {
        if strict {
            rustmesh::triangulate_strict(&plc, &options)
        } else {
            rustmesh::triangulate(&plc, &options).map_err(TriangulationError::Meshing)
        }
    });
    let (mesh, incomplete) = match result {
        Ok(mesh) => (mesh, false),
        Err(TriangulationError::Incomplete(mesh)) => (*mesh, true),
        Err(TriangulationError::Meshing(error)) => return Err(to_py_err(error)),
    };
    let incomplete_message = incomplete.then(|| {
        format!(
            "requested 2D mesh goals were not met: {}",
            mesh.report.issues.join("; ")
        )
    });

    let out = PyDict::new(py);
    let n = mesh.nodes.len();
    out.set_item(
        "nodes",
        Array2::from_shape_vec((n, 2), flat(&mesh.nodes))
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
    let tris: Vec<i64> = mesh
        .triangles
        .iter()
        .flat_map(|t| t.map(|k| k as i64))
        .collect();
    out.set_item(
        "triangles",
        Array2::from_shape_vec((mesh.triangles.len(), 3), tris)
            .unwrap()
            .into_pyarray(py),
    )?;
    out.set_item(
        "cell_markers",
        Array1::from(mesh.cell_markers).into_pyarray(py),
    )?;
    out.set_item(
        "cell_regions",
        Array1::from(mesh.cell_regions).into_pyarray(py),
    )?;
    let edges: Vec<i64> = mesh
        .edges
        .iter()
        .flat_map(|e| e.map(|k| k as i64))
        .collect();
    out.set_item(
        "edges",
        Array2::from_shape_vec((mesh.edges.len(), 2), edges)
            .unwrap()
            .into_pyarray(py),
    )?;
    out.set_item(
        "edge_markers",
        Array1::from(mesh.edge_markers).into_pyarray(py),
    )?;
    out.set_item(
        "edge_segments",
        Array1::from(mesh.edge_segments).into_pyarray(py),
    )?;

    let r = mesh.report;
    let report = PyDict::new(py);
    report.set_item("complete", r.complete)?;
    report.set_item("refinement_complete", r.refinement_complete)?;
    report.set_item("quality_met", r.quality_met)?;
    report.set_item("issues", r.issues)?;
    report.set_item("warnings", r.warnings)?;
    report.set_item("min_angle", r.min_angle)?;
    report.set_item("angle_violations", r.angle_violations)?;
    let diagnostics = pyo3::types::PyList::empty(py);
    for diagnostic in r.angle_diagnostics {
        let item = PyDict::new(py);
        item.set_item("cell", diagnostic.cell)?;
        item.set_item("reason", diagnostic.reason)?;
        diagnostics.append(item)?;
    }
    report.set_item("angle_diagnostics", diagnostics)?;
    report.set_item("area_violations", r.area_violations)?;
    report.set_item("refine_calls", r.refine_calls)?;
    report.set_item("vertices_outside", r.vertices_outside)?;
    out.set_item("report", &report)?;
    if let Some(message) = incomplete_message {
        let error = Mesh2DQualityError::new_err(message);
        error.value(py).setattr("mesh", &out)?;
        error.value(py).setattr("report", &report)?;
        return Err(error);
    }
    Ok(out)
}

#[pymodule]
fn _rustmesh(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add(
        "Mesh2DQualityError",
        m.py().get_type::<Mesh2DQualityError>(),
    )?;
    m.add_function(wrap_pyfunction!(triangulate, m)?)?;
    m.add_function(wrap_pyfunction!(mesh3d::tetrahedralize, m)?)?;
    Ok(())
}
