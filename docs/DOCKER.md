# Docker — Agent Manager server

Run the **HTTP API + UI** in a container. The image **does not install** `claude`, `grok`, or `codex`.

Coding agents are an **app-side** feature: enable them in **Settings** on a Windows host where those CLIs are installed and authenticated. Auth is always local (front-end / CLI login) — never baked into the container.

## Quick start

```powershell
# Point at the folder that contains your repos
$env:AGENT_MANAGER_ROOT = "C:\Users\you\Desktop\Repos"

cd path\to\agent-manager
docker compose up -d --build
```

Open http://127.0.0.1:7878/ or use the thin Windows client:

```powershell
.\target\release\agent-manager.exe --mode client --server-url http://127.0.0.1:7878/
```

## What runs in the container

| Included | Not included |
|----------|----------------|
| Axum API, embedded UI | Claude / Grok / Codex CLIs |
| TODO scan + write-back | Agent auth / OAuth |
| Git metadata | In-container agent sessions |
| Settings (agents force-off) | Usage meters (until agents enabled on a host that allows them) |

`AGENT_MANAGER_ALLOW_AGENTS=0` is set in the image and compose file. Enabling agents in Settings on this server is ignored.

## Volumes

| Mount | Purpose |
|-------|---------|
| `AGENT_MANAGER_ROOT` → `/data/repos` | Scan root for `TODO.md` |
| volume `agent-manager-config` | `agent-manager.config.json` |
| volume `agent-manager-logs` | `agent-manager.log` |

## Agents on the app (not Docker)

1. Run Agent Manager **app mode** on Windows (local process, not the container), **or** plan to keep agents on a host with `AGENT_MANAGER_ALLOW_AGENTS=1` and CLIs installed.
2. Open **Settings → Coding agents → Enable**.
3. Sign in with each CLI **on that machine**:
   - Claude: `claude /login`
   - Codex: `codex login`
   - Grok: Grok CLI / xAI credentials under your user profile
4. The UI shows local install + auth status (discover API). Credentials never go into the Docker image.

Connecting a pure WebView client to Docker alone cannot run agents — there is no CLI inside the container.

## Rebuild / redeploy

```powershell
docker compose up -d --build
docker compose logs -f agent-manager
docker compose down
```

## Environment

| Variable | Default in compose |
|----------|--------------------|
| `AGENT_MANAGER_ROOT` | **required** (host path) |
| `AGENT_MANAGER_PORT` | `7878` |
| `AGENT_MANAGER_ALLOW_AGENTS` | `0` |
| `AGENT_MANAGER_LOG_LEVEL` | `info` |
