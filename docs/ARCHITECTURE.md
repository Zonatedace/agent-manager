# Architecture

Local **Rust / Axum** process serving a single-page dashboard and JSON APIs. No cloud backend of its own — it reads your repos and local agent credentials on disk.

## Runtime

### Desktop app mode (default on Windows)

```text
┌─────────────────────────────────────┐
│  todo-dashboard.exe                 │
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
- Release builds use `windows_subsystem = "windows"` (no console); logs still go to `todo-dashboard.log`.

### Server mode (`--mode server`)

Same Axum stack without a window; optional external browser via `open`.

- **UI**: `static/index.html` embedded at compile time (`include_str!`). Rebuild after HTML changes.
- **API**: Axum routes in `src/server.rs`.
- **Config**: `todo-dashboard.config.json` (settings API) + CLI flags (`--root`, `--port`, …).
- **Logs**: `todo-dashboard.log`.

### Dev process management (Windows)

| Script | Purpose |
|--------|---------|
| `run.ps1` | Foreground rebuild/run (Ctrl+C stops) |
| `restart.ps1` | Stop existing process, optional `cargo build --release`, start detached via WMI |
| `ensure-running.ps1` | Health-check and start if down |
| `install-autostart.ps1` | **Deprecated** — user prefers no Task Scheduler for this app |

Prod intent: container / k8s (not Task Scheduler). See open items in `TODO.md`.

## Modules (`src/`)

| Module | Responsibility |
|--------|----------------|
| `main.rs` | CLI, logging, boot (app vs server) |
| `desktop.rs` | Windows WebView2 window (tao/wry) |
| `server.rs` | HTTP routes, health, wiring |
| `scanner.rs` / `parser.rs` / `todos.rs` | Discover and parse/write TODO markdown |
| `models.rs` | Shared DTOs |
| `git.rs` | Git metadata and status |
| `agents.rs` | Discover CLIs; external agent launch (detached on Windows) |
| `sessions.rs` | In-app multi-turn sessions, SSE, orchestrator/sub-agents |
| `usage.rs` | Live Claude / Grok / Codex quota meters |
| `settings.rs` | Persistent settings |
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
- Usage and agent start use **your** existing CLI logins — do not expose the port on a public interface without auth.
- Never log full OAuth tokens; usage responses omit secrets.

## Build

```powershell
cargo build --release
# binary: target\release\todo-dashboard.exe
```

HTML and server code ship in one binary; `cargo build` is required after `static/` edits.
