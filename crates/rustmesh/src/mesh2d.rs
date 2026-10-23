use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};

use spade::handles::{FixedFaceHandle, FixedVertexHandle, InnerTag};
use spade::{
    AngleLimit, ConstrainedDelaunayTriangulation, Point2, PositionInTriangulation,
    RefinementParameters, Triangulation,
};

use crate::error::MeshError;
use crate::plc::Plc;

type Cdt = ConstrainedDelaunayTriangulation<Point2<f64>>;

/// Face label: outside the domain (exterior, air, hole).
const EXCLUDED: i32 = -2;
/// Face label: inside the domain, reached by no region seed.
const NO_SEED: i32 = -1;

/// How [`triangulate`] refines.
#[derive(Debug, Clone)]
pub struct Options {
    /// Smallest angle a cell should have, in degrees (0: no angle refinement).
    /// spade may not terminate much above 30.
    pub min_angle: f64,
    /// Largest cell area everywhere (0: no global limit). A region's own limit
    /// applies where it is smaller.
    pub max_area: f64,
    /// Most vertices the triangulation may hold, input included.
    pub max_vertices: usize,
    /// Points to refine around, usually the electrodes.
    pub refine_points: Vec<[f64; 2]>,
    /// Target cell edge length at the refine points (0: no point refinement).
    pub point_size: f64,
    /// How fast the target edge length grows with distance from the nearest
    /// refine point, in metres per metre. 0 holds `point_size` everywhere.
    pub size_growth: f64,
    /// Input segments (indices into [`Plc::segments`]) that refinement must
    /// not split: no vertex is added on them. Where they cross other segments
    /// they are still split at the crossing.
    pub keep_segments: Vec<usize>,
    /// Also keep whole every segment piece on the domain's boundary (outside
    /// or a hole on one side). The pyGIMLi adapter sets it for
    /// `preserveBoundary=True`. A mesh that is later joined along that boundary
    /// to another mesh then matches it node for node.
    pub keep_boundary: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            min_angle: 28.0,
            max_area: 0.0,
            max_vertices: 5_000_000,
            refine_points: Vec::new(),
            point_size: 0.0,
            size_growth: 0.5,
            keep_segments: Vec::new(),
            keep_boundary: false,
        }
    }
}

impl Options {
    fn validate(&self) -> Result<(), MeshError> {
        if !(0.0..=40.0).contains(&self.min_angle) {
            return Err(MeshError::BadOption(format!(
                "min_angle must be between 0 and 40 degrees, not {}",
                self.min_angle
            )));
        }
        for (name, value) in [
            ("max_area", self.max_area),
            ("point_size", self.point_size),
            ("size_growth", self.size_growth),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(MeshError::BadOption(format!(
                    "{name} must be 0 or positive, not {value}"
                )));
            }
        }
        if let Some(i) = self
            .refine_points
            .iter()
            .position(|p| !(p[0].is_finite() && p[1].is_finite()))
        {
            return Err(MeshError::NonFiniteSeed {
                what: "refine point",
                index: i,
            });
        }
        Ok(())
    }

    fn sizes_cells(&self) -> bool {
        self.max_area > 0.0 || (self.point_size > 0.0 && !self.refine_points.is_empty())
    }
}

/// The area limit a region (`label`) and the global limit set, infinite for none.
fn region_limit(plc: &Plc, opts: &Options, label: i32) -> f64 {
    let mut limit = if opts.max_area > 0.0 {
        opts.max_area
    } else {
        f64::INFINITY
    };
    if label >= 0 {
        let own = plc.regions[label as usize].max_area;
        if own > 0.0 {
            limit = limit.min(own);
        }
    }
    limit
}

/// Points sorted into a uniform grid of about one point per cell, so that
/// the nearest refine point (the cell size near electrodes) is found without
/// comparing every one.
struct PointGrid {
    lo: [f64; 2],
    cell: f64,
    nx: usize,
    ny: usize,
    /// Points of grid cell `k` are `points[start[k]..start[k + 1]]`.
    start: Vec<usize>,
    points: Vec<[f64; 2]>,
}

