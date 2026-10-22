use super::{Limits, Outcome, Request, StopReason};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Serialize, Deserialize)]
struct Envelope {
    request: Request,
    limits: Limits,
}

/// Worker entry point called by both binaries before starting a UI or CLI.
/// Its files live in a unique per-job temporary directory, never the workspace.
pub fn worker_main() -> Result<(), String> {
    let directory = std::env::args_os()
        .skip_while(|a| a != "--mesh-worker")
        .nth(1)
        .ok_or("--mesh-worker requires a job directory")?;
    let directory = PathBuf::from(directory);
    let input = fs::File::open(directory.join("input.json")).map_err(|e| e.to_string())?;
    let envelope: Envelope =
        serde_json::from_reader(input.take(256 * 1024 * 1024)).map_err(|e| e.to_string())?;
    envelope.limits.validate().map_err(|e| e.to_string())?;
    let limit = envelope.limits.max_output_bytes;
    let start = Instant::now();
    let mut outcome = super::run_with_limits(envelope.request, envelope.limits, move || {
        start.elapsed().as_secs_f64()
    });
    struct BoundedWriter {
        file: fs::File,
        remaining: u64,
    }
    impl Write for BoundedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() as u64 > self.remaining {
                return Err(std::io::Error::other("output budget exceeded"));
            }
            let n = self.file.write(bytes)?;
            self.remaining -= n as u64;
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.file.flush()
        }
    }
    let output = directory.join("output.json");
    let file = fs::File::create(&output).map_err(|e| e.to_string())?;
    if serde_json::to_writer(
        BoundedWriter {
            file,
            remaining: limit,
        },
        &outcome,
    )
    .is_err()
    {
        outcome = Outcome::failure(
            StopReason::OutputBudget,
            "serialized mesh exceeded output budget",
            start.elapsed().as_secs_f64(),
        );
        // The small failure record is control-plane data, not a mesh artifact.
        fs::write(
            &output,
            serde_json::to_vec(&outcome).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub struct ProcessJob {
    child: Child,
    directory: PathBuf,
    started: Instant,
    limits: Limits,
    completed: bool,
    pending: Option<Outcome>,
}
impl ProcessJob {
    pub fn start(request: Request) -> Result<Self, String> {
        let max_nodes = match &request {
            Request::TwoD { max_nodes, .. } => *max_nodes,
            Request::ThreeD { options, .. } => options.max_vertices,
        };
        Self::start_with_limits(
            request,
            Limits {
                max_nodes,
                ..Limits::default()
            },
        )
    }
    pub fn start_with_limits(request: Request, limits: Limits) -> Result<Self, String> {
        let exe = std::env::var_os("RUSTMESH_WORKER_EXE")
            .map(PathBuf::from)
            .map(Ok)
            .unwrap_or_else(std::env::current_exe)
            .map_err(|e| e.to_string())?;
        Self::start_with_executable(request, limits, exe)
    }
    pub fn start_with_executable(
        request: Request,
        limits: Limits,
        executable: PathBuf,
    ) -> Result<Self, String> {
        limits.validate().map_err(|e| e.to_string())?;
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "rustmesh-job-{}-{stamp}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).map_err(|e| e.to_string())?;
        let envelope = Envelope {
            request,
            limits: limits.clone(),
        };
        let setup = (|| {
            let file = fs::File::create(directory.join("input.json")).map_err(|e| e.to_string())?;
            serde_json::to_writer(file, &envelope).map_err(|e| e.to_string())?;
            let mut command = Command::new(executable);
            command
                .arg("--mesh-worker")
                .arg(&directory)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x08000000); // CREATE_NO_WINDOW
            }
            command
                .spawn()
                .map_err(|e| format!("could not start mesh worker: {e}"))
        })();
        match setup {
            Ok(child) => Ok(Self {
                child,
                directory,
                started: Instant::now(),
                limits,
                completed: false,
                pending: None,
            }),
            Err(e) => {
                let _ = fs::remove_dir_all(&directory);
                Err(e)
            }
        }
    }
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
    pub fn has_exited(&mut self) -> Result<bool, String> {
        self.child
            .try_wait()
            .map(|s| s.is_some())
            .map_err(|e| e.to_string())
    }
    pub fn cancel(&mut self) -> Result<(), String> {
        if self.completed {
            return Ok(());
        }
        self.terminate()?;
        self.pending = Some(Outcome::failure(
            StopReason::Cancelled,
            "mesh worker terminated by cancellation",
            self.started.elapsed().as_secs_f64(),
        ));
        Ok(())
    }
    fn terminate(&mut self) -> Result<(), String> {
        if self.child.try_wait().map_err(|e| e.to_string())?.is_none() {
            self.child.kill().map_err(|e| e.to_string())?;
        }
        self.child.wait().map_err(|e| e.to_string())?;
        self.completed = true;
        Ok(())
    }
    pub fn poll(&mut self) -> Option<Outcome> {
        if let Some(outcome) = self.pending.take() {
            return Some(outcome);
        }
        if self.completed {
            return None;
        }
        match self.child.try_wait() {
            Ok(None) => {
                if self.started.elapsed().as_secs_f64() >= self.limits.time_seconds {
                    let message = match self.terminate() {
                        Ok(()) => "wall-clock budget exhausted; worker terminated".to_string(),
                        Err(e) => e,
                    };
                    Some(Outcome::failure(
                        StopReason::TimeBudget,
                        message,
                        self.started.elapsed().as_secs_f64(),
                    ))
                } else {
                    None
                }
            }
            Err(e) => {
                let _ = self.terminate();
                Some(Outcome::failure(
                    StopReason::BackendFailure,
                    e.to_string(),
                    self.started.elapsed().as_secs_f64(),
                ))
            }
            Ok(Some(exit)) => {
                self.completed = true;
                let path = self.directory.join("output.json");
                let result = (|| {
                    if !exit.success() {
                        return Err(format!("mesh worker exited with {exit}"));
                    }
                    let file = fs::File::open(path).map_err(|e| e.to_string())?;
                    // Allow a small control-plane failure response for tiny budgets.
                    let limit = self.limits.max_output_bytes.max(16 * 1024);
                    if file.metadata().map_err(|e| e.to_string())?.len() > limit {
                        return Err("worker output exceeds the configured limit".into());
                    }
                    serde_json::from_reader(file.take(limit))
                        .map_err(|e| format!("invalid worker response: {e}"))
                })();
                Some(result.unwrap_or_else(|e| {
                    Outcome::failure(
                        StopReason::BackendFailure,
                        e,
                        self.started.elapsed().as_secs_f64(),
                    )
                }))
            }
        }
    }
}
impl Drop for ProcessJob {
    fn drop(&mut self) {
        if !self.completed {
            let _ = self.terminate();
        }
        // This directory was created by this instance, never supplied by a user.
        let _ = fs::remove_dir_all(&self.directory);
    }
}
pub struct Job {
    process: ProcessJob,
    ctx: eframe::egui::Context,
}
impl Job {
    pub fn start(ctx: &eframe::egui::Context, request: Request) -> Result<Self, String> {
        let process = ProcessJob::start(request)?;
        ctx.request_repaint_after(Duration::from_millis(30));
        Ok(Self {
            process,
            ctx: ctx.clone(),
        })
    }
    pub fn poll(&mut self) -> Option<Outcome> {
        let result = self.process.poll();
        if result.is_none() {
            self.ctx.request_repaint_after(Duration::from_millis(30));
        }
        result
    }
    pub const STOP_ENDS_WORK: bool = true;
    pub fn stop(mut self) {
        let _ = self.process.cancel();
    }
}
