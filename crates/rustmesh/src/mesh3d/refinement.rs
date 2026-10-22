//! Bounded constrained Bowyer-Watson insertion on a recovered planar complex.
use super::local_refinement::State;
use super::*;

/// Experimental quality refinement. Sizing3D supplies only its spatial field;
/// max_splits/flip_passes belong to the older bisection API and are unused here.
#[derive(Debug, Clone)]
pub struct Quality3D {
    pub max_radius_edge: f64,
    /// Accepted local insertions after initial PLC recovery, including boundary repair.
    pub max_insertions: usize,
}
impl Default for Quality3D {
    fn default() -> Self {
        Self {
            max_radius_edge: 2.,
            max_insertions: 2000,
        }
    }
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct QualityReport3D {
    pub max_radius_edge: f64,
    pub radius_edge_violations: usize,
    pub segment_insertions: usize,
    pub facet_insertions: usize,
    pub volume_insertions: usize,
    pub rejected_transactions: usize,
    pub encroached_segments: usize,
    pub encroached_faces: usize,
    pub max_cavity_cells: usize,
    pub stop_reason: String,
}

/// Initial global PLC recovery, followed by segment/facet-first local conflict
/// cavities. Original points, physical surfaces, labels and coverage are kept.
/// Inspect report.complete: acute features or finite budgets can prevent success.
pub fn tetrahedralize_quality(
    plc: &Plc3D,
    options: &Options3D,
    sizing: &Sizing3D,
    quality: &Quality3D,
    protected_edges: &[[usize; 2]],
) -> Result<PatchedMesh3D, MeshError> {
    if !quality.max_radius_edge.is_finite() || quality.max_radius_edge < 0.6123724356957945 {
        return Err(bad(
            "radius-edge target must be finite and at least sqrt(6)/4",
        ));
    }
    surface::generate(plc, options, sizing, protected_edges, Some(quality))
}

fn on_segment(p: Point, a: Point, b: Point) -> bool {
    collinear([a, b, p]) && (0..3).all(|k| p[k] >= a[k].min(b[k]) && p[k] <= a[k].max(b[k]))
}
fn inside(p: Point, t: [Point; 4]) -> bool {
    (0..4).all(|k| {
        let mut q = t;
        q[k] = p;
        orient(q[0], q[1], q[2], q[3]) >= 0.
    })
}
fn on_triangle(p: Point, t: [Point; 3]) -> bool {
    if orient(t[0], t[1], t[2], p) != 0. {
        return false;
    }
    let normal = cross(sub(t[1], t[0]), sub(t[2], t[0]));
    let k = (0..3)
        .max_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()))
        .unwrap();
    let coord = |p: Point| Coord {
        x: p[(k + 1) % 3],
        y: p[(k + 2) % 3],
    };
    let turns: Vec<_> = (0..3)
        .map(|i| orient2d(coord(t[i]), coord(t[(i + 1) % 3]), coord(p)))
        .collect();
    turns.iter().all(|&v| v >= 0.) || turns.iter().all(|&v| v <= 0.)
}
fn circle(t: [Point; 3]) -> Option<(Point, f64)> {
    let a = sub(t[1], t[0]);
    let b = sub(t[2], t[0]);
    let n = cross(a, b);
    let den = 2. * dot(n, n);
    if den <= 0. || !den.is_finite() {
        return None;
    }
    let w = std::array::from_fn(|k| dot(a, a) * b[k] - dot(b, b) * a[k]);
    let d = cross(w, n).map(|x| x / den);
    let p = std::array::from_fn(|k| t[0][k] + d[k]);
    p.iter().all(|x| x.is_finite()).then_some((p, norm(d)))
}
fn segment_encroached(p: Point, a: Point, b: Point) -> bool {
    dot(sub(p, a), sub(p, b)) < -1e-10 * dot(sub(a, b), sub(a, b))
}
fn features(state: &State, policy: &surface::KernelPatches) -> Vec<[usize; 2]> {
    let edges: BTreeSet<_> = state
        .constraints
        .iter()
        .flat_map(|c| (0..3).map(move |k| sorted([c.vertices[k], c.vertices[(k + 1) % 3]])))
        .collect();
    edges
        .into_iter()
        .filter(|e| {
            policy.features.iter().any(|&[a, b]| {
                e.iter()
                    .all(|&v| on_segment(state.world[v], state.world[a], state.world[b]))
            })
        })
        .collect()
}

