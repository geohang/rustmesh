//! Build tasks. `cargo xtask web` builds the web app into `dist/web`:
//! the app compiled to WebAssembly, its JavaScript glue, the page, the
//! meshing worker and the licences of the crates and fonts compiled into it.
//! Serve that folder with any static file server.

use std::path::{Path, PathBuf};
use std::process::Command;

mod licenses;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["web"] => web(true),
        ["web", "--debug"] => web(false),
        ["licenses"] => web_licenses(&root().join("dist/web")),
        _ => Err("usage: cargo xtask web [--debug] | cargo xtask licenses".into()),
    };
    if let Err(e) = result {
        eprintln!("xtask: {e}");
        std::process::exit(1);
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace")
        .to_path_buf()
}

fn web(release: bool) -> Result<(), String> {
    let root = root();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut build = Command::new(&cargo);
    build.current_dir(&root).args([
        "build",
        "-p",
        "rustmesh-app",
        "--bin",
        "rustmesh-app",
        "--target",
        "wasm32-unknown-unknown",
    ]);
    if release {
        build.arg("--release");
    }
    let status = build
        .status()
        .map_err(|e| format!("could not run cargo: {e}"))?;
    if !status.success() {
        return Err("the WebAssembly build failed (is the target installed? rustup target add wasm32-unknown-unknown)".into());
    }
    let profile = if release { "release" } else { "debug" };
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let wasm = target
        .join("wasm32-unknown-unknown")
        .join(profile)
        .join("rustmesh-app.wasm");
    let out = root.join("dist/web");
    std::fs::create_dir_all(&out)
        .map_err(|e| format!("could not create {}: {e}", out.display()))?;
    wasm_bindgen_cli_support::Bindgen::new()
        .input_path(&wasm)
        .out_name("rustmesh_app")
        .typescript(false)
        // Let init() find rustmesh_app_bg.wasm next to the script (the
        // library's default, unlike the command-line tool's, leaves it out).
        .omit_default_module_path(false)
        .web(true)
        .map_err(|e| e.to_string())?
        .generate(&out)
        .map_err(|e| format!("wasm-bindgen: {e}"))?;
    for file in ["index.html", "worker.js"] {
        let from = root.join("crates/rustmesh-app/web").join(file);
        std::fs::copy(&from, out.join(file))
            .map_err(|e| format!("could not copy {}: {e}", from.display()))?;
    }
    web_licenses(&out)?;
    let size = std::fs::metadata(out.join("rustmesh_app_bg.wasm"))
        .map(|m| m.len())
        .unwrap_or(0);
    println!(
        "Built {} ({:.1} MB of WebAssembly).",
        out.display(),
        size as f64 / 1e6
    );
    println!("Try it: python -m http.server 8000 -d dist/web, then open http://localhost:8000");
    Ok(())
}

/// The licences of the crates and fonts in the web app, written to
/// `out/THIRD_PARTY_LICENSES.txt`.
fn web_licenses(out: &Path) -> Result<(), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    std::fs::create_dir_all(out).map_err(|e| format!("could not create {}: {e}", out.display()))?;
    let crates = licenses::write(
        &root(),
        &cargo,
        "rustmesh-app",
        "wasm32-unknown-unknown",
        &out.join("THIRD_PARTY_LICENSES.txt"),
    )?;
    println!("Wrote THIRD_PARTY_LICENSES.txt ({crates} crates).");
    Ok(())
}
