//! Stdio client for the Codex app-server, pinned to one Codex CLI version.
use super::{AgentConfig, AgentEvent};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

pub const SUPPORTED_VERSION: &str = "codex-cli 0.163.0-alpha.10.2";

fn failure_message(value: &Value) -> String {
    // Classify provider-owned details without forwarding arbitrary error bodies or credentials.
    let text = value.to_string().to_ascii_lowercase();
    if text.contains("usage_limit")
        || text.contains("quota")
        || text.contains("ratelimit")
        || text.contains("rate_limit")
    {
        "ChatGPT usage or rate limit reached. Review ChatGPT Settings → Usage; no billed API fallback was selected.".into()
    } else if text.contains("unauthorized")
        || text.contains("invalid_token")
        || text.contains("token_expired")
        || text.contains("authentication")
    {
        "ChatGPT authorization expired or was rejected. Reauthorize the saved rustmesh account."
            .into()
    } else if text.contains("policy") || text.contains("permission") || text.contains("forbidden") {
        "ChatGPT account or workspace policy rejected this operation. Check the selected account's plan-use permission.".into()
    } else {
        "The Codex runtime or the model request failed. Check the connection, the account usage and the Codex version.".into()
    }
}

pub struct Server {
    child: Child,
    input: ChildStdin,
    output: Receiver<Value>,
    events: Sender<AgentEvent>,
    next_id: u64,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub completed_tool_calls: usize,
    cancelled: Arc<AtomicBool>,
}