impl PointGrid {
    fn new(pts: &[[f64; 2]]) -> Self {
        let mut grid = Self {
            lo: [0.0; 2],
            cell: 1.0,
            nx: 1,
            ny: 1,
            start: vec![0, 0],
            points: Vec::new(),
        };
        if pts.is_empty() {
            return grid;
        }
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for p in pts {
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let (ex, ey, n) = (hi[0] - lo[0], hi[1] - lo[1], pts.len() as f64);
        // About one point per cell, also for points along a line (ey = 0);
        // at most 3n + 1 cells.
        let mut cell = (ex * ey / n).sqrt().max(ex.max(ey) / n);
        if !(cell > 0.0 && cell.is_finite()) {
            cell = 1.0;
        }
        grid.lo = lo;
        grid.cell = cell;
        grid.nx = (ex / cell) as usize + 1;
        grid.ny = (ey / cell) as usize + 1;
        let mut start = vec![0usize; grid.nx * grid.ny + 1];
        for p in pts {
            start[grid.cell_of(*p) + 1] += 1;
        }
        for k in 1..start.len() {
            start[k] += start[k - 1];
        }
        let mut next = start.clone();
        grid.points = vec![[0.0; 2]; pts.len()];
        for p in pts {
            let k = grid.cell_of(*p);
            grid.points[next[k]] = *p;
            next[k] += 1;
        }
        grid.start = start;
        grid
    }

    /// Grid column and row of `p`, clamped to the grid.
    fn column_row(&self, p: [f64; 2]) -> (usize, usize) {
        let along = |v: f64, lo: f64, n: usize| {
            let f = (v - lo) / self.cell;
            if f > 0.0 {
                (f as usize).min(n - 1)
            } else {
                0
            }
        };
        (
            along(p[0], self.lo[0], self.nx),
            along(p[1], self.lo[1], self.ny),
        )
    }

    fn cell_of(&self, p: [f64; 2]) -> usize {
        let (i, j) = self.column_row(p);
        j * self.nx + i
    }

    /// The grid cells `ring` cells out from `(ci, cj)` (the cell itself for 0).
    fn ring(&self, ci: usize, cj: usize, ring: usize, mut visit: impl FnMut(usize)) {
        let (j0, j1) = (cj.saturating_sub(ring), (cj + ring).min(self.ny - 1));
        for j in j0..=j1 {
            if j + ring == cj || j == cj + ring {
                for i in ci.saturating_sub(ring)..=(ci + ring).min(self.nx - 1) {
                    visit(j * self.nx + i);
                }
            } else {
                if ci >= ring {
                    visit(j * self.nx + ci - ring);
                }
                if ci + ring < self.nx {
                    visit(j * self.nx + ci + ring);
                }
            }
        }
    }

    /// Squared distance from `p` to the nearest point (infinite for none),
    /// exactly what comparing every point gives.
    ///
    /// Rings of cells are searched outwards until no cell left unsearched
    /// can hold a nearer point: the stop compares the best distance with
    /// the distance from `p` to the unsearched part of the grid, not with
    /// the ring's width alone. For electrodes along a line (a grid one
    /// row high) and a point deep below them, the ring's width would have to
    /// grow to the depth before the search stopped, comparing a number of
    /// electrodes that grows with the length of the line.
    fn nearest2(&self, p: [f64; 2]) -> f64 {
        let (ci, cj) = self.column_row(p);
        let mut best = f64::INFINITY;
        // Cell boundaries are computed, so a point may sit a rounding error
        // on the far side of one; the unsearched part is widened by this.
        let margin = self.cell * 1e-9;
        let (x_end, y_end) = (
            self.lo[0] + self.nx as f64 * self.cell,
            self.lo[1] + self.ny as f64 * self.cell,
        );
        let rect = |x0: f64, x1: f64, y0: f64, y1: f64| {
            let dx = (x0 - p[0]).max(p[0] - x1).max(0.0);
            let dy = (y0 - p[1]).max(p[1] - y1).max(0.0);
            dx * dx + dy * dy
        };
        for ring in 0..=self.nx.max(self.ny) {
            self.ring(ci, cj, ring, |k| {
                for q in &self.points[self.start[k]..self.start[k + 1]] {
                    best = best.min((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2));
                }
            });
            // The searched block is columns i0..=i1, rows j0..=j1; the rest
            // of the grid is up to four slabs beside it.
            let (i0, i1) = (ci.saturating_sub(ring), (ci + ring).min(self.nx - 1));
            let (j0, j1) = (cj.saturating_sub(ring), (cj + ring).min(self.ny - 1));
            let x = |i: usize| self.lo[0] + i as f64 * self.cell;
            let y = |j: usize| self.lo[1] + j as f64 * self.cell;
            let mut rest = f64::INFINITY;
            if i0 > 0 {
                rest = rest.min(rect(self.lo[0], x(i0) + margin, self.lo[1], y_end));
            }
            if i1 + 1 < self.nx {
                rest = rest.min(rect(x(i1 + 1) - margin, x_end, self.lo[1], y_end));
            }
            if j0 > 0 {
                rest = rest.min(rect(self.lo[0], x_end, self.lo[1], y(j0) + margin));
            }
            if j1 + 1 < self.ny {
                rest = rest.min(rect(self.lo[0], x_end, y(j1 + 1) - margin, y_end));
            }
            if best <= rest {
                break;
            }
        }
        best
    }
}

/// The cell size the refine points ask for: the area of an equilateral
/// triangle whose edge is `point_size` plus `size_growth` times the distance
/// to the nearest refine point. Infinite when point refinement is off.
struct PointSizing {
    size: f64,
    growth: f64,
    grid: Option<PointGrid>,
}

impl PointSizing {
    fn new(opts: &Options) -> Self {
        let on = opts.point_size > 0.0 && !opts.refine_points.is_empty();
        Self {
            size: opts.point_size,
            growth: opts.size_growth,
            grid: on.then(|| PointGrid::new(&opts.refine_points)),
        }
    }

    fn limit(&self, p: Point2<f64>) -> f64 {
        let Some(grid) = &self.grid else {
            return f64::INFINITY;
        };
        let h = self.size + self.growth * grid.nearest2([p.x, p.y]).sqrt();
        0.25 * 3f64.sqrt() * h * h
    }
}

/// The circumradius-to-shortest-edge ratio `1 / (2 sin(angle))` of the
/// angle limit, with the sine from its Taylor series: only + - * /, so a
/// browser and a native build get the same bits (`sin` comes from each
/// platform's math library and may differ in the last one).
fn radius_edge_ratio(min_angle_deg: f64) -> f64 {
    let x = min_angle_deg * (std::f64::consts::PI / 180.0);
    let (mut term, mut sine) = (x, x);
    for k in 1..12 {
        term *= -x * x / ((2 * k) as f64 * (2 * k + 1) as f64);
        sine += term;
    }
    0.5 / sine
}

fn centroid(p: &[Point2<f64>; 3]) -> Point2<f64> {
    Point2::new(
        (p[0].x + p[1].x + p[2].x) / 3.0,
        (p[0].y + p[1].y + p[2].y) / 3.0,
    )
}

/// Labels of the domain faces at `p` in `cdt` (several when `p` lies on an
/// edge or a vertex).
fn labels_at(cdt: &Cdt, labels: &[i32], p: Point2<f64>) -> Vec<i32> {
    let faces: Vec<FixedFaceHandle<InnerTag>> = match cdt.locate(p) {
        PositionInTriangulation::OnFace(f) => vec![f],
        PositionInTriangulation::OnEdge(e) => {
            let e = cdt.directed_edge(e);
            [e.face(), e.rev().face()]
                .iter()
                .filter_map(|f| f.as_inner())
                .map(|f| f.fix())
                .collect()
        }
        PositionInTriangulation::OnVertex(v) => cdt
            .vertex(v)
            .out_edges()
            .filter_map(|e| e.face().as_inner())
            .map(|f| f.fix())
            .collect(),
        _ => Vec::new(),
    };
    faces
        .into_iter()
        .map(|f| labels[f.index()])
        .filter(|&l| l != EXCLUDED)
        .collect()
}

/// A cell whose smallest angle is below the requested one.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AngleDiagnostic {
    /// Index into the exported triangle array.
    pub cell: usize,
    /// The last refinement decision on this triangle, or `unresolved` or
    /// `budget_exhausted`. It says why refinement left the cell, not that no
    /// better mesh exists.
    pub reason: String,
}

/// What meshing achieved, measured on the exported mesh.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Report {
    /// True only when refinement finished and every requested quality limit holds.
    /// A small input angle does not excuse a cell below the requested angle.
    pub complete: bool,
    /// The refinement driver stopped without hitting its budget. This does
    /// not mean that all requested angles or areas were achieved.
    #[cfg_attr(feature = "serde", serde(default))]
    pub refinement_complete: bool,
    /// Every exported cell meets the requested angle and area tolerances.
    #[cfg_attr(feature = "serde", serde(default))]
    pub quality_met: bool,
    /// Why the mesh is incomplete, one line each.
    pub issues: Vec<String>,
    /// Things worth knowing that do not make the mesh incomplete.
    pub warnings: Vec<String>,
    pub min_angle: f64,
    pub angle_violations: usize,
    /// One diagnostic per angle violation. A cell with a diagnostic still
    /// counts against `quality_met` and strict acceptance. Empty when a
    /// serialized report does not have the field.
    #[cfg_attr(feature = "serde", serde(default))]
    pub angle_diagnostics: Vec<AngleDiagnostic>,
    pub area_violations: usize,
    pub refine_calls: usize,
    /// Triangulation vertices left out of the mesh: input vertices outside the
    /// domain, and any refinement vertex placed in air or a hole (expected 0).
    pub vertices_outside: usize,
}

