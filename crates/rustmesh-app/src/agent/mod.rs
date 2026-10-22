//! Optional connection to Codex for the app's agent panel. The mesher itself has no provider dependency.
mod auth;
mod process;

pub use auth::{ModelChoice, Registration};
pub use process::SUPPORTED_VERSION;
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    thread,
    time::Duration,
};

pub fn detect_codex() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("RUSTMESH_CODEX")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    {
        return Some(path);
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(if cfg!(windows) { "codex.exe" } else { "codex" });
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub struct AgentConfig {
    pub codex: PathBuf,
    /// Application-owned account metadata and isolated Codex history, outside the project archive.
    pub data_dir: PathBuf,
    /// Empty, application-owned directory. Project access is exclusively through MCP attach.
    pub workspace_dir: PathBuf,
    pub mcp_command: PathBuf,
    pub mcp_args: Vec<String>,
    pub mcp_token: String,
}

#[derive(Debug)]
pub enum AgentEvent {
    Status(String),
    Auth { label: String, plan_use: bool },
    Models(Vec<ModelChoice>),
    Delta(String),
    Tool { name: String, status: String },
    Usage(Value),
    Completed { status: String, tool_calls: usize },
    Error(String),
    Disconnected,
}
enum Action {
    SignIn(Option<usize>),
    Message(String, String),
    Cancel,
    Disconnect,
    Shutdown,
}
pub struct AgentController {
    commands: Sender<Action>,
    events: Receiver<AgentEvent>,
    cancel: Arc<AtomicBool>,
    data_dir: PathBuf,
}
impl AgentController {
    pub fn new(config: AgentConfig) -> Self {
        let (commands, rx) = mpsc::channel();
        let (tx, events) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let data_dir = config.data_dir.clone();
        thread::spawn(move || run(config, rx, tx, worker_cancel));
        Self {
            commands,
            events,
            cancel,
            data_dir,
        }
    }
    pub fn registrations(&self) -> Vec<Registration> {
        auth::load_registrations(&self.data_dir)
    }
    /// None creates a distinct account/workspace registration; Some(index) reuses its issued client ID.
    pub fn begin_sign_in(&self, registration: Option<usize>) {
        self.cancel.store(false, Ordering::Relaxed);
        let _ = self.commands.send(Action::SignIn(registration));
    }
    pub fn send_message(&self, model: &str, text: &str) {
        self.cancel.store(false, Ordering::Relaxed);
        let _ = self
            .commands
            .send(Action::Message(model.into(), text.into()));
    }
    pub fn poll(&self) -> Vec<AgentEvent> {
        self.events.try_iter().collect()
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        let _ = self.commands.send(Action::Cancel);
    }
    /// Ends only rustmesh's renewable session; never logs the user out of another app.
    pub fn disconnect(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        let _ = self.commands.send(Action::Disconnect);
    }
}
impl Drop for AgentController {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        let _ = self.commands.send(Action::Shutdown);
    }
}

