# TODO — todo-dashboard

Local backlog for the TODO Dashboard. Checked items are done; open items are still planned.

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

## Dashboard core

- [x] Global + per-project TODO views
- [x] Checkbox write-back to `TODO.md`
- [x] Git metadata + open folder / terminal / remote
- [x] Settings UI (root path, refresh intervals)
- [x] Filesystem browser for project context
- [x] Dev runners: `run.ps1`, `restart.ps1`, `ensure-running.ps1` (no Task Scheduler)
- [ ] Container / k8s deployment manifests (prod target)
- [ ] Health banner auto-restart option (opt-in, still no Task Scheduler unless user chooses)
- [ ] Export / print filtered TODO list
- [ ] Dark/light theme toggle

## Reliability

- [x] Detached agent spawn so starting an agent does not kill the dashboard process
- [x] Panic catch on HTTP layer + logging to `todo-dashboard.log`
- [ ] Graceful shutdown + drain active sessions
- [ ] Integration smoke test script (`ensure-running` + `/api/health` + `/api/usage`)

## Docs

- [x] README feature/API overview
- [x] `docs/USAGE.md` — agent usage meters
- [x] `docs/ARCHITECTURE.md` — layout and runtime notes
- [x] This `TODO.md` backlog
- [ ] Short `docs/SESSIONS.md` for orchestrator / multi-agent UX
- [ ] Changelog / release notes when packaging for k8s
