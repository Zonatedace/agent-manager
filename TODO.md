# TODO — agent-manager

Local backlog for **Agent Manager**.  
Repo: https://github.com/Zonatedace/agent-manager  

Checked items are done; open items are still planned.

## Project identity & packaging

- [x] Rename product from todo-dashboard → **Agent Manager** / `agent-manager`
- [x] Public GitHub repo `Zonatedace/agent-manager`
- [x] Binary `agent-manager.exe`, config `agent-manager.config.json`, log `agent-manager.log`
- [x] Dedicated thin client binary `agent-manager-client.exe` (always client mode)
- [x] Legacy config migration from `todo-dashboard.config.json`
- [x] Start Menu / Desktop shortcuts for app + client (`install-desktop.ps1`)
- [x] GitHub link in app chrome header
- [x] Paths via `.env` / `AGENT_MANAGER_*` (no personal machine paths in repo)
- [x] README / ARCHITECTURE / DOCKER / USAGE docs match dual-binary setup
- [ ] Embed window icon in exe resources reliably (`winres` + `assets/icon.ico`)
- [ ] Single-file installer (MSIX / Inno Setup / cargo-packager)
- [ ] System tray minimize instead of full exit on close
- [ ] Optional “Open data folder” action (config + logs directory)

## Usage meters (Claude / Grok / Codex)

- [x] Live usage API `GET /api/usage` aggregating three agent CLIs
- [x] Claude Code OAuth usage (`five_hour` / `seven_day` / `limits[]` soft session + hard weekly)
- [x] Grok billing dual fetch: weekly soft credits (`?format=credits`) + monthly hard spend
- [x] Codex wham usage: 5h soft (`primary_window`) + weekly hard (`secondary_window`)
- [x] Soft/hard kind tags, severity coloring, multi-window bars in UI
- [x] Live reset countdown timers per window (client tick from `resets_at`)
- [x] Server-side cache + 429 backoff for Anthropic usage endpoint
- [x] Document usage sources, windows, and rate-limit behavior (`docs/USAGE.md`)
- [ ] Persist last-good usage snapshot to disk so UI survives process restart mid-429
- [ ] Optional settings: poll interval, which providers to enable
- [ ] Surface Claude `extra_usage` / spend windows when enabled on the account
- [ ] Surface Codex banked rate-limit reset credits as a clickable action (if API allows)
- [ ] Unit tests for usage JSON parsers (fixture responses per provider)

## In-app multi-agent sessions

- [x] Background concurrent sessions (Grok / Claude / Codex)
- [x] SSE event stream for live transcript updates
- [x] Stream decoder for Grok `streaming-json` (text/data only, not raw thought dumps)
- [x] Orchestrator + sub-agent spawn from UI
- [ ] Session history search / filter by project and agent
- [ ] Resume external CLI sessions by id when supported
- [ ] Per-session token/cost estimate when provider reports usage on the turn
- [ ] Document orchestrator UX in `docs/SESSIONS.md`

## Core product

- [x] Global + per-project TODO views
- [x] Checkbox write-back to `TODO.md`
- [x] Git metadata + open folder / terminal / remote
- [x] Settings UI (root path, refresh intervals)
- [x] Filesystem browser for project context
- [x] Dev runners: `run.ps1`, `restart.ps1`, `ensure-running.ps1` (no Task Scheduler)
- [x] Windows desktop app (WebView2 via tao/wry; `--mode server` for headless)
- [x] Thin Windows client (`--mode client`) with required **Server URL** (prompt until set)
- [x] Dedicated client binary `agent-manager-client.exe` (double-clickable thin client)
- [x] Shared UI parity: browser + WebView same HTML; connection chrome + Change server (IPC)
- [x] Client config `agent-manager.client.json` + `AGENT_MANAGER_SERVER_URL` / `--server-url`
- [x] Server bind host (`--host` / `AGENT_MANAGER_HOST`) for LAN clients
- [x] Windows Service host (`--mode service`) with graceful stop (optional; Docker preferred for always-on)
- [x] `install-service.ps1` / `deploy-service.ps1` (build + redeploy service)
- [x] Docker server (`Dockerfile` + `docker-compose.yml`) — **no** agent CLIs in image
- [x] Settings: enable/disable coding agents + per-provider toggles; auth status on front-end
- [x] `AGENT_MANAGER_ALLOW_AGENTS=0` in container; agents gated on API
- [ ] Health banner auto-restart option (opt-in)
- [ ] Optional local agent bridge so Docker UI + Windows client can run CLIs on the client only
- [ ] Export / print filtered TODO list
- [ ] Dark/light theme toggle

## Reliability

- [x] Detached agent spawn so starting an agent does not kill the app process
- [x] Panic catch on HTTP layer + logging to `agent-manager.log`
- [x] Safe stdout/stderr on detached server start (no panic when pipe closed)
- [x] `CREATE_NO_WINDOW` for background `git`/CLI spawns (no console flash under GUI)
- [ ] Graceful shutdown + drain active sessions on window close
- [ ] Integration smoke test script (`ensure-running` + `/api/health` + `/api/usage`)
- [ ] After folder moves: document re-running `install-desktop.ps1` so shortcuts stay valid

## Docs

- [x] README feature/API overview (name, path, binary, GitHub)
- [x] `docs/USAGE.md` — agent usage meters
- [x] `docs/ARCHITECTURE.md` — layout and runtime notes
- [x] This `TODO.md` backlog (aligned with Agent Manager)
- [ ] Short `docs/SESSIONS.md` for orchestrator / multi-agent UX
- [ ] `CHANGELOG.md` for releases / packaging
