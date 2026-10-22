//! Conservative planar patches and coverage provenance. No input coordinate moves.
use super::*;
use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};

/// Positive-area contribution from one original input triangle, in input units².
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FacetCoverage3D {
    pub facet: usize,
    pub area: f64,
}

/// A re-triangulated PLC with the same points, markers, regions and physical surface.
/// `plc.facets` have new indices. Use coverage, not those indices, for original ancestry.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PreparedSurface3D {
    pub plc: Plc3D,
    /// Original input facet ids, one connected coplanar same-marker patch per row.
    pub patches: Vec<Vec<usize>>,
    pub facet_patches: Vec<usize>,
    pub facet_sources: Vec<Vec<FacetCoverage3D>>,
    /// Atomic crease, marker, junction and explicitly protected input edges.
    pub feature_edges: Vec<[usize; 2]>,
    pub removable_seams: usize,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PatchedMesh3D {
    /// `mesh.face_facets` indexes a prepared triangle only when it is the sole
    /// contributor; -1 means multiple prepared contributors. Use face_sources.
    pub mesh: Mesh3D,
    pub surface: PreparedSurface3D,
    pub face_patches: Vec<usize>,
    /// Coverage of ORIGINAL input triangles for every exported mesh face.
    pub face_sources: Vec<Vec<FacetCoverage3D>>,
}

struct Projection {
    axes: [usize; 2],
    factor: f64,
}
impl Projection {
    fn new(points: &[Point], f: Face) -> Self {
        let magnitude = points
            .iter()
            .flatten()
            .map(|x| x.abs())
            .fold(0_f64, f64::max);
        let factor = 2_f64.powi(-(magnitude.log2().floor() as i32));
        let p = f.map(|v| points[v].map(|x| x * factor));
        let n = cross(sub(p[1], p[0]), sub(p[2], p[0]));
        let drop = (0..3)
            .max_by(|&i, &j| n[i].abs().total_cmp(&n[j].abs()))
            .unwrap();
        Self {
            axes: [(drop + 1) % 3, (drop + 2) % 3],
            factor,
        }
    }
    fn point(&self, p: Point) -> [f64; 2] {
        self.axes.map(|k| p[k] * self.factor)
    }
}
fn turn(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    let coord = |p: [f64; 2]| Coord { x: p[0], y: p[1] };
    orient2d(coord(a), coord(b), coord(c))
}
fn area(poly: &[[f64; 2]]) -> f64 {
    if poly.len() < 3 {
        return 0.;
    }
    (1..poly.len() - 1)
        .map(|i| turn(poly[0], poly[i], poly[i + 1]))
        .sum::<f64>()
        .abs()
        * 0.5
}
/// Convex clipping; predicates evaluate signs robustly, intersections are f64.
fn intersection(a: [[f64; 2]; 3], mut b: [[f64; 2]; 3]) -> f64 {
    if turn(b[0], b[1], b[2]) < 0. {
        b.swap(0, 1);
    }
    let mut poly = a.to_vec();
    for k in 0..3 {
        if poly.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for i in 0..poly.len() {
            let p = poly[i];
            let q = poly[(i + 1) % poly.len()];
            let u = turn(b[k], b[(k + 1) % 3], p);
            let v = turn(b[k], b[(k + 1) % 3], q);
            if u >= 0. {
                next.push(p);
            }
            if (u < 0. && v > 0.) || (u > 0. && v < 0.) {
                let t = u / (u - v);
                next.push(std::array::from_fn(|j| p[j] * (1. - t) + q[j] * t));
            }
        }
        poly = next;
    }
    area(&poly)
}
pub(super) fn coverage(
    plc: &Plc3D,
    patch: &[usize],
    triangle: [Point; 3],
) -> Result<Vec<FacetCoverage3D>, MeshError> {
    let projection = Projection::new(&plc.vertices, plc.facets[patch[0]]);
    let a = triangle.map(|p| projection.point(p));
    let size = area(&a);
    let world_area = triangle_area(triangle);
    if !size.is_finite() || size <= 0. || !world_area.is_finite() || world_area <= 0. {
        return Err(failed("invalid surface triangle area"));
    }
    let seed = plc.facets[patch[0]].map(|v| plc.vertices[v].map(|x| x * projection.factor));
    let normal = cross(sub(seed[1], seed[0]), sub(seed[2], seed[0]));
    let extent = triangle
        .iter()
        .map(|p| norm(sub(p.map(|x| x * projection.factor), seed[0])))
        .fold(0_f64, f64::max)
        .max(norm(sub(seed[1], seed[0])));
    if triangle.iter().any(|p| {
        dot(normal, sub(p.map(|x| x * projection.factor), seed[0])).abs()
            > 1e-10 * norm(normal) * extent
    }) {
        return Err(failed("surface triangle left its patch plane"));
    }
    let mut result = Vec::new();
    for &facet in patch {
        let b = plc.facets[facet].map(|v| projection.point(plc.vertices[v]));
        let overlap = intersection(a, b);
        if overlap > size * 1e-13 {
            result.push(FacetCoverage3D {
                facet,
                area: overlap / size * world_area,
            });
        }
    }
    Ok(result)
}
fn audit_coverage(plc: &Plc3D, pieces: &[Vec<FacetCoverage3D>]) -> Result<(), MeshError> {
    let mut totals = vec![0.; plc.facets.len()];
    for sources in pieces {
        for c in sources {
            totals[c.facet] += c.area;
        }
    }
    for (i, f) in plc.facets.iter().enumerate() {
        let expected = triangle_area(f.map(|v| plc.vertices[v]));
        if (totals[i] - expected).abs() > expected * 1e-8 {
            return Err(failed(format!(
                "surface coverage changed for original facet {i}"
            )));
        }
    }
    Ok(())
}

