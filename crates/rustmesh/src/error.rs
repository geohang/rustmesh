use std::fmt;

/// Why a mesh could not be built.
#[derive(Debug, Clone, PartialEq)]
pub enum MeshError {
    Stopped(crate::control::StopReason),
    TooFewVertices,
    NonFiniteVertex {
        vertex: usize,
    },
    DuplicateVertex {
        first: usize,
        second: usize,
    },
    MarkerCount {
        what: &'static str,
        expected: usize,
        got: usize,
    },
    SegmentIndex {
        segment: usize,
        index: usize,
    },
    ZeroLengthSegment {
        segment: usize,
    },
    SegmentsIntersect {
        first: usize,
        second: usize,
    },
    NonFiniteSeed {
        what: &'static str,
        index: usize,
    },
    BadOption(String),
    Insertion(String),
    Internal(String),
}

impl fmt::Display for MeshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stopped(reason) => write!(f, "meshing stopped: {reason:?}"),
            Self::TooFewVertices => write!(f, "a mesh needs at least three vertices"),
            Self::NonFiniteVertex { vertex } => {
                write!(f, "vertex {vertex} has a coordinate that is not finite")
            }
            Self::DuplicateVertex { first, second } => {
                write!(f, "vertices {first} and {second} are at the same position")
            }
            Self::MarkerCount {
                what,
                expected,
                got,
            } => {
                write!(f, "{what}: expected {expected} values, got {got}")
            }
            Self::SegmentIndex { segment, index } => {
                write!(
                    f,
                    "segment {segment} refers to vertex {index}, which does not exist"
                )
            }
            Self::ZeroLengthSegment { segment } => {
                write!(f, "segment {segment} starts and ends at the same vertex")
            }
            Self::SegmentsIntersect { first, second } => write!(
                f,
                "segments {first} and {second} cross or overlap; segments may only meet at vertices"
            ),
            Self::NonFiniteSeed { what, index } => {
                write!(f, "{what} seed {index} has a coordinate that is not finite")
            }
            Self::BadOption(msg) => write!(f, "{msg}"),
            Self::Insertion(msg) => write!(f, "could not build the triangulation: {msg}"),
            Self::Internal(msg) => write!(f, "internal meshing error: {msg}"),
        }
    }
}

impl std::error::Error for MeshError {}

/// Failure from [`crate::triangulate_strict`]. Invalid input and backend errors
/// never carry a mesh. A meshing call that finishes but misses its requested
/// refinement or quality goals keeps its mesh, so the caller can inspect it.
#[derive(Debug, Clone)]
pub enum TriangulationError {
    /// No usable result was produced (including cancellation and invalid input).
    Meshing(MeshError),
    /// A nonempty, positively oriented mesh did not meet all requested goals.
    /// The goals are the requested limits only, not finite-element suitability.
    Incomplete(Box<crate::Mesh2D>),
}
impl TriangulationError {
    /// The partial mesh, for an incomplete result only.
    pub fn mesh(&self) -> Option<&crate::Mesh2D> {
        match self {
            Self::Incomplete(mesh) => Some(mesh),
            Self::Meshing(_) => None,
        }
    }
    /// The report that belongs to the partial mesh.
    pub fn report(&self) -> Option<&crate::Report> {
        self.mesh().map(|mesh| &mesh.report)
    }
    /// Takes ownership of the partial mesh.
    pub fn into_mesh(self) -> Option<crate::Mesh2D> {
        match self {
            Self::Incomplete(mesh) => Some(*mesh),
            Self::Meshing(_) => None,
        }
    }
}
impl fmt::Display for TriangulationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Meshing(error) => error.fmt(f),
            Self::Incomplete(mesh) => write!(
                f,
                "requested mesh goals were not met: {}",
                mesh.report.issues.join("; ")
            ),
        }
    }
}
impl std::error::Error for TriangulationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Meshing(error) => Some(error),
            Self::Incomplete(_) => None,
        }
    }
}
impl From<MeshError> for TriangulationError {
    fn from(error: MeshError) -> Self {
        Self::Meshing(error)
    }
}
