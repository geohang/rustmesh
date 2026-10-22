use rustmesh::{tetrahedralize, Mesh3D, Options3D, Plc3D, Region3D};
use std::collections::BTreeMap;

fn cube() -> Plc3D {
    Plc3D {
        vertices: vec![
            [0., 0., 0.],
            [1., 0., 0.],
            [1., 1., 0.],
            [0., 1., 0.],
            [0., 0., 1.],
            [1., 0., 1.],
            [1., 1., 1.],
            [0., 1., 1.],
        ],
        facets: vec![
            [0, 1, 2],
            [0, 2, 3],
            [4, 6, 5],
            [4, 7, 6],
            [0, 4, 5],
            [0, 5, 1],
            [1, 5, 6],
            [1, 6, 2],
            [2, 6, 7],
            [2, 7, 3],
            [3, 7, 4],
            [3, 4, 0],
        ],
        facet_markers: (1..=12).collect(),
        ..Default::default()
    }
}

fn audit(m: &Mesh3D, volume: f64) {
    assert!(!m.tetrahedra.is_empty());
    assert!(
        (m.report.total_volume - volume).abs() < 1e-8 * volume,
        "{} != {volume}",
        m.report.total_volume
    );
    let mut incidence = BTreeMap::new();
    for (i, t) in m.tetrahedra.iter().enumerate() {
        let p = t.map(|v| m.nodes[v]);
        let a = std::array::from_fn::<_, 3, _>(|k| p[1][k] - p[0][k]);
        let b = std::array::from_fn::<_, 3, _>(|k| p[2][k] - p[0][k]);
        let c = std::array::from_fn::<_, 3, _>(|k| p[3][k] - p[0][k]);
        let det = a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0]);
        assert!(det > 0.0);
        for k in 0..4 {
            let mut f: Vec<_> = t
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != k)
                .map(|(_, v)| *v)
                .collect();
            f.sort_unstable();
            incidence.entry(f).or_insert_with(Vec::new).push((i, k));
        }
    }
    for cells in incidence.values() {
        assert!(cells.len() == 1 || cells.len() == 2);
        if cells.len() == 2 {
            let (a, ka) = cells[0];
            let (b, kb) = cells[1];
            assert_eq!(m.neighbors[a][ka], b as i64);
            assert_eq!(m.neighbors[b][kb], a as i64);
        } else {
            assert_eq!(m.neighbors[cells[0].0][cells[0].1], -1);
        }
    }
    for (f, c) in m.faces.iter().zip(&m.face_cells) {
        if c[1] == -1 {
            let p = f.map(|v| m.nodes[v]);
            let t = m.tetrahedra[c[0] as usize];
            let opposite = *t.iter().find(|v| !f.contains(v)).unwrap();
            let a = std::array::from_fn::<_, 3, _>(|k| p[1][k] - p[0][k]);
            let b = std::array::from_fn::<_, 3, _>(|k| p[2][k] - p[0][k]);
            let q = std::array::from_fn::<_, 3, _>(|k| m.nodes[opposite][k] - p[0][k]);
            let normal = [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ];
            assert!((0..3).map(|k| normal[k] * q[k]).sum::<f64>() < 0.0);
        }
    }
}
#[test]
fn cube_constraints_markers_and_determinism() {
    let mut p = cube();
    p.vertex_markers = vec![-99; 8];
    let m = tetrahedralize(&p, &Options3D::default()).unwrap();
    audit(&m, 1.0);
    assert!(m.report.complete);
    assert!(m.face_facets.iter().all(|&v| v >= 0));
    for (&f, &marker) in m.face_facets.iter().zip(&m.face_markers) {
        assert_eq!(marker, p.facet_markers[f as usize]);
    }
    for (v, &i) in m.input_vertex_nodes.iter().enumerate() {
        assert!(i >= 0);
        assert_eq!(m.nodes[i as usize], p.vertices[v]);
        assert_eq!(m.node_markers[i as usize], -99);
    }
    let n = tetrahedralize(&p, &Options3D::default()).unwrap();
    assert_eq!(m.nodes, n.nodes);
    assert_eq!(m.tetrahedra, n.tetrahedra);
}
#[test]
fn inclined_material_interface_and_regional_volume() {
    let mut p = Plc3D::default();
    for layer in 0..3 {
        for [x, y] in [[0., 0.], [1., 0.], [1., 1.], [0., 1.]] {
            let z = match layer {
                0 => 0.,
                1 => 0.3 + 0.2 * x,
                _ => 1.,
            };
            p.vertices.push([x, y, z]);
        }
    }
    p.facets.extend([
        [0, 1, 2],
        [0, 2, 3],
        [8, 9, 10],
        [8, 10, 11],
        [4, 5, 6],
        [4, 6, 7],
    ]);
    p.facet_markers = vec![-2, -2, -1, -1, 12, 12];
    for layer in 0..2 {
        for k in 0..4 {
            let a = layer * 4 + k;
            let b = layer * 4 + (k + 1) % 4;
            p.facets.extend([[a, b, b + 4], [a, b + 4, a + 4]]);
            p.facet_markers.extend([-2, -2]);
        }
    }
    p.regions = vec![
        Region3D {
            point: [0.23, 0.31, 0.1],
            marker: 2,
            max_volume: 0.04,
        },
        Region3D {
            point: [0.23, 0.31, 0.8],
            marker: 3,
            max_volume: 0.1,
        },
    ];
    let m = tetrahedralize(&p, &Options3D::default()).unwrap();
    audit(&m, 1.0);
    assert!(m.report.complete, "{:?}", m.report);
    for (t, &marker) in m.tetrahedra.iter().zip(&m.cell_markers) {
        let ds = t.map(|v| m.nodes[v][2] - 0.3 - 0.2 * m.nodes[v][0]);
        assert!(if marker == 2 {
            ds.iter().all(|&d| d <= 1e-10)
        } else {
            assert_eq!(marker, 3);
            ds.iter().all(|&d| d >= -1e-10)
        });
    }
    assert!(m
        .face_markers
        .iter()
        .zip(&m.face_cells)
        .any(|(&k, c)| k == 12 && c[1] >= 0));
}
#[test]
fn nonconvex_l_domain() {
    let xy = [[0., 0.], [2., 0.], [2., 1.], [1., 1.], [1., 2.], [0., 2.]];
    let mut p = Plc3D::default();
    for z in [0., 1.] {
        p.vertices.extend(xy.map(|[x, y]| [x, y, z]));
    }
    let cap = [[0, 1, 3], [1, 2, 3], [0, 3, 5], [3, 4, 5]];
    p.facets.extend(cap);
    p.facets.extend(cap.map(|f| f.map(|v| v + 6)));
    for a in 0..6 {
        let b = (a + 1) % 6;
        p.facets.extend([[a, b, b + 6], [a, b + 6, a + 6]]);
    }
    let m = tetrahedralize(&p, &Options3D::default()).unwrap();
    audit(&m, 3.0);
    for t in &m.tetrahedra {
        let x = t.iter().map(|&v| m.nodes[v][0]).sum::<f64>() / 4.;
        let y = t.iter().map(|&v| m.nodes[v][1]).sum::<f64>() / 4.;
        assert!(x <= 1. || y <= 1.);
    }
}
#[test]
fn internal_tetrahedron_hole() {
    let mut p = cube();
    let offset = p.vertices.len();
    p.vertices.extend([
        [0.3, 0.3, 0.3],
        [0.7, 0.3, 0.3],
        [0.3, 0.7, 0.3],
        [0.3, 0.3, 0.7],
    ]);
    p.facets
        .extend([[0, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]].map(|f| f.map(|v| v + offset)));
    p.facet_markers.extend([-3; 4]);
    p.holes.push([0.4, 0.4, 0.4]);
    let m = tetrahedralize(&p, &Options3D::default()).unwrap();
    audit(&m, 1. - 0.4f64.powi(3) / 6.);
    assert!(m
        .face_markers
        .iter()
        .zip(&m.face_cells)
        .any(|(&k, c)| k == -3 && c[1] == -1));
}
#[test]
fn invalid_inputs_fail_without_panicking() {
    let mut p = cube();
    p.facets.pop();
    assert!(tetrahedralize(&p, &Options3D::default()).is_err());
    let mut p = cube();
    p.vertices[0][0] = f64::NAN;
    assert!(tetrahedralize(&p, &Options3D::default()).is_err());
    let mut p = cube();
    p.facets[0][0] = 99;
    assert!(tetrahedralize(&p, &Options3D::default()).is_err());
    let mut p = cube();
    p.vertices[7] = p.vertices[0];
    assert!(tetrahedralize(&p, &Options3D::default()).is_err());
    let mut p = cube();
    p.regions.push(Region3D {
        point: [0.5, 0.5, 0.],
        marker: 2,
        max_volume: 0.,
    });
    assert!(tetrahedralize(&p, &Options3D::default()).is_err());
    let mut p = cube();
    p.facets.clear();
    p.facet_markers.clear();
    for v in &mut p.vertices {
        v[2] = 0.;
    }
    assert!(tetrahedralize(&p, &Options3D::default()).is_err());
}