/// A triangle mesh with its markers and how it maps back to the input.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Mesh2D {
    pub nodes: Vec<[f64; 2]>,
    /// The input vertex marker on input vertices, 0 on added nodes.
    pub node_markers: Vec<i32>,
    /// Node of each input vertex, or -1 when it lies outside the domain.
    pub input_vertex_nodes: Vec<i64>,
    /// Counter-clockwise node triples.
    pub triangles: Vec<[usize; 3]>,
    /// The marker of the region seed that reached the cell, 0 when none did.
    pub cell_markers: Vec<i32>,
    /// Index of that region seed, -1 when none did.
    pub cell_regions: Vec<i32>,
    /// Every segment piece in the mesh, and the domain's outline where no
    /// segment runs.
    pub edges: Vec<[usize; 2]>,
    /// The marker of the input segment an edge is part of, 0 for none.
    pub edge_markers: Vec<i32>,
    /// Index of that input segment, -1 for none.
    pub edge_segments: Vec<i64>,
    pub report: Report,
}

impl Mesh2D {
    fn corners(&self, cell: usize) -> [Point2<f64>; 3] {
        self.triangles[cell].map(|n| Point2::new(self.nodes[n][0], self.nodes[n][1]))
    }

    /// Cell areas in m².
    pub fn cell_areas(&self) -> Vec<f64> {
        (0..self.triangles.len())
            .map(|c| {
                let [a, b, p] = self.corners(c);
                0.5 * ((b.x - a.x) * (p.y - a.y) - (p.x - a.x) * (b.y - a.y))
            })
            .collect()
    }

    /// Smallest angle of every cell in degrees.
    pub fn cell_min_angles(&self) -> Vec<f64> {
        (0..self.triangles.len())
            .map(|c| min_angle_deg(self.corners(c)))
            .collect()
    }
}

/// Meshes `plc` with triangles that honour its segments, regions and holes.
///
/// Compatibility entry point: an incomplete refinement/quality result is still
/// `Ok`; callers must inspect [`Report::complete`]. Use [`triangulate_strict`]
/// when unmet requested limits should be returned as an error with the result.
pub fn triangulate(plc: &Plc, options: &Options) -> Result<Mesh2D, MeshError> {
    crate::control::checkpoint(plc.vertices.len(), 0, false)?;
    plc.validate()?;
    options.validate()?;
    if let Some(&i) = options
        .keep_segments
        .iter()
        .find(|&&i| i >= plc.segments.len())
    {
        return Err(MeshError::BadOption(format!(
            "keep_segments names segment {i}, but the input has {} segments",
            plc.segments.len()
        )));
    }
    let shift = Shift::of(plc);
    let (local_plc, local_options) = (shift.plc(plc), shift.options(options));
    let mut mesh = match catch_unwind(AssertUnwindSafe(|| {
        Mesher::new(&local_plc, &local_options)?.run()
    })) {
        Ok(result) => result?,
        Err(panic) => {
            let msg = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_string());
            return Err(MeshError::Internal(format!(
                "the triangulation library panicked: {msg}"
            )));
        }
    };
    shift.restore(&mut mesh, plc);
    Ok(mesh)
}

/// Meshes a PLC like [`triangulate`], but returns `Ok` only when the report
/// marks the mesh complete: refinement finished and every requested angle and
/// area limit holds. Invalid input, cancellation and backend errors carry no
/// mesh. A nonempty, positive-area mesh that misses its goals is available
/// through [`crate::TriangulationError::mesh`], and its report through
/// [`crate::TriangulationError::report`]. The check covers the requested
/// limits only; it is not a global overlap or finite-element audit.
pub fn triangulate_strict(
    plc: &Plc,
    options: &Options,
) -> Result<Mesh2D, crate::TriangulationError> {
    let mesh = triangulate(plc, options)?;
    if mesh.triangles.is_empty()
        || mesh.nodes.iter().flatten().any(|x| !x.is_finite())
        || mesh
            .cell_areas()
            .iter()
            .any(|&area| !area.is_finite() || area <= 0.0)
    {
        return Err(MeshError::Insertion(
            "strict meshing produced no nonempty positive-area mesh".into(),
        )
        .into());
    }
    if mesh.report.complete {
        Ok(mesh)
    } else {
        Err(crate::TriangulationError::Incomplete(Box::new(mesh)))
    }
}

/// Meshing about a local origin. Far from the origin (UTM eastings and
/// northings, 1e5 to 1e7) the coordinates leave few significant digits for
/// the circumcentres and split points of small cells, which then come out
/// degenerate. The input is moved to its middle when it lies more than ten
/// times its own size away, and moved back afterwards; the input vertices get
/// their exact coordinates back. Nearer inputs are meshed where they are.
#[derive(Clone, Copy)]
struct Shift([f64; 2]);

