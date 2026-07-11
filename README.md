# Agent Manager

[![GitHub](https://img.shields.io/badge/GitHub-Zonatedace%2Fagent--manager-181717?logo=github)](https://github.com/Zonatedace/agent-manager)

| | |
|--|--|
| **Repository** | https://github.com/Zonatedace/agent-manager |
| **Local path** | `C:\Users\Brandon\Desktop\Repos\agent-manager` |
| **Binary** | `target\release\agent-manager.exe` |
| **Default scan root** | `C:\Users\Brandon\Desktop\Repos` (configurable) |
| **Local API** | http://127.0.0.1:7878/ |

Formerly **todo-dashboard**. **Windows desktop app** (WebView2) for multi-repo `TODO.md` management, git actions, in-app multi-agent sessions, and live **Claude / Grok / Codex** usage meters.

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
- Detached spawn on Windows so agent start does not kill the app process

### Usage meters

Header meters for **Claude**, **Grok**, and **Codex**:

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

### Clone / open

```powershell
cd C:\Users\Brandon\Desktop\Repos\agent-manager
```

### Install shortcuts (Start Menu + Desktop)

```powershell
.\install-desktop.ps1
```

Then launch **Agent Manager** from the Start Menu or Desktop. Closing the window stops the app.

### Dev / rebuild

```powershell
.\run.ps1                 # build if needed, open desktop window
.\restart.ps1             # rebuild + restart detached
.\restart.ps1 --no-build
.\ensure-running.ps1      # start only if /api/health is down
cargo run --release       # desktop window (default on Windows)
```

### Server-only mode (no window)

```powershell
.\run.ps1 --server
# or
.\target\release\agent-manager.exe --mode server
.\target\release\agent-manager.exe --mode server --no-open
```

### CLI options

```text
agent-manager --mode app              # native window (default on Windows)
agent-manager --mode server           # HTTP only
agent-manager --server                # alias for --mode server
agent-manager --console               # attach console on GUI builds
agent-manager --root "D:\Repos" --port 7878
agent-manager --log-file agent-manager.log --log-level info
agent-manager --config agent-manager.config.json
```

> After editing `static/index.html`, rebuild — the UI is embedded in the binary.
> Logs: `agent-manager.log`. Config: `agent-manager.config.json` (legacy `todo-dashboard.config.json` is migrated if present).

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
Repos\agent-manager\          # this repo (local checkout)
  Cargo.toml
  README.md
  TODO.md
  docs/
    ARCHITECTURE.md
    USAGE.md
  assets/
    icon.ico
  src/
    main.rs desktop.rs server.rs usage.rs sessions.rs …
  static/
    index.html                 # UI (embedded at build)
  run.ps1 restart.ps1 ensure-running.ps1 install-desktop.ps1
  agent-manager.config.json    # local (gitignored)
  agent-manager.log            # local (gitignored)
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

MIT (see repository); local credentials and config stay on your machine.