#[test]
fn explicit_surface_and_edge_electrodes() {
    let mut p = cube();
    p.vertices
        .extend([[0.23, 0.31, 1.0], [0.5, 0.0, 1.0], [0.5, 0.5, 0.5]]);
    p.vertex_markers = vec![0; 11];
    p.vertex_markers[8..].fill(-99);
    let m = tetrahedralize(
        &p,
        &Options3D {
            max_vertices: 300,
            max_passes: 20,
            ..Default::default()
        },
    )
    .unwrap();
    audit(&m, 1.0);
    for i in 8..11 {
        let j = m.input_vertex_nodes[i];
        assert!(j >= 0);
        assert_eq!(m.nodes[j as usize], p.vertices[i]);
        assert_eq!(m.node_markers[j as usize], -99);
    }
}

#[test]
fn cospherical_grid_and_unique_coplanar_input() {
    let mut p = Plc3D::default();
    for x in 0..3 {
        for y in 0..3 {
            for z in 0..3 {
                p.vertices.push([x as f64, y as f64, z as f64]);
            }
        }
    }
    let m = tetrahedralize(&p, &Options3D::default()).unwrap();
    audit(&m, 8.);
    assert!(m.input_vertex_nodes.iter().all(|&n| n >= 0));
    let p = Plc3D {
        vertices: vec![[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]],
        ..Default::default()
    };
    assert!(tetrahedralize(&p, &Options3D::default()).is_err());
}

