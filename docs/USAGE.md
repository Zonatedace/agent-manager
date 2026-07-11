# Agent usage meters

Agent Manager’s header shows live **Claude**, **Grok**, and **Codex** usage. Each card can show multiple **windows** (soft vs hard limits) with fill bars, used %, and a **live reset countdown**.

| | |
|--|--|
| **App** | Agent Manager (`Repos\agent-manager`) |
| **Endpoint** | `GET http://127.0.0.1:7878/api/usage` |
| **UI poll** | Every **60s** (client); server caches longer per provider |
| **Timers** | Tick every **1s** from `resets_at` between polls |

Tokens never leave the machine in the JSON response (only plan labels, percents, and timers). Credentials are read from local CLI auth files.

---

## Response shape (summary)

```json
{
  "fetched_at": "…",
  "meters": [
    {
      "id": "claude|grok|codex",
      "label": "Claude",
      "logged_in": true,
      "plan": "pro",
      "account": "…",
      "used_percent": 3.0,
      "windows": [
        {
          "id": "5h",
          "label": "5h session",
          "used_percent": 3.0,
          "kind": "soft",
          "severity": "normal",
          "is_hard_limit": false,
          "resets_at": "2026-07-11T20:29:59Z",
          "resets_in_seconds": 7200,
          "detail": null
        }
      ],
      "source": "…",
      "error": null
    }
  ]
}
```

| Field | Meaning |
|--------|---------|
| `kind` | `soft` (burst / included pool) or `hard` (plan hard gate / spend cap), plus `credits` / `spend` / `rate` when relevant |
| `severity` | UI heat: `normal` → `warn` → `hot` → `hard` |
| `resets_at` | Absolute UTC reset; UI ticks countdown every second from this |
| `used_percent` (meter-level) | Hottest / most constraining window |

---

## Claude Code

| | |
|--|--|
| **Installed when documented** | Claude Code **2.1.137** |
| **Credentials** | `~/.claude/.credentials.json` → `claudeAiOauth.accessToken` |
| **Endpoint** | `GET https://api.anthropic.com/api/oauth/usage` |
| **Headers** | `Authorization: Bearer …`, `anthropic-version: 2023-06-01`, `User-Agent: claude-code/…` |

### Windows

| Window | Kind | Source fields |
|--------|------|----------------|
| 5h session | **soft** | `limits[]` kind `session` / group `session`, or `five_hour.utilization` |
| weekly | **hard** | `limits[]` kind `weekly_all`, or `seven_day.utilization` |
| week \<model\> | **hard** | scoped weekly (`seven_day_sonnet`, `seven_day_opus`, `weekly_scoped`) |
| extra monthly/daily/weekly | credits | `extra_usage` when `is_enabled` |
| spend | **hard** | `spend` when `enabled` |

Session is the short rolling gate; weekly is the longer plan pool. Both apply.

### Rate limits (429)

Anthropic rate-limits `/api/oauth/usage` if polled too often. Agent Manager:

- Caches a successful Claude reading for **≥ 120s**
- On **429**, backs off (~5 minutes or `Retry-After`) and serves **stale** bars when available
- UI polls `/api/usage` every **60s** (not 30s)

Do not re-enable sub-30s polling without revisiting cache TTLs.

---

## Grok (Grok Build TUI)

| | |
|--|--|
| **Installed when documented** | Grok **0.2.93** |
| **Credentials** | `~/.grok/auth.json` (OIDC entry → `key`, `user_id`, `email`) |
| **Same as TUI** | Slash command `/usage show` |

### Endpoints (cli-chat-proxy)

| URL | Role |
|-----|------|
| `GET https://cli-chat-proxy.grok.com/v1/billing?format=credits` | **Weekly soft** credit pool (product usage, e.g. GrokBuild %) |
| `GET https://cli-chat-proxy.grok.com/v1/billing` | **Monthly hard** spend (`used` / `monthlyLimit`) |
| `GET https://cli-chat-proxy.grok.com/v1/user?include=subscription` | Plan tier (e.g. `GrokPro`) |

Required headers include:

- `Authorization: Bearer <OIDC token>`
- `X-XAI-Token-Auth: xai-grok-cli`
- `x-userid: <user_id>` (recommended)

### Windows

| Window | Kind | Notes |
|--------|------|--------|
| week build / week chat | **soft** | From `productUsage` + `creditUsagePercent`; period end → reset |
| monthly | **hard** | `used.val / monthlyLimit.val` (e.g. `$2870 / $15000`) |
| on-demand | **hard** | Paid overage when cap configured |

`api.x.ai/v1/models` alone is **not** a usage meter (only API health). Always use billing for %.

---

## Codex CLI

| | |
|--|--|
| **Installed when documented** | Codex CLI **0.144.1** (`@openai/codex`) |
| **Credentials** | `~/.codex/auth.json` → `tokens.access_token`, `tokens.account_id` |
| **Endpoint** | `GET https://chatgpt.com/backend-api/wham/usage` |
| **Headers** | `Authorization: Bearer …`, `ChatGPT-Account-Id` when present |

### Windows

| Window | Kind | Source |
|--------|------|--------|
| 5h | **soft** | `rate_limit.primary_window` — `limit_window_seconds` ≈ **18000** |
| week | **hard** | `rate_limit.secondary_window` — ≈ **604800** |
| credits | credits | Flexible overage when `has_credits` |
| spend | **hard** | Only if `spend_control.individual_limit` is non-null |

**Both 5h and weekly apply in parallel** on each request (not sequential buckets). Identify windows by `limit_window_seconds`, not by name alone.

Also exposed when present:

- `rate_limit.limit_reached` / `allowed`
- `rate_limit_reset_credits.available_count` (banked free resets) → meter `detail`

---

## UI behavior

- Header cards: one row per window — **label · soft/hard tag · bar · % · reset timer**
- Timer formats: `5d 9h`, `2h 55m`, `12m`, `45s`, `now`
- Timer color: warning when &lt; 15 minutes
- Hover row for absolute reset time and dollar detail (Grok monthly, etc.)
- Client ticks every **1s** from `resets_at` between 60s API polls
- Server caches per provider; Claude 429 does not wipe other meters

---

## Local verification

```powershell
# After Agent Manager is up on :7878
Invoke-RestMethod http://127.0.0.1:7878/api/usage |
  Select-Object -ExpandProperty meters |
  ForEach-Object {
    "{0} plan={1}" -f $_.id, $_.plan
    $_.windows | ForEach-Object {
      "  {0} {1}% {2} reset={3}s" -f $_.label, $_.used_percent, $_.kind, $_.resets_in_seconds
    }
  }
```

Optional direct probes (requires local tokens; do not commit output):

```powershell
# Grok weekly + monthly via cli-chat-proxy
# Claude: api.anthropic.com/api/oauth/usage
# Codex: chatgpt.com/backend-api/wham/usage
```

---

## Related source

| File | Role |
|------|------|
| `src/usage.rs` | Fetch, parse, cache, window model |
| `src/server.rs` | `GET /api/usage` |
| `static/index.html` | Meter cards, timers, 60s poll |

Open follow-ups live in [`../TODO.md`](../TODO.md) under **Usage meters**.
