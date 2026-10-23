//! THIRD_PARTY_LICENSES.txt: the licence texts of the crates (and the fonts
//! inside them) that a build of the app contains.
//!
//! The crates are those `cargo tree` resolves for the target: normal
//! dependencies, without procedural macros, which run only at build time.
//! Where a crate offers a choice of licences, MIT, Zlib or a BSD licence is
//! used when the crate ships its text, and Apache-2.0 otherwise, whose text is
//! printed once at the end. A crate whose licence text cannot be found stops the
//! build until it is added to `EXTRAS`.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// Licences picked in this order when a crate offers a choice and ships the
/// text; Apache-2.0 comes after all of them.
const PREFERRED: &[&str] = &[
    "MIT",
    "Zlib",
    "BSD-3-Clause",
    "BSD-2-Clause",
    "0BSD",
    "Unlicense",
];

/// Licence texts a crate keeps outside its top-level licence files, and the
/// licences of its expression they cover.
struct Extra {
    package: &'static str,
    covers: &'static [&'static str],
    files: &'static [(&'static str, &'static str)],
}

const EXTRAS: &[Extra] = &[Extra {
    package: "epaint_default_fonts",
    covers: &["OFL-1.1", "Ubuntu-font-1.0"],
    files: &[
        ("Hack Regular font", "fonts/Hack-Regular.txt"),
        ("Noto Emoji Regular font", "fonts/OFL.txt"),
        ("Ubuntu Light font", "fonts/UFL.txt"),
        ("emoji-icon-font", "fonts/emoji-icon-font-mit-license.txt"),
    ],
}];

struct Package {
    name: String,
    version: String,
    license: String,
    authors: Vec<String>,
    repository: Option<String>,
    dir: PathBuf,
}

/// Write the licences of `package` built for `target` to `out`; returns the
/// number of crates listed.
pub fn write(
    root: &Path,
    cargo: &str,
    package: &str,
    target: &str,
    out: &Path,
) -> Result<usize, String> {
    let packages = packages(root, cargo, package, target)?;
    let mut text = format!(
        "Third-party licences of rustmesh {} ({package}, {target})\n\n\
         rustmesh is MIT OR Apache-2.0. This build also contains the {} crates below,\n\
         each under its own licence. \"Used under\" names the licence that applies\n\
         where a crate offers a choice. The Apache License 2.0 is printed once, at\n\
         the end. Written by `cargo xtask web` from Cargo.lock.\n",
        env!("CARGO_PKG_VERSION"),
        packages.len()
    );
    for p in &packages {
        entry(&mut text, p)?;
    }
    let apache = read(&root.join("LICENSE-APACHE"))?;
    write!(
        text,
        "\n{}\nApache License 2.0\n\n{apache}\n",
        "=".repeat(72)
    )
    .unwrap();
    std::fs::write(out, text).map_err(|e| format!("could not write {}: {e}", out.display()))?;
    Ok(packages.len())
}