/// Prepare all cells and constraints off to the side. No mutation before every
/// cavity, material-volume, surface-coverage and incidence check has passed.
fn insert(
    state: &mut State,
    point: Point,
    plc: &Plc3D,
    policy: &surface::KernelPatches,
    factor: f64,
) -> Result<Option<usize>, MeshError> {
    if point.iter().any(|x| !x.is_finite()) || state.world.contains(&point) {
        return Ok(None);
    }
    let q = point.map(|x| x * factor);
    let points: Vec<_> = state.world.iter().map(|p| p.map(|x| x * factor)).collect();
    let map = face_map(&state.tets)?;
    let barriers: BTreeMap<_, _> = state
        .constraints
        .iter()
        .map(|c| (sorted(c.vertices), c.parent))
        .collect();
    let active: BTreeSet<_> = state
        .constraints
        .iter()
        .filter(|c| on_triangle(point, c.vertices.map(|v| state.world[v])))
        .map(|c| c.parent)
        .collect();
    let mut cavity = BTreeSet::new();
    let mut todo = VecDeque::new();
    for (i, t) in state.tets.iter().enumerate() {
        if inside(q, t.map(|v| points[v])) {
            cavity.insert(i);
            todo.push_back(i);
        }
    }
    if cavity.is_empty() {
        return Ok(None);
    }
    while let Some(i) = todo.pop_front() {
        crate::control::checkpoint(state.world.len(), state.tets.len(), false)?;
        for k in 0..4 {
            let f = sorted(face(state.tets[i], k));
            if barriers.contains_key(&f) {
                continue;
            }
            for &(j, _) in &map[&f] {
                if cavity.contains(&j) || state.labels[i] != state.labels[j] {
                    continue;
                }
                // Coplanarity alone does not authorize deleting a neighboring
                // marker patch. Clip the conflict search at that feature.
                if (0..4).any(|k| {
                    let f = face(state.tets[j], k);
                    barriers.get(&sorted(f)).is_some_and(|parent| {
                        !active.contains(parent)
                            && orient(points[f[0]], points[f[1]], points[f[2]], q) == 0.
                    })
                }) {
                    continue;
                }
                let p = state.tets[j].map(|v| coord(points[v]));
                if insphere(p[0], p[1], p[2], p[3], coord(q)) >= 0. {
                    cavity.insert(j);
                    todo.push_back(j);
                }
            }
        }
    }
    let vertex = points.len();
    let mut children = Vec::new();
    let mut new_labels = Vec::new();
    let mut old_vol = BTreeMap::<i32, f64>::new();
    let mut new_vol = BTreeMap::<i32, f64>::new();
    for &i in &cavity {
        let t = state.tets[i];
        let label = state.labels[i];
        let p = t.map(|v| points[v]);
        *old_vol.entry(label).or_default() += orient(p[0], p[1], p[2], p[3]);
        for k in 0..4 {
            let f = face(t, k);
            let key = sorted(f);
            if !barriers.contains_key(&key)
                && map[&key]
                    .iter()
                    .any(|&(j, _)| j != i && cavity.contains(&j))
            {
                continue;
            }
            let p = f.map(|v| points[v]);
            let d = -orient(p[0], p[1], p[2], q);
            // Only a protected coplanar surface may disappear into the new
            // boundary fan. Unconstrained flat cavity faces are rejected.
            if d == 0.
                && barriers
                    .get(&key)
                    .is_some_and(|parent| active.contains(parent))
            {
                continue;
            }
            if !d.is_finite() || d <= 0. {
                return Ok(None);
            }
            children.push([f[1], f[0], f[2], vertex]);
            new_labels.push(label);
            *new_vol.entry(label).or_default() += d;
        }
    }
    if children.is_empty()
        || old_vol
            .iter()
            .any(|(l, v)| (new_vol.get(l).copied().unwrap_or(0.) - v).abs() > v.abs() * 1e-9)
    {
        return Ok(None);
    }
    let mut world = state.world.clone();
    world.push(point);
    let mut tets = Vec::new();
    let mut labels = Vec::new();
    for (i, &t) in state.tets.iter().enumerate() {
        if !cavity.contains(&i) {
            tets.push(t);
            labels.push(state.labels[i]);
        }
    }
    tets.extend(children);
    labels.extend(new_labels);
    let Ok(new_map) = face_map(&tets) else {
        return Ok(None);
    };
    let used: BTreeSet<_> = tets.iter().flatten().copied().collect();
    if state.tets.iter().flatten().any(|v| !used.contains(v)) {
        return Ok(None);
    }
    let mut membership: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for c in &state.constraints {
        membership.entry(c.parent).or_default().extend(c.vertices);
        if on_triangle(point, c.vertices.map(|v| state.world[v])) {
            membership.entry(c.parent).or_default().insert(vertex);
        }
    }
    let mut constraints = Vec::new();
    for (&parent, vertices) in &membership {
        let candidates: Vec<_> = new_map
            .keys()
            .filter(|f| f.iter().all(|v| vertices.contains(v)))
            .copied()
            .collect();
        let Some(recovered) = policy.recover(plc, &world, parent, &candidates)? else {
            return Ok(None);
        };
        constraints.extend(
            recovered
                .into_iter()
                .map(|vertices| Constraint { vertices, parent }),
        );
    }
    let new_barriers: BTreeSet<_> = constraints.iter().map(|c| sorted(c.vertices)).collect();
    for (f, entries) in &new_map {
        if entries.len() == 1 && !new_barriers.contains(f) {
            return Ok(None);
        }
        if entries.len() == 2 {
            let (a, ka) = entries[0];
            let (b, kb) = entries[1];
            let p = f.map(|v| world[v]);
            let sa = orient(p[0], p[1], p[2], world[tets[a][ka]]);
            let sb = orient(p[0], p[1], p[2], world[tets[b][kb]]);
            if sa.signum() == sb.signum() || (labels[a] != labels[b] && !new_barriers.contains(f)) {
                return Ok(None);
            }
        }
    }
    crate::control::checkpoint(world.len(), tets.len(), false)?;
    state.world = world;
    state.tets = tets;
    state.labels = labels;
    state.constraints = constraints;
    Ok(Some(cavity.len()))
}

