use crate::agents::{self, AgentRequest};
use crate::fsbrowser;
use crate::git;
use crate::models::Snapshot;
use crate::scanner;
use crate::sessions::{
    CreateSessionRequest, SessionManager, SessionMessageRequest,
};
use crate::settings::{self, Settings};
use crate::todos;
use axum::extract::{Path, Query, State};
use axum::http::{Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::stream::Stream;
use serde::Deserialize;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt as _;
use tower::ServiceBuilder;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{error, info, warn};

pub struct AppState {
    pub root: RwLock<PathBuf>,
    pub snapshot: RwLock<Snapshot>,
    pub settings: RwLock<Settings>,
    pub config_path: PathBuf,
    pub sessions: SessionManager,
    /// Process-level gate (false in Docker). User toggle is `settings.agents_enabled`.
    pub allow_agents: bool,
}

impl AppState {
    pub fn new(
        root: PathBuf,
        snapshot: Snapshot,
        settings: Settings,
        config_path: PathBuf,
        sessions: SessionManager,
        allow_agents: bool,
    ) -> Self {
        Self {
            root: RwLock::new(root),
            snapshot: RwLock::new(snapshot),
            settings: RwLock::new(settings),
            config_path,
            sessions,
            allow_agents,
        }
    }

    pub async fn root_path(&self) -> PathBuf {
        self.root.read().await.clone()
    }

    /// Agents active = host allows them AND user enabled them in settings.
    pub async fn agents_active(&self) -> bool {
        if !self.allow_agents {
            return false;
        }
        self.settings.read().await.agents_enabled
    }
}

fn agents_disabled_response(allow: bool) -> Response {
    if !allow {
        json_error(
            StatusCode::FORBIDDEN,
            "Coding agents are not available on this server (e.g. Docker image has no agent CLIs). \
             Enable agents in the Windows app settings on a machine where claude/grok/codex are installed, \
             or run Agent Manager in app mode locally.",
        )
    } else {
        json_error(
            StatusCode::FORBIDDEN,
            "Coding agents are disabled in Settings. Turn on “Enable coding agents” to use CLIs, \
             sessions, and usage meters. Authentication is done on this machine (front-end / local CLI login).",
        )
    }
}

fn recompute_totals(snap: &mut Snapshot) {
    snap.total_open = snap.projects.iter().map(|p| p.open).sum();
    snap.total_done = snap.projects.iter().map(|p| p.done).sum();
    snap.total_items = snap.projects.iter().map(|p| p.total).sum();
    snap.git_repo_count = snap.projects.iter().filter(|p| p.git.is_some()).count();
}

fn json_error(status: StatusCode, msg: impl Into<String>) -> Response {
    let msg = msg.into();
    (
        status,
        Json(serde_json::json!({
            "error": msg,
            "ok": false,
        })),
    )
        .into_response()
}

fn json_ok<T: serde::Serialize>(value: T) -> Response {
    match serde_json::to_value(value) {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) => {
            error!(error = %e, "failed to serialize response");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, format!("serialize error: {e}"))
        }
    }
}

async fn request_log(req: Request<axum::body::Body>, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or("").to_string();
    let start = Instant::now();

    let response = next.run(req).await;

    let status = response.status().as_u16();
    let ms = start.elapsed().as_millis();
    if status >= 500 {
        error!(%method, %path, %query, status, ms, "request error");
    } else if status >= 400 {
        warn!(%method, %path, %query, status, ms, "request client error");
    } else {
        info!(%method, %path, %query, status, ms, "request");
    }
    response
}

pub async fn serve(state: Arc<AppState>, addr: &str) -> Result<(), String> {
    serve_with_shutdown(state, addr, std::future::pending()).await
}

