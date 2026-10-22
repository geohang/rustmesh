//! Versioned domain commands shared by local clients and MCP.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const VERSION: u32 = 1;
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Empty {
    pub schema_version: u32,
}
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Create {
    pub schema_version: u32,
    pub operation_id: String,
    pub dimension: u32,
    pub template: String,
    #[serde(default)]
    pub parameters: std::collections::BTreeMap<String, f64>,
    #[serde(default = "default_nodes")]
    pub max_nodes: usize,
}
pub fn default_nodes() -> usize {
    100_000
}
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadProject {
    pub schema_version: u32,
    pub project_id: String,
    pub revision: Option<u64>,
}
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    SetSizing {
        min_angle: Option<f64>,
        max_area: Option<f64>,
        point_size: Option<f64>,
        size_growth: Option<f64>,
    },
    SetBudget {
        max_nodes: usize,
    },
    AddElectrode {
        at: [f64; 2],
        marker: i32,
        refine: bool,
    },
    AddRegion {
        name: String,
        outline: Vec<[f64; 2]>,
        material_marker: i32,
        boundary_marker: i32,
        max_area: f64,
    },
    AddHole {
        outline: Vec<[f64; 2]>,
        boundary_marker: i32,
    },
    AddConstraint {
        name: String,
        points: Vec<[f64; 2]>,
        marker: i32,
    },
    RemoveEntity {
        entity_id: String,
    },
    SetMaterial {
        entity_id: String,
        marker: i32,
    },
}
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Preview {
    pub schema_version: u32,
    pub operation_id: String,
    pub project_id: String,
    pub base_revision: u64,
    pub operations: Vec<Edit>,
}
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Apply {
    pub schema_version: u32,
    pub operation_id: String,
    pub patch_id: String,
    pub expected_revision: u64,
}
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Restore {
    pub schema_version: u32,
    pub operation_id: String,
    pub project_id: String,
    pub target_revision: u64,
    pub expected_revision: u64,
}
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    pub max_nodes: usize,
    #[serde(default = "default_iterations")]
    pub max_iterations: usize,
    pub max_cells: usize,
    pub wall_seconds: f64,
    pub max_output_bytes: usize,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            max_nodes: 100_000,
            max_iterations: default_iterations(),
            max_cells: 200_000,
            wall_seconds: 120.0,
            max_output_bytes: 64 * 1024 * 1024,
        }
    }
}
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Generate {
    pub schema_version: u32,
    pub operation_id: String,
    pub project_id: String,
    pub revision: u64,
    pub backend: String,
    #[serde(default)]
    pub budget: Budget,
}
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobRef {
    pub schema_version: u32,
    pub job_id: String,
}
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshRef {
    pub schema_version: u32,
    pub mesh_id: String,
}
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Compare {
    pub schema_version: u32,
    pub mesh_ids: [String; 2],
}
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Export {
    pub schema_version: u32,
    pub operation_id: String,
    pub mesh_id: String,
    pub format: String,
    pub filename: String,
    #[serde(default)]
    pub allow_partial: bool,
}