fn hidden(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(not(windows))]
    let _ = command;
}
pub fn version(config: &AgentConfig) -> Result<String, String> {
    let mut command = Command::new(&config.codex);
    hidden(&mut command);
    let output = command.arg("--version").output().map_err(|_| {
        "Official Codex executable not found; install Codex CLI or select its executable"
    })?;
    if !output.status.success() {
        return Err("Codex version check failed".into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().into())
}
pub fn overrides(config: &AgentConfig) -> Vec<String> {
    let quoted = |v: &str| serde_json::to_string(v).unwrap();
    let mut values = vec![
        "model_provider=\"openai_chatgpt_plan\"".into(),
        "model_providers.openai_chatgpt_plan.name=\"ChatGPT plan\"".into(),
        "model_providers.openai_chatgpt_plan.base_url=\"https://api.openai.com/v1\"".into(),
        "model_providers.openai_chatgpt_plan.env_key=\"RUSTMESH_CHATGPT_ACCESS_TOKEN\"".into(),
        "model_providers.openai_chatgpt_plan.wire_api=\"responses\"".into(),
        "model_providers.openai_chatgpt_plan.requires_openai_auth=false".into(),
        "model_providers.openai_chatgpt_plan.supports_websockets=false".into(),
        "sandbox_mode=\"read-only\"".into(),
        "approval_policy=\"never\"".into(),
        "web_search=\"disabled\"".into(),
        "project_doc_max_bytes=0".into(),
        "shell_environment_policy.inherit=\"none\"".into(),
        "analytics.enabled=false".into(),
        format!(
            "mcp_servers.rustmesh.command={}",
            quoted(&config.mcp_command.to_string_lossy())
        ),
        format!(
            "mcp_servers.rustmesh.args={}",
            serde_json::to_string(&config.mcp_args).unwrap()
        ),
        "mcp_servers.rustmesh.env_vars=[\"RUSTMESH_ATTACH_TOKEN\"]".into(),
        "mcp_servers.rustmesh.startup_timeout_sec=30".into(),
        "mcp_servers.rustmesh.tool_timeout_sec=60".into(),
        "mcp_servers.rustmesh.required=true".into(),
    ];
    for feature in [
        "shell_tool",
        "unified_exec",
        "apps",
        "plugins",
        "hooks",
        "browser_use",
        "browser_use_external",
        "computer_use",
        "image_generation",
        "multi_agent",
        "multi_agent_v2",
        "code_mode",
        "code_mode_tool_search",
        "skill_search",
        "skill_mcp_dependency_install",
        "memories",
        "workspace_dependencies",
        "in_app_local_automation",
        "remote_plugin",
        "tool_suggest",
        "view_image",
        "goals",
    ] {
        values.push(format!("features.{feature}=false"));
    }
    values.push("features.skip_host_skill_discovery=true".into());
    values
}
impl Server {
    pub fn start(
        config: &AgentConfig,
        access_token: &str,
        events: Sender<AgentEvent>,
        resume: Option<String>,
        model: &str,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let actual = version(config)?;
        if actual != SUPPORTED_VERSION {
            return Err(format!(
                "Unsupported Codex runtime: {actual}. Supported version: {SUPPORTED_VERSION}."
            ));
        }
        let home = config.data_dir.join("codex-runtime");
        std::fs::create_dir_all(&home).map_err(|_| "Cannot create app-server runtime directory")?;
        std::fs::create_dir_all(&config.workspace_dir)
            .map_err(|_| "Cannot create isolated agent workspace")?;
        let mut command = Command::new(&config.codex);
        hidden(&mut command);
        command
            .arg("app-server")
            .args(["--listen", "stdio://", "--strict-config"]);
        for setting in overrides(config) {
            command.arg("-c").arg(setting);
        }
        command
            .current_dir(&config.workspace_dir)
            .env("CODEX_HOME", &home)
            .env("RUSTMESH_CHATGPT_ACCESS_TOKEN", access_token)
            .env("RUSTMESH_ATTACH_TOKEN", &config.mcp_token)
            .env_remove("OPENAI_API_KEY")
            .env_remove("CODEX_ACCESS_TOKEN")
            .env_remove("OPENAI_BASE_URL")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .map_err(|_| "Cannot start official Codex app-server")?;
        let input = child.stdin.take().ok_or("Missing app-server input")?;
        let stdout = child.stdout.take().ok_or("Missing app-server output")?;
        let (tx, output) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if line.len() > 8 * 1024 * 1024 {
                    break;
                }
                let Ok(value) = serde_json::from_str(&line) else {
                    continue;
                };
                if tx.send(value).is_err() {
                    break;
                }
            }
        });
        let mut server = Self {
            child,
            input,
            output,
            events,
            next_id: 1,
            thread_id: None,
            turn_id: None,
            completed_tool_calls: 0,
            cancelled,
        };
        server.request("initialize",json!({"clientInfo":{"name":"rustmesh","title":"rustmesh","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":false}}))?;
        server.write(json!({"method":"initialized"}))?;
        let mut params = json!({"model":model,"modelProvider":"openai_chatgpt_plan","cwd":config.workspace_dir,"approvalPolicy":"never","sandbox":"read-only","developerInstructions":"Use only rustmesh MCP domain tools to operate the attached project. Never use shell, file editing, web search or other providers. Mesh job completion is asynchronous: poll read_job, inspect reports, and distinguish geometry validity from requested quality. Preserve operation_id across retries. A failed quality goal is not success. Use artifacts rather than large arrays in chat."});
        let method = if let Some(id) = resume {
            params["threadId"] = json!(id);
            "thread/resume"
        } else {
            "thread/start"
        };
        let result = server.request(method, params)?;
        server.thread_id = Some(
            result["thread"]["id"]
                .as_str()
                .ok_or("app-server omitted thread ID")?
                .into(),
        );
        Ok(server)
    }
    fn write(&mut self, value: Value) -> Result<(), String> {
        serde_json::to_writer(&mut self.input, &value)
            .map_err(|_| "Cannot write app-server request")?;
        self.input
            .write_all(b"\n")
            .and_then(|_| self.input.flush())
            .map_err(|_| "app-server input closed".into())
    }
    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(json!({"id":id,"method":method,"params":params}))?;
        let deadline = Instant::now() + Duration::from_secs(45);
        while Instant::now() < deadline {
            if self.cancelled.load(Ordering::Relaxed) {
                return Err("Codex request cancelled".into());
            }
            let value = self.output.recv_timeout(Duration::from_millis(100));
            match value {
                Ok(value) if value["id"] == id && value.get("method").is_none() => {
                    if value.get("error").is_some() {
                        return Err(format!("app-server {method} rejected (code {}); check runtime compatibility or authorization",value["error"]["code"]));
                    }
                    return Ok(value["result"].clone());
                }
                Ok(value) => self.handle(value)?,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err("app-server exited before responding".into()),
            }
        }
        Err(format!("app-server {method} timed out"))
    }
    pub fn send_message(&mut self, model: &str, message: &str) -> Result<(), String> {
        if self.turn_id.is_some() {
            return Err("A Codex turn is already running".into());
        }
        self.completed_tool_calls = 0;
        let result=self.request("turn/start",json!({"threadId":self.thread_id,"model":model,"input":[{"type":"text","text":message}]}))?;
        self.turn_id = Some(
            result["turn"]["id"]
                .as_str()
                .ok_or("app-server omitted turn ID")?
                .into(),
        );
        Ok(())
    }
    pub fn cancel(&mut self) -> Result<(), String> {
        if let Some(turn) = self.turn_id.clone() {
            let id = self.next_id;
            self.next_id += 1;
            self.write(json!({"id":id,"method":"turn/interrupt","params":{"threadId":self.thread_id,"turnId":turn}}))?;
        }
        Ok(())
    }
    pub fn poll(&mut self) -> Result<(), String> {
        loop {
            match self.output.try_recv() {
                Ok(v) => self.handle(v)?,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(_) => return Err("Codex app-server exited".into()),
            }
        }
        Ok(())
    }
    fn handle(&mut self, value: Value) -> Result<(), String> {
        let method = value["method"].as_str().unwrap_or("");
        if value.get("id").is_some() && !method.is_empty() {
            // Domain tools have their own authorization boundary. Never grant shell/file/network escalation.
            let result = match method {
                "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                    json!({"decision":"decline"})
                }
                "item/permissions/requestApproval" => json!({"permissions":{},"scope":"turn"}),
                "mcpServer/elicitation/request" => json!({"action":"decline"}),
                _ => {
                    self.write(json!({"id":value["id"],"error":{"code":-32601,"message":"Unsupported request; rustmesh denied it"}}))?;
                    return Ok(());
                }
            };
            self.write(json!({"id":value["id"],"result":result}))?;
            let _ = self.events.send(AgentEvent::Status(
                "Denied an action outside rustmesh domain tools".into(),
            ));
            return Ok(());
        }
        let params = &value["params"];
        match method {
            "item/agentMessage/delta" => {
                if let Some(delta) = params["delta"].as_str() {
                    let _ = self.events.send(AgentEvent::Delta(delta.into()));
                }
            }
            "item/started" | "item/completed" if params["item"]["type"] == "mcpToolCall" => {
                let item = &params["item"];
                let status = item["status"].as_str().unwrap_or("running");
                if method == "item/completed"
                    && status == "completed"
                    && item["server"] == "rustmesh"
                    && item["error"].is_null()
                    && item["result"]["structuredContent"]["ok"] == true
                {
                    self.completed_tool_calls += 1;
                }
                let _ = self.events.send(AgentEvent::Tool {
                    name: item["tool"].as_str().unwrap_or("rustmesh tool").into(),
                    status: status.into(),
                });
            }
            "turn/completed" => {
                self.turn_id = None;
                let status = params["turn"]["status"].as_str().unwrap_or("unknown");
                let _ = self.events.send(AgentEvent::Completed {
                    status: status.into(),
                    tool_calls: self.completed_tool_calls,
                });
                if status == "failed" {
                    let _ = self
                        .events
                        .send(AgentEvent::Error(failure_message(&params["turn"]["error"])));
                }
            }
            "error" => {
                let _ = self.events.send(AgentEvent::Error(failure_message(params)));
            }
            "thread/tokenUsage/updated" => {
                let _ = self
                    .events
                    .send(AgentEvent::Usage(params["tokenUsage"].clone()));
            }
            _ => {}
        }
        Ok(())
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn child_configuration_never_contains_credentials() {
        let config = AgentConfig {
            codex: "codex".into(),
            data_dir: "data".into(),
            workspace_dir: "workspace".into(),
            mcp_command: "rustmesh".into(),
            mcp_args: vec!["mcp".into()],
            mcp_token: "private-secret-test".into(),
        };
        let all = overrides(&config).join("\n");
        assert!(!all.contains("private-secret-test"));
        assert!(all.contains("supports_websockets=false"));
        assert!(all.contains("features.shell_tool=false"));
        assert!(all.contains("features.code_mode_tool_search=false"));
        assert!(all.contains("sandbox_mode=\"read-only\""));
    }
    #[test]
    fn errors_are_classified_without_echoing_secrets() {
        let message = failure_message(
            &json!({"code":"subscription_sharing_usage_limit_exceeded","message":"secret-token-never-print"}),
        );
        assert!(message.contains("Usage"));
        assert!(!message.contains("secret-token"));
        assert!(failure_message(&json!({"code":"invalid_token"})).contains("Reauthorize"));
        assert!(failure_message(&json!({"code":"forbidden"})).contains("policy"));
    }
}
