//! The mesh builder window: controls on the left, the drawing on the right.
//! The same code runs on the desktop and in the browser; only meshing
//! ([`crate::jobs`]) and files ([`crate::files`]) differ.
//!
//! The controls describe a general 2D geometry (domain, zones and holes,
//! lines, points, mesh size); the templates, including the geophysical
//! surveys, only fill it in.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use eframe::egui::{self, AtomExt, RichText};
use rustmesh::examples::{Example, Field};
use rustmesh::model::{ellipse, points_along, rectangle};
use rustmesh::{export, Domain, Geometry2D, Line, Shape, Zone};

use crate::edit::{
    outline_to_text, parse_outline, parse_points, parse_xy, points_to_text, xy_to_text,
};
use crate::files::{self, Inbox, Purpose};
use crate::jobs::{Job, Product, Request};
use crate::project::{self, Project};
use crate::view::{ColorBy, Fit, Picture, View};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Inspector {
    Templates,
    Geometry,
    Mesh,
}

const WEB: bool = cfg!(target_arch = "wasm32");
/// Node budget: the browser gets a smaller default; both can be changed.
const DEFAULT_MAX_NODES: usize = if WEB { 500_000 } else { 5_000_000 };

/// Editable text for every point list in the geometry.
#[derive(Default)]
struct Texts {
    domain: String,
    points: String,
    shapes: Vec<String>,
    lines: Vec<String>,
}

impl Texts {
    fn of(g: &Geometry2D) -> Self {
        Self {
            domain: outline_to_text(&g.domain.outline, &g.domain.edge_markers),
            points: points_to_text(&g.points),
            shapes: g.shapes.iter().map(|s| xy_to_text(&s.outline)).collect(),
            lines: g.lines.iter().map(|l| xy_to_text(&l.points)).collect(),
        }
    }
}

struct Running {
    job: Job,
    started: f64,
    geometry: Geometry2D,
}

struct Done {
    picture: Picture,
    seconds: f64,
    geometry: Geometry2D,
}

/// The template being set up and the parameters typed for each.
struct Templates {
    field: Field,
    example: Example,
    values: HashMap<Example, Vec<f64>>,
}

impl Templates {
    fn values(&mut self) -> &mut Vec<f64> {
        let e = self.example;
        self.values.entry(e).or_insert_with(|| e.defaults())
    }
}

/// A row of evenly spaced points to add.
struct PointRow {
    from: [f64; 2],
    to: [f64; 2],
    count: usize,
    marker: i32,
}

pub struct MeshApp {
    geometry: Geometry2D,
    texts: Texts,
    /// Parse errors of the point editors, by editor.
    errors: HashMap<String, String>,
    templates: Templates,
    view: View,
    max_nodes: usize,
    running: Option<Running>,
    done: Option<Done>,
    failure: Option<String>,
    notice: Option<(bool, String)>,
    inbox: Inbox,
    row: PointRow,
    rect: [f64; 4],
    inspector: Inspector,
    mode3d: bool,
    studio3d: crate::studio3d::Studio3D,
    #[cfg(not(target_arch = "wasm32"))]
    desktop: Option<crate::desktop::DesktopSession>,
    #[cfg(not(target_arch = "wasm32"))]
    displayed_mesh: Option<String>,
}