/// Serve until `shutdown` completes (e.g. Windows service stop signal).
pub async fn serve_with_shutdown<F>(
    state: Arc<AppState>,
    addr: &str,
    shutdown: F,
) -> Result<(), String>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let app = Router::new()
        .route("/", get(index_html))
        .route("/api/global", get(api_global))
        .route("/api/projects", get(api_projects))
        // Prefer query-param project load (avoids path encoding issues)
        .route("/api/project", get(api_project_query))
        .route("/api/git/refresh", post(api_git_refresh))
        .route("/api/todos/toggle", post(api_todo_toggle))
        .route("/api/agents/discover", get(api_agents_discover))
        .route("/api/agents/start", post(api_agents_start))
        // In-app multi-agent sessions
        .route("/api/sessions", get(api_sessions_list).post(api_sessions_create))
        .route("/api/sessions/events", get(api_sessions_events_all))
        .route("/api/sessions/{id}", get(api_sessions_get))
        .route("/api/sessions/{id}/message", post(api_sessions_message))
        .route("/api/sessions/{id}/stop", post(api_sessions_stop))
        .route("/api/sessions/{id}/spawn", post(api_sessions_spawn))
        .route("/api/sessions/{id}/events", get(api_sessions_events))
        .route("/api/actions/open-folder", post(api_open_folder))
        .route("/api/actions/open-url", post(api_open_url))
        .route("/api/actions/open-terminal", post(api_open_terminal))
        .route("/api/refresh", post(api_refresh))
        .route("/api/health", get(api_health))
        .route("/api/usage", get(api_usage))
        .route("/api/settings", get(api_settings_get).put(api_settings_put))
        .route("/api/fs/list", get(api_fs_list))
        // Path-based fallbacks (may be finicky with special chars)
        .route("/api/git/{*id}", get(api_project_git))
        .route("/api/projects/{*id}", get(api_project_path))
        .layer(
            ServiceBuilder::new()
                .layer(CatchPanicLayer::custom(handle_panic))
                .layer(TraceLayer::new_for_http())
                .layer(middleware::from_fn(request_log))
                .layer(CorsLayer::permissive()),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("Failed to bind {addr}: {e}"))?;

    info!(%addr, "http server bound");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(|e| format!("server error: {e}"))
}

fn handle_panic(err: Box<dyn std::any::Any + Send + 'static>) -> Response {
    let detail = if let Some(s) = err.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = err.downcast_ref::<&str>() {
        (*s).to_string()
    } else {
        "unknown panic".to_string()
    };
    error!(panic = %detail, "request handler panicked (caught)");
    json_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("internal server error (panic caught): {detail}"),
    )
}

async fn index_html() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

async fn api_health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let snap = state.snapshot.read().await;
    let root = state.root.read().await;
    let settings = state.settings.read().await;
    let agents_active = state.allow_agents && settings.agents_enabled;
    Json(serde_json::json!({
        "ok": true,
        "status": "ok",
        "projects": snap.projects.len(),
        "open": snap.total_open,
        "done": snap.total_done,
        "scanned_at": snap.scanned_at,
        "root": root.to_string_lossy(),
        "health_poll_seconds": settings.health_poll_seconds,
        "auto_refresh_minutes": settings.auto_refresh_minutes,
        "allow_agents": state.allow_agents,
        "agents_enabled": settings.agents_enabled,
        "agents_active": agents_active,
    }))
}

async fn api_usage(State(state): State<Arc<AppState>>) -> Response {
    if !state.agents_active().await {
        return json_ok(serde_json::json!({
            "ok": true,
            "agents_active": false,
            "meters": [],
            "message": if state.allow_agents {
                "Usage meters hidden — enable coding agents in Settings."
            } else {
                "Usage meters unavailable on this host (no agent CLIs; e.g. Docker)."
            },
        }));
    }
    let settings = state.settings.read().await.clone();
    match tokio::task::spawn_blocking(move || {
        crate::usage::collect_selected(
            settings.agents_claude,
            settings.agents_grok,
            settings.agents_codex,
        )
    })
    .await
    {
        Ok(snap) => json_ok(snap),
        Err(e) => {
            error!(error = %e, "usage collect panicked");
            json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("usage collect failed: {e}"),
            )
        }
    }
}