impl Shift {
    fn of(plc: &Plc) -> Self {
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for v in &plc.vertices {
            for k in 0..2 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
        if plc.vertices.is_empty() {
            return Shift([0.0; 2]);
        }
        let size = (hi[0] - lo[0]).max(hi[1] - lo[1]);
        let middle = [0.5 * (lo[0] + hi[0]), 0.5 * (lo[1] + hi[1])];
        if middle[0].abs().max(middle[1].abs()) > 10.0 * size {
            Shift(middle)
        } else {
            Shift([0.0; 2])
        }
    }

    fn is_none(self) -> bool {
        self.0 == [0.0; 2]
    }

    fn point(self, p: [f64; 2]) -> [f64; 2] {
        [p[0] - self.0[0], p[1] - self.0[1]]
    }

    fn plc(self, plc: &Plc) -> Plc {
        let mut local = plc.clone();
        if !self.is_none() {
            local.vertices.iter_mut().for_each(|p| *p = self.point(*p));
            local
                .regions
                .iter_mut()
                .for_each(|r| r.point = self.point(r.point));
            local.holes.iter_mut().for_each(|p| *p = self.point(*p));
        }
        local
    }

    fn options(self, options: &Options) -> Options {
        let mut local = options.clone();
        if !self.is_none() {
            local
                .refine_points
                .iter_mut()
                .for_each(|p| *p = self.point(*p));
        }
        local
    }

    fn restore(self, mesh: &mut Mesh2D, plc: &Plc) {
        if self.is_none() {
            return;
        }
        for p in &mut mesh.nodes {
            *p = [p[0] + self.0[0], p[1] + self.0[1]];
        }
        for (i, &n) in mesh.input_vertex_nodes.iter().enumerate() {
            if n >= 0 {
                mesh.nodes[n as usize] = plc.vertices[i];
            }
        }
    }
}

/// One point inside every area the segments of `plc` enclose (holes and
/// regions ignored): the centroid of the area's largest triangle, which keeps
/// it clear of the area's outline. Used to seed regions automatically.
pub(crate) fn enclosed_areas(plc: &Plc) -> Result<Vec<[f64; 2]>, MeshError> {
    plc.validate()?;
    let shift = Shift::of(plc);
    let bare = Plc {
        regions: Vec::new(),
        holes: Vec::new(),
        ..shift.plc(plc)
    };
    let opts = Options::default();
    let mesher = Mesher::new(&bare, &opts)?;
    let mut labels = mesher.classify();
    let mut best: Vec<(f64, [f64; 2])> = Vec::new();
    for f in mesher.cdt.inner_faces() {
        let i = f.fix().index();
        if labels[i] == NO_SEED {
            mesher.flood(vec![f.fix()], best.len() as i32, &mut labels);
            best.push((-1.0, [0.0; 2]));
        }
        let label = labels[i];
        if label >= 0 {
            let p = f.positions();
            let area = f.area();
            if area > best[label as usize].0 {
                let c = centroid(&p);
                best[label as usize] = (area, [c.x, c.y]);
            }
        }
    }
    Ok(best
        .into_iter()
        .map(|(_, p)| [p[0] + shift.0[0], p[1] + shift.0[1]])
        .collect())
}

fn key(a: usize, b: usize) -> (usize, usize) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

fn min_angle_deg(p: [Point2<f64>; 3]) -> f64 {
    // The smallest angle is opposite the shortest edge. Measure it with
    // atan2 once, avoiding three transcendental calls per exported cell.
    let lengths = [
        p[1].distance_2(p[2]),
        p[2].distance_2(p[0]),
        p[0].distance_2(p[1]),
    ];
    let mut i = 0;
    if lengths[1] < lengths[i] {
        i = 1;
    }
    if lengths[2] < lengths[i] {
        i = 2;
    }
    let (a, b, c) = (p[i], p[(i + 1) % 3], p[(i + 2) % 3]);
    let (ux, uy) = (b.x - a.x, b.y - a.y);
    let (vx, vy) = (c.x - a.x, c.y - a.y);
    (ux * vy - uy * vx)
        .abs()
        .atan2(ux * vx + uy * vy)
        .to_degrees()
}

struct Mesher<'a> {
    plc: &'a Plc,
    opts: &'a Options,
    cdt: Cdt,
    input_handles: Vec<FixedVertexHandle>,
    /// Indices of the frame vertices around the input; never part of the mesh.
    frame: Vec<usize>,
    /// Constraint edge (vertex pair) -> input segment it is part of.
    parents: HashMap<(usize, usize), usize>,
    report: Report,
    skipped_faces: HashMap<usize, ([FixedVertexHandle; 3], &'static str)>,
}

impl<'a> Mesher<'a> {
    fn new(plc: &'a Plc, opts: &'a Options) -> Result<Self, MeshError> {
        let mut cdt = Cdt::new();
        let mut input_handles = Vec::with_capacity(plc.vertices.len());
        for (i, v) in plc.vertices.iter().enumerate() {
            crate::control::checkpoint(i, cdt.num_inner_faces(), false)?;
            let handle = cdt
                .insert(Point2::new(v[0], v[1]))
                .map_err(|e| MeshError::Insertion(format!("vertex {i}: {e:?}")))?;
            input_handles.push(handle);
        }
        // A frame well outside the input keeps the domain's outline off the
        // convex hull. Otherwise a vertex a rounding error off a straight
        // outline (an electrode on a sloping surface) leaves a flat sliver
        // between the hull and the outline, and spade then refuses to split
        // the outline next to it, leaving cells there unrefined.
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for v in &plc.vertices {
            for k in 0..2 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
        let margin = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1.0);
        let (x0, x1, y0, y1) = (
            lo[0] - margin,
            hi[0] + margin,
            lo[1] - margin,
            hi[1] + margin,
        );
        let mut frame = Vec::new();
        for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] {
            let handle = cdt
                .insert(Point2::new(x, y))
                .map_err(|e| MeshError::Insertion(format!("frame: {e:?}")))?;
            frame.push(handle.index());
        }
        let mut mesher = Self {
            plc,
            opts,
            cdt,
            input_handles,
            frame,
            parents: HashMap::new(),
            report: Report::default(),
            skipped_faces: HashMap::new(),
        };
        let mut crossing = Vec::new();
        for (i, s) in plc.segments.iter().enumerate() {
            let (a, b) = (mesher.input_handles[s[0]], mesher.input_handles[s[1]]);
            let before = mesher.cdt.num_vertices();
            let mut edges = mesher.cdt.try_add_constraint(a, b);
            if edges.is_empty() {
                // It crosses earlier segments: add a vertex at each crossing
                // and split both segments there.
                edges = mesher.cdt.add_constraint_and_split(a, b, |p| p);
                if edges.is_empty() {
                    return Err(MeshError::Insertion(format!(
                        "segment {i} could not be inserted"
                    )));
                }
                crossing.push(i);
            }
            for e in edges {
                let e = mesher.cdt.directed_edge(e);
                mesher
                    .parents
                    .insert(key(e.from().fix().index(), e.to().fix().index()), i);
            }
            if mesher.cdt.num_vertices() > before {
                mesher.update_parents(before)?;
            }
        }
        if !crossing.is_empty() {
            let shown: Vec<String> = crossing.iter().take(10).map(|i| i.to_string()).collect();
            mesher.report.warnings.push(format!(
                "{} segment(s) cross earlier segments (segment {}{}); a vertex was added at each crossing",
                crossing.len(),
                shown.join(", "),
                if crossing.len() > 10 { ", ..." } else { "" }
            ));
        }
        Ok(mesher)
    }

    fn run(mut self) -> Result<Mesh2D, MeshError> {
        self.refine()?;
        Ok(self.export())
    }

