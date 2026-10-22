//! MCP newline-delimited JSON-RPC over stdio; authenticated local attach.
//! Attach is a private TCP protocol, deliberately not an unauthenticated HTTP server.
use crate::workspace::Workspace;
use serde_json::{json, Value};
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
pub const PROTOCOL_VERSION: &str = "2025-11-25";
const MAX_LINE: usize = 1024 * 1024;
/// Keeps deadlines and completion persistence active while stdio awaits input.
pub struct JobMonitor {
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}
impl Drop for JobMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
pub fn start_job_monitor(workspace: Arc<Mutex<Workspace>>) -> JobMonitor {
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let join = thread::spawn(move || {
        while !worker_stop.load(Ordering::Acquire) {
            if let Ok(mut guard) = workspace.try_lock() {
                guard.poll_jobs();
            }
            thread::sleep(Duration::from_millis(25));
        }
    });
    JobMonitor {
        stop,
        join: Some(join),
    }
}
pub struct AttachServer {
    pub address: String,
    pub token: String,
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}
impl Drop for AttachServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
pub fn start_attach(workspace: Arc<Mutex<Workspace>>) -> Result<AttachServer, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|e| e.to_string())?
        .to_string();
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let secret = token.clone();
    let join = thread::spawn(move || {
        while !worker_stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, peer)) => {
                    if !peer.ip().is_loopback() {
                        continue;
                    }
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                    let reply = (|| -> Result<Value, String> {
                        let mut reader =
                            BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
                        let line = read_line(&mut reader)?.ok_or("empty request")?;
                        let envelope: Value =
                            serde_json::from_str(&line).map_err(|e| e.to_string())?;
                        let provided = envelope["token"].as_str().unwrap_or("");
                        if !same_secret(provided, &secret) {
                            return Err("unauthorized attach capability".into());
                        }
                        let mut guard = workspace.lock().map_err(|_| "workspace lock poisoned")?;
                        Ok(
                            dispatch(&mut guard, envelope["request"].clone())
                                .unwrap_or(Value::Null),
                        )
                    })();
                    let value = reply.unwrap_or_else(
                        |e| json!({"jsonrpc":"2.0","id":null,"error":{"code":-32001,"message":e}}),
                    );
                    let _ = writeln!(stream, "{value}");
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if let Ok(mut w) = workspace.lock() {
                        w.poll_jobs()
                    }
                    thread::sleep(Duration::from_millis(25));
                }
                Err(_) => break,
            }
        }
    });
    Ok(AttachServer {
        address,
        token,
        stop,
        join: Some(join),
    })
}
fn same_secret(a: &str, b: &str) -> bool {
    let mut mismatch = a.len() ^ b.len();
    for (i, x) in b.bytes().enumerate() {
        mismatch |= (x ^ a.as_bytes().get(i).copied().unwrap_or(0)) as usize;
    }
    mismatch == 0
}
fn read_line(reader: &mut impl BufRead) -> Result<Option<String>, String> {
    let mut buf = Vec::new();
    let count = reader
        .take((MAX_LINE + 1) as u64)
        .read_until(b'\n', &mut buf)
        .map_err(|e| e.to_string())?;
    if count == 0 {
        return Ok(None);
    }
    if count > MAX_LINE {
        return Err("request exceeds 1 MiB".into());
    }
    String::from_utf8(buf).map(Some).map_err(|e| e.to_string())
}
pub fn serve_stdio(workspace: Arc<Mutex<Workspace>>) -> Result<(), String> {
    let _monitor = start_job_monitor(workspace.clone());
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    while let Some(line) = read_line(&mut input)? {
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(request) => dispatch(
                &mut *workspace.lock().map_err(|_| "workspace lock poisoned")?,
                request,
            ),
            Err(e) => Some(
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":e.to_string()}}),
            ),
        };
        if let Some(response) = response {
            writeln!(output, "{response}").map_err(|e| e.to_string())?;
            output.flush().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
pub fn serve_attach_stdio(address: &str) -> Result<(), String> {
    let endpoint: SocketAddr = address.parse().map_err(|_| "invalid attach endpoint")?;
    if !endpoint.ip().is_loopback() {
        return Err("attach accepts only numeric loopback endpoints".into());
    }
    let token =
        std::env::var("RUSTMESH_ATTACH_TOKEN").map_err(|_| "RUSTMESH_ATTACH_TOKEN is required")?;
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    while let Some(line) = read_line(&mut input)? {
        let request: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        let expects_reply = request.get("id").is_some();
        let mut stream = TcpStream::connect_timeout(&endpoint, Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .map_err(|e| e.to_string())?;
        writeln!(stream, "{}", json!({"token":token,"request":request}))
            .map_err(|e| e.to_string())?;
        let reply = read_line(&mut BufReader::new(stream))?.ok_or("attach runtime closed")?;
        if expects_reply {
            write!(output, "{reply}").map_err(|e| e.to_string())?;
            output.flush().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
pub fn dispatch(workspace: &mut Workspace, request: Value) -> Option<Value> {
    let id = request.get("id")?.clone();
    let method = request["method"].as_str().unwrap_or("");
    let result = match method {
        "initialize" => {
            let requested = request["params"]["protocolVersion"]
                .as_str()
                .unwrap_or(PROTOCOL_VERSION);
            json!({"protocolVersion":if ["2025-11-25","2025-06-18","2024-11-05"].contains(&requested){requested}else{PROTOCOL_VERSION},"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"rustmesh","version":env!("CARGO_PKG_VERSION")},"instructions":"Operate only on the authorized workspace. All domain tool arguments require schema_version:1. Mutations need a stable operation_id. Compare actual reports; never equate node-budget exhaustion with completion."})
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools":crate::tools::definitions()}),
        "tools/call" => {
            let name = request["params"]["name"].as_str().unwrap_or("");
            let answer =
                workspace.call_with_owner(name, request["params"]["arguments"].clone(), "mcp");
            json!({"content":[{"type":"text","text":answer.to_string()}],"structuredContent":answer,"isError":answer["ok"]!=true})
        }
        _ => {
            return Some(
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"method not supported"}}),
            )
        }
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}
