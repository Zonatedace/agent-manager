//! In-app agent sessions: background multi-turn runs with live event fan-out.
//!
//! Each session runs CLI tools in headless mode (no external terminal). Multiple
//! sessions can run concurrently. Orchestrator sessions can spawn child workers.

use crate::agents::{self, AgentCli, AgentMode, AgentRequest};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::process::{Command as StdCommand, Stdio as StdStdio};
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{error, info, warn};
use uuid::Uuid;

const MAX_LOG_CHARS: usize = 400_000;
const MAX_MESSAGES: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Queued,
    Running,
    Waiting, // idle, ready for next user message
    Stopping,
    Stopped,
    Failed,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionRole {
    Worker,
    Orchestrator,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub role: String, // user | assistant | system | tool
    pub content: String,
    pub ts: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    Snapshot {
        session: SessionSummary,
    },
    Status {
        session_id: String,
        status: SessionStatus,
    },
    Log {
        session_id: String,
        stream: String, // stdout | stderr | system
        text: String,
    },
    /// Incremental assistant text while a turn is running (for live chat UI)
    StreamDelta {
        session_id: String,
        text: String,
    },
    Message {
        session_id: String,
        message: ChatMessage,
    },
    ChildSpawned {
        parent_id: String,
        child: SessionSummary,
    },
    Done {
        session_id: String,
        exit_code: Option<i32>,
    },
    Error {
        session_id: String,
        error: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub parent_id: Option<String>,
    pub role: SessionRole,
    pub name: String,
    pub cli: String,
    pub model: String,
    pub effort: String,
    pub cwd: String,
    pub status: SessionStatus,
    pub always_approve: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub exit_code: Option<i32>,
    pub child_ids: Vec<String>,
    pub message_count: usize,
    pub last_message_preview: String,
}

#[derive(Debug, Clone)]
struct SessionInner {
    summary: SessionSummary,
    messages: Vec<ChatMessage>,
    log: String,
    /// cancel flag for current run
    cancel: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    pub cli: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub effort: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub always_approve: bool,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub extra_args: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct SessionMessageRequest {
    pub text: String,
}

#[derive(Clone)]
pub struct SessionManager {
    sessions: Arc<RwLock<HashMap<String, SessionInner>>>,
    events: broadcast::Sender<SessionEvent>,
}

impl SessionManager {
    pub fn new() -> Self {
        // Large buffer so bursty streaming logs don't lag out SSE clients
        let (events, _) = broadcast::channel(8192);
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            events,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    fn emit(&self, ev: SessionEvent) {
        let _ = self.events.send(ev);
    }

    pub async fn list(&self) -> Vec<SessionSummary> {
        let map = self.sessions.read().await;
        let mut v: Vec<_> = map.values().map(|s| s.summary.clone()).collect();
        v.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        v
    }

    pub async fn get(&self, id: &str) -> Option<(SessionSummary, Vec<ChatMessage>, String)> {
        let map = self.sessions.read().await;
        let s = map.get(id)?;
        Some((s.summary.clone(), s.messages.clone(), s.log.clone()))
    }

    pub async fn create(
        &self,
        req: CreateSessionRequest,
        default_cwd: PathBuf,
    ) -> Result<SessionSummary, String> {
        let cli = AgentCli::from_str_loose(&req.cli)
            .ok_or_else(|| format!("unknown cli: {}", req.cli))?;

        let cwd = if req.cwd.trim().is_empty() {
            default_cwd
        } else {
            let p = PathBuf::from(req.cwd.trim());
            if !p.is_dir() {
                return Err(format!("cwd is not a directory: {}", p.display()));
            }
            p.canonicalize().unwrap_or(p)
        };

        let role = match req.role.as_deref().unwrap_or("worker") {
            "orchestrator" => SessionRole::Orchestrator,
            _ => SessionRole::Worker,
        };

        let id = Uuid::new_v4().to_string();
        let name = if req.name.trim().is_empty() {
            match role {
                SessionRole::Orchestrator => format!("orchestrator · {}", cli.as_str()),
                SessionRole::Worker => format!("{} · {}", cli.as_str(), short_path(&cwd)),
            }
        } else {
            req.name.trim().to_string()
        };

        let now = Utc::now();
        let summary = SessionSummary {
            id: id.clone(),
            parent_id: req.parent_id.clone(),
            role,
            name,
            cli: cli.as_str().into(),
            model: req.model.clone(),
            effort: req.effort.clone(),
            cwd: cwd.to_string_lossy().into(),
            status: SessionStatus::Queued,
            always_approve: req.always_approve,
            created_at: now,
            updated_at: now,
            exit_code: None,
            child_ids: vec![],
            message_count: 0,
            last_message_preview: String::new(),
        };

        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut inner = SessionInner {
            summary: summary.clone(),
            messages: vec![],
            log: String::new(),
            cancel: cancel.clone(),
        };

        // System preamble for orchestrator
        if role == SessionRole::Orchestrator {
            let sys = orchestrator_system_prompt();
            let msg = ChatMessage {
                id: Uuid::new_v4().to_string(),
                role: "system".into(),
                content: sys,
                ts: Utc::now(),
            };
            inner.messages.push(msg.clone());
            self.emit(SessionEvent::Message {
                session_id: id.clone(),
                message: msg,
            });
        }

        if let Some(parent_id) = &req.parent_id {
            let mut map = self.sessions.write().await;
            if let Some(parent) = map.get_mut(parent_id) {
                parent.summary.child_ids.push(id.clone());
                parent.summary.updated_at = Utc::now();
            }
        }

        {
            let mut map = self.sessions.write().await;
            map.insert(id.clone(), inner);
        }

        self.emit(SessionEvent::Snapshot {
            session: summary.clone(),
        });

        if let Some(parent_id) = req.parent_id.clone() {
            self.emit(SessionEvent::ChildSpawned {
                parent_id,
                child: summary.clone(),
            });
        }

        let initial = req.prompt.trim().to_string();
        if !initial.is_empty() {
            self.push_user_and_run(&id, initial, req.extra_args.clone())
                .await?;
        } else {
            self.set_status(&id, SessionStatus::Waiting).await;
        }

        let map = self.sessions.read().await;
        Ok(map.get(&id).map(|s| s.summary.clone()).unwrap_or(summary))
    }

    pub async fn send_message(
        &self,
        id: &str,
        text: String,
        extra_args: Vec<String>,
    ) -> Result<(), String> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err("message is empty".into());
        }
        self.push_user_and_run(id, text, extra_args).await
    }

    pub async fn stop(&self, id: &str) -> Result<(), String> {
        let map = self.sessions.read().await;
        let Some(s) = map.get(id) else {
            return Err(format!("session not found: {id}"));
        };
        s.cancel
            .store(true, std::sync::atomic::Ordering::SeqCst);
        drop(map);
        self.set_status(id, SessionStatus::Stopping).await;
        self.append_log(id, "system", "Stop requested…").await;
        Ok(())
    }

    async fn push_user_and_run(
        &self,
        id: &str,
        text: String,
        extra_args: Vec<String>,
    ) -> Result<(), String> {
        // Guard: not already running
        {
            let map = self.sessions.read().await;
            let s = map
                .get(id)
                .ok_or_else(|| format!("session not found: {id}"))?;
            // Only block if a turn is actively in flight (Queued is "about to start" on create)
            if matches!(
                s.summary.status,
                SessionStatus::Running | SessionStatus::Stopping
            ) {
                return Err("session is already running a turn".into());
            }
        }

        let msg = ChatMessage {
            id: Uuid::new_v4().to_string(),
            role: "user".into(),
            content: text.clone(),
            ts: Utc::now(),
        };
        {
            let mut map = self.sessions.write().await;
            let s = map
                .get_mut(id)
                .ok_or_else(|| format!("session not found: {id}"))?;
            s.messages.push(msg.clone());
            if s.messages.len() > MAX_MESSAGES {
                let drop_n = s.messages.len() - MAX_MESSAGES;
                s.messages.drain(0..drop_n);
            }
            s.summary.message_count = s.messages.len();
            s.summary.last_message_preview = preview(&text);
            s.summary.updated_at = Utc::now();
            s.cancel
                .store(false, std::sync::atomic::Ordering::SeqCst);
        }
        self.emit(SessionEvent::Message {
            session_id: id.to_string(),
            message: msg,
        });

        self.set_status(id, SessionStatus::Running).await;

        let mgr = self.clone();
        let sid = id.to_string();
        tokio::spawn(async move {
            if let Err(e) = mgr.run_turn(&sid, extra_args).await {
                error!(session = %sid, error = %e, "agent turn failed");
                mgr.append_log(&sid, "system", &format!("ERROR: {e}")).await;
                mgr.emit(SessionEvent::Error {
                    session_id: sid.clone(),
                    error: e.clone(),
                });
                mgr.set_status(&sid, SessionStatus::Failed).await;
            }
        });
        Ok(())
    }

    async fn run_turn(&self, id: &str, extra_args: Vec<String>) -> Result<(), String> {
        let (cli, model, effort, cwd, always_approve, role, messages, cancel) = {
            let map = self.sessions.read().await;
            let s = map
                .get(id)
                .ok_or_else(|| format!("session not found: {id}"))?;
            (
                s.summary.cli.clone(),
                s.summary.model.clone(),
                s.summary.effort.clone(),
                PathBuf::from(&s.summary.cwd),
                s.summary.always_approve,
                s.summary.role,
                s.messages.clone(),
                s.cancel.clone(),
            )
        };

        let cli = AgentCli::from_str_loose(&cli).ok_or_else(|| format!("bad cli: {cli}"))?;

        let prompt = build_turn_prompt(role, &messages);
        let req = AgentRequest {
            project_id: String::new(),
            cwd: Some(cwd.to_string_lossy().into()),
            cli: cli.as_str().into(),
            prompt: prompt.clone(),
            model,
            effort,
            mode: "headless".into(),
            always_approve,
            extra_args,
            name: String::new(),
        };

        let mut args = agents::build_args(cli, &req, AgentMode::Headless);
        // Prefer streaming formats so CLIs flush more often (non-TTY pipes are otherwise
        // fully block-buffered and only appear after the process exits).
        match cli {
            AgentCli::Grok => {
                // strip any existing --output-format pair, force streaming-json
                strip_flag_pair(&mut args, "--output-format");
                // insert after binary flags, before prompt: put near front
                args.insert(0, "streaming-json".into());
                args.insert(0, "--output-format".into());
            }
            AgentCli::Claude => {
                strip_flag_pair(&mut args, "--output-format");
                args.push("--output-format".into());
                args.push("stream-json".into());
                // partial message chunks for live text
                if !args.iter().any(|a| a == "--include-partial-messages") {
                    args.push("--include-partial-messages".into());
                }
            }
            AgentCli::Codex => {
                if !args.iter().any(|a| a == "--json") {
                    // after "exec" if present
                    if let Some(i) = args.iter().position(|a| a == "exec") {
                        args.insert(i + 1, "--json".into());
                    } else {
                        args.insert(0, "--json".into());
                    }
                }
            }
        }

        self.append_log(
            id,
            "system",
            &format!("$ {} {}", cli.as_str(), args.join(" ")),
        )
        .await;

        // Run the CLI on a blocking thread so the async runtime stays Send-friendly.
        // Stream stdout/stderr lines back over a channel for live UI updates.
        let (tx, mut rx) = mpsc::unbounded_channel::<(String, String)>();
        let cli_name = cli.as_str().to_string();
        let args_owned = args.clone();
        let cwd_owned = cwd.clone();
        let cancel_thread = cancel.clone();

        let join = tokio::task::spawn_blocking(move || -> Result<Option<i32>, String> {
            let mut child = StdCommand::new(&cli_name)
                .args(&args_owned)
                .current_dir(&cwd_owned)
                .stdin(StdStdio::null())
                .stdout(StdStdio::piped())
                .stderr(StdStdio::piped())
                // Encourage line-buffering / less pipe buffering where tools honor these
                .env("PYTHONUNBUFFERED", "1")
                .env("NODE_NO_READLINE", "1")
                .env("RUST_LOG_STYLE", "always")
                .spawn()
                .map_err(|e| format!("failed to spawn {cli_name}: {e}"))?;

            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            let tx_out = tx.clone();
            let t_out = std::thread::spawn(move || {
                if let Some(out) = stdout {
                    pump_stream_chunks(out, "stdout", &tx_out);
                }
            });
            let tx_err = tx.clone();
            let t_err = std::thread::spawn(move || {
                if let Some(err) = stderr {
                    pump_stream_chunks(err, "stderr", &tx_err);
                }
            });

            let code = loop {
                if cancel_thread.load(std::sync::atomic::Ordering::SeqCst) {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                match child.try_wait() {
                    Ok(Some(st)) => break st.code(),
                    Ok(None) => std::thread::sleep(std::time::Duration::from_millis(80)),
                    Err(e) => {
                        warn!(error = %e, "try_wait failed");
                        break None;
                    }
                }
            };
            let _ = t_out.join();
            let _ = t_err.join();
            Ok(code)
        });

        // Forward logs while process runs (channel closes when blocking task finishes).
        // Emit StreamDelta so the UI can update a live assistant bubble, not only at exit.
        let mut turn_stdout = String::new();
        let mut turn_stderr = String::new();
        let mut visible = String::new();
        let mut json_line_buf = String::new();
        while let Some((stream, text)) = rx.recv().await {
            if stream == "stdout" {
                turn_stdout.push_str(&text);
                // Decode Grok/Claude/Codex streaming-json into plain assistant text
                let delta =
                    visible_delta_from_chunk(&cli, &text, &mut visible, &mut json_line_buf);
                if !delta.is_empty() {
                    self.emit(SessionEvent::StreamDelta {
                        session_id: id.to_string(),
                        text: delta,
                    });
                }
            } else if stream == "stderr" {
                turn_stderr.push_str(&text);
                // Keep stderr in the live log only (not chat) — often noise
            }
            self.append_log(id, &stream, &text).await;
        }
        // Flush any trailing incomplete line buffer
        if !json_line_buf.trim().is_empty() {
            let tail = json_line_buf.clone();
            json_line_buf.clear();
            let delta = visible_delta_from_chunk(&cli, &format!("{tail}\n"), &mut visible, &mut json_line_buf);
            if !delta.is_empty() {
                self.emit(SessionEvent::StreamDelta {
                    session_id: id.to_string(),
                    text: delta,
                });
            }
        }

        let code = match join.await {
            Ok(Ok(c)) => c,
            Ok(Err(e)) => return Err(e),
            Err(e) => return Err(format!("join error: {e}")),
        };

        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            self.append_log(id, "system", "Session stopped by user").await;
            self.set_status(id, SessionStatus::Stopped).await;
            self.emit(SessionEvent::Done {
                session_id: id.to_string(),
                exit_code: None,
            });
            return Ok(());
        }
        {
            let mut map = self.sessions.write().await;
            if let Some(s) = map.get_mut(id) {
                s.summary.exit_code = code;
                s.summary.updated_at = Utc::now();
            }
        }

        // Prefer decoded visible text, then try a full-pass decode of stdout, then raw
        let assistant_text = if !visible.trim().is_empty() {
            visible
        } else if !turn_stdout.trim().is_empty() {
            summarize_raw_output(&cli, &turn_stdout)
        } else if !turn_stderr.trim().is_empty() {
            // only if nothing on stdout
            format!("(stderr)\n{}", turn_stderr.trim())
        } else {
            format!("(no output; exit code {:?})", code)
        };

        let msg = ChatMessage {
            id: Uuid::new_v4().to_string(),
            role: "assistant".into(),
            content: assistant_text.clone(),
            ts: Utc::now(),
        };
        {
            let mut map = self.sessions.write().await;
            if let Some(s) = map.get_mut(id) {
                s.messages.push(msg.clone());
                s.summary.message_count = s.messages.len();
                s.summary.last_message_preview = preview(&assistant_text);
                s.summary.updated_at = Utc::now();
            }
        }
        self.emit(SessionEvent::Message {
            session_id: id.to_string(),
            message: msg,
        });

        // Orchestrator: auto-spawn from directives in the response (fire-and-forget tasks)
        if role == SessionRole::Orchestrator {
            let spawns = parse_spawn_directives(&assistant_text);
            for sp in spawns {
                info!(parent = %id, child_prompt = %sp.prompt, "orchestrator spawning child");
                let child_req = CreateSessionRequest {
                    cli: sp.cli.unwrap_or_else(|| cli.as_str().into()),
                    model: sp.model.unwrap_or_default(),
                    effort: sp.effort.unwrap_or_default(),
                    prompt: sp.prompt,
                    cwd: sp.cwd.unwrap_or_else(|| cwd.to_string_lossy().into()),
                    name: sp.name.unwrap_or_default(),
                    always_approve,
                    role: Some("worker".into()),
                    parent_id: Some(id.to_string()),
                    project_id: String::new(),
                    extra_args: vec![],
                };
                // Spawn on a detached thread + tiny runtime to avoid async recursion
                // (create → push_user_and_run → run_turn → create) confusing rustc Send checks.
                let mgr = self.clone();
                let parent = id.to_string();
                let cwd2 = cwd.clone();
                std::thread::spawn(move || {
                    let rt = match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(rt) => rt,
                        Err(e) => {
                            error!(error = %e, "failed to build runtime for child spawn");
                            return;
                        }
                    };
                    rt.block_on(async move {
                        match mgr.create(child_req, cwd2).await {
                            Ok(child) => {
                                mgr.append_log(
                                    &parent,
                                    "system",
                                    &format!("Spawned sub-agent {} ({})", child.name, child.id),
                                )
                                .await;
                            }
                            Err(e) => {
                                mgr.append_log(
                                    &parent,
                                    "system",
                                    &format!("Failed to spawn child: {e}"),
                                )
                                .await;
                            }
                        }
                    });
                });
            }
        }

        let failed = code.unwrap_or(1) != 0;
        if failed {
            self.set_status(id, SessionStatus::Failed).await;
        } else {
            self.set_status(id, SessionStatus::Waiting).await;
        }
        self.emit(SessionEvent::Done {
            session_id: id.to_string(),
            exit_code: code,
        });
        Ok(())
    }

    async fn extract_last_stdout_chunk(&self, id: &str) -> String {
        let map = self.sessions.read().await;
        let Some(s) = map.get(id) else {
            return String::new();
        };
        // Take lines after the last "$ cli" system command marker in log that are stdout
        // Simpler: use messages — actually we only have combined log.
        // Parse log for stdout lines since last system $ line
        let mut chunk = String::new();
        let mut capturing = false;
        for line in s.log.lines() {
            if line.starts_with("$ ") {
                chunk.clear();
                capturing = true;
                continue;
            }
            if capturing {
                // skip pure stderr markers — we prefix streams in append_log
                if let Some(rest) = line.strip_prefix("[stdout] ") {
                    chunk.push_str(rest);
                    chunk.push('\n');
                } else if line.starts_with("[stderr] ") || line.starts_with("[system] ") {
                    // skip
                } else if !line.is_empty() {
                    // legacy unprefixed
                    chunk.push_str(line);
                    chunk.push('\n');
                }
            }
        }
        chunk
    }

    async fn set_status(&self, id: &str, status: SessionStatus) {
        {
            let mut map = self.sessions.write().await;
            if let Some(s) = map.get_mut(id) {
                s.summary.status = status;
                s.summary.updated_at = Utc::now();
            }
        }
        self.emit(SessionEvent::Status {
            session_id: id.to_string(),
            status,
        });
        if let Some((sum, _, _)) = self.get(id).await {
            self.emit(SessionEvent::Snapshot { session: sum });
        }
    }

    async fn append_log(&self, id: &str, stream: &str, text: &str) {
        let line = if text.ends_with('\n') {
            text.to_string()
        } else {
            format!("{text}\n")
        };
        {
            let mut map = self.sessions.write().await;
            if let Some(s) = map.get_mut(id) {
                s.log.push_str(&format!("[{stream}] {line}"));
                if s.log.len() > MAX_LOG_CHARS {
                    let keep = MAX_LOG_CHARS / 2;
                    s.log = s.log[s.log.len() - keep..].to_string();
                }
                s.summary.updated_at = Utc::now();
            }
        }
        self.emit(SessionEvent::Log {
            session_id: id.to_string(),
            stream: stream.into(),
            text: line,
        });
    }
}

