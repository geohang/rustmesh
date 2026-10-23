//! Experimental conforming 3D meshing. See docs/mesh3d.md for the contract.
use crate::MeshError;
use robust::{insphere, orient2d, orient3d, Coord, Coord3D};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

mod local_refinement;
mod refinement;
mod smoothing;
mod surface;
pub use refinement::{tetrahedralize_quality, Quality3D, QualityReport3D};
pub use surface::{
    prepare_surface_patches, tetrahedralize_patched, FacetCoverage3D, PatchedMesh3D,
    PreparedSurface3D,
};

type Point = [f64; 3];
type Tet = [usize; 4];
type Face = [usize; 3];
type FaceMap = BTreeMap<Face, Vec<(usize, usize)>>;

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Region3D {
    pub point: Point,
    pub marker: i32,
    /// Zero means no regional volume limit.
    pub max_volume: f64,
}

/// Triangulated piecewise linear complex. Facets must meet along complete
/// shared edges/vertices. Closed outer shells and closed cavities are required;
/// internal interfaces may meet the shell. Facet winding is immaterial.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Plc3D {
    pub vertices: Vec<Point>,
    pub vertex_markers: Vec<i32>,
    pub facets: Vec<Face>,
    pub facet_markers: Vec<i32>,
    pub regions: Vec<Region3D>,
    pub holes: Vec<Point>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Options3D {
    pub max_volume: f64,
    pub max_vertices: usize,
    /// Shared budget for boundary recovery and volume refinement rebuilds.
    pub max_passes: usize,
    /// Target smallest dihedral angle in degrees (0: off). Cells below it are
    /// refined, best effort, within the vertex budget. Global rebuilds use
    /// max_passes; the subsequent local optimizer adds at most 16 trial rounds.
    pub min_dihedral: f64,
}

/// Opt-in experimental edge sizing on a recovered mesh. Lengths are in input
/// units. This does not move input points or use global Delaunay reconstruction.
#[derive(Debug, Clone)]
pub struct Sizing3D {
    pub points: Vec<Point>,
    pub near_size: f64,
    pub far_size: f64,
    pub growth: f64,
    pub max_splits: usize,
    /// Total shape-based flip sweeps, interleaved with at most three interior
    /// contraction passes. Zero disables both topology operations.
    pub flip_passes: usize,
}
impl Sizing3D {
    fn edge_ratio(&self, a: Point, b: Point) -> f64 {
        let distance = self
            .points
            .iter()
            .map(|&p| norm(sub(a, p)).min(norm(sub(b, p))))
            .fold(f64::INFINITY, f64::min);
        norm(sub(a, b)) / self.far_size.min(self.near_size + self.growth * distance)
    }
}
impl Default for Options3D {
    fn default() -> Self {
        Self {
            max_volume: 0.0,
            max_vertices: 10_000,
            max_passes: 100,
            min_dihedral: 0.0,
        }
    }
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Report3D {
    /// Present only for the opt-in constrained Delaunay refinement path.
    #[cfg_attr(feature = "serde", serde(default))]
    pub quality: Option<QualityReport3D>,
    /// All constraints recovered and requested volume/dihedral/sizing/quality limits met.
    /// This is not a guarantee of FEM accuracy or of general PLC support.
    pub complete: bool,
    pub issues: Vec<String>,
    pub warnings: Vec<String>,
    pub passes: usize,
    pub steiner_points: usize,
    pub volume_violations: usize,
    pub dihedral_violations: usize,
    /// Violations at an edge where two boundary or interface faces of the
    /// cell meet, i.e. at an angle the input geometry itself forms.
    pub forced_dihedrals: usize,
    /// Passes that inserted vertices to destroy cells below `min_dihedral`.
    pub sliver_passes: usize,
    /// Edge removals and 2-3 flips that improved the worst angle locally.
    pub flips: usize,
    /// Generated interior vertices removed by validated star contractions.
    #[cfg_attr(feature = "serde", serde(default))]
    pub interior_collapses: usize,
    /// Accepted relocations along the selected output candidate path.
    pub smoothing_moves: usize,
    pub smoothing_sweeps: usize,
    /// Edge and cell subdivisions retained on the selected candidate path.
    pub local_edge_splits: usize,
    pub local_cell_splits: usize,
    /// Local trial rounds attempted, including discarded trials (at most 16).
    pub local_refinement_passes: usize,
    /// Quantiles of the per-cell minimum dihedral angle (degrees).
    pub min_dihedral_p05: f64,
    pub min_dihedral_median: f64,
    pub min_dihedral_p95: f64,
    /// Cells with a minimum dihedral below one degree, regardless of target.
    pub sliver_count: usize,
    /// Circumradius / shortest edge. The explicit target, when requested, is in quality.
    /// None if any cell's shape metrics could not be evaluated finitely.
    pub radius_edge_median: Option<f64>,
    pub radius_edge_p95: Option<f64>,
    pub radius_edge_max: Option<f64>,
    /// 12 (3V)^(2/3) / sum(edge_length^2): 1 for a regular tetrahedron,
    /// approaching 0 at degeneration, including slivers with modest radius-edge.
    pub mean_ratio_min: Option<f64>,
    pub mean_ratio_median: Option<f64>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub shape_metric_failures: usize,
    /// Present only for the opt-in recovered-mesh sizing path.
    pub sizing_edge_violations: Option<usize>,
    pub sizing_max_edge_ratio: Option<f64>,
    pub min_dihedral: f64,
    pub min_volume: f64,
    pub max_volume: f64,
    pub total_volume: f64,
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Mesh3D {
    pub nodes: Vec<Point>,
    pub node_markers: Vec<i32>,
    pub input_vertex_nodes: Vec<i64>,
    /// Positive determinant det(b-a,c-a,d-a).
    pub tetrahedra: Vec<Tet>,
    pub cell_markers: Vec<i32>,
    pub cell_regions: Vec<i32>,
    /// Neighbor opposite each local vertex; -1 on the boundary.
    pub neighbors: Vec<[i64; 4]>,
    /// Boundary faces and recovered internal constraints. Boundary faces point out.
    pub faces: Vec<Face>,
    pub face_markers: Vec<i32>,
    pub face_facets: Vec<i64>,
    /// Incident cells, -1 for the exterior.
    pub face_cells: Vec<[i64; 2]>,
    pub report: Report3D,
}

fn bad(s: impl Into<String>) -> MeshError {
    MeshError::BadOption(s.into())
}
fn failed(s: impl Into<String>) -> MeshError {
    MeshError::Insertion(s.into())
}
fn coord(p: Point) -> Coord3D<f64> {
    Coord3D {
        x: p[0],
        y: p[1],
        z: p[2],
    }
}
fn orient(a: Point, b: Point, c: Point, d: Point) -> f64 {
    orient3d(coord(a), coord(b), coord(c), coord(d))
}
fn sorted<const N: usize>(mut a: [usize; N]) -> [usize; N] {
    a.sort_unstable();
    a
}
fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|k| a[k] - b[k])
}
fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: Point, b: Point) -> f64 {
    (0..3).map(|k| a[k] * b[k]).sum()
}
fn norm(a: Point) -> f64 {
    a[0].hypot(a[1]).hypot(a[2])
}
fn face(t: Tet, k: usize) -> Face {
    match k {
        0 => [t[1], t[2], t[3]],
        1 => [t[0], t[3], t[2]],
        2 => [t[0], t[1], t[3]],
        _ => [t[0], t[2], t[1]],
    }
}
fn face_map(tets: &[Tet]) -> Result<FaceMap, MeshError> {
    let mut map: FaceMap = BTreeMap::new();
    for (i, &t) in tets.iter().enumerate() {
        for k in 0..4 {
            let entries = map.entry(sorted(face(t, k))).or_default();
            entries.push((i, k));
            if entries.len() > 2 {
                return Err(failed("non-manifold tetrahedral cavity"));
            }
        }
    }
    Ok(map)
}
fn triangle_area(p: [Point; 3]) -> f64 {
    norm(cross(sub(p[1], p[0]), sub(p[2], p[0]))) * 0.5
}

/// Scale and translate before measuring shape. Unlike the angle target, these
/// diagnostics expose both long/thin cells and near-coplanar slivers. A failed
/// floating-point evaluation gives `None`, never a made-up quality.
fn shape_metrics(p: [Point; 4]) -> Option<(f64, f64)> {
    let mut q = p.map(|x| sub(x, p[0]));
    let scale = q
        .iter()
        .flat_map(|x| x.iter())
        .map(|x| x.abs())
        .fold(0., f64::max);
    if !scale.is_finite() || scale <= 0. {
        return None;
    }
    for x in &mut q {
        for c in x {
            *c /= scale;
        }
    }
    let mut shortest = f64::INFINITY;
    let mut sum = 0.;
    for i in 0..4 {
        for j in i + 1..4 {
            let length = norm(sub(q[i], q[j]));
            shortest = shortest.min(length);
            sum += length * length;
        }
    }
    let volume = orient(q[0], q[1], q[2], q[3]).abs() / 6.;
    let (_, radius) = circumsphere(q)?;
    let ratio = radius / shortest;
    let mean = 12. * (3. * volume).powf(2. / 3.) / sum;
    if volume > 0. && ratio.is_finite() && ratio > 0. && mean.is_finite() && mean > 0. {
        Some((ratio, mean.min(1.)))
    } else {
        None
    }
}
fn collinear(p: [Point; 3]) -> bool {
    [(0, 1), (0, 2), (1, 2)].iter().all(|&(i, j)| {
        let c = |q: Point| Coord { x: q[i], y: q[j] };
        orient2d(c(p[0]), c(p[1]), c(p[2])) == 0.0
    })
}

