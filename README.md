# Agent Manager

[![GitHub](https://img.shields.io/badge/GitHub-Zonatedace%2Fagent--manager-181717?logo=github)](https://github.com/Zonatedace/agent-manager)

**Repository:** https://github.com/Zonatedace/agent-manager

Formerly **todo-dashboard**. **Windows desktop app** (native window via WebView2) for `TODO.md` across your repos: global and per-project views, git actions, in-app multi-agent sessions, and live **Claude / Grok / Codex** usage meters.

Default scan root: your projects folder (configurable). The UI runs in an OS window; a local HTTP API still listens on `http://127.0.0.1:7878/` for health checks and tooling.

## Features

### TODO scanning & editing

- Finds `TODO.md` / `todo.md` / `TODOS.md` (skips `node_modules`, `.git`, `target`, `dist`, …)
- Parses markdown checkboxes (`- [ ]` / `- [x]`) with section headings
- **Global overview**: stats, project cards, open items
- **Per-project view**: filter open/done, click checkboxes to write back to disk
- **Refresh** re-scans without restart

### Git

When a project is in a git worktree:

- Branch, dirty/clean, staged/unstaged/untracked, ahead/behind, remote, last commit
- Actions: open folder, terminal, remote URL, copy links, refresh status

### Agents & sessions

- **External launch**: Grok / Claude / Codex in the project directory (model, effort, permissions, interactive or headless)
- **In-app sessions**: concurrent background runs, live SSE transcript, chat, orchestrator + sub-agents
- Stream parsing for Grok `streaming-json` (assistant text, not raw thought dumps)
- Detached spawn on Windows so agent start does not kill the dashboard process

### Usage meters

Sticky header meters for **Claude**, **Grok**, and **Codex**:

- Soft + hard windows (e.g. 5h + weekly, weekly credits + monthly spend)
- Live **reset countdown** on every window
- Server cache and 429 backoff (Anthropic usage API)

Details: **[docs/USAGE.md](docs/USAGE.md)** · architecture: **[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)** · backlog: **[TODO.md](TODO.md)**

## Requirements

- Windows 10/11 with **Microsoft Edge WebView2 Runtime** (usually preinstalled)
- [Rust](https://rustup.rs/) 1.75+ to build
- Optional: `claude`, `grok`, `codex` CLIs on `PATH` (agents + usage meters)
- PowerShell for helper scripts

## Run (Windows app)

### Install shortcuts (Start Menu + Desktop)

```powershell
cd agent-manager
.\install-desktop.ps1
```

Then launch **Agent Manager** from the Start Menu or Desktop. Closes when you close the window.

### Dev / rebuild

```powershell
.\run.ps1                 # build if needed, open desktop window
.\restart.ps1             # rebuild + restart detached
.\restart.ps1 --no-build
cargo run --release       # desktop window (default on Windows)
```

### Server-only mode (no window)

Useful for scripting, remote access over loopback, or containers later:

```powershell
.\run.ps1 --server
# or
agent-manager.exe --mode server
agent-manager.exe --mode server --no-open
```

### CLI options

```text
agent-manager --mode app              # native window (default on Windows)
agent-manager --mode server           # HTTP only
agent-manager --server                # alias for --mode server
agent-manager --console               # attach console on GUI builds
agent-manager --root "D:\Repos" --port 7878
agent-manager --log-file agent-manager.log --log-level info
```

> After editing `static/index.html`, rebuild — the UI is embedded in the binary.
> Logs always go to `agent-manager.log` (GUI builds hide the console unless `--console`).

## API (selected)

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/health` | Health + project counts |
| GET | `/api/global` | Aggregate stats + open items |
| GET | `/api/projects` | Project summaries |
| GET | `/api/projects/{id}` | Project todos + git |
| GET | `/api/usage` | Claude / Grok / Codex usage windows |
| GET/PUT | `/api/settings` | Persistent settings |
| POST | `/api/todos/toggle` | Check/uncheck TODO line |
| POST | `/api/refresh` | Re-scan disk |
| GET | `/api/agents/discover` | CLIs on PATH + presets |
| POST | `/api/agents/start` | External agent launch |
| GET/POST | `/api/sessions` | List / create in-app sessions |
| GET | `/api/sessions/events` | SSE (all sessions) |
| GET | `/api/sessions/{id}` | Session detail |
| POST | `/api/sessions/{id}/message` | Send message |
| POST | `/api/sessions/{id}/stop` | Stop session |
| POST | `/api/sessions/{id}/spawn` | Spawn sub-agent |
| GET | `/api/fs/list` | Filesystem browser |
| POST | `/api/actions/open-folder` | Explorer |
| POST | `/api/actions/open-terminal` | PowerShell |
| POST | `/api/actions/open-url` | Browser |

## Project layout

```text
agent-manager/
  Cargo.toml
  README.md
  TODO.md                 # backlog (this repo)
  docs/
    ARCHITECTURE.md
    USAGE.md              # usage meters deep-dive
  src/
    main.rs
    server.rs
    usage.rs              # agent quota collectors
    sessions.rs           # multi-agent sessions
    agents.rs scanner.rs parser.rs todos.rs git.rs …
  static/
    index.html            # UI (embedded at build)
  run.ps1 restart.ps1 ensure-running.ps1
```

## Keyboard (UI)

| Key | Action |
|-----|--------|
| `/` | Focus filter |
| `r` | Refresh scan |
| `g` | Global view |
| `a` | Agent / session actions |

## Security

Local-dev tool: binds to loopback, uses your existing agent logins on disk. Do not expose `:7878` on a public network without additional auth.

## License

Private / local use unless noted otherwise.