fn short_path(p: &Path) -> String {
    p.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("workspace")
        .to_string()
}

fn preview(s: &str) -> String {
    let t = s.trim().replace('\n', " ");
    if t.len() > 120 {
        format!("{}…", &t[..120])
    } else {
        t
    }
}

fn orchestrator_system_prompt() -> String {
    r#"You are an orchestrator agent running inside TODO Dashboard.
Your job is to break goals into parallel workstreams and spawn sub-agents.

When you want to spawn one or more sub-agents, include a fenced JSON block exactly like:

```json
{
  "spawn": [
    {
      "name": "short-name",
      "cli": "grok",
      "cwd": "C:\\absolute\\path\\to\\repo",
      "model": "",
      "prompt": "Clear task for the sub-agent..."
    }
  ]
}
```

Rules:
- Prefer multiple focused sub-agents over one giant agent.
- Use absolute cwd paths under the repos root when possible.
- cli may be grok, claude, or codex.
- After spawning, summarize what each sub-agent will do.
- If you need more user input, ask a concise question and do not spawn yet.
"#
    .to_string()
}

fn build_turn_prompt(role: SessionRole, messages: &[ChatMessage]) -> String {
    let mut parts = Vec::new();
    if role == SessionRole::Orchestrator {
        // system already first
    }
    for m in messages {
        match m.role.as_str() {
            "system" => {
                parts.push(format!("### System\n{}", m.content));
            }
            "user" => {
                parts.push(format!("### User\n{}", m.content));
            }
            "assistant" => {
                parts.push(format!("### Assistant\n{}", m.content));
            }
            _ => {
                parts.push(format!("### {}\n{}", m.role, m.content));
            }
        }
    }
    parts.push(
        "### Instruction\nRespond to the latest user message. Continue the work. Do not repeat prior assistant output unless needed."
            .into(),
    );
    parts.join("\n\n")
}

