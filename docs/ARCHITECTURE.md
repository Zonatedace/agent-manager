# Architecture — Agent Manager

Local **Rust** process: native Windows window (WebView2) + **Axum** JSON API on loopback. No cloud backend of its own — it scans your repos and reads local agent credentials.

| | |
|--|--|
| **Repo** | https://github.com/Zonatedace/agent-manager |
| **Binary** | `target\release\agent-manager.exe` |
| **API** | `http://127.0.0.1:7878/` |
| **Paths** | `.env` (`AGENT_MANAGER_ROOT`, …) — see `.env.example` |

## Runtime

### Desktop app mode (default on Windows)

```text
┌─────────────────────────────────────┐
│  agent-manager.exe                  │
│  ┌──────────────┐   ┌─────────────┐ │
│  │ WebView2     │──►│ Axum :7878  │ │
│  │ (tao + wry)  │   │ loopback    │ │
│  └──────────────┘   └──────┬──────┘ │
│                            │        │
│         scan TODOs / git / agents / usage
└─────────────────────────────────────┘
```

- Main thread: native window (`src/desktop.rs`, WebView2).
- Background Tokio runtime: HTTP API + sessions.
- Closing the window exits the process.
- Release builds use `windows_subsystem = "windows"` (no console); logs go to `agent-manager.log`.

### Server mode (`--mode server`)

Same Axum stack without a window; optional external browser via `open`. Bind with `--host 0.0.0.0` (or `AGENT_MANAGER_HOST`) so remote Windows clients can connect.

### Docker server (preferred always-on)

```text
┌─────────────────────────────┐
│  Docker: agent-manager      │
│  --mode server              │
│  AGENT_MANAGER_ALLOW_AGENTS=0
│  (no claude/grok/codex)     │
│  /data/repos  (bind mount)  │
└──────────────┬──────────────┘
               │ HTTP
     ┌─────────┴──────────┐
     │ Browser or         │
     │ --mode client      │
     └────────────────────┘
```

See [DOCKER.md](DOCKER.md). Coding agents are **not** in the image; enable them only on a host with CLIs (`Settings → Coding agents`).

### Agents capability model

| Layer | Meaning |
|-------|---------|
| `AGENT_MANAGER_ALLOW_AGENTS` | Process/host gate (`0` in Docker) |
| `settings.agents_enabled` | User toggle in Settings |
| `agents_active` | both true → discover/start/sessions/usage live |

Auth status is reported by `/api/agents/discover` from **local** credential files / PATH — never from the container.

### Client mode (`--mode client`)

Thin WebView2 shell only — **no** local scan or HTTP server.

```text
┌──────────────────────┐         ┌─────────────────────────┐
│ agent-manager.exe    │  HTTP   │ agent-manager --mode    │
│ --mode client        │────────►│ server  (:7878)         │
│ WebView2 + Server URL│         │ Axum + scan / agents    │
└──────────────────────┘         └─────────────────────────┘
```

- **Server URL** is required. Resolution: `--server-url` / `AGENT_MANAGER_SERVER_URL` → `agent-manager.client.json` → **setup prompt until provided**.
- On connect, the client probes `GET /api/health`, then persists the URL.
- Stale/unreachable saved URLs re-open the setup prompt. `--reset-server-url` forces it.
- Client config is separate from server settings (`agent-manager.client.json`).

| Artifact | Path / note |
|----------|-------------|
| UI | `static/index.html` embedded via `include_str!` (rebuild after HTML edits) |
| API | `src/server.rs` |
| Client shell | `src/desktop.rs` (`run_client_window`) + `src/client_config.rs` |
| Env | `.env` / `AGENT_MANAGER_*` (see `.env.example`) |
| Config | `agent-manager.config.json` (legacy `todo-dashboard.config.json` migrated if needed) |
| Client config | `agent-manager.client.json` (`server_url`) |
| Logs | `agent-manager.log` |

### Configuration priority

Scan root resolution:

1. `--force-root`
2. `--root` or `AGENT_MANAGER_ROOT` (from process env or `.env`)
3. `root` in `agent-manager.config.json`
4. Portable default (`~/Desktop/Repos`, `~/repos`, home, or cwd)

### Dev process management (Windows)

| Script | Purpose |
|--------|---------|
| `run.ps1` | Build if needed; launch desktop window (or `--server` / `--client`) |
| `restart.ps1` | Stop process, optional release build, start detached (WMI) |
| `ensure-running.ps1` | Health-check and start if down (prefers Windows Service if installed) |
| `install-desktop.ps1` | Start Menu + Desktop shortcuts → this checkout’s exe |
| `deploy-service.ps1` | **Build + deploy** Windows Service (admin) |
| `install-service.ps1` | Install / update / uninstall `AgentManager` service |
| `install-autostart.ps1` | Points at service deploy (Task Scheduler not used) |

Production intent for headless on Windows: **Windows Service** via `deploy-service.ps1`. Container / k8s remains an option for non-Windows hosts.

## Modules (`src/`)

| Module | Responsibility |
|--------|----------------|
| `main.rs` | CLI, dotenv, logging, boot (app / server / client / service), config path migration |
| `desktop.rs` | Windows WebView2 window (tao/wry); client setup prompt for Server URL |
| `client_config.rs` | Client `server_url` load/save + health probe |
| `service.rs` | Windows Service dispatcher + stop/status (Windows only) |
| `server.rs` | HTTP routes, health, graceful shutdown |
| `scanner.rs` / `parser.rs` / `todos.rs` | Discover and parse/write TODO markdown |
| `models.rs` | Shared DTOs |
| `git.rs` | Git metadata and status |
| `agents.rs` | Discover CLIs; external agent launch (detached on Windows) |
| `sessions.rs` | In-app multi-turn sessions, SSE, orchestrator/sub-agents |
| `usage.rs` | Live Claude / Grok / Codex quota meters |
| `settings.rs` | Persistent settings + portable default root |
| `fsbrowser.rs` | Directory listing for project browser |

## Sessions (high level)

1. UI creates a session → server spawns headless agent with streaming output.
2. Events broadcast over SSE (`/api/sessions/events`, per-session streams).
3. Grok `streaming-json` is decoded to assistant text/data only (thought/raw frames dropped for display).
4. Orchestrator mode can spawn sub-agents against project workspaces.

## Usage meters (high level)

Blocking collectors in `usage.rs` run under `spawn_blocking` for `GET /api/usage`.

- Per-provider **min TTL cache** + **429 backoff** (critical for Anthropic).
- Multi-window model: soft vs hard, reset timestamps, severity.
- Details: [`USAGE.md`](USAGE.md).

## Security notes

- Binds to localhost by default; treat as a local-dev tool.
- Usage and agent start use **your** existing CLI logins — do not expose the port publicly without auth.
- Never log full OAuth tokens; usage responses omit secrets.
- Do not commit `.env`, config JSON with local paths, or credential files.

## Build

```powershell
git clone https://github.com/Zonatedace/agent-manager.git
cd agent-manager
copy .env.example .env
# set AGENT_MANAGER_ROOT
cargo build --release
# binary: target\release\agent-manager.exe
```

HTML and server code ship in one binary; `cargo build` is required after `static/` edits.
