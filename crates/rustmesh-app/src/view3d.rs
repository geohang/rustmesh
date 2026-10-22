//! Orthographic triangle preview. Depth-sorted surfaces and whole-cell x cuts
//! keep the viewer portable across native egui and WebGL without another engine.
use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, Vec2};
use rustmesh::{Mesh3D, Plc3D};

type Point = [f64; 3];
fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|k| a[k] - b[k])
}
fn dot(a: Point, b: Point) -> f64 {
    (0..3).map(|k| a[k] * b[k]).sum()
}
fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn norm(a: Point) -> f64 {
    dot(a, a).sqrt()
}
fn tet_face(t: [usize; 4], k: usize) -> [usize; 3] {
    match k {
        0 => [t[1], t[2], t[3]],
        1 => [t[0], t[3], t[2]],
        2 => [t[0], t[1], t[3]],
        _ => [t[0], t[2], t[1]],
    }
}
pub fn bounds(points: &[Point]) -> (Point, Point) {
    let lo = std::array::from_fn(|k| points.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min));
    let hi = std::array::from_fn(|k| {
        points
            .iter()
            .map(|p| p[k])
            .fold(f64::NEG_INFINITY, f64::max)
    });
    (lo, hi)
}

pub fn min_dihedral(p: [Point; 4]) -> f64 {
    let mut normals = [[0.0; 3]; 4];
    for k in 0..4 {
        let [a, b, c] = tet_face([0, 1, 2, 3], k).map(|i| p[i]);
        let mut n = cross(sub(b, a), sub(c, a));
        if dot(n, sub(p[k], a)) > 0.0 {
            n = n.map(|v| -v);
        }
        let len = norm(n);
        if len <= f64::MIN_POSITIVE {
            return 0.0;
        }
        normals[k] = n.map(|v| v / len);
    }
    let mut angle = 180.0f64;
    for a in 0..4 {
        for b in a + 1..4 {
            angle = angle.min(
                (-dot(normals[a], normals[b]))
                    .clamp(-1.0, 1.0)
                    .acos()
                    .to_degrees(),
            );
        }
    }
    angle
}