fn definition<T: JsonSchema>(name: &str, description: &str, read_only: bool) -> Value {
    json!({"name":name,"description":description,"inputSchema":schemars::schema_for!(T),
        "outputSchema":output_schema(name),
        "annotations":{"readOnlyHint":read_only,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}})
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn array(items: Value) -> Value {
    json!({"type":"array","items":items})
}
fn nullable(schema: Value) -> Value {
    json!({"anyOf":[schema,{"type":"null"}]})
}
fn artifact_schema() -> Value {
    object(
        json!({"artifact_id":{"type":"string"},"sha256":{"type":"string","pattern":"^[a-f0-9]{64}$"},"bytes":{"type":"integer","minimum":0},"path":{"type":"string"}}),
        &["artifact_id", "sha256", "bytes", "path"],
    )
}
fn sizing_schema() -> Value {
    object(
        json!({"min_angle":{"type":"number"},"max_area":{"type":"number"},"point_size":{"type":"number"},"size_growth":{"type":"number"}}),
        &["min_angle", "max_area", "point_size", "size_growth"],
    )
}
fn scientific_request_schema() -> Value {
    let point = json!({"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2});
    let zone = object(
        json!({"name":{"type":"string"},"marker":{"type":"integer"},"max_area":{"type":"number"}}),
        &["name", "marker", "max_area"],
    );
    let geometry = object(
        json!({"name":{"type":"string"},"units":{"type":"string"},"domain":object(json!({"outline":array(point.clone()),"edge_markers":array(json!({"type":"integer"}))}),&["outline","edge_markers"]),"background":zone.clone(),"shapes":array(object(json!({"zone":zone,"outline":array(point.clone()),"edge_marker":{"type":"integer"},"hole":{"type":"boolean"}}),&["zone","outline","edge_marker","hole"])),"lines":array(object(json!({"name":{"type":"string"},"points":array(point.clone()),"marker":{"type":"integer"}}),&["name","points","marker"])),"points":array(object(json!({"at":point,"marker":{"type":"integer"},"refine":{"type":"boolean"}}),&["at","marker","refine"])),"sizing":sizing_schema()}),
        &[
            "name",
            "units",
            "domain",
            "background",
            "shapes",
            "lines",
            "points",
            "sizing",
        ],
    );
    object(
        json!({"dimension":{"const":"TwoD"},"geometry":geometry,"max_nodes":{"type":"integer"}}),
        &["dimension", "geometry", "max_nodes"],
    )
}
fn summary_schema() -> Value {
    object(
        json!({"project_id":{"type":"string"},"revision":{"type":"integer","minimum":1},"name":{"type":"string"},"units":{"type":"string"},"max_nodes":{"type":"integer"},"sizing":sizing_schema(),
        "entities":{"type":"object","additionalProperties":object(json!({"kind":{"enum":["domain","background","region","constraint","electrode"]},"index":{"type":"integer","minimum":0}}),&["kind","index"])},
        "counts":object(json!({"domain_corners":{"type":"integer"},"regions":{"type":"integer"},"constraints":{"type":"integer"},"points":{"type":"integer"}}),&["domain_corners","regions","constraints","points"]),
        "geometry_hash":{"type":"string"},"geometry_warnings":array(json!({"type":"string"})),"materials":array(object(json!({"name":{"type":"string"},"marker":{"type":"integer"},"max_area":{"type":"number"}}),&["name","marker","max_area"]))}),
        &[
            "project_id",
            "revision",
            "name",
            "units",
            "max_nodes",
            "sizing",
            "entities",
            "counts",
            "geometry_hash",
            "geometry_warnings",
            "materials",
        ],
    )
}
fn reason_schema() -> Value {
    json!({"enum":["converged","node_budget","cell_budget","pass_budget","time_budget","memory_limit","cancelled","stalled","no_placeable_vertex","geometry_recovery_failed","backend_failure","output_budget"]})
}
fn goal_schema() -> Value {
    json!({"enum":["satisfied","unsatisfied","not_requested","not_evaluated"]})
}
fn geometry_schema() -> Value {
    json!({"enum":["valid","invalid","not_checked"]})
}
fn runtime_schema() -> Value {
    object(
        json!({"geometry_status":geometry_schema(),"size_goal":goal_schema(),"angle_goal":goal_schema(),"stop_reason":reason_schema(),
        "remaining_size_violations":nullable(json!({"type":"integer","minimum":0})),"remaining_angle_violations":nullable(json!({"type":"integer","minimum":0})),"constrained_or_protected_violations":nullable(json!({"type":"integer","minimum":0})),
        "observed_quality_distribution":nullable(object(json!({"minimum_angle":{"type":"number"},"p05":nullable(json!({"type":"number"})),"median":nullable(json!({"type":"number"})),"p95":nullable(json!({"type":"number"}))}),&["minimum_angle","p05","median","p95"])),
        "input_hash":nullable(json!({"type":"string"})),"build_id":{"type":"string"},
        "effective_options":nullable(object(json!({"request":scientific_request_schema(),"limits":object(json!({"max_nodes":{"type":"integer"},"max_cells":{"type":"integer"},"max_iterations":{"type":"integer"},"time_seconds":{"type":"number"},"max_output_bytes":{"type":"integer"}}),&["max_nodes","max_cells","max_iterations","time_seconds","max_output_bytes"])}),&["request","limits"])),
        "resource_usage":nullable(object(json!({"wall_seconds":{"type":"number"},"iterations":{"type":"integer"},"memory_enforcement":{"type":"string"},"cell_budget_scope":{"type":"string"}}),&["wall_seconds","iterations","memory_enforcement","cell_budget_scope"])),"geometry_audit":array(json!({"type":"string"}))}),
        &[
            "geometry_status",
            "size_goal",
            "angle_goal",
            "stop_reason",
            "remaining_size_violations",
            "remaining_angle_violations",
            "constrained_or_protected_violations",
            "observed_quality_distribution",
            "input_hash",
            "build_id",
            "effective_options",
            "resource_usage",
            "geometry_audit",
        ],
    )
}
fn report_schema() -> Value {
    let distribution = nullable(object(
        json!({"min":{"type":"number"},"p05":{"type":"number"},"median":{"type":"number"},"p95":{"type":"number"},"max":{"type":"number"}}),
        &["min", "p05", "median", "p95", "max"],
    ));
    let legacy = object(
        json!({"complete":{"type":"boolean"},"refinement_complete":{"type":"boolean"},"quality_met":{"type":"boolean"},"issues":array(json!({"type":"string"})),"warnings":array(json!({"type":"string"})),"min_angle":{"type":"number"},"angle_violations":{"type":"integer"},"area_violations":{"type":"integer"},"refine_calls":{"type":"integer"},"vertices_outside":{"type":"integer"}}),
        &[
            "complete",
            "refinement_complete",
            "quality_met",
            "issues",
            "warnings",
            "min_angle",
            "angle_violations",
            "area_violations",
            "refine_calls",
            "vertices_outside",
        ],
    );
    let checks = object(
        json!({"connectivity":{"type":"boolean"},"positive_area":{"type":"boolean"},"input_vertices_exact":{"type":"boolean"},"segment_ancestry_coverage":{"type":"boolean"},"segment_markers":{"type":"boolean"},"input_node_markers":{"type":"boolean"},"cell_centroids_in_domain":{"type":"boolean"},"domain_area":{"type":"boolean"},"segment_location":{"type":"boolean"}}),
        &[
            "connectivity",
            "positive_area",
            "input_vertices_exact",
            "segment_ancestry_coverage",
            "segment_markers",
            "input_node_markers",
            "cell_centroids_in_domain",
            "domain_area",
            "segment_location",
        ],
    );
    let audit = object(
        json!({"valid":{"type":"boolean"},"error":{"type":"string"},"checks":checks,"expected_area":{"type":"number"},"measured_area":{"type":"number"},"global_overlap":{"type":"string"},"domain_coverage":{"type":"string"}}),
        &["valid"],
    );
    object(
        json!({"geometry_status":geometry_schema(),"geometry_audit":audit,"size_goal":goal_schema(),"angle_goal":goal_schema(),"stop_reason":reason_schema(),"remaining_size_violations":{"type":"integer"},"remaining_angle_violations":{"type":"integer"},"constrained_or_protected_violations":{"type":"null"},
        "observed_quality_distribution":object(json!({"min_angles":distribution,"areas":distribution}),&["min_angles","areas"]),"nodes":{"type":"integer"},"cells":{"type":"integer"},"markers":object(json!({"nodes":array(json!({"type":"integer"})),"edges":array(json!({"type":"integer"})),"cells":array(json!({"type":"integer"}))}),&["nodes","edges","cells"]),"seconds":{"type":"number"},"input_hash":{"type":"string"},"build_id":{"type":"string"},"effective_options":sizing_schema(),"budget":schemars::schema_for!(Budget),"legacy":legacy,"runtime":runtime_schema(),"report_artifact":artifact_schema()}),
        &[
            "geometry_status",
            "geometry_audit",
            "size_goal",
            "angle_goal",
            "stop_reason",
            "remaining_size_violations",
            "remaining_angle_violations",
            "constrained_or_protected_violations",
            "observed_quality_distribution",
            "nodes",
            "cells",
            "markers",
            "seconds",
            "input_hash",
            "build_id",
            "effective_options",
            "budget",
            "legacy",
            "runtime",
            "report_artifact",
        ],
    )
}
fn job_schema() -> Value {
    object(
        json!({"job_id":{"type":"string"},"mesh_id":{"type":"string"},"state":{"enum":["running","completed","failed","cancelled","interrupted"]},"owner":{"type":"string"},"input_hash":{"type":"string"},"input_artifact":artifact_schema(),"project_id":{"type":"string"},"revision":{"type":"integer"},"pid":{"type":"integer"},"budget":schemars::schema_for!(Budget),"stop_reason":reason_schema(),"process_exited":{"type":"boolean"},"error":{"type":"string"},"report":{"anyOf":[report_schema(),runtime_schema()]},"artifact":artifact_schema(),"persistence_error":{"type":"string"}}),
        &[
            "job_id",
            "mesh_id",
            "state",
            "owner",
            "input_hash",
            "input_artifact",
            "project_id",
            "revision",
            "pid",
            "budget",
        ],
    )
}
pub fn output_schema(name: &str) -> Value {
    let result = match name {
        "create_project" | "read_project" | "apply_project_patch" | "restore_project_revision" => {
            summary_schema()
        }
        "preview_project_patch" => object(
            json!({"patch_id":{"type":"string"},"base_revision":{"type":"integer"},"validation":{"const":"valid"},"candidate":summary_schema()}),
            &["patch_id", "base_revision", "validation", "candidate"],
        ),
        "generate_mesh" | "read_job" | "cancel_job" => job_schema(),
        "inspect_mesh" => object(
            json!({"mesh_id":{"type":"string"},"job_id":{"type":"string"},"project_id":{"type":"string"},"revision":{"type":"integer"},"input_hash":{"type":"string"},"report":report_schema(),"artifact":artifact_schema()}),
            &[
                "mesh_id",
                "job_id",
                "project_id",
                "revision",
                "input_hash",
                "report",
                "artifact",
            ],
        ),
        "compare_meshes" => object(
            json!({"mesh_ids":{"type":"array","items":{"type":"string"},"minItems":2,"maxItems":2},"reports":{"type":"array","items":report_schema(),"minItems":2,"maxItems":2},"same_project":{"type":"boolean"},"same_revision":{"type":"boolean"},"same_input":{"type":"boolean"},"node_delta":{"type":"integer"},"cell_delta":{"type":"integer"},"marker_sets_equal":{"type":"boolean"},"scientific_error":{"type":"string"}}),
            &[
                "mesh_ids",
                "reports",
                "same_project",
                "same_revision",
                "same_input",
                "node_delta",
                "cell_delta",
                "marker_sets_equal",
                "scientific_error",
            ],
        ),
        "export_mesh" => object(
            json!({"mesh_id":{"type":"string"},"artifact":artifact_schema(),"filename":{"type":"string"},"path":{"type":"string"},"partial":{"type":"boolean"},"marker_semantics":{"type":"string"}}),
            &[
                "mesh_id",
                "artifact",
                "filename",
                "path",
                "partial",
                "marker_semantics",
            ],
        ),
        "get_capabilities" => object(
            json!({"dimensions":array(json!({"type":"integer"})),"experimental_dimensions":array(json!({"type":"integer"})),"domain_3d":{"type":"boolean"},"templates":array(json!({"type":"string"})),"formats":array(json!({"type":"string"})),"backend":{"type":"string"},"schema_version":{"const":1},"max_nodes":{"type":"integer"},"max_concurrent_jobs":{"type":"integer"},"owner_candidate_quota":nullable(object(json!({"used":{"type":"integer"},"limit":{"type":"integer"}}),&["used","limit"])),"budgets":object(json!({"nodes":{"type":"string"},"cells":{"type":"string"},"wall_time":{"type":"string"},"output_bytes":{"type":"string"},"memory":{"type":"string"},"iterations":{"type":"string"}}),&["nodes","cells","wall_time","output_bytes","memory","iterations"]),"auth":{"type":"string"},"export_directory":{"type":"string"}}),
            &[
                "dimensions",
                "experimental_dimensions",
                "domain_3d",
                "templates",
                "formats",
                "backend",
                "schema_version",
                "max_nodes",
                "max_concurrent_jobs",
                "owner_candidate_quota",
                "budgets",
                "auth",
                "export_directory",
            ],
        ),
        _ => object(json!({}), &[]),
    };
    let success = object(
        json!({"schema_version":{"const":VERSION},"ok":{"const":true},"result":result}),
        &["schema_version", "ok", "result"],
    );
    let failure = object(
        json!({"schema_version":{"const":VERSION},"ok":{"const":false},"error":object(json!({"code":{"type":"string"},"message":{"type":"string"}}),&["code","message"])}),
        &["schema_version", "ok", "error"],
    );
    json!({"type":"object","oneOf":[success,failure]})
}
pub fn definitions() -> Vec<Value> {
    vec![
        definition::<Empty>(
            "get_capabilities",
            "Supported dimensions, templates, exports and resource limits",
            true,
        ),
        definition::<Create>(
            "create_project",
            "Create a validated 2D project from a registered template",
            false,
        ),
        definition::<ReadProject>(
            "read_project",
            "Read project geometry summary and stable entity identities",
            true,
        ),
        definition::<Preview>(
            "preview_project_patch",
            "Validate domain edits without changing the current project",
            false,
        ),
        definition::<Apply>(
            "apply_project_patch",
            "Atomically apply a validated patch at its expected revision",
            false,
        ),
        definition::<Restore>(
            "restore_project_revision",
            "Undo by restoring a snapshot as a new revision",
            false,
        ),
        definition::<Generate>(
            "generate_mesh",
            "Start a mesh job in a separate worker process for a fixed project revision",
            false,
        ),
        definition::<JobRef>(
            "read_job",
            "Poll a job's state and its result artifact references",
            true,
        ),
        definition::<JobRef>(
            "cancel_job",
            "Terminate a running mesh process and wait for it to exit",
            false,
        ),
        definition::<MeshRef>(
            "inspect_mesh",
            "Read measured quality and geometry audit, never full arrays",
            true,
        ),
        definition::<Compare>(
            "compare_meshes",
            "Compare two measured meshes and their input configurations",
            true,
        ),
        definition::<Export>(
            "export_mesh",
            "Export an audited mesh into the authorized workspace exports directory",
            false,
        ),
    ]
}

pub fn default_iterations() -> usize {
    5_000_000
}

/// Normalize optional/defaulted fields before computing operation identity.
pub fn canonical_arguments(name: &str, args: Value) -> Result<Value, String> {
    fn convert<T: serde::de::DeserializeOwned + Serialize>(args: Value) -> Result<Value, String> {
        serde_json::from_value::<T>(args)
            .and_then(serde_json::to_value)
            .map_err(|e| e.to_string())
    }
    match name {
        "create_project" => {
            let mut a: Create = serde_json::from_value(args).map_err(|e| e.to_string())?;
            if let Some(template) = rustmesh::examples::Example::from_key(&a.template) {
                for p in template.params() {
                    a.parameters.entry(p.key.to_owned()).or_insert(p.default);
                }
            }
            serde_json::to_value(a).map_err(|e| e.to_string())
        }
        "preview_project_patch" => convert::<Preview>(args),
        "apply_project_patch" => convert::<Apply>(args),
        "restore_project_revision" => convert::<Restore>(args),
        "generate_mesh" => convert::<Generate>(args),
        "export_mesh" => convert::<Export>(args),
        _ => Ok(args),
    }
}