#[derive(Debug, Default)]
struct SpawnSpec {
    name: Option<String>,
    cli: Option<String>,
    cwd: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    prompt: String,
}

/// Read child stdout/stderr in small chunks (not only full lines) so partial
/// flushes show up in the UI before the process exits.
fn pump_stream_chunks<R: std::io::Read>(
    reader: R,
    stream: &str,
    tx: &tokio::sync::mpsc::UnboundedSender<(String, String)>,
) {
    use std::io::Read;
    let mut reader = reader;
    let mut buf = [0u8; 512];
    let mut carry = Vec::new();
    loop {
        match reader.read(&mut buf) {
            Ok(0) => {
                if !carry.is_empty() {
                    let s = String::from_utf8_lossy(&carry).into_owned();
                    let _ = tx.send((stream.into(), s));
                }
                break;
            }
            Ok(n) => {
                carry.extend_from_slice(&buf[..n]);
                // Emit on newlines or when buffer grows enough (partial streaming)
                let should_flush = carry.contains(&b'\n') || carry.len() >= 256;
                if should_flush {
                    let s = String::from_utf8_lossy(&carry).into_owned();
                    carry.clear();
                    let _ = tx.send((stream.into(), s));
                }
            }
            Err(_) => break,
        }
    }
}

fn strip_flag_pair(args: &mut Vec<String>, flag: &str) {
    while let Some(i) = args.iter().position(|a| a == flag) {
        args.remove(i);
        if i < args.len() {
            args.remove(i);
        }
    }
}

