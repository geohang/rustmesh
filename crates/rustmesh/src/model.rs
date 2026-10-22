//! A 2D geometry to mesh, described the way it is drawn: a domain outline
//! whose edges carry boundary markers, shapes inside it (material zones or
//! holes), lines the mesh must follow, and points that must be nodes.
//!
//! [`Geometry2D::plc`] turns the drawing into a [`Plc`]: everything is cut
//! where it crosses or touches something else, coinciding edges are merged,
//! parts outside the domain are dropped, and every enclosed area gets a
//! region seed with the marker and cell size of the shape it lies in (the
//! last one listed wins) or of the background. [`Geometry2D::mesh`] meshes
//! it. Coordinates are `(x, y)` in one length unit of the user's choice; for
//! a cross-section through the ground, y is elevation.
//!
//! Nothing here is specific to an application: the surveys in
//! [`crate::examples`] are built from these pieces.

use std::collections::HashMap;

use robust::{orient2d, Coord};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::error::MeshError;
use crate::mesh2d::{enclosed_areas, triangulate, Mesh2D, Options};
use crate::plc::{Plc, Region};

/// A material or zone: its name, cell marker and largest cell area.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(default))]
pub struct Zone {
    pub name: String,
    pub marker: i32,
    /// Largest cell area (0: no limit).
    pub max_area: f64,
}

impl Zone {
    pub fn new(name: &str, marker: i32, max_area: f64) -> Self {
        Self {
            name: name.to_owned(),
            marker,
            max_area,
        }
    }
}

impl Default for Zone {
    fn default() -> Self {
        Self::new("Domain", 1, 0.0)
    }
}

/// The outer boundary: corners in order (the first not repeated). Edge `i`
/// runs from corner `i` to the next and carries `edge_markers[i]` (0 where
/// the list is short).
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(default))]
pub struct Domain {
    pub outline: Vec<[f64; 2]>,
    pub edge_markers: Vec<i32>,
}

impl Domain {
    /// A rectangle; its edges are marked 1 (bottom), 2 (right), 3 (top), 4 (left).
    pub fn rectangle(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Self {
            outline: rectangle(x0, y0, x1, y1),
            edge_markers: vec![1, 2, 3, 4],
        }
    }

    pub fn edge_marker(&self, i: usize) -> i32 {
        self.edge_markers.get(i).copied().unwrap_or(0)
    }
}

/// A closed outline inside the domain: a zone of its own, or a hole. A
/// later shape wins over an earlier one where they overlap.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(default))]
pub struct Shape {
    pub zone: Zone,
    pub outline: Vec<[f64; 2]>,
    /// Marker of the shape's edges, where no domain edge or earlier edge
    /// with a marker runs.
    pub edge_marker: i32,
    /// No cells inside.
    pub hole: bool,
}

/// A polyline the mesh must follow, without it changing any zone (a crack,
/// a borehole, an internal boundary). Its edges carry `marker`.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(default))]
pub struct Line {
    pub name: String,
    pub points: Vec<[f64; 2]>,
    pub marker: i32,
}

/// A point that must be a mesh node (a sensor, a source, a load point),
/// with cells refined around it when `refine` is set.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(default))]
pub struct Point {
    pub at: [f64; 2],
    pub marker: i32,
    pub refine: bool,
}

/// How fine the mesh is.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(default))]
pub struct Sizing {
    /// Smallest cell angle in degrees.
    pub min_angle: f64,
    /// Largest cell area everywhere (0: none).
    pub max_area: f64,
    /// Cell edge length at the refined points (0: no point refinement).
    pub point_size: f64,
    /// How much the edge length grows per unit of distance from the nearest
    /// refined point.
    pub size_growth: f64,
}

impl Default for Sizing {
    fn default() -> Self {
        Self {
            min_angle: 28.0,
            max_area: 0.0,
            point_size: 0.0,
            size_growth: 0.5,
        }
    }
}

/// See the [module documentation](self).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(default))]
pub struct Geometry2D {
    pub name: String,
    /// Length unit of every coordinate and size, for display ("m", "mm", "km").
    pub units: String,
    pub domain: Domain,
    /// The zone of the domain outside every shape.
    pub background: Zone,
    pub shapes: Vec<Shape>,
    pub lines: Vec<Line>,
    pub points: Vec<Point>,
    pub sizing: Sizing,
}

