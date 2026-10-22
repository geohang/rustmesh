//! Desktop bridge: one shared workspace for manual edits, MCP and the agent panel.
use crate::{
    agent::{AgentConfig, AgentController, AgentEvent},
    mcp::{self, AttachServer},
    workspace::{MeshRecord, ProjectSnapshot, Workspace},
};
use eframe::egui::{self, RichText};
use rustmesh::Geometry2D;
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Arc, Mutex},
};

pub struct DesktopSession {
    pub shared: Arc<Mutex<Workspace>>,
    _attach: AttachServer,
    agent: AgentController,
    pub open: bool,
    pub selected_mesh: Option<String>,
    pub revision: Option<(String, u64)>,
    root: PathBuf,
    prompt: String,
    models: Vec<crate::agent::ModelChoice>,
    model: String,
    account: String,
    status: String,
    connected: bool,
    busy: bool,
    messages: Vec<String>,
    report: Option<Value>,
    comparison: Vec<String>,
    jobs_before_turn: BTreeSet<String>,
    manual_job: Option<String>,
    last_auto_mesh: Option<String>,
}
fn op() -> String {
    uuid::Uuid::new_v4().to_string()
}
impl DesktopSession {
    pub fn new(geometry: &Geometry2D) -> Result<Self, String> {
        let data = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("rustmesh");
        let root = std::env::var_os("RUSTMESH_WORKSPACE")
            .map(PathBuf::from)
            .unwrap_or_else(|| data.join("workspaces/default"));
        Self::at_root(geometry, root, data)
    }
    pub(crate) fn at_root(
        geometry: &Geometry2D,
        root: PathBuf,
        data: PathBuf,
    ) -> Result<Self, String> {
        let mut workspace = Workspace::new(root.clone())?;
        if workspace.current_project().is_none() {
            workspace.commit_geometry(0, geometry.clone(), 100_000)?;
        }
        let shared = Arc::new(Mutex::new(workspace));
        let attach = mcp::start_attach(shared.clone())?;
        let cli = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .with_file_name(if cfg!(windows) {
                "rustmesh.exe"
            } else {
                "rustmesh"
            });
        let config = AgentConfig {
            codex: crate::agent::detect_codex().unwrap_or_else(|| PathBuf::from("codex")),
            data_dir: data.join("agent"),
            workspace_dir: data.join("agent/isolated-workspace"),
            mcp_command: cli,
            mcp_args: vec!["mcp".into(), "--attach".into(), attach.address.clone()],
            mcp_token: attach.token.clone(),
        };
        Ok(Self{shared,_attach:attach,agent:AgentController::new(config),open:false,selected_mesh:None,revision:None,root,
            prompt:"Create a 2D project from the plate-with-hole template. Keep at most 100000 nodes. Generate the default mesh and a finer one with a smaller largest cell area, inspect both reports, compare them, undo the sizing change and export both as Gmsh. Report the mesh IDs and any quality limits that were not met.".into(),
            models:vec![],model:String::new(),account:"Disconnected".into(),status:"Sign in with ChatGPT to use the assistant.".into(),connected:false,busy:false,messages:vec![],report:None,comparison:vec![],jobs_before_turn:BTreeSet::new(),manual_job:None,last_auto_mesh:None})
    }
    pub fn snapshot(&self) -> Option<ProjectSnapshot> {
        self.shared.lock().ok()?.current_project().cloned()
    }
    pub fn commit(&mut self, geometry: Geometry2D, max_nodes: usize) -> Result<(), String> {
        let expected = self.revision.as_ref().map_or(0, |p| p.1);
        let snapshot = self
            .shared
            .lock()
            .map_err(|e| e.to_string())?
            .commit_geometry(expected, geometry, max_nodes)?;
        self.revision = Some((snapshot.project_id, snapshot.revision));
        Ok(())
    }
    fn call(&mut self, name: &str, mut args: Value) -> Result<Value, String> {
        args["schema_version"] = json!(1);
        let answer = self
            .shared
            .lock()
            .map_err(|e| e.to_string())?
            .call(name, args);
        if answer["ok"] == true {
            Ok(answer["result"].clone())
        } else {
            Err(answer["error"].to_string())
        }
    }
    pub fn build(&mut self) -> Result<(), String> {
        let p = self.snapshot().ok_or("No current project")?;
        let result=self.call("generate_mesh",json!({"operation_id":op(),"project_id":p.project_id,"revision":p.revision,"backend":"rustmesh","budget":{"max_nodes":p.max_nodes,"max_cells":200000,"wall_seconds":120.0,"max_output_bytes":67108864}}))?;
        self.manual_job = result["job_id"].as_str().map(str::to_owned);
        Ok(())
    }
    pub fn export_for_dialog(&mut self, format: &str) -> Result<Vec<u8>, String> {
        let id = self.selected_mesh.clone().ok_or("Select a mesh first")?;
        let filename = format!("{id}-{}.{format}", op());
        self.call("export_mesh",json!({"operation_id":op(),"mesh_id":id,"format":format,"filename":filename,"allow_partial":true}))?;
        std::fs::read(self.root.join("exports").join(filename)).map_err(|e| e.to_string())
    }
    fn save_turn_record(&mut self, status: &str, tool_calls: usize) {
        let meshes:Vec<_>=self.shared.lock().map(|w|w.meshes().map(|m|json!({"mesh_id":m.mesh_id,"project_id":m.project_id,"revision":m.revision,"artifact":m.artifact})).collect()).unwrap_or_default();
        let record = json!({"schema_version":1,"provider":"codex","requested_model":self.model,"runtime_version":crate::agent::SUPPORTED_VERSION,"build_id":env!("RUSTMESH_BUILD_ID"),"unix_seconds":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0,|d|d.as_secs()),"turn_status":status,"successful_rustmesh_tool_calls":tool_calls,"meshes":meshes,"note":"Summary of one agent turn. It contains no credentials or account identity."});
        let path = self.root.join(format!("agent-turn-{}.json", op()));
        if let Err(e) = std::fs::write(&path, serde_json::to_vec_pretty(&record).unwrap()) {
            self.messages
                .push(format!("Could not save the turn record: {e}"));
        }
    }
    pub fn is_running(&self) -> bool {
        self.manual_job.is_some()
    }
    pub fn stop_manual(&mut self) {
        if let Some(id) = self.manual_job.take() {
            if let Err(e) = self.call("cancel_job", json!({"job_id":id})) {
                self.status = e;
                self.manual_job = Some(id);
            }
        }
    }
    pub fn mesh_geometry(&self, mesh: &MeshRecord) -> Option<Geometry2D> {
        self.shared
            .lock()
            .ok()?
            .snapshot(&mesh.project_id, Some(mesh.revision))
            .ok()
            .map(|p| p.geometry)
    }
    pub fn selected(&self) -> Option<MeshRecord> {
        self.shared
            .lock()
            .ok()?
            .mesh(self.selected_mesh.as_deref()?)
            .cloned()
    }
    pub fn poll(&mut self) {
        if let Ok(mut w) = self.shared.lock() {
            w.poll_jobs();
            if let Some(mesh) = w.latest_mesh() {
                if self.last_auto_mesh.as_deref() != Some(&mesh.mesh_id) {
                    self.selected_mesh = Some(mesh.mesh_id.clone());
                    self.last_auto_mesh = Some(mesh.mesh_id.clone());
                }
            }
        }
        if let Some(job) = self.manual_job.clone() {
            match self.call("read_job", json!({"job_id":job})) {
                Ok(value) => {
                    if value["state"] != "running" {
                        self.manual_job = None;
                        self.status = format!("Mesh job {}", value["state"]);
                        if value["state"] != "completed" {
                            self.messages.push(value.to_string());
                        }
                    }
                }
                Err(e) => {
                    self.manual_job = None;
                    self.status = e
                }
            }
        }
        for event in self.agent.poll() {
            match event {
                AgentEvent::Auth { label, plan_use } => {
                    self.account = label;
                    self.connected = plan_use;
                }
                AgentEvent::Models(models) => {
                    if !models.iter().any(|m| m.slug == self.model) {
                        self.model = models.first().map(|m| m.slug.clone()).unwrap_or_default();
                    }
                    self.models = models;
                }
                AgentEvent::Delta(delta) => {
                    if self
                        .messages
                        .last()
                        .is_none_or(|m| !m.starts_with("Assistant: "))
                    {
                        self.messages.push("Assistant: ".into());
                    }
                    self.messages.last_mut().unwrap().push_str(&delta);
                }
                AgentEvent::Tool { name, status } => {
                    self.messages.push(format!("Tool {name}: {status}"))
                }
                AgentEvent::Usage(value) => self.messages.push(format!("Usage: {value}")),
                AgentEvent::Completed { status, tool_calls } => {
                    if let Ok(mut w) = self.shared.lock() {
                        w.end_owner_turn("mcp");
                    }
                    self.save_turn_record(&status, tool_calls);
                    self.busy = false;
                    self.status = format!("Turn {status}; {tool_calls} tool calls.");
                    self.messages.push(self.status.clone());
                }
                AgentEvent::Error(e) => {
                    if let Ok(mut w) = self.shared.lock() {
                        w.end_owner_turn("mcp");
                    }
                    self.busy = false;
                    self.status = e.clone();
                    self.messages.push(e);
                }
                AgentEvent::Status(status) => self.status = status,
                AgentEvent::Disconnected => {
                    self.connected = false;
                    self.busy = false;
                    self.account = "Disconnected".into();
                }
            }
        }
        if self.messages.len() > 300 {
            self.messages.drain(..100);
        }
    }
    fn stop_turn(&mut self) {
        if let Ok(mut w) = self.shared.lock() {
            w.end_owner_turn("mcp");
        }
        self.agent.cancel();
        self.busy = true;
        let jobs: Vec<String> = self
            .shared
            .lock()
            .map(|w| {
                w.jobs()
                    .filter(|j| j["state"] == "running" && j["owner"] == "mcp")
                    .filter_map(|j| j["job_id"].as_str().map(str::to_owned))
                    .filter(|id| !self.jobs_before_turn.contains(id))
                    .collect()
            })
            .unwrap_or_default();
        let mut failures = Vec::new();
        for id in jobs {
            if let Err(e) = self.call("cancel_job", json!({"job_id":id})) {
                failures.push(e);
            }
        }
        self.status = if failures.is_empty() {
            "Stopping agent; associated running mesh processes have exited.".into()
        } else {
            format!(
                "Agent stop requested; mesh cancellation errors: {}",
                failures.join("; ")
            )
        };
    }
    pub fn show(&mut self, ui: &mut egui::Ui) {
        if !self.open {
            return;
        }
        egui::Panel::right("agent-panel").resizable(true).default_size(350.0).min_size(280.0).show(ui,|ui|{
            ui.heading("Agent & workspace");
            ui.label(RichText::new("Meshing runs locally. Prompts and tool summaries go to the selected model provider.").small());
            egui::ScrollArea::vertical().id_salt("agent-scroll").show(ui,|ui|{
                ui.label(format!("Codex / ChatGPT · {}",self.account));
                if ui.button("Continue with ChatGPT").clicked(){self.agent.begin_sign_in(None);}
                let registrations=self.agent.registrations();
                for (i,r) in registrations.iter().enumerate(){if ui.small_button(format!("Reconnect {}",if r.email.is_empty(){&r.subject}else{&r.email})).clicked(){self.agent.begin_sign_in(Some(i));}}
                if self.connected && ui.small_button("Disconnect rustmesh session").clicked(){self.stop_turn();self.agent.disconnect();}
                egui::ComboBox::from_id_salt("provider-model").selected_text(if self.model.is_empty(){"Sign in to load model access"}else{&self.model}).show_ui(ui,|ui|{for model in &self.models{ui.selectable_value(&mut self.model,model.slug.clone(),&model.display_name);}});
                ui.colored_label(ui.visuals().weak_text_color(),&self.status);
                ui.add(egui::TextEdit::multiline(&mut self.prompt).desired_rows(5).desired_width(f32::INFINITY));
                ui.horizontal(|ui|{
                    if ui.add_enabled(self.connected&&!self.busy&&!self.model.is_empty(),egui::Button::new("Send")).clicked(){
                        self.jobs_before_turn=self.shared.lock().map(|w|w.jobs().filter_map(|j|j["job_id"].as_str().map(str::to_owned)).collect()).unwrap_or_default();
                        if let Ok(mut w)=self.shared.lock(){w.begin_owner_turn("mcp",3);}
                        self.messages.push(format!("You: {}",self.prompt));self.busy=true;self.agent.send_message(&self.model,&self.prompt);
                    }
                    if ui.add_enabled(self.busy,egui::Button::new("Stop turn & jobs")).clicked(){self.stop_turn();}
                });
                ui.separator();
                if let Some(p)=self.snapshot(){
                    ui.label(format!("{} · revision {} · {}",p.project_id,p.revision,p.geometry.units));
                    if ui.add_enabled(p.revision>1,egui::Button::new("Undo last project change")).clicked(){match self.call("restore_project_revision",json!({"operation_id":op(),"project_id":p.project_id,"target_revision":p.revision-1,"expected_revision":p.revision})){Ok(v)=>self.status=format!("Restored as revision {}",v["revision"]),Err(e)=>self.status=e}}
                }
                let meshes:Vec<_>=self.shared.lock().map(|w|w.meshes().map(|m|(m.mesh_id.clone(),m.revision,m.mesh.nodes.len(),m.mesh.triangles.len())).collect()).unwrap_or_default();
                for (id,rev,nodes,cells) in meshes{
                    ui.group(|ui|{
                        ui.label(format!("{id}\nrevision {rev} · {nodes} nodes · {cells} cells"));
                        ui.horizontal_wrapped(|ui|{
                            if ui.selectable_label(self.selected_mesh.as_deref()==Some(&id),"Canvas / report").clicked(){self.selected_mesh=Some(id.clone());match self.call("inspect_mesh",json!({"mesh_id":id})){Ok(v)=>self.report=Some(v),Err(e)=>self.status=e}}
                            let mut compare=self.comparison.contains(&id);if ui.checkbox(&mut compare,"Compare").changed(){self.comparison.retain(|m|m!=&id);if compare{self.comparison.push(id.clone());if self.comparison.len()>2{self.comparison.remove(0);}}}
                            if ui.button("Export MSH").clicked(){match self.call("export_mesh",json!({"operation_id":op(),"mesh_id":id,"format":"msh","filename":format!("{id}-{}.msh",op()),"allow_partial":true})){Ok(v)=>{self.status="Exported with its report; unmet quality limits are listed there.".into();self.report=Some(v)},Err(e)=>self.status=e}}
                        });
                    });
                }
                if ui.add_enabled(self.comparison.len()==2,egui::Button::new("Compare selected reports")).clicked(){match self.call("compare_meshes",json!({"mesh_ids":self.comparison})){Ok(v)=>self.report=Some(v),Err(e)=>self.status=e}}
                if ui.button("Save offline workflow").clicked(){let path=self.root.join(format!("workflow-{}.json",op()));match self.shared.lock().map_err(|e|e.to_string()).and_then(|w|w.export_workflow(&path)){Ok(())=>self.status=format!("Saved {}",path.display()),Err(e)=>self.status=e}}
                ui.label(RichText::new(format!("Workspace: {}",self.root.display())).small());
                if let Some(report)=&self.report{egui::CollapsingHeader::new("Report / comparison").default_open(true).show(ui,|ui|{ui.add(egui::Label::new(RichText::new(serde_json::to_string_pretty(report).unwrap_or_default()).monospace()).wrap());});}
                ui.separator();
                for message in &self.messages{ui.label(message);}
            });
        });
    }
}
