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

Same Axum stack without a window; optional external browser via `open`.

| Artifact | Path / note |
|----------|-------------|
| UI | `static/index.html` embedded via `include_str!` (rebuild after HTML edits) |
| API | `src/server.rs` |
| Env | `.env` / `AGENT_MANAGER_*` (see `.env.example`) |
| Config | `agent-manager.config.json` (legacy `todo-dashboard.config.json` migrated if needed) |
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
| `run.ps1` | Build if needed; launch desktop window (or `--server`) |
| `restart.ps1` | Stop process, optional release build, start detached (WMI) |
| `ensure-running.ps1` | Health-check and start if down |
| `install-desktop.ps1` | Start Menu + Desktop shortcuts → this checkout’s exe |
| `install-autostart.ps1` | **Deprecated** — no Task Scheduler for this app |

Production intent for headless: container / k8s using **server mode** (not Task Scheduler). See `TODO.md`.

## Modules (`src/`)

| Module | Responsibility |
|--------|----------------|
| `main.rs` | CLI, dotenv, logging, boot (app vs server), config path migration |
| `desktop.rs` | Windows WebView2 window (tao/wry) |
| `server.rs` | HTTP routes, health, wiring |
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