    /// One spade refinement for angle and cell size. The faces the seeds put
    /// outside the domain (air, holes) go to spade as excluded, so they are
    /// never refined and never split the domain's outline from outside.
    fn refine(&mut self) -> Result<(), MeshError> {
        let sized = self.opts.sizes_cells() || self.plc.regions.iter().any(|r| r.max_area > 0.0);
        if self.opts.min_angle <= 0.0 && !sized {
            return Ok(());
        }
        let room = self
            .opts
            .max_vertices
            .saturating_sub(self.cdt.num_vertices());
        if room == 0 {
            self.report.issues.push(format!(
                "the vertex budget ({}) ran out before refinement",
                self.opts.max_vertices
            ));
            return Ok(());
        }
        let labels = self.classify();
        let excluded: Vec<_> = self
            .cdt
            .inner_faces()
            .filter(|f| labels[f.fix().index()] == EXCLUDED)
            .map(|f| f.fix())
            .collect();
        // Refinement only splits segments, so which region a point lies in
        // never changes: the triangulation as it is now answers it for every
        // face refinement creates.
        let (plc, opts) = (self.plc, self.opts);
        let sizing = PointSizing::new(opts);
        // When every region has the same limit there is nothing to look up.
        // The lookup walks the unrefined triangulation, whose cells along a
        // long electrode line are slivers the length of the line, so it is
        // skipped whenever it can be.
        let same = region_limit(plc, opts, NO_SEED);
        let uniform = (0..plc.regions.len())
            .all(|i| region_limit(plc, opts, i as i32) == same)
            .then_some(same);
        let locator = uniform.is_none().then(|| self.cdt.clone());
        let max_area = |p: &[Point2<f64>; 3]| {
            let c = centroid(p);
            let region = match uniform {
                Some(limit) => limit,
                None => labels_at(locator.as_ref().unwrap(), &labels, c)
                    .into_iter()
                    .map(|label| region_limit(plc, opts, label))
                    .fold(f64::INFINITY, f64::min),
            };
            let limit = region.min(sizing.limit(c));
            limit.is_finite().then_some(limit)
        };
        let kept = self.kept_edges(&labels);
        let params = RefinementParameters::<f64>::new()
            .with_angle_limit(AngleLimit::from_radius_to_shortest_edge_ratio(
                radius_edge_ratio(self.opts.min_angle),
            ))
            .with_max_additional_vertices(room);
        let before = self.cdt.num_vertices();
        let stopped = std::cell::RefCell::new(None);
        let result =
            self.cdt
                .refine_with_kept_edges(params, &excluded, &kept, max_area, |nodes, cells| {
                    // Four enclosing frame vertices are implementation details, not
                    // input/output nodes. Working cells include exterior cells.
                    match crate::control::checkpoint(nodes.saturating_sub(4), cells, true) {
                        Ok(()) => true,
                        Err(e) => {
                            *stopped.borrow_mut() = Some(e);
                            false
                        }
                    }
                });
        if let Some(error) = stopped.into_inner() {
            return Err(error);
        }
        self.skipped_faces = result
            .skipped_faces
            .into_iter()
            .map(|(face, vertices, reason)| (face.index(), (vertices, reason)))
            .collect();
        self.report.refine_calls += 1;
        self.update_parents(before)?;
        if !result.refinement_complete {
            self.report.issues.push(format!(
                "the vertex budget ({}) ran out during refinement",
                self.opts.max_vertices
            ));
        }
        Ok(())
    }

    /// The constraint edges refinement must not split: the pieces of the
    /// segments in [`Options::keep_segments`] and, with
    /// [`Options::keep_boundary`], every piece with the domain on one side
    /// only (outside or a hole on the other, as `labels` has it).
    fn kept_edges(&self, labels: &[i32]) -> Vec<[FixedVertexHandle; 2]> {
        if self.opts.keep_segments.is_empty() && !self.opts.keep_boundary {
            return Vec::new();
        }
        let mut keep = vec![false; self.plc.segments.len()];
        for &i in &self.opts.keep_segments {
            keep[i] = true;
        }
        let excluded = |face: Option<FixedFaceHandle<InnerTag>>| {
            face.is_none_or(|f| labels[f.index()] == EXCLUDED)
        };
        self.cdt
            .undirected_edges()
            .filter(|e| e.is_constraint_edge())
            .filter(|e| {
                let [a, b] = e.vertices().map(|v| v.fix().index());
                let d = e.as_directed();
                let one_side = excluded(d.face().as_inner().map(|f| f.fix()))
                    != excluded(d.rev().face().as_inner().map(|f| f.fix()));
                self.parents.get(&key(a, b)).is_some_and(|&p| keep[p])
                    || (self.opts.keep_boundary && one_side)
            })
            .map(|e| e.vertices().map(|v| v.fix()))
            .collect()
    }

    /// Labels every face: [`EXCLUDED`], [`NO_SEED`], or the index of the
    /// region seed that reaches it (a later seed wins). Seed problems are
    /// reported once, by [`Self::seed_warnings`].
    fn classify(&self) -> Vec<i32> {
        let mut warnings = Vec::new();
        let mut labels = vec![NO_SEED; self.cdt.num_all_faces()];
        labels[spade::handles::OUTER_FACE.index()] = EXCLUDED;
        let exterior: Vec<_> = self
            .cdt
            .convex_hull()
            .map(|e| e.rev())
            .filter(|e| !e.is_constraint_edge())
            .filter_map(|e| e.face().as_inner().map(|f| f.fix()))
            .collect();
        self.flood(exterior, EXCLUDED, &mut labels);
        for (i, hole) in self.plc.holes.iter().enumerate() {
            let start = self.seed_faces(*hole, "hole", i, &mut warnings);
            self.flood(start, EXCLUDED, &mut labels);
        }
        for (i, region) in self.plc.regions.iter().enumerate() {
            let start = self.seed_faces(region.point, "region", i, &mut warnings);
            if start.iter().any(|f| labels[f.index()] == EXCLUDED) {
                continue;
            }
            self.flood(start, i as i32, &mut labels);
        }
        labels
    }

    fn seed_warnings(&self, labels: &[i32]) -> Vec<String> {
        let mut warnings = Vec::new();
        for (i, hole) in self.plc.holes.iter().enumerate() {
            self.seed_faces(*hole, "hole", i, &mut warnings);
        }
        for (i, region) in self.plc.regions.iter().enumerate() {
            let start = self.seed_faces(region.point, "region", i, &mut warnings);
            if start.iter().any(|f| labels[f.index()] == EXCLUDED) {
                warnings.push(format!(
                    "region seed {i} lies outside the domain or in a hole; ignored"
                ));
            }
        }
        warnings
    }

    /// The face(s) a seed starts in.
    fn seed_faces(
        &self,
        p: [f64; 2],
        what: &str,
        index: usize,
        warnings: &mut Vec<String>,
    ) -> Vec<FixedFaceHandle<InnerTag>> {
        match self.cdt.locate(Point2::new(p[0], p[1])) {
            PositionInTriangulation::OnFace(f) => vec![f],
            PositionInTriangulation::OnEdge(e) => {
                let e = self.cdt.directed_edge(e);
                let faces: Vec<_> = [e.face(), e.rev().face()]
                    .iter()
                    .filter_map(|f| f.as_inner())
                    .map(|f| f.fix())
                    .collect();
                if e.is_constraint_edge() {
                    warnings.push(format!(
                        "{what} seed {index} lies on a segment; the side it fills is arbitrary"
                    ));
                    faces.into_iter().take(1).collect()
                } else {
                    faces
                }
            }
            PositionInTriangulation::OnVertex(v) => {
                let v = self.cdt.vertex(v);
                let on_segment = v.out_edges().any(|e| e.is_constraint_edge());
                let faces: Vec<_> = v
                    .out_edges()
                    .filter_map(|e| e.face().as_inner())
                    .map(|f| f.fix())
                    .collect();
                if on_segment {
                    warnings.push(format!("{what} seed {index} lies on a segment vertex; the side it fills is arbitrary"));
                    faces.into_iter().take(1).collect()
                } else {
                    faces
                }
            }
            PositionInTriangulation::OutsideOfConvexHull(_)
            | PositionInTriangulation::NoTriangulation => {
                warnings.push(format!(
                    "{what} seed {index} lies outside the mesh; ignored"
                ));
                Vec::new()
            }
        }
    }

