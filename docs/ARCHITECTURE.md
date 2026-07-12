# Architecture — Agent Manager

Local **Rust** process: native Windows window (WebView2) + **Axum** JSON API on loopback. No cloud backend of its own — it scans your repos and reads local agent credentials.

| | |
|--|--|
| **Repo** | https://github.com/Zonatedace/agent-manager |
| **Full app** | `target\release\agent-manager.exe` |
| **Thin client** | `target\release\agent-manager-client.exe` |
| **API** | `http://127.0.0.1:7878/` |
| **Paths** | `.env` (`AGENT_MANAGER_ROOT`, …) — see `.env.example` |

Browser and Windows client load the **same** embedded UI (`static/index.html`).

## Binaries

| Binary | Role |
|--------|------|
| `agent-manager.exe` | Full app (default: local server + window), or `--mode server` / `service` / `client` |
| `agent-manager-client.exe` | **Always** thin client — WebView only, connects via Server URL |

Both are built from the same `src/main.rs` (`Cargo.toml` `[[bin]]` entries). The client binary forces client mode from its executable name.

```powershell
cargo build --release
# produces both agent-manager.exe and agent-manager-client.exe
```

## Runtime

### Desktop app mode (default on Windows — `agent-manager.exe`)

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
- Background `git` / CLI spawns use `CREATE_NO_WINDOW` so child consoles do not flash (`src/process_util.rs`).

### Server mode (`--mode server`)

Same Axum stack without a window; optional external browser via `open`. Bind with `--host 0.0.0.0` (or `AGENT_MANAGER_HOST`) so remote Windows clients can connect.

**Detached starts** (WMI / `restart.ps1` / `ensure-running.ps1`) must not use panicking `println!` — stdout may be closed. Safe writers are used instead.

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
     │ agent-manager-     │
     │   client.exe       │
     └────────────────────┘
```

See [DOCKER.md](DOCKER.md). Coding agents are **not** in the image; enable them only on a host with CLIs (`Settings → Coding agents`).

### Agents capability model

| Layer | Meaning |
|-------|---------|
| `AGENT_MANAGER_ALLOW_AGENTS` | Process/host gate (`0` in Docker) |
| `settings.agents_enabled` | User toggle in Settings |
| `agents_active` | both true → discover/start/sessions/usage live |

Auth status is reported by `/api/agents/discover` from **local** credential files / PATH — never from the container. The shared UI only calls discover when agents are active (avoids 403 noise when off).

### Client mode (`agent-manager-client.exe` or `--mode client`)

Thin WebView2 shell only — **no** local scan or HTTP server. Prefer the dedicated binary:

| Binary | Behavior |
|--------|----------|
| `agent-manager-client.exe` | Always client mode (double-clickable) |
| `agent-manager.exe --mode client` | Same, via flag |

```text
┌────────────────────────────┐         ┌─────────────────────────┐
│ agent-manager-client.exe   │  HTTP   │ agent-manager --mode    │
│ WebView2 + Server URL      │────────►│ server  (:7878)         │
└────────────────────────────┘         │ Axum + scan / agents    │
                                       └─────────────────────────┘
```

- **Server URL** is required. Resolution: `--server-url` / `AGENT_MANAGER_SERVER_URL` → `agent-manager.client.json` → **setup prompt until provided**.
- On connect, the client probes `GET /api/health`, then persists the URL.
- Stale/unreachable saved URLs re-open the setup prompt. `--reset-server-url` forces it; under WebView, the header **Change server** button uses IPC to re-open setup.
- Client config is separate from server settings (`agent-manager.client.json`).
- Default log file for the client binary: `agent-manager-client.log`.
- UI chrome shows connection origin and mode label (`local` / `web` / `desktop` / `client`).

| Artifact | Path / note |
|----------|-------------|
| UI | `static/index.html` embedded via `include_str!` (rebuild after HTML edits) |
| API | `src/server.rs` |
| Client shell | `src/desktop.rs` (`run_client_window`) + `src/client_config.rs` |
| Env | `.env` / `AGENT_MANAGER_*` (see `.env.example`) |
| Config | `agent-manager.config.json` (legacy `todo-dashboard.config.json` migrated if needed) |
| Client config | `agent-manager.client.json` (`server_url`) |
| Logs | `agent-manager.log` / `agent-manager-client.log` |

### Configuration priority

Scan root resolution:

1. `--force-root`
2. `--root` or `AGENT_MANAGER_ROOT` (from process env or `.env`)
3. `root` in `agent-manager.config.json`
4. Portable default (`~/Desktop/Repos`, `~/repos`, home, or cwd)

Client Server URL resolution:

1. `--server-url` / `AGENT_MANAGER_SERVER_URL`
2. `server_url` in `agent-manager.client.json`
3. Setup prompt (required until provided)

### Dev process management (Windows)

| Script | Purpose |
|--------|---------|
| `run.ps1` | Build if needed; launch app, `--server`, or `--client` (client exe) |
| `restart.ps1` | Stop process, optional release build, start detached (WMI) |
| `ensure-running.ps1` | Health-check and start if down (prefers Windows Service if installed) |
| `install-desktop.ps1` | Start Menu + Desktop: **Agent Manager** + **Agent Manager Client** |
| `deploy-service.ps1` | **Build + deploy** Windows Service (admin) |
| `install-service.ps1` | Install / update / uninstall `AgentManager` service |
| `install-autostart.ps1` | Points at service deploy (Task Scheduler not used) |

Production intent for headless on Windows: **Windows Service** via `deploy-service.ps1`. Container remains an option for non-Windows hosts.

## Modules (`src/`)

| Module | Responsibility |
|--------|----------------|
| `main.rs` | CLI, dotenv, logging, boot (app / server / client / service); shared by both bins |
| `process_util.rs` | Windows `CREATE_NO_WINDOW` for background child processes |
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
# binaries:
#   target\release\agent-manager.exe
#   target\release\agent-manager-client.exe
```

HTML and server code ship in both binaries; `cargo build` is required after `static/` edits.
