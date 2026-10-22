//! Build script. It records a hash of the sources as `RUSTMESH_BUILD_ID`, which
//! the app stores in job records and saved workflows, and on windows-gnu it can
//! add a folder of extra import libraries to the link search path.
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn collect(path: &Path, files: &mut Vec<PathBuf>) {
    if path.is_dir() {
        for entry in fs::read_dir(path).expect("source directory") {
            collect(&entry.expect("source entry").path(), files);
        }
    } else if path.extension().is_some_and(|e| e == "rs" || e == "toml") {
        files.push(path.to_owned());
    }
}

fn main() {
    // rustup's GNU toolchain without a full MinGW has no import library for some
    // system DLLs (shlwapi). A folder named here is searched for them.
    println!("cargo:rerun-if-env-changed=RUSTMESH_WINDOWS_LIB_DIR");
    if std::env::var("TARGET").is_ok_and(|t| t == "x86_64-pc-windows-gnu") {
        if let Some(path) = std::env::var_os("RUSTMESH_WINDOWS_LIB_DIR") {
            println!(
                "cargo:rustc-link-search=native={}",
                PathBuf::from(path).display()
            );
        }
    }

    // Hash the sources of the mesher, the app and the vendored triangulation
    // library, with their manifests.
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let mut files = Vec::new();
    for rel in [
        "crates/rustmesh/src",
        "crates/rustmesh-app/src",
        "vendor/spade/src",
    ] {
        collect(&root.join(rel), &mut files);
    }
    for rel in [
        "Cargo.toml",
        "Cargo.lock",
        "crates/rustmesh/Cargo.toml",
        "crates/rustmesh-app/Cargo.toml",
        "crates/rustmesh-app/build.rs",
        "vendor/spade/Cargo.toml",
    ] {
        files.push(root.join(rel));
    }
    files.sort();
    let mut digest = Sha256::new();
    for path in files {
        println!("cargo:rerun-if-changed={}", path.display());
        let rel = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        digest.update(rel.as_bytes());
        digest.update([0]);
        digest.update(fs::read(path).expect("read source"));
        digest.update([0]);
    }
    println!(
        "cargo:rustc-env=RUSTMESH_BUILD_ID=source-sha256:{:x}",
        digest.finalize()
    );
}
