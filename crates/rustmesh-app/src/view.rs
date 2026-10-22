//! The drawing: geometry outline, points and mesh cells, with pan (drag),
//! zoom (scroll or pinch, around the pointer), fit (double-click), axes and a
//! legend.

use eframe::egui::{self, Align2, Color32, FontId, Mesh, Pos2, Rect, Sense, Shape, Stroke, Vec2};
use rustmesh::{Geometry2D, Mesh2D};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorBy {
    Zone,
    Quality,
    Size,
}

impl ColorBy {
    pub const ALL: [ColorBy; 3] = [ColorBy::Zone, ColorBy::Quality, ColorBy::Size];

    pub fn label(self) -> &'static str {
        match self {
            ColorBy::Zone => "Zones",
            ColorBy::Quality => "Smallest angle",
            ColorBy::Size => "Cell size",
        }
    }
}

/// A mesh with what the drawing needs from it, worked out once.
pub struct Picture {
    pub mesh: Mesh2D,
    min_angles: Vec<f64>,
    log_areas: Vec<f64>,
    log_area_range: (f64, f64),
}

impl Picture {
    pub fn new(mesh: Mesh2D) -> Self {
        let min_angles = mesh.cell_min_angles();
        let log_areas: Vec<f64> = mesh
            .cell_areas()
            .iter()
            .map(|a| a.max(1e-300).log10())
            .collect();
        let lo = log_areas.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = log_areas.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Self {
            mesh,
            min_angles,
            log_areas,
            log_area_range: (lo, hi),
        }
    }

    pub fn smallest_angle(&self) -> f64 {
        self.min_angles
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min)
    }

    pub fn cells_below(&self, angle: f64) -> usize {
        self.min_angles
            .iter()
            .filter(|&&a| a < angle - 1e-6)
            .count()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// The whole domain.
    All,
    /// The points, lines and shapes smaller than the domain.
    Details,
}

pub struct View {
    /// World point at the middle of the drawing.
    center: [f64; 2],
    /// Pixels per unit across.
    scale: f64,
    pub exaggeration: f64,
    pub color_by: ColorBy,
    pub show_edges: bool,
    pub show_geometry: bool,
    pending: Option<Fit>,
}

impl Default for View {
    fn default() -> Self {
        Self {
            center: [0.0, 0.0],
            scale: 0.0,
            exaggeration: 1.0,
            color_by: ColorBy::Zone,
            show_edges: true,
            show_geometry: true,
            pending: Some(Fit::All),
        }
    }
}

const PALETTE: [(u8, u8, u8); 10] = [
    (182, 211, 242),
    (172, 202, 234),
    (170, 215, 205),
    (200, 191, 228),
    (235, 199, 165),
    (224, 183, 201),
    (160, 206, 222),
    (222, 216, 169),
    (190, 198, 210),
    (184, 210, 176),
];

pub fn zone_color(marker: i32, dark: bool) -> Color32 {
    let (r, g, b) = PALETTE[marker.rem_euclid(PALETTE.len() as i32) as usize];
    let k = if dark { 0.55 } else { 1.0 };
    Color32::from_rgb(
        (r as f32 * k) as u8,
        (g as f32 * k) as u8,
        (b as f32 * k) as u8,
    )
}

fn ramp(stops: &[(u8, u8, u8)], t: f64) -> Color32 {
    let t = t.clamp(0.0, 1.0) * (stops.len() - 1) as f64;
    let i = (t.floor() as usize).min(stops.len() - 2);
    let f = (t - i as f64) as f32;
    let (a, b) = (stops[i], stops[i + 1]);
    let mix = |p: u8, q: u8| (p as f32 + (q as f32 - p as f32) * f) as u8;
    Color32::from_rgb(mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2))
}

const QUALITY: [(u8, u8, u8); 3] = [(215, 48, 39), (254, 224, 139), (26, 152, 80)];
const SIZE: [(u8, u8, u8); 5] = [
    (68, 1, 84),
    (59, 82, 139),
    (33, 145, 140),
    (94, 201, 98),
    (253, 231, 37),
];