impl Plc3D {
    pub fn validate(&self) -> Result<(), MeshError> {
        if self.vertices.len() < 4 {
            return Err(bad("3D meshing needs at least four vertices"));
        }
        for (what, len, expected) in [
            (
                "vertex markers",
                self.vertex_markers.len(),
                self.vertices.len(),
            ),
            ("facet markers", self.facet_markers.len(), self.facets.len()),
        ] {
            if len != 0 && len != expected {
                return Err(MeshError::MarkerCount {
                    what,
                    expected,
                    got: len,
                });
            }
        }
        for (i, p) in self.vertices.iter().enumerate() {
            if !p.iter().all(|x| x.is_finite()) {
                return Err(MeshError::NonFiniteVertex { vertex: i });
            }
        }
        let mut order: Vec<_> = (0..self.vertices.len()).collect();
        order.sort_by(|&i, &j| {
            (0..3)
                .map(|k| self.vertices[i][k].total_cmp(&self.vertices[j][k]))
                .find(|c| !c.is_eq())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        // Normalize signed zeros for duplicate detection.
        let mut seen = BTreeMap::new();
        for &i in &order {
            let key = self.vertices[i].map(|x| if x == 0.0 { 0 } else { x.to_bits() });
            if let Some(&j) = seen.get(&key) {
                return Err(MeshError::DuplicateVertex {
                    first: j,
                    second: i,
                });
            }
            seen.insert(key, i);
        }
        let mut facets = BTreeSet::new();
        let mut edges = BTreeMap::new();
        for (i, &f) in self.facets.iter().enumerate() {
            if f.iter().any(|&v| v >= self.vertices.len()) {
                return Err(bad(format!("facet {i} has an invalid vertex index")));
            }
            if !facets.insert(sorted(f)) {
                return Err(bad(format!("duplicate facet {i}")));
            }
            if collinear(f.map(|v| self.vertices[v])) {
                return Err(bad(format!("degenerate facet {i}")));
            }
            for k in 0..3 {
                *edges.entry(sorted([f[k], f[(k + 1) % 3]])).or_insert(0) += 1;
            }
        }
        if edges.values().any(|&n| n < 2) {
            return Err(bad("PLC has an open edge; outer and cavity shells must be closed and interfaces must meet conformingly"));
        }
        for (i, r) in self.regions.iter().enumerate() {
            if !r.point.iter().all(|v| v.is_finite())
                || !r.max_volume.is_finite()
                || r.max_volume < 0.0
            {
                return Err(bad(format!("invalid region {i}")));
            }
        }
        if self.holes.iter().flatten().any(|v| !v.is_finite()) {
            return Err(bad("hole coordinates must be finite"));
        }
        if self.facets.is_empty() && !self.holes.is_empty() {
            return Err(bad("holes require a closed PLC"));
        }
        Ok(())
    }
}

/// Incremental Bowyer-Watson kernel with an explicit convex hull. Visible
/// hull faces play the role of cells incident to a symbolic vertex at infinity;
/// no finite super-tetrahedron or arbitrarily large bounding coordinates occur.
/// Full conflict scans prioritize inspectability over production performance.
fn delaunay(points: &[Point]) -> Result<Vec<Tet>, MeshError> {
    let a = 0;
    let b = (1..points.len())
        .find(|&i| points[i] != points[a])
        .ok_or_else(|| failed("all 3D points coincide"))?;
    let c = (0..points.len())
        .find(|&i| !collinear([points[a], points[b], points[i]]))
        .ok_or_else(|| failed("all 3D points are collinear"))?;
    let d = (0..points.len())
        .find(|&i| orient(points[a], points[b], points[c], points[i]) != 0.0)
        .ok_or_else(|| failed("all 3D points are coplanar"))?;
    let mut first = [a, b, c, d];
    if orient(points[a], points[b], points[c], points[d]) < 0.0 {
        first.swap(0, 1);
    }
    let mut tets = vec![first];
    for (v, &p) in points.iter().enumerate() {
        crate::control::checkpoint(points.len(), tets.len(), false)?;
        if first.contains(&v) {
            continue;
        }
        let map = face_map(&tets)?;
        let bad: Vec<_> = tets
            .iter()
            .map(|t| {
                let q = t.map(|i| coord(points[i]));
                // New points win exact cospherical ties, consistently with their
                // insertion order; coordinates are never jittered.
                insphere(q[0], q[1], q[2], q[3], coord(p)) >= 0.0
            })
            .collect();
        let mut boundary = Vec::new();
        for (f, cells) in map {
            if cells.len() == 2 {
                if bad[cells[0].0] != bad[cells[1].0] {
                    boundary.push(f);
                }
            } else {
                let (i, k) = cells[0];
                let oriented = face(tets[i], k).map(|j| points[j]);
                let visible = orient(oriented[0], oriented[1], oriented[2], p) > 0.0;
                // A face belongs to the cavity boundary iff precisely one
                // of its finite and symbolic exterior cells is in conflict.
                if bad[i] != visible {
                    boundary.push(f);
                }
            }
        }
        let mut next: Vec<_> = tets
            .into_iter()
            .zip(&bad)
            .filter(|(_, b)| !**b)
            .map(|(t, _)| t)
            .collect();
        let before = next.len();
        for f in boundary {
            let mut t = [f[0], f[1], f[2], v];
            let o = orient(points[t[0]], points[t[1]], points[t[2]], p);
            if !o.is_finite() {
                return Err(failed("non-finite insertion orientation"));
            }
            if o == 0.0 {
                continue;
            }
            if o < 0.0 {
                t.swap(0, 1);
            }
            next.push(t);
            crate::control::checkpoint(points.len(), next.len(), false)?;
        }
        if next.len() == before {
            return Err(failed(format!(
                "no nondegenerate insertion cavity for vertex {v}"
            )));
        }
        tets = next;
    }
    let map = face_map(&tets)?;
    let used: BTreeSet<_> = tets.iter().flatten().copied().collect();
    if used.len() != points.len() {
        return Err(failed("a point was lost during Delaunay insertion"));
    }
    for cells in map.values().filter(|c| c.len() == 1) {
        let (i, k) = cells[0];
        let f = face(tets[i], k).map(|j| points[j]);
        if points.iter().any(|&p| orient(f[0], f[1], f[2], p) > 0.0) {
            return Err(failed("Delaunay hull verification failed"));
        }
    }
    Ok(tets)
}

#[derive(Clone)]
struct Constraint {
    vertices: Face,
    parent: usize,
}
// Split the facets at input vertices (electrodes) that already lie on a PLC
// face or edge, before adding Steiner points. Every incident facet is split, preserving conformity.
fn split_at_input_vertices(mut facets: Vec<Constraint>, points: &[Point]) -> Vec<Constraint> {
    for (v, &p) in points.iter().enumerate() {
        let mut next = Vec::new();
        for c in facets {
            let f = c.vertices;
            if f.contains(&v) {
                next.push(c);
                continue;
            }
            let q = f.map(|i| points[i]);
            if orient(q[0], q[1], q[2], p) != 0.0 {
                next.push(c);
                continue;
            }
            let normal = cross(sub(q[1], q[0]), sub(q[2], q[0]));
            let drop = (0..3)
                .max_by(|&i, &j| normal[i].abs().total_cmp(&normal[j].abs()))
                .unwrap();
            let i = (drop + 1) % 3;
            let j = (drop + 2) % 3;
            let project = |q: Point| Coord { x: q[i], y: q[j] };
            let signs = std::array::from_fn::<_, 3, _>(|k| {
                orient2d(project(q[k]), project(q[(k + 1) % 3]), project(p))
            });
            if !(signs.iter().all(|&x| x >= 0.0) || signs.iter().all(|&x| x <= 0.0)) {
                next.push(c);
                continue;
            }
            for k in 0..3 {
                if signs[k] != 0.0 {
                    next.push(Constraint {
                        vertices: [f[k], f[(k + 1) % 3], v],
                        parent: c.parent,
                    });
                }
            }
        }
        facets = next;
    }
    facets
}
fn locate(
    p: Point,
    points: &[Point],
    tets: &[Tet],
    map: &FaceMap,
    constraints: &BTreeMap<Face, usize>,
) -> Result<usize, MeshError> {
    let mut found = None;
    for (i, &t) in tets.iter().enumerate() {
        let o = std::array::from_fn::<_, 4, _>(|k| {
            let f = face(t, k);
            orient(points[f[0]], points[f[1]], points[f[2]], p)
        });
        if o.iter().all(|&x| x <= 0.0) {
            for (k, &orientation) in o.iter().enumerate() {
                if orientation == 0.0 {
                    let f = sorted(face(t, k));
                    if constraints.contains_key(&f) || map[&f].len() == 1 {
                        return Err(bad(
                            "a region/hole seed lies on a constraint or hull boundary",
                        ));
                    }
                }
            }
            found = Some(i);
        }
    }
    found.ok_or_else(|| bad("a region/hole seed is outside the convex hull"))
}
fn flood(
    start: usize,
    value: i32,
    labels: &mut [i32],
    tets: &[Tet],
    map: &FaceMap,
    constraints: &BTreeMap<Face, usize>,
) {
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from([start]);
    while let Some(i) = queue.pop_front() {
        if !seen.insert(i) {
            continue;
        }
        labels[i] = value;
        for k in 0..4 {
            let f = sorted(face(tets[i], k));
            if !constraints.contains_key(&f) {
                for &(j, _) in &map[&f] {
                    if j != i {
                        queue.push_back(j);
                    }
                }
            }
        }
    }
}
fn classify(
    plc: &Plc3D,
    points: &[Point],
    tets: &[Tet],
    map: &FaceMap,
    barriers: &BTreeMap<Face, usize>,
    transform: impl Fn(Point) -> Point,
) -> Result<Vec<i32>, MeshError> {
    let mut labels = vec![-1; tets.len()];
    if !plc.facets.is_empty() {
        for (f, entries) in map {
            if entries.len() == 1 && !barriers.contains_key(f) && labels[entries[0].0] != -2 {
                flood(entries[0].0, -2, &mut labels, tets, map, barriers);
            }
        }
    }
    for &h in &plc.holes {
        let i = locate(transform(h), points, tets, map, barriers)?;
        if labels[i] == -2 {
            return Err(bad(
                "hole seed lies in the exterior or an already excluded cavity",
            ));
        }
        flood(i, -2, &mut labels, tets, map, barriers);
    }
    for (r, region) in plc.regions.iter().enumerate() {
        let i = locate(transform(region.point), points, tets, map, barriers)?;
        if labels[i] == -2 {
            return Err(bad(format!("region {r} lies in the exterior or a hole")));
        }
        flood(i, r as i32, &mut labels, tets, map, barriers);
    }
    if labels.iter().all(|&l| l == -2) {
        return Err(failed("PLC encloses no volume"));
    }
    Ok(labels)
}
fn limit(global: f64, regional: f64) -> f64 {
    match (global > 0.0, regional > 0.0) {
        (true, true) => global.min(regional),
        (true, false) => global,
        (false, true) => regional,
        _ => f64::INFINITY,
    }
}
/// The six dihedral angles in degrees, each with its edge as local vertex
/// indices. atan2 keeps the resolution near 0 and 180 degrees that acos of a
/// normalised dot product loses, and a degenerate face gives 0, not NaN.
fn dihedrals(p: [Point; 4]) -> [(f64, [usize; 2]); 6] {
    let normals: [Point; 4] = std::array::from_fn(|k| {
        let f = face([0, 1, 2, 3], k);
        cross(sub(p[f[1]], p[f[0]]), sub(p[f[2]], p[f[0]]))
    });
    let mut out = [(0.0, [0, 0]); 6];
    let mut n = 0;
    for i in 0..4 {
        for j in i + 1..4 {
            // Faces i and j (opposite vertices i and j) meet at the other two.
            let mut edge = (0..4).filter(|&k| k != i && k != j);
            let edge = [edge.next().unwrap(), edge.next().unwrap()];
            let angle = norm(cross(normals[i], normals[j]))
                .atan2(-dot(normals[i], normals[j]))
                .to_degrees();
            out[n] = (angle, edge);
            n += 1;
        }
    }
    out
}
/// The worst dihedral of a cell and the local indices of its edge.
fn worst_dihedral(p: [Point; 4]) -> (f64, [usize; 2]) {
    dihedrals(p)
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .unwrap()
}
/// Whether `q` lies strictly inside the diametral sphere of triangle `t` (the
/// smallest sphere through its corners), i.e. encroaches it.
fn encroaches(q: Point, t: [Point; 3]) -> bool {
    let a = sub(t[1], t[0]);
    let b = sub(t[2], t[0]);
    let axb = cross(a, b);
    let denom = 2.0 * dot(axb, axb);
    if denom == 0.0 {
        return false;
    }
    let w: Point = std::array::from_fn(|k| dot(a, a) * b[k] - dot(b, b) * a[k]);
    let offset = cross(w, axb);
    let centre: Point = std::array::from_fn(|k| t[0][k] + offset[k] / denom);
    let radius = norm(sub(t[0], centre));
    norm(sub(q, centre)) < radius * (1.0 - 1e-9)
}
fn midpoint(a: Point, b: Point) -> Point {
    std::array::from_fn(|k| a[k] * 0.5 + b[k] * 0.5)
}
/// Splits each edge at its midpoint, in every constraint piece that has it.
fn split_edges(
    edges: impl IntoIterator<Item = [usize; 2]>,
    points: &mut Vec<Point>,
    world: &mut Vec<Point>,
    mut constraints: Vec<Constraint>,
    transform: &impl Fn(Point) -> Point,
    max_vertices: usize,
) -> Result<Vec<Constraint>, MeshError> {
    for [a, b] in edges {
        if points.len() >= max_vertices {
            return Err(failed("boundary recovery exhausted max_vertices"));
        }
        let p = midpoint(world[a], world[b]);
        let q = transform(p);
        // The midpoint can exist already: a facet adopting the volume mesh's
        // own triangulation may have dropped it from its pieces, and the same
        // edge is now split again. Reuse that vertex; it is on the edge.
        let v = match points.iter().position(|&x| x == q) {
            Some(existing) if existing == a || existing == b => {
                return Err(failed(
                    "boundary recovery stagnated: an edge is below floating-point resolution",
                ));
            }
            Some(existing) => existing,
            None => {
                points.push(q);
                world.push(p);
                points.len() - 1
            }
        };
        let mut split = Vec::new();
        for c in constraints {
            if c.vertices.contains(&a) && c.vertices.contains(&b) {
                let other = *c.vertices.iter().find(|&&k| k != a && k != b).unwrap();
                split.push(Constraint {
                    vertices: [a, v, other],
                    parent: c.parent,
                });
                split.push(Constraint {
                    vertices: [v, b, other],
                    parent: c.parent,
                });
            } else {
                split.push(c);
            }
        }
        constraints = split;
    }
    Ok(constraints)
}

/// Index of a cell containing `p` (on its boundary included), if any.
fn find_cell(p: Point, points: &[Point], tets: &[Tet]) -> Option<usize> {
    tets.iter().position(|&t| {
        (0..4).all(|k| {
            let f = face(t, k);
            orient(points[f[0]], points[f[1]], points[f[2]], p) <= 0.0
        })
    })
}
/// Centre and radius of the sphere through a cell's corners, if it is well
/// enough conditioned to be worth using.
fn circumsphere(p: [Point; 4]) -> Option<(Point, f64)> {
    let a = sub(p[1], p[0]);
    let b = sub(p[2], p[0]);
    let c = sub(p[3], p[0]);
    let det = dot(a, cross(b, c));
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let (aa, bb, cc) = (dot(a, a), dot(b, b), dot(c, c));
    let (bc, ca, ab) = (cross(b, c), cross(c, a), cross(a, b));
    let offset: Point =
        std::array::from_fn(|k| (aa * bc[k] + bb * ca[k] + cc * ab[k]) / (2.0 * det));
    let centre: Point = std::array::from_fn(|k| p[0][k] + offset[k]);
    let radius = norm(offset);
    radius.is_finite().then_some((centre, radius))
}

/// Local topological improvement of the cells below a dihedral target:
/// edge removal (the 3-2, 4-4, 5-6, ... flips, with the best retriangulation
/// of the edge's ring chosen by Klincsek's dynamic programme; J. R. Shewchuk,
/// "Two discrete optimization algorithms for the topological improvement of
/// tetrahedral meshes") and 2-3 face flips. A change is kept
/// only when its selected worst-cell score strictly improves (dihedral for
/// ordinary meshing, normalized mean ratio for experimental edge sizing).
/// Constraint faces and edges are never removed, every change stays inside
/// one region, and every new cell has a positive exact orientation.
struct Flipper<'a> {
    shape_guard: bool,
    shape_objective: bool,
    angle_floor: f64,
    edge_field: Option<&'a Sizing3D>,
    points: &'a [Point],
    world: &'a [Point],
    volume_limits: &'a BTreeMap<i32, f64>,
    tets: Vec<Tet>,
    labels: Vec<i32>,
    alive: Vec<bool>,
    incident: Vec<Vec<usize>>,
    barrier_faces: &'a BTreeMap<Face, usize>,
    constraint_edges: &'a BTreeSet<[usize; 2]>,
}

impl<'a> Flipper<'a> {
    fn new(
        points: &'a [Point],
        tets: Vec<Tet>,
        labels: Vec<i32>,
        barrier_faces: &'a BTreeMap<Face, usize>,
        constraint_edges: &'a BTreeSet<[usize; 2]>,
        world: &'a [Point],
        volume_limits: &'a BTreeMap<i32, f64>,
    ) -> Self {
        let mut incident = vec![Vec::new(); points.len()];
        for (i, t) in tets.iter().enumerate() {
            for &v in t {
                incident[v].push(i);
            }
        }
        let alive = vec![true; tets.len()];
        Self {
            shape_guard: false,
            shape_objective: false,
            angle_floor: 0.,
            edge_field: None,
            points,
            world,
            volume_limits,
            tets,
            labels,
            alive,
            incident,
            barrier_faces,
            constraint_edges,
        }
    }

