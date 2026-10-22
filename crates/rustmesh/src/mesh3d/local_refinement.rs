//! Bounded refinement of an already recovered complex, without a Delaunay rebuild.
use super::*;

#[derive(Clone)]
pub(super) struct State {
    pub world: Vec<Point>,
    pub tets: Vec<Tet>,
    pub labels: Vec<i32>,
    pub constraints: Vec<Constraint>,
    pub splits: usize,
    pub insertions: usize,
    pub rounds: usize,
    pub flips: usize,
    pub collapses: usize,
    pub moves: usize,
    pub sweeps: usize,
}

impl State {
    fn score(&self, target: f64, factor: f64, limits: &BTreeMap<i32, f64>) -> (usize, usize, f64) {
        let mut oversized = 0;
        let mut below = 0;
        let mut worst = 180.0f64;
        for (t, &label) in self.tets.iter().zip(&self.labels) {
            if label == -2 {
                continue;
            }
            let p = t.map(|v| self.world[v]);
            if orient(p[0], p[1], p[2], p[3]) / 6. > limits[&label] * (1. + 1e-12) {
                oversized += 1;
            }
            let q = worst_dihedral(p.map(|p| p.map(|x| x * factor))).0;
            worst = worst.min(q);
            if q + 1e-10 < target {
                below += 1;
            }
        }
        (oversized, below, -worst)
    }