async fn api_settings_get(State(state): State<Arc<AppState>>) -> Response {
    let settings = state.settings.read().await.clone();
    let agents_active = state.allow_agents && settings.agents_enabled;
    json_ok(serde_json::json!({
        "settings": settings,
        "config_path": state.config_path,
        "root": state.root.read().await.to_string_lossy(),
        "allow_agents": state.allow_agents,
        "agents_active": agents_active,
        "agents_note": if state.allow_agents {
            "Coding agents run on this host. Auth is done via local CLI login (front-end shows status)."
        } else {
            "This server does not host coding agents (Docker/server-only). Use the Windows app on a machine with CLIs, or set AGENT_MANAGER_ALLOW_AGENTS=1 only where agents are installed."
        },
    }))
}

async fn api_settings_put(
    State(state): State<Arc<AppState>>,
    Json(mut body): Json<Settings>,
) -> Response {
    body.normalize();
    // Docker / locked hosts: never persist agents_enabled=true
    if body.agents_enabled && !state.allow_agents {
        warn!("ignoring agents_enabled=true — host disallows agents");
        body.agents_enabled = false;
    }
    let new_root = PathBuf::from(&body.root);
    if let Err(e) = Settings::validate_root(&new_root) {
        return json_error(StatusCode::BAD_REQUEST, e);
    }

    let old_root = state.root.read().await.clone();
    let root_changed = old_root != new_root
        && old_root.canonicalize().ok() != new_root.canonicalize().ok();

    if let Err(e) = settings::save(&state.config_path, &body) {
        error!(error = %e, "save settings failed");
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, e);
    }

    *state.settings.write().await = body.clone();

    if root_changed {
        info!(
            old = %old_root.display(),
            new = %new_root.display(),
            "root changed; rescanning"
        );
        *state.root.write().await = new_root.clone();
        let scan_root = new_root.clone();
        match tokio::task::spawn_blocking(move || scanner::scan_repos(&scan_root)).await {
            Ok(snapshot) => {
                info!(
                    projects = snapshot.projects.len(),
                    open = snapshot.total_open,
                    "rescan after root change complete"
                );
                *state.snapshot.write().await = snapshot;
            }
            Err(e) => {
                error!(error = %e, "rescan after root change failed");
                return json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("settings saved but rescan failed: {e}"),
                );
            }
        }
    }

    let snap = state.snapshot.read().await;
    let agents_active = state.allow_agents && body.agents_enabled;
    json_ok(serde_json::json!({
        "ok": true,
        "settings": body,
        "root_changed": root_changed,
        "projects": snap.projects.len(),
        "open": snap.total_open,
        "global": snap.to_global_view(),
        "allow_agents": state.allow_agents,
        "agents_active": agents_active,
    }))
}

#[derive(Debug, Deserialize)]
struct FsListQuery {
    #[serde(default)]
    path: String,
}

async fn api_fs_list(Query(q): Query<FsListQuery>) -> Response {
    match fsbrowser::list_dir(&q.path) {
        Ok(listing) => {
            info!(
                path = %listing.path,
                entries = listing.entries.len(),
                "fs list"
            );
            json_ok(listing)
        }
        Err(e) => {
            warn!(path = %q.path, error = %e, "fs list failed");
            json_error(StatusCode::BAD_REQUEST, e)
        }
    }
}

async fn api_global(State(state): State<Arc<AppState>>) -> Response {
    let snap = state.snapshot.read().await;
    info!(projects = snap.projects.len(), "serving global view");
    json_ok(snap.to_global_view())
}

async fn api_projects(State(state): State<Arc<AppState>>) -> Response {
    let snap = state.snapshot.read().await;
    let summaries: Vec<_> = snap
        .projects
        .iter()
        .map(|p| crate::models::ProjectSummary {
            id: p.id.clone(),
            name: p.name.clone(),
            open: p.open,
            done: p.done,
            total: p.total,
            modified: p.modified,
            git: p.git.as_ref().map(|g| g.to_summary()),
        })
        .collect();
    json_ok(summaries)
}

