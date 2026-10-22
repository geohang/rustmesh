//! Random 2D geometries, meshed and checked against the drawing itself:
//! which zone (or hole, or nothing) lies at sample points, every point a
//! node, every line made of mesh edges, no degenerate cell, every area limit
//! met, and the same mesh twice. The cases are drawn from a fixed seed, so a
//! failure names a case that can be rerun on its own.

use rustmesh::{Domain, Geometry2D, Line, Mesh2D, Point, Shape, Sizing, Zone};

/// xorshift64*: small, fast and the same on every platform.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.next()
    }
    fn int(&mut self, a: usize, b: usize) -> usize {
        a + (self.next() * (b - a + 1) as f64) as usize % (b - a + 1)
    }
}

fn star(rng: &mut Rng, c: [f64; 2], r: f64, n: usize, jitter: f64) -> Vec<[f64; 2]> {
    let mut angles: Vec<f64> = (0..n)
        .map(|_| rng.range(0.0, std::f64::consts::TAU))
        .collect();
    angles.sort_by(f64::total_cmp);
    angles
        .into_iter()
        .map(|a| {
            let rr = r * (1.0 - jitter * rng.next());
            [c[0] + rr * a.cos(), c[1] + rr * a.sin()]
        })
        .collect()
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<[f64; 2]> {
    vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
}

fn area(p: &[[f64; 2]]) -> f64 {
    let o = p[0];
    0.5 * (1..p.len() - 1)
        .map(|i| (p[i][0] - o[0]) * (p[i + 1][1] - o[1]) - (p[i + 1][0] - o[0]) * (p[i][1] - o[1]))
        .sum::<f64>()
}

fn segment_distance(q: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let len2 = d[0] * d[0] + d[1] * d[1];
    let t = if len2 > 0.0 {
        (((q[0] - a[0]) * d[0] + (q[1] - a[1]) * d[1]) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    ((q[0] - a[0] - t * d[0]).powi(2) + (q[1] - a[1] - t * d[1]).powi(2)).sqrt()
}

/// One random geometry: `case % 6` picks the kind of domain, and every
/// fourth case is moved to UTM-sized coordinates, every eighth at a
/// millimetre scale, where the coordinates keep only a few digits for it.
fn geometry(rng: &mut Rng, case: usize) -> Geometry2D {
    let mut scale = [1e-3, 1.0, 1.0, 50.0, 1e4][rng.int(0, 4)];
    let offset = if case % 4 == 3 {
        [5.0e5, 4.6e6]
    } else {
        [0.0, 0.0]
    };
    if case % 8 == 3 {
        scale = 1e-3;
    }
    let t = |p: Vec<[f64; 2]>| -> Vec<[f64; 2]> {
        p.into_iter()
            .map(|[x, y]| [offset[0] + scale * x, offset[1] + scale * y])
            .collect()
    };
    let outline = match case % 6 {
        0 => {
            let n = rng.int(3, 12);
            star(rng, [0.0, 0.0], 1.0, n, 0.4)
        }
        1 => rect(-1.0, -1.0, 1.0, 0.3),
        2 => vec![
            [-1.0, -1.0],
            [1.0, -1.0],
            [1.0, 0.0],
            [0.0, 0.0],
            [0.0, 1.0],
            [-1.0, 1.0],
        ],
        3 => {
            let n = rng.int(3, 30);
            let phase = rng.next();
            let mut p = vec![[-1.0, -1.0], [1.0, -1.0]];
            p.extend((0..n).rev().map(|i| {
                let x = -1.0 + 2.0 * i as f64 / (n - 1) as f64;
                [x, 0.1 * (3.0 * x + phase).sin()]
            }));
            p
        }
        4 => rect(-1.0, -0.02, 1.0, 0.02),
        _ => {
            let n = rng.int(5, 9);
            star(rng, [0.0, 0.0], 1.0, n, 0.1)
        }
    };
    let edges = outline.len();
    let mut g = Geometry2D {
        name: format!("case {case}"),
        domain: Domain {
            outline: t(outline),
            edge_markers: (0..edges).map(|_| rng.int(1, 4) as i32).collect(),
        },
        background: Zone::new("background", 1, 0.0),
        ..Geometry2D::default()
    };
    for k in 0..rng.int(0, 5) {
        let kind = rng.next();
        let c = [rng.range(-0.8, 0.8), rng.range(-0.8, 0.5)];
        let outline = if case % 6 == 1 && kind < 0.4 {
            rect(-1.0, -1.0, 1.0, rng.range(-0.9, 0.2)) // a layer sharing the domain's edges
        } else if kind < 0.6 {
            let (r, n) = (rng.range(0.05, 0.6), rng.int(3, 10));
            star(rng, c, r, n, 0.4)
        } else if kind < 0.8 {
            let (w, h) = (rng.range(0.02, 0.8), rng.range(0.005, 0.5));
            rect(c[0] - w, c[1] - h, c[0] + w, c[1] + h)
        } else {
            let r = rng.range(0.05, 0.4);
            (0..24)
                .map(|i| {
                    let a = std::f64::consts::TAU * i as f64 / 24.0;
                    [c[0] + r * a.cos(), c[1] + r * a.sin()]
                })
                .collect()
        };
        let hole = rng.next() < 0.25;
        g.shapes.push(Shape {
            zone: Zone::new(
                &format!("zone {k}"),
                if hole { 0 } else { 2 + k as i32 },
                0.0,
            ),
            outline: t(outline),
            edge_marker: 20 + k as i32,
            hole,
        });
    }
    for k in 0..rng.int(0, 3) {
        let n = rng.int(2, 4);
        let points = (0..n)
            .map(|_| [rng.range(-1.2, 1.2), rng.range(-1.2, 1.2)])
            .collect();
        g.lines.push(Line {
            name: format!("line {k}"),
            points: t(points),
            marker: 50 + k as i32,
        });
    }
    for k in 0..rng.int(0, 25) {
        let at = t(vec![[rng.range(-1.1, 1.1), rng.range(-1.1, 1.1)]])[0];
        g.points.push(Point {
            at,
            marker: -(100 + k as i32),
            refine: rng.next() < 0.7,
        });
    }
    let domain_area = area(&g.domain.outline).abs();
    g.sizing = Sizing {
        min_angle: rng.range(20.0, 30.0),
        max_area: domain_area / rng.range(50.0, 1500.0),
        point_size: [0.0, 0.02, 0.05][rng.int(0, 2)] * scale,
        size_growth: rng.range(0.1, 0.8),
    };
    g
}

fn edges_of(g: &Geometry2D) -> Vec<([f64; 2], [f64; 2])> {
    let mut out = Vec::new();
    for poly in std::iter::once(&g.domain.outline).chain(g.shapes.iter().map(|s| &s.outline)) {
        for i in 0..poly.len() {
            out.push((poly[i], poly[(i + 1) % poly.len()]));
        }
    }
    for l in &g.lines {
        out.extend(l.points.windows(2).map(|w| (w[0], w[1])));
    }
    out
}

fn cell_at(m: &Mesh2D, q: [f64; 2]) -> Vec<usize> {
    (0..m.triangles.len())
        .filter(|&t| {
            let [a, b, c] = m.triangles[t].map(|n| m.nodes[n]);
            let side = |p: [f64; 2], r: [f64; 2]| {
                (r[0] - p[0]) * (q[1] - p[1]) - (r[1] - p[1]) * (q[0] - p[0])
            };
            side(a, b) >= 0.0 && side(b, c) >= 0.0 && side(c, a) >= 0.0
        })
        .collect()
}

fn min_angle(p: [[f64; 2]; 3]) -> f64 {
    (0..3)
        .map(|i| {
            let (a, b, c) = (p[i], p[(i + 1) % 3], p[(i + 2) % 3]);
            let (u, v) = ([b[0] - a[0], b[1] - a[1]], [c[0] - a[0], c[1] - a[1]]);
            (u[0] * v[1] - u[1] * v[0])
                .abs()
                .atan2(u[0] * v[0] + u[1] * v[1])
                .to_degrees()
        })
        .fold(180.0, f64::min)
}

/// What is wrong with the mesh of `g`, one line each.
fn check(g: &Geometry2D, m: &Mesh2D, rng: &mut Rng) -> Vec<String> {
    let mut problems = Vec::new();
    if !m.report.refinement_complete || m.report.area_violations != 0 {
        problems.push(format!("refinement/area failure: {:?}", m.report.issues));
    }
    // This corpus checks geometry and area; angle gaps are a separate result.
    assert_eq!(
        m.report.quality_met,
        m.report.angle_violations == 0 && m.report.area_violations == 0
    );
    assert_eq!(
        m.report.complete,
        m.report.refinement_complete && m.report.quality_met
    );
    for t in 0..m.triangles.len() {
        let p = m.triangles[t].map(|n| m.nodes[n]);
        let twice =
            (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
        if twice <= 0.0 || min_angle(p) < 0.01 {
            problems.push(format!("cell {t} is degenerate or clockwise: {p:?}"));
            break;
        }
    }
    let (lo, hi) = g.bounds();
    let size = (hi[0] - lo[0]).max(hi[1] - lo[1]);
    let far = lo.iter().chain(&hi).fold(0.0f64, |a, v| a.max(v.abs()));
    let tol = (1e-9 * size).max(64.0 * f64::EPSILON * far);
    let edges = edges_of(g);
    let near_edge = |q: [f64; 2], d: f64| edges.iter().any(|&(a, b)| segment_distance(q, a, b) < d);
    let meshed = |q: [f64; 2]| g.zone_at(q).is_some();
    // Zones, holes and the outside, at random points clear of every outline.
    let mut wrong = 0;
    for _ in 0..300 {
        let q = [rng.range(lo[0], hi[0]), rng.range(lo[1], hi[1])];
        if near_edge(q, 1e-5 * size) {
            continue;
        }
        let want = g.zone_at(q).map(|z| z.marker);
        let cells = cell_at(m, q);
        let got = match cells.as_slice() {
            [] => None,
            [t] => Some(m.cell_markers[*t]),
            _ => {
                problems.push(format!("cells {cells:?} overlap at {q:?}"));
                continue;
            }
        };
        if got != want {
            wrong += 1;
            if wrong <= 2 {
                problems.push(format!(
                    "at {q:?} the drawing has {want:?}, the mesh {got:?}"
                ));
            }
        }
    }
    // Every point inside the meshed part is a node with its marker.
    for (i, p) in g.points.iter().enumerate() {
        if !meshed(p.at) || near_edge(p.at, 10.0 * tol) {
            continue;
        }
        match m
            .nodes
            .iter()
            .position(|n| (n[0] - p.at[0]).abs() <= tol && (n[1] - p.at[1]).abs() <= tol)
        {
            Some(n) if m.node_markers[n] == p.marker => {}
            Some(n) => problems.push(format!("point {i} has node marker {}", m.node_markers[n])),
            None => problems.push(format!("point {i} at {:?} is not a node", p.at)),
        }
    }
    // Every line, inside the meshed part, runs along mesh edges.
    for l in &g.lines {
        for w in l.points.windows(2) {
            for k in 1..8 {
                let s = k as f64 / 8.0;
                let q = [
                    w[0][0] + s * (w[1][0] - w[0][0]),
                    w[0][1] + s * (w[1][1] - w[0][1]),
                ];
                let others = edges.iter().filter(|&&(a, b)| (a, b) != (w[0], w[1]));
                if !meshed(q)
                    || others
                        .clone()
                        .any(|&(a, b)| segment_distance(q, a, b) < 1e-4 * size)
                {
                    continue;
                }
                let on = m
                    .edges
                    .iter()
                    .any(|e| segment_distance(q, m.nodes[e[0]], m.nodes[e[1]]) <= 8.0 * tol);
                if !on {
                    problems.push(format!("{} at {q:?} is not on a mesh edge", l.name));
                    break;
                }
            }
        }
    }
    if wrong > 2 {
        problems.push(format!("{wrong} wrong samples in all"));
    }
    problems
}

#[test]
fn random_geometries_mesh_as_drawn() {
    let mut rng = Rng(0x5eed_2d00_0000_0001);
    let mut failures = Vec::new();
    for case in 0..120 {
        let g = geometry(&mut rng, case);
        let m = match g.mesh(200_000) {
            Ok(m) => m,
            Err(e) => {
                failures.push(format!("case {case}: {e}"));
                continue;
            }
        };
        let problems = check(&g, &m, &mut rng);
        if !problems.is_empty() {
            failures.push(format!("case {case}: {}", problems.join("; ")));
        }
        let again = g.mesh(200_000).expect("meshed once already");
        if again.nodes != m.nodes || again.triangles != m.triangles {
            failures.push(format!("case {case}: a second run gave another mesh"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of 120 cases failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// A 0.2 mm square with a round zone and 41 refined points, at the origin
/// and at UTM coordinates. Far from the origin the coordinates keep about
/// six digits for it; meshed there as they are, its smallest angle fell to
/// 27.9 degrees of the 30 asked.
#[test]
fn far_from_the_origin_meshes_as_well_as_at_it() {
    let tiny = |offset: [f64; 2]| {
        let scale = 1e-4;
        let t = |p: [f64; 2]| [offset[0] + scale * p[0], offset[1] + scale * p[1]];
        let circle = (0..32)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / 32.0;
                t([0.3 * a.cos(), 0.3 * a.sin()])
            })
            .collect();
        Geometry2D {
            domain: Domain {
                outline: rect(-1.0, -1.0, 1.0, 1.0).into_iter().map(t).collect(),
                edge_markers: vec![1, 2, 3, 4],
            },
            shapes: vec![Shape {
                zone: Zone::new("circle", 2, 0.0),
                outline: circle,
                edge_marker: 5,
                hole: false,
            }],
            points: (0..41)
                .map(|k| Point {
                    at: t([-0.9 + 1.8 * k as f64 / 40.0, 0.95]),
                    marker: -99,
                    refine: true,
                })
                .collect(),
            sizing: Sizing {
                min_angle: 30.0,
                max_area: 4.0 * scale * scale / 2000.0,
                point_size: 0.01 * scale,
                size_growth: 0.3,
            },
            ..Geometry2D::default()
        }
    };
    let here = tiny([0.0, 0.0]).mesh(200_000).unwrap();
    let there = tiny([5.0e5, 4.6e6]).mesh(200_000).unwrap();
    for m in [&here, &there] {
        assert!(m.report.complete, "{:?}", m.report);
        assert!(m.report.min_angle >= 30.0 - 1e-6, "{:?}", m.report);
    }
    let (a, b) = (here.nodes.len() as f64, there.nodes.len() as f64);
    assert!(
        (a - b).abs() < 0.02 * a,
        "{a} nodes at the origin, {b} far from it"
    );
}

/// A strip of ground over a hole, crossed by two lines (case 157 of a
/// random run). A cell next to the hole had its circumcentre in the hole,
/// and the refinement passed over it, leaving it larger than the area
/// limit; now the edge in between is split instead.
#[test]
fn a_cell_whose_circumcentre_lies_in_a_hole_still_meets_its_area_limit() {
    let g = Geometry2D {
        domain: Domain {
            outline: rect(-50.0, -50.0, 50.0, 15.0),
            edge_markers: vec![3, 1, 4, 4],
        },
        shapes: vec![Shape {
            zone: Zone::new("hole", 0, 0.0),
            outline: rect(-50.0, -50.0, 50.0, 3.190715864604571),
            edge_marker: 22,
            hole: true,
        }],
        lines: vec![
            Line {
                name: "l0".into(),
                points: vec![
                    [51.34307348794819, -20.81892725686237],
                    [4.734683846543075, -0.33902252875886996],
                ],
                marker: 50,
            },
            Line {
                name: "l1".into(),
                points: vec![
                    [-51.08058538587337, 54.73254612005819],
                    [2.1872989658569653, -1.2388868934118058],
                ],
                marker: 51,
            },
        ],
        sizing: Sizing {
            min_angle: 24.07011231075301,
            max_area: 2.8555650313619876,
            point_size: 0.0,
            size_growth: 0.38,
        },
        ..Geometry2D::default()
    };
    let m = g.mesh(100_000).unwrap();
    assert!(m.report.complete, "{:?}", m.report);
    assert_eq!(m.report.area_violations, 0);
}