impl MeshApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self::with_context(&cc.egui_ctx);
        #[cfg(not(target_arch = "wasm32"))]
        match crate::desktop::DesktopSession::new(&app.geometry) {
            Ok(session) => {
                app.desktop = Some(session);
                app.sync_workspace();
            }
            Err(e) => app.notice = Some((true, format!("Workspace unavailable: {e}"))),
        }
        app
    }

    fn with_context(ctx: &egui::Context) -> Self {
        crate::theme::install(ctx);
        let example = Example::PlateWithHole;
        let geometry = example.geometry();
        let (lo, hi) = geometry.bounds();
        Self {
            texts: Texts::of(&geometry),
            geometry,
            errors: HashMap::new(),
            templates: Templates {
                field: example.field(),
                example,
                values: HashMap::new(),
            },
            view: View::default(),
            max_nodes: DEFAULT_MAX_NODES,
            running: None,
            done: None,
            failure: None,
            notice: None,
            inbox: Rc::new(RefCell::new(Vec::new())),
            row: PointRow {
                from: [lo[0], hi[1]],
                to: [hi[0], hi[1]],
                count: 11,
                marker: 0,
            },
            rect: [lo[0], lo[1], hi[0], hi[1]],
            inspector: Inspector::Templates,
            mode3d: false,
            studio3d: crate::studio3d::Studio3D::default(),
            #[cfg(not(target_arch = "wasm32"))]
            desktop: None,
            #[cfg(not(target_arch = "wasm32"))]
            displayed_mesh: None,
        }
    }

    fn is_running(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        if self.desktop.as_ref().is_some_and(|s| s.is_running()) {
            return true;
        }
        self.running.is_some()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn sync_workspace(&mut self) {
        let Some(mut session) = self.desktop.take() else {
            return;
        };
        session.poll();
        if let Some(snapshot) = session.snapshot() {
            let identity = (snapshot.project_id.clone(), snapshot.revision);
            if session.revision.as_ref() != Some(&identity) {
                self.max_nodes = snapshot.max_nodes;
                self.load(snapshot.geometry, Fit::Details);
                self.displayed_mesh = None;
                session.revision = Some(identity);
            }
        }
        if self.displayed_mesh != session.selected_mesh {
            if let Some(record) = session.selected() {
                // The view's mesh stays bound to its original input even after undo.
                let geometry = session
                    .mesh_geometry(&record)
                    .expect("stored meshes retain their project revision");
                self.done = Some(Done {
                    picture: Picture::new(record.mesh),
                    seconds: record.report["seconds"].as_f64().unwrap_or(0.0),
                    geometry,
                });
                self.displayed_mesh = Some(record.mesh_id);
            }
        }
        self.desktop = Some(session);
    }

    /// Shows `geometry`, fitted to `fit`.
    fn load(&mut self, geometry: Geometry2D, fit: Fit) {
        let (lo, hi) = geometry.bounds();
        self.rect = [lo[0], lo[1], hi[0], hi[1]];
        self.texts = Texts::of(&geometry);
        self.geometry = geometry;
        self.errors.clear();
        self.done = None;
        self.failure = None;
        self.view.fit(fit);
    }

    fn file_stem(&self) -> String {
        let stem: String = self
            .geometry
            .name
            .trim()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let stem = stem
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        if stem.is_empty() {
            "geometry".into()
        } else {
            stem
        }
    }

    fn take_inbox(&mut self) {
        let picked: Vec<_> = self.inbox.borrow_mut().drain(..).collect();
        for p in picked {
            let text = match p.text {
                Ok(text) => text,
                Err(e) => {
                    if self.mode3d {
                        self.studio3d.file_error(e.clone());
                    }
                    self.notice = Some((true, e));
                    continue;
                }
            };
            if p.purpose == Purpose::Project && crate::studio3d::Project3D::recognizes(&text) {
                self.mode3d = true;
                self.studio3d.open(&text);
                continue;
            }
            let result = match p.purpose {
                Purpose::Project => Project::from_json(&text).map(|project| {
                    self.mode3d = false;
                    self.max_nodes = project.max_nodes;
                    self.load(project.geometry, Fit::Details);
                }),
                Purpose::Domain => parse_outline(&text).map(|(outline, edge_markers)| {
                    self.geometry.domain = Domain {
                        outline,
                        edge_markers,
                    };
                    self.texts.domain = outline_to_text(
                        &self.geometry.domain.outline,
                        &self.geometry.domain.edge_markers,
                    );
                }),
                Purpose::Points => parse_points(&text).map(|points| {
                    self.geometry.points = points;
                    self.texts.points = points_to_text(&self.geometry.points);
                }),
            };
            if self.mode3d {
                if let Err(e) = &result {
                    self.studio3d.file_error(e.clone());
                }
            }
            self.notice = Some(match result {
                Ok(()) => (false, format!("Opened {}", p.name)),
                Err(e) => (true, format!("{}: {e}", p.name)),
            });
        }
    }

    fn build(&mut self, ctx: &egui::Context) {
        self.failure = None;
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(session) = &mut self.desktop {
            let changed = session
                .snapshot()
                .is_none_or(|p| p.geometry != self.geometry || p.max_nodes != self.max_nodes);
            let result = if changed {
                session.commit(self.geometry.clone(), self.max_nodes)
            } else {
                Ok(())
            };
            if let Err(e) = result.and_then(|_| session.build()) {
                self.failure = Some(e);
            }
            return;
        }
        if let Err(e) = self.geometry.validate() {
            self.failure = Some(e.to_string());
            return;
        }
        let request = Request::TwoD {
            geometry: self.geometry.clone(),
            max_nodes: self.max_nodes,
        };
        match Job::start(ctx, request) {
            Ok(job) => {
                self.running = Some(Running {
                    job,
                    started: ctx.input(|i| i.time),
                    geometry: self.geometry.clone(),
                })
            }
            Err(e) => self.failure = Some(e),
        }
    }

    fn poll(&mut self) {
        let Some(outcome) = self.running.as_mut().and_then(|r| r.job.poll()) else {
            return;
        };
        let running = self.running.take().expect("polled a running job");
        match outcome.mesh {
            Ok(Product::TwoD(mesh)) => {
                self.done = Some(Done {
                    picture: Picture::new(mesh),
                    seconds: outcome.seconds,
                    geometry: running.geometry,
                })
            }
            Ok(Product::ThreeD(_)) => {
                self.failure = Some("Unexpected 3D result for a 2D job".into())
            }
            Err(e) => self.failure = Some(e),
        }
    }

    fn save_project(&mut self) {
        if self.mode3d {
            self.studio3d.save();
            return;
        }
        let json = Project::new(self.geometry.clone(), self.max_nodes).to_json();
        let name = format!("{}.{}", self.file_stem(), project::EXTENSION);
        self.saved(files::save(&name, files::PROJECT, |w| {
            w.write_all(json.as_bytes())
        }));
    }

    fn export(&mut self, gmsh: bool) {
        if self.mode3d {
            self.studio3d.export(gmsh);
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(session) = &mut self.desktop {
            let format = if gmsh { "msh" } else { "vtu" };
            let bytes = match session.export_for_dialog(format) {
                Ok(bytes) => bytes,
                Err(e) => {
                    self.notice = Some((true, e));
                    return;
                }
            };
            let name = format!("{}.{}", self.file_stem(), format);
            let filter = if gmsh {
                ("Gmsh mesh", &["msh"][..])
            } else {
                ("VTK unstructured grid", &["vtu"][..])
            };
            self.saved(files::save(&name, filter, |w| w.write_all(&bytes)));
            return;
        }
        let Some(done) = &self.done else { return };
        let mesh = &done.picture.mesh;
        let stem = self.file_stem();
        let result = if gmsh {
            files::save(&format!("{stem}.msh"), ("Gmsh mesh", &["msh"]), |w| {
                export::write_gmsh(mesh, w)
            })
        } else {
            files::save(
                &format!("{stem}.vtu"),
                ("VTK unstructured grid", &["vtu"]),
                |w| export::write_vtu(mesh, w),
            )
        };
        self.saved(result);
    }

    fn saved(&mut self, result: Result<Option<String>, String>) {
        self.notice = match result {
            Ok(Some(place)) => Some((false, format!("Saved {place}"))),
            Ok(None) => None,
            Err(e) => Some((true, e)),
        };
    }

    fn text_box(ui: &mut egui::Ui, id: &str, text: &mut String) -> bool {
        egui::ScrollArea::vertical()
            .id_salt(id)
            .max_height(110.0)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(text)
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                )
                .changed()
            })
            .inner
    }

    /// A text editor whose text `parse` turns into a value when it changed
    /// and parses; the parse error is shown under it otherwise.
    fn editor<T>(
        ui: &mut egui::Ui,
        errors: &mut HashMap<String, String>,
        id: &str,
        text: &mut String,
        parse: impl Fn(&str) -> Result<T, String>,
    ) -> Option<T> {
        let mut out = None;
        if Self::text_box(ui, id, text) {
            match parse(text) {
                Ok(value) => {
                    errors.remove(id);
                    out = Some(value);
                }
                Err(e) => {
                    errors.insert(id.to_owned(), e);
                }
            }
        }
        if let Some(e) = errors.get(id) {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        out
    }

    fn zone_row(ui: &mut egui::Ui, zone: &mut Zone) {
        // Sized explicitly: a grid column otherwise keeps the text box narrow.
        ui.add_sized([130.0, 20.0], egui::TextEdit::singleline(&mut zone.name));
        ui.add(egui::DragValue::new(&mut zone.marker).speed(0.1));
        ui.add(
            egui::DragValue::new(&mut zone.max_area)
                .speed(0.01)
                .range(0.0..=f64::INFINITY),
        );
        ui.end_row();
    }

    fn next_marker(&self) -> i32 {
        self.geometry
            .zones()
            .iter()
            .map(|z| z.marker)
            .max()
            .unwrap_or(0)
            + 1
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
            let p = ui.painter();
            p.rect_filled(rect, 9, crate::theme::ACCENT);
            let a = rect.min + egui::vec2(8.0, 25.0);
            let b = rect.min + egui::vec2(17.0, 7.0);
            let c = rect.min + egui::vec2(27.0, 25.0);
            let m = rect.min + egui::vec2(17.0, 19.0);
            for [x, y] in [[a, b], [b, c], [c, a], [a, m], [b, m], [c, m]] {
                p.line_segment([x, y], egui::Stroke::new(1.2, egui::Color32::WHITE));
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(RichText::new("rustmesh").strong().size(18.0));
                ui.label(RichText::new("Mesh Studio").small().weak());
            });
            ui.add_space(12.0);
            ui.selectable_value(&mut self.mode3d, false, "2D");
            ui.selectable_value(&mut self.mode3d, true, "3D");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button("Appearance", |ui| {
                    egui::widgets::global_theme_preference_buttons(ui);
                });
                ui.add_enabled_ui(
                    if self.mode3d {
                        self.studio3d.has_mesh()
                    } else {
                        self.done.is_some()
                    },
                    |ui| {
                        ui.menu_button("Export", |ui| {
                            if ui.button("VTK mesh (.vtu)").clicked() {
                                self.export(false);
                                ui.close();
                            }
                            if ui.button("Gmsh mesh (.msh)").clicked() {
                                self.export(true);
                                ui.close();
                            }
                        });
                    },
                );
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(session) = &mut self.desktop {
                    if ui.selectable_label(session.open, "Agent").clicked() {
                        session.open = !session.open;
                    }
                }
                if ui.button("Save").on_hover_text("Save project").clicked() {
                    self.save_project();
                }
                if ui.button("Open").on_hover_text("Open project").clicked() {
                    files::open(ui.ctx(), &self.inbox, Purpose::Project, files::PROJECT);
                }
            });
        });
    }

    fn template_section(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.horizontal_wrapped(|ui| {
                for field in Field::ALL {
                    if ui
                        .selectable_label(self.templates.field == field, field.title())
                        .clicked()
                    {
                        self.templates.field = field;
                        if self.templates.example.field() != field {
                            if let Some(e) = Example::ALL.into_iter().find(|e| e.field() == field) {
                                self.templates.example = e;
                            }
                        }
                    }
                }
            });
            let field = self.templates.field;
            egui::ComboBox::from_id_salt("template")
                .selected_text(self.templates.example.title())
                .width(ui.available_width() - 8.0)
                .show_ui(ui, |ui| {
                    for e in Example::ALL.into_iter().filter(|e| e.field() == field) {
                        ui.selectable_value(&mut self.templates.example, e, e.title());
                    }
                });
            ui.label(RichText::new(self.templates.example.description()).small());
            let params = self.templates.example.params();
            let example = self.templates.example;
            let values = self.templates.values();
            egui::Grid::new(("params", example.key()))
                .num_columns(2)
                .show(ui, |ui| {
                    for (p, v) in params.iter().zip(values.iter_mut()) {
                        ui.label(p.label);
                        let speed = if p.whole {
                            1.0
                        } else {
                            (v.abs() * 0.01).max(1e-3)
                        };
                        let mut drag = egui::DragValue::new(v).speed(speed).range(p.min..=p.max);
                        if p.whole {
                            drag = drag.fixed_decimals(0);
                        }
                        ui.add(drag);
                        ui.end_row();
                    }
                });
            ui.horizontal(|ui| {
                if ui
                    .button("Use template")
                    .on_hover_text("Replaces the current geometry")
                    .clicked()
                {
                    let values = self.templates.values().clone();
                    // General geometries are seen whole; surveys around their sensors.
                    let fit = if example.field() == Field::General || example == Example::Watershed
                    {
                        Fit::All
                    } else {
                        Fit::Details
                    };
                    self.load(example.build(&values), fit);
                    self.inspector = Inspector::Geometry;
                }
                if ui.small_button("Defaults").clicked() {
                    *self.templates.values() = example.defaults();
                }
            });
        });
    }

    fn domain_section(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::CollapsingHeader::new("Domain").default_open(true).show(ui, |ui| {
            egui::Grid::new("names").num_columns(2).show(ui, |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut self.geometry.name);
                ui.end_row();
                ui.label("Length unit").on_hover_text("For display only; the mesher works in any unit");
                ui.add(egui::TextEdit::singleline(&mut self.geometry.units).desired_width(60.0));
                ui.end_row();
            });
            ui.label(RichText::new("Corners in order: x, y, marker of the edge to the next corner.").small());
            if let Some((outline, edge_markers)) =
                Self::editor(ui, &mut self.errors, "domain", &mut self.texts.domain, parse_outline)
            {
                self.geometry.domain = Domain { outline, edge_markers };
            }
            ui.horizontal(|ui| {
                if ui.small_button("Import table…").on_hover_text("x, y[, edge marker] per line").clicked() {
                    files::open(ctx, &self.inbox, Purpose::Domain, files::TABLE);
                }
                if ui
                    .small_button("Make rectangle")
                    .on_hover_text("From the corners below; edges marked 1 (bottom), 2, 3, 4 counter-clockwise")
                    .clicked()
                {
                    let [x0, y0, x1, y1] = self.rect;
                    self.geometry.domain = Domain::rectangle(x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1));
                    self.texts.domain = outline_to_text(&self.geometry.domain.outline, &self.geometry.domain.edge_markers);
                    self.errors.remove("domain");
                }
            });
            ui.horizontal_wrapped(|ui| {
                for (label, v) in ["x0", "y0", "x1", "y1"].iter().zip(self.rect.iter_mut()) {
                    ui.label(*label);
                    ui.add(egui::DragValue::new(v).speed(0.1));
                }
            });
        });
    }

    fn zones_section(&mut self, ui: &mut egui::Ui) {
        let units = self.geometry.units.clone();
        egui::CollapsingHeader::new("Zones and holes").default_open(true).show(ui, |ui| {
            ui.label(
                RichText::new("Each zone's cells get its marker. Where shapes overlap, the later one wins. Largest cell 0: no limit.")
                    .small(),
            );
            egui::Grid::new("background").num_columns(3).striped(true).show(ui, |ui| {
                ui.label(RichText::new("Background").strong());
                ui.label(RichText::new("Marker").strong());
                ui.label(RichText::new(format!("Area ({units}²)")).strong());
                ui.end_row();
                Self::zone_row(ui, &mut self.geometry.background);
            });
            let mut remove = None;
            for (i, shape) in self.geometry.shapes.iter_mut().enumerate() {
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!("Shape {}", i + 1)).strong());
                    ui.checkbox(&mut shape.hole, "Hole");
                    ui.label("edge marker").on_hover_text("Marker of this shape's edges");
                    ui.add(egui::DragValue::new(&mut shape.edge_marker).speed(0.1));
                    if ui.small_button("Remove").clicked() {
                        remove = Some(i);
                    }
                });
                if shape.hole {
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        ui.add_sized([130.0, 20.0], egui::TextEdit::singleline(&mut shape.zone.name));
                    });
                } else {
                    egui::Grid::new(("shape", i)).num_columns(3).show(ui, |ui| Self::zone_row(ui, &mut shape.zone));
                }
                ui.label(RichText::new("Corners, x, y per line:").small());
                if let Some(p) = Self::editor(ui, &mut self.errors, &format!("shape{i}"), &mut self.texts.shapes[i], parse_xy) {
                    shape.outline = p;
                }
            }
            if let Some(i) = remove {
                self.geometry.shapes.remove(i);
                self.texts.shapes.remove(i);
                self.errors.clear();
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Add");
                let (lo, hi) = self.geometry.bounds();
                let (c, w, h) = ([0.5 * (lo[0] + hi[0]), 0.5 * (lo[1] + hi[1])], 0.1 * (hi[0] - lo[0]), 0.1 * (hi[1] - lo[1]));
                let r = w.min(h);
                let new = [
                    ("rectangle", rectangle(c[0] - w, c[1] - h, c[0] + w, c[1] + h)),
                    ("circle", ellipse(c, r, r, 32)),
                    ("polygon", vec![[c[0] - w, c[1] - h], [c[0] + w, c[1] - h], [c[0], c[1] + h]]),
                ];
                for (label, outline) in new {
                    if ui.button(label).clicked() {
                        let n = self.geometry.shapes.len() + 1;
                        let zone = Zone::new(&format!("Zone {n}"), self.next_marker(), 0.0);
                        self.texts.shapes.push(xy_to_text(&outline));
                        self.geometry.shapes.push(Shape { zone, outline, edge_marker: 0, hole: false });
                    }
                }
            });
        });
    }

    fn lines_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(format!("Lines ({})", self.geometry.lines.len())).show(ui, |ui| {
            ui.label(RichText::new("The mesh follows them without them changing any zone; their edges get the marker.").small());
            let mut remove = None;
            for (i, line) in self.geometry.lines.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut line.name).desired_width(120.0));
                    ui.label("marker");
                    ui.add(egui::DragValue::new(&mut line.marker).speed(0.1));
                    if ui.small_button("Remove").clicked() {
                        remove = Some(i);
                    }
                });
                if let Some(p) = Self::editor(ui, &mut self.errors, &format!("line{i}"), &mut self.texts.lines[i], parse_xy) {
                    line.points = p;
                }
            }
            if let Some(i) = remove {
                self.geometry.lines.remove(i);
                self.texts.lines.remove(i);
                self.errors.clear();
            }
            if ui.button("Add line").clicked() {
                let (lo, hi) = self.geometry.bounds();
                let y = 0.5 * (lo[1] + hi[1]);
                let points = vec![[lo[0] + 0.3 * (hi[0] - lo[0]), y], [lo[0] + 0.7 * (hi[0] - lo[0]), y]];
                self.texts.lines.push(xy_to_text(&points));
                self.geometry.lines.push(Line { name: format!("Line {}", self.geometry.lines.len() + 1), points, marker: 0 });
            }
        });
    }

    fn points_section(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::CollapsingHeader::new(format!("Points ({})", self.geometry.points.len())).show(ui, |ui| {
            ui.label(
                RichText::new("Nodes the mesh must have (sensors, sources, load points), with cells refined around them.")
                    .small(),
            );
            egui::Grid::new("row").num_columns(5).show(ui, |ui| {
                ui.label("From");
                ui.add(egui::DragValue::new(&mut self.row.from[0]).speed(0.1).prefix("x "));
                ui.add(egui::DragValue::new(&mut self.row.from[1]).speed(0.1).prefix("y "));
                ui.end_row();
                ui.label("To");
                ui.add(egui::DragValue::new(&mut self.row.to[0]).speed(0.1).prefix("x "));
                ui.add(egui::DragValue::new(&mut self.row.to[1]).speed(0.1).prefix("y "));
                ui.end_row();
                ui.label("Count");
                ui.add(egui::DragValue::new(&mut self.row.count).range(1..=100_000));
                ui.add(egui::DragValue::new(&mut self.row.marker).speed(0.1).prefix("marker "))
                    .on_hover_text("pyGIMLi reads marker -99 as a sensor");
                ui.end_row();
            });
            ui.horizontal_wrapped(|ui| {
                if ui.button("Add points").clicked() {
                    let r = &self.row;
                    self.geometry.points.extend(points_along(r.from, r.to, r.count, r.marker));
                    self.texts.points = points_to_text(&self.geometry.points);
                }
                if ui.button("Import table…").on_hover_text("x, y[, marker[, 0 for no refinement]] per line").clicked() {
                    files::open(ctx, &self.inbox, Purpose::Points, files::TABLE);
                }
                if ui.button("Remove all").clicked() {
                    self.geometry.points.clear();
                    self.texts.points.clear();
                }
            });
            ui.label(RichText::new("x, y, marker per line; a fourth column 0 means no refinement:").small());
            if let Some(points) = Self::editor(ui, &mut self.errors, "points", &mut self.texts.points, parse_points) {
                self.geometry.points = points;
            }
        });
    }

    fn sizing_section(&mut self, ui: &mut egui::Ui) {
        let units = self.geometry.units.clone();
        egui::CollapsingHeader::new("Mesh size")
            .default_open(true)
            .show(ui, |ui| {
                let s = &mut self.geometry.sizing;
                egui::Grid::new("sizing").num_columns(2).show(ui, |ui| {
                    ui.label("Minimum angle (°)");
                    ui.add(egui::Slider::new(&mut s.min_angle, 0.0..=34.0));
                    ui.end_row();
                    ui.label(format!("Max. area ({units}²)"))
                        .on_hover_text("0: no limit");
                    ui.add(
                        egui::DragValue::new(&mut s.max_area)
                            .speed(0.01)
                            .range(0.0..=f64::INFINITY),
                    );
                    ui.end_row();
                    ui.label(format!("Point size ({units})"))
                        .on_hover_text("0: no refinement around points");
                    ui.add(
                        egui::DragValue::new(&mut s.point_size)
                            .speed(0.01)
                            .range(0.0..=f64::INFINITY),
                    );
                    ui.end_row();
                    ui.label("Size growth")
                        .on_hover_text("How much the cell size grows per unit of distance");
                    ui.add(
                        egui::DragValue::new(&mut s.size_growth)
                            .speed(0.01)
                            .range(0.0..=10.0),
                    );
                    ui.end_row();
                    ui.label("Node limit")
                        .on_hover_text("Meshing stops here and says so");
                    ui.add(
                        egui::DragValue::new(&mut self.max_nodes)
                            .speed(1000.0)
                            .range(100..=usize::MAX),
                    );
                    ui.end_row();
                });
                if WEB && self.max_nodes > DEFAULT_MAX_NODES {
                    ui.label(
                        RichText::new(
                            "Large meshes can be slow or run out of memory in a browser.",
                        )
                        .small(),
                    );
                }
            });
    }

    fn build_controls(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add_space(6.0);
        // Build stays where it is (disabled while meshing) and Stop sits
        // below it, so a late click on Stop cannot start another mesh.
        let build = egui::Button::new(
            RichText::new("Build mesh")
                .strong()
                .color(egui::Color32::WHITE)
                .atom_grow(true),
        )
        .fill(crate::theme::ACCENT)
        .min_size(egui::vec2(ui.available_width(), 40.0));
        if ui
            .add_enabled(!self.is_running() && self.errors.is_empty(), build)
            .clicked()
        {
            self.build(ctx);
            self.inspector = Inspector::Mesh;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(session) = &mut self.desktop {
            if session.is_running() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Meshing in local worker…");
                    if ui.button("Stop").clicked() {
                        session.stop_manual();
                    }
                });
            }
        }
        if let Some(running) = &self.running {
            let elapsed = ctx.input(|i| i.time) - running.started;
            let tip = if Job::STOP_ENDS_WORK {
                "Stops meshing"
            } else {
                "The desktop app finishes the computation in the background and drops the result"
            };
            let mut stop = false;
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Meshing… {elapsed:.1} s"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    stop = ui.button("Stop").on_hover_text(tip).clicked();
                });
            });
            if stop {
                if let Some(r) = self.running.take() {
                    r.job.stop();
                }
            }
        }
        if !self.errors.is_empty() {
            ui.colored_label(
                ui.visuals().error_fg_color,
                "Fix the highlighted geometry entries to build.",
            );
        } else if !self.is_running() {
            ui.label(
                RichText::new("Computed locally on your device")
                    .small()
                    .weak(),
            );
        }
    }

    fn results_section(&mut self, ui: &mut egui::Ui) {
        if let Some(e) = &self.failure {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        if let Some(done) = &self.done {
            let mesh = &done.picture.mesh;
            let report = &mesh.report;
            ui.separator();
            ui.label(RichText::new("Mesh").strong());
            if done.geometry != self.geometry {
                ui.label(
                    RichText::new("The geometry changed since this mesh was built.").italics(),
                );
            }
            egui::Grid::new("result").num_columns(2).show(ui, |ui| {
                ui.label("Nodes");
                ui.label(mesh.nodes.len().to_string());
                ui.end_row();
                ui.label("Cells");
                ui.label(mesh.triangles.len().to_string());
                ui.end_row();
                ui.label("Smallest angle");
                ui.label(format!("{:.1}°", done.picture.smallest_angle()));
                ui.end_row();
                let target = done.geometry.sizing.min_angle;
                ui.label(format!("Cells under {target:.0}°"));
                ui.label(done.picture.cells_below(target).to_string());
                ui.end_row();
                ui.label("Time");
                ui.label(format!("{:.2} s", done.seconds));
                ui.end_row();
            });
            if report.complete {
                ui.label("Every requested size and angle limit is met.");
            }
            for issue in &report.issues {
                ui.colored_label(ui.visuals().warn_fg_color, format!("• {issue}"));
            }
            for warning in &report.warnings {
                ui.label(RichText::new(format!("• {warning}")).small());
            }
            ui.horizontal(|ui| {
                if ui
                    .button("Save .vtu")
                    .on_hover_text("VTK unstructured grid, for ParaView")
                    .clicked()
                {
                    self.export(false);
                }
                if ui
                    .button("Save .msh")
                    .on_hover_text("Gmsh 2.2, for Gmsh, meshio and pygimli.meshtools.readGmsh")
                    .clicked()
                {
                    self.export(true);
                }
            });
        }
        if let Some((error, text)) = &self.notice {
            let color = if *error {
                ui.visuals().error_fg_color
            } else {
                ui.visuals().weak_text_color()
            };
            ui.colored_label(color, text);
        }
        ui.add_space(12.0);
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.heading(&self.geometry.name);
            let status = if self.running.is_some() {
                "Building…"
            } else if !self.errors.is_empty() {
                "Invalid input"
            } else if let Some(d) = &self.done {
                if d.geometry != self.geometry {
                    "Needs rebuild"
                } else if !d.picture.mesh.report.complete {
                    "Review mesh report"
                } else {
                    "Mesh ready"
                }
            } else {
                "Geometry preview"
            };
            egui::Frame::new()
                .fill(ui.visuals().faint_bg_color)
                .corner_radius(20)
                .inner_margin(egui::Margin::symmetric(10, 5))
                .show(ui, |ui| {
                    ui.label(RichText::new(status).small().weak());
                });
        });
        if let Some(done) = &self.done {
            ui.label(
                RichText::new(format!(
                    "{} nodes   ·   {} cells   ·   {:.1}° minimum angle   ·   {:.2} s",
                    done.picture.mesh.nodes.len(),
                    done.picture.mesh.triangles.len(),
                    done.picture.smallest_angle(),
                    done.seconds
                ))
                .small()
                .weak(),
            );
        } else {
            ui.label(RichText::new("Shape your geometry. Build a quality mesh.").weak());
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Fit all").clicked() {
                self.view.fit(Fit::All);
            }
            if ui
                .button("Fit details")
                .on_hover_text("Points, lines and small shapes")
                .clicked()
            {
                self.view.fit(Fit::Details);
            }
            egui::ComboBox::from_id_salt("color_by")
                .width(125.0)
                .selected_text(self.view.color_by.label())
                .show_ui(ui, |ui| {
                    for c in ColorBy::ALL {
                        ui.selectable_value(&mut self.view.color_by, c, c.label());
                    }
                });
            ui.menu_button("View options", |ui| {
                ui.checkbox(&mut self.view.show_edges, "Cell edges");
                ui.checkbox(&mut self.view.show_geometry, "Geometry outlines");
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Vertical scale");
                    ui.add(
                        egui::DragValue::new(&mut self.view.exaggeration)
                            .speed(0.05)
                            .range(0.1..=100.0)
                            .suffix(" ×"),
                    );
                });
            });
        });
        ui.add_space(4.0);
        egui::Panel::bottom("canvas_hint")
            .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 8)))
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Scroll to zoom  ·  Drag to pan  ·  Double-click to fit")
                        .small()
                        .weak(),
                );
            });
        let picture = self.done.as_ref().map(|d| &d.picture);
        let target = self
            .done
            .as_ref()
            .map(|d| d.geometry.sizing.min_angle)
            .unwrap_or(self.geometry.sizing.min_angle);
        crate::theme::card(ui)
            .inner_margin(8)
            .show(ui, |ui| self.view.show(ui, &self.geometry, picture, target));
    }
}