/// Merge only exactly coplanar, edge-connected, same-marker facets. Original
/// crease/junction edges and requested input edges remain constrained. Merely
/// near-coplanar input is NOT snapped or merged. `protected_edges` must be
/// edges of the original input facets; electrodes retain their coordinates.
pub fn prepare_surface_patches(
    plc: &Plc3D,
    protected_edges: &[[usize; 2]],
) -> Result<PreparedSurface3D, MeshError> {
    crate::control::checkpoint(plc.vertices.len(), plc.facets.len(), true)?;
    plc.validate()?;
    if plc.facets.is_empty() {
        return Err(bad("surface patches require explicit PLC facets"));
    }
    let original_edges: BTreeSet<_> = plc
        .facets
        .iter()
        .flat_map(|f| (0..3).map(move |k| sorted([f[k], f[(k + 1) % 3]])))
        .collect();
    if protected_edges
        .iter()
        .any(|e| !original_edges.contains(&sorted(*e)))
    {
        return Err(bad("protected surface edge is not an input facet edge"));
    }
    let atomic = split_at_input_vertices(
        plc.facets
            .iter()
            .enumerate()
            .map(|(parent, &vertices)| Constraint { vertices, parent })
            .collect(),
        &plc.vertices,
    );
    let mut edges: BTreeMap<[usize; 2], Vec<usize>> = BTreeMap::new();
    for (i, c) in atomic.iter().enumerate() {
        for k in 0..3 {
            edges
                .entry(sorted([c.vertices[k], c.vertices[(k + 1) % 3]]))
                .or_default()
                .push(i);
        }
    }
    let mut neighbours = vec![Vec::new(); atomic.len()];
    let mut features = BTreeSet::new();
    let mut seams = 0;
    let is_protected = |e: [usize; 2]| {
        protected_edges.iter().any(|&[a, b]| {
            e.iter().all(|&v| {
                collinear([plc.vertices[a], plc.vertices[b], plc.vertices[v]])
                    && (0..3).all(|k| {
                        plc.vertices[v][k] >= plc.vertices[a][k].min(plc.vertices[b][k])
                            && plc.vertices[v][k] <= plc.vertices[a][k].max(plc.vertices[b][k])
                    })
            })
        })
    };
    for (&edge, adj) in &edges {
        let merge = if adj.len() == 2 && !is_protected(edge) {
            let a = &atomic[adj[0]];
            let b = &atomic[adj[1]];
            let p = a.vertices.map(|v| plc.vertices[v]);
            plc.facet_markers.get(a.parent) == plc.facet_markers.get(b.parent)
                && b.vertices
                    .iter()
                    .all(|&v| orient(p[0], p[1], p[2], plc.vertices[v]) == 0.)
        } else {
            false
        };
        if merge {
            neighbours[adj[0]].push(adj[1]);
            neighbours[adj[1]].push(adj[0]);
            seams += 1;
        } else {
            features.insert(edge);
        }
    }
    let mut visited = vec![false; atomic.len()];
    let mut groups = Vec::new();
    for i in 0..atomic.len() {
        if visited[i] {
            continue;
        }
        let mut stack = vec![i];
        visited[i] = true;
        let mut group = Vec::new();
        while let Some(j) = stack.pop() {
            group.push(j);
            for &k in &neighbours[j] {
                if !visited[k] {
                    visited[k] = true;
                    stack.push(k);
                }
            }
        }
        group.sort_unstable();
        groups.push(group);
    }
    let mut result = PreparedSurface3D {
        plc: plc.clone(),
        patches: Vec::new(),
        facet_patches: Vec::new(),
        facet_sources: Vec::new(),
        feature_edges: features.iter().copied().collect(),
        removable_seams: seams,
    };
    result.plc.facets.clear();
    result.plc.facet_markers.clear();
    for (patch, group) in groups.iter().enumerate() {
        crate::control::checkpoint(plc.vertices.len(), result.plc.facets.len(), true)?;
        let sources: Vec<_> = group
            .iter()
            .map(|&i| atomic[i].parent)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let vertices: BTreeSet<_> = group.iter().flat_map(|&i| atomic[i].vertices).collect();
        let projection = Projection::new(&plc.vertices, atomic[group[0]].vertices);
        let mut cdt = ConstrainedDelaunayTriangulation::<Point2<f64>>::new();
        let mut handles = BTreeMap::new();
        let mut vertex_ids = Vec::new();
        for &v in &vertices {
            let p = projection.point(plc.vertices[v]);
            let h = cdt
                .insert(Point2::new(p[0], p[1]))
                .map_err(|_| failed("patch projection insertion failed"))?;
            if h.index() != vertex_ids.len() {
                return Err(failed("patch projection collapsed distinct points"));
            }
            vertex_ids.push(v);
            handles.insert(v, h);
        }
        for e in &features {
            if vertices.contains(&e[0])
                && vertices.contains(&e[1])
                && cdt
                    .try_add_constraint(handles[&e[0]], handles[&e[1]])
                    .is_empty()
            {
                return Err(failed(
                    "patch feature edges could not be recovered without new intersections",
                ));
            }
        }
        for f in cdt.inner_faces() {
            let face = f.vertices().map(|v| vertex_ids[v.fix().index()]);
            let tri = face.map(|v| plc.vertices[v]);
            let covered = coverage(plc, &sources, tri)?;
            let total: f64 = covered.iter().map(|c| c.area).sum();
            let expected = triangle_area(tri);
            if total <= expected * 1e-12 {
                continue;
            } // Exterior or a hole in this patch.
            if (total - expected).abs() > expected * 1e-8 {
                return Err(failed(
                    "patch triangle crosses uncovered or overlapping surface",
                ));
            }
            result.plc.facets.push(face);
            result
                .plc
                .facet_markers
                .push(plc.facet_markers.get(sources[0]).copied().unwrap_or(0));
            result.facet_patches.push(patch);
            result.facet_sources.push(covered);
        }
        result.patches.push(sources);
    }
    audit_coverage(plc, &result.facet_sources)?;
    result.plc.validate()?;
    Ok(result)
}