    fn flood(&self, start: Vec<FixedFaceHandle<InnerTag>>, value: i32, labels: &mut [i32]) {
        let mut seen = vec![false; labels.len()];
        let mut stack = start;
        while let Some(f) = stack.pop() {
            let i = f.index();
            if seen[i] {
                continue;
            }
            seen[i] = true;
            labels[i] = value;
            for e in self.cdt.face(f).adjacent_edges() {
                if e.is_constraint_edge() {
                    continue;
                }
                if let Some(g) = e.rev().face().as_inner() {
                    if !seen[g.fix().index()] {
                        stack.push(g.fix());
                    }
                }
            }
        }
    }

    /// Re-derives which input segment every constraint edge belongs to after
    /// the triangulation gained vertices `before..`: a split segment is a
    /// chain of new vertices between the ends of an edge that existed before.
    /// A new vertex lies on two constraint edges of that chain, plus, where
    /// two segments cross, two edges already claimed by the other segment.
    fn update_parents(&mut self, before: usize) -> Result<(), MeshError> {
        let mut current = HashSet::new();
        let mut adjacency: HashMap<usize, Vec<usize>> = HashMap::new();
        for e in self.cdt.undirected_edges() {
            if !e.is_constraint_edge() {
                continue;
            }
            let [a, b] = e.vertices().map(|v| v.fix().index());
            current.insert(key(a, b));
            adjacency.entry(a).or_default().push(b);
            adjacency.entry(b).or_default().push(a);
        }
        let mut parents = HashMap::with_capacity(current.len());
        let mut missing = Vec::new();
        for (k, p) in self.parents.drain() {
            if current.contains(&k) {
                parents.insert(k, p);
            } else {
                missing.push((k, p));
            }
        }
        let empty = Vec::new();
        for ((a, b), p) in missing {
            let mut found = false;
            for &first in adjacency.get(&a).unwrap_or(&empty) {
                if first < before || parents.get(&key(a, first)).is_some_and(|&q| q != p) {
                    continue;
                }
                let mut chain = vec![key(a, first)];
                let (mut prev, mut cur) = (a, first);
                let mut ok = true;
                while cur >= before {
                    let options: Vec<usize> = adjacency
                        .get(&cur)
                        .unwrap_or(&empty)
                        .iter()
                        .copied()
                        .filter(|&n| n != prev && parents.get(&key(cur, n)).is_none_or(|&q| q == p))
                        .collect();
                    let [next] = options[..] else {
                        ok = false;
                        break;
                    };
                    chain.push(key(cur, next));
                    prev = cur;
                    cur = next;
                }
                if ok && cur == b {
                    for k in chain {
                        parents.insert(k, p);
                    }
                    found = true;
                    break;
                }
            }
            if !found {
                return Err(MeshError::Internal(format!(
                    "lost track of input segment {p} after refinement"
                )));
            }
        }
        if let Some(k) = current.iter().find(|k| !parents.contains_key(k)) {
            return Err(MeshError::Internal(format!(
                "constraint edge {k:?} belongs to no input segment"
            )));
        }
        self.parents = parents;
        Ok(())
    }

