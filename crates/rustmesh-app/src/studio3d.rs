//! The 3D workspace shares files, styling and background jobs with the 2D editor.
use crate::{
    files,
    jobs::{Job, Product, Request},
    theme,
    view3d::{Picture3D, View3D},
};
use eframe::egui::{self, AtomExt, RichText};
use rustmesh::{Options3D, Plc3D, Region3D};
use serde::{Deserialize, Serialize};

const FORMAT: &str = "rustmesh-project-3d";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project3D {
    pub format: String,
    pub version: u32,
    pub name: String,
    pub units: String,
    pub plc: Plc3D,
    pub options: Options3D,
}

impl Project3D {
    pub fn recognizes(text: &str) -> bool {
        serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .and_then(|v| {
                v.get("format")
                    .and_then(|s| s.as_str())
                    .map(|s| s == FORMAT)
            })
            .unwrap_or(false)
    }
    pub fn from_json(text: &str) -> Result<Self, String> {
        let p: Self = serde_json::from_str(text).map_err(|e| format!("Invalid 3D project: {e}"))?;
        if p.format != FORMAT || p.version != 1 {
            return Err("Unsupported 3D project format or version".into());
        }
        p.validate()?;
        Ok(p)
    }
    fn validate(&self) -> Result<(), String> {
        validate_geometry(&self.plc)?;
        let o = &self.options;
        if !o.max_volume.is_finite()
            || o.max_volume < 0.0
            || !o.min_dihedral.is_finite()
            || !(0.0..=90.0).contains(&o.min_dihedral)
            || o.max_passes == 0
            || o.max_vertices < self.plc.vertices.len()
        {
            return Err(
                "Invalid mesh settings: use finite nonnegative limits and a node budget that fits the geometry.".into(),
            );
        }
        Ok(())
    }
}

