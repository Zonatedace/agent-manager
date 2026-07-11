# TODO Dashboard

[![GitHub](https://img.shields.io/badge/GitHub-Zonatedace%2Ftodo--dashboard-181717?logo=github)](https://github.com/Zonatedace/todo-dashboard)

**Repository:** https://github.com/Zonatedace/todo-dashboard

Local **Rust** web dashboard for `TODO.md` files under your project repos: global and per-project views, git actions, in-app multi-agent sessions, and live **Claude / Grok / Codex** usage meters.

Default scan root: your projects folder (configurable). UI: http://127.0.0.1:7878/

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

- [Rust](https://rustup.rs/) 1.75+
- Optional: `claude`, `grok`, `codex` CLIs on `PATH` (agents + usage meters)
- Windows scripts assume PowerShell

## Run

### Foreground (dev)

```powershell
cd todo-dashboard
.\run.ps1              # build if needed, run in this window
# or
cargo run --release
```

### Rebuild & restart (detached, no Task Scheduler)

```powershell
.\restart.ps1          # cargo build --release + WMI-detached start
.\restart.ps1 --no-build
.\ensure-running.ps1   # start only if /api/health is down
```

Opens (or serves) **http://127.0.0.1:7878/**

### CLI options

```text
todo-dashboard --root "C:\Users\Brandon\Desktop\Repos" --port 7878
todo-dashboard --no-open
todo-dashboard --log-file todo-dashboard.log --log-level info
```

> After editing `static/index.html`, rebuild (`cargo build --release` / `restart.ps1`) — the UI is embedded in the binary.

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
todo-dashboard/
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