    fn export(mut self) -> Mesh2D {
        let labels = self.classify();
        self.report.warnings.extend(self.seed_warnings(&labels));
        let sizing = PointSizing::new(self.opts);
        let mut mesh = Mesh2D::default();
        let mut node_of = vec![usize::MAX; self.cdt.num_vertices()];
        let mut min_angle = 180.0f64;
        for f in self.cdt.inner_faces() {
            let label = labels[f.fix().index()];
            if label == EXCLUDED {
                continue;
            }
            let mut tri = [0usize; 3];
            for (k, v) in f.vertices().iter().enumerate() {
                let vi = v.fix().index();
                if node_of[vi] == usize::MAX {
                    node_of[vi] = mesh.nodes.len();
                    let p = v.position();
                    mesh.nodes.push([p.x, p.y]);
                }
                tri[k] = node_of[vi];
            }
            let angle = min_angle_deg(f.positions());
            min_angle = min_angle.min(angle);
            if self.opts.min_angle > 0.0 && angle < self.opts.min_angle - 1e-6 {
                self.report.angle_violations += 1;
                let reason = self
                    .skipped_faces
                    .get(&f.fix().index())
                    .filter(|(vertices, _)| *vertices == f.vertices().map(|v| v.fix()))
                    .map(|(_, reason)| *reason)
                    .unwrap_or(if self.report.issues.is_empty() {
                        "unresolved"
                    } else {
                        "budget_exhausted"
                    });
                self.report.angle_diagnostics.push(AngleDiagnostic {
                    cell: mesh.triangles.len(),
                    reason: reason.into(),
                });
            }
            let positions = f.positions();
            let limit =
                region_limit(self.plc, self.opts, label).min(sizing.limit(centroid(&positions)));
            if f.area().abs() > limit * (1.0 + 1e-9) {
                self.report.area_violations += 1;
            }
            mesh.triangles.push(tri);
            mesh.cell_regions.push(label);
            mesh.cell_markers.push(if label >= 0 {
                self.plc.regions[label as usize].marker
            } else {
                0
            });
        }
        mesh.node_markers = vec![0; mesh.nodes.len()];
        mesh.input_vertex_nodes = self
            .input_handles
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let node = node_of[h.index()];
                if node == usize::MAX {
                    -1
                } else {
                    mesh.node_markers[node] = self.plc.vertex_marker(i);
                    node as i64
                }
            })
            .collect();
        for e in self.cdt.undirected_edges() {
            let d = e.as_directed();
            let side = |f: Option<FixedFaceHandle<InnerTag>>| {
                f.map(|f| labels[f.index()] != EXCLUDED).unwrap_or(false)
            };
            let (left, right) = (
                side(d.face().as_inner().map(|f| f.fix())),
                side(d.rev().face().as_inner().map(|f| f.fix())),
            );
            if !left && !right {
                continue;
            }
            let [a, b] = e.vertices().map(|v| v.fix().index());
            let (marker, segment) = if e.is_constraint_edge() {
                let p = self.parents[&key(a, b)];
                (self.plc.segment_marker(p), p as i64)
            } else if left != right {
                (0, -1)
            } else {
                continue;
            };
            mesh.edges.push([node_of[a], node_of[b]]);
            mesh.edge_markers.push(marker);
            mesh.edge_segments.push(segment);
        }
        self.report.min_angle = if mesh.triangles.is_empty() {
            0.0
        } else {
            min_angle
        };
        self.report.vertices_outside = node_of
            .iter()
            .enumerate()
            .filter(|&(i, &n)| n == usize::MAX && !self.frame.contains(&i))
            .count();
        self.report.refinement_complete = self.report.issues.is_empty();
        self.report.quality_met = !mesh.triangles.is_empty()
            && self.report.area_violations == 0
            && self.report.angle_violations == 0;
        if self.report.area_violations > 0 {
            self.report.issues.push(format!(
                "{} cells are larger than their area limit",
                self.report.area_violations
            ));
        }
        if self.report.angle_violations > 0 {
            self.report.issues.push(format!(
                "{} cells are below the requested angle ({} degrees); small input angles may force some",
                self.report.angle_violations, self.opts.min_angle
            ));
        }
        if mesh.triangles.is_empty() {
            self.report
                .issues
                .push("the domain is empty: every cell is outside or in a hole".into());
        }
        self.report.complete = self.report.issues.is_empty();
        mesh.report = self.report;
        mesh
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plc::Region;

    /// Appends a closed polygon (vertices and segments) to `plc`.
    fn add_polygon(plc: &mut Plc, points: &[[f64; 2]], marker: i32) {
        let start = plc.vertices.len();
        plc.vertices.extend_from_slice(points);
        for i in 0..points.len() {
            plc.segments
                .push([start + i, start + (i + 1) % points.len()]);
            plc.segment_markers.push(marker);
        }
    }

    fn signed_area(p: &[[f64; 2]]) -> f64 {
        (0..p.len())
            .map(|i| {
                let (a, b) = (p[i], p[(i + 1) % p.len()]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum::<f64>()
            / 2.0
    }

    fn inside(poly: &[[f64; 2]], q: [f64; 2]) -> bool {
        let mut c = false;
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            if (a[1] > q[1]) != (b[1] > q[1])
                && q[0] < (b[0] - a[0]) * (q[1] - a[1]) / (b[1] - a[1]) + a[0]
            {
                c = !c;
            }
        }
        c
    }

    fn cell(m: &Mesh2D, i: usize) -> [[f64; 2]; 3] {
        m.triangles[i].map(|n| m.nodes[n])
    }

    /// Every cell is counter-clockwise, and every input segment is covered
    /// exactly by the edges that claim it.
    fn check_topology(plc: &Plc, m: &Mesh2D) {
        for i in 0..m.triangles.len() {
            assert!(
                signed_area(&cell(m, i)) > 0.0,
                "cell {i} is not counter-clockwise"
            );
        }
        let mut covered = vec![0.0; plc.segments.len()];
        for (e, &s) in m.edges.iter().zip(&m.edge_segments) {
            if s >= 0 {
                let (a, b) = (m.nodes[e[0]], m.nodes[e[1]]);
                covered[s as usize] += ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
            }
        }
        for (i, s) in plc.segments.iter().enumerate() {
            let (a, b) = (plc.vertices[s[0]], plc.vertices[s[1]]);
            let len = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
            assert!(
                (covered[i] - len).abs() < 1e-9 * len.max(1.0),
                "segment {i}: {} of {len}",
                covered[i]
            );
        }
    }

    #[test]
    fn square_meets_regional_area_and_angle() {
        let mut plc = Plc::default();
        add_polygon(
            &mut plc,
            &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            -2,
        );
        plc.regions.push(Region {
            point: [0.5, 0.5],
            marker: 2,
            max_area: 0.01,
        });
        let m = triangulate(&plc, &Options::default()).unwrap();
        assert!(m.report.complete, "{:?}", m.report);
        check_topology(&plc, &m);
        let areas: Vec<f64> = (0..m.triangles.len())
            .map(|i| signed_area(&cell(&m, i)))
            .collect();
        assert!((areas.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        assert!(areas.iter().all(|&a| a <= 0.01));
        assert!(m.report.min_angle >= 28.0 - 1e-6, "{}", m.report.min_angle);
        assert!(m.cell_markers.iter().all(|&k| k == 2));
        assert!(m.edge_markers.iter().all(|&k| k == -2));
        assert!(m.input_vertex_nodes.iter().all(|&n| n >= 0));
    }

    #[test]
    fn concave_surface_enclosed_zone_and_hole() {
        // A valley in the surface puts air inside the convex hull; the zone is
        // enclosed (spade's parity would drop it) and the hole must stay empty.
        let outer = [
            [-10.0, -10.0],
            [10.0, -10.0],
            [10.0, 0.0],
            [0.0, -3.0],
            [-10.0, 0.0],
        ];
        let zone = [[-2.0, -8.0], [2.0, -8.0], [2.0, -6.0], [-2.0, -6.0]];
        let hole = [[5.0, -8.0], [7.0, -8.0], [7.0, -6.0], [5.0, -6.0]];
        let mut plc = Plc::default();
        add_polygon(&mut plc, &outer, -1);
        add_polygon(&mut plc, &zone, 12);
        add_polygon(&mut plc, &hole, -3);
        plc.vertex_markers = vec![0; plc.vertices.len()];
        plc.vertex_markers[3] = -99;
        plc.regions.push(Region {
            point: [-8.0, -8.0],
            marker: 1,
            max_area: 0.0,
        });
        plc.regions.push(Region {
            point: [0.0, -7.0],
            marker: 3,
            max_area: 0.05,
        });
        plc.holes.push([6.0, -7.0]);
        let m = triangulate(&plc, &Options::default()).unwrap();
        assert!(m.report.complete, "{:?}", m.report);
        // No refinement vertex went into the air above the valley or the hole.
        assert_eq!(m.report.vertices_outside, 0, "{:?}", m.report);
        check_topology(&plc, &m);
        let mut total = 0.0;
        let mut zone_area = 0.0;
        for i in 0..m.triangles.len() {
            let c = cell(&m, i);
            let area = signed_area(&c);
            let centre = [
                (c[0][0] + c[1][0] + c[2][0]) / 3.0,
                (c[0][1] + c[1][1] + c[2][1]) / 3.0,
            ];
            assert!(
                inside(&outer, centre) && !inside(&hole, centre),
                "cell {i} lies in air or the hole"
            );
            let in_zone = inside(&zone, centre);
            assert_eq!(m.cell_markers[i], if in_zone { 3 } else { 1 });
            if in_zone {
                assert!(area <= 0.05);
                zone_area += area;
            }
            total += area;
        }
        let expected = signed_area(&outer) - signed_area(&hole);
        assert!((total - expected).abs() < 1e-9, "{total} vs {expected}");
        assert!((zone_area - 8.0).abs() < 1e-9);
        assert_eq!(m.node_markers[m.input_vertex_nodes[3] as usize], -99);
    }

    #[test]
    fn angle_refinement_reaches_a_domain_under_the_surface() {
        // pyGIMLi's parameter-mesh layout: the inner domain touches the ground
        // surface and sits inside the outer region, so it is one segment from
        // outside through the surface and two through the outer region.
        // spade's own inside/outside peel would leave it out of refinement.
        // Electrodes every 0.5 m on the surface leave skinny cells to refine.
        let mut plc = Plc::default();
        plc.vertices.push([-20.0, 0.0]);
        plc.vertices
            .extend((0..9).map(|i| [-2.0 + 0.5 * i as f64, 0.0]));
        plc.vertices.extend([
            [20.0, 0.0],
            [20.0, -20.0],
            [-20.0, -20.0],
            [2.0, -3.0],
            [-2.0, -3.0],
        ]);
        plc.segments = (0..10).map(|i| [i, i + 1]).collect();
        plc.segments
            .extend([[10, 11], [11, 12], [12, 0], [9, 13], [13, 14], [14, 1]]);
        plc.regions.push(Region {
            point: [-15.0, -15.0],
            marker: 1,
            max_area: 0.0,
        });
        plc.regions.push(Region {
            point: [0.0, -1.0],
            marker: 2,
            max_area: 0.0,
        });
        let m = triangulate(&plc, &Options::default()).unwrap();
        assert!(m.report.complete, "{:?}", m.report);
        check_topology(&plc, &m);
        assert!(m.report.min_angle >= 28.0 - 1e-6, "{:?}", m.report);
    }

    #[test]
    fn cells_grade_from_the_refine_points() {
        // A 10 m square with an electrode in the middle of its top edge.
        let mut plc = Plc::default();
        add_polygon(
            &mut plc,
            &[
                [0.0, 0.0],
                [10.0, 0.0],
                [10.0, 10.0],
                [5.0, 10.0],
                [0.0, 10.0],
            ],
            -1,
        );
        let opts = Options {
            refine_points: vec![[5.0, 10.0]],
            point_size: 0.2,
            size_growth: 0.5,
            ..Options::default()
        };
        let m = triangulate(&plc, &opts).unwrap();
        assert!(m.report.complete, "{:?}", m.report);
        assert_eq!(m.report.area_violations, 0);
        check_topology(&plc, &m);
        let electrode = m.input_vertex_nodes[3] as usize;
        let length =
            |a: [f64; 2], b: [f64; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
        let (mut near, mut far) = (Vec::new(), Vec::new());
        for (i, t) in m.triangles.iter().enumerate() {
            let c = cell(&m, i);
            if t.contains(&electrode) {
                for k in 0..3 {
                    assert!(
                        length(c[k], c[(k + 1) % 3]) <= 3.0 * opts.point_size,
                        "cell {i} at the electrode is too big"
                    );
                }
            }
            let centre = [
                (c[0][0] + c[1][0] + c[2][0]) / 3.0,
                (c[0][1] + c[1][1] + c[2][1]) / 3.0,
            ];
            let d = length(centre, [5.0, 10.0]);
            if d < 1.0 {
                near.push(signed_area(&c))
            } else if d > 6.0 {
                far.push(signed_area(&c))
            }
        }
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        assert!(
            mean(&far) > 10.0 * mean(&near),
            "near {} far {}",
            mean(&near),
            mean(&far)
        );
    }

    #[test]
    fn crossing_segments_are_split_where_they_cross() {
        let mut plc = Plc::default();
        add_polygon(
            &mut plc,
            &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            -2,
        );
        plc.segments.extend([[0, 2], [1, 3]]);
        plc.segment_markers.extend([5, 6]);
        let m = triangulate(&plc, &Options::default()).unwrap();
        assert!(m.report.complete, "{:?}", m.report);
        assert!(
            m.report.warnings.iter().any(|w| w.contains("cross")),
            "{:?}",
            m.report.warnings
        );
        check_topology(&plc, &m);
        assert!(m
            .nodes
            .iter()
            .any(|p| (p[0] - 0.5).abs() < 1e-12 && (p[1] - 0.5).abs() < 1e-12));
    }

    #[test]
    fn kept_segments_get_no_new_vertex() {
        // A box around a hole (as pyGIMLi's appendTriangleBoundary meshes
        // around an existing mesh) and an interior line in the box.
        let ring = |c: f64, n: usize| -> Vec<[f64; 2]> {
            let corners = [[-c, -c], [c, -c], [c, c], [-c, c]];
            (0..4)
                .flat_map(|k| {
                    let (a, b) = (corners[k], corners[(k + 1) % 4]);
                    (0..n).map(move |i| {
                        let t = i as f64 / n as f64;
                        [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]
                    })
                })
                .collect()
        };
        let mut plc = Plc::default();
        add_polygon(&mut plc, &ring(10.0, 2), -2);
        add_polygon(&mut plc, &ring(2.0, 4), 1);
        let line = plc.segments.len();
        plc.vertices.extend([[-8.0, 6.0], [8.0, 6.0]]);
        plc.segments
            .push([plc.vertices.len() - 2, plc.vertices.len() - 1]);
        plc.segment_markers.push(5);
        plc.vertex_markers = vec![0; plc.vertices.len()];
        plc.holes.push([0.0, 0.0]);
        let pieces =
            |m: &Mesh2D, s: usize| m.edge_segments.iter().filter(|&&k| k == s as i64).count();
        let boundary: Vec<usize> = (0..plc.segments.len()).filter(|&s| s != line).collect();
        let base = Options {
            max_area: 0.5,
            ..Options::default()
        };

        let free = triangulate(&plc, &base).unwrap();
        check_topology(&plc, &free);
        assert!(
            boundary.iter().any(|&s| pieces(&free, s) > 1),
            "the test needs split boundary segments"
        );
        assert!(pieces(&free, line) > 1);

        let kept = triangulate(
            &plc,
            &Options {
                keep_boundary: true,
                ..base.clone()
            },
        )
        .unwrap();
        check_topology(&plc, &kept);
        for &s in &boundary {
            assert_eq!(pieces(&kept, s), 1, "boundary segment {s} was split");
        }
        assert!(
            pieces(&kept, line) > 1,
            "an interior segment is not boundary"
        );
        assert!(kept
            .triangles
            .iter()
            .all(|t| t.iter().all(|&v| v < kept.nodes.len())));

        let line_kept = triangulate(
            &plc,
            &Options {
                keep_segments: vec![line],
                ..base.clone()
            },
        )
        .unwrap();
        check_topology(&plc, &line_kept);
        assert_eq!(pieces(&line_kept, line), 1);
        assert!(boundary.iter().any(|&s| pieces(&line_kept, s) > 1));

        let bad = triangulate(
            &plc,
            &Options {
                keep_segments: vec![plc.segments.len()],
                ..base
            },
        );
        assert!(matches!(bad, Err(MeshError::BadOption(_))));
    }
}