impl Default for Geometry2D {
    fn default() -> Self {
        Self {
            name: "Untitled".into(),
            units: "m".into(),
            domain: Domain::rectangle(0.0, 0.0, 100.0, 50.0),
            background: Zone::default(),
            shapes: Vec::new(),
            lines: Vec::new(),
            points: Vec::new(),
            sizing: Sizing::default(),
        }
    }
}

/// A geometry turned into mesher input.
#[derive(Debug, Clone)]
pub struct ModelPlc {
    pub plc: Plc,
    /// Positions of the refined points.
    pub refine_points: Vec<[f64; 2]>,
    /// What was left out, in plain words.
    pub warnings: Vec<String>,
}

/// A rectangle's corners, counter-clockwise from `(x0, y0)`.
pub fn rectangle(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<[f64; 2]> {
    vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
}

/// An ellipse (a circle when `a == b`) as `n` corners.
pub fn ellipse(centre: [f64; 2], a: f64, b: f64, n: usize) -> Vec<[f64; 2]> {
    (0..n)
        .map(|i| 2.0 * std::f64::consts::PI * i as f64 / n as f64)
        .map(|t| [centre[0] + a * t.cos(), centre[1] + b * t.sin()])
        .collect()
}

/// `n` points from `a` to `b`, evenly spaced, ends included.
pub fn points_along(a: [f64; 2], b: [f64; 2], n: usize, marker: i32) -> Vec<Point> {
    (0..n)
        .map(|i| {
            if n > 1 {
                i as f64 / (n - 1) as f64
            } else {
                0.0
            }
        })
        .map(|t| Point {
            at: [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
            marker,
            refine: true,
        })
        .collect()
}

fn inside(polygon: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut odd = false;
    let mut j = polygon.len() - 1;
    for (i, a) in polygon.iter().enumerate() {
        let b = polygon[j];
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < a[0] + (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1])
        {
            odd = !odd;
        }
        j = i;
    }
    odd
}

/// Exact the same everywhere, unlike `hypot`, whose last bits depend on the
/// math library (a geometry must mesh the same in a browser and natively).
fn distance(p: [f64; 2], q: [f64; 2]) -> f64 {
    let (dx, dy) = (p[0] - q[0], p[1] - q[1]);
    (dx * dx + dy * dy).sqrt()
}

fn distance_to_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let len2 = d[0] * d[0] + d[1] * d[1];
    let t = if len2 > 0.0 {
        (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    distance(p, [a[0] + t * d[0], a[1] + t * d[1]])
}

/// Inside `polygon` or within `tol` of its outline.
fn covers(polygon: &[[f64; 2]], p: [f64; 2], tol: f64) -> bool {
    inside(polygon, p)
        || (0..polygon.len())
            .any(|i| distance_to_segment(p, polygon[i], polygon[(i + 1) % polygon.len()]) <= tol)
}

fn finite(points: &[[f64; 2]]) -> bool {
    points.iter().all(|p| p[0].is_finite() && p[1].is_finite())
}

/// Signed area, taken about the first corner: with absolute coordinates the
/// products of UTM eastings and northings (1e12 and more) swallow the area
/// of a small polygon.
fn area(polygon: &[[f64; 2]]) -> f64 {
    let n = polygon.len();
    let o = polygon[0];
    0.5 * (1..n.saturating_sub(1))
        .map(|i| {
            let (a, b) = (polygon[i], polygon[i + 1]);
            (a[0] - o[0]) * (b[1] - o[1]) - (b[0] - o[0]) * (a[1] - o[1])
        })
        .sum::<f64>()
}

impl Geometry2D {
    /// Zones in drawing order: the background, then every shape that is not a hole.
    pub fn zones(&self) -> Vec<&Zone> {
        std::iter::once(&self.background)
            .chain(self.shapes.iter().filter(|s| !s.hole).map(|s| &s.zone))
            .collect()
    }

    /// The domain's bounding box, `([xmin, ymin], [xmax, ymax])`.
    pub fn bounds(&self) -> ([f64; 2], [f64; 2]) {
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        for p in &self.domain.outline {
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        (lo, hi)
    }

    /// Whether `p` lies inside the domain outline (holes included).
    pub fn contains(&self, p: [f64; 2]) -> bool {
        self.domain.outline.len() >= 3 && inside(&self.domain.outline, p)
    }

    /// The zone at `p`; `None` outside the domain or in a hole.
    pub fn zone_at(&self, p: [f64; 2]) -> Option<&Zone> {
        if self.domain.outline.len() < 3 || !inside(&self.domain.outline, p) {
            return None;
        }
        match self
            .shapes
            .iter()
            .rev()
            .find(|s| s.outline.len() >= 3 && inside(&s.outline, p))
        {
            Some(s) if s.hole => None,
            Some(s) => Some(&s.zone),
            None => Some(&self.background),
        }
    }

    /// Rejects a geometry that cannot be meshed, saying what to fix.
    pub fn validate(&self) -> Result<(), MeshError> {
        let bad = |msg: String| Err(MeshError::BadOption(msg));
        let outline = &self.domain.outline;
        if outline.len() < 3 || !finite(outline) {
            return bad("the domain needs at least three corners, all with numbers".into());
        }
        if area(outline).abs() == 0.0 {
            return bad("the domain outline encloses no area".into());
        }
        for (i, s) in self.shapes.iter().enumerate() {
            if s.outline.len() < 3 || !finite(&s.outline) {
                return bad(format!(
                    "shape {} (\"{}\") needs at least three corners, all with numbers",
                    i + 1,
                    s.zone.name
                ));
            }
        }
        for (i, l) in self.lines.iter().enumerate() {
            if l.points.len() < 2 || !finite(&l.points) {
                return bad(format!(
                    "line {} (\"{}\") needs at least two points, all with numbers",
                    i + 1,
                    l.name
                ));
            }
        }
        if let Some(i) = self.points.iter().position(|p| !finite(&[p.at])) {
            return bad(format!(
                "point {} has a position that is not a number",
                i + 1
            ));
        }
        Ok(())
    }

    /// The mesher input; see the [module documentation](self).
    pub fn plc(&self) -> Result<ModelPlc, MeshError> {
        self.validate()?;
        let outline = &self.domain.outline;
        let (lo, hi) = self.bounds();
        // A billionth of the domain, but never below what the coordinates can
        // resolve: far from the origin (UTM eastings and northings) the last
        // bits of a computed crossing are worth more than that.
        let far = lo.iter().chain(&hi).fold(0.0f64, |m, v| m.max(v.abs()));
        let tol = (1e-9 * (hi[0] - lo[0]).max(hi[1] - lo[1])).max(64.0 * f64::EPSILON * far);

        let mut noder = Noder::new(tol);
        let ids: Vec<usize> = outline.iter().map(|&p| noder.vertex(p)).collect();
        for i in 0..ids.len() {
            let (a, b) = (ids[i], ids[(i + 1) % ids.len()]);
            if a != b {
                noder.segments.push(Piece {
                    a,
                    b,
                    marker: self.domain.edge_marker(i),
                    kinds: BOUNDARY,
                });
            }
        }
        for s in &self.shapes {
            noder.polyline(&s.outline, true, s.edge_marker, SHAPE);
        }
        for l in &self.lines {
            noder.polyline(&l.points, false, l.marker, LINE);
        }
        let mut warnings = Vec::new();
        let point_nodes: Vec<Option<usize>> = self
            .points
            .iter()
            .enumerate()
            .map(|(i, p)| {
                if covers(outline, p.at, tol) {
                    Some(noder.vertex(p.at))
                } else {
                    warnings.push(format!(
                        "point {} at ({}, {}) lies outside the domain; left out",
                        i + 1,
                        p.at[0],
                        p.at[1]
                    ));
                    None
                }
            })
            .collect();
        let points: Vec<usize> = point_nodes.iter().flatten().copied().collect();
        let (nodes, pieces) = noder.node(&points);

        // Keep the domain's edges and what lies inside the domain.
        let mut plc = Plc::default();
        let mut index = vec![usize::MAX; nodes.len()];
        let mut node = |plc: &mut Plc, i: usize| {
            if index[i] == usize::MAX {
                index[i] = plc.vertices.len();
                plc.vertices.push(nodes[i]);
                plc.vertex_markers.push(0);
            }
            index[i]
        };
        for p in &pieces {
            let (a, b) = (nodes[p.a], nodes[p.b]);
            let m = [0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1])];
            if p.kinds & BOUNDARY == 0 && !inside(outline, m) {
                continue;
            }
            let (a, b) = (node(&mut plc, p.a), node(&mut plc, p.b));
            plc.segments.push([a, b]);
            plc.segment_markers.push(p.marker);
        }
        let mut refine_points = Vec::new();
        for (p, n) in self.points.iter().zip(&point_nodes) {
            if let Some(&n) = n.as_ref() {
                let v = node(&mut plc, n);
                plc.vertex_markers[v] = p.marker;
                if p.refine {
                    refine_points.push(plc.vertices[v]);
                }
            }
        }
        for p in enclosed_areas(&plc)? {
            match self.zone_at(p) {
                Some(z) => plc.regions.push(Region {
                    point: p,
                    marker: z.marker,
                    max_area: z.max_area,
                }),
                None => plc.holes.push(p),
            }
        }
        Ok(ModelPlc {
            plc,
            refine_points,
            warnings,
        })
    }

    /// Mesher options from [`Self::sizing`] and the refined points.
    pub fn options(&self, refine_points: Vec<[f64; 2]>, max_vertices: usize) -> Options {
        Options {
            min_angle: self.sizing.min_angle,
            max_area: self.sizing.max_area,
            max_vertices,
            refine_points,
            point_size: self.sizing.point_size,
            size_growth: self.sizing.size_growth,
            ..Options::default()
        }
    }

    /// Meshes the geometry; what was left out is in `report.warnings`.
    pub fn mesh(&self, max_vertices: usize) -> Result<Mesh2D, MeshError> {
        let ModelPlc {
            plc,
            refine_points,
            warnings,
        } = self.plc()?;
        let mut mesh = triangulate(&plc, &self.options(refine_points, max_vertices))?;
        let mut all = warnings;
        all.append(&mut mesh.report.warnings);
        mesh.report.warnings = all;
        Ok(mesh)
    }
}

const BOUNDARY: u8 = 1;
const SHAPE: u8 = 2;
const LINE: u8 = 4;

/// A straight piece of the geometry, `kinds` saying what it is part of.
#[derive(Debug, Clone, Copy)]
struct Piece {
    a: usize,
    b: usize,
    marker: i32,
    kinds: u8,
}

/// Collects polylines and cuts them where they meet. Points closer than
/// `tol` are one point.
struct Noder {
    tol: f64,
    points: Vec<[f64; 2]>,
    grid: HashMap<(i64, i64), Vec<usize>>,
    segments: Vec<Piece>,
}

impl Noder {
    fn new(tol: f64) -> Self {
        Self {
            tol,
            points: Vec::new(),
            grid: HashMap::new(),
            segments: Vec::new(),
        }
    }

    fn vertex(&mut self, p: [f64; 2]) -> usize {
        let (kx, ky) = (
            (p[0] / self.tol).floor() as i64,
            (p[1] / self.tol).floor() as i64,
        );
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &i in self.grid.get(&(kx + dx, ky + dy)).into_iter().flatten() {
                    if distance(self.points[i], p) <= self.tol {
                        return i;
                    }
                }
            }
        }
        self.points.push(p);
        self.grid
            .entry((kx, ky))
            .or_default()
            .push(self.points.len() - 1);
        self.points.len() - 1
    }

    fn polyline(&mut self, points: &[[f64; 2]], closed: bool, marker: i32, kinds: u8) {
        let ids: Vec<usize> = points.iter().map(|&p| self.vertex(p)).collect();
        let n = ids.len();
        let count = if closed { n } else { n.saturating_sub(1) };
        for k in 0..count {
            let (a, b) = (ids[k], ids[(k + 1) % n]);
            if a != b {
                self.segments.push(Piece {
                    a,
                    b,
                    marker,
                    kinds,
                });
            }
        }
    }

    /// Whether point `v` lies on `s` between its ends.
    fn on_interior(&self, v: usize, s: &Piece) -> bool {
        if v == s.a || v == s.b {
            return false;
        }
        let (p, a, b) = (self.points[v], self.points[s.a], self.points[s.b]);
        let d = [b[0] - a[0], b[1] - a[1]];
        let t = ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / (d[0] * d[0] + d[1] * d[1]);
        if !(t > 0.0 && t < 1.0) {
            return false;
        }
        distance(p, [a[0] + t * d[0], a[1] + t * d[1]]) <= self.tol
    }

    /// Where segments `i` and `j` meet: an end of one on the other, or a crossing.
    fn meet(&mut self, i: usize, j: usize, cuts: &mut [Vec<usize>]) {
        let (si, sj) = (self.segments[i], self.segments[j]);
        for v in [sj.a, sj.b] {
            if self.on_interior(v, &si) {
                cuts[i].push(v);
            }
        }
        for v in [si.a, si.b] {
            if self.on_interior(v, &sj) {
                cuts[j].push(v);
            }
        }
        if si.a == sj.a || si.a == sj.b || si.b == sj.a || si.b == sj.b {
            return;
        }
        let c = |v: usize| Coord {
            x: self.points[v][0],
            y: self.points[v][1],
        };
        let (o1, o2) = (
            orient2d(c(si.a), c(si.b), c(sj.a)),
            orient2d(c(si.a), c(si.b), c(sj.b)),
        );
        let (o3, o4) = (
            orient2d(c(sj.a), c(sj.b), c(si.a)),
            orient2d(c(sj.a), c(sj.b), c(si.b)),
        );
        let opposite = |p: f64, q: f64| (p > 0.0 && q < 0.0) || (p < 0.0 && q > 0.0);
        if opposite(o1, o2) && opposite(o3, o4) {
            let t = o3 / (o3 - o4);
            let (a, b) = (self.points[si.a], self.points[si.b]);
            let v = self.vertex([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]);
            if v != si.a && v != si.b {
                cuts[i].push(v);
            }
            if v != sj.a && v != sj.b {
                cuts[j].push(v);
            }
        }
    }

    /// Cuts every segment where another segment or one of `points` meets
    /// it, then merges pieces that coincide: the first one's marker is kept,
    /// a zero marker giving way to a later non-zero one.
    fn node(mut self, points: &[usize]) -> (Vec<[f64; 2]>, Vec<Piece>) {
        let n = self.segments.len();
        let tol = self.tol;
        let bounds: Vec<[f64; 4]> = self
            .segments
            .iter()
            .map(|s| {
                let (a, b) = (self.points[s.a], self.points[s.b]);
                [
                    a[0].min(b[0]) - tol,
                    a[0].max(b[0]) + tol,
                    a[1].min(b[1]) - tol,
                    a[1].max(b[1]) + tol,
                ]
            })
            .collect();
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&i, &j| bounds[i][0].total_cmp(&bounds[j][0]));
        let mut cuts = vec![Vec::new(); n];
        for (k, &i) in order.iter().enumerate() {
            for &j in &order[k + 1..] {
                if bounds[j][0] > bounds[i][1] {
                    break;
                }
                if bounds[j][2] <= bounds[i][3] && bounds[i][2] <= bounds[j][3] {
                    self.meet(i, j, &mut cuts);
                }
            }
        }
        for &v in points {
            let p = self.points[v];
            for (i, s) in self.segments.iter().enumerate() {
                let b = bounds[i];
                if p[0] >= b[0]
                    && p[0] <= b[1]
                    && p[1] >= b[2]
                    && p[1] <= b[3]
                    && self.on_interior(v, s)
                {
                    cuts[i].push(v);
                }
            }
        }
        let mut pieces: Vec<Piece> = Vec::new();
        let mut seen: HashMap<(usize, usize), usize> = HashMap::new();
        for (s, mut chain) in self.segments.iter().zip(cuts) {
            let (a, b) = (self.points[s.a], self.points[s.b]);
            let along = |v: usize| {
                let p = self.points[v];
                (p[0] - a[0]) * (b[0] - a[0]) + (p[1] - a[1]) * (b[1] - a[1])
            };
            chain.sort_by(|&u, &v| along(u).total_cmp(&along(v)));
            chain.insert(0, s.a);
            chain.push(s.b);
            chain.dedup();
            for w in chain.windows(2) {
                let key = (w[0].min(w[1]), w[0].max(w[1]));
                match seen.get(&key) {
                    Some(&k) => {
                        let p = &mut pieces[k];
                        p.kinds |= s.kinds;
                        if p.marker == 0 {
                            p.marker = s.marker;
                        }
                    }
                    None => {
                        seen.insert(key, pieces.len());
                        pieces.push(Piece {
                            a: w[0],
                            b: w[1],
                            ..*s
                        });
                    }
                }
            }
        }
        (self.points, pieces)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell_area(mesh: &Mesh2D) -> f64 {
        mesh.cell_areas().iter().sum()
    }

    #[test]
    fn pieces_that_cross_overlap_or_touch_are_cut_and_merged() {
        // A domain with a sloping top, a shape whose top runs along it, a
        // crossing band, a line through both, a hole, and points on a sloping
        // edge, on the line and outside.
        let mut g = Geometry2D {
            domain: Domain {
                outline: vec![
                    [-10.0, -10.0],
                    [10.0, -10.0],
                    [10.0, 0.0],
                    [0.0, 2.0],
                    [-10.0, 0.0],
                ],
                edge_markers: vec![2, 2, 1, 1, 2],
            },
            ..Geometry2D::default()
        };
        g.shapes.push(Shape {
            zone: Zone::new("Band", 2, 0.0),
            outline: vec![[-10.0, -3.0], [10.0, 1.0], [10.0, -10.0], [-10.0, -10.0]],
            ..Shape::default()
        });
        g.shapes.push(Shape {
            zone: Zone::new("Target", 3, 0.5),
            outline: vec![
                [-4.0, -6.0],
                [4.0, -6.0],
                [4.0, 1.2],
                [0.0, 2.0],
                [-4.0, 1.2],
            ],
            ..Shape::default()
        });
        g.shapes.push(Shape {
            zone: Zone::new("Hole", 0, 0.0),
            outline: ellipse([6.0, -7.0], 1.0, 1.0, 16),
            hole: true,
            ..Shape::default()
        });
        g.lines.push(Line {
            name: "Crack".into(),
            points: vec![[-8.0, -9.0], [6.0, 3.0]],
            marker: 7,
        });
        g.points.push(Point {
            at: [-1.0, -3.0],
            marker: -99,
            refine: true,
        });
        g.points.push(Point {
            at: [1.0, 1.8],
            marker: -99,
            refine: true,
        });
        g.points.push(Point {
            at: [0.0, 50.0],
            marker: -99,
            refine: true,
        });

        let mesh = g.mesh(1_000_000).unwrap();
        assert!(mesh.report.refinement_complete, "{:?}", mesh.report);
        assert_eq!(mesh.report.area_violations, 0);
        assert_eq!(mesh.report.angle_violations, 3);
        assert!(!mesh.report.complete);
        assert_eq!(mesh.report.warnings.len(), 1, "{:?}", mesh.report.warnings);
        let expected = 20.0 * 10.0 + 20.0 - area(&ellipse([6.0, -7.0], 1.0, 1.0, 16));
        assert!(
            (cell_area(&mesh) - expected).abs() < 1e-9,
            "{} vs {expected}",
            cell_area(&mesh)
        );
        for (t, &m) in mesh.triangles.iter().zip(&mesh.cell_markers) {
            let [a, b, c] = t.map(|k| mesh.nodes[k]);
            let centre = [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0];
            assert_eq!(g.zone_at(centre).unwrap().marker, m, "cell at {centre:?}");
        }
        for marker in [1, 2, 3] {
            assert!(mesh.cell_markers.contains(&marker), "zone {marker} missing");
        }
        assert!(mesh
            .cell_areas()
            .iter()
            .zip(&mesh.cell_markers)
            .all(|(&a, &m)| m != 3 || a <= 0.5 + 1e-9));
        assert_eq!(mesh.node_markers.iter().filter(|&&m| m == -99).count(), 2);
        for marker in [1, 2, 7] {
            assert!(
                mesh.edge_markers.contains(&marker),
                "edge marker {marker} missing"
            );
        }
    }
}