fn validate_geometry(plc: &Plc3D) -> Result<(), String> {
    plc.validate().map_err(|e| e.to_string())?;
    let (lo, hi) = crate::view3d::bounds(&plc.vertices);
    let extent = (0..3).map(|k| hi[k] - lo[k]).fold(0.0f64, f64::max);
    if !extent.is_finite() || !(1e-90..=1e90).contains(&extent) {
        return Err("Geometry extent must be finite and between 1e-90 and 1e90.".into());
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Template3D {
    Block,
    Layers,
    Sphere,
    Torus,
}
impl Template3D {
    const ALL: [Self; 4] = [Self::Block, Self::Layers, Self::Sphere, Self::Torus];
    fn title(self) -> &'static str {
        match self {
            Self::Block => "Block",
            Self::Layers => "Layered ground",
            Self::Sphere => "Sphere",
            Self::Torus => "Torus",
        }
    }
    fn description(self) -> &'static str {
        match self {
            Self::Block => "A solid volume with six marked boundary surfaces.",
            Self::Layers => {
                "Two material regions separated by a conforming horizontal interface, for hydrology and geophysics."
            }
            Self::Sphere => "A faceted spherical volume. Surface resolution controls the geometric approximation.",
            Self::Torus => {
                "A ring-shaped volume with a through-hole. Surface recovery can take longer for finer resolutions."
            }
        }
    }
    pub(crate) fn build(
        self,
        dimensions: [f64; 3],
        radius: f64,
        tube: f64,
        resolution: usize,
    ) -> Project3D {
        let mut p = Plc3D::default();
        match self {
            Self::Block | Self::Layers => {
                let [w, l, h] = dimensions;
                let levels = if self == Self::Layers {
                    vec![0.0, 0.45 * h, h]
                } else {
                    vec![0.0, h]
                };
                for &z in &levels {
                    p.vertices
                        .extend([[0.0, 0.0, z], [w, 0.0, z], [w, l, z], [0.0, l, z]]);
                }
                let top = (levels.len() - 1) * 4;
                p.facets.extend([
                    [0, 2, 1],
                    [0, 3, 2],
                    [top, top + 1, top + 2],
                    [top, top + 2, top + 3],
                ]);
                p.facet_markers.extend([1, 1, 2, 2]);
                for layer in 0..levels.len() - 1 {
                    for k in 0..4 {
                        let a = layer * 4 + k;
                        let b = layer * 4 + (k + 1) % 4;
                        p.facets.extend([[a, b, b + 4], [a, b + 4, a + 4]]);
                        p.facet_markers.extend([3 + k as i32, 3 + k as i32]);
                    }
                    p.regions.push(Region3D {
                        point: [
                            0.43 * w,
                            0.37 * l,
                            (levels[layer] + levels[layer + 1]) * 0.5,
                        ],
                        marker: layer as i32 + 1,
                        max_volume: 0.0,
                    });
                }
                if self == Self::Layers {
                    p.facets.extend([[4, 5, 6], [4, 6, 7]]);
                    p.facet_markers.extend([10, 10]);
                }
            }
            Self::Sphere => {
                let lat = resolution.clamp(4, 12);
                let lon = lat * 2;
                p.vertices.push([0.0, 0.0, radius]);
                for j in 1..lat {
                    let v = std::f64::consts::PI * j as f64 / lat as f64;
                    for i in 0..lon {
                        let u = std::f64::consts::TAU * i as f64 / lon as f64;
                        p.vertices.push([
                            radius * v.sin() * u.cos(),
                            radius * v.sin() * u.sin(),
                            radius * v.cos(),
                        ]);
                    }
                }
                let south = p.vertices.len();
                p.vertices.push([0.0, 0.0, -radius]);
                for i in 0..lon {
                    let next = (i + 1) % lon;
                    p.facets.push([0, 1 + i, 1 + next]);
                    let last = 1 + (lat - 2) * lon;
                    p.facets.push([south, last + next, last + i]);
                    for j in 0..lat - 2 {
                        let a = 1 + j * lon + i;
                        let b = 1 + j * lon + next;
                        p.facets.extend([[a, a + lon, b + lon], [a, b + lon, b]]);
                    }
                }
                p.regions.push(Region3D {
                    point: [0.0, 0.0, 0.0],
                    marker: 1,
                    max_volume: 0.0,
                });
            }
            Self::Torus => {
                let nu = resolution.clamp(4, 12) * 2 - 2;
                let nv = resolution.clamp(4, 12) + 2;
                let tube = tube.min(radius * 0.8);
                for i in 0..nu {
                    for j in 0..nv {
                        let u = std::f64::consts::TAU * i as f64 / nu as f64;
                        let v = std::f64::consts::TAU * j as f64 / nv as f64;
                        let r = radius + tube * v.cos();
                        p.vertices.push([r * u.cos(), r * u.sin(), tube * v.sin()]);
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
                p.regions.push(Region3D {
                    point: [radius, 0.0, 0.0],
                    marker: 1,
                    max_volume: 0.0,
                });
            }
        }
        if p.facet_markers.is_empty() {
            p.facet_markers = vec![1; p.facets.len()];
        }
        let (lo, hi) = crate::view3d::bounds(&p.vertices);
        let volume = (0..3).map(|k| hi[k] - lo[k]).product::<f64>();
        Project3D {
            format: FORMAT.into(),
            version: 1,
            name: self.title().into(),
            units: "m".into(),
            plc: p,
            options: Options3D {
                max_volume: volume / 60.0,
                min_dihedral: 5.0,
                max_vertices: 3000,
                max_passes: 24,
            },
        }
    }
}

struct Running {
    job: Job,
    started: f64,
    project: Project3D,
}
struct Done {
    picture: Picture3D,
    seconds: f64,
    project: Project3D,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Templates,
    Geometry,
    Mesh,
}

pub struct Studio3D {
    pub project: Project3D,
    template: Template3D,
    dimensions: [f64; 3],
    radius: f64,
    tube: f64,
    resolution: usize,
    tab: Tab,
    editor: String,
    applied_editor: String,
    error: Option<String>,
    notice: Option<String>,
    running: Option<Running>,
    done: Option<Done>,
    view: View3D,
}
impl Default for Studio3D {
    fn default() -> Self {
        let template = Template3D::Block;
        let dimensions = [10.0, 8.0, 6.0];
        let project = template.build(dimensions, 2.0, 0.6, 6);
        let editor = serde_json::to_string_pretty(&project.plc).unwrap();
        Self {
            project,
            template,
            dimensions,
            radius: 2.0,
            tube: 0.6,
            resolution: 6,
            tab: Tab::Templates,
            applied_editor: editor.clone(),
            editor,
            error: None,
            notice: None,
            running: None,
            done: None,
            view: View3D::default(),
        }
    }
}
impl Studio3D {
    pub fn has_mesh(&self) -> bool {
        self.done.is_some()
    }
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }
    fn dirty(&self) -> bool {
        self.editor != self.applied_editor
    }
    fn stale(&self) -> bool {
        self.done.as_ref().is_some_and(|d| {
            d.project.plc != self.project.plc || d.project.options != self.project.options
        })
    }
    fn stem(&self) -> String {
        let s: String = self
            .project
            .name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        if s.is_empty() {
            "mesh-3d".into()
        } else {
            s
        }
    }
    fn load(&mut self, project: Project3D) {
        if let Some(r) = self.running.take() {
            r.job.stop();
        }
        self.project = project;
        self.editor = serde_json::to_string_pretty(&self.project.plc).unwrap();
        self.applied_editor = self.editor.clone();
        self.done = None;
        self.error = None;
        self.notice = None;
        self.view = View3D::default();
    }
    pub fn open(&mut self, text: &str) {
        match Project3D::from_json(text) {
            Ok(project) => {
                self.load(project);
                self.tab = Tab::Geometry;
                self.notice = Some("Opened 3D project".into());
            }
            Err(e) => self.error = Some(e),
        }
    }
    pub fn file_error(&mut self, error: String) {
        self.error = Some(error);
    }
    pub fn save(&mut self) {
        if self.dirty() {
            self.error = Some("Apply or reset the geometry edits before saving.".into());
            return;
        }
        if let Err(e) = self.project.validate() {
            self.error = Some(e);
            return;
        }
        match serde_json::to_string_pretty(&self.project) {
            Ok(text) => {
                let result = files::save(
                    &format!("{}.rmesh.json", self.stem()),
                    files::PROJECT,
                    |w| w.write_all(text.as_bytes()),
                );
                self.saved(result);
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }
    pub fn export(&mut self, gmsh: bool) {
        let Some(done) = &self.done else { return };
        let stem = self.stem();
        let mesh = &done.picture.mesh;
        let result = if gmsh {
            files::save(&format!("{stem}.msh"), ("Gmsh mesh", &["msh"]), |w| {
                rustmesh::export::write_gmsh3d(mesh, w)
            })
        } else {
            files::save(&format!("{stem}.vtu"), ("VTK mesh", &["vtu"]), |w| {
                rustmesh::export::write_vtu3d(mesh, w)
            })
        };
        self.saved(result);
    }
    fn saved(&mut self, result: Result<Option<String>, String>) {
        match result {
            Ok(Some(name)) => self.notice = Some(format!("Saved {name}")),
            Err(e) => self.error = Some(e),
            _ => {}
        }
    }
    fn build(&mut self, ctx: &egui::Context) {
        if self.running.is_some() || self.dirty() {
            return;
        }
        self.error = None;
        self.notice = None;
        self.tab = Tab::Mesh;
        if let Err(e) = self.project.validate() {
            self.error = Some(e);
            return;
        }
        let request = Request::ThreeD {
            plc: self.project.plc.clone(),
            options: self.project.options.clone(),
        };
        match Job::start(ctx, request) {
            Ok(job) => {
                self.running = Some(Running {
                    job,
                    started: ctx.input(|i| i.time),
                    project: self.project.clone(),
                })
            }
            Err(e) => self.error = Some(e),
        }
    }
    pub fn poll(&mut self) {
        let Some(outcome) = self.running.as_mut().and_then(|r| r.job.poll()) else {
            return;
        };
        let running = self.running.take().unwrap();
        match outcome.mesh {
            Ok(Product::ThreeD(mesh)) => {
                self.done = Some(Done {
                    picture: Picture3D::new(mesh),
                    seconds: outcome.seconds,
                    project: running.project,
                })
            }
            Ok(Product::TwoD(_)) => self.error = Some("Unexpected 2D result for a 3D job".into()),
            Err(e) => self.error = Some(e),
        }
    }
    fn footer(&mut self, ui: &mut egui::Ui) {
        let button = egui::Button::new(
            RichText::new("Build 3D mesh")
                .color(egui::Color32::WHITE)
                .strong()
                .atom_grow(true),
        )
        .fill(theme::ACCENT)
        .min_size(egui::vec2(ui.available_width(), 40.0));
        if ui
            .add_enabled(!self.is_running() && !self.dirty(), button)
            .clicked()
        {
            self.build(ui.ctx());
        }
        if let Some(r) = &self.running {
            let elapsed = ui.input(|i| i.time) - r.started;
            let mut stop = false;
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Meshing… {elapsed:.1} s"));
                stop = ui
                    .button("Stop")
                    .on_hover_text(if Job::STOP_ENDS_WORK {
                        "Terminates the mesh worker"
                    } else {
                        "Drops the result; the computation finishes in the background"
                    })
                    .clicked();
            });
            if stop {
                self.running.take().unwrap().job.stop();
                self.notice = Some("Mesh request stopped".into());
            }
        } else {
            ui.label(
                RichText::new(if self.dirty() {
                    "Apply or reset geometry edits to build"
                } else {
                    "3D meshing · Experimental"
                })
                .small()
                .weak(),
            );
        }
    }
    fn templates(&mut self, ui: &mut egui::Ui) {
        for t in Template3D::ALL {
            ui.selectable_value(&mut self.template, t, t.title());
        }
        ui.label(RichText::new(self.template.description()).small().weak());
        ui.separator();
        egui::Grid::new("template3d_params")
            .num_columns(2)
            .show(ui, |ui| {
                if matches!(self.template, Template3D::Block | Template3D::Layers) {
                    for (label, value) in ["Width (x)", "Length (y)", "Height (z)"]
                        .into_iter()
                        .zip(self.dimensions.iter_mut())
                    {
                        ui.label(label);
                        ui.add(egui::DragValue::new(value).speed(0.1).range(0.01..=1e5));
                        ui.end_row();
                    }
                } else {
                    ui.label(if self.template == Template3D::Torus {
                        "Major radius"
                    } else {
                        "Radius"
                    });
                    ui.add(
                        egui::DragValue::new(&mut self.radius)
                            .speed(0.05)
                            .range(0.01..=1e5),
                    );
                    ui.end_row();
                    if self.template == Template3D::Torus {
                        self.tube = self.tube.min(self.radius * 0.8);
                        ui.label("Tube radius");
                        ui.add(
                            egui::DragValue::new(&mut self.tube)
                                .speed(0.02)
                                .range(0.001..=self.radius * 0.8),
                        );
                        ui.end_row();
                    }
                    ui.label("Surface resolution");
                    ui.add(egui::DragValue::new(&mut self.resolution).range(4..=12));
                    ui.end_row();
                }
            });
        if ui
            .add_enabled(!self.is_running(), egui::Button::new("Use 3D template"))
            .clicked()
        {
            self.load(self.template.build(
                self.dimensions,
                self.radius,
                self.tube,
                self.resolution,
            ));
            self.tab = Tab::Geometry;
        }
    }
    fn geometry(&mut self, ui: &mut egui::Ui) {
        ui.label("Name");
        ui.text_edit_singleline(&mut self.project.name);
        ui.horizontal(|ui| {
            ui.label("Length unit");
            ui.add(egui::TextEdit::singleline(&mut self.project.units).desired_width(65.0));
        });
        let p = &self.project.plc;
        ui.label(format!(
            "{} vertices · {} surface facets",
            p.vertices.len(),
            p.facets.len()
        ));
        ui.label(format!(
            "{} material seeds · {} cavity seeds",
            p.regions.len(),
            p.holes.len()
        ));
        ui.label(
            RichText::new("Open a 3D project to use your own geometry, or edit the triangulated surfaces below.")
                .small()
                .weak(),
        );
        egui::CollapsingHeader::new("Edit geometry (JSON)").show(ui,|ui| {
            ui.label(RichText::new("Vertices are [x,y,z]. Facets reference three zero-based vertex indices. Keep the outer surface closed.").small());
            egui::ScrollArea::both().id_salt("plc3d_editor").max_height(280.0).show(ui,|ui| {
                ui.add(egui::TextEdit::multiline(&mut self.editor).font(egui::TextStyle::Monospace).desired_width(ui.available_width()).desired_rows(12));
            });
            ui.horizontal(|ui| {
                if ui.button("Apply geometry").clicked() {
                    let result=serde_json::from_str::<Plc3D>(&self.editor).map_err(|e| e.to_string())
                        .and_then(|p| {validate_geometry(&p)?; Ok(p)});
                    match result {
                        Ok(plc)=>{ self.project.plc=plc; self.applied_editor=self.editor.clone(); self.error=None; self.view.reset(); },
                        Err(e)=>self.error=Some(e),
                    }
                }
                if ui.button("Reset edits").clicked() { self.editor=self.applied_editor.clone(); self.error=None; }
            });
        });
    }
    fn settings(&mut self, ui: &mut egui::Ui) {
        let o = &mut self.project.options;
        egui::Grid::new("mesh3d_options")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label(format!("Max. volume ({}³)", self.project.units));
                ui.add(
                    egui::DragValue::new(&mut o.max_volume)
                        .speed(0.01)
                        .range(0.0..=1e30),
                )
                .on_hover_text("0 disables the global volume limit");
                ui.end_row();
                ui.label("Min. dihedral (°)");
                ui.add(egui::Slider::new(&mut o.min_dihedral, 0.0..=30.0))
                    .on_hover_text("Best effort target; 0 disables angle refinement");
                ui.end_row();
                ui.label("Node budget");
                ui.add(
                    egui::DragValue::new(&mut o.max_vertices)
                        .speed(100.0)
                        .range(100..=100_000),
                );
                ui.end_row();
                ui.label("Pass budget");
                ui.add(egui::DragValue::new(&mut o.max_passes).range(1..=500));
                ui.end_row();
            });
        ui.label(RichText::new("Tighter targets can increase computation time. Review unmet constraints and quality before using a mesh in a solver.").small().weak());
        if let Some(d) = &self.done {
            let m = &d.picture.mesh;
            let r = &m.report;
            ui.separator();
            ui.strong("Mesh report");
            if self.stale() {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "Settings changed — rebuild to update the mesh.",
                );
            }
            egui::Grid::new("mesh3d_report")
                .num_columns(2)
                .show(ui, |ui| {
                    for (label, value) in [
                        ("Nodes", m.nodes.len().to_string()),
                        ("Tetrahedra", m.tetrahedra.len().to_string()),
                        ("Smallest dihedral", format!("{:.2}°", r.min_dihedral)),
                        ("Total volume", format!("{:.4}", r.total_volume)),
                        ("Volume violations", r.volume_violations.to_string()),
                        ("Angle violations", r.dihedral_violations.to_string()),
                        ("Slivers below 1°", r.sliver_count.to_string()),
                        ("Time", format!("{:.2} s", d.seconds)),
                    ] {
                        ui.label(label);
                        ui.label(value);
                        ui.end_row();
                    }
                });
            for issue in &r.issues {
                ui.colored_label(ui.visuals().warn_fg_color, issue);
            }
            egui::CollapsingHeader::new("Mesher notes").show(ui, |ui| {
                for warning in &r.warnings {
                    ui.label(RichText::new(warning).small());
                }
            });
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("controls3d")
            .resizable(true)
            .default_size(380.0)
            .min_size(350.0)
            .frame(
                egui::Frame::new()
                    .fill(ui.visuals().panel_fill)
                    .inner_margin(16),
            )
            .show(ui, |ui| {
                egui::Panel::bottom("build3d")
                    .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 12)))
                    .show(ui, |ui| self.footer(ui));
                let width = (ui.available_width() - 8.0) / 3.0;
                egui::Frame::new()
                    .fill(ui.visuals().widgets.noninteractive.bg_stroke.color)
                    .corner_radius(9)
                    .inner_margin(3)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 1.0;
                        ui.horizontal(|ui| {
                            for (tab, label) in [
                                (Tab::Templates, "Templates"),
                                (Tab::Geometry, "Geometry"),
                                (Tab::Mesh, "Mesh"),
                            ] {
                                let selected = self.tab == tab;
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
                                    self.tab = tab;
                                }
                            }
                        });
                    });
                ui.add_space(8.0);
                ui.label(
                    RichText::new(match self.tab {
                        Tab::Templates => "Build in three dimensions",
                        Tab::Geometry => "Define your volume",
                        Tab::Mesh => "Refine your volume",
                    })
                    .size(19.0)
                    .strong(),
                );
                egui::ScrollArea::vertical()
                    .id_salt(("inspector3d", self.tab as u8))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        theme::card(ui).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            match self.tab {
                                Tab::Templates => self.templates(ui),
                                Tab::Geometry => self.geometry(ui),
                                Tab::Mesh => self.settings(ui),
                            }
                        });
                        if let Some(e) = &self.error {
                            ui.colored_label(ui.visuals().error_fg_color, e);
                        }
                        if let Some(n) = &self.notice {
                            ui.label(RichText::new(n).small().weak());
                        }
                    });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(ui.visuals().panel_fill).inner_margin(20))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.heading(&self.project.name);
                    let status = if self.is_running() {
                        "Building…"
                    } else if self.dirty() {
                        "Unapplied geometry edits"
                    } else if self.stale() {
                        "Needs rebuild"
                    } else if let Some(d) = &self.done {
                        if d.picture.mesh.report.complete {
                            "Mesh ready"
                        } else {
                            "Review mesh report"
                        }
                    } else {
                        "3D geometry preview"
                    };
                    ui.label(RichText::new(status).small().weak());
                });
                if let Some(d) = &self.done {
                    ui.label(
                        RichText::new(format!(
                            "{} nodes · {} tetrahedra · {:.2}° minimum dihedral",
                            d.picture.mesh.nodes.len(),
                            d.picture.mesh.tetrahedra.len(),
                            d.picture.mesh.report.min_dihedral
                        ))
                        .small()
                        .weak(),
                    );
                } else {
                    ui.label(RichText::new("A solid volume, ready to explore.").weak());
                }
                self.view.toolbar(ui, self.has_mesh());
                egui::Panel::bottom("hint3d")
                    .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 8)))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("Drag to rotate · Right-drag to pan · Scroll to zoom · Double-click to fit")
                                .small()
                                .weak(),
                        );
                    });
                let picture = self.done.as_ref().map(|d| &d.picture);
                theme::card(ui).inner_margin(8).show(ui, |ui| {
                    self.view.show(ui, &self.project.plc, picture, &self.project.units)
                });
            });
    }
}