impl MeshApp {
    fn workspace(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        #[cfg(not(target_arch = "wasm32"))]
        self.sync_workspace();
        #[cfg(not(target_arch = "wasm32"))]
        let before_edit = (self.geometry.clone(), self.max_nodes);
        self.take_inbox();
        self.poll();
        self.studio3d.poll();
        if self.running.is_some() || self.studio3d.is_running() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        egui::Panel::top("bar")
            .frame(
                egui::Frame::new()
                    .fill(ui.visuals().window_fill)
                    .inner_margin(egui::Margin::symmetric(20, 12)),
            )
            .show(ui, |ui| self.top_bar(ui));
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(session) = &mut self.desktop {
            session.show(ui);
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if self.mode3d {
            self.studio3d.show(ui);
            return;
        }
        egui::Panel::left("controls")
            .resizable(true)
            .default_size(380.0)
            .min_size(350.0)
            .frame(
                egui::Frame::new()
                    .fill(ui.visuals().panel_fill)
                    .inner_margin(16),
            )
            .show(ui, |ui| {
                egui::Panel::bottom("build_controls")
                    .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 12)))
                    .show(ui, |ui| self.build_controls(ui, &ctx));
                let width = (ui.available_width() - 8.0) / 3.0;
                egui::Frame::new()
                    .fill(ui.visuals().widgets.noninteractive.bg_stroke.color)
                    .corner_radius(9)
                    .inner_margin(3)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 1.0;
                        ui.horizontal(|ui| {
                            for (tab, label) in [
                                (Inspector::Templates, "Templates"),
                                (Inspector::Geometry, "Geometry"),
                                (Inspector::Mesh, "Mesh"),
                            ] {
                                let selected = self.inspector == tab;
                                let fill = if selected {
                                    ui.visuals().window_fill
                                } else {
                                    egui::Color32::TRANSPARENT
                                };
                                if ui
                                    .add_sized(
                                        [width, 30.0],
                                        egui::Button::new(
                                            RichText::new(label)
                                                .strong()
                                                .color(ui.visuals().text_color()),
                                        )
                                        .selected(selected)
                                        .fill(fill)
                                        .stroke(egui::Stroke::NONE),
                                    )
                                    .clicked()
                                {
                                    self.inspector = tab;
                                }
                            }
                        });
                    });
                ui.add_space(10.0);
                let (title, subtitle) = match self.inspector {
                    Inspector::Templates => (
                        "A place to start",
                        "Choose a template, then make it your own.",
                    ),
                    Inspector::Geometry => (
                        "Define your geometry",
                        "Edit the domain and the features inside it.",
                    ),
                    Inspector::Mesh => (
                        "Build with precision",
                        "Control resolution and review mesh quality.",
                    ),
                };
                ui.label(RichText::new(title).size(19.0).strong());
                ui.label(RichText::new(subtitle).small().weak());
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt(("inspector", self.inspector as u8))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        crate::theme::card(ui).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            match self.inspector {
                                Inspector::Templates => self.template_section(ui),
                                Inspector::Geometry => {
                                    self.domain_section(ui, &ctx);
                                    ui.separator();
                                    self.zones_section(ui);
                                    ui.separator();
                                    self.lines_section(ui);
                                    ui.separator();
                                    self.points_section(ui, &ctx);
                                }
                                Inspector::Mesh => {
                                    self.sizing_section(ui);
                                    self.results_section(ui);
                                }
                            }
                        });
                        if self.inspector != Inspector::Mesh {
                            if let Some((error, text)) = &self.notice {
                                let color = if *error {
                                    ui.visuals().error_fg_color
                                } else {
                                    ui.visuals().weak_text_color()
                                };
                                ui.colored_label(color, text);
                            }
                        }
                    });
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(ui.visuals().panel_fill)
                    .inner_margin(20),
            )
            .show(ui, |ui| self.canvas(ui));
        #[cfg(not(target_arch = "wasm32"))]
        if self.errors.is_empty()
            && (self.geometry != before_edit.0 || self.max_nodes != before_edit.1)
        {
            if let Some(session) = &mut self.desktop {
                if session
                    .snapshot()
                    .is_some_and(|p| p.geometry != self.geometry || p.max_nodes != self.max_nodes)
                {
                    if let Err(e) = session.commit(self.geometry.clone(), self.max_nodes) {
                        self.failure = Some(e);
                    }
                }
            }
        }
        if self.is_running() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }
}

impl eframe::App for MeshApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.workspace(ui);
    }
}