/// Patch constraints remain active through global boundary recovery; temporary
/// prepared triangle diagonals are not permanent recovery constraints.
pub(super) struct KernelPatches {
    pub representatives: Vec<usize>,
    pub groups: BTreeMap<usize, Vec<usize>>,
    pub(super) features: Vec<[usize; 2]>,
}
impl KernelPatches {
    pub(super) fn new(surface: &PreparedSurface3D) -> Self {
        let mut grouped = vec![Vec::new(); surface.patches.len()];
        for (f, &patch) in surface.facet_patches.iter().enumerate() {
            grouped[patch].push(f);
        }
        let mut representatives = vec![0; surface.plc.facets.len()];
        let mut groups = BTreeMap::new();
        for g in grouped {
            for &i in &g {
                representatives[i] = g[0];
            }
            groups.insert(g[0], g);
        }
        Self {
            representatives,
            groups,
            features: surface.feature_edges.clone(),
        }
    }
    pub(super) fn recover(
        &self,
        plc: &Plc3D,
        world: &[Point],
        parent: usize,
        candidates: &[Face],
    ) -> Result<Option<Vec<Face>>, MeshError> {
        let group = &self.groups[&parent];
        let mut totals = BTreeMap::new();
        let mut kept = Vec::new();
        for (i, &f) in candidates.iter().enumerate() {
            if i % 128 == 0 {
                crate::control::checkpoint(world.len(), candidates.len(), true)?;
            }
            let tri = f.map(|v| world[v]);
            let Ok(pieces) = coverage(plc, group, tri) else {
                continue;
            };
            let total: f64 = pieces.iter().map(|c| c.area).sum();
            let expected = triangle_area(tri);
            if (total - expected).abs() > expected * 1e-8 {
                continue;
            }
            for c in pieces {
                *totals.entry(c.facet).or_insert(0.) += c.area;
            }
            kept.push(f);
        }
        if group.iter().any(|&i| {
            let expected = triangle_area(plc.facets[i].map(|v| plc.vertices[v]));
            (totals.get(&i).copied().unwrap_or(0.) - expected).abs() > expected * 1e-8
        }) {
            return Ok(None);
        }
        let vertices: BTreeSet<_> = group.iter().flat_map(|&i| plc.facets[i]).collect();
        let edges: BTreeSet<_> = kept
            .iter()
            .flat_map(|f| (0..3).map(move |k| sorted([f[k], f[(k + 1) % 3]])))
            .collect();
        for &[a, b] in &self.features {
            if !vertices.contains(&a) || !vertices.contains(&b) {
                continue;
            }
            // Original atomic features may be subdivided; require a connected
            // mesh-edge chain on that exact segment, never just endpoint presence.
            let on = |v: usize| {
                collinear([world[a], world[b], world[v]])
                    && (0..3).all(|k| {
                        world[v][k] >= world[a][k].min(world[b][k])
                            && world[v][k] <= world[a][k].max(world[b][k])
                    })
            };
            let mut reached = BTreeSet::from([a]);
            loop {
                let before = reached.len();
                for &[u, v] in &edges {
                    if on(u) && on(v) && (reached.contains(&u) || reached.contains(&v)) {
                        reached.insert(u);
                        reached.insert(v);
                    }
                }
                if before == reached.len() {
                    break;
                }
            }
            if !reached.contains(&b) {
                return Ok(None);
            }
        }
        Ok(Some(kept))
    }
}