#[derive(Clone, Copy)]
struct Proposal {
    point: Point,
    kind: usize,
    priority: f64,
}
fn boundary_proposal(state: &State, c: &Constraint) -> Proposal {
    let t = c.vertices.map(|v| state.world[v]);
    if let Some((p, _)) = circle(t) {
        if on_triangle(p, t) {
            return Proposal {
                point: p,
                kind: 1,
                priority: 1.,
            };
        }
    }
    let [a, b] = (0..3)
        .map(|k| [c.vertices[k], c.vertices[(k + 1) % 3]])
        .max_by(|e, f| {
            norm(sub(state.world[e[0]], state.world[e[1]]))
                .total_cmp(&norm(sub(state.world[f[0]], state.world[f[1]])))
        })
        .unwrap();
    Proposal {
        point: midpoint(state.world[a], state.world[b]),
        kind: 1,
        priority: 1.,
    }
}
fn protect(mut p: Proposal, state: &State, edges: &[[usize; 2]]) -> Proposal {
    if let Some(&[a, b]) = edges
        .iter()
        .filter(|&&[a, b]| segment_encroached(p.point, state.world[a], state.world[b]))
        .min_by(|e, f| {
            norm(sub(state.world[e[0]], state.world[e[1]]))
                .total_cmp(&norm(sub(state.world[f[0]], state.world[f[1]])))
        })
    {
        p.point = midpoint(state.world[a], state.world[b]);
        p.kind = 0;
        return p;
    }
    if p.kind == 2 {
        if let Some(c) = state
            .constraints
            .iter()
            .filter(|c| encroaches(p.point, c.vertices.map(|v| state.world[v])))
            .min_by(|a, b| {
                triangle_area(a.vertices.map(|v| state.world[v]))
                    .total_cmp(&triangle_area(b.vertices.map(|v| state.world[v])))
            })
        {
            let priority = p.priority;
            p = protect(boundary_proposal(state, c), state, edges);
            p.priority = priority;
        }
    }
    p
}