    fn quality(&self, t: Tet) -> f64 {
        if self.shape_objective {
            shape_metrics(t.map(|v| self.points[v])).map_or(0., |(_, m)| m)
        } else {
            worst_dihedral(t.map(|v| self.points[v])).0
        }
    }

    /// Contract a generated, strictly interior vertex onto an existing neighbour.
    /// The entire vertex star is a transaction: its boundary, material, volume,
    /// constraints and positive orientations must survive `replace` unchanged.
    fn collapse(&mut self, from: usize, to: usize, input_vertices: usize) -> bool {
        if from < input_vertices
            || from == to
            || self.constraint_edges.iter().any(|e| e.contains(&from))
        {
            return false;
        }
        let old = self.cells_with(&[from]);
        if old.is_empty() || !old.iter().any(|&i| self.tets[i].contains(&to)) {
            return false;
        }
        let old_tets: Vec<_> = old.iter().map(|&i| self.tets[i]).collect();
        let Ok(faces) = face_map(&old_tets) else {
            return false;
        };
        if faces.iter().any(|(f, c)| c.len() == 1 && f.contains(&from)) {
            return false;
        }
        let new: Vec<_> = old_tets
            .iter()
            .filter(|t| !t.contains(&to))
            .map(|t| t.map(|v| if v == from { to } else { v }))
            .collect();
        if new.is_empty() || new.iter().any(|&t| self.orient(t) <= 0.) {
            return false;
        }
        let before = old_tets
            .iter()
            .map(|&t| self.quality(t))
            .fold(f64::INFINITY, f64::min);
        let after = new
            .iter()
            .map(|&t| self.quality(t))
            .fold(f64::INFINITY, f64::min);
        if after <= before + 1e-9 {
            return false;
        }
        self.replace(&old, new)
    }