#[test]
fn very_thin_tetrahedron_preserves_orientation() {
    let p = Plc3D {
        vertices: vec![
            [0., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [0.25, 0.25, 1e-12],
        ],
        ..Default::default()
    };
    let m = tetrahedralize(
        &p,
        &Options3D {
            min_dihedral: 5.,
            ..Default::default()
        },
    )
    .unwrap();
    audit(&m, 1e-12 / 6.);
    assert!(!m.report.complete);
    assert!(m.report.dihedral_violations > 0);
}

/// A torus surface of 10 x 8 vertices, and its volume. Its quads are planar
/// and cocircular, so the Delaunay cells on the surface can be flat.
fn torus() -> (Plc3D, f64) {
    let (nu, nv) = (10, 8);
    let mut p = Plc3D::default();
    for i in 0..nu {
        for j in 0..nv {
            let u = 2.0 * std::f64::consts::PI * i as f64 / nu as f64;
            let v = 2.0 * std::f64::consts::PI * j as f64 / nv as f64;
            let r = 2.0 + 0.6 * v.cos();
            p.vertices.push([r * u.cos(), r * u.sin(), 0.6 * v.sin()]);
        }
    }
    for i in 0..nu {
        for j in 0..nv {
            let a = i * nv + j;
            let b = ((i + 1) % nu) * nv + j;
            let c = ((i + 1) % nu) * nv + (j + 1) % nv;
            let d = i * nv + (j + 1) % nv;
            p.facets.extend([[a, b, c], [a, c, d]]);
        }
    }
    let volume = p
        .facets
        .iter()
        .map(|f| {
            let [a, b, c] = f.map(|k| p.vertices[k]);
            a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0])
        })
        .sum::<f64>()
        .abs()
        / 6.0;
    (p, volume)
}

#[test]
fn dihedral_target_is_met_or_reported_without_failing() {
    let (p, volume) = torus();
    let options = |target: f64| Options3D {
        max_vertices: 3000,
        min_dihedral: target,
        ..Default::default()
    };
    let met = tetrahedralize(&p, &options(5.0)).unwrap();
    audit(&met, volume);
    assert!(met.report.complete, "{:?}", met.report);
    assert!(met.report.min_dihedral >= 5.0, "{:?}", met.report);
    // A target that cannot be reached within a small budget is reported as
    // unmet.
    let unmet = tetrahedralize(
        &p,
        &Options3D {
            max_vertices: 120,
            max_passes: 8,
            min_dihedral: 80.,
            ..Default::default()
        },
    )
    .unwrap();
    audit(&unmet, volume);
    assert!(!unmet.report.complete);
    assert!(unmet.report.dihedral_violations > 0);
    assert!(!unmet.report.issues.is_empty());
    assert!(unmet.report.min_dihedral > 5.0, "{:?}", unmet.report);
}
