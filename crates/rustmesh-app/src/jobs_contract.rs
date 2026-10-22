use super::{Outcome, Product, Request};
use rustmesh::control::StopReason;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GeometryStatus {
    Valid,
    Invalid,
    NotChecked,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Satisfied,
    Unsatisfied,
    NotRequested,
    NotEvaluated,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshOutcome {
    pub geometry_status: GeometryStatus,
    pub size_goal: GoalStatus,
    pub angle_goal: GoalStatus,
    pub stop_reason: StopReason,
    pub remaining_size_violations: Option<usize>,
    pub remaining_angle_violations: Option<usize>,
    pub constrained_or_protected_violations: Option<usize>,
    pub observed_quality_distribution: serde_json::Value,
    pub input_hash: Option<String>,
    pub build_id: String,
    pub effective_options: serde_json::Value,
    pub resource_usage: serde_json::Value,
    pub geometry_audit: Vec<String>,
}
impl Default for MeshOutcome {
    fn default() -> Self {
        Self {
            geometry_status: GeometryStatus::NotChecked,
            size_goal: GoalStatus::NotEvaluated,
            angle_goal: GoalStatus::NotEvaluated,
            stop_reason: StopReason::BackendFailure,
            remaining_size_violations: None,
            remaining_angle_violations: None,
            constrained_or_protected_violations: None,
            observed_quality_distribution: serde_json::Value::Null,
            input_hash: None,
            build_id: env!("RUSTMESH_BUILD_ID").to_string(),
            effective_options: serde_json::Value::Null,
            resource_usage: serde_json::Value::Null,
            geometry_audit: Vec::new(),
        }
    }
}
pub fn issue_reason(issues: &[String], complete: bool) -> StopReason {
    if complete {
        return StopReason::Converged;
    }
    let text = issues.join(" ");
    if text.contains("vertex budget") || text.contains("max_vertices") {
        StopReason::NodeBudget
    } else if text.contains("boundary recovery") || text.contains("facet recovery") {
        StopReason::GeometryRecoveryFailed
    } else if text.contains("pass budget") || text.contains("max_passes") {
        StopReason::PassBudget
    } else if text.contains("no further vertex") {
        StopReason::NoPlaceableVertex
    } else {
        StopReason::Stalled
    }
}
fn goal(requested: bool, remaining: usize) -> GoalStatus {
    if !requested {
        GoalStatus::NotRequested
    } else if remaining == 0 {
        GoalStatus::Satisfied
    } else {
        GoalStatus::Unsatisfied
    }
}

/// Independent output checks, in addition to the mesher's own recovery checks.
/// Every input constraint must be covered without a gap. Invalid output is never
/// returned as a mesh, including when a caller allows partial quality.
fn audit_2d(plc: &rustmesh::Plc, mesh: &rustmesh::Mesh2D) -> Vec<String> {
    use std::collections::BTreeMap;
    let mut errors = Vec::new();
    if mesh.triangles.is_empty() {
        errors.push("empty triangle complex".into());
    }
    if mesh.nodes.iter().flatten().any(|x| !x.is_finite()) {
        errors.push("nonfinite nodes".into());
    }
    if mesh
        .triangles
        .iter()
        .flatten()
        .any(|&i| i >= mesh.nodes.len())
    {
        return vec!["triangle node index out of bounds".into()];
    }
    if mesh.edges.iter().flatten().any(|&i| i >= mesh.nodes.len()) {
        return vec!["constraint node index out of bounds".into()];
    }
    if mesh
        .cell_areas()
        .iter()
        .any(|&a| !a.is_finite() || a <= 0.0)
    {
        errors.push("nonpositive triangle area".into());
    }
    let mut incidence = BTreeMap::new();
    for t in &mesh.triangles {
        for k in 0..3 {
            let mut e = [t[k], t[(k + 1) % 3]];
            e.sort();
            *incidence.entry(e).or_insert(0usize) += 1;
        }
    }
    let edges: std::collections::BTreeSet<_> = mesh
        .edges
        .iter()
        .map(|e| {
            let mut e = *e;
            e.sort();
            e
        })
        .collect();
    if incidence
        .iter()
        .any(|(e, &n)| n > 2 || (n == 1 && !edges.contains(e)))
    {
        errors.push("nonmanifold or unconstrained boundary edge".into());
    }
    for (i, segment) in plc.segments.iter().enumerate() {
        let [a, b] = segment.map(|v| plc.vertices[v]);
        let d = [b[0] - a[0], b[1] - a[1]];
        let length2 = d[0] * d[0] + d[1] * d[1];
        let mut intervals = Vec::new();
        for (edge, &parent) in mesh.edges.iter().zip(&mesh.edge_segments) {
            if parent != i as i64 {
                continue;
            }
            let mut interval = edge.map(|v| {
                let p = mesh.nodes[v];
                ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / length2
            });
            interval.sort_by(f64::total_cmp);
            intervals.push(interval);
        }
        intervals.sort_by(|a, b| a[0].total_cmp(&b[0]));
        let mut end = 0.0;
        for [lo, hi] in intervals {
            if lo > end + 1e-8 {
                break;
            }
            end = end.max(hi);
        }
        if end < 1.0 - 1e-8 {
            errors.push(format!("input segment {i} is not fully recovered"));
        }
    }
    errors
}

pub fn assess(request: &Request, outcome: &mut Outcome) {
    match (&outcome.mesh, request) {
        (Ok(Product::TwoD(mesh)), Request::TwoD { geometry, .. }) => {
            let report = &mesh.report;
            let plc = match geometry.plc() {
                Ok(p) => p.plc,
                Err(e) => {
                    outcome.status.geometry_audit.push(e.to_string());
                    return;
                }
            };
            outcome.status.geometry_audit = audit_2d(&plc, mesh);
            outcome.status.geometry_status = if outcome.status.geometry_audit.is_empty() {
                GeometryStatus::Valid
            } else {
                GeometryStatus::Invalid
            };
            outcome.status.size_goal = goal(
                geometry.sizing.max_area > 0.0
                    || geometry.sizing.point_size > 0.0
                    || plc.regions.iter().any(|r| r.max_area > 0.0),
                report.area_violations,
            );
            outcome.status.angle_goal =
                goal(geometry.sizing.min_angle > 0.0, report.angle_violations);
            outcome.status.remaining_size_violations = Some(report.area_violations);
            outcome.status.remaining_angle_violations = Some(report.angle_violations);
            outcome.status.stop_reason = issue_reason(&report.issues, report.complete);
            let mut angles = mesh.cell_min_angles();
            angles.sort_by(f64::total_cmp);
            let q = |p: f64| {
                angles
                    .get(((angles.len().saturating_sub(1)) as f64 * p) as usize)
                    .copied()
            };
            outcome.status.observed_quality_distribution = serde_json::json!({"minimum_angle":report.min_angle,"p05":q(0.05),"median":q(0.5),"p95":q(0.95)});
        }
        (Ok(Product::ThreeD(mesh)), Request::ThreeD { options, .. }) => {
            let report = &mesh.report;
            // tetrahedralize checks robust positive orientations, manifold
            // neighbors, overlap and complete recovered facet areas before Ok.
            let valid = !mesh.tetrahedra.is_empty()
                && report.min_volume > 0.0
                && report.min_volume.is_finite();
            outcome.status.geometry_status = if valid {
                GeometryStatus::Valid
            } else {
                GeometryStatus::Invalid
            };
            if !valid {
                outcome
                    .status
                    .geometry_audit
                    .push("empty complex or nonpositive volume".into());
            }
            outcome.status.size_goal = goal(
                options.max_volume > 0.0
                    || match request {
                        Request::ThreeD { plc, .. } => {
                            plc.regions.iter().any(|r| r.max_volume > 0.0)
                        }
                        _ => false,
                    },
                report.volume_violations,
            );
            outcome.status.angle_goal =
                goal(options.min_dihedral > 0.0, report.dihedral_violations);
            outcome.status.remaining_size_violations = Some(report.volume_violations);
            outcome.status.remaining_angle_violations = Some(report.dihedral_violations);
            outcome.status.constrained_or_protected_violations = Some(report.forced_dihedrals);
            outcome.status.stop_reason = issue_reason(&report.issues, report.complete);
            outcome.status.observed_quality_distribution = serde_json::json!({"minimum_dihedral":report.min_dihedral,"p05":report.min_dihedral_p05,"median":report.min_dihedral_median,"p95":report.min_dihedral_p95,"sliver_count":report.sliver_count,"minimum_volume":report.min_volume});
        }
        _ => {}
    }
    if outcome.status.geometry_status == GeometryStatus::Invalid {
        outcome.status.stop_reason = StopReason::GeometryRecoveryFailed;
        outcome.mesh = Err(format!(
            "output geometry audit failed: {}",
            outcome.status.geometry_audit.join("; ")
        ));
    }
}