/// A straight band from `a` to `b`, cheaper than a stroked line when there
/// are hundreds of thousands of them.
fn add_band(mesh: &mut Mesh, a: Pos2, b: Pos2, width: f32, color: Color32) {
    let d = b - a;
    let len = d.length();
    if len < 0.05 {
        return;
    }
    let n = Vec2::new(-d.y, d.x) * (0.5 * width / len);
    let i = mesh.vertices.len() as u32;
    for p in [a + n, a - n, b - n, b + n] {
        mesh.colored_vertex(p, color);
    }
    mesh.add_triangle(i, i + 1, i + 2);
    mesh.add_triangle(i, i + 2, i + 3);
}

/// A round step for axis ticks about `raw` long.
fn nice_step(raw: f64) -> f64 {
    let p = 10f64.powf(raw.log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|k| k * p)
        .find(|&s| s >= raw)
        .unwrap_or(10.0 * p)
}

fn number(v: f64, step: f64) -> String {
    let digits = (-step.log10().floor()).max(0.0) as usize;
    format!("{v:.digits$}")
}

impl View {
    pub fn fit(&mut self, fit: Fit) {
        self.pending = Some(fit);
    }

    fn to_screen(&self, rect: Rect, p: [f64; 2]) -> Pos2 {
        let c = rect.center();
        Pos2::new(
            c.x + ((p[0] - self.center[0]) * self.scale) as f32,
            c.y - ((p[1] - self.center[1]) * self.scale * self.exaggeration) as f32,
        )
    }

    fn to_world(&self, rect: Rect, p: Pos2) -> [f64; 2] {
        let c = rect.center();
        [
            self.center[0] + (p.x - c.x) as f64 / self.scale,
            self.center[1] - (p.y - c.y) as f64 / (self.scale * self.exaggeration),
        ]
    }

    fn fit_box(&mut self, rect: Rect, lo: [f64; 2], hi: [f64; 2]) {
        let w = (hi[0] - lo[0]).max(1e-12);
        let h = ((hi[1] - lo[1]) * self.exaggeration).max(1e-12);
        self.scale = 0.82 * (rect.width() as f64 / w).min(rect.height() as f64 / h);
        self.center = [0.5 * (lo[0] + hi[0]), 0.5 * (lo[1] + hi[1])];
    }