    /// A cell bounded by two protected faces at its worst edge cannot open
    /// that dihedral by interior refinement. Avoid an unbounded cascade of
    /// ever smaller cells along a genuinely acute PLC wedge.
    fn refinable_cells(&self, target: f64, factor: f64) -> Vec<usize> {
        let mut barriers: BTreeSet<_> = self
            .constraints
            .iter()
            .map(|c| sorted(c.vertices))
            .collect();
        if let Ok(map) = face_map(&self.tets) {
            barriers.extend(
                map.into_iter()
                    .filter(|(_, entries)| entries.len() == 1)
                    .map(|(f, _)| f),
            );
        }
        self.tets
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                let (q, edge) = worst_dihedral(t.map(|v| self.world[v].map(|x| x * factor)));
                if q + 1e-10 >= target {
                    return None;
                }
                let protected = (0..4)
                    .filter(|k| !edge.contains(k))
                    .all(|k| barriers.contains(&sorted(face(*t, k))));
                if protected {
                    None
                } else {
                    Some(i)
                }
            })
            .collect()
    }

    /// Split the full star of an edge, including excluded cells and every
    /// incident constraint face. Prepare all children before committing.
    fn split(&mut self, edge: [usize; 2]) -> bool {
        let [a, b] = edge;
        let midpoint: Point =
            std::array::from_fn(|k| self.world[a][k] * 0.5 + self.world[b][k] * 0.5);
        if self.world.contains(&midpoint) || midpoint.iter().any(|x| !x.is_finite()) {
            return false;
        }
        let vertex = self.world.len();
        let incident: Vec<_> = self
            .tets
            .iter()
            .enumerate()
            .filter(|(_, t)| t.contains(&a) && t.contains(&b))
            .map(|(i, _)| i)
            .collect();
        if incident.is_empty() {
            return false;
        }
        let point = |v: usize| if v == vertex { midpoint } else { self.world[v] };
        let mut children = Vec::new();
        for &i in &incident {
            let t = self.tets[i];
            let pair = [
                t.map(|v| if v == a { vertex } else { v }),
                t.map(|v| if v == b { vertex } else { v }),
            ];
            let p = t.map(point);
            let previous = orient(p[0], p[1], p[2], p[3]);
            let mut total = 0.;
            for child in pair {
                let p = child.map(point);
                let det = orient(p[0], p[1], p[2], p[3]);
                if !det.is_finite() || det <= 0. {
                    return false;
                }
                total += det;
                children.push((child, self.labels[i]));
            }
            if (total - previous).abs() > 1e-10 * previous {
                return false;
            }
        }
        let mut constraints = Vec::new();
        for c in &self.constraints {
            if c.vertices.contains(&a) && c.vertices.contains(&b) {
                let pieces = [
                    c.vertices.map(|v| if v == a { vertex } else { v }),
                    c.vertices.map(|v| if v == b { vertex } else { v }),
                ];
                let old = triangle_area(c.vertices.map(point));
                let new: f64 = pieces.iter().map(|f| triangle_area(f.map(point))).sum();
                if !new.is_finite() || (new - old).abs() > 1e-12 * old {
                    return false;
                }
                for vertices in pieces {
                    constraints.push(Constraint {
                        vertices,
                        parent: c.parent,
                    });
                }
            } else {
                constraints.push(c.clone());
            }
        }
        let removed: BTreeSet<_> = incident.into_iter().collect();
        let mut tets = Vec::new();
        let mut labels = Vec::new();
        for (i, &t) in self.tets.iter().enumerate() {
            if !removed.contains(&i) {
                tets.push(t);
                labels.push(self.labels[i]);
            }
        }
        for (t, l) in children {
            tets.push(t);
            labels.push(l);
        }
        self.world.push(midpoint);
        self.tets = tets;
        self.labels = labels;
        self.constraints = constraints;
        self.splits += 1;
        true
    }

    /// 1-to-4 subdivision: all old faces and labels are retained.
    fn insert_cell(&mut self, i: usize) -> bool {
        let tet = self.tets[i];
        let center: Point =
            std::array::from_fn(|k| tet.iter().map(|&v| self.world[v][k] * 0.25).sum());
        if self.world.contains(&center) {
            return false;
        }
        let vertex = self.world.len();
        let mut children = Vec::new();
        let mut total = 0.;
        let p = tet.map(|v| self.world[v]);
        let previous = orient(p[0], p[1], p[2], p[3]);
        for k in 0..4 {
            let mut t = tet;
            t[k] = vertex;
            let p = t.map(|v| if v == vertex { center } else { self.world[v] });
            let d = orient(p[0], p[1], p[2], p[3]);
            if !d.is_finite() || d <= 0. {
                return false;
            }
            total += d;
            children.push(t);
        }
        if (total - previous).abs() > 1e-10 * previous {
            return false;
        }
        self.world.push(center);
        self.tets[i] = children[0];
        for &t in &children[1..] {
            self.tets.push(t);
            self.labels.push(self.labels[i]);
        }
        self.insertions += 1;
        true
    }

    fn improve(
        &mut self,
        target: f64,
        factor: f64,
        input_vertices: usize,
        facets: &[Face],
        limits: &BTreeMap<i32, f64>,
    ) -> Result<(), MeshError> {
        for _ in 0..3 {
            let points: Vec<_> = self.world.iter().map(|p| p.map(|x| x * factor)).collect();
            let barriers = self
                .constraints
                .iter()
                .map(|c| (sorted(c.vertices), c.parent))
                .collect();
            let edges = self
                .constraints
                .iter()
                .flat_map(|c| (0..3).map(move |k| sorted([c.vertices[k], c.vertices[(k + 1) % 3]])))
                .collect();
            let (tets, labels, flips) = Flipper::new(
                &points,
                self.tets.clone(),
                self.labels.clone(),
                &barriers,
                &edges,
                &self.world,
                limits,
            )
            .run(target, 10);
            self.tets = tets;
            self.labels = labels;
            self.flips += flips;
            let stats = smoothing::smooth(
                &mut self.world,
                &self.tets,
                &self.labels,
                &self.constraints,
                limits,
                &smoothing::Options {
                    input_vertices,
                    target,
                    factor,
                    max_sweeps: 8,
                    preserve_violation_count: false,
                    boundary_facets: facets,
                },
            )?;
            self.moves += stats.moves;
            self.sweeps += stats.sweeps;
            if flips == 0 && stats.moves == 0 {
                break;
            }
        }
        Ok(())
    }
}