pub(super) fn refine(
    mut state: State,
    plc: &Plc3D,
    policy: &surface::KernelPatches,
    options: &Options3D,
    sizing: &Sizing3D,
    quality: &Quality3D,
    factor: f64,
) -> Result<(State, QualityReport3D), MeshError> {
    let kept: Vec<_> = state
        .tets
        .iter()
        .copied()
        .zip(state.labels.iter().copied())
        .filter(|(_, l)| *l != -2)
        .collect();
    state.tets = kept.iter().map(|x| x.0).collect();
    state.labels = kept.iter().map(|x| x.1).collect();
    let mut stats = QualityReport3D {
        max_radius_edge: quality.max_radius_edge,
        ..Default::default()
    };
    let mut rejected = BTreeSet::new();
    let mut inserted = 0;
    loop {
        crate::control::checkpoint(state.world.len(), state.tets.len(), true)?;
        let edges = features(&state, policy);
        let mut segments = Vec::new();
        let mut facets = Vec::new();
        let mut cells = Vec::new();
        stats.encroached_segments = 0;
        stats.encroached_faces = 0;
        for &[a, b] in &edges {
            let enc = state
                .world
                .iter()
                .any(|&p| segment_encroached(p, state.world[a], state.world[b]));
            stats.encroached_segments += usize::from(enc);
            let ratio = sizing.edge_ratio(state.world[a], state.world[b]);
            if enc || ratio > 1. + 1e-10 {
                segments.push(Proposal {
                    point: midpoint(state.world[a], state.world[b]),
                    kind: 0,
                    priority: ratio,
                });
            }
        }
        for c in &state.constraints {
            let t = c.vertices.map(|v| state.world[v]);
            let enc = state
                .world
                .iter()
                .enumerate()
                .any(|(i, &p)| !c.vertices.contains(&i) && encroaches(p, t));
            stats.encroached_faces += usize::from(enc);
            let ratio = (0..3)
                .map(|k| sizing.edge_ratio(t[k], t[(k + 1) % 3]))
                .fold(0_f64, f64::max);
            if enc || ratio > 1. + 1e-10 {
                let mut p = boundary_proposal(&state, c);
                p.priority = ratio;
                facets.push(protect(p, &state, &edges));
            }
        }
        for (i, t) in state.tets.iter().enumerate() {
            let p = t.map(|v| state.world[v]);
            let Some((r, _)) = shape_metrics(p) else {
                continue;
            };
            let edge = (0..4)
                .flat_map(|a| (a + 1..4).map(move |b| sizing.edge_ratio(p[a], p[b])))
                .fold(0_f64, f64::max);
            let regional = if state.labels[i] >= 0 {
                plc.regions[state.labels[i] as usize].max_volume
            } else {
                0.
            };
            let v = orient(p[0], p[1], p[2], p[3]) / 6. / limit(options.max_volume, regional);
            let angle = worst_dihedral(p).0;
            let severity = (r / quality.max_radius_edge)
                .max(edge)
                .max(v.cbrt())
                .max(options.min_dihedral / angle);
            if severity <= 1. + 1e-10 {
                continue;
            }
            if let Some((center, _)) = circumsphere(p) {
                cells.push(protect(
                    Proposal {
                        point: center,
                        kind: 2,
                        priority: severity,
                    },
                    &state,
                    &edges,
                ));
            }
        }
        for queue in [&mut segments, &mut facets, &mut cells] {
            queue.sort_by(|a, b| {
                b.priority.total_cmp(&a.priority).then_with(|| {
                    a.point
                        .partial_cmp(&b.point)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            });
        }
        if segments.is_empty() && facets.is_empty() && cells.is_empty() {
            stats.stop_reason = "criteria met".into();
            break;
        }
        if inserted >= quality.max_insertions || state.world.len() >= options.max_vertices {
            stats.stop_reason = "insertion or vertex budget".into();
            break;
        }
        let proposal = segments
            .into_iter()
            .chain(facets)
            .chain(cells)
            .find(|p| !rejected.contains(&p.point.map(f64::to_bits)));
        let Some(p) = proposal else {
            stats.stop_reason = "no admissible cavity for remaining proposals".into();
            break;
        };
        match insert(&mut state, p.point, plc, policy, factor)? {
            Some(size) => {
                stats.max_cavity_cells = stats.max_cavity_cells.max(size);
                inserted += 1;
                rejected.clear();
                match p.kind {
                    0 => stats.segment_insertions += 1,
                    1 => stats.facet_insertions += 1,
                    _ => stats.volume_insertions += 1,
                }
            }
            None => {
                rejected.insert(p.point.map(f64::to_bits));
                stats.rejected_transactions += 1;
            }
        }
        if stats.rejected_transactions > quality.max_insertions.saturating_mul(8).saturating_add(64)
        {
            stats.stop_reason = "rejected transaction budget".into();
            break;
        }
    }
    Ok((state, stats))
}