/// Pull human-readable text out of streaming-json / stream-json / plain chunks.
///
/// Grok emits NDJSON like:
///   {"type":"thought","data":"..."}
///   {"type":"text","data":"I'm"}
///   {"type":"end","stopReason":"EndTurn",...}
///
/// We only surface `text` (and Claude/Codex equivalents), never raw JSON or thoughts.
/// `line_buf` carries incomplete lines across chunk boundaries.
fn visible_delta_from_chunk(
    cli: &AgentCli,
    chunk: &str,
    visible: &mut String,
    line_buf: &mut String,
) -> String {
    let mut delta = String::new();
    line_buf.push_str(chunk);

    // Prefer newline-delimited processing (NDJSON). Keep incomplete trailing line in buf.
    loop {
        let Some(pos) = line_buf.find('\n') else {
            break;
        };
        let line = line_buf[..pos].trim_end_matches('\r').trim().to_string();
        *line_buf = line_buf[pos + 1..].to_string();
        if line.is_empty() {
            continue;
        }
        if let Some(piece) = extract_user_facing_piece(cli, &line) {
            if !piece.is_empty() {
                visible.push_str(&piece);
                delta.push_str(&piece);
            }
        }
    }

    // If buffer is a complete single-line JSON object (no pending newline yet), try it
    let pending = line_buf.trim();
    if pending.starts_with('{') && pending.ends_with('}') {
        if let Some(piece) = extract_user_facing_piece(cli, pending) {
            if !piece.is_empty() {
                visible.push_str(&piece);
                delta.push_str(&piece);
                line_buf.clear();
            }
        }
    } else if !pending.is_empty()
        && !pending.starts_with('{')
        && !pending.contains("{\"type\"")
    {
        // plain text without newline yet — stream as-is
        visible.push_str(line_buf);
        delta.push_str(line_buf);
        line_buf.clear();
    }

    delta
}

