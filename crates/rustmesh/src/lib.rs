//! Quality mesh generation in 2D and 3D.
//!
//! [`triangulate`] meshes a planar straight-line graph ([`Plc`]): vertices,
//! segments that must appear as mesh edges, region seeds that set a marker and
//! a largest cell area, and hole seeds. It can replace Triangle as pyGIMLi's
//! 2D mesher, under a permissive licence. [`Geometry2D`] builds that input from
//! a drawing (a domain with marked edges, shapes that are zones or holes,
//! lines, points), and [`examples`] has templates, general ones and those of
//! geophysical and groundwater surveys. [`tetrahedralize`] meshes a 3D
//! piecewise-linear complex; it is experimental.

pub mod control;
mod error;
pub mod examples;
pub mod export;
mod mesh2d;
mod mesh3d;
pub mod model;
mod plc;

pub use error::{MeshError, TriangulationError};
pub use mesh2d::{triangulate, triangulate_strict, AngleDiagnostic, Mesh2D, Options, Report};
pub use model::{Domain, Geometry2D, Line, ModelPlc, Point, Shape, Sizing, Zone};
pub use plc::{Plc, Region};

pub use mesh3d::{
    prepare_surface_patches, tetrahedralize_patched, FacetCoverage3D, PatchedMesh3D,
    PreparedSurface3D,
};
pub use mesh3d::{
    tetrahedralize, tetrahedralize_sized, Mesh3D, Options3D, Plc3D, Region3D, Report3D, Sizing3D,
};
pub use mesh3d::{tetrahedralize_quality, Quality3D, QualityReport3D};
