//! Bounded relocation of generated vertices on a fixed complex. Original input
//! points stay fixed; optional boundary sliding retains original PLC supports.
use super::{face_map, norm, orient, sub, worst_dihedral, Constraint, Face, MeshError, Point, Tet};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Options<'a> {
    pub input_vertices: usize,
    pub target: f64,
    pub factor: f64,
    pub max_sweeps: usize,
    pub preserve_violation_count: bool,
    /// Optional original facets: generated points may slide on their parent plane/edge.
    pub boundary_facets: &'a [Face],
}

#[derive(Default)]
pub(super) struct Stats {
    pub moves: usize,
    pub sweeps: usize,
}

/// Only generated points move; constrained points optionally slide on their
/// original input face or edge. Each proposal is evaluated without
/// modifying the mesh. Every incident cell must remain strictly positive in
/// original coordinates, keep its previous volume compliance, and the worst
/// incident dihedral must strictly improve. Fixed cavity boundaries therefore
/// stay unchanged. No new tetrahedra or boundary projections are introduced.
pub(super) fn smooth(
    world: &mut [Point],
    tets: &[Tet],
    labels: &[i32],
    constraints: &[Constraint],
    limits: &BTreeMap<i32, f64>,
    options: &Options<'_>,
) -> Result<Stats, MeshError> {
    let mut stats = Stats::default();
    if options.target <= 0.0 {
        return Ok(stats);
    }
    let mut fixed = vec![false; world.len()];
    fixed[..options.input_vertices].fill(true);
    let mut parents = vec![BTreeSet::new(); world.len()];
    let mut constraint_incident = vec![Vec::new(); world.len()];
    for (i, c) in constraints.iter().enumerate() {
        for &v in &c.vertices {
            fixed[v] = true;
            parents[v].insert(c.parent);
            constraint_incident[v].push(i);
        }
    }
    for (f, cells) in face_map(tets)? {
        if cells.len() == 1 || cells.iter().any(|&(i, _)| labels[i] != labels[cells[0].0]) {
            for v in f {
                fixed[v] = true;
            }
        }
    }
    // A generated constrained vertex can slide only on the intersection of
    // its original input facets: one triangle plane or a common original edge.
    // Original inputs and unrepresented hull/material boundaries stay fixed.
    let mut sliding = vec![Vec::new(); world.len()];
    if !options.boundary_facets.is_empty() {
        for v in options.input_vertices..world.len() {
            let mut iter = parents[v].iter();
            if let Some(&first) = iter.next() {
                let mut common = options.boundary_facets[first].to_vec();
                for &parent in iter {
                    common.retain(|x| options.boundary_facets[parent].contains(x));
                }
                if common.len() == 2 || common.len() == 3 {
                    sliding[v] = common;
                    fixed[v] = false;
                }
            }
        }
    }
    let mut incident = vec![Vec::new(); world.len()];
    for (i, t) in tets.iter().enumerate() {
        for &v in t {
            if labels[i] == -2 && sliding[v].is_empty() {
                fixed[v] = true;
            }
            incident[v].push(i);
        }
    }
    let neighbors: Vec<BTreeSet<usize>> = incident
        .iter()
        .enumerate()
        .map(|(v, cells)| {
            cells
                .iter()
                .flat_map(|&i| tets[i])
                .filter(|&w| w != v)
                .collect()
        })
        .collect();
    let angle = |p: [Point; 4]| worst_dihedral(p.map(|q| q.map(|x| x * options.factor))).0;
    for _ in 0..options.max_sweeps {
        crate::control::checkpoint(world.len(), tets.len(), true)?;
        let mut order: Vec<_> = (0..world.len())
            .filter(|&v| !fixed[v] && !incident[v].is_empty())
            .map(|v| {
                (
                    incident[v]
                        .iter()
                        .map(|&i| {
                            if labels[i] == -2 {
                                180.
                            } else {
                                angle(tets[i].map(|w| world[w]))
                            }
                        })
                        .fold(180.0, f64::min),
                    v,
                )
            })
            .filter(|&(q, _)| q + 1e-9 < options.target)
            .collect();
        if order.is_empty() {
            break;
        }
        order.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        stats.sweeps += 1;
        let before = stats.moves;
        for (_, v) in order {
            let cells = &incident[v];
            let mut quality = cells
                .iter()
                .map(|&i| {
                    if labels[i] == -2 {
                        180.
                    } else {
                        angle(tets[i].map(|w| world[w]))
                    }
                })
                .fold(180.0, f64::min);
            if quality + 1e-9 >= options.target {
                continue;
            }
            // Keep each currently satisfied cell limit satisfied; an already
            // oversized cell may shrink but must not grow during smoothing.
            let caps: Vec<_> = cells
                .iter()
                .map(|&i| {
                    let p = tets[i].map(|w| world[w]);
                    let volume = orient(p[0], p[1], p[2], p[3]) / 6.0;
                    limits
                        .get(&labels[i])
                        .copied()
                        .unwrap_or(f64::INFINITY)
                        .max(volume)
                })
                .collect();
            let mut step = neighbors[v]
                .iter()
                .map(|&w| norm(sub(world[w], world[v])))
                .fold(f64::INFINITY, f64::min)
                * 0.2;
            if !step.is_finite() || step <= 0.0 {
                continue;
            }
            for _ in 0..10 {
                let origin = world[v];
                // Neighbour average plus a bounded pattern search. Shorter
                // steps are tried when no proposal improves the local minimum.
                let average: Point = std::array::from_fn(|k| {
                    neighbors[v]
                        .iter()
                        .map(|&w| world[w][k] / neighbors[v].len() as f64)
                        .sum()
                });
                let displacement = sub(average, origin);
                let length = norm(displacement);
                let mut candidates = Vec::new();
                if length.is_finite() && length > 0.0 {
                    for fraction in [1.0, 0.5, 0.25] {
                        let scale = (step / length).min(1.0) * fraction;
                        candidates
                            .push(std::array::from_fn(|k| origin[k] + displacement[k] * scale));
                    }
                }
                for x in -1..=1 {
                    for y in -1..=1 {
                        for z in -1..=1 {
                            if x == 0 && y == 0 && z == 0 {
                                continue;
                            }
                            let direction = [x as f64, y as f64, z as f64];
                            let scale = step / norm(direction);
                            candidates
                                .push(std::array::from_fn(|k| origin[k] + direction[k] * scale));
                        }
                    }
                }
                let old_below = cells
                    .iter()
                    .filter(|&&i| {
                        labels[i] != -2 && angle(tets[i].map(|w| world[w])) + 1e-10 < options.target
                    })
                    .count();
                let mut best = None;
                let mut best_quality = quality;
                for mut candidate in candidates {
                    let support = &sliding[v];
                    if support.len() == 2 {
                        let a = world[support[0]];
                        let edge = sub(world[support[1]], a);
                        let length = norm(edge);
                        let direction = edge.map(|x| x / length);
                        let t = super::dot(sub(candidate, a), direction).clamp(0., length);
                        candidate = std::array::from_fn(|k| a[k] + t * direction[k]);
                    } else if support.len() == 3 {
                        let a = world[support[0]];
                        let ab = sub(world[support[1]], a);
                        let ac = sub(world[support[2]], a);
                        let n = super::cross(ab, ac);
                        let n = n.map(|x| x / norm(n));
                        let distance = super::dot(sub(candidate, a), n);
                        candidate = std::array::from_fn(|k| candidate[k] - distance * n[k]);
                    }
                    // Refuse a fold in any incident boundary/interface triangle.
                    if constraint_incident[v].iter().any(|&j| {
                        let c = &constraints[j];
                        let old = c.vertices.map(|w| world[w]);
                        let new = c
                            .vertices
                            .map(|w| if w == v { candidate } else { world[w] });
                        let a = super::cross(sub(old[1], old[0]), sub(old[2], old[0]));
                        let b = super::cross(sub(new[1], new[0]), sub(new[2], new[0]));
                        let alignment = super::dot(a.map(|x| x / norm(a)), b.map(|x| x / norm(b)));
                        !alignment.is_finite() || alignment <= 0.
                    }) {
                        continue;
                    }

                    if candidate == origin || candidate.iter().any(|x| !x.is_finite()) {
                        continue;
                    }
                    let mut q = 180.0f64;
                    let mut below = 0;
                    let valid = cells.iter().zip(&caps).all(|(&i, &cap)| {
                        let p = tets[i].map(|w| if w == v { candidate } else { world[w] });
                        let determinant = orient(p[0], p[1], p[2], p[3]);
                        if !determinant.is_finite()
                            || determinant <= 0.0
                            || determinant / 6.0 > cap * (1.0 + 1e-12)
                        {
                            return false;
                        }
                        let candidate_angle = if labels[i] == -2 { 180. } else { angle(p) };
                        q = q.min(candidate_angle);
                        if candidate_angle + 1e-10 < options.target {
                            below += 1;
                        }
                        q > best_quality + 1e-9
                            && (!options.preserve_violation_count || below <= old_below)
                    });
                    if valid && q > best_quality + 1e-9 {
                        best = Some(candidate);
                        best_quality = q;
                    }
                }
                match best {
                    Some(p) => {
                        world[v] = p;
                        quality = best_quality;
                        stats.moves += 1;
                    }
                    None => step *= 0.5,
                }
                if quality + 1e-9 >= options.target {
                    break;
                }
            }
        }
        if stats.moves == before {
            break;
        }
    }
    Ok(stats)
}
