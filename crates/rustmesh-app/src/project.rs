//! Project files: the geometry and settings that made a mesh, not the mesh,
//! so a project started in the browser opens in the desktop app and the
//! other way round.

use rustmesh::Geometry2D;
use serde::{Deserialize, Serialize};

pub const FORMAT: &str = "rustmesh-project";
pub const VERSION: u32 = 2;
pub const EXTENSION: &str = "rmesh.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub format: String,
    pub version: u32,
    /// rustmesh version that wrote the file.
    pub rustmesh: String,
    pub geometry: Geometry2D,
    /// Most nodes a mesh of this project may have.
    pub max_nodes: usize,
}

impl Project {
    pub fn new(geometry: Geometry2D, max_nodes: usize) -> Self {
        Self {
            format: FORMAT.into(),
            version: VERSION,
            rustmesh: env!("CARGO_PKG_VERSION").into(),
            geometry,
            max_nodes,
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a project always serializes")
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct Header {
            format: String,
            version: u32,
            #[serde(default)]
            rustmesh: String,
        }
        let header: Header = serde_json::from_str(text)
            .map_err(|e| format!("this is not a rustmesh project file ({e})"))?;
        if header.format != FORMAT {
            return Err(format!(
                "this file is a \"{}\", not a rustmesh project",
                header.format
            ));
        }
        if header.version > VERSION {
            return Err(format!(
                "this project was saved by a newer rustmesh ({}); update the app to open it",
                header.rustmesh
            ));
        }
        if header.version < VERSION {
            return Err(
                "this project was saved by a preview of the app whose files are no longer read; \
                        create it again from a template"
                    .into(),
            );
        }
        serde_json::from_str(text).map_err(|e| format!("the project file is damaged ({e})"))
    }
}
