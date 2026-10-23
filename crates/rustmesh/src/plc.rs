use std::cmp::Ordering;

use robust::{orient2d, Coord};

use crate::error::MeshError;

/// A region seed: every cell reachable from `point` without crossing a
/// segment gets `marker`, and no cell larger than `max_area` (0: no limit).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Region {
    pub point: [f64; 2],
    pub marker: i32,
    pub max_area: f64,
}

/// A planar straight-line graph, the input of [`crate::triangulate`].
///
/// Vertices and segments with optional markers, regions and hole seeds.
/// Marker vectors may be left empty (all zero).
#[derive(Debug, Clone, Default)]
pub struct Plc {
    pub vertices: Vec<[f64; 2]>,
    pub vertex_markers: Vec<i32>,
    pub segments: Vec<[usize; 2]>,
    pub segment_markers: Vec<i32>,
    pub regions: Vec<Region>,
    pub holes: Vec<[f64; 2]>,
}

impl Plc {
    pub fn vertex_marker(&self, i: usize) -> i32 {
        self.vertex_markers.get(i).copied().unwrap_or(0)
    }

    pub fn segment_marker(&self, i: usize) -> i32 {
        self.segment_markers.get(i).copied().unwrap_or(0)
    }

    /// Checks everything the mesher relies on, so meshing never starts on
    /// input it would mishandle.
    pub fn validate(&self) -> Result<(), MeshError> {
        let n = self.vertices.len();
        if n < 3 {
            return Err(MeshError::TooFewVertices);
        }
        for (what, len) in [
            ("vertex markers", self.vertex_markers.len()),
            ("segment markers", self.segment_markers.len()),
        ] {
            let expected = if what == "vertex markers" {
                n
            } else {
                self.segments.len()
            };
            if len != 0 && len != expected {
                return Err(MeshError::MarkerCount {
                    what,
                    expected,
                    got: len,
                });
            }
        }
        for (i, v) in self.vertices.iter().enumerate() {
            if !(v[0].is_finite() && v[1].is_finite()) {
                return Err(MeshError::NonFiniteVertex { vertex: i });
            }
        }
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| cmp_point(self.vertices[a], self.vertices[b]));
        for w in order.windows(2) {
            if self.vertices[w[0]] == self.vertices[w[1]] {
                return Err(MeshError::DuplicateVertex {
                    first: w[0].min(w[1]),
                    second: w[0].max(w[1]),
                });
            }
        }
        for (i, s) in self.segments.iter().enumerate() {
            for &index in s {
                if index >= n {
                    return Err(MeshError::SegmentIndex { segment: i, index });
                }
            }
            if s[0] == s[1] {
                return Err(MeshError::ZeroLengthSegment { segment: i });
            }
        }
        for (what, points) in [
            (
                "region",
                self.regions.iter().map(|r| r.point).collect::<Vec<_>>(),
            ),
            ("hole", self.holes.clone()),
        ] {
            for (i, p) in points.iter().enumerate() {
                if !(p[0].is_finite() && p[1].is_finite()) {
                    return Err(MeshError::NonFiniteSeed { what, index: i });
                }
            }
        }
        self.check_segment_intersections()
    }

    /// Segments may meet at vertices, a segment end may lie on another
    /// segment, and segments may cross (the mesher adds a vertex at the
    /// crossing), but no two segments may overlap along a stretch.
    fn check_segment_intersections(&self) -> Result<(), MeshError> {
        let seg = |i: usize| {
            let [a, b] = self.segments[i];
            (self.vertices[a], self.vertices[b])
        };
        let mut order: Vec<usize> = (0..self.segments.len()).collect();
        let min_x = |i: usize| seg(i).0[0].min(seg(i).1[0]);
        order.sort_by(|&a, &b| min_x(a).total_cmp(&min_x(b)));
        for (k, &i) in order.iter().enumerate() {
            let (p1, p2) = seg(i);
            let max_x = p1[0].max(p2[0]);
            let (lo_y, hi_y) = (p1[1].min(p2[1]), p1[1].max(p2[1]));
            for &j in &order[k + 1..] {
                if min_x(j) > max_x {
                    break;
                }
                let (q1, q2) = seg(j);
                if q1[1].max(q2[1]) < lo_y || q1[1].min(q2[1]) > hi_y {
                    continue;
                }
                if segments_overlap(p1, p2, q1, q2) {
                    return Err(MeshError::SegmentsIntersect {
                        first: i.min(j),
                        second: i.max(j),
                    });
                }
            }
        }
        Ok(())
    }
}

fn cmp_point(a: [f64; 2], b: [f64; 2]) -> Ordering {
    a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1]))
}

fn orient(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    let c2 = |p: [f64; 2]| Coord { x: p[0], y: p[1] };
    orient2d(c2(a), c2(b), c2(c))
}

/// True when two segments are collinear and overlap along a stretch of
/// positive length. Touching at an end, or crossing, is not an overlap.
fn segments_overlap(p1: [f64; 2], p2: [f64; 2], q1: [f64; 2], q2: [f64; 2]) -> bool {
    if orient(p1, p2, q1) == 0.0 && orient(p1, p2, q2) == 0.0 {
        // Collinear: compare the extents along the dominant axis.
        let axis = if (p2[0] - p1[0]).abs() >= (p2[1] - p1[1]).abs() {
            0
        } else {
            1
        };
        let (a0, a1) = (p1[axis].min(p2[axis]), p1[axis].max(p2[axis]));
        let (b0, b1) = (q1[axis].min(q2[axis]), q1[axis].max(q2[axis]));
        return a1.min(b1) > a0.max(b0);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Plc {
        Plc {
            vertices: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            segments: vec![[0, 1], [1, 2], [2, 3], [3, 0]],
            ..Default::default()
        }
    }

    #[test]
    fn rejects_bad_input() {
        let mut overlap = square();
        overlap.vertices.push([0.5, 0.0]);
        overlap.segments.push([0, 4]);
        let mut duplicate = square();
        duplicate.vertices.push([1.0, 1.0]);
        let mut nan = square();
        nan.vertices[2][1] = f64::NAN;
        let mut index = square();
        index.segments.push([0, 9]);
        let cases = [
            (
                overlap,
                MeshError::SegmentsIntersect {
                    first: 0,
                    second: 4,
                },
            ),
            (
                duplicate,
                MeshError::DuplicateVertex {
                    first: 2,
                    second: 4,
                },
            ),
            (nan, MeshError::NonFiniteVertex { vertex: 2 }),
            (
                index,
                MeshError::SegmentIndex {
                    segment: 4,
                    index: 9,
                },
            ),
        ];
        for (plc, expected) in cases {
            assert_eq!(plc.validate(), Err(expected));
        }
        // A segment end on another segment, and crossing segments, are allowed.
        let mut tee = square();
        tee.vertices.extend([[0.5, 0.0], [0.5, 0.5]]);
        tee.segments.push([4, 5]);
        let mut crossing = square();
        crossing.segments.extend([[0, 2], [1, 3]]);
        assert_eq!(tee.validate(), Ok(()));
        assert_eq!(crossing.validate(), Ok(()));
    }
}
