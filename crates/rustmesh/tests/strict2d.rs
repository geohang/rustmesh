use rustmesh::{triangulate_strict, Options, Plc, TriangulationError};

fn square() -> Plc {
    Plc {
        vertices: vec![[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
        segments: vec![[0, 1], [1, 2], [2, 3], [3, 0]],
        ..Default::default()
    }
}
#[test]
fn strict_budget_failure_retains_the_original_mesh_and_report() {
    let options = Options {
        max_area: 0.001,
        max_vertices: 4,
        ..Default::default()
    };
    let error = triangulate_strict(&square(), &options).unwrap_err();
    assert!(matches!(error, TriangulationError::Incomplete(_)));
    assert!(!error.report().unwrap().refinement_complete);
    assert!(error.report().unwrap().area_violations > 0);
    assert!(!error.mesh().unwrap().triangles.is_empty());
    assert!(error.to_string().contains("vertex budget"));
    assert!(!error.into_mesh().unwrap().report.complete);
}
#[test]
fn unmet_angle_is_an_error_even_when_refinement_finished() {
    let plc = Plc {
        vertices: vec![[0., 0.], [1., 0.], [1., 0.1]],
        segments: vec![[0, 1], [1, 2], [2, 0]],
        ..Default::default()
    };
    let error = triangulate_strict(
        &plc,
        &Options {
            max_vertices: 10_000,
            ..Default::default()
        },
    )
    .unwrap_err();
    let report = error.report().unwrap();
    assert!(report.refinement_complete);
    assert!(report.angle_violations > 0);
    assert!(!report.quality_met && !report.complete);
    assert!(error.mesh().unwrap().cell_areas().iter().all(|&a| a > 0.0));
}

#[test]
fn high_angle_line2_finishes_without_waiving_constraints_or_quality() {
    // A pyGIMLi PLC from a surface ERT line. Angle targets of 32 and 34
    // degrees must be met within 6000 vertices, with every input vertex and
    // segment kept.
    let value: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/line2-plc.json")).unwrap();
    let plc = Plc {
        vertices: serde_json::from_value(value["vertices"].clone()).unwrap(),
        segments: serde_json::from_value(value["segments"].clone()).unwrap(),
        vertex_markers: serde_json::from_value(value["vertex_markers"].clone()).unwrap(),
        segment_markers: serde_json::from_value(value["segment_markers"].clone()).unwrap(),
        holes: serde_json::from_value(value["holes"].clone()).unwrap(),
        regions: value["regions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| rustmesh::Region {
                point: [r[0].as_f64().unwrap(), r[1].as_f64().unwrap()],
                marker: r[2].as_f64().unwrap() as i32,
                max_area: r[3].as_f64().unwrap(),
            })
            .collect(),
    };
    for target in [32.0, 34.0] {
        let options = Options {
            min_angle: target,
            max_vertices: 6000,
            ..Default::default()
        };
        let mesh = triangulate_strict(&plc, &options).unwrap();
        assert_eq!(mesh.report.angle_violations, 0);
        for triangle in &mesh.triangles {
            let p = triangle.map(|i| mesh.nodes[i]);
            for i in 0..3 {
                let a = [p[(i + 1) % 3][0] - p[i][0], p[(i + 1) % 3][1] - p[i][1]];
                let b = [p[(i + 2) % 3][0] - p[i][0], p[(i + 2) % 3][1] - p[i][1]];
                let angle = (a[0] * b[1] - a[1] * b[0])
                    .abs()
                    .atan2(a[0] * b[0] + a[1] * b[1])
                    .to_degrees();
                assert!(
                    angle >= target - 1e-6,
                    "independent angle {angle} < {target}"
                );
            }
        }
        for (i, point) in plc.vertices.iter().enumerate() {
            let node = usize::try_from(mesh.input_vertex_nodes[i]).unwrap();
            assert_eq!(&mesh.nodes[node], point);
            assert_eq!(mesh.node_markers[node], plc.vertex_markers[i]);
        }
        for (i, &[a, b]) in plc.segments.iter().enumerate() {
            let length = (plc.vertices[a][0] - plc.vertices[b][0])
                .hypot(plc.vertices[a][1] - plc.vertices[b][1]);
            let mut covered = 0.0;
            for (edge, parent) in mesh.edges.iter().zip(&mesh.edge_segments) {
                if *parent == i as i64 {
                    let [p, q] = edge.map(|v| mesh.nodes[v]);
                    covered += (p[0] - q[0]).hypot(p[1] - q[1]);
                }
            }
            assert!((covered - length).abs() < 1e-9 * length.max(1.0));
        }
        let again = triangulate_strict(&plc, &options).unwrap();
        assert_eq!(mesh.nodes, again.nodes);
        assert_eq!(mesh.triangles, again.triangles);
    }
    // A kept boundary stays unsplit at a high angle target, and the area goal
    // it makes unreachable is reported as incomplete.
    let kept = triangulate_strict(
        &square(),
        &Options {
            min_angle: 34.0,
            max_area: 0.001,
            max_vertices: 6000,
            keep_boundary: true,
            ..Default::default()
        },
    )
    .unwrap_err()
    .into_mesh()
    .unwrap();
    for i in 0..4 {
        assert_eq!(kept.edge_segments.iter().filter(|p| **p == i).count(), 1);
    }
    assert!(!kept.report.complete);
}

#[test]
fn internal_acute_interface_does_not_use_boundary_corner_relaxation() {
    // An acute angle between two interior interfaces gets no boundary-corner
    // relaxation. Refinement stays bounded, the mesh is reported incomplete,
    // and both materials and the angle diagnostics are kept.
    let h = 2.0 * 5.0f64.to_radians().tan();
    let plc = Plc {
        vertices: vec![
            [0., 0.],
            [10., 0.],
            [10., 8.],
            [0., 8.],
            [3., 4.],
            [5., 4. - h],
            [5., 4. + h],
        ],
        segments: vec![[0, 1], [1, 2], [2, 3], [3, 0], [4, 5], [5, 6], [6, 4]],
        regions: vec![
            rustmesh::Region {
                point: [1., 1.],
                marker: 1,
                max_area: 0.15,
            },
            rustmesh::Region {
                point: [4.5, 4.],
                marker: 2,
                max_area: 0.03,
            },
        ],
        ..Default::default()
    };
    let mesh = triangulate_strict(
        &plc,
        &Options {
            min_angle: 34.,
            max_vertices: 4000,
            ..Default::default()
        },
    )
    .unwrap_err()
    .into_mesh()
    .unwrap();
    assert!(mesh.report.refinement_complete);
    assert!(mesh.nodes.len() < 1500);
    assert_eq!(mesh.report.area_violations, 0);
    assert!(!mesh.report.quality_met);
    assert_eq!(
        mesh.report.angle_diagnostics.len(),
        mesh.report.angle_violations
    );
    assert!(mesh
        .report
        .angle_diagnostics
        .iter()
        .all(|d| d.reason == "shared_input_corner_guard"));
    assert!(mesh.cell_markers.contains(&1) && mesh.cell_markers.contains(&2));
}

#[test]
fn accepted_cavities_preserve_local_constrained_delaunay_property() {
    use std::collections::{HashMap, HashSet};
    let mut fixtures = vec![square()];
    let mut interface = square();
    interface
        .vertices
        .extend([[0.0, 0.37], [1.0, 0.37], [0.2, 0.7], [0.8, 0.7]]);
    interface.segments.extend([[4, 5], [6, 7]]);
    fixtures.push(interface);
    // Cocircular input vertices exercise ambiguous but valid flip orders.
    let vertices: Vec<_> = (0..16)
        .map(|i| {
            let a = (i as f64) * std::f64::consts::TAU / 16.0;
            [a.cos(), a.sin()]
        })
        .collect();
    fixtures.push(Plc {
        vertices,
        segments: (0..16).map(|i| [i, (i + 1) % 16]).collect(),
        ..Default::default()
    });
    for plc in fixtures {
        for angle in [28.0, 30.0, 32.0, 34.0] {
            let mesh = triangulate_strict(
                &plc,
                &Options {
                    min_angle: angle,
                    max_area: 0.005,
                    max_vertices: 6000,
                    ..Default::default()
                },
            )
            .unwrap();
            let key = |a, b| if a < b { (a, b) } else { (b, a) };
            let fixed: HashSet<_> = mesh.edges.iter().map(|e| key(e[0], e[1])).collect();
            let mut opposite = HashMap::new();
            for tri in &mesh.triangles {
                for i in 0..3 {
                    let (a, b, c) = (tri[i], tri[(i + 1) % 3], tri[(i + 2) % 3]);
                    if fixed.contains(&key(a, b)) {
                        continue;
                    }
                    if let Some(d) = opposite.insert(key(a, b), c) {
                        let coord = |v: usize| robust::Coord {
                            x: mesh.nodes[v][0],
                            y: mesh.nodes[v][1],
                        };
                        let determinant = robust::incircle(coord(a), coord(b), coord(c), coord(d));
                        let length2 = |u: usize, v: usize| {
                            (mesh.nodes[u][0] - mesh.nodes[v][0]).powi(2)
                                + (mesh.nodes[u][1] - mesh.nodes[v][1]).powi(2)
                        };
                        let scale = length2(a, b).max(length2(a, c)).max(length2(a, d));
                        assert!(
                            determinant <= 1e-12 * scale * scale,
                            "illegal edge {a}-{b}: incircle={determinant}"
                        );
                    }
                }
            }
        }
    }
}