fn extract_user_facing_piece(cli: &AgentCli, line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // Plain text line
    if !line.starts_with('{') {
        return Some(format!("{line}\n"));
    }

    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let typ = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

    match typ {
        // Grok streaming-json
        "text" => v
            .get("data")
            .and_then(|d| d.as_str())
            .map(|s| s.to_string()),
        "thought" | "thinking" | "reasoning" | "end" | "error" | "tool" | "tool_call"
        | "tool_result" | "status" => None,

        // Claude stream-json
        "content_block_delta" => v
            .pointer("/delta/text")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string()),
        "content_block_start" | "content_block_stop" | "message_start" | "message_delta"
        | "message_stop" | "ping" => None,
        "assistant" | "result" => v
            .get("result")
            .and_then(|x| x.as_str())
            .or_else(|| v.get("message").and_then(|x| x.as_str()))
            .map(|s| s.to_string()),

        // Codex / generic
        t if t.contains("message") || t == "agent_message" => v
            .pointer("/message/text")
            .or_else(|| v.pointer("/text"))
            .or_else(|| v.get("data"))
            .and_then(|x| x.as_str())
            .map(|s| s.to_string()),

        // Unknown JSON: only take explicit text/data string fields if present
        _ => {
            let _ = cli;
            v.get("data")
                .and_then(|d| d.as_str())
                .filter(|_| typ == "text" || typ.is_empty())
                .map(|s| s.to_string())
                .or_else(|| {
                    v.pointer("/delta/text")
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string())
                })
        }
    }
}