    fn apply_fit(&mut self, fit: Fit, rect: Rect, g: &Geometry2D) {
        let whole = g.bounds();
        let (span_x, span_y) = (whole.1[0] - whole.0[0], whole.1[1] - whole.0[1]);
        // Details: what is much smaller than the domain (sensors, a hole, a
        // body), not shapes spanning most of it (layers, a ground region).
        let mut points: Vec<[f64; 2]> = g.points.iter().map(|p| p.at).collect();
        for outline in g
            .shapes
            .iter()
            .map(|s| &s.outline)
            .chain(g.lines.iter().map(|l| &l.points))
        {
            let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            for p in outline {
                for k in 0..2 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            if hi[0] - lo[0] < 0.5 * span_x && hi[1] - lo[1] < 0.5 * span_y {
                points.extend(outline.iter().copied());
            }
        }
        let (lo, hi) = if fit == Fit::All || points.is_empty() || !whole.0[0].is_finite() {
            whole
        } else {
            let mut lo = [f64::INFINITY; 2];
            let mut hi = [f64::NEG_INFINITY; 2];
            for p in &points {
                for k in 0..2 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            let pad = 0.15
                * (hi[0] - lo[0])
                    .max(hi[1] - lo[1])
                    .max(1e-3 * span_x.max(span_y));
            ([lo[0] - pad, lo[1] - pad], [hi[0] + pad, hi[1] + pad])
        };
        if lo[0].is_finite() && hi[0].is_finite() {
            self.fit_box(rect, lo, hi);
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        g: &Geometry2D,
        picture: Option<&Picture>,
        target_angle: f64,
    ) {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;
        let visuals = ui.visuals().clone();
        let dark = visuals.dark_mode;
        painter.rect_filled(rect, 0.0, visuals.extreme_bg_color);

        // A requested fit holds, through window resizes and edits, until the
        // user pans or zooms.
        if let Some(fit) = self.pending {
            self.apply_fit(fit, rect, g);
        }
        if self.scale <= 0.0 || !self.scale.is_finite() {
            self.apply_fit(Fit::All, rect, g);
        }
        if response.double_clicked() {
            self.pending = Some(Fit::All);
            self.apply_fit(Fit::All, rect, g);
        }
        if response.dragged() {
            self.pending = None;
            let d = response.drag_delta();
            self.center[0] -= d.x as f64 / self.scale;
            self.center[1] += d.y as f64 / (self.scale * self.exaggeration);
        }
        if let Some(pointer) = response.hover_pos() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = pinch as f64 * (scroll as f64 * 0.002).exp();
            if factor != 1.0 && self.scale > 0.0 {
                self.pending = None;
                let before = self.to_world(rect, pointer);
                self.scale = (self.scale * factor).clamp(1e-12, 1e12);
                let after = self.to_world(rect, pointer);
                self.center[0] += before[0] - after[0];
                self.center[1] += before[1] - after[1];
            }
        }

        let ink = if dark {
            Color32::from_gray(225)
        } else {
            Color32::from_gray(35)
        };
        if let Some(picture) = picture {
            self.draw_cells(&painter, rect, picture, target_angle, dark);
        }
        if self.show_geometry || picture.is_none() {
            self.draw_geometry(
                &painter,
                rect,
                g,
                ink,
                picture.is_none(),
                dark,
                visuals.extreme_bg_color,
            );
        }
        let point = if dark {
            Color32::from_rgb(255, 120, 100)
        } else {
            Color32::from_rgb(180, 30, 30)
        };
        for p in &g.points {
            let s = self.to_screen(rect, p.at);
            if rect.expand(4.0).contains(s) {
                painter.circle_filled(s, 2.8, point);
            }
        }
        self.draw_axes(&painter, rect, &visuals, &g.units);
        self.draw_legend(&painter, rect, g, picture, target_angle, &visuals);

        if let Some(pointer) = response.hover_pos() {
            let p = self.to_world(rect, pointer);
            let zone = match g.zone_at(p) {
                Some(z) => format!("  ·  {} (marker {})", z.name, z.marker),
                None if g.contains(p) => "  ·  hole".into(),
                None => String::new(),
            };
            let step = nice_step(1.0 / self.scale);
            let text = format!("x {}  y {}{zone}", number(p[0], step), number(p[1], step));
            let galley = painter.layout_no_wrap(text, FontId::proportional(13.0), ink);
            let at = rect.right_top() + Vec2::new(-galley.size().x - 10.0, 8.0);
            painter.rect_filled(
                Rect::from_min_size(at, galley.size()).expand(4.0),
                4.0,
                visuals.window_fill,
            );
            painter.galley(at, galley, ink);
        }
    }

    fn draw_cells(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        picture: &Picture,
        target_angle: f64,
        dark: bool,
    ) {
        let mesh = &picture.mesh;
        let screen: Vec<Pos2> = mesh
            .nodes
            .iter()
            .map(|&p| self.to_screen(rect, p))
            .collect();
        let (lo, hi) = picture.log_area_range;
        let mut fill = Mesh::default();
        let mut visible = Vec::new();
        for (c, t) in mesh.triangles.iter().enumerate() {
            let [a, b, d] = t.map(|n| screen[n]);
            if !Rect::from_points(&[a, b, d]).intersects(rect) {
                continue;
            }
            visible.push(c);
            let color = match self.color_by {
                ColorBy::Zone => zone_color(mesh.cell_markers[c], dark),
                ColorBy::Quality => ramp(&QUALITY, picture.min_angles[c] / target_angle.max(1.0)),
                ColorBy::Size => ramp(
                    &SIZE,
                    if hi > lo {
                        (picture.log_areas[c] - lo) / (hi - lo)
                    } else {
                        0.5
                    },
                ),
            };
            let i = fill.vertices.len() as u32;
            for p in [a, b, d] {
                fill.colored_vertex(p, color);
            }
            fill.add_triangle(i, i + 1, i + 2);
        }
        painter.add(Shape::mesh(fill));
        // Cell edges only while there are few enough on screen to read.
        if self.show_edges && visible.len() <= 300_000 {
            let color = if dark {
                Color32::from_white_alpha(70)
            } else {
                Color32::from_black_alpha(65)
            };
            let mut wire = Mesh::default();
            for &c in &visible {
                let [a, b, d] = mesh.triangles[c].map(|n| screen[n]);
                for (p, q) in [(a, b), (b, d), (d, a)] {
                    add_band(&mut wire, p, q, 0.7, color);
                }
            }
            painter.add(Shape::mesh(wire));
        }
        let marked = if dark {
            Color32::from_gray(235)
        } else {
            Color32::from_gray(20)
        };
        let mut lines = Mesh::default();
        for (e, &[a, b]) in mesh.edges.iter().enumerate() {
            if mesh.edge_markers[e] != 0 {
                add_band(&mut lines, screen[a], screen[b], 1.6, marked);
            }
        }
        painter.add(Shape::mesh(lines));
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_geometry(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        g: &Geometry2D,
        ink: Color32,
        fill: bool,
        dark: bool,
        empty: Color32,
    ) {
        let screen = |points: &[[f64; 2]]| -> Vec<Pos2> {
            points.iter().map(|&p| self.to_screen(rect, p)).collect()
        };
        let polygon = |points: &[[f64; 2]], stroke: Stroke| {
            let mut pts = screen(points);
            if pts.len() >= 2 {
                pts.push(pts[0]);
                painter.add(Shape::line(pts, stroke));
            }
        };
        // Before meshing, show the zones as tinted areas: shapes are drawn in
        // order, so later ones cover earlier ones as they do in the mesh.
        if fill && g.domain.outline.len() >= 3 {
            let tint = |marker: i32| zone_color(marker, dark).gamma_multiply(0.45);
            if let Some(m) = triangulated(&screen(&g.domain.outline), tint(g.background.marker)) {
                painter.add(Shape::mesh(m));
            }
            for s in &g.shapes {
                let color = if s.hole { empty } else { tint(s.zone.marker) };
                if let Some(m) = triangulated(&screen(&s.outline), color) {
                    painter.add(Shape::mesh(m));
                }
            }
        }
        polygon(&g.domain.outline, Stroke::new(2.0, ink));
        for s in &g.shapes {
            let stroke = if s.hole {
                Stroke::new(1.6, ink)
            } else {
                Stroke::new(1.2, ink.gamma_multiply(0.75))
            };
            polygon(&s.outline, stroke);
        }
        let blue = Color32::from_rgb(40, 110, 210);
        for l in &g.lines {
            let pts = screen(&l.points);
            if pts.len() >= 2 {
                painter.add(Shape::line(pts, Stroke::new(2.0, blue)));
            }
        }
    }

    fn draw_axes(&self, painter: &egui::Painter, rect: Rect, visuals: &egui::Visuals, units: &str) {
        let color = visuals.weak_text_color();
        let font = FontId::proportional(11.0);
        let (lo, hi) = (
            self.to_world(rect, rect.left_bottom()),
            self.to_world(rect, rect.right_top()),
        );
        let step = nice_step(90.0 / self.scale);
        let mut x = (lo[0] / step).ceil() * step;
        while x <= hi[0] {
            let p = self.to_screen(rect, [x, 0.0]);
            painter.line_segment(
                [
                    Pos2::new(p.x, rect.bottom()),
                    Pos2::new(p.x, rect.bottom() - 5.0),
                ],
                (1.0, color),
            );
            painter.text(
                Pos2::new(p.x, rect.bottom() - 7.0),
                Align2::CENTER_BOTTOM,
                number(x, step),
                font.clone(),
                color,
            );
            x += step;
        }
        let step = nice_step(60.0 / (self.scale * self.exaggeration));
        let mut y = (lo[1] / step).ceil() * step;
        while y <= hi[1] {
            let p = self.to_screen(rect, [0.0, y]);
            painter.line_segment(
                [
                    Pos2::new(rect.left(), p.y),
                    Pos2::new(rect.left() + 5.0, p.y),
                ],
                (1.0, color),
            );
            painter.text(
                Pos2::new(rect.left() + 7.0, p.y),
                Align2::LEFT_CENTER,
                number(y, step),
                font.clone(),
                color,
            );
            y += step;
        }
        let label = if self.exaggeration == 1.0 {
            units.to_string()
        } else {
            format!("{units}, y × {}", self.exaggeration)
        };
        painter.text(
            rect.right_bottom() + Vec2::new(-7.0, -20.0),
            Align2::RIGHT_BOTTOM,
            label,
            font,
            color,
        );
    }

    fn draw_legend(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        g: &Geometry2D,
        picture: Option<&Picture>,
        target_angle: f64,
        visuals: &egui::Visuals,
    ) {
        let font = FontId::proportional(12.0);
        let ink = visuals.text_color();
        let mut rows: Vec<(Color32, String)> = Vec::new();
        match (self.color_by, picture) {
            (ColorBy::Quality, Some(_)) => {
                let t = target_angle.max(1.0);
                for (angle, label) in [
                    (t, format!("≥ {t:.0}° (target)")),
                    (0.6 * t, format!("{:.0}°", 0.6 * t)),
                    (0.0, "0°".into()),
                ] {
                    rows.push((ramp(&QUALITY, angle / t), label));
                }
            }
            (ColorBy::Size, Some(p)) => {
                let (lo, hi) = p.log_area_range;
                rows.push((
                    ramp(&SIZE, 1.0),
                    format!("{:.3e} {}²", 10f64.powf(hi), g.units),
                ));
                rows.push((
                    ramp(&SIZE, 0.0),
                    format!("{:.3e} {}²", 10f64.powf(lo), g.units),
                ));
            }
            _ => {
                let mut seen = Vec::new();
                for z in g.zones() {
                    if !seen.contains(&z.marker) {
                        seen.push(z.marker);
                        rows.push((
                            zone_color(z.marker, visuals.dark_mode),
                            format!("{} ({})", z.name, z.marker),
                        ));
                    }
                }
            }
        }
        if rows.is_empty() {
            return;
        }
        let width = rows
            .iter()
            .map(|(_, t)| {
                painter
                    .layout_no_wrap(t.clone(), font.clone(), ink)
                    .size()
                    .x
            })
            .fold(0.0, f32::max);
        let height = 18.0 * rows.len() as f32;
        let origin = rect.left_top() + Vec2::new(10.0, 10.0);
        painter.rect_filled(
            Rect::from_min_size(origin, Vec2::new(width + 34.0, height + 8.0)),
            6.0,
            visuals.window_fill,
        );
        for (i, (color, text)) in rows.into_iter().enumerate() {
            let y = origin.y + 4.0 + 18.0 * i as f32;
            painter.rect_filled(
                Rect::from_min_size(Pos2::new(origin.x + 6.0, y + 3.0), Vec2::splat(12.0)),
                2.0,
                color,
            );
            painter.text(
                Pos2::new(origin.x + 24.0, y + 9.0),
                Align2::LEFT_CENTER,
                text,
                font.clone(),
                ink,
            );
        }
    }
}

/// A filled polygon (any shape, not just convex) as an egui mesh, by ear
/// clipping; `None` when it is degenerate.
fn triangulated(points: &[Pos2], color: Color32) -> Option<Mesh> {
    let n = points.len();
    if n < 3 {
        return None;
    }
    let twice_area: f32 = (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            a.x * b.y - b.x * a.y
        })
        .sum();
    if twice_area == 0.0 {
        return None;
    }
    let ccw = twice_area > 0.0;
    let cross = |o: Pos2, a: Pos2, b: Pos2| (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
    let mut mesh = Mesh::default();
    for &p in points {
        mesh.colored_vertex(p, color);
    }
    let mut left: Vec<u32> = (0..n as u32).collect();
    let mut guard = 0;
    while left.len() > 3 && guard < 4 * n * n {
        guard += 1;
        let m = left.len();
        let mut clipped = false;
        for k in 0..m {
            let (i, j, l) = (left[(k + m - 1) % m], left[k], left[(k + 1) % m]);
            let (a, b, c) = (points[i as usize], points[j as usize], points[l as usize]);
            let turn = cross(a, b, c);
            if (turn > 0.0) != ccw || turn == 0.0 {
                continue;
            }
            let contains = left.iter().any(|&q| {
                if q == i || q == j || q == l {
                    return false;
                }
                let p = points[q as usize];
                let (d1, d2, d3) = (cross(a, b, p), cross(b, c, p), cross(c, a, p));
                if ccw {
                    d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0
                } else {
                    d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0
                }
            });
            if !contains {
                mesh.add_triangle(i, j, l);
                left.remove(k);
                clipped = true;
                break;
            }
        }
        if !clipped {
            break;
        }
    }
    if left.len() == 3 {
        mesh.add_triangle(left[0], left[1], left[2]);
    }
    Some(mesh)
}
