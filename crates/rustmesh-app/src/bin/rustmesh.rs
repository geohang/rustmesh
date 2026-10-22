//! rustmesh on the command line: write an example project, mesh a project
//! into a mesh file. For scripts and batches; the projects are the ones the
//! mesh builder app saves and opens.

use rustmesh_app::project;
use rustmesh_app::{mcp, workspace::Workspace};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use std::fs;
use std::process::ExitCode;

use project::Project;
use rustmesh::examples::Example;

const USAGE: &str = "\
usage:
  rustmesh templates                     list the templates and their parameters
  rustmesh template <name> [key=value ...] -o <project.json>
                                         write a template as a project
  rustmesh mesh <project.json> -o <mesh> mesh a project; <mesh> ends in
                                         .msh (Gmsh 2.2, read by pyGIMLi) or .vtu (ParaView)
  rustmesh mcp --workspace <directory>    serve the 12 domain tools over stdio
  rustmesh mcp --attach <loopback:port>    attach using RUSTMESH_ATTACH_TOKEN
  rustmesh workflow --workspace <dir> -o <workflow.json>
  rustmesh replay <workflow.json> --workspace <new-directory>";

fn main() -> ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--mesh-worker") {
        return match rustmesh_app::jobs::worker_main() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        };
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args.iter().map(String::as_str).collect::<Vec<_>>()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rustmesh: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[&str]) -> Result<(), String> {
    match *args {
        ["mcp", "--workspace", root] => {
            mcp::serve_stdio(Arc::new(Mutex::new(Workspace::new(PathBuf::from(root))?)))?;
        }
        ["mcp", "--attach", address] => mcp::serve_attach_stdio(address)?,
        ["replay", workflow, "--workspace", root] => {
            let mut workspace = Workspace::new(PathBuf::from(root))?;
            let result = workspace.replay(std::path::Path::new(workflow))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?
            );
        }
        ["workflow", "--workspace", root, "-o", output] => {
            Workspace::new(PathBuf::from(root))?.export_workflow(std::path::Path::new(output))?;
        }
        [] | ["help"] | ["-h"] | ["--help"] => println!("{USAGE}"),
        ["templates"] => {
            for e in Example::ALL {
                let params: Vec<String> = e
                    .params()
                    .iter()
                    .map(|p| format!("{}={}", p.key, p.default))
                    .collect();
                println!(
                    "{:<18} {:<11} {}  [{}]",
                    e.key(),
                    e.field().title(),
                    e.title(),
                    params.join(" ")
                );
            }
        }
        ["template", key, ref rest @ ..] if rest.len() >= 2 && rest[rest.len() - 2] == "-o" => {
            let out = rest[rest.len() - 1];
            let example = Example::from_key(key)
                .ok_or_else(|| format!("no template \"{key}\"; see: rustmesh templates"))?;
            let geometry = example.build(&values(example, &rest[..rest.len() - 2])?);
            let json = Project::new(geometry, 5_000_000).to_json();
            fs::write(out, json).map_err(|e| format!("could not write {out}: {e}"))?;
        }
        ["mesh", input, "-o", out] => mesh(input, out)?,
        _ => return Err(format!("did not understand the command\n{USAGE}")),
    }
    Ok(())
}

/// A template's parameters from `key=value` arguments, defaults for the rest.
fn values(example: Example, args: &[&str]) -> Result<Vec<f64>, String> {
    let params = example.params();
    let mut values = example.defaults();
    for arg in args {
        let (key, value) = arg
            .split_once('=')
            .ok_or_else(|| format!("expected key=value, not \"{arg}\""))?;
        let i = params.iter().position(|p| p.key == key).ok_or_else(|| {
            let keys: Vec<&str> = params.iter().map(|p| p.key).collect();
            format!(
                "{} has no parameter \"{key}\" (it has: {})",
                example.key(),
                keys.join(", ")
            )
        })?;
        values[i] = value
            .parse()
            .map_err(|_| format!("{key}: \"{value}\" is not a number"))?;
    }
    Ok(values)
}

fn mesh(input: &str, out: &str) -> Result<(), String> {
    let text = fs::read_to_string(input).map_err(|e| format!("could not read {input}: {e}"))?;
    let format = match out.rsplit('.').next() {
        Some("msh") => "msh",
        Some("vtu") => "vtu",
        _ => return Err(format!("{out}: the mesh file must end in .msh or .vtu")),
    };
    let root = std::env::temp_dir().join(format!("rustmesh-cli-{}", uuid::Uuid::new_v4()));
    let mut workspace = Workspace::new(root.clone())?;
    let project = workspace
        .import_project(&text)
        .map_err(|e| format!("{input}: {e}"))?;
    let started=workspace.call("generate_mesh",serde_json::json!({"schema_version":1,"operation_id":"cli-generate","project_id":project.project_id,"revision":project.revision,"backend":"rustmesh","budget":{"max_nodes":project.max_nodes,"max_cells":project.max_nodes.saturating_mul(2),"wall_seconds":600.0,"max_output_bytes":134217728}}));
    if started["ok"] != true {
        return Err(started["error"].to_string());
    }
    let job_id = started["result"]["job_id"]
        .as_str()
        .ok_or("worker returned no job ID")?;
    let mesh_id = loop {
        let polled = workspace.call(
            "read_job",
            serde_json::json!({"schema_version":1,"job_id":job_id}),
        );
        if polled["ok"] != true {
            return Err(polled["error"].to_string());
        }
        let job = &polled["result"];
        match job["state"].as_str() {
            Some("running") => std::thread::sleep(std::time::Duration::from_millis(10)),
            Some("completed") => {
                break job["mesh_id"]
                    .as_str()
                    .ok_or("completed worker has no mesh ID")?
                    .to_owned()
            }
            _ => return Err(format!("mesh job stopped: {job}")),
        }
    };
    let exported=workspace.call("export_mesh",serde_json::json!({"schema_version":1,"operation_id":"cli-export","mesh_id":mesh_id,"format":format,"filename":format!("mesh.{format}"),"allow_partial":true}));
    if exported["ok"] != true {
        return Err(exported["error"].to_string());
    }
    // The CLI's explicit -o path is user authorization for this file destination.
    fs::copy(root.join("exports").join(format!("mesh.{format}")), out)
        .map_err(|e| format!("could not write {out}: {e}"))?;
    let record = workspace.mesh(&mesh_id).ok_or("missing mesh artifact")?;
    let mesh = &record.mesh;
    let r = &mesh.report;
    eprintln!(
        "{out}: {} nodes, {} cells, smallest angle {:.1}°, meshed in {:.2} s",
        mesh.nodes.len(),
        mesh.triangles.len(),
        r.min_angle,
        record.report["seconds"].as_f64().unwrap_or(0.0)
    );
    for line in r.issues.iter().chain(&r.warnings) {
        eprintln!("  {line}");
    }
    if r.complete {
        Ok(())
    } else {
        Err("the mesh does not meet every limit (see above); it was written anyway".into())
    }
}