fn summarize_raw_output(cli: &AgentCli, raw: &str) -> String {
    let mut visible = String::new();
    let mut line_buf = String::new();
    let _ = visible_delta_from_chunk(cli, raw, &mut visible, &mut line_buf);
    if !line_buf.is_empty() {
        let _ = visible_delta_from_chunk(cli, "\n", &mut visible, &mut line_buf);
    }
    if visible.trim().is_empty() {
        // Last resort: strip obvious JSON noise rather than dump everything
        raw.lines()
            .filter_map(|l| {
                let l = l.trim();
                if l.is_empty() {
                    return None;
                }
                extract_user_facing_piece(cli, l)
            })
            .collect::<String>()
    } else {
        visible
    }
}

#[cfg(test)]
mod stream_parse_tests {
    use super::*;

    #[test]
    fn decodes_grok_streaming_json() {
        let raw = r#"{"type":"thought","data":"The"}
{"type":"thought","data":" user"}
{"type":"text","data":"I'm"}
{"type":"text","data":" here"}
{"type":"text","data":" —"}
{"type":"text","data":" everything"}
{"type":"text","data":"'s"}
{"type":"text","data":" working"}
{"type":"text","data":"."}
{"type":"end","stopReason":"EndTurn","sessionId":"x"}
"#;
        let out = summarize_raw_output(&AgentCli::Grok, raw);
        assert_eq!(out, "I'm here — everything's working.");
        assert!(!out.contains("thought"));
        assert!(!out.contains("EndTurn"));
    }
}