#[derive(Debug, Deserialize)]
struct ProjectQuery {
    id: String,
}

async fn api_project_query(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ProjectQuery>,
) -> Response {
    let id = q.id;
    info!(%id, "load project (query)");
    serve_project(&state, &id).await
}

async fn api_project_path(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let id = urlencoding_decode(&id);
    info!(%id, "load project (path)");
    serve_project(&state, &id).await
}

async fn serve_project(state: &AppState, id: &str) -> Response {
    let snap = state.snapshot.read().await;
    match snap.project_by_id(id) {
        Some(p) => {
            info!(
                id = %p.id,
                open = p.open,
                done = p.done,
                items = p.items.len(),
                has_git = p.git.is_some(),
                "project found"
            );
            json_ok(p)
        }
        None => {
            // Helpful: show close matches
            let similar: Vec<_> = snap
                .projects
                .iter()
                .filter(|p| {
                    p.id.to_lowercase().contains(&id.to_lowercase())
                        || p.name.to_lowercase().contains(&id.to_lowercase())
                })
                .map(|p| p.id.clone())
                .take(8)
                .collect();
            warn!(%id, ?similar, "project not found");
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": format!("project not found: {id}"),
                    "id": id,
                    "similar": similar,
                    "ok": false,
                })),
            )
                .into_response()
        }
    }
}

async fn api_project_git(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let id = urlencoding_decode(&id);
    let snap = state.snapshot.read().await;
    match snap.project_by_id(&id) {
        Some(p) => match &p.git {
            Some(g) => json_ok(g),
            None => json_error(StatusCode::NOT_FOUND, format!("project has no git repo: {id}")),
        },
        None => json_error(StatusCode::NOT_FOUND, format!("project not found: {id}")),
    }
}

#[derive(Debug, Deserialize)]
struct ProjectIdBody {
    project_id: String,
}

#[derive(Debug, Deserialize)]
struct TodoToggleBody {
    project_id: String,
    line: usize,
    done: bool,
}

async fn api_todo_toggle(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TodoToggleBody>,
) -> Response {
    let root = state.root.read().await.clone();
    let project_id = body.project_id.clone();
    let line = body.line;
    let done = body.done;
    info!(%project_id, line, done, "todo toggle");

    let mut snap = state.snapshot.write().await;
    let Some(project) = snap.project_by_id_mut(&project_id) else {
        warn!(%project_id, "toggle: project not found");
        return json_error(StatusCode::NOT_FOUND, format!("project not found: {project_id}"));
    };

    match todos::toggle_project_todo(&root, project, line, done) {
        Ok(item) => {
            recompute_totals(&mut snap);
            let project = match snap.project_by_id(&project_id) {
                Some(p) => p,
                None => {
                    error!(%project_id, "project missing after toggle");
                    return json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "project disappeared after toggle",
                    );
                }
            };
            info!(
                %project_id,
                line,
                done = item.done,
                open = project.open,
                "todo toggled"
            );
            json_ok(serde_json::json!({
                "ok": true,
                "item": item,
                "project": {
                    "id": project.id,
                    "open": project.open,
                    "done": project.done,
                    "total": project.total,
                    "items": project.items,
                },
                "totals": {
                    "total_open": snap.total_open,
                    "total_done": snap.total_done,
                    "total_items": snap.total_items,
                }
            }))
        }
        Err(e) => {
            error!(%project_id, line, error = %e, "todo toggle failed");
            json_error(StatusCode::BAD_REQUEST, e)
        }
    }
}

