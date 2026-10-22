//! Shared application services used by the desktop, CLI and MCP transports.
pub mod app;
mod edit;
mod files;
pub mod jobs;
pub mod project;
mod studio3d;
mod theme;
mod view;
mod view3d;

#[cfg(not(target_arch = "wasm32"))]
pub mod agent;
#[cfg(not(target_arch = "wasm32"))]
mod desktop;
#[cfg(not(target_arch = "wasm32"))]
pub mod mcp;
#[cfg(not(target_arch = "wasm32"))]
pub mod tools;
#[cfg(not(target_arch = "wasm32"))]
pub mod workspace;