struct Face {
    vertices: [usize; 3],
    cells: [i64; 2],
}
pub struct Picture3D {
    pub mesh: Mesh3D,
    faces: Vec<Face>,
    centers: Vec<f64>,
    angles: Vec<f64>,
}
impl Picture3D {
    pub fn new(mesh: Mesh3D) -> Self {
        let mut faces = Vec::new();
        for (i, &t) in mesh.tetrahedra.iter().enumerate() {
            for k in 0..4 {
                let neighbor = mesh.neighbors[i][k];
                if neighbor < 0 || neighbor > i as i64 {
                    faces.push(Face {
                        vertices: tet_face(t, k),
                        cells: [i as i64, neighbor],
                    });
                }
            }
        }
        let centers = mesh
            .tetrahedra
            .iter()
            .map(|t| t.iter().map(|&n| mesh.nodes[n][0]).sum::<f64>() / 4.0)
            .collect();
        let angles = mesh
            .tetrahedra
            .iter()
            .map(|t| min_dihedral(t.map(|n| mesh.nodes[n])))
            .collect();
        Self {
            mesh,
            faces,
            centers,
            angles,
        }
    }
    /// Exposes the boundary of retained *whole tetrahedra*, including cut faces.
    fn visible_faces(&self, cut: f64) -> Vec<([usize; 3], usize)> {
        self.faces
            .iter()
            .filter_map(|f| {
                let a = f.cells[0] as usize;
                let keep_a = self.centers[a] <= cut;
                let keep_b = f.cells[1] >= 0 && self.centers[f.cells[1] as usize] <= cut;
                match (keep_a, keep_b) {
                    (true, false) => Some((f.vertices, a)),
                    (false, true) => Some((f.vertices, f.cells[1] as usize)),
                    _ => None,
                }
            })
            .collect()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ColorBy3D {
    Region,
    Angle,
}
pub struct View3D {
    yaw: f64,
    pitch: f64,
    zoom: f64,
    pan: Vec2,
    edges: bool,
    cut: f64,
    color: ColorBy3D,
}
impl Default for View3D {
    fn default() -> Self {
        Self {
            yaw: -0.65,
            pitch: 0.55,
            zoom: 1.0,
            pan: Vec2::ZERO,
            edges: true,
            cut: 100.0,
            color: ColorBy3D::Region,
        }
    }
}
impl View3D {
    pub fn reset(&mut self) {
        self.zoom = 1.0;
        self.pan = Vec2::ZERO;
    }
    fn rotate(&self, p: Point) -> Point {
        let (s, c) = self.yaw.sin_cos();
        let (v, w) = self.pitch.sin_cos();
        [
            c * p[0] - s * p[1],
            v * s * p[0] + v * c * p[1] + w * p[2],
            w * s * p[0] + w * c * p[1] - v * p[2],
        ]
    }
    pub fn toolbar(&mut self, ui: &mut egui::Ui, has_mesh: bool) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Fit all").clicked() {
                self.reset();
            }
            ui.menu_button("Camera", |ui| {
                for (label, yaw, pitch) in [
                    ("Isometric", -0.65, 0.55),
                    ("Top", 0.0, std::f64::consts::FRAC_PI_2),
                    ("Front", 0.0, 0.0),
                    ("Side", std::f64::consts::FRAC_PI_2, 0.0),
                ] {
                    if ui.button(label).clicked() {
                        self.yaw = yaw;
                        self.pitch = pitch;
                        self.reset();
                        ui.close();
                    }
                }
            });
            ui.checkbox(&mut self.edges, "Edges");
            ui.add_enabled_ui(has_mesh, |ui| {
                egui::ComboBox::from_id_salt("color3d")
                    .width(105.0)
                    .selected_text(match self.color {
                        ColorBy3D::Region => "Regions",
                        ColorBy3D::Angle => "Dihedral angle",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.color, ColorBy3D::Region, "Regions");
                        ui.selectable_value(&mut self.color, ColorBy3D::Angle, "Dihedral angle");
                    });
                ui.label("Cut X");
                ui.add(egui::Slider::new(&mut self.cut, 5.0..=100.0).suffix("%"))
                    .on_hover_text("Retain whole cells by centroid along x. At 100% the entire volume is shown.");
            });
        });
        ui.add_space(4.0);
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        plc: &Plc3D,
        picture: Option<&Picture3D>,
        units: &str,
    ) {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;
        let visuals = ui.visuals();
        painter.rect_filled(rect, 0, visuals.extreme_bg_color);
        if response.double_clicked() {
            self.reset();
        }
        if response.dragged_by(egui::PointerButton::Primary) {
            let d = response.drag_delta();
            self.yaw -= d.x as f64 * 0.008;
            self.pitch = (self.pitch + d.y as f64 * 0.008).clamp(-1.5, 1.5);
        }
        if response.dragged_by(egui::PointerButton::Secondary) {
            self.pan += response.drag_delta();
        }
        if response.hovered() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            self.zoom =
                (self.zoom * (scroll as f64 * 0.002).exp() * pinch as f64).clamp(0.05, 30.0);
        }
        let nodes = picture
            .map(|p| p.mesh.nodes.as_slice())
            .unwrap_or(&plc.vertices);
        if nodes.is_empty() {
            return;
        }
        let (lo, hi) = bounds(nodes);
        let center: Point = std::array::from_fn(|k| (lo[k] + hi[k]) * 0.5);
        let radius = norm(sub(hi, lo)).max(1e-10) * 0.5;
        let scale = 0.39 * rect.width().min(rect.height()) as f64 / radius * self.zoom;
        let project = |p: Point| {
            let r = self.rotate(sub(p, center));
            (
                rect.center() + self.pan + Vec2::new((r[0] * scale) as f32, (-r[1] * scale) as f32),
                r[2],
            )
        };
        let projected: Vec<_> = nodes.iter().map(|&p| project(p)).collect();
        // A quiet ground grid provides orientation without competing with cells.
        let grid = Stroke::new(
            0.5,
            if visuals.dark_mode {
                Color32::from_white_alpha(14)
            } else {
                Color32::from_black_alpha(12)
            },
        );
        for k in 0..=8 {
            let t = k as f64 / 8.0;
            let x = lo[0] + (hi[0] - lo[0]) * t;
            let y = lo[1] + (hi[1] - lo[1]) * t;
            painter.line_segment(
                [project([x, lo[1], lo[2]]).0, project([x, hi[1], lo[2]]).0],
                grid,
            );
            painter.line_segment(
                [project([lo[0], y, lo[2]]).0, project([hi[0], y, lo[2]]).0],
                grid,
            );
        }
        let faces: Vec<_> = if let Some(p) = picture {
            let cut = if self.cut >= 100.0 {
                f64::INFINITY
            } else {
                lo[0] + (hi[0] - lo[0]) * self.cut / 100.0
            };
            p.visible_faces(cut)
                .into_iter()
                .map(|(f, cell)| {
                    let color = match self.color {
                        ColorBy3D::Region => {
                            crate::view::zone_color(p.mesh.cell_markers[cell], visuals.dark_mode)
                        }
                        ColorBy3D::Angle => angle_color(p.angles[cell]),
                    };
                    (f, color)
                })
                .collect()
        } else {
            plc.facets
                .iter()
                .enumerate()
                .map(|(i, &f)| {
                    (
                        f,
                        crate::view::zone_color(
                            plc.facet_markers.get(i).copied().unwrap_or(1),
                            visuals.dark_mode,
                        ),
                    )
                })
                .collect()
        };
        let mut faces: Vec<_> = faces
            .into_iter()
            .map(|(f, color)| {
                (
                    f.iter().map(|&i| projected[i].1).sum::<f64>() / 3.0,
                    f,
                    color,
                )
            })
            .collect();
        faces.sort_by(|a, b| b.0.total_cmp(&a.0));
        let wire = if visuals.dark_mode {
            Color32::from_white_alpha(70)
        } else {
            Color32::from_black_alpha(75)
        };
        for (_, f, color) in faces {
            let [a, b, c] = f.map(|i| nodes[i]);
            let n = cross(sub(b, a), sub(c, a));
            let normal = self.rotate(n);
            let light = (0.68
                + 0.32 * (normal[0] * 0.2 + normal[1] * 0.4 - normal[2] * 0.9).abs()
                    / norm(n).max(1e-30))
            .clamp(0.65, 1.0);
            let fill = Color32::from_rgb(
                (color.r() as f64 * light) as u8,
                (color.g() as f64 * light) as u8,
                (color.b() as f64 * light) as u8,
            );
            let points = f.map(|i| projected[i].0);
            if !Rect::from_points(&points).intersects(rect) {
                continue;
            }
            // A projected triangle can be nearly collinear. Polygon stroke
            // joins produce long miter spikes there; explicit triangles and
            // independent edge segments keep the preview within its geometry.
            let mut triangle = egui::Mesh::default();
            for point in points {
                triangle.colored_vertex(point, fill);
            }
            triangle.add_triangle(0, 1, 2);
            painter.add(egui::Shape::mesh(triangle));
            if self.edges {
                for k in 0..3 {
                    painter.line_segment([points[k], points[(k + 1) % 3]], Stroke::new(0.65, wire));
                }
            }
        }
        if picture.is_none() && plc.facets.is_empty() {
            for &(point, _) in &projected {
                painter.circle_filled(point, 2.5, crate::theme::ACCENT);
            }
        }
        let label = if let Some(p) = picture {
            if self.color == ColorBy3D::Angle {
                "Minimum dihedral: red 0° · green 20° · blue 40°+".into()
            } else {
                let regions: std::collections::BTreeSet<_> =
                    p.mesh.cell_markers.iter().copied().collect();
                format!(
                    "Regions: {}",
                    regions
                        .iter()
                        .map(|m| m.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        } else {
            "Geometry · boundary markers".into()
        };
        let text =
            painter.layout_no_wrap(label, FontId::proportional(11.5), visuals.weak_text_color());
        let at = rect.left_top() + Vec2::splat(10.0);
        painter.rect_filled(
            Rect::from_min_size(at, text.size()).expand(5.0),
            5,
            visuals.window_fill,
        );
        painter.galley(at, text, visuals.text_color());
        painter.text(
            rect.left_bottom() + Vec2::new(10.0, -10.0),
            Align2::LEFT_BOTTOM,
            format!(
                "{:.3} × {:.3} × {:.3} {units}",
                hi[0] - lo[0],
                hi[1] - lo[1],
                hi[2] - lo[2]
            ),
            FontId::proportional(11.0),
            visuals.weak_text_color(),
        );
        let origin = rect.right_bottom() - Vec2::new(52.0, 52.0);
        for (axis, label, color) in [
            ([1.0, 0.0, 0.0], "x", Color32::from_rgb(225, 85, 85)),
            ([0.0, 1.0, 0.0], "y", Color32::from_rgb(55, 165, 105)),
            ([0.0, 0.0, 1.0], "z", crate::theme::ACCENT),
        ] {
            let p = self.rotate(axis);
            let end = origin + Vec2::new(p[0] as f32, -p[1] as f32) * 30.0;
            painter.line_segment([origin, end], Stroke::new(2.0, color));
            painter.text(
                end,
                Align2::CENTER_CENTER,
                label,
                FontId::proportional(12.0),
                color,
            );
        }
    }
}
fn angle_color(angle: f64) -> Color32 {
    let t = (angle / 40.0).clamp(0.0, 1.0);
    let (a, b, t) = if t < 0.5 {
        ([220.0, 65.0, 60.0], [80.0, 185.0, 130.0], t * 2.0)
    } else {
        ([80.0, 185.0, 130.0], [80.0, 145.0, 230.0], (t - 0.5) * 2.0)
    };
    let c: [u8; 3] = std::array::from_fn(|k| (a[k] + (b[k] - a[k]) * t) as u8);
    Color32::from_rgb(c[0], c[1], c[2])
}