    /// Short edges first; live incidence is rechecked after every transaction.
    fn collapse_pass(&mut self, input_vertices: usize) -> Result<usize, MeshError> {
        let mut edges: Vec<_> = self
            .tets
            .iter()
            .enumerate()
            .filter(|(i, _)| self.alive[*i])
            .flat_map(|(_, t)| {
                (0..4).flat_map(move |i| (i + 1..4).map(move |j| sorted([t[i], t[j]])))
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        edges.sort_by(|a, b| {
            let length = |e: &[usize; 2]| norm(sub(self.points[e[0]], self.points[e[1]]));
            length(a).total_cmp(&length(b)).then(a.cmp(b))
        });
        let mut count = 0;
        for (i, [a, b]) in edges.into_iter().enumerate() {
            if i % 128 == 0 {
                crate::control::checkpoint(self.world.len(), self.tets.len(), true)?;
            }
            if self.collapse(a, b, input_vertices) || self.collapse(b, a, input_vertices) {
                count += 1;
            }
        }
        Ok(count)
    }

    fn orient(&self, t: Tet) -> f64 {
        orient(
            self.points[t[0]],
            self.points[t[1]],
            self.points[t[2]],
            self.points[t[3]],
        )
    }

    /// Live cells containing all of `vertices`.
    fn cells_with(&self, vertices: &[usize]) -> Vec<usize> {
        self.incident[vertices[0]]
            .iter()
            .copied()
            .filter(|&i| self.alive[i] && vertices.iter().all(|v| self.tets[i].contains(v)))
            .collect()
    }

    /// Validate the entire replacement before mutating the live complex.
    /// Positive individual cells alone do not establish a valid cavity.
    fn replace(&mut self, old: &[usize], new: Vec<Tet>) -> bool {
        if old.is_empty() || new.is_empty() {
            return false;
        }
        let label = self.labels[old[0]];
        if label == -2
            || old
                .iter()
                .any(|&i| !self.alive[i] || self.labels[i] != label)
        {
            return false;
        }
        let old_tets: Vec<_> = old.iter().map(|&i| self.tets[i]).collect();
        let (Ok(before), Ok(after)) = (face_map(&old_tets), face_map(&new)) else {
            return false;
        };
        let boundary = |map: &FaceMap| {
            map.iter()
                .filter(|(_, c)| c.len() == 1)
                .map(|(f, _)| *f)
                .collect::<BTreeSet<_>>()
        };
        if boundary(&before) != boundary(&after) {
            return false;
        }
        if before
            .keys()
            .any(|f| self.barrier_faces.contains_key(f) && !after.contains_key(f))
        {
            return false;
        }
        let mut distinct = BTreeSet::new();
        if new.iter().any(|&t| !distinct.insert(sorted(t))) {
            return false;
        }
        // An interior replacement face must not already be used outside the cavity.
        if after.iter().any(|(f, cells)| {
            cells.len() == 2 && self.cells_with(f).iter().any(|i| !old.contains(i))
        }) {
            return false;
        }
        let volume = |t: Tet| {
            let p = t.map(|v| self.world[v]);
            orient(p[0], p[1], p[2], p[3]) / 6.0
        };
        let cap = self.volume_limits[&label];
        if new.iter().any(|&t| {
            let v = volume(t);
            !v.is_finite() || v <= 0.0 || v > cap * (1.0 + 1e-12)
        }) {
            return false;
        }
        let previous: f64 = old_tets.iter().map(|&t| volume(t)).sum();
        let proposed: f64 = new.iter().map(|&t| volume(t)).sum();
        if (previous - proposed).abs() > 1e-10 * previous {
            return false;
        }
        if self.shape_guard {
            let summary = |cells: &[Tet]| -> Option<(f64, f64)> {
                let mut radius = 0_f64;
                let mut mean = 1_f64;
                for &t in cells {
                    let (r, m) = shape_metrics(t.map(|v| self.world[v]))?;
                    radius = radius.max(r);
                    mean = mean.min(m);
                }
                Some((radius, mean))
            };
            let (Some(a), Some(b)) = (summary(&old_tets), summary(&new)) else {
                return false;
            };
            if b.0 > a.0 * (1. + 1e-12) || b.1 < a.1 * (1. - 1e-12) {
                return false;
            }
            let min_angle = |cells: &[Tet]| {
                cells
                    .iter()
                    .map(|t| worst_dihedral(t.map(|v| self.points[v])).0)
                    .fold(180_f64, f64::min)
            };
            if min_angle(&new) + 1e-9 < min_angle(&old_tets).min(self.angle_floor) {
                return false;
            }
        }
        if let Some(field) = self.edge_field {
            let world = self.world;
            let worst = |cells: &[Tet]| {
                cells
                    .iter()
                    .flat_map(|t| {
                        (0..4).flat_map(move |i| {
                            (i + 1..4).map(move |j| field.edge_ratio(world[t[i]], world[t[j]]))
                        })
                    })
                    .fold(0_f64, f64::max)
            };
            if worst(&new) > worst(&old_tets).max(1.) * (1. + 1e-10) {
                return false;
            }
        }
        for &i in old {
            self.alive[i] = false;
        }
        for t in new {
            let i = self.tets.len();
            self.tets.push(t);
            self.labels.push(label);
            self.alive.push(true);
            for &v in &t {
                self.incident[v].push(i);
            }
        }
        true
    }

    /// Removes edge `[u, v]` if a retriangulation of its ring improves the
    /// worst angle there.
    fn remove_edge(&mut self, u: usize, v: usize) -> bool {
        if self.constraint_edges.contains(&sorted([u, v])) {
            return false;
        }
        let ring_cells = self.cells_with(&[u, v]);
        let m = ring_cells.len();
        if !(3..=10).contains(&m) {
            return false;
        }
        let label = self.labels[ring_cells[0]];
        if label == -2 || ring_cells.iter().any(|&i| self.labels[i] != label) {
            return false;
        }
        // Each ring cell contributes one ring edge; a closed ring is a cycle.
        let pairs: Vec<[usize; 2]> = ring_cells
            .iter()
            .map(|&i| {
                let mut rest = self.tets[i].iter().copied().filter(|&w| w != u && w != v);
                [rest.next().unwrap(), rest.next().unwrap()]
            })
            .collect();
        if pairs.iter().any(|&[a, b]| {
            self.barrier_faces.contains_key(&sorted([u, v, a]))
                || self.barrier_faces.contains_key(&sorted([u, v, b]))
        }) {
            return false;
        }
        let mut ring = vec![pairs[0][0], pairs[0][1]];
        let mut used = vec![false; m];
        used[0] = true;
        while ring.len() < m {
            let last = *ring.last().unwrap();
            let Some(k) = (0..m).find(|&k| !used[k] && pairs[k].contains(&last)) else {
                return false;
            };
            used[k] = true;
            let next = if pairs[k][0] == last {
                pairs[k][1]
            } else {
                pairs[k][0]
            };
            if ring.contains(&next) {
                return false;
            }
            ring.push(next);
        }
        let closing = (0..m).find(|&k| !used[k]).unwrap();
        if !(pairs[closing].contains(&ring[0]) && pairs[closing].contains(&ring[m - 1])) {
            return false;
        }
        // Orient the ring so that the old cells (u, v, r_i, r_i+1) are positive.
        let sign = |r: &[usize]| {
            (0..m)
                .map(|i| self.orient([u, v, r[i], r[(i + 1) % m]]).signum())
                .collect::<Vec<_>>()
        };
        let signs = sign(&ring);
        if signs.iter().any(|&s| s != signs[0]) || signs[0] == 0.0 {
            return false;
        }
        if signs[0] < 0.0 {
            ring.reverse();
        }
        let old_quality = ring_cells
            .iter()
            .map(|&i| self.quality(self.tets[i]))
            .fold(180.0, f64::min);
        // A triangle (a, b, c) of the ring, in ring order, makes the cells
        // (a, b, c, v) and (a, c, b, u); both must be positive and new.
        let triangle = |i: usize, j: usize, k: usize| -> Option<(f64, [Tet; 2])> {
            let (a, b, c) = (ring[i], ring[j], ring[k]);
            let below = [a, b, c, v];
            let above = [a, c, b, u];
            if self.orient(below) <= 0.0 || self.orient(above) <= 0.0 {
                return None;
            }
            if !self.cells_with(&[a, b, c]).is_empty() {
                return None;
            }
            // A chord of the ring must not already bound a face with u or v
            // on the far side of the ring.
            for (x, y) in [(i, j), (j, k), (i, k)] {
                let adjacent = y - x == 1 || (x == 0 && y == m - 1);
                if !adjacent
                    && (!self.cells_with(&[u, ring[x], ring[y]]).is_empty()
                        || !self.cells_with(&[v, ring[x], ring[y]]).is_empty())
                {
                    return None;
                }
            }
            Some((self.quality(below).min(self.quality(above)), [below, above]))
        };
        // best[i][j]: best worst-angle triangulating ring[i..=j], and the apex.
        let mut best = vec![vec![(f64::NEG_INFINITY, usize::MAX); m]; m];
        for gap in 2..m {
            for i in 0..m - gap {
                let j = i + gap;
                for k in i + 1..j {
                    let Some((q, _)) = triangle(i, k, j) else {
                        continue;
                    };
                    let left = if k - i >= 2 { best[i][k].0 } else { 180.0 };
                    let right = if j - k >= 2 { best[k][j].0 } else { 180.0 };
                    let value = q.min(left).min(right);
                    if value > best[i][j].0 {
                        best[i][j] = (value, k);
                    }
                }
            }
        }
        if best[0][m - 1].0 <= old_quality + 1e-9 {
            return false;
        }
        let mut new = Vec::new();
        let mut stack = vec![(0, m - 1)];
        while let Some((i, j)) = stack.pop() {
            if j - i < 2 {
                continue;
            }
            let k = best[i][j].1;
            new.extend(triangle(i, k, j).unwrap().1);
            stack.push((i, k));
            stack.push((k, j));
        }
        self.replace(&ring_cells, new)
    }

    /// The 2-3 flip across face `k` of cell `i`, if it improves the worst
    /// angle of the two cells.
    fn flip23(&mut self, i: usize, k: usize) -> bool {
        let mut f = face(self.tets[i], k);
        if self.barrier_faces.contains_key(&sorted(f)) {
            return false;
        }
        let pair = self.cells_with(&f);
        if pair.len() != 2 {
            return false;
        }
        let j = if pair[0] == i { pair[1] } else { pair[0] };
        if self.labels[i] != self.labels[j] || self.labels[i] == -2 {
            return false;
        }
        let p = self.tets[i][k];
        let q = *self.tets[j].iter().find(|w| !f.contains(w)).unwrap();
        if self.orient([f[0], f[1], f[2], p]) < 0.0 {
            f.swap(0, 1);
        }
        let new = [[f[1], f[0], p, q], [f[2], f[1], p, q], [f[0], f[2], p, q]];
        if new.iter().any(|&t| self.orient(t) <= 0.0) {
            return false;
        }
        let old = self.quality(self.tets[i]).min(self.quality(self.tets[j]));
        let improved = new.iter().map(|&t| self.quality(t)).fold(180.0, f64::min);
        if improved <= old + 1e-9 {
            return false;
        }
        self.replace(&[i, j], new.to_vec())
    }

    /// Sweeps over the cells below `target`, worst first, until a sweep
    /// changes nothing. Returns the live cells and their labels.
    fn run(mut self, target: f64, max_sweeps: usize) -> (Vec<Tet>, Vec<i32>, usize) {
        let mut flips = 0;
        for _ in 0..max_sweeps {
            if crate::control::checkpoint(self.world.len(), self.tets.len(), true).is_err() {
                break;
            }
            let mut bad: Vec<(f64, usize)> = (0..self.tets.len())
                .filter(|&i| self.alive[i] && self.labels[i] != -2)
                .map(|i| (self.quality(self.tets[i]), i))
                .filter(|&(q, _)| q < target)
                .collect();
            bad.sort_by(|a, b| a.0.total_cmp(&b.0));
            let before = flips;
            for (_, i) in bad {
                if !self.alive[i] {
                    continue;
                }
                let t = self.tets[i];
                let mut edges: Vec<[usize; 2]> = (0..4)
                    .flat_map(|a| (a + 1..4).map(move |b| [a, b]))
                    .map(|[a, b]| [t[a], t[b]])
                    .collect();
                // Try the edges at the largest angles first: removing a
                // sliver's long diagonal is what flattens it away.
                let p = t.map(|v| self.points[v]);
                let angles = dihedrals(p);
                edges.sort_by(|e, f| {
                    let angle = |e: &[usize; 2]| {
                        angles
                            .iter()
                            .find(|(_, local)| {
                                let pair = [t[local[0]], t[local[1]]];
                                sorted(pair) == sorted(*e)
                            })
                            .map_or(0.0, |d| d.0)
                    };
                    angle(f).total_cmp(&angle(e))
                });
                if edges.iter().any(|&[a, b]| self.remove_edge(a, b))
                    || (0..4).any(|k| self.flip23(i, k))
                {
                    flips += 1;
                }
            }
            if flips == before {
                break;
            }
        }
        let mut tets = Vec::new();
        let mut labels = Vec::new();
        for i in 0..self.tets.len() {
            if self.alive[i] {
                tets.push(self.tets[i]);
                labels.push(self.labels[i]);
            }
        }
        (tets, labels, flips)
    }
}

/// Generate a conforming tetrahedral mesh, with bounded constraint recovery.
/// Failure to recover ANY facet is an error, never a partial geometric mesh.
/// Size/quality budget exhaustion returns a valid mesh with `complete=false`.
/// With no facets, meshes the convex hull. This is an experimental
/// implementation and does not cover every PLC.
pub fn tetrahedralize(plc: &Plc3D, options: &Options3D) -> Result<Mesh3D, MeshError> {
    tetrahedralize_impl(plc, options, None, None, None)
}

/// Experimental graded local refinement, followed by protected topology edits.
/// Unmet sizing/angle/volume goals remain visible through `report.complete`.
pub fn tetrahedralize_sized(
    plc: &Plc3D,
    options: &Options3D,
    sizing: &Sizing3D,
) -> Result<Mesh3D, MeshError> {
    validate_sizing(sizing)?;
    tetrahedralize_impl(plc, options, Some(sizing), None, None)
}

fn validate_sizing(sizing: &Sizing3D) -> Result<(), MeshError> {
    if sizing.points.is_empty()
        || sizing.points.iter().flatten().any(|v| !v.is_finite())
        || !sizing.near_size.is_finite()
        || sizing.near_size <= 0.
        || !sizing.far_size.is_finite()
        || sizing.far_size < sizing.near_size
        || !sizing.growth.is_finite()
        || sizing.growth < 0.
    {
        return Err(bad("invalid experimental 3D sizing field"));
    }
    Ok(())
}

fn tetrahedralize_impl(
    plc: &Plc3D,
    options: &Options3D,
    sizing: Option<&Sizing3D>,
    patches: Option<&surface::KernelPatches>,
    quality: Option<&Quality3D>,
) -> Result<Mesh3D, MeshError> {
    crate::control::checkpoint(plc.vertices.len(), 0, false)?;
    plc.validate()?;
    if !options.max_volume.is_finite()
        || options.max_volume < 0.0
        || !options.min_dihedral.is_finite()
        || !(0.0..=90.0).contains(&options.min_dihedral)
    {
        return Err(bad("invalid 3D volume or dihedral limit"));
    }
    if options.max_vertices < plc.vertices.len() || options.max_passes == 0 {
        return Err(bad(
            "3D budgets must fit the input and allow at least one pass",
        ));
    }
    let lo: Point = std::array::from_fn(|k| {
        plc.vertices
            .iter()
            .map(|p| p[k])
            .fold(f64::INFINITY, f64::min)
    });
    let hi: Point = std::array::from_fn(|k| {
        plc.vertices
            .iter()
            .map(|p| p[k])
            .fold(f64::NEG_INFINITY, f64::max)
    });
    let scale = (0..3).map(|k| hi[k] - lo[k]).fold(0.0, f64::max);
    if !scale.is_finite() || !(1e-90..=1e90).contains(&scale) {
        return Err(bad(
            "coordinate extent must be finite and between 1e-90 and 1e90",
        ));
    }
    // Power-of-two scaling preserves the input's binary coordinates exactly.
    // Translating by a rounded center and dividing by an arbitrary extent can
    // change the orientation of nearly coplanar tetrahedra before export.
    let magnitude = plc
        .vertices
        .iter()
        .flatten()
        .map(|x| x.abs())
        .fold(0.0, f64::max);
    let factor = 2.0f64.powi(-(magnitude.log2().floor() as i32));
    let transform = |p: Point| p.map(|x| x * factor);
    let mut world = plc.vertices.clone();
    let mut points: Vec<_> = world.iter().map(|&p| transform(p)).collect();
    let mut constraints: Vec<_> = plc
        .facets
        .iter()
        .enumerate()
        .map(|(i, &f)| Constraint {
            vertices: f,
            parent: patches.map_or(i, |p| p.representatives[i]),
        })
        .collect();
    constraints = split_at_input_vertices(constraints, &world);
    let mut report = Report3D::default();
    report.warnings.push(if quality.is_some() {
        "Experimental: global PLC recovery followed by bounded local constrained Delaunay insertion. Check the report for unmet goals before using the mesh.".into()
    } else {
        "Experimental: bounded boundary recovery and full-scan Delaunay rebuilds. Check the shape diagnostics before using the mesh. Quality improvement needs min_dihedral > 0 or the opt-in sizing topology passes.".into()
    });
    // Sliver removal stops after six passes that improve neither the number
    // of slivers nor the worst angle.
    let mut best_slivers = usize::MAX;
    let mut best_worst = 0.0f64;
    let mut sliver_stall = 0usize;
    let mut sliver_budget_hit = false;
    let mut sliver_unplaceable = false;
    // The state just before the latest sliver insertion, which was fully
    // recovered. When boundary recovery cannot follow an insertion (it fails,
    // or runs away past `recovery_allowance` vertices), that state is
    // restored and insertion stops, so asking for a quality target never
    // costs the mesh itself.
    let mut checkpoint: Option<(Vec<Point>, Vec<Point>, Vec<Constraint>)> = None;
    let mut recovery_allowance = usize::MAX;
    let mut sliver_rolled_back = false;
    // The best recovered state seen so far, already flipped, scored by the
    // oversized cells, cells below the angle target, then the worst angle.
    struct Candidate {
        score: (usize, usize, f64),
        world: Vec<Point>,
        constraints: Vec<Constraint>,
        tets: Vec<Tet>,
        labels: Vec<i32>,
        flips: usize,
    }
    let mut best: Option<Candidate> = None;
    // Refinement only adds vertices, so the domain's volume is fixed by the
    // first recovered state; a different volume later means classification
    // leaked through a gap in the recovered facets.
    let mut domain_volume: Option<f64> = None;
    let mut volume_limits = BTreeMap::from([(-1, limit(options.max_volume, 0.0))]);
    for (i, region) in plc.regions.iter().enumerate() {
        volume_limits.insert(i as i32, limit(options.max_volume, region.max_volume));
    }
    let flip_target = options.min_dihedral;
    let score =
        |points: &[Point], world: &[Point], tets: &[Tet], labels: &[i32]| -> (usize, usize, f64) {
            let mut oversized = 0;
            let mut below = 0;
            let mut worst = 180.0f64;
            for (t, &l) in tets.iter().zip(labels) {
                if l == -2 {
                    continue;
                }
                let p = t.map(|v| world[v]);
                if orient(p[0], p[1], p[2], p[3]) / 6.0 > volume_limits[&l] * (1.0 + 1e-12) {
                    oversized += 1;
                }
                let angle = worst_dihedral(t.map(|v| points[v])).0;
                worst = worst.min(angle);
                if angle < flip_target {
                    below += 1;
                }
            }
            (oversized, below, -worst)
        };
    macro_rules! roll_back_or {
        ($pass:lifetime, $error:expr) => {
            match checkpoint.take() {
                Some((p, w, c)) => {
                    points = p;
                    world = w;
                    constraints = c;
                    sliver_rolled_back = true;
                    sliver_stall = 6;
                    recovery_allowance = usize::MAX;
                    continue $pass;
                }
                None => return Err($error),
            }
        };
    }
    macro_rules! or_roll_back {
        ($pass:lifetime, $result:expr) => {
            match $result {
                Ok(value) => value,
                Err(error @ MeshError::Stopped(_)) => return Err(error),
                Err(error) => roll_back_or!($pass, error),
            }
        };
    }
    'pass: for pass in 0..options.max_passes {
        crate::control::checkpoint(points.len(), 0, true)?;
        report.passes = pass + 1;
        // Reserve the final rebuild for the known recovered checkpoint. An
        // experimental insertion must not consume the pass needed to undo it.
        if pass + 1 == options.max_passes {
            if let Some((p, w, c)) = checkpoint.take() {
                points = p;
                world = w;
                constraints = c;
                sliver_rolled_back = true;
                sliver_stall = 6;
            }
        }
        let tets = or_roll_back!('pass, delaunay(&points));
        let map = or_roll_back!('pass, face_map(&tets));
        // Subdivision diagonals are bookkeeping, not new physical constraints.
        // Adopt the volume kernel's triangulation of a parent facet whenever
        // its complete area is covered using that facet's own vertices. This
        // prevents artificial spoke edges near surface electrodes from causing
        // endless refinement. Original facet borders remain protected.
        let mut membership = vec![BTreeSet::new(); plc.facets.len()];
        for c in &constraints {
            membership[c.parent].extend(c.vertices);
        }
        let mut replacements: BTreeMap<usize, Vec<Constraint>> = BTreeMap::new();
        for (parent, vertices) in membership.iter().enumerate() {
            if vertices.is_empty() {
                continue;
            }
            let candidates: Vec<_> = map
                .keys()
                .filter(|f| f.iter().all(|v| vertices.contains(v)))
                .copied()
                .collect();
            if let Some(patches) = patches {
                if let Some(faces) = patches.recover(plc, &world, parent, &candidates)? {
                    replacements.insert(
                        parent,
                        faces
                            .into_iter()
                            .map(|vertices| Constraint { vertices, parent })
                            .collect(),
                    );
                }
                continue;
            }
            let area: f64 = candidates
                .iter()
                .map(|f| triangle_area(f.map(|v| points[v])))
                .sum();
            let expected = triangle_area(plc.facets[parent].map(|v| points[v]));
            if (area - expected).abs() <= expected * 1e-10 {
                replacements.insert(
                    parent,
                    candidates
                        .into_iter()
                        .map(|vertices| Constraint { vertices, parent })
                        .collect(),
                );
            }
        }
        if !replacements.is_empty() {
            constraints.retain(|c| !replacements.contains_key(&c.parent));
            for (_, pieces) in replacements {
                constraints.extend(pieces);
            }
        }
        let missing: Vec<_> = constraints
            .iter()
            .filter(|c| !map.contains_key(&sorted(c.vertices)))
            .cloned()
            .collect();
        if !missing.is_empty() {
            if checkpoint.is_some()
                && (pass + 2 >= options.max_passes || points.len() > recovery_allowance)
            {
                roll_back_or!('pass, failed("boundary recovery could not follow sliver refinement"));
            }
            if pass + 1 == options.max_passes {
                return Err(failed(format!(
                    "boundary recovery exhausted max_passes: {} facet pieces remain missing",
                    missing.len()
                )));
            }
            let mesh_edges: BTreeSet<_> = tets
                .iter()
                .flat_map(|t| {
                    (0..4).flat_map(move |i| (i + 1..4).map(move |j| sorted([t[i], t[j]])))
                })
                .collect();
            let mut edges = BTreeSet::new();
            let mut faces_to_split = BTreeSet::new();
            for c in missing {
                let edge = (0..3)
                    .map(|k| sorted([c.vertices[k], c.vertices[(k + 1) % 3]]))
                    .filter(|e| !mesh_edges.contains(e))
                    .max_by(|a, b| {
                        let length = |e: &[usize; 2]| {
                            dot(
                                sub(points[e[0]], points[e[1]]),
                                sub(points[e[0]], points[e[1]]),
                            )
                        };
                        length(a).total_cmp(&length(b)).then(a.cmp(b))
                    });
                if let Some(e) = edge {
                    edges.insert(e);
                } else {
                    faces_to_split.insert(sorted(c.vertices));
                }
            }
            // Recover missing segments first. Splitting an unrelated longest
            // edge can refine forever without repairing a short missing edge.
            if edges.is_empty() {
                let mut split = Vec::new();
                for c in constraints {
                    if !faces_to_split.contains(&sorted(c.vertices)) {
                        split.push(c);
                        continue;
                    }
                    if points.len() >= options.max_vertices {
                        roll_back_or!('pass, failed("facet recovery exhausted max_vertices"));
                    }
                    let p: Point = std::array::from_fn(|k| {
                        c.vertices.iter().map(|&v| world[v][k] / 3.0).sum()
                    });
                    let q = transform(p);
                    if points.contains(&q) {
                        roll_back_or!('pass, failed("facet recovery stagnated at an existing point"));
                    }
                    let v = points.len();
                    points.push(q);
                    world.push(p);
                    for k in 0..3 {
                        split.push(Constraint {
                            vertices: [c.vertices[k], c.vertices[(k + 1) % 3], v],
                            parent: c.parent,
                        });
                    }
                }
                constraints = split;
            }
            constraints = or_roll_back!('pass, split_edges(
                edges,
                &mut points,
                &mut world,
                constraints,
                &transform,
                options.max_vertices,
            ));
            continue;
        }
        let barriers: BTreeMap<_, _> = constraints
            .iter()
            .map(|c| (sorted(c.vertices), c.parent))
            .collect();
        let labels =
            or_roll_back!('pass, classify(plc, &points, &tets, &map, &barriers, transform));
        let mut additions = Vec::new();
        for (i, t) in tets.iter().enumerate() {
            if labels[i] == -2 {
                continue;
            }
            let p = t.map(|v| world[v]);
            let volume = orient(p[0], p[1], p[2], p[3]) / 6.0;
            let regional = if labels[i] >= 0 {
                plc.regions[labels[i] as usize].max_volume
            } else {
                0.0
            };
            if volume > limit(options.max_volume, regional) * (1.0 + 1e-12) {
                additions.push(std::array::from_fn::<_, 3, _>(|k| {
                    p.iter().map(|v| v[k] * 0.25).sum()
                }));
            }
        }
        if quality.is_none()
            && !additions.is_empty()
            && pass + 1 < options.max_passes
            && points.len() + additions.len() <= options.max_vertices
        {
            for p in additions {
                let q = transform(p);
                if points.contains(&q) {
                    roll_back_or!('pass, failed(
                        "volume refinement stagnated at floating-point resolution",
                    ));
                }
                points.push(q);
                world.push(p);
            }
            continue;
        }
        // Sliver removal by insertion. A vertex inside a cell's circumsphere
        // destroys it; the circumcentre is used when it lies inside the
        // domain, encroaches no constraint piece (so facets stay conforming,
        // and no boundary edge is split for a sliver) and keeps a quarter of
        // the cell's shortest edge from every vertex. Slivers on the boundary
        // and flat cells on (nearly) coplanar facets are left to the flips
        // below: a vertex in a flat region only makes more flat cells.
        let volume: f64 = tets
            .iter()
            .zip(&labels)
            .filter(|(_, &l)| l != -2)
            .map(|(t, _)| orient(world[t[0]], world[t[1]], world[t[2]], world[t[3]]) / 6.0)
            .sum();
        match domain_volume {
            None => domain_volume = Some(volume),
            Some(expected) if (volume - expected).abs() > 1e-9 * expected.abs() => {
                roll_back_or!('pass, failed("the recovered facets no longer enclose the domain (classification leaked)"));
            }
            Some(_) => {}
        }
        let constraint_edges: BTreeSet<[usize; 2]> = constraints
            .iter()
            .flat_map(|c| (0..3).map(move |k| sorted([c.vertices[k], c.vertices[(k + 1) % 3]])))
            .collect();
        // With no target the conforming Delaunay cells are returned as they
        // are (a point cloud gives exactly its Delaunay tetrahedralization).
        let (flipped_tets, flipped_labels, flips) =
            if quality.is_none() && options.min_dihedral > 0.0 {
                Flipper::new(
                    &points,
                    tets.clone(),
                    labels.clone(),
                    &barriers,
                    &constraint_edges,
                    &world,
                    &volume_limits,
                )
                .run(flip_target, 30)
            } else {
                (tets.clone(), labels.clone(), 0)
            };
        let current = score(&points, &world, &flipped_tets, &flipped_labels);
        if best.as_ref().is_none_or(|b| current < b.score) {
            best = Some(Candidate {
                score: current,
                world: world.clone(),
                constraints: constraints.clone(),
                tets: flipped_tets.clone(),
                labels: flipped_labels.clone(),
                flips,
            });
        }
        // Insert only where flips alone leave cells below the target.
        if quality.is_none()
            && options.min_dihedral > 0.0
            && current.1 > 0
            && pass + 1 < options.max_passes
            && sliver_stall < 6
        {
            let mut slivers = 0usize;
            let mut worst = 180.0f64;
            let mut free: Vec<Point> = Vec::new();
            let mut splits = BTreeSet::new();
            for (i, t) in tets.iter().enumerate() {
                if labels[i] == -2 {
                    continue;
                }
                let (angle, _) = worst_dihedral(t.map(|v| points[v]));
                if angle >= options.min_dihedral {
                    continue;
                }
                slivers += 1;
                worst = worst.min(angle);
                let q = t.map(|v| world[v]);
                let Some((centre, radius)) = circumsphere(q) else {
                    continue;
                };
                let lengths: Vec<f64> = (0..4)
                    .flat_map(|a| (a + 1..4).map(move |b| (a, b)))
                    .map(|(a, b)| norm(sub(q[a], q[b])))
                    .collect();
                let shortest = lengths.iter().copied().fold(f64::INFINITY, f64::min);
                let longest = lengths.iter().copied().fold(0.0, f64::max);
                if radius > 20.0 * longest {
                    continue;
                }
                match find_cell(transform(centre), &points, &tets) {
                    Some(cell) if labels[cell] != -2 => {}
                    _ => continue,
                }
                // An encroached constraint piece is split at its longest
                // edge instead (the encroachment rule of Delaunay refinement,
                // see docs/mesh3d.md), which also refines skinny boundary
                // triangles that no interior vertex can fix.
                if let Some(c) = constraints
                    .iter()
                    .find(|c| encroaches(centre, c.vertices.map(|v| world[v])))
                {
                    let longest = (0..3)
                        .map(|k| sorted([c.vertices[k], c.vertices[(k + 1) % 3]]))
                        .max_by(|e, f| {
                            let length = |e: &[usize; 2]| norm(sub(world[e[0]], world[e[1]]));
                            length(e).total_cmp(&length(f)).then(e.cmp(f))
                        })
                        .unwrap();
                    splits.insert(longest);
                    continue;
                }
                let gap = 0.25 * shortest;
                if world
                    .iter()
                    .chain(free.iter())
                    .any(|p| norm(sub(*p, centre)) < gap)
                {
                    continue;
                }
                free.push(centre);
            }
            if slivers < best_slivers || worst > best_worst {
                best_slivers = best_slivers.min(slivers);
                best_worst = best_worst.max(worst);
                sliver_stall = 0;
            } else {
                sliver_stall += 1;
            }
            let added = free.len() + splits.len();
            if slivers > 0 && added == 0 {
                sliver_unplaceable = true;
            }
            if slivers > 0 && added > 0 && sliver_stall < 6 {
                if points.len() + added <= options.max_vertices {
                    report.sliver_passes += 1;
                    checkpoint = Some((points.clone(), world.clone(), constraints.clone()));
                    recovery_allowance = points.len() + added + (points.len() / 2).max(100);
                    for centre in free {
                        points.push(transform(centre));
                        world.push(centre);
                    }
                    constraints = or_roll_back!('pass, split_edges(
                        splits,
                        &mut points,
                        &mut world,
                        constraints,
                        &transform,
                        options.max_vertices,
                    ));
                    continue;
                }
                sliver_budget_hit = true;
            }
        }
        // With a target, flips run on every recovered state; they also remove
        // the flat cells that (nearly) coplanar, cocircular input leaves on
        // facets. The best flipped state is exported, so sliver insertion can
        // never leave the mesh worse than flips alone.
        let (tets, labels) = match best.take() {
            Some(b) if b.score < current => {
                report.warnings.push(format!(
                    "Kept an earlier pass before final smoothing ({} cells below {} degrees): it ranked better by volume violations, angle violations, then worst angle",
                    b.score.1, flip_target
                ));
                world = b.world;
                constraints = b.constraints;
                report.flips = b.flips;
                (b.tets, b.labels)
            }
            _ => {
                report.flips = flips;
                (flipped_tets, flipped_labels)
            }
        };
        let smoothing = smoothing::smooth(
            &mut world,
            &tets,
            &labels,
            &constraints,
            &volume_limits,
            &smoothing::Options {
                input_vertices: plc.vertices.len(),
                target: if quality.is_some() {
                    0.
                } else {
                    options.min_dihedral
                },
                factor,
                max_sweeps: 8,
                preserve_violation_count: true,
                boundary_facets: &[],
            },
        )?;
        report.smoothing_moves = smoothing.moves;
        report.smoothing_sweeps = smoothing.sweeps;
        let initial = local_refinement::State {
            world,
            tets,
            labels,
            constraints,
            splits: 0,
            insertions: 0,
            rounds: 0,
            flips: report.flips,
            collapses: 0,
            moves: report.smoothing_moves,
            sweeps: report.smoothing_sweeps,
        };
        let optimized = if let Some(quality) = quality {
            let (state, stats) = refinement::refine(
                initial,
                plc,
                patches.expect("quality patch policy"),
                options,
                sizing.expect("quality sizing field"),
                quality,
                factor,
            )?;
            report.quality = Some(stats);
            state
        } else {
            local_refinement::optimize(
                initial,
                options,
                factor,
                plc.vertices.len(),
                if patches.is_some() { &[] } else { &plc.facets },
                &volume_limits,
            )?
        };
        let optimized = if let Some(field) = sizing.filter(|_| quality.is_none()) {
            local_refinement::graded(
                optimized,
                options,
                field,
                factor,
                plc.vertices.len(),
                &volume_limits,
            )?
        } else {
            optimized
        };
        world = optimized.world;
        let tets = optimized.tets;
        let labels = optimized.labels;
        constraints = optimized.constraints;
        report.local_edge_splits = optimized.splits;
        report.local_cell_splits = optimized.insertions;
        report.local_refinement_passes = optimized.rounds;
        report.flips = optimized.flips;
        report.interior_collapses = optimized.collapses;
        report.smoothing_moves = optimized.moves;
        report.smoothing_sweeps = optimized.sweeps;
        // Return only after geometric constraints have all been recovered.
        let mut mesh = Mesh3D {
            report,
            ..Default::default()
        };
        let used: BTreeSet<_> = tets
            .iter()
            .zip(&labels)
            .filter(|(_, l)| **l != -2)
            .flat_map(|(t, _)| t.iter().copied())
            .collect();
        let mut remap = vec![-1; world.len()];
        for v in used {
            remap[v] = mesh.nodes.len() as i64;
            mesh.nodes.push(world[v]);
            mesh.node_markers
                .push(plc.vertex_markers.get(v).copied().unwrap_or(0));
        }
        mesh.input_vertex_nodes = remap[..plc.vertices.len()].to_vec();
        for (t, &label) in tets.iter().zip(&labels) {
            if label == -2 {
                continue;
            }
            let mut out = t.map(|v| remap[v] as usize);
            out.swap(0, 1);
            let p = out.map(|v| mesh.nodes[v]);
            let volume = -orient(p[0], p[1], p[2], p[3]) / 6.0;
            if !volume.is_finite() || volume <= 0.0 {
                return Err(failed(
                    "a tetrahedron is degenerate or inverted in original coordinates",
                ));
            }
            mesh.tetrahedra.push(out);
            mesh.cell_regions.push(label);
            mesh.cell_markers.push(if label >= 0 {
                plc.regions[label as usize].marker
            } else {
                0
            });
        }
        let out_map = face_map(&mesh.tetrahedra)?;
        let out_barriers: BTreeMap<_, _> = constraints
            .iter()
            .filter(|c| c.vertices.iter().all(|&v| remap[v] >= 0))
            .map(|c| (sorted(c.vertices.map(|v| remap[v] as usize)), c.parent))
            .collect();
        mesh.neighbors = vec![[-1; 4]; mesh.tetrahedra.len()];
        let mut recovered_area = vec![0.0; plc.facets.len()];
        let hull_faces: BTreeSet<Face> = out_map
            .iter()
            .filter(|(_, entries)| entries.len() == 1)
            .map(|(f, _)| *f)
            .collect();
        for (f, entries) in out_map {
            if entries.len() == 2 {
                let [(a, ka), (b, kb)] = [entries[0], entries[1]];
                mesh.neighbors[a][ka] = b as i64;
                mesh.neighbors[b][kb] = a as i64;
                let p = f.map(|v| mesh.nodes[v]);
                let sa = orient(p[0], p[1], p[2], mesh.nodes[mesh.tetrahedra[a][ka]]);
                let sb = orient(p[0], p[1], p[2], mesh.nodes[mesh.tetrahedra[b][kb]]);
                if sa.signum() == sb.signum() {
                    return Err(failed("adjacent tetrahedra overlap"));
                }
            }
            let parent = out_barriers.get(&f).copied();
            if entries.len() == 1 || parent.is_some() {
                if !plc.facets.is_empty() && entries.len() == 1 && parent.is_none() {
                    return Err(failed("unconstrained face leaked onto the domain boundary"));
                }
                let (a, k) = entries[0];
                mesh.faces.push(face(mesh.tetrahedra[a], k));
                mesh.face_cells
                    .push([a as i64, entries.get(1).map_or(-1, |x| x.0 as i64)]);
                mesh.face_facets.push(parent.map_or(-1, |p| p as i64));
                mesh.face_markers
                    .push(parent.map_or(0, |p| plc.facet_markers.get(p).copied().unwrap_or(0)));
                if let Some(p) = parent {
                    if let Some(patches) = patches {
                        for c in
                            surface::coverage(plc, &patches.groups[&p], f.map(|v| mesh.nodes[v]))?
                        {
                            recovered_area[c.facet] += c.area;
                        }
                    } else {
                        recovered_area[p] += triangle_area(f.map(|v| mesh.nodes[v]));
                    }
                }
            }
        }
        for (i, &f) in plc.facets.iter().enumerate() {
            let expected = triangle_area(f.map(|v| plc.vertices[v]));
            if (recovered_area[i] - expected).abs() > expected * 1e-8 {
                return Err(failed(format!("facet {i} is not fully represented on the exported mesh (possibly excluded on both sides)")));
            }
        }
        mesh.report.min_volume = f64::INFINITY;
        mesh.report.min_dihedral = 180.0;
        let mut cell_angles = Vec::with_capacity(mesh.tetrahedra.len());
        let mut radius_edges = Vec::with_capacity(mesh.tetrahedra.len());
        let mut mean_ratios = Vec::with_capacity(mesh.tetrahedra.len());
        for (i, &t) in mesh.tetrahedra.iter().enumerate() {
            let p = t.map(|v| mesh.nodes[v]);
            let v = -orient(p[0], p[1], p[2], p[3]) / 6.0;
            let (d, worst_edge) = worst_dihedral(p.map(transform));
            cell_angles.push(d);
            if let Some((radius, mean)) = shape_metrics(p) {
                radius_edges.push(radius);
                mean_ratios.push(mean);
            } else {
                mesh.report.shape_metric_failures += 1;
            }
            if d < 1.0 {
                mesh.report.sliver_count += 1;
            }
            mesh.report.min_volume = mesh.report.min_volume.min(v);
            mesh.report.max_volume = mesh.report.max_volume.max(v);
            mesh.report.total_volume += v;
            mesh.report.min_dihedral = mesh.report.min_dihedral.min(d);
            let r = mesh.cell_regions[i];
            let regional = if r >= 0 {
                plc.regions[r as usize].max_volume
            } else {
                0.0
            };
            if v > limit(options.max_volume, regional) * (1.0 + 1e-12) {
                mesh.report.volume_violations += 1;
            }
            if d + 1e-10 < options.min_dihedral {
                mesh.report.dihedral_violations += 1;
                let forced = (0..4).filter(|k| !worst_edge.contains(k)).all(|k| {
                    let f = sorted(face(t, k));
                    out_barriers.contains_key(&f) || hull_faces.contains(&f)
                });
                if forced {
                    mesh.report.forced_dihedrals += 1;
                }
            }
        }
        cell_angles.sort_by(f64::total_cmp);
        let quantile = |fraction: f64| {
            let pos = fraction * (cell_angles.len() - 1) as f64;
            let lo = pos.floor() as usize;
            let hi = pos.ceil() as usize;
            cell_angles[lo] + (cell_angles[hi] - cell_angles[lo]) * (pos - lo as f64)
        };
        mesh.report.min_dihedral_p05 = quantile(0.05);
        mesh.report.min_dihedral_median = quantile(0.5);
        mesh.report.min_dihedral_p95 = quantile(0.95);
        if mesh.report.shape_metric_failures == 0 && !radius_edges.is_empty() {
            radius_edges.sort_by(f64::total_cmp);
            mean_ratios.sort_by(f64::total_cmp);
            let at = |values: &[f64], fraction: f64| {
                let pos = fraction * (values.len() - 1) as f64;
                let lo = pos.floor() as usize;
                let hi = pos.ceil() as usize;
                values[lo] + (values[hi] - values[lo]) * (pos - lo as f64)
            };
            mesh.report.radius_edge_median = Some(at(&radius_edges, 0.5));
            mesh.report.radius_edge_p95 = Some(at(&radius_edges, 0.95));
            mesh.report.radius_edge_max = radius_edges.last().copied();
            mesh.report.mean_ratio_min = mean_ratios.first().copied();
            mesh.report.mean_ratio_median = Some(at(&mean_ratios, 0.5));
        }
        mesh.report.steiner_points = world.len() - plc.vertices.len();
        if mesh.report.volume_violations > 0 {
            mesh.report
                .issues
                .push("volume limit unmet within the vertex/pass budget".into());
        }
        let forced = mesh.report.forced_dihedrals;
        let fixable = mesh.report.dihedral_violations - forced;
        if fixable > 0 {
            let why = if mesh.report.local_refinement_passes > 0 {
                "bounded local refinement, flips and constrained smoothing did not meet the target"
            } else if sliver_rolled_back {
                "boundary recovery could not follow the last sliver refinement, which was undone"
            } else if sliver_budget_hit {
                "the vertex budget ran out"
            } else if sliver_stall >= 6 {
                "sliver removal stopped improving"
            } else if sliver_unplaceable {
                "no further vertex could be placed; flips and interior smoothing did not meet the target"
            } else {
                "the pass budget ran out"
            };
            mesh.report.issues.push(format!(
                "{fixable} cells stay below the {} degree dihedral target: {why}",
                options.min_dihedral
            ));
        }
        if forced > 0 {
            mesh.report.issues.push(format!(
                "{forced} cells stay below the {} degree dihedral target where two boundary or interface faces meet at that angle in the input",
                options.min_dihedral
            ));
        }
        if mesh.report.min_dihedral < 1.0 {
            mesh.report.warnings.push(if options.min_dihedral > 0.0 {
                "Severe slivers remain (minimum dihedral below 1 degree); this mesh has not passed a simulation-quality criterion.".to_string()
            } else {
                "Severe slivers detected (minimum dihedral below 1 degree); set min_dihedral (5 to 10 degrees) to attempt flips, refinement and interior smoothing.".to_string()
            });
        }
        crate::control::checkpoint(mesh.nodes.len(), mesh.tetrahedra.len(), false)?;
        if let Some(field) = sizing {
            let edges: BTreeSet<_> = mesh
                .tetrahedra
                .iter()
                .flat_map(|t| {
                    (0..4).flat_map(move |i| (i + 1..4).map(move |j| sorted([t[i], t[j]])))
                })
                .collect();
            let ratios: Vec<_> = edges
                .iter()
                .map(|&[a, b]| field.edge_ratio(mesh.nodes[a], mesh.nodes[b]))
                .collect();
            let count = ratios.iter().filter(|&&r| r > 1. + 1e-10).count();
            mesh.report.sizing_edge_violations = Some(count);
            mesh.report.sizing_max_edge_ratio = Some(ratios.into_iter().fold(0_f64, f64::max));
            if count > 0 {
                mesh.report
                    .issues
                    .push("experimental edge sizing unmet within local budget".into());
            }
        }
        if let Some(q) = mesh.report.quality.as_mut() {
            q.radius_edge_violations = mesh
                .tetrahedra
                .iter()
                .filter(|t| {
                    shape_metrics(t.map(|v| mesh.nodes[v]))
                        .is_none_or(|(r, _)| r > q.max_radius_edge * (1. + 1e-10))
                })
                .count();
            if q.radius_edge_violations > 0 {
                mesh.report.issues.push(format!(
                    "{} cells exceed requested radius-edge {}",
                    q.radius_edge_violations, q.max_radius_edge
                ));
            }
            if q.encroached_segments > 0 || q.encroached_faces > 0 {
                mesh.report
                    .issues
                    .push("Delaunay boundary protection remains incomplete".into());
            }
        }
        mesh.report.complete = mesh.report.issues.is_empty();
        return Ok(mesh);
    }
    Err(failed("3D pass budget exhausted"))
}