/// Experimental planar-patch recovery followed by the current sized volume
/// kernel. This is not yet boundary-aware Delaunay quality refinement.
pub fn tetrahedralize_patched(
    plc: &Plc3D,
    options: &Options3D,
    sizing: &Sizing3D,
    protected_edges: &[[usize; 2]],
) -> Result<PatchedMesh3D, MeshError> {
    generate(plc, options, sizing, protected_edges, None)
}

pub(super) fn generate(
    plc: &Plc3D,
    options: &Options3D,
    sizing: &Sizing3D,
    protected_edges: &[[usize; 2]],
    quality: Option<&Quality3D>,
) -> Result<PatchedMesh3D, MeshError> {
    validate_sizing(sizing)?;
    let surface = prepare_surface_patches(plc, protected_edges)?;
    let policy = KernelPatches::new(&surface);
    let mut mesh =
        tetrahedralize_impl(&surface.plc, options, Some(sizing), Some(&policy), quality)?;
    let mut face_sources = Vec::new();
    let mut face_patches = Vec::new();
    for (i, f) in mesh.faces.iter().enumerate() {
        let parent = mesh.face_facets[i];
        if parent < 0 {
            return Err(failed("patch mesh has an untracked boundary face"));
        }
        let patch = surface.facet_patches[parent as usize];
        let tri = f.map(|v| mesh.nodes[v]);
        face_patches.push(patch);
        let prepared_sources = coverage(&surface.plc, &policy.groups[&(parent as usize)], tri)?;
        mesh.face_facets[i] = if prepared_sources.len() == 1 {
            prepared_sources[0].facet as i64
        } else {
            -1
        };
        let sources = coverage(plc, &surface.patches[patch], tri)?;
        let expected = triangle_area(tri);
        let total: f64 = sources.iter().map(|c| c.area).sum();
        if (total - expected).abs() > expected * 1e-8 {
            return Err(failed("exported patch face has incomplete coverage"));
        }
        face_sources.push(sources);
    }
    audit_coverage(plc, &face_sources)?;
    Ok(PatchedMesh3D {
        mesh,
        surface,
        face_patches,
        face_sources,
    })
}