async fn api_agents_discover(State(state): State<Arc<AppState>>) -> Response {
    if !state.agents_active().await {
        return agents_disabled_response(state.allow_agents);
    }
    // Enumerating models shells out — keep it off the async runtime
    match tokio::task::spawn_blocking(agents::discover).await {
        Ok(d) => json_ok(d),
        Err(e) => {
            error!(error = %e, "agent discover panicked");
            json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("discover failed: {e}"),
            )
        }
    }
}

/* ---------- In-app agent sessions ---------- */

async fn api_sessions_list(State(state): State<Arc<AppState>>) -> Response {
    if !state.agents_active().await {
        return json_ok(serde_json::json!({ "sessions": [], "agents_active": false }));
    }
    let list = state.sessions.list().await;
    json_ok(serde_json::json!({ "sessions": list, "agents_active": true }))
}

async fn api_sessions_create(
    State(state): State<Arc<AppState>>,
    Json(mut req): Json<CreateSessionRequest>,
) -> Response {
    if !state.agents_active().await {
        return agents_disabled_response(state.allow_agents);
    }
    let root = state.root.read().await.clone();
    // Resolve project_id → cwd if needed
    if req.cwd.trim().is_empty() && !req.project_id.trim().is_empty() {
        let snap = state.snapshot.read().await;
        if let Some(p) = snap.project_by_id(req.project_id.trim()) {
            req.cwd = p
                .git
                .as_ref()
                .map(|g| g.root.clone())
                .unwrap_or_else(|| p.path.clone());
        }
    }
    if req.cwd.trim().is_empty() {
        req.cwd = root.to_string_lossy().into();
    }

    match state.sessions.create(req, root).await {
        Ok(s) => {
            info!(id = %s.id, cli = %s.cli, role = ?s.role, "session created");
            json_ok(s)
        }
        Err(e) => {
            error!(error = %e, "session create failed");
            json_error(StatusCode::BAD_REQUEST, e)
        }
    }
}

async fn api_sessions_get(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    match state.sessions.get(&id).await {
        Some((summary, messages, log)) => json_ok(serde_json::json!({
            "session": summary,
            "messages": messages,
            "log": log,
        })),
        None => json_error(StatusCode::NOT_FOUND, format!("session not found: {id}")),
    }
}

async fn api_sessions_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<SessionMessageRequest>,
) -> Response {
    match state.sessions.send_message(&id, body.text, vec![]).await {
        Ok(()) => json_ok(serde_json::json!({ "ok": true, "id": id })),
        Err(e) => json_error(StatusCode::BAD_REQUEST, e),
    }
}

async fn api_sessions_stop(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    match state.sessions.stop(&id).await {
        Ok(()) => json_ok(serde_json::json!({ "ok": true, "id": id })),
        Err(e) => json_error(StatusCode::BAD_REQUEST, e),
    }
}

async fn api_sessions_spawn(
    State(state): State<Arc<AppState>>,
    Path(parent_id): Path<String>,
    Json(mut req): Json<CreateSessionRequest>,
) -> Response {
    if !state.agents_active().await {
        return agents_disabled_response(state.allow_agents);
    }
    // Force parent linkage and worker role
    req.parent_id = Some(parent_id.clone());
    if req.role.is_none() {
        req.role = Some("worker".into());
    }
    let root = state.root.read().await.clone();
    if req.cwd.trim().is_empty() {
        // inherit parent cwd
        if let Some((parent, _, _)) = state.sessions.get(&parent_id).await {
            req.cwd = parent.cwd;
        } else {
            return json_error(
                StatusCode::NOT_FOUND,
                format!("parent session not found: {parent_id}"),
            );
        }
    }
    match state.sessions.create(req, root).await {
        Ok(s) => json_ok(s),
        Err(e) => json_error(StatusCode::BAD_REQUEST, e),
    }
}