pub(super) fn graded(
    mut state: State,
    options: &Options3D,
    field: &Sizing3D,
    factor: f64,
    input_vertices: usize,
    limits: &BTreeMap<i32, f64>,
) -> Result<State, MeshError> {
    let kept: Vec<_> = state
        .tets
        .iter()
        .copied()
        .zip(state.labels.iter().copied())
        .filter(|(_, l)| *l != -2)
        .collect();
    state.tets = kept.iter().map(|x| x.0).collect();
    state.labels = kept.iter().map(|x| x.1).collect();
    for pass in 0..=field.max_splits {
        crate::control::checkpoint(state.world.len(), state.tets.len(), true)?;
        // All proposals operate on the current complex, never stale cell ids.
        let edges: BTreeSet<_> = state
            .tets
            .iter()
            .flat_map(|t| (0..4).flat_map(move |i| (i + 1..4).map(move |j| sorted([t[i], t[j]]))))
            .collect();
        let worst = edges
            .iter()
            .map(|&e| (field.edge_ratio(state.world[e[0]], state.world[e[1]]), e))
            .max_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let Some((ratio, edge)) = worst else {
            break;
        };
        if ratio <= 1. + 1e-10
            || pass == field.max_splits
            || state.world.len() >= options.max_vertices
        {
            break;
        }
        if !state.split(edge) {
            break;
        }
        state.rounds += 1;
    }
    for round in 0..field.flip_passes.min(3) {
        let points: Vec<_> = state.world.iter().map(|p| p.map(|x| x * factor)).collect();
        let barriers = state
            .constraints
            .iter()
            .map(|c| (sorted(c.vertices), c.parent))
            .collect();
        let edges = state
            .constraints
            .iter()
            .flat_map(|c| (0..3).map(move |k| sorted([c.vertices[k], c.vertices[(k + 1) % 3]])))
            .collect();
        let mut flipper = Flipper::new(
            &points,
            state.tets.clone(),
            state.labels.clone(),
            &barriers,
            &edges,
            &state.world,
            limits,
        );
        flipper.shape_guard = true;
        flipper.shape_objective = true;
        flipper.angle_floor = options.min_dihedral.max(1.);
        flipper.edge_field = Some(field);
        state.collapses += flipper.collapse_pass(input_vertices)?;
        let passes = field.flip_passes / 3 + usize::from(round < field.flip_passes % 3);
        let (tets, labels, flips) = flipper.run(0.95, passes);
        crate::control::checkpoint(state.world.len(), tets.len(), true)?;
        state.tets = tets;
        state.labels = labels;
        state.flips += flips;
    }
    Ok(state)
}

pub(super) fn optimize(
    mut state: State,
    options: &Options3D,
    factor: f64,
    input_vertices: usize,
    facets: &[Face],
    limits: &BTreeMap<i32, f64>,
) -> Result<State, MeshError> {
    if options.min_dihedral <= 0. {
        return Ok(state);
    }
    // Classification is final here: excluded cells are not part of the
    // optimized domain. Flat exterior cells must not block a valid boundary
    // split, and no local operation needs to rebuild the exterior Delaunay.
    let kept: Vec<_> = state
        .tets
        .iter()
        .copied()
        .zip(state.labels.iter().copied())
        .filter(|(_, l)| *l != -2)
        .collect();
    state.tets = kept.iter().map(|(t, _)| *t).collect();
    state.labels = kept.iter().map(|(_, l)| *l).collect();
    let target = options.min_dihedral;
    let mut best = state.clone();
    let met = |s: &State| {
        let q = s.score(target, factor, limits);
        q.0 == 0 && q.1 == 0
    };
    if met(&best) {
        return Ok(best);
    }
    if state
        .improve(target, factor, input_vertices, facets, limits)
        .is_err()
    {
        return Ok(best);
    }
    if state.score(target, factor, limits) < best.score(target, factor, limits) {
        best = state.clone();
    }
    let mut attempts = 0;
    // Long-edge bisection supplies a conforming finer boundary. If that stalls,
    // restart from the best complex and supply interior degrees of freedom.
    // Work on trial copies; only the best measured complex is returned.
    'phases: for phase in 0..2 {
        if phase == 1 {
            state = best.clone();
        }
        for _ in 0..if phase == 0 { 12 } else { 4 } {
            crate::control::checkpoint(state.world.len(), state.tets.len(), true)?;
            if met(&best) {
                break 'phases;
            }
            let bad = state.refinable_cells(target, factor);
            let mut changed = false;
            if phase == 0 {
                let mut edges = BTreeSet::new();
                for i in bad {
                    let t = state.tets[i];
                    let longest = (0..4)
                        .flat_map(|a| (a + 1..4).map(move |b| sorted([t[a], t[b]])))
                        .max_by(|a, b| {
                            norm(sub(state.world[a[0]], state.world[a[1]]))
                                .total_cmp(&norm(sub(state.world[b[0]], state.world[b[1]])))
                                .then(a.cmp(b))
                        })
                        .unwrap();
                    edges.insert(longest);
                }
                for e in edges {
                    crate::control::checkpoint(state.world.len(), state.tets.len(), false)?;
                    if state.world.len() >= options.max_vertices {
                        break;
                    }
                    changed |= state.split(e);
                }
            } else {
                for i in bad {
                    crate::control::checkpoint(state.world.len(), state.tets.len(), false)?;
                    if state.world.len() >= options.max_vertices {
                        break;
                    }
                    changed |= state.insert_cell(i);
                }
            }
            if !changed {
                break;
            }
            attempts += 1;
            if face_map(&state.tets).is_err()
                || state
                    .improve(target, factor, input_vertices, facets, limits)
                    .is_err()
            {
                break;
            }
            if state.score(target, factor, limits) < best.score(target, factor, limits) {
                best = state.clone();
            }
        }
    }
    best.rounds = attempts;
    Ok(best)
}