/// The crates in the build, workspace members left out, sorted by name.
fn packages(root: &Path, cargo: &str, package: &str, target: &str) -> Result<Vec<Package>, String> {
    let tree = run(
        root,
        cargo,
        &[
            "tree",
            "-p",
            package,
            "--target",
            target,
            "-e",
            "normal,no-proc-macro",
            "--prefix",
            "none",
            "--format",
            "{p}",
        ],
    )?;
    let mut wanted = BTreeSet::new();
    for line in String::from_utf8_lossy(&tree).lines() {
        let mut words = line.split_whitespace();
        if let (Some(name), Some(version)) = (words.next(), words.next()) {
            let version = version.strip_prefix('v').unwrap_or(version);
            wanted.insert((name.to_string(), version.to_string()));
        }
    }
    let meta = run(
        root,
        cargo,
        &[
            "metadata",
            "--format-version",
            "1",
            "--filter-platform",
            target,
        ],
    )?;
    let meta: Value = serde_json::from_slice(&meta).map_err(|e| format!("cargo metadata: {e}"))?;
    let members: BTreeSet<&str> = meta["workspace_members"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut found = Vec::new();
    for p in meta["packages"].as_array().into_iter().flatten() {
        let key = (
            p["name"].as_str().unwrap_or("").to_string(),
            p["version"].as_str().unwrap_or("").to_string(),
        );
        if !wanted.remove(&key) || members.contains(p["id"].as_str().unwrap_or("")) {
            continue;
        }
        let manifest = PathBuf::from(p["manifest_path"].as_str().unwrap_or(""));
        found.push(Package {
            name: key.0,
            version: key.1,
            license: p["license"].as_str().unwrap_or("").to_string(),
            authors: p["authors"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|a| a.as_str().map(str::to_string))
                .collect(),
            repository: p["repository"].as_str().map(str::to_string),
            dir: manifest.parent().map(Path::to_path_buf).unwrap_or_default(),
        });
    }
    if let Some((name, version)) = wanted.into_iter().next() {
        return Err(format!("cargo metadata does not list {name} {version}"));
    }
    found.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    Ok(found)
}

fn entry(text: &mut String, p: &Package) -> Result<(), String> {
    let files = licence_files(&p.dir)?;
    let extra = EXTRAS.iter().find(|e| e.package == p.name);
    let terms = terms(&p.license).map_err(|e| format!("{} {}: {e}", p.name, p.version))?;
    let single = terms.len() == 1 && terms[0].len() == 1;
    let mut used = Vec::new();
    let mut texts = Vec::new();
    for alternatives in &terms {
        if extra.is_some_and(|e| alternatives.iter().all(|a| e.covers.contains(&a.as_str()))) {
            used.push(alternatives.join(" OR "));
            continue;
        }
        let mut ranked = alternatives.clone();
        ranked.sort_by_key(|a| {
            PREFERRED
                .iter()
                .position(|p| p == a)
                .unwrap_or(PREFERRED.len())
        });
        if let Some((id, file)) = ranked
            .iter()
            .filter(|a| *a != "Apache-2.0")
            .find_map(|a| file_for(a, &files, single).map(|f| (a, f)))
        {
            used.push(id.clone());
            texts.push(file);
        } else if alternatives.iter().any(|a| a == "Apache-2.0") {
            used.push("Apache-2.0 (text at the end)".into());
        } else {
            return Err(format!(
                "{} {}: no licence text for {} in {}; add it to EXTRAS in xtask/src/licenses.rs",
                p.name,
                p.version,
                alternatives.join(" OR "),
                p.dir.display()
            ));
        }
    }
    // Apache-2.0 asks for the NOTICE file to travel with the work.
    texts.extend(
        files
            .iter()
            .filter(|f| upper_name(f).starts_with("NOTICE"))
            .cloned(),
    );
    texts.dedup();

    write!(
        text,
        "\n{}\n{} {}\nLicence: {}\nUsed under: {}\n",
        "-".repeat(72),
        p.name,
        p.version,
        p.license,
        used.join(" AND ")
    )
    .unwrap();
    if !p.authors.is_empty() {
        writeln!(text, "Authors: {}", p.authors.join(", ")).unwrap();
    }
    if let Some(repository) = &p.repository {
        writeln!(text, "Source: {repository}").unwrap();
    }
    for file in &texts {
        let name = file.file_name().unwrap_or_default().to_string_lossy();
        write!(text, "\n[{name}]\n{}\n", read(file)?).unwrap();
    }
    for (label, path) in extra.map(|e| e.files).unwrap_or_default() {
        write!(text, "\n[{label}: {path}]\n{}\n", read(&p.dir.join(path))?).unwrap();
    }
    Ok(())
}

/// The AND-terms of a licence expression, each as its OR-alternatives:
/// "(MIT OR Apache-2.0) AND OFL-1.1" gives [[MIT, Apache-2.0], [OFL-1.1]].
fn terms(expression: &str) -> Result<Vec<Vec<String>>, String> {
    let expression = expression.replace('/', " OR ");
    let mut parts = Vec::new();
    let (mut depth, mut start, mut i) = (0, 0, 0);
    while i < expression.len() {
        match expression.as_bytes()[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ if depth == 0 && expression.as_bytes()[i..].starts_with(b" AND ") => {
                parts.push(&expression[start..i]);
                i += 5;
                start = i;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(&expression[start..]);
    parts
        .into_iter()
        .map(|part| {
            let part = part.trim();
            let part = part
                .strip_prefix('(')
                .and_then(|p| p.strip_suffix(')'))
                .unwrap_or(part);
            let alternatives: Vec<String> =
                part.split(" OR ").map(|a| a.trim().to_string()).collect();
            if alternatives
                .iter()
                .any(|a| a.is_empty() || a.contains(['(', ')', ' ']))
            {
                Err(format!("cannot read the licence expression {expression:?}"))
            } else {
                Ok(alternatives)
            }
        })
        .collect()
}

/// The licence, copying and notice files at the top of a crate.
fn licence_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("could not read {}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|f| f.is_file())
        .filter(|f| {
            let name = upper_name(f);
            ["LICEN", "COPYING", "NOTICE", "UNLICENSE"]
                .iter()
                .any(|p| name.starts_with(p))
        })
        .collect();
    files.sort();
    Ok(files)
}

/// The file holding licence `id`: one named for it (LICENSE-MIT, LICENSE.ZLIB),
/// or the only licence file of a crate with a single licence.
fn file_for(id: &str, files: &[PathBuf], single: bool) -> Option<PathBuf> {
    let key = match id {
        "MIT" => "MIT",
        "Zlib" => "ZLIB",
        "BSD-3-Clause" | "BSD-2-Clause" => "BSD",
        "0BSD" => "0BSD",
        "Unlicense" => "UNLICENSE",
        "Unicode-3.0" | "Unicode-DFS-2016" => "UNICODE",
        _ => "",
    };
    let licences: Vec<&PathBuf> = files
        .iter()
        .filter(|f| !upper_name(f).starts_with("NOTICE"))
        .collect();
    if !key.is_empty() {
        let named = licences.iter().find(|f| {
            let name = upper_name(f);
            name.contains(key) && !(key == "BSD" && name.contains("0BSD"))
        });
        if let Some(f) = named {
            return Some((*f).clone());
        }
    }
    match licences[..] {
        [only] if single => Some(only.clone()),
        _ => None,
    }
}

fn upper_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_uppercase()
}

fn read(path: &Path) -> Result<String, String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes)
        .replace("\r\n", "\n")
        .trim_end()
        .to_string())
}

fn run(root: &Path, cargo: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new(cargo)
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|e| format!("could not run cargo {}: {e}", args[0]))?;
    if !output.status.success() {
        return Err(format!(
            "cargo {} failed: {}",
            args[0],
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}