async fn api_sessions_events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.sessions.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(move |item| {
        let id = id.clone();
        match item {
            Ok(ev) => {
                let keep = match &ev {
                    crate::sessions::SessionEvent::Snapshot { session } => session.id == id,
                    crate::sessions::SessionEvent::Status { session_id, .. } => *session_id == id,
                    crate::sessions::SessionEvent::Log { session_id, .. } => *session_id == id,
                    crate::sessions::SessionEvent::StreamDelta { session_id, .. } => {
                        *session_id == id
                    }
                    crate::sessions::SessionEvent::Message { session_id, .. } => *session_id == id,
                    crate::sessions::SessionEvent::ChildSpawned { parent_id, child } => {
                        *parent_id == id || child.id == id
                    }
                    crate::sessions::SessionEvent::Done { session_id, .. } => *session_id == id,
                    crate::sessions::SessionEvent::Error { session_id, .. } => *session_id == id,
                };
                if !keep {
                    return None;
                }
                let data = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
                Some(Ok(Event::default().data(data)))
            }
            Err(_) => None,
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn api_sessions_events_all(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.sessions.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|item| match item {
        Ok(ev) => {
            let data = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
            Some(Ok(Event::default().data(data)))
        }
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn api_agents_start(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AgentRequest>,
) -> Response {
    if !state.agents_active().await {
        return agents_disabled_response(state.allow_agents);
    }
    info!(
        project_id = %req.project_id,
        cwd = ?req.cwd,
        cli = %req.cli,
        mode = %req.mode,
        prompt_len = req.prompt.len(),
        "start agent"
    );

    let root = state.root.read().await.clone();

    // Resolve working directory:
    // 1) explicit cwd override
    // 2) project_id → project path / git root
    // 3) settings root (global default)
    let cwd = if let Some(cwd) = req.cwd.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        match fsbrowser::resolve_cwd(cwd) {
            Ok(p) => p,
            Err(e) => return json_error(StatusCode::BAD_REQUEST, e),
        }
    } else if !req.project_id.trim().is_empty() {
        let snap = state.snapshot.read().await;
        match snap.project_by_id(req.project_id.trim()) {
            Some(p) => p
                .git
                .as_ref()
                .map(|g| PathBuf::from(&g.root))
                .unwrap_or_else(|| PathBuf::from(&p.path)),
            None => {
                warn!(project_id = %req.project_id, "agent start: project not found");
                return json_error(
                    StatusCode::NOT_FOUND,
                    format!("project not found: {}", req.project_id),
                );
            }
        }
    } else {
        root.clone()
    };

    // Prefer paths under the configured root; still allow absolute override if it exists
    let safe = git::path_under_root(&root, &cwd).unwrap_or_else(|| cwd.clone());
    if !safe.is_dir() {
        return json_error(
            StatusCode::BAD_REQUEST,
            format!("working directory is not a directory: {}", safe.display()),
        );
    }

    // CRITICAL: never run process spawn on the async runtime — and never let agent
    // process lifetime be tied to the server's console/job (that was killing the server).
    let result = tokio::task::spawn_blocking(move || agents::start_agent(&req, &safe)).await;

    match result {
        Ok(Ok(result)) => {
            info!(
                cli = %result.cli,
                mode = %result.mode,
                cwd = %result.cwd,
                "agent started"
            );
            json_ok(result)
        }
        Ok(Err(e)) => {
            error!(error = %e, "agent start failed");
            json_error(StatusCode::BAD_REQUEST, e)
        }
        Err(e) => {
            error!(error = %e, "agent start task panicked");
            json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("agent start panicked: {e}"),
            )
        }
    }
}

async fn api_git_refresh(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ProjectIdBody>,
) -> Response {
    let id = body.project_id;
    info!(%id, "git refresh");
    let project_path = {
        let snap = state.snapshot.read().await;
        match snap.project_by_id(&id) {
            Some(p) => PathBuf::from(&p.path),
            None => return json_error(StatusCode::NOT_FOUND, format!("project not found: {id}")),
        }
    };

    let scan_root = state.root.read().await.clone();
    let git_info =
        match tokio::task::spawn_blocking(move || git::collect_git_info(&project_path, &scan_root))
            .await
        {
            Ok(v) => v,
            Err(e) => {
                error!(error = %e, "git refresh task failed");
                return json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("git refresh task failed: {e}"),
                );
            }
        };

    let mut snap = state.snapshot.write().await;
    if let Some(proj) = snap.project_by_id_mut(&id) {
        proj.git = git_info.clone();
    }
    snap.git_repo_count = snap.projects.iter().filter(|p| p.git.is_some()).count();

    match git_info {
        Some(g) => json_ok(g),
        None => json_error(StatusCode::NOT_FOUND, format!("not a git repo: {id}")),
    }
}

#[derive(Debug, Deserialize)]
struct PathAction {
    path: Option<String>,
    project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UrlAction {
    url: String,
}

async fn resolve_project_path(state: &AppState, body: &PathAction) -> Result<PathBuf, String> {
    let root = state.root.read().await.clone();
    if let Some(id) = &body.project_id {
        let snap = state.snapshot.read().await;
        let p = snap
            .project_by_id(id)
            .ok_or_else(|| format!("project not found: {id}"))?;
        let path = p
            .git
            .as_ref()
            .map(|g| PathBuf::from(&g.root))
            .unwrap_or_else(|| PathBuf::from(&p.path));
        return git::path_under_root(&root, &path)
            .ok_or_else(|| "path is outside repos root".to_string());
    }
    if let Some(path) = &body.path {
        let path = PathBuf::from(path);
        // Allow absolute paths that exist (folder browser / overrides)
        if path.is_dir() {
            return Ok(path.canonicalize().unwrap_or(path));
        }
        return git::path_under_root(&root, &path)
            .ok_or_else(|| "path is outside repos root".to_string());
    }
    Err("path or project_id required".into())
}

async fn api_open_folder(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PathAction>,
) -> Response {
    match resolve_project_path(&state, &body).await {
        Ok(path) => match git::open_folder(&path) {
            Ok(()) => {
                info!(path = %path.display(), "opened folder");
                json_ok(serde_json::json!({ "ok": true, "path": path }))
            }
            Err(e) => {
                error!(error = %e, "open folder failed");
                json_error(StatusCode::INTERNAL_SERVER_ERROR, e)
            }
        },
        Err(e) => json_error(StatusCode::BAD_REQUEST, e),
    }
}

async fn api_open_url(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<UrlAction>,
) -> Response {
    match git::open_url(&body.url) {
        Ok(()) => {
            info!(url = %body.url, "opened url");
            json_ok(serde_json::json!({ "ok": true, "url": body.url }))
        }
        Err(e) => {
            error!(error = %e, "open url failed");
            json_error(StatusCode::BAD_REQUEST, e)
        }
    }
}

async fn api_open_terminal(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PathAction>,
) -> Response {
    match resolve_project_path(&state, &body).await {
        Ok(path) => match git::open_terminal(&path) {
            Ok(()) => {
                info!(path = %path.display(), "opened terminal");
                json_ok(serde_json::json!({ "ok": true, "path": path }))
            }
            Err(e) => {
                error!(error = %e, "open terminal failed");
                json_error(StatusCode::INTERNAL_SERVER_ERROR, e)
            }
        },
        Err(e) => json_error(StatusCode::BAD_REQUEST, e),
    }
}

async fn api_refresh(State(state): State<Arc<AppState>>) -> Response {
    info!("full rescan requested");
    let root = state.root.read().await.clone();
    let started = Instant::now();
    let snapshot =
        match tokio::task::spawn_blocking(move || scanner::scan_repos(&root)).await {
            Ok(s) => s,
            Err(e) => {
                error!(error = %e, "rescan task panicked");
                return json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("rescan failed: {e}"),
                );
            }
        };

    info!(
        projects = snapshot.projects.len(),
        open = snapshot.total_open,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "rescan complete"
    );
    let view = snapshot.to_global_view();
    *state.snapshot.write().await = snapshot;
    json_ok(view)
}

fn urlencoding_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