fn parse_spawn_directives(text: &str) -> Vec<SpawnSpec> {
    let mut out = Vec::new();
    // Find ```json ... ``` blocks
    let mut rest = text;
    while let Some(start) = rest.find("```") {
        rest = &rest[start + 3..];
        let rest_trim = rest.trim_start();
        let mut block_src = if rest_trim.to_ascii_lowercase().starts_with("json") {
            rest_trim[4..].trim_start()
        } else {
            rest_trim
        };
        let Some(end) = block_src.find("```") else { break };
        let block = &block_src[..end];
        rest = &block_src[end + 3..];

        if let Ok(v) = serde_json::from_str::<serde_json::Value>(block) {
            if let Some(arr) = v.get("spawn").and_then(|x| x.as_array()) {
                for item in arr {
                    let prompt = item
                        .get("prompt")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if prompt.is_empty() {
                        continue;
                    }
                    out.push(SpawnSpec {
                        name: item
                            .get("name")
                            .and_then(|x| x.as_str())
                            .map(|s| s.to_string()),
                        cli: item
                            .get("cli")
                            .and_then(|x| x.as_str())
                            .map(|s| s.to_string()),
                        cwd: item
                            .get("cwd")
                            .and_then(|x| x.as_str())
                            .map(|s| s.to_string()),
                        model: item
                            .get("model")
                            .and_then(|x| x.as_str())
                            .map(|s| s.to_string()),
                        effort: item
                            .get("effort")
                            .and_then(|x| x.as_str())
                            .map(|s| s.to_string()),
                        prompt,
                    });
                }
            }
        }
    }
    out
}


