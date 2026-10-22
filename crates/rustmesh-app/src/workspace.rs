//! Workspace transactions, immutable inputs and worker-process jobs.
use crate::{
    jobs::{ProcessJob, Product, Request},
    tools::*,
};
use rustmesh::{Geometry2D, Mesh2D};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn id(kind: &str, seed: &str) -> String {
    format!("{kind}-{}", &hash(seed.as_bytes())[..24])
}
fn err(code: &str, message: impl ToString) -> Value {
    json!({"schema_version":VERSION,"ok":false,"error":{"code":code,"message":message.to_string()}})
}
fn ok(value: Value) -> Value {
    json!({"schema_version":VERSION,"ok":true,"result":value})
}
fn parse<T: DeserializeOwned>(a: Value) -> Result<T, String> {
    serde_json::from_value(a).map_err(|e| format!("INVALID_ARGUMENT: {e}"))
}
fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ProjectSnapshot {
    pub project_id: String,
    pub revision: u64,
    pub geometry: Geometry2D,
    pub max_nodes: usize,
    pub entities: BTreeMap<String, Entity>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Entity {
    pub kind: String,
    pub index: usize,
}
#[derive(Clone, Serialize, Deserialize)]
struct Patch {
    project_id: String,
    base_revision: u64,
    candidate: ProjectSnapshot,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct MeshRecord {
    pub mesh_id: String,
    pub job_id: String,
    pub project_id: String,
    pub revision: u64,
    pub input_hash: String,
    pub mesh: Mesh2D,
    pub report: Value,
    pub artifact: Value,
}
#[derive(Clone, Serialize, Deserialize)]
struct Operation {
    signature: String,
    result: Value,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct WorkflowCommand {
    pub name: String,
    pub arguments: Value,
}
#[derive(Clone, Serialize, Deserialize, Default)]
struct State {
    current: Option<String>,
    projects: BTreeMap<String, Vec<ProjectSnapshot>>,
    patches: BTreeMap<String, Patch>,
    meshes: BTreeMap<String, MeshRecord>,
    jobs: BTreeMap<String, Value>,
    operations: BTreeMap<String, Operation>,
    workflow: Vec<WorkflowCommand>,
}
struct Running {
    process: ProcessJob,
    snapshot: ProjectSnapshot,
    budget: Budget,
    input_hash: String,
}
pub struct Workspace {
    root: PathBuf,
    state: State,
    running: BTreeMap<String, Running>,
    _lease: fs::File,
    call_owner: String,
    owner_limits: BTreeMap<String, (usize, usize)>,
}

impl Workspace {
    pub fn new(root: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        let lease = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("workspace.lock"))
            .map_err(|e| e.to_string())?;
        lease
            .try_lock()
            .map_err(|e| format!("WORKSPACE_BUSY: use MCP attach to the running owner ({e})"))?;
        for name in ["artifacts", "exports"] {
            fs::create_dir_all(root.join(name)).map_err(|e| e.to_string())?;
            require(
                root.join(name).canonicalize().map_err(|e| e.to_string())? == root.join(name),
                "INVALID_PATH: workspace subdirectories cannot be links",
            )?;
        }
        let saved = if root.join("workspace.json").exists() {
            root.join("workspace.json")
        } else {
            root.join("workspace.previous.json")
        };
        let mut state = if saved.exists() {
            let v: Value = serde_json::from_slice(&fs::read(saved).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            require(
                v["schema_version"] == VERSION,
                "UNSUPPORTED_VERSION: workspace schema",
            )?;
            serde_json::from_value::<State>(v["state"].clone()).map_err(|e| e.to_string())?
        } else {
            State::default()
        };
        for job in state.jobs.values_mut() {
            if job["state"] == "running" {
                job["state"] = json!("interrupted");
                job["stop_reason"] = json!("backend_failure");
                job["error"] = json!("application exited before recording worker completion");
            }
        }
        Ok(Self {
            root,
            state,
            running: BTreeMap::new(),
            _lease: lease,
            call_owner: "manual".into(),
            owner_limits: BTreeMap::new(),
        })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn current_project(&self) -> Option<&ProjectSnapshot> {
        self.state
            .current
            .as_ref()
            .and_then(|id| self.state.projects.get(id))
            .and_then(|v| v.last())
    }
    pub fn jobs(&self) -> impl Iterator<Item = &Value> {
        self.state.jobs.values()
    }
    pub fn meshes(&self) -> impl Iterator<Item = &MeshRecord> {
        self.state.meshes.values()
    }
    pub fn mesh(&self, id: &str) -> Option<&MeshRecord> {
        self.state.meshes.get(id)
    }
    pub fn latest_mesh(&self) -> Option<&MeshRecord> {
        self.state
            .workflow
            .iter()
            .rev()
            .filter(|c| c.name == "generate_mesh")
            .find_map(|c| {
                self.state
                    .meshes
                    .get(&id("mesh", c.arguments["operation_id"].as_str()?))
            })
    }
    fn save(&self) -> Result<(), String> {
        let data = serde_json::to_vec_pretty(&json!({"schema_version":VERSION,"state":self.state}))
            .map_err(|e| e.to_string())?;
        let temp = self
            .root
            .join(format!("workspace-{}.tmp", uuid::Uuid::new_v4()));
        fs::write(&temp, data).map_err(|e| e.to_string())?;
        // std::fs::rename uses replace-existing semantics on Windows and Unix.
        // Keep the prior complete archive as a recovery copy.
        let target = self.root.join("workspace.json");
        if target.exists() {
            fs::copy(&target, self.root.join("workspace.previous.json"))
                .map_err(|e| e.to_string())?;
        }
        fs::rename(temp, target).map_err(|e| e.to_string())
    }
    pub fn snapshot(
        &self,
        project: &str,
        revision: Option<u64>,
    ) -> Result<ProjectSnapshot, String> {
        let all = self
            .state
            .projects
            .get(project)
            .ok_or("NOT_FOUND: project")?;
        all.iter()
            .find(|s| Some(s.revision) == revision)
            .or_else(|| if revision.is_none() { all.last() } else { None })
            .cloned()
            .ok_or_else(|| "NOT_FOUND: revision".into())
    }
    fn current_revision(&self, project: &str, expected: u64) -> Result<(), String> {
        require(
            self.snapshot(project, None)?.revision == expected,
            "REVISION_CONFLICT: read current project and preview again",
        )
    }
    fn artifact(&self, kind: &str, bytes: &[u8]) -> Result<Value, String> {
        let sha = hash(bytes);
        let filename = format!("{sha}.{kind}");
        let path = self.root.join("artifacts").join(&filename);
        if !path.exists() {
            fs::write(&path, bytes).map_err(|e| e.to_string())?;
        }
        Ok(
            json!({"artifact_id":format!("sha256:{sha}"),"sha256":sha,"bytes":bytes.len(),"path":format!("artifacts/{filename}")}),
        )
    }
    pub fn commit_geometry(
        &mut self,
        base_revision: u64,
        geometry: Geometry2D,
        max_nodes: usize,
    ) -> Result<ProjectSnapshot, String> {
        validate_geometry(&geometry, max_nodes)?;
        let mut candidate = if let Some(old) = self.current_project() {
            self.current_revision(&old.project_id, base_revision)?;
            old.clone()
        } else {
            ProjectSnapshot {
                project_id: format!("project-{}", uuid::Uuid::new_v4()),
                revision: 0,
                geometry: geometry.clone(),
                max_nodes,
                entities: BTreeMap::new(),
            }
        };
        let before = self.state.clone();
        let previous = candidate.clone();
        candidate.geometry = geometry;
        candidate.max_nodes = max_nodes;
        candidate.revision += 1;
        candidate.entities = entities(
            &candidate.project_id,
            &candidate.geometry,
            Some(&previous),
            &format!("manual-{}", candidate.revision),
        );
        self.state.current = Some(candidate.project_id.clone());
        self.state
            .projects
            .entry(candidate.project_id.clone())
            .or_default()
            .push(candidate.clone());
        self.state.workflow.push(WorkflowCommand {
            name: "__manual_snapshot".into(),
            arguments: json!({"snapshot":candidate,"base_revision":base_revision}),
        });
        if let Err(e) = self.save() {
            self.state = before;
            return Err(e);
        }
        Ok(candidate)
    }
    pub fn import_project(&mut self, text: &str) -> Result<ProjectSnapshot, String> {
        let p = crate::project::Project::from_json(text)?;
        self.commit_geometry(
            self.current_project().map_or(0, |p| p.revision),
            p.geometry,
            p.max_nodes,
        )
    }
    pub fn export_workflow(&self, path: &Path) -> Result<(), String> {
        fs::write(path,serde_json::to_vec_pretty(&json!({"format":"rustmesh-workflow","schema_version":VERSION,"commands":self.state.workflow,"build":env!("RUSTMESH_BUILD_ID")})).map_err(|e|e.to_string())?).map_err(|e|e.to_string())
    }
    pub fn replay(&mut self, path: &Path) -> Result<Value, String> {
        let v: Value = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        require(
            v["format"] == "rustmesh-workflow" && v["schema_version"] == VERSION,
            "UNSUPPORTED_VERSION: workflow",
        )?;
        let commands: Vec<WorkflowCommand> =
            serde_json::from_value(v["commands"].clone()).map_err(|e| e.to_string())?;
        let mut results = Vec::new();
        for command in commands {
            if command.name == "__manual_snapshot" {
                let snapshot: ProjectSnapshot =
                    serde_json::from_value(command.arguments["snapshot"].clone())
                        .map_err(|e| e.to_string())?;
                validate_geometry(&snapshot.geometry, snapshot.max_nodes)?;
                let base = command.arguments["base_revision"]
                    .as_u64()
                    .ok_or("INVALID_WORKFLOW: manual base revision")?;
                require(
                    snapshot.revision == base + 1,
                    "INVALID_WORKFLOW: nonmonotonic manual revision",
                )?;
                if let Some(existing) = self.state.projects.get(&snapshot.project_id) {
                    if let Some(same) = existing.iter().find(|s| s.revision == snapshot.revision) {
                        require(
                            serde_json::to_value(same).unwrap()
                                == serde_json::to_value(&snapshot).unwrap(),
                            "REVISION_CONFLICT: replay snapshot differs",
                        )?;
                        continue;
                    }
                    require(
                        existing.last().unwrap().revision == base,
                        "REVISION_CONFLICT: replay base differs",
                    )?;
                } else {
                    require(base == 0, "REVISION_CONFLICT: missing replay base")?;
                }
                self.state.current = Some(snapshot.project_id.clone());
                self.state
                    .projects
                    .entry(snapshot.project_id.clone())
                    .or_default()
                    .push(snapshot);
                self.state.workflow.push(command);
                self.save()?;
                continue;
            }
            let response = self.call(&command.name, command.arguments);
            require(response["ok"] == true, &response.to_string())?;
            if command.name == "generate_mesh" {
                let job = response["result"]["job_id"]
                    .as_str()
                    .ok_or("missing job id")?
                    .to_string();
                loop {
                    self.poll_jobs();
                    if self.state.jobs[&job]["state"] != "running" {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                require(
                    self.state.jobs[&job]["state"] == "completed",
                    &self.state.jobs[&job].to_string(),
                )?;
            }
            results.push(response);
        }
        Ok(json!({"commands":results,"offline":true}))
    }
    pub fn begin_owner_turn(&mut self, owner: &str, max_jobs: usize) {
        self.owner_limits.insert(owner.into(), (0, max_jobs));
    }
    pub fn end_owner_turn(&mut self, owner: &str) {
        if let Some((used, limit)) = self.owner_limits.get_mut(owner) {
            *limit = *used;
        }
    }
    pub fn call_with_owner(&mut self, name: &str, args: Value, owner: &str) -> Value {
        let previous = std::mem::replace(&mut self.call_owner, owner.to_owned());
        let result = self.call(name, args);
        self.call_owner = previous;
        result
    }
    pub fn call(&mut self, name: &str, args: Value) -> Value {
        self.poll_jobs();
        if args.get("schema_version").and_then(Value::as_u64) != Some(VERSION as u64) {
            return err("UNSUPPORTED_VERSION", "schema_version must be 1");
        }
        let args = match canonical_arguments(name, args) {
            Ok(v) => v,
            Err(e) => return err("INVALID_ARGUMENT", e),
        };
        let signature = hash(
            serde_json::to_string(&json!([name, args]))
                .unwrap()
                .as_bytes(),
        );
        let operation = args
            .get("operation_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(op) = &operation {
            if op.is_empty() || op.len() > 128 {
                return err(
                    "INVALID_ARGUMENT",
                    "operation_id must contain 1..128 characters",
                );
            }
            if let Some(prior) = self.state.operations.get(op) {
                return if prior.signature == signature {
                    prior.result.clone()
                } else {
                    err(
                        "OPERATION_CONFLICT",
                        "operation_id already used with different payload",
                    )
                };
            }
        }
        if matches!(name, "get_capabilities" | "read_project" | "read_job") {
            return match self.dispatch(name, args) {
                Ok(v) => ok(v),
                Err(e) => {
                    let (code, msg) = e.split_once(": ").unwrap_or(("DOMAIN_ERROR", &e));
                    err(code, msg)
                }
            };
        }
        let before = self.state.clone();
        let result = match self.dispatch(name, args.clone()) {
            Ok(v) => ok(v),
            Err(e) => {
                self.state = before;
                let (code, msg) = e.split_once(": ").unwrap_or(("DOMAIN_ERROR", &e));
                return err(code, msg);
            }
        };
        if let Some(operation) = operation {
            self.state.operations.insert(
                operation,
                Operation {
                    signature,
                    result: result.clone(),
                },
            );
            self.state.workflow.push(WorkflowCommand {
                name: name.into(),
                arguments: args.clone(),
            });
        }
        if matches!(name, "inspect_mesh" | "compare_meshes") {
            self.state.workflow.push(WorkflowCommand {
                name: name.into(),
                arguments: args,
            });
        }
        if let Err(e) = self.save() {
            let uncommitted: Vec<_> = self
                .running
                .keys()
                .filter(|id| !before.jobs.contains_key(*id))
                .cloned()
                .collect();
            for id in uncommitted {
                if let Some(mut job) = self.running.remove(&id) {
                    let _ = job.process.cancel();
                }
            }
            self.state = before;
            return err("STORAGE_ERROR", e);
        }
        result
    }
    fn dispatch(&mut self, name: &str, args: Value) -> Result<Value, String> {
        match name {
            "get_capabilities" => {
                let _: Empty = parse(args)?;
                Ok(
                    json!({"dimensions":[2],"experimental_dimensions":[3],"domain_3d":false,"templates":rustmesh::examples::Example::ALL.map(|e|e.key()),"formats":["msh","vtu","json"],"backend":"rustmesh-spade","schema_version":VERSION,"max_nodes":5000000,"max_concurrent_jobs":3,"owner_candidate_quota":self.owner_limits.get(&self.call_owner).map(|(used,limit)|json!({"used":used,"limit":limit})),"budgets":{"nodes":"core hard limit","cells":"working triangulation hard checkpoint plus postcondition","wall_time":"worker deadline","output_bytes":"postcondition","memory":"not supported","iterations":"cooperative checkpoint hard limit"},"auth":"workspace owner; attach bearer capability","export_directory":"exports"}),
                )
            }
            "create_project" => {
                let a: Create = parse(args)?;
                require(
                    a.dimension == 2,
                    "UNSUPPORTED_DIMENSION: domain tools currently support 2D",
                )?;
                let template = rustmesh::examples::Example::from_key(&a.template)
                    .ok_or("INVALID_ARGUMENT: unknown template")?;
                let params = template.params();
                for (key, value) in &a.parameters {
                    let p = params
                        .iter()
                        .find(|p| p.key == key)
                        .ok_or("INVALID_ARGUMENT: unknown template parameter")?;
                    require(
                        value.is_finite()
                            && *value >= p.min
                            && *value <= p.max
                            && (!p.whole || value.fract() == 0.0),
                        "INVALID_ARGUMENT: parameter out of range",
                    )?;
                }
                let geometry = template.build(
                    &params
                        .iter()
                        .map(|p| a.parameters.get(p.key).copied().unwrap_or(p.default))
                        .collect::<Vec<_>>(),
                );
                validate_geometry(&geometry, a.max_nodes)?;
                let project_id = id("project", &a.operation_id);
                let candidate = ProjectSnapshot {
                    entities: entities(&project_id, &geometry, None, &a.operation_id),
                    project_id: project_id.clone(),
                    revision: 1,
                    geometry,
                    max_nodes: a.max_nodes,
                };
                self.state.current = Some(project_id.clone());
                self.state
                    .projects
                    .insert(project_id, vec![candidate.clone()]);
                Ok(summary(&candidate))
            }
            "read_project" => {
                let a: ReadProject = parse(args)?;
                Ok(summary(&self.snapshot(&a.project_id, a.revision)?))
            }
            "preview_project_patch" => {
                let a: Preview = parse(args)?;
                self.current_revision(&a.project_id, a.base_revision)?;
                require(
                    !a.operations.is_empty() && a.operations.len() <= 100,
                    "INVALID_ARGUMENT: expected 1..100 domain edits",
                )?;
                let old = self.snapshot(&a.project_id, Some(a.base_revision))?;
                let mut candidate = old.clone();
                for edit in a.operations {
                    edit_geometry(&mut candidate, edit)?;
                }
                validate_geometry(&candidate.geometry, candidate.max_nodes)?;
                candidate.entities = extend_entities(&candidate, &a.operation_id);
                let patch_id = id("patch", &a.operation_id);
                self.state.patches.insert(
                    patch_id.clone(),
                    Patch {
                        project_id: a.project_id,
                        base_revision: a.base_revision,
                        candidate: candidate.clone(),
                    },
                );
                Ok(
                    json!({"patch_id":patch_id,"base_revision":a.base_revision,"validation":"valid","candidate":summary(&candidate)}),
                )
            }
            "apply_project_patch" => {
                let a: Apply = parse(args)?;
                let patch = self
                    .state
                    .patches
                    .get(&a.patch_id)
                    .cloned()
                    .ok_or("NOT_FOUND: patch")?;
                require(
                    a.expected_revision == patch.base_revision,
                    "REVISION_CONFLICT: patch base differs",
                )?;
                self.current_revision(&patch.project_id, a.expected_revision)?;
                let mut candidate = patch.candidate;
                candidate.revision += 1;
                self.state
                    .projects
                    .get_mut(&patch.project_id)
                    .unwrap()
                    .push(candidate.clone());
                self.state.current = Some(patch.project_id);
                Ok(summary(&candidate))
            }
            "restore_project_revision" => {
                let a: Restore = parse(args)?;
                self.current_revision(&a.project_id, a.expected_revision)?;
                let mut candidate = self.snapshot(&a.project_id, Some(a.target_revision))?;
                candidate.revision = a.expected_revision + 1;
                self.state
                    .projects
                    .get_mut(&a.project_id)
                    .unwrap()
                    .push(candidate.clone());
                self.state.current = Some(a.project_id);
                Ok(summary(&candidate))
            }
            "generate_mesh" => {
                let a: Generate = parse(args)?;
                if let Some((used, limit)) = self.owner_limits.get(&self.call_owner) {
                    require(
                        used < limit,
                        "BUDGET_EXCEEDED: candidate limit for this agent turn reached",
                    )?;
                }
                require(
                    a.backend == "rustmesh" || a.backend == "rustmesh-spade",
                    "UNSUPPORTED_BACKEND: use rustmesh",
                )?;
                require(
                    self.running.len() < 3,
                    "BUDGET_EXCEEDED: at most three concurrent candidates",
                )?;
                let snapshot = self.snapshot(&a.project_id, Some(a.revision))?;
                require(
                    a.budget.max_iterations > 0
                        && a.budget.max_iterations <= 10_000_000
                        && a.budget.max_nodes >= 3
                        && a.budget.max_nodes <= snapshot.max_nodes
                        && a.budget.max_cells >= 1
                        && a.budget.max_cells <= snapshot.max_nodes.saturating_mul(2)
                        && a.budget.wall_seconds.is_finite()
                        && a.budget.wall_seconds > 0.0
                        && a.budget.wall_seconds <= 600.0
                        && a.budget.max_output_bytes > 0
                        && a.budget.max_output_bytes <= 128 * 1024 * 1024,
                    "INVALID_ARGUMENT: budget exceeds workspace capability",
                )?;
                let request = Request::TwoD {
                    geometry: snapshot.geometry.clone(),
                    max_nodes: a.budget.max_nodes,
                };
                let bytes = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
                let input_hash = hash(&bytes);
                let artifact = self.artifact("input.json", &bytes)?;
                let mut limits = crate::jobs::Limits::default();
                limits.max_nodes = a.budget.max_nodes;
                limits.max_cells = a.budget.max_cells;
                limits.max_iterations = a.budget.max_iterations;
                limits.time_seconds = a.budget.wall_seconds;
                limits.max_output_bytes = a.budget.max_output_bytes as u64;
                let process = ProcessJob::start_with_limits(request, limits)?;
                if let Some((used, _)) = self.owner_limits.get_mut(&self.call_owner) {
                    *used += 1;
                }
                let job_id = id("job", &a.operation_id);
                let mesh_id = id("mesh", &a.operation_id);
                let result = json!({"job_id":job_id,"mesh_id":mesh_id,"state":"running","owner":self.call_owner,"input_hash":input_hash,"input_artifact":artifact,"project_id":a.project_id,"revision":a.revision,"pid":process.pid(),"budget":a.budget});
                self.running.insert(
                    job_id.clone(),
                    Running {
                        process,
                        snapshot,
                        budget: a.budget,
                        input_hash,
                    },
                );
                self.state.jobs.insert(job_id, result.clone());
                Ok(result)
            }
            "read_job" => {
                let a: JobRef = parse(args)?;
                self.state
                    .jobs
                    .get(&a.job_id)
                    .cloned()
                    .ok_or_else(|| "NOT_FOUND: job".into())
            }
            "cancel_job" => {
                let a: JobRef = parse(args)?;
                if let Some(mut job) = self.running.remove(&a.job_id) {
                    job.process.cancel()?;
                    let state = self.state.jobs.get_mut(&a.job_id).unwrap();
                    state["state"] = json!("cancelled");
                    state["stop_reason"] = json!("cancelled");
                    state["process_exited"] = json!(true);
                }
                self.state
                    .jobs
                    .get(&a.job_id)
                    .cloned()
                    .ok_or_else(|| "NOT_FOUND: job".into())
            }
            "inspect_mesh" => {
                let a: MeshRef = parse(args)?;
                let m = self.state.meshes.get(&a.mesh_id).ok_or("NOT_FOUND: mesh")?;
                Ok(
                    json!({"mesh_id":m.mesh_id,"job_id":m.job_id,"project_id":m.project_id,"revision":m.revision,"input_hash":m.input_hash,"report":m.report,"artifact":m.artifact}),
                )
            }
            "compare_meshes" => {
                let a: Compare = parse(args)?;
                let first = self
                    .state
                    .meshes
                    .get(&a.mesh_ids[0])
                    .ok_or("NOT_FOUND: first mesh")?;
                let second = self
                    .state
                    .meshes
                    .get(&a.mesh_ids[1])
                    .ok_or("NOT_FOUND: second mesh")?;
                Ok(
                    json!({"mesh_ids":a.mesh_ids,"reports":[first.report,second.report],"same_project":first.project_id==second.project_id,"same_revision":first.revision==second.revision,"same_input":first.input_hash==second.input_hash,"node_delta":second.mesh.nodes.len() as i64-first.mesh.nodes.len() as i64,"cell_delta":second.mesh.triangles.len() as i64-first.mesh.triangles.len() as i64,"marker_sets_equal":markers(&first.mesh)==markers(&second.mesh),"scientific_error":"not evaluated by mesh comparison"}),
                )
            }
            "export_mesh" => {
                let a: Export = parse(args)?;
                let m = self.state.meshes.get(&a.mesh_id).ok_or("NOT_FOUND: mesh")?;
                require(
                    m.report["geometry_status"] == "valid",
                    "GEOMETRY_INVALID: normal export requires audited geometry",
                )?;
                require(
                    a.allow_partial || m.mesh.report.complete,
                    "QUALITY_UNMET: read the report, then pass allow_partial to export anyway",
                )?;
                require(
                    !a.filename.is_empty()
                        && a.filename.len() <= 120
                        && a.filename
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
                        && !a.filename.starts_with('.')
                        && a.filename != "workspace.json",
                    "INVALID_PATH: use a simple filename inside exports",
                )?;
                let mut bytes = Vec::new();
                match a.format.as_str() {
                    "msh" => rustmesh::export::write_gmsh(&m.mesh, &mut bytes)
                        .map_err(|e| e.to_string())?,
                    "vtu" => rustmesh::export::write_vtu(&m.mesh, &mut bytes)
                        .map_err(|e| e.to_string())?,
                    "json" => bytes = serde_json::to_vec(&m.mesh).map_err(|e| e.to_string())?,
                    _ => return Err("UNSUPPORTED_FORMAT: msh, vtu or json".into()),
                }
                let exports = self
                    .root
                    .join("exports")
                    .canonicalize()
                    .map_err(|e| e.to_string())?;
                require(
                    exports == self.root.join("exports"),
                    "INVALID_PATH: export directory must not be a link",
                )?;
                let path = exports.join(&a.filename);
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(|e| format!("EXPORT_CONFLICT: {e}"))?;
                use std::io::Write;
                file.write_all(&bytes).map_err(|e| e.to_string())?;
                Ok(
                    json!({"mesh_id":a.mesh_id,"artifact":self.artifact(&a.format,&bytes)?,"filename":a.filename,"path":path,"partial":!m.mesh.report.complete,"marker_semantics":"MSH physical tags use absolute magnitude; signed markers retained in rustmesh NodeData/ElementData"}),
                )
            }
            _ => Err("UNKNOWN_TOOL: unsupported domain tool".into()),
        }
    }
    pub fn poll_jobs(&mut self) {
        let ids: Vec<_> = self.running.keys().cloned().collect();
        let mut changed = false;
        for job_id in ids {
            let outcome = self.running.get_mut(&job_id).unwrap().process.poll();
            let Some(outcome) = outcome else { continue };
            changed = true;
            let running = self.running.remove(&job_id).unwrap();
            let mesh_id = self.state.jobs[&job_id]["mesh_id"]
                .as_str()
                .unwrap()
                .to_string();
            match outcome.mesh {
                Ok(Product::TwoD(mesh)) => {
                    let audit = audit_geometry(&running.snapshot.geometry, &mesh);
                    let valid = audit["valid"] == true;
                    let bytes = serde_json::to_vec(&mesh).unwrap();
                    if mesh.triangles.len() > running.budget.max_cells
                        || bytes.len() > running.budget.max_output_bytes
                    {
                        let job = self.state.jobs.get_mut(&job_id).unwrap();
                        job["state"] = json!("failed");
                        job["stop_reason"] =
                            json!(if mesh.triangles.len() > running.budget.max_cells {
                                "cell_budget"
                            } else {
                                "output_budget"
                            });
                        continue;
                    }
                    let artifact = match self.artifact("mesh.json", &bytes) {
                        Ok(a) => a,
                        Err(e) => {
                            let job = self.state.jobs.get_mut(&job_id).unwrap();
                            job["state"] = json!("failed");
                            job["error"] = json!(e);
                            job["stop_reason"] = json!("backend_failure");
                            continue;
                        }
                    };
                    let mut report = json!({"geometry_status":if valid{"valid"}else{"invalid"},"geometry_audit":audit,"size_goal":if running.snapshot.geometry.sizing.max_area==0.0&&running.snapshot.geometry.sizing.point_size==0.0&&running.snapshot.geometry.zones().iter().all(|z|z.max_area==0.0){"not_requested"}else if mesh.report.area_violations==0{"satisfied"}else{"unsatisfied"},"angle_goal":if running.snapshot.geometry.sizing.min_angle==0.0{"not_requested"}else if mesh.report.angle_violations==0{"satisfied"}else{"unsatisfied"},"stop_reason":if !mesh.report.refinement_complete{"node_budget"}else if mesh.report.complete{"converged"}else{"no_placeable_vertex"},"remaining_size_violations":mesh.report.area_violations,"remaining_angle_violations":mesh.report.angle_violations,"constrained_or_protected_violations":Value::Null,"observed_quality_distribution":{"min_angles":distribution(mesh.cell_min_angles()),"areas":distribution(mesh.cell_areas())},"nodes":mesh.nodes.len(),"cells":mesh.triangles.len(),"markers":markers(&mesh),"seconds":outcome.seconds,"input_hash":running.input_hash,"build_id":env!("RUSTMESH_BUILD_ID"),"effective_options":running.snapshot.geometry.sizing,"budget":running.budget,"legacy":mesh.report});
                    report["stop_reason"] =
                        serde_json::to_value(outcome.status.stop_reason).unwrap();
                    report["runtime"] = serde_json::to_value(&outcome.status).unwrap();
                    match self.artifact("report.json", &serde_json::to_vec(&report).unwrap()) {
                        Ok(artifact) => report["report_artifact"] = artifact,
                        Err(e) => {
                            let job = self.state.jobs.get_mut(&job_id).unwrap();
                            job["state"] = json!("failed");
                            job["error"] = json!(e);
                            continue;
                        }
                    }
                    self.state.meshes.insert(
                        mesh_id.clone(),
                        MeshRecord {
                            mesh_id: mesh_id.clone(),
                            job_id: job_id.clone(),
                            project_id: running.snapshot.project_id,
                            revision: running.snapshot.revision,
                            input_hash: running.input_hash,
                            mesh,
                            report: report.clone(),
                            artifact: artifact.clone(),
                        },
                    );
                    let job = self.state.jobs.get_mut(&job_id).unwrap();
                    job["state"] = json!(if valid { "completed" } else { "failed" });
                    job["report"] = report;
                    job["artifact"] = artifact;
                }
                Ok(Product::ThreeD(_)) => {
                    self.state.jobs.get_mut(&job_id).unwrap()["state"] = json!("failed");
                }
                Err(e) => {
                    let job = self.state.jobs.get_mut(&job_id).unwrap();
                    job["state"] = json!("failed");
                    job["error"] = json!(e);
                    job["stop_reason"] = serde_json::to_value(outcome.status.stop_reason).unwrap();
                    job["report"] = serde_json::to_value(outcome.status).unwrap();
                }
            }
        }
        if changed {
            if let Err(e) = self.save() {
                for job in self.state.jobs.values_mut() {
                    job["persistence_error"] = json!(e);
                }
            }
        }
    }
}

fn validate_geometry(g: &Geometry2D, max_nodes: usize) -> Result<(), String> {
    require(
        (3..=5_000_000).contains(&max_nodes),
        "INVALID_ARGUMENT: max_nodes must be 3..5000000",
    )?;
    require(
        !g.units.trim().is_empty(),
        "INVALID_ARGUMENT: units required",
    )?;
    require(
        g.sizing.min_angle.is_finite() && (0.0..=40.0).contains(&g.sizing.min_angle),
        "INVALID_ARGUMENT: angle must be 0..40",
    )?;
    for v in [g.sizing.max_area, g.sizing.point_size, g.sizing.size_growth] {
        require(
            v.is_finite() && v >= 0.0,
            "INVALID_ARGUMENT: sizing must be finite nonnegative",
        )?;
    }
    for z in g.zones() {
        require(
            z.max_area.is_finite() && z.max_area >= 0.0,
            "INVALID_ARGUMENT: material area must be finite nonnegative",
        )?;
    }
    g.plc()
        .map_err(|e| format!("GEOMETRY_INVALID: {e}"))?
        .plc
        .validate()
        .map_err(|e| format!("GEOMETRY_INVALID: {e}"))
}
fn summary(p: &ProjectSnapshot) -> Value {
    json!({"project_id":p.project_id,"revision":p.revision,"name":p.geometry.name,"units":p.geometry.units,"max_nodes":p.max_nodes,"sizing":p.geometry.sizing,"entities":p.entities,"counts":{"domain_corners":p.geometry.domain.outline.len(),"regions":p.geometry.shapes.len(),"constraints":p.geometry.lines.len(),"points":p.geometry.points.len()},"geometry_hash":hash(&serde_json::to_vec(&p.geometry).unwrap()),"geometry_warnings":p.geometry.plc().map(|m|m.warnings).unwrap_or_default(),"materials":p.geometry.zones().iter().map(|z|json!({"name":z.name,"marker":z.marker,"max_area":z.max_area})).collect::<Vec<_>>()})
}
fn entity_values(g: &Geometry2D) -> Vec<(String, usize, Value)> {
    let mut v = vec![
        ("domain".into(), 0, json!(g.domain)),
        ("background".into(), 0, json!(g.background)),
    ];
    for (i, s) in g.shapes.iter().enumerate() {
        v.push(("region".into(), i, json!(s)))
    }
    for (i, s) in g.lines.iter().enumerate() {
        v.push(("constraint".into(), i, json!(s)))
    }
    for (i, s) in g.points.iter().enumerate() {
        v.push(("electrode".into(), i, json!(s)))
    }
    v
}
fn entities(
    project: &str,
    g: &Geometry2D,
    old: Option<&ProjectSnapshot>,
    seed: &str,
) -> BTreeMap<String, Entity> {
    use std::collections::{BTreeSet, VecDeque};
    let previous = old.map(|o| entity_values(&o.geometry)).unwrap_or_default();
    let next = entity_values(g);
    let mut old_counts = BTreeMap::<String, usize>::new();
    let mut new_counts = BTreeMap::<String, usize>::new();
    let prior_by_index: BTreeMap<_, _> = old
        .map(|o| {
            o.entities
                .iter()
                .map(|(key, e)| ((e.kind.clone(), e.index), key.clone()))
                .collect()
        })
        .unwrap_or_default();
    let mut by_value = BTreeMap::<(String, String), VecDeque<String>>::new();
    for (kind, index, value) in &previous {
        *old_counts.entry(kind.clone()).or_default() += 1;
        if let Some(key) = prior_by_index.get(&(kind.clone(), *index)) {
            by_value
                .entry((kind.clone(), hash(&serde_json::to_vec(value).unwrap())))
                .or_default()
                .push_back(key.clone());
        }
    }
    for (kind, _, _) in &next {
        *new_counts.entry(kind.clone()).or_default() += 1;
    }
    let mut used = BTreeSet::new();
    let mut assignments = BTreeMap::new();
    // Match all unchanged values first so inserted rows cannot steal identities.
    for (kind, index, value) in &next {
        if let Some(key) = by_value
            .get_mut(&(kind.clone(), hash(&serde_json::to_vec(value).unwrap())))
            .and_then(VecDeque::pop_front)
        {
            used.insert(key.clone());
            assignments.insert((kind.clone(), *index), key);
        }
    }
    next.into_iter()
        .map(|(kind, index, _)| {
            let retained = assignments
                .get(&(kind.clone(), index))
                .cloned()
                .or_else(|| {
                    if old_counts.get(&kind) != new_counts.get(&kind) {
                        return None;
                    }
                    prior_by_index
                        .get(&(kind.clone(), index))
                        .filter(|key| !used.contains(*key))
                        .cloned()
                });
            let key = retained
                .unwrap_or_else(|| id("entity", &format!("{project}:{seed}:{kind}:{index}")));
            used.insert(key.clone());
            (key, Entity { kind, index })
        })
        .collect()
}
fn extend_entities(p: &ProjectSnapshot, seed: &str) -> BTreeMap<String, Entity> {
    let mut out = p.entities.clone();
    for (kind, index, _) in entity_values(&p.geometry) {
        if !out.values().any(|e| e.kind == kind && e.index == index) {
            out.insert(
                id("entity", &format!("{}:{seed}:{kind}:{index}", p.project_id)),
                Entity { kind, index },
            );
        }
    }
    out
}
fn edit_geometry(p: &mut ProjectSnapshot, edit: Edit) -> Result<(), String> {
    use rustmesh::{Line, Point, Shape, Zone};
    match edit {
        Edit::SetSizing {
            min_angle,
            max_area,
            point_size,
            size_growth,
        } => {
            if let Some(v) = min_angle {
                p.geometry.sizing.min_angle = v
            }
            if let Some(v) = max_area {
                p.geometry.sizing.max_area = v
            }
            if let Some(v) = point_size {
                p.geometry.sizing.point_size = v
            }
            if let Some(v) = size_growth {
                p.geometry.sizing.size_growth = v
            }
        }
        Edit::SetBudget { max_nodes } => p.max_nodes = max_nodes,
        Edit::AddElectrode { at, marker, refine } => {
            p.geometry.points.push(Point { at, marker, refine })
        }
        Edit::AddRegion {
            name,
            outline,
            material_marker,
            boundary_marker,
            max_area,
        } => p.geometry.shapes.push(Shape {
            zone: Zone::new(&name, material_marker, max_area),
            outline,
            edge_marker: boundary_marker,
            hole: false,
        }),
        Edit::AddHole {
            outline,
            boundary_marker,
        } => p.geometry.shapes.push(Shape {
            zone: Zone::default(),
            outline,
            edge_marker: boundary_marker,
            hole: true,
        }),
        Edit::AddConstraint {
            name,
            points,
            marker,
        } => p.geometry.lines.push(Line {
            name,
            points,
            marker,
        }),
        Edit::RemoveEntity { entity_id } => {
            let e = p
                .entities
                .get(&entity_id)
                .ok_or("NOT_FOUND: entity")?
                .clone();
            match e.kind.as_str() {
                "region" => {
                    p.geometry.shapes.remove(e.index);
                }
                "constraint" => {
                    p.geometry.lines.remove(e.index);
                }
                "electrode" => {
                    p.geometry.points.remove(e.index);
                }
                _ => return Err("INVALID_ARGUMENT: cannot remove domain or background".into()),
            }
            p.entities.remove(&entity_id);
            for other in p.entities.values_mut() {
                if other.kind == e.kind && other.index > e.index {
                    other.index -= 1
                }
            }
        }
        Edit::SetMaterial { entity_id, marker } => {
            let e = p.entities.get(&entity_id).ok_or("NOT_FOUND: entity")?;
            match e.kind.as_str() {
                "region" => p.geometry.shapes[e.index].zone.marker = marker,
                "background" => p.geometry.background.marker = marker,
                _ => return Err("INVALID_ARGUMENT: material requires region or background".into()),
            }
        }
    }
    Ok(())
}
fn distribution(mut values: Vec<f64>) -> Value {
    values.sort_by(f64::total_cmp);
    if values.is_empty() {
        return Value::Null;
    }
    json!({"min":values[0],"p05":values[(values.len()-1)/20],"median":values[(values.len()-1)/2],"p95":values[(values.len()-1)*19/20],"max":values[values.len()-1]})
}
fn markers(m: &Mesh2D) -> Value {
    fn unique(v: &[i32]) -> Vec<i32> {
        v.iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    json!({"nodes":unique(&m.node_markers),"edges":unique(&m.edge_markers),"cells":unique(&m.cell_markers)})
}
fn audit_geometry(g: &Geometry2D, m: &Mesh2D) -> Value {
    let Ok(input) = g.plc() else {
        return json!({"valid":false,"error":"input PLC failed"});
    };
    let plc = input.plc;
    let indices = m
        .triangles
        .iter()
        .all(|t| t.iter().all(|i| *i < m.nodes.len()))
        && m.edges.iter().all(|t| t.iter().all(|i| *i < m.nodes.len()));
    if !indices {
        return json!({"valid":false,"error":"invalid connectivity"});
    }
    let orientation =
        m.cell_areas().iter().all(|a| a.is_finite() && *a > 0.0) && !m.triangles.is_empty();
    let points = m.input_vertex_nodes.len() == plc.vertices.len()
        && m.input_vertex_nodes.iter().enumerate().all(|(i, &n)| {
            n >= 0 && (n as usize) < m.nodes.len() && m.nodes[n as usize] == plc.vertices[i]
        });
    let mut coverage = true;
    let mut marker_valid = true;
    let mut segment_location = true;
    for (i, segment) in plc.segments.iter().enumerate() {
        let a = plc.vertices[segment[0]];
        let b = plc.vertices[segment[1]];
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let len2 = dx * dx + dy * dy;
        let mut intervals = Vec::new();
        for (j, e) in m.edges.iter().enumerate() {
            if m.edge_segments.get(j).copied() == Some(i as i64) {
                let p = m.nodes[e[0]];
                let q = m.nodes[e[1]];
                let t = |v: [f64; 2]| ((v[0] - a[0]) * dx + (v[1] - a[1]) * dy) / len2;
                let (u, v) = (t(p), t(q));
                for r in [p, q] {
                    let cross = (r[0] - a[0]) * dy - (r[1] - a[1]) * dx;
                    if cross.abs() > len2 * 1e-8 {
                        segment_location = false
                    }
                }
                if u < -1e-8 || v < -1e-8 || u > 1.0 + 1e-8 || v > 1.0 + 1e-8 {
                    segment_location = false
                }
                intervals.push((u.min(v), u.max(v)));
                if m.edge_markers[j] != plc.segment_marker(i) {
                    marker_valid = false
                }
            }
        }
        intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut end = 0.0;
        for (lo, hi) in intervals {
            if lo > end + 1e-8 {
                coverage = false
            }
            end = end.max(hi)
        }
        if end < 1.0 - 1e-8 {
            coverage = false
        }
    }
    let node_markers = m
        .input_vertex_nodes
        .iter()
        .enumerate()
        .all(|(i, &n)| n < 0 || m.node_markers.get(n as usize) == Some(&plc.vertex_marker(i)));
    let centroids = m.triangles.iter().all(|t| {
        let a = m.nodes[t[0]];
        let b = m.nodes[t[1]];
        let c = m.nodes[t[2]];
        let p = [
            a[0] + ((b[0] - a[0]) + (c[0] - a[0])) / 3.0,
            a[1] + ((b[1] - a[1]) + (c[1] - a[1])) / 3.0,
        ];
        inside_polygon(&g.domain.outline, p)
            && !g
                .shapes
                .iter()
                .filter(|s| s.hole)
                .any(|s| inside_polygon(&s.outline, p))
    });
    let expected_area = polygon_area(&g.domain.outline).abs()
        - g.shapes
            .iter()
            .filter(|s| s.hole)
            .map(|s| polygon_area(&s.outline).abs())
            .sum::<f64>();
    let measured_area = m.cell_areas().iter().sum::<f64>();
    let area_matches =
        expected_area > 0.0 && (measured_area - expected_area).abs() <= expected_area * 1e-8;
    json!({"valid":indices&&orientation&&points&&coverage&&marker_valid&&node_markers&&centroids&&area_matches&&segment_location,"checks":{"connectivity":indices,"positive_area":orientation,"input_vertices_exact":points,"segment_ancestry_coverage":coverage,"segment_markers":marker_valid,"input_node_markers":node_markers,"cell_centroids_in_domain":centroids,"domain_area":area_matches,"segment_location":segment_location},"expected_area":expected_area,"measured_area":measured_area,"global_overlap":"not independently checked","domain_coverage":"centroids plus total area; overlapping or clipped holes conservatively rejected"})
}

fn polygon_area(points: &[[f64; 2]]) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }
    let o = points[0];
    (1..points.len() - 1)
        .map(|i| {
            let a = points[i];
            let b = points[i + 1];
            0.5 * ((a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]))
        })
        .sum()
}
fn inside_polygon(points: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut inside = false;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] - a[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1])
        {
            inside = !inside
        }
    }
    inside
}