fn run(
    config: AgentConfig,
    commands: Receiver<Action>,
    events: Sender<AgentEvent>,
    cancel: Arc<AtomicBool>,
) {
    let mut credentials: Option<auth::Credentials> = None;
    let mut server: Option<process::Server> = None;
    let mut thread_id: Option<String> = None;
    let mut available_models = Vec::<String>::new();
    let emit_error = |message: String| {
        let _ = events.send(AgentEvent::Error(message));
    };
    match process::version(&config) {
        Ok(version) => {
            let _=events.send(AgentEvent::Status(format!("Detected {version}. Account connection requires rustmesh sign-in; credentials are session-only.")));
        }
        Err(error) => emit_error(error),
    }
    loop {
        if let Some(active) = server.as_mut() {
            if let Err(error) = active.poll() {
                thread_id = active.thread_id.clone();
                server = None;
                emit_error(error);
            }
        }
        // Only this thread refreshes tokens. Restart the runtime before resuming a thread.
        if credentials
            .as_ref()
            .is_some_and(|c| c.direct && c.expires_at <= auth::now() + 120)
        {
            if let Some(active) = server.take() {
                thread_id = active.thread_id.clone();
                if active.turn_id.is_some() {
                    let _ = events.send(AgentEvent::Completed {
                        status: "interrupted".into(),
                        tool_calls: active.completed_tool_calls,
                    });
                }
                drop(active);
                let _=events.send(AgentEvent::Status("Token renewal stopped the runtime; unfinished turns are not automatically replayed.".into()));
            }
            match auth::refresh(credentials.as_ref().unwrap()) {
                Ok(value) => {
                    if !value.direct {
                        available_models.clear();
                        let _ = events.send(AgentEvent::Auth {
                            label: value.registration.email.clone(),
                            plan_use: false,
                        });
                        let _ = events.send(AgentEvent::Models(vec![]));
                        emit_error("Renewed identity no longer grants ChatGPT plan use; reauthorize this registration.".into());
                    }
                    credentials = Some(value);
                    let _ = events.send(AgentEvent::Status(
                        "Session renewed; next message resumes the saved thread.".into(),
                    ));
                }
                Err(error) => {
                    credentials = None;
                    available_models.clear();
                    let _ = events.send(AgentEvent::Disconnected);
                    emit_error(error);
                }
            }
        }
        let action = match commands.recv_timeout(Duration::from_millis(50)) {
            Ok(a) => a,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        };
        match action {
            Action::SignIn(index) => {
                if cancel.load(Ordering::Relaxed) {
                    continue;
                }
                let registration = match index {
                    Some(i) => match auth::load_registrations(&config.data_dir).get(i) {
                        Some(r) => Some(r.clone()),
                        None => {
                            emit_error("Saved account no longer exists".into());
                            continue;
                        }
                    },
                    None => None,
                };
                let _=events.send(AgentEvent::Status("Complete Continue with ChatGPT in your system browser. Waiting up to five minutes.".into()));
                match auth::sign_in(
                    &config.data_dir,
                    registration,
                    credentials.as_ref(),
                    &cancel,
                ) {
                    Ok(value) => {
                        server = None;
                        thread_id = None;
                        available_models.clear();
                        let label = format!(
                            "{} ({})",
                            if value.registration.email.is_empty() {
                                &value.registration.subject
                            } else {
                                &value.registration.email
                            },
                            value.registration.client_id
                        );
                        let _ = events.send(AgentEvent::Auth {
                            label,
                            plan_use: value.direct,
                        });
                        match auth::models(&value) {
                            Ok(models) => {
                                available_models = models.iter().map(|m| m.slug.clone()).collect();
                                let _ = events.send(AgentEvent::Models(models));
                            }
                            Err(error) => emit_error(error),
                        }
                        credentials = Some(value);
                    }
                    Err(error) => emit_error(error),
                }
            }
            Action::Message(model, text) => {
                if cancel.load(Ordering::Relaxed) {
                    continue;
                }
                let Some(account) = credentials.as_ref() else {
                    emit_error("Use Continue with ChatGPT before sending a message".into());
                    continue;
                };
                if !account.direct || !available_models.contains(&model) {
                    emit_error("Select a model from this authorized account's catalog".into());
                    continue;
                }
                if text.trim().is_empty() || text.len() > 128 * 1024 {
                    emit_error("Message must be nonempty and at most 128 KiB".into());
                    continue;
                }
                if server.is_none() {
                    match process::Server::start(
                        &config,
                        &account.access_token,
                        events.clone(),
                        thread_id.clone(),
                        &model,
                        cancel.clone(),
                    ) {
                        Ok(active) => {
                            thread_id = active.thread_id.clone();
                            server = Some(active);
                        }
                        Err(error) => {
                            emit_error(error);
                            continue;
                        }
                    }
                }
                if let Err(error) = server.as_mut().unwrap().send_message(&model, &text) {
                    emit_error(error);
                }
            }
            Action::Cancel => {
                if let Some(active) = server.as_mut() {
                    if let Err(error) = active.cancel() {
                        emit_error(error);
                    }
                }
                // Kill even if interrupt acknowledgement succeeds: no agent-owned retry survives Stop.
                if let Some(active) = server.take() {
                    thread_id = active.thread_id.clone();
                    drop(active);
                }
                let _ = events.send(AgentEvent::Completed {
                    status: "interrupted".into(),
                    tool_calls: 0,
                });
                let _ = events.send(AgentEvent::Status(
                    "Agent stopped. Mesh job cancellation is tracked separately in the workspace."
                        .into(),
                ));
            }
            Action::Disconnect => {
                server = None;
                thread_id = None;
                available_models.clear();
                if let Some(value) = credentials.take() {
                    if let Err(error) = auth::revoke(&value) {
                        emit_error(error);
                    }
                }
                let _ = events.send(AgentEvent::Disconnected);
            }
            Action::Shutdown => break,
        }
    }
    drop(server);
    // Memory-only tokens are dropped. Explicit Disconnect attempts remote revocation.
    drop(credentials);
}
