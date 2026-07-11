//! Live usage / quota meters for Claude, Grok, and Codex.
//!
//! Human-facing reference: `docs/USAGE.md`. Backlog: root `TODO.md`.
//!
//! Researched against installed CLIs (2026-07-11):
//! - Claude Code 2.1.137 → GET api.anthropic.com/api/oauth/usage
//!   Windows: session/5h + weekly (+ scoped model weeks). `limits[]` carries
//!   severity (normal/warn/…) and is_active. Session is the soft burst gate;
//!   weekly is the harder plan pool. optional extra_usage/spend monthly.
//! - Grok 0.2.93 → cli-chat-proxy.grok.com/v1/billing
//!   `?format=credits` = weekly soft credit pool (GrokBuild product %).
//!   default/`?format=full` = monthly hard spend (monthlyLimit/used).
//!   onDemand = paid overage after included credits.
//! - Codex CLI 0.144.1 → chatgpt.com/backend-api/wham/usage
//!   primary_window (limit_window_seconds≈18000 → 5h soft rate)
//!   secondary_window (≈604800 → weekly hard pool). Both gate in parallel.
//!   limit_reached = hard stop. credits = flexible overage.

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tracing::{debug, warn};

/// Per-provider success cache + 429 backoff.
/// Anthropic's `/api/oauth/usage` is aggressively rate-limited; Claude Code
/// itself caches utilization until `expiresAt` and retries carefully.
struct MeterCacheEntry {
    meter: UsageMeter,
    fetched_at: Instant,
    /// Do not hit remote again until this instant (429 backoff / min TTL).
    quiet_until: Instant,
    from_remote: bool,
}

fn meter_cache() -> &'static Mutex<HashMap<String, MeterCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, MeterCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Minimum time between remote fetches per provider (seconds).
fn provider_min_ttl_secs(id: &str) -> u64 {
    match id {
        // Claude usage endpoint is shared/rate-limited; poll sparingly.
        "claude" => 120,
        "grok" => 60,
        "codex" => 60,
        _ => 60,
    }
}

fn cache_get_fresh(id: &str) -> Option<UsageMeter> {
    let guard = meter_cache().lock().ok()?;
    let e = guard.get(id)?;
    if Instant::now() < e.quiet_until && e.from_remote && e.meter.error.is_none() {
        let mut m = e.meter.clone();
        // Refresh relative reset countdowns from absolute timestamps.
        refresh_window_countdowns(&mut m);
        if m.detail.as_deref().map(|d| !d.contains("cached")).unwrap_or(true) {
            // leave detail; UI uses windows
        }
        return Some(m);
    }
    // Also serve during quiet backoff even if last had a soft note
    if Instant::now() < e.quiet_until && !e.meter.windows.is_empty() {
        let mut m = e.meter.clone();
        refresh_window_countdowns(&mut m);
        return Some(m);
    }
    None
}

fn cache_get_stale(id: &str) -> Option<UsageMeter> {
    let guard = meter_cache().lock().ok()?;
    let e = guard.get(id)?;
    if e.meter.windows.is_empty() && e.meter.used_percent.is_none() {
        return None;
    }
    let age = e.fetched_at.elapsed().as_secs();
    let mut m = e.meter.clone();
    refresh_window_countdowns(&mut m);
    let note = format!("cached {age}s ago");
    m.detail = Some(match m.detail.take() {
        Some(d) if !d.contains("cached") => format!("{d} · {note}"),
        Some(d) => d,
        None => note,
    });
    // Clear hard error so UI stays online with stale bars
    m.error = None;
    Some(m)
}

fn cache_put(id: &str, meter: &UsageMeter, quiet_extra: Duration) {
    let ttl = Duration::from_secs(provider_min_ttl_secs(id));
    let quiet = ttl.max(quiet_extra);
    if let Ok(mut guard) = meter_cache().lock() {
        guard.insert(
            id.to_string(),
            MeterCacheEntry {
                meter: meter.clone(),
                fetched_at: Instant::now(),
                quiet_until: Instant::now() + quiet,
                from_remote: meter.error.is_none() && (!meter.windows.is_empty() || meter.used_percent.is_some()),
            },
        );
    }
}

fn cache_backoff(id: &str, secs: u64) {
    if let Ok(mut guard) = meter_cache().lock() {
        if let Some(e) = guard.get_mut(id) {
            e.quiet_until = Instant::now() + Duration::from_secs(secs);
        } else {
            // Placeholder so we don't hammer while empty
            guard.insert(
                id.to_string(),
                MeterCacheEntry {
                    meter: UsageMeter {
                        id: id.into(),
                        label: id.into(),
                        available: false,
                        logged_in: false,
                        plan: None,
                        account: None,
                        used_percent: None,
                        remaining_percent: None,
                        primary_label: None,
                        secondary_label: None,
                        resets_at: None,
                        resets_in_seconds: None,
                        detail: None,
                        error: None,
                        source: None,
                        fetched_at: Utc::now(),
                        windows: vec![],
                    },
                    fetched_at: Instant::now(),
                    quiet_until: Instant::now() + Duration::from_secs(secs),
                    from_remote: false,
                },
            );
        }
    }
}

fn refresh_window_countdowns(meter: &mut UsageMeter) {
    for w in &mut meter.windows {
        if let Some(r) = w.resets_at {
            let secs = seconds_until(&r);
            w.resets_in_seconds = Some(secs);
        }
    }
    if let Some(r) = meter.resets_at {
        meter.resets_in_seconds = Some(seconds_until(&r));
    }
}

#[derive(Debug)]
struct HttpError {
    status: Option<u16>,
    message: String,
    retry_after_secs: Option<u64>,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

fn is_rate_limited(err: &HttpError) -> bool {
    err.status == Some(429)
        || err.message.contains("429")
        || err.message.to_ascii_lowercase().contains("rate limit")
        || err.message.to_ascii_lowercase().contains("rate_limit")
}

/// One usage window (5h / weekly / monthly / product / spend).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageWindow {
    pub id: String,
    pub label: String,
    /// 0..100 used, if known
    pub used_percent: Option<f64>,
    pub remaining_percent: Option<f64>,
    /// soft | hard | rate | spend | credits | product
    pub kind: Option<String>,
    /// normal | warn | hot | hard (UI heat)
    pub severity: Option<String>,
    pub resets_at: Option<DateTime<Utc>>,
    pub resets_in_seconds: Option<i64>,
    /// Extra human text e.g. "$2,756 / $15,000"
    pub detail: Option<String>,
    pub is_active: Option<bool>,
    /// True when this window is a hard stop if exhausted
    pub is_hard_limit: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageMeter {
    pub id: String,
    pub label: String,
    pub available: bool,
    pub logged_in: bool,
    pub plan: Option<String>,
    pub account: Option<String>,
    /// Hottest / most constraining window (for the main bar)
    pub used_percent: Option<f64>,
    pub remaining_percent: Option<f64>,
    pub primary_label: Option<String>,
    pub secondary_label: Option<String>,
    pub resets_at: Option<DateTime<Utc>>,
    pub resets_in_seconds: Option<i64>,
    pub detail: Option<String>,
    pub error: Option<String>,
    pub source: Option<String>,
    pub fetched_at: DateTime<Utc>,
    /// All known windows (5h, weekly, monthly, …)
    #[serde(default)]
    pub windows: Vec<UsageWindow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub fetched_at: DateTime<Utc>,
    pub meters: Vec<UsageMeter>,
}

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn http_get_json(url: &str, headers: &[(&str, &str)]) -> Result<Value, HttpError> {
    let mut req = ureq::get(url);
    for (k, v) in headers {
        req = req.set(k, v);
    }
    let resp = match req.timeout(Duration::from_secs(12)).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, resp)) => {
            let retry_after = resp
                .header("retry-after")
                .and_then(|s| s.parse::<u64>().ok());
            let body = resp.into_string().unwrap_or_default();
            let snippet: String = body.chars().take(120).collect();
            let message = if code == 429 {
                "rate limited (429)".into()
            } else {
                format!("HTTP {code}: {snippet}")
            };
            return Err(HttpError {
                status: Some(code),
                message,
                retry_after_secs: retry_after,
            });
        }
        Err(e) => {
            return Err(HttpError {
                status: None,
                message: format!("http error: {e}"),
                retry_after_secs: None,
            });
        }
    };
    let status = resp.status();
    let body = resp.into_string().map_err(|e| HttpError {
        status: Some(status),
        message: format!("read body: {e}"),
        retry_after_secs: None,
    })?;
    if !(200..300).contains(&status) {
        let snippet: String = body.chars().take(120).collect();
        return Err(HttpError {
            status: Some(status),
            message: if status == 429 {
                "rate limited (429)".into()
            } else {
                format!("HTTP {status}: {snippet}")
            },
            retry_after_secs: None,
        });
    }
    serde_json::from_str(&body).map_err(|e| HttpError {
        status: Some(status),
        message: format!("json: {e}"),
        retry_after_secs: None,
    })
}

fn seconds_until(ts: &DateTime<Utc>) -> i64 {
    (*ts - Utc::now()).num_seconds().max(0)
}

fn parse_rfc3339(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

fn fmt_duration(secs: i64) -> String {
    if secs <= 0 {
        return "now".into();
    }
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    if h >= 48 {
        format!("{}d {}h", h / 24, h % 24)
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

fn severity_from_percent(pct: f64, is_hard: bool) -> String {
    if pct >= 100.0 && is_hard {
        "hard".into()
    } else if pct >= 85.0 {
        "hot".into()
    } else if pct >= 60.0 {
        "warn".into()
    } else {
        "normal".into()
    }
}

fn map_api_severity(s: &str) -> String {
    match s.to_ascii_lowercase().as_str() {
        "critical" | "hard" | "exhausted" | "blocked" => "hard".into(),
        "high" | "hot" | "severe" => "hot".into(),
        "warn" | "warning" | "elevated" | "approaching" => "warn".into(),
        other if !other.is_empty() => other.into(),
        _ => "normal".into(),
    }
}

fn window_seconds_label(secs: i64) -> String {
    match secs {
        s if (17_000..=19_000).contains(&s) => "5h".into(),
        s if (86_000..=90_000).contains(&s) => "24h".into(),
        s if (600_000..=650_000).contains(&s) => "week".into(),
        s if (2_400_000..=2_700_000).contains(&s) => "month".into(),
        s if s >= 3600 => format!("{}h", s / 3600),
        s if s >= 60 => format!("{}m", s / 60),
        s => format!("{s}s"),
    }
}

fn make_window(
    id: &str,
    label: &str,
    used: Option<f64>,
    kind: &str,
    is_hard: bool,
    resets: Option<DateTime<Utc>>,
    detail: Option<String>,
    is_active: Option<bool>,
    api_severity: Option<&str>,
) -> UsageWindow {
    let used_c = used.map(|u| u.clamp(0.0, 100.0));
    let sev = if let Some(s) = api_severity {
        map_api_severity(s)
    } else if let Some(u) = used_c {
        severity_from_percent(u, is_hard)
    } else {
        "normal".into()
    };
    let (resets_at, resets_in_seconds) = match resets {
        Some(r) => {
            let secs = seconds_until(&r);
            (Some(r), Some(secs))
        }
        None => (None, None),
    };
    UsageWindow {
        id: id.into(),
        label: label.into(),
        used_percent: used_c,
        remaining_percent: used_c.map(|u| (100.0 - u).clamp(0.0, 100.0)),
        kind: Some(kind.into()),
        severity: Some(sev),
        resets_at,
        resets_in_seconds,
        detail,
        is_active,
        is_hard_limit: Some(is_hard),
    }
}

/// Pick primary meter fields from windows: hottest used %, prefer active/hard.
fn apply_windows_to_meter(meter: &mut UsageMeter) {
    if meter.windows.is_empty() {
        return;
    }

    // Most constraining = highest used% among windows with a reading.
    // Prefer hard windows when tied.
    let mut best: Option<(f64, usize)> = None;
    for (i, w) in meter.windows.iter().enumerate() {
        let Some(u) = w.used_percent else { continue };
        let hard_boost = if w.is_hard_limit == Some(true) {
            0.01
        } else {
            0.0
        };
        let score = u + hard_boost;
        match best {
            None => best = Some((score, i)),
            Some((s, _)) if score > s => best = Some((score, i)),
            _ => {}
        }
    }

    // Prefer the most-used active window when one is marked active with usage.
    let active_best = meter
        .windows
        .iter()
        .enumerate()
        .filter(|(_, w)| w.is_active == Some(true) && w.used_percent.is_some())
        .max_by(|a, b| {
            a.1.used_percent
                .partial_cmp(&b.1.used_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    if let Some((i, w)) = active_best {
        if let Some(u) = w.used_percent {
            if u > 0.0 || best.is_none() {
                best = Some((u, i));
            }
        }
    }

    if let Some((_, idx)) = best {
        let w = &meter.windows[idx];
        meter.used_percent = w.used_percent;
        meter.remaining_percent = w.remaining_percent;
        meter.resets_at = w.resets_at;
        meter.resets_in_seconds = w.resets_in_seconds;
    }

    // Labels: primary = top 2 windows summary
    let mut parts: Vec<String> = Vec::new();
    for w in &meter.windows {
        let mut bit = w.label.clone();
        if let Some(u) = w.used_percent {
            bit.push_str(&format!(" {u:.0}%"));
        }
        if let Some(k) = &w.kind {
            if k == "soft" || k == "hard" {
                bit.push(' ');
                bit.push_str(k);
            }
        }
        if let Some(secs) = w.resets_in_seconds {
            if secs > 0 && secs < 7 * 24 * 3600 {
                bit.push_str(&format!(" · {}", fmt_duration(secs)));
            }
        }
        parts.push(bit);
    }
    if !parts.is_empty() {
        meter.primary_label = Some(parts[0].clone());
        if parts.len() > 1 {
            meter.secondary_label = Some(parts[1..].join(" · "));
        }
    }
}

// ---------------------------------------------------------------------------
// Claude Code 2.1.137
// ---------------------------------------------------------------------------

fn fetch_claude() -> UsageMeter {
    // Serve in-process cache (Claude Code also caches utilization).
    if let Some(m) = cache_get_fresh("claude") {
        debug!("claude usage: serving cache (min TTL / backoff)");
        return m;
    }

    let mut meter = UsageMeter {
        id: "claude".into(),
        label: "Claude".into(),
        available: false,
        logged_in: false,
        plan: None,
        account: None,
        used_percent: None,
        remaining_percent: None,
        primary_label: None,
        secondary_label: None,
        resets_at: None,
        resets_in_seconds: None,
        detail: None,
        error: None,
        source: Some("api.anthropic.com/api/oauth/usage".into()),
        fetched_at: Utc::now(),
        windows: Vec::new(),
    };

    let path = home().join(".claude").join(".credentials.json");
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            meter.error = Some(format!("no credentials ({e})"));
            return meter;
        }
    };
    let cred: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            meter.error = Some(format!("bad credentials json: {e}"));
            return meter;
        }
    };
    let oauth = &cred["claudeAiOauth"];
    let token = oauth["accessToken"].as_str().unwrap_or("");
    if token.is_empty() {
        meter.error = Some("not logged in".into());
        return meter;
    }
    meter.available = true;
    meter.logged_in = true;
    meter.plan = oauth["subscriptionType"]
        .as_str()
        .map(|s| s.to_string());
    // Prefer local credential fields — avoid spawning `claude auth status` every poll
    // (that was extra load and not needed for the usage bars).
    if let Some(email) = oauth["email"].as_str().or_else(|| oauth["account"].as_str()) {
        meter.account = Some(email.to_string());
    }

    let authz = format!("Bearer {token}");
    // Headers aligned with Claude Code's fetchUtilization (Content-Type + UA + OAuth).
    match http_get_json(
        "https://api.anthropic.com/api/oauth/usage",
        &[
            ("Authorization", authz.as_str()),
            ("Content-Type", "application/json"),
            ("anthropic-version", "2023-06-01"),
            ("User-Agent", "claude-code/2.1.137"),
            ("Accept", "application/json"),
        ],
    ) {
        Ok(v) => {
            // Prefer structured limits[] (Claude Code 2.x).
            let mut saw_limits = false;
            if let Some(arr) = v["limits"].as_array() {
                for lim in arr {
                    let kind = lim["kind"].as_str().unwrap_or("limit");
                    let group = lim["group"].as_str().unwrap_or("");
                    let pct = lim["percent"]
                        .as_f64()
                        .or_else(|| lim["percent"].as_i64().map(|i| i as f64));
                    let scope_name = lim
                        .pointer("/scope/model/display_name")
                        .and_then(|x| x.as_str())
                        .or_else(|| lim.pointer("/scope/model/id").and_then(|x| x.as_str()));

                    // Skip scoped models at 0% with no reset (noise)
                    if kind == "weekly_scoped"
                        && pct.unwrap_or(0.0) == 0.0
                        && lim["resets_at"].is_null()
                        && lim["is_active"] != true
                    {
                        continue;
                    }

                    let (id, label, is_hard, win_kind): (String, String, bool, &str) =
                        match (kind, group) {
                            ("session", _) | (_, "session") => {
                                ("5h".into(), "5h session".into(), false, "soft")
                            }
                            ("weekly_all", _) | ("weekly", _) => {
                                ("weekly".into(), "weekly".into(), true, "hard")
                            }
                            ("weekly_scoped", _) => {
                                let name = scope_name.unwrap_or("scoped");
                                (
                                    format!("weekly_{}", name.to_ascii_lowercase()),
                                    format!("week {name}"),
                                    true,
                                    "hard",
                                )
                            }
                            ("monthly", _) | (_, "monthly") => {
                                ("monthly".into(), "monthly".into(), true, "hard")
                            }
                            (k, _) => (k.into(), k.into(), false, "rate"),
                        };

                    let resets = lim["resets_at"].as_str().and_then(parse_rfc3339);
                    let sev = lim["severity"].as_str();
                    let active = lim["is_active"].as_bool();
                    meter.windows.push(make_window(
                        &id, &label, pct, win_kind, is_hard, resets, None, active, sev,
                    ));
                    saw_limits = true;
                }
            }

            // Fallback / supplement from classic five_hour + seven_day fields
            if !saw_limits || meter.windows.iter().all(|w| w.id != "5h") {
                let five = &v["five_hour"];
                if !five.is_null() {
                    let used = five["utilization"].as_f64();
                    let resets = five["resets_at"].as_str().and_then(parse_rfc3339);
                    let detail = dollars_detail(five);
                    meter.windows.push(make_window(
                        "5h",
                        "5h session",
                        used,
                        "soft",
                        false,
                        resets,
                        detail,
                        Some(true),
                        None,
                    ));
                }
            }
            if !saw_limits || meter.windows.iter().all(|w| w.id != "weekly") {
                let week = &v["seven_day"];
                if !week.is_null() {
                    let used = week["utilization"].as_f64();
                    let resets = week["resets_at"].as_str().and_then(parse_rfc3339);
                    let detail = dollars_detail(week);
                    meter.windows.push(make_window(
                        "weekly",
                        "weekly",
                        used,
                        "hard",
                        true,
                        resets,
                        detail,
                        None,
                        None,
                    ));
                }
            }

            // Model-specific weekly buckets (sonnet/opus/…)
            for (key, label) in [
                ("seven_day_sonnet", "week sonnet"),
                ("seven_day_opus", "week opus"),
                ("seven_day_oauth_apps", "week oauth"),
                ("seven_day_cowork", "week cowork"),
            ] {
                let node = &v[key];
                if node.is_null() {
                    continue;
                }
                if let Some(u) = node["utilization"].as_f64() {
                    if u <= 0.0 && node["resets_at"].is_null() {
                        continue;
                    }
                    let resets = node["resets_at"].as_str().and_then(parse_rfc3339);
                    meter.windows.push(make_window(
                        key,
                        label,
                        Some(u),
                        "hard",
                        true,
                        resets,
                        dollars_detail(node),
                        None,
                        None,
                    ));
                }
            }

            // Extra usage / monthly credits (soft overage after plan limits)
            let extra = &v["extra_usage"];
            if extra["is_enabled"].as_bool() == Some(true) {
                let used = extra["utilization"].as_f64();
                let mut detail = None;
                if let (Some(lim), Some(used_c)) = (
                    extra["monthly_limit"].as_f64(),
                    extra["used_credits"].as_f64(),
                ) {
                    detail = Some(format!("{used_c:.0} / {lim:.0} credits"));
                }
                // daily / weekly sub-buckets under extra_usage
                meter.windows.push(make_window(
                    "extra_monthly",
                    "extra monthly",
                    used,
                    "credits",
                    false,
                    None,
                    detail,
                    Some(true),
                    None,
                ));
                for (sub, label) in [("daily", "extra daily"), ("weekly", "extra weekly")] {
                    let n = &extra[sub];
                    if n.is_null() {
                        continue;
                    }
                    if let Some(u) = n["utilization"].as_f64() {
                        meter.windows.push(make_window(
                            &format!("extra_{sub}"),
                            label,
                            Some(u),
                            "credits",
                            false,
                            n["resets_at"].as_str().and_then(parse_rfc3339),
                            None,
                            None,
                            None,
                        ));
                    }
                }
            }

            // Spend (hard dollar cap when enabled)
            let spend = &v["spend"];
            if spend["enabled"].as_bool() == Some(true) {
                let pct = spend["percent"].as_f64();
                let mut detail = None;
                if let Some(used) = spend.pointer("/used/amount_minor").and_then(|x| x.as_i64()) {
                    let exp = spend
                        .pointer("/used/exponent")
                        .and_then(|x| x.as_i64())
                        .unwrap_or(2) as u32;
                    let used_f = used as f64 / 10f64.powi(exp as i32);
                    if let Some(lim) = spend["limit"]
                        .as_f64()
                        .or_else(|| spend.pointer("/limit/amount_minor").and_then(|x| {
                            let e = spend
                                .pointer("/limit/exponent")
                                .and_then(|y| y.as_i64())
                                .unwrap_or(2) as u32;
                            x.as_i64().map(|m| m as f64 / 10f64.powi(e as i32))
                        }))
                    {
                        detail = Some(format!("${used_f:.2} / ${lim:.2}"));
                    } else {
                        detail = Some(format!("${used_f:.2} spent"));
                    }
                }
                let sev = spend["severity"].as_str();
                meter.windows.push(make_window(
                    "spend",
                    "spend",
                    pct,
                    "spend",
                    true,
                    None,
                    detail,
                    None,
                    sev,
                ));
            }

            apply_windows_to_meter(&mut meter);
            meter.detail = meter.plan.clone();
            cache_put("claude", &meter, Duration::from_secs(0));
        }
        Err(e) => {
            warn!(error = %e, "claude usage fetch failed");
            if is_rate_limited(&e) {
                let backoff = e.retry_after_secs.unwrap_or(300).clamp(60, 900);
                cache_backoff("claude", backoff);
                if let Some(mut stale) = cache_get_stale("claude") {
                    stale.detail = Some(format!(
                        "rate limited · retry ~{}s · {}",
                        backoff,
                        stale.detail.unwrap_or_default()
                    ));
                    // keep windows; soft note only
                    stale.error = None;
                    return stale;
                }
                meter.error = Some(format!("rate limited — retry in ~{backoff}s"));
                meter.primary_label = Some("rate limited".into());
                meter.detail = Some("Anthropic usage API 429 (cached empty)".into());
            } else if let Some(stale) = cache_get_stale("claude") {
                let mut stale = stale;
                stale.detail = Some(format!("offline · {e} · {}", stale.detail.unwrap_or_default()));
                return stale;
            } else {
                meter.error = Some(e.to_string());
            }
        }
    }
    meter
}

fn dollars_detail(node: &Value) -> Option<String> {
    let used = node["used_dollars"].as_f64()?;
    let limit = node["limit_dollars"].as_f64();
    match limit {
        Some(l) if l > 0.0 => Some(format!("${used:.2} / ${l:.2}")),
        _ => Some(format!("${used:.2}")),
    }
}

// ---------------------------------------------------------------------------
// Codex CLI 0.144.1
// ---------------------------------------------------------------------------

fn fetch_codex() -> UsageMeter {
    if let Some(m) = cache_get_fresh("codex") {
        return m;
    }

    let mut meter = UsageMeter {
        id: "codex".into(),
        label: "Codex".into(),
        available: false,
        logged_in: false,
        plan: None,
        account: None,
        used_percent: None,
        remaining_percent: None,
        primary_label: None,
        secondary_label: None,
        resets_at: None,
        resets_in_seconds: None,
        detail: None,
        error: None,
        source: Some("chatgpt.com/backend-api/wham/usage".into()),
        fetched_at: Utc::now(),
        windows: Vec::new(),
    };

    let path = home().join(".codex").join("auth.json");
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            meter.error = Some(format!("no credentials ({e})"));
            return meter;
        }
    };
    let auth: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            meter.error = Some(format!("bad auth json: {e}"));
            return meter;
        }
    };
    let token = auth["tokens"]["access_token"].as_str().unwrap_or("");
    let account_id = auth["tokens"]["account_id"].as_str().unwrap_or("");
    if token.is_empty() {
        meter.error = Some("not logged in".into());
        return meter;
    }
    meter.available = true;
    meter.logged_in = true;

    let mut headers = vec![
        ("Authorization", format!("Bearer {token}")),
        ("Accept", "application/json".into()),
        ("User-Agent", "agent-manager".into()),
    ];
    if !account_id.is_empty() {
        headers.push(("ChatGPT-Account-Id", account_id.to_string()));
    }
    let header_refs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(k, v)| (*k, v.as_str()))
        .collect();

    match http_get_json("https://chatgpt.com/backend-api/wham/usage", &header_refs) {
        Ok(v) => {
            meter.plan = v["plan_type"].as_str().map(|s| s.to_string());
            meter.account = v["email"].as_str().map(|s| s.to_string());

            let limit_reached = v
                .pointer("/rate_limit/limit_reached")
                .and_then(|x| x.as_bool())
                .unwrap_or(false);
            let allowed = v
                .pointer("/rate_limit/allowed")
                .and_then(|x| x.as_bool())
                .unwrap_or(true);

            // primary = 5h soft rate window, secondary = weekly hard pool
            // (both apply in parallel — Codex docs / wham shape)
            if let Some(rl) = v.get("rate_limit").filter(|x| !x.is_null()) {
                for (key, default_label, is_hard, kind) in [
                    ("primary_window", "5h", false, "soft"),
                    ("secondary_window", "weekly", true, "hard"),
                ] {
                    let w = &rl[key];
                    if w.is_null() {
                        continue;
                    }
                    let mut used = w["used_percent"].as_f64();
                    if let Some(u) = used {
                        // some payloads use 0..1 fraction
                        if u <= 1.0 && w["used_percent"].as_i64().is_none() {
                            // Heuristic: if it's a float 0..1 treat as fraction only when < 1
                            // But used_percent: 1 means 1% as integer in our live payload.
                            // Live sample has used_percent: 1 (integer 1%). Keep as-is.
                            let _ = u;
                        }
                        // If value is between 0 and 1 exclusive and not integer JSON, *100
                        if let Some(f) = w["used_percent"].as_f64() {
                            if f > 0.0 && f < 1.0 && w["used_percent"].to_string().contains('.') {
                                used = Some(f * 100.0);
                            }
                        }
                    }
                    // percent_left fallback
                    if used.is_none() {
                        if let Some(left) = w["percent_left"].as_f64() {
                            used = Some((100.0 - left).clamp(0.0, 100.0));
                        }
                    }

                    let win_secs = w["limit_window_seconds"].as_i64().unwrap_or(0);
                    let label = if win_secs > 0 {
                        window_seconds_label(win_secs)
                    } else {
                        default_label.to_string()
                    };

                    let resets = w["reset_at"]
                        .as_i64()
                        .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
                        .or_else(|| {
                            w["reset_after_seconds"].as_i64().map(|secs| {
                                Utc::now() + chrono::Duration::seconds(secs)
                            })
                        });

                    let mut hard = is_hard;
                    // If overall limit_reached and this window is at 100%, hard
                    if limit_reached {
                        if used.unwrap_or(0.0) >= 99.0 {
                            hard = true;
                        }
                    }

                    meter.windows.push(make_window(
                        if key == "primary_window" {
                            "5h"
                        } else {
                            "weekly"
                        },
                        &label,
                        used,
                        kind,
                        hard,
                        resets,
                        None,
                        Some(key == "primary_window"),
                        if limit_reached && hard {
                            Some("hard")
                        } else {
                            None
                        },
                    ));
                }
            }

            // Credits / flexible overage (soft bridge after plan limits)
            if let Some(cr) = v.get("credits").filter(|x| !x.is_null()) {
                if cr["has_credits"].as_bool() == Some(true)
                    || cr["unlimited"].as_bool() == Some(true)
                {
                    let bal = cr["balance"]
                        .as_str()
                        .and_then(|s| s.parse::<f64>().ok())
                        .or_else(|| cr["balance"].as_f64())
                        .unwrap_or(0.0);
                    let over = cr["overage_limit_reached"].as_bool().unwrap_or(false);
                    let detail = if cr["unlimited"].as_bool() == Some(true) {
                        Some("unlimited credits".into())
                    } else {
                        Some(format!("balance {bal:.0}"))
                    };
                    meter.windows.push(make_window(
                        "credits",
                        "credits",
                        if over { Some(100.0) } else { None },
                        "credits",
                        over,
                        None,
                        detail,
                        None,
                        if over { Some("hard") } else { None },
                    ));
                }
            }

            // Spend control — only when an individual_limit is actually present
            if let Some(ind) = v
                .pointer("/spend_control/individual_limit")
                .filter(|x| !x.is_null())
            {
                let used_pct = ind["used_percent"].as_f64();
                let used_f = ind["used"]
                    .as_str()
                    .and_then(|s| s.parse::<f64>().ok())
                    .or_else(|| ind["used"].as_f64());
                let limit_f = ind["limit"]
                    .as_str()
                    .and_then(|s| s.parse::<f64>().ok())
                    .or_else(|| ind["limit"].as_f64());
                let detail = match (used_f, limit_f) {
                    (Some(u), Some(l)) if l > 0.0 => Some(format!("${u:.0} / ${l:.0}")),
                    _ => None,
                };
                let resets = ind["reset_after_seconds"].as_i64().map(|secs| {
                    Utc::now() + chrono::Duration::seconds(secs)
                }).or_else(|| {
                    ind["reset_at"]
                        .as_i64()
                        .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
                });
                meter.windows.push(make_window(
                    "spend",
                    "spend",
                    used_pct,
                    "spend",
                    true,
                    resets,
                    detail,
                    None,
                    None,
                ));
            }

            // Free rate-limit reset credits (Plus/Pro banking)
            if let Some(n) = v
                .pointer("/rate_limit_reset_credits/available_count")
                .and_then(|x| x.as_i64())
            {
                if n > 0 {
                    meter.detail = Some(format!("{n} rate-limit resets banked"));
                }
            }

            if !allowed || limit_reached {
                if meter.detail.is_none() {
                    meter.detail = Some("rate limit reached".into());
                }
            }

            apply_windows_to_meter(&mut meter);
            if meter.detail.is_none() {
                meter.detail = meter.plan.clone();
            }
            cache_put("codex", &meter, Duration::from_secs(0));
        }
        Err(e) => {
            warn!(error = %e, "codex usage fetch failed");
            if is_rate_limited(&e) {
                let backoff = e.retry_after_secs.unwrap_or(120).clamp(30, 600);
                cache_backoff("codex", backoff);
            }
            if let Some(stale) = cache_get_stale("codex") {
                return stale;
            }
            meter.error = Some(e.to_string());
        }
    }
    meter
}

// ---------------------------------------------------------------------------
// Grok 0.2.93 — weekly soft credits + monthly hard spend
// ---------------------------------------------------------------------------

fn fetch_grok() -> UsageMeter {
    if let Some(m) = cache_get_fresh("grok") {
        return m;
    }

    let mut meter = UsageMeter {
        id: "grok".into(),
        label: "Grok".into(),
        available: false,
        logged_in: false,
        plan: None,
        account: None,
        used_percent: None,
        remaining_percent: None,
        primary_label: None,
        secondary_label: None,
        resets_at: None,
        resets_in_seconds: None,
        detail: None,
        error: None,
        source: Some(
            "cli-chat-proxy.grok.com/v1/billing (+?format=credits)".into(),
        ),
        fetched_at: Utc::now(),
        windows: Vec::new(),
    };

    let path = home().join(".grok").join("auth.json");
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            meter.error = Some(format!("no credentials ({e})"));
            return meter;
        }
    };
    let auth: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            meter.error = Some(format!("bad auth json: {e}"));
            return meter;
        }
    };

    let entry = auth
        .as_object()
        .and_then(|m| m.values().next())
        .cloned()
        .unwrap_or(Value::Null);

    let token = entry["key"].as_str().unwrap_or("");
    if token.is_empty() {
        meter.error = Some("not logged in".into());
        return meter;
    }
    meter.available = true;
    meter.logged_in = true;
    meter.account = entry["email"].as_str().map(|s| s.to_string());
    meter.plan = entry["principal_type"].as_str().map(|s| s.to_string());

    let authz = format!("Bearer {token}");
    let user_id = entry["user_id"].as_str().unwrap_or("");
    let mut headers: Vec<(&str, &str)> = vec![
        ("Authorization", authz.as_str()),
        ("Accept", "application/json"),
        ("X-XAI-Token-Auth", "xai-grok-cli"),
        ("x-grok-client-version", "0.2.93"),
    ];
    if !user_id.is_empty() {
        headers.push(("x-userid", user_id));
    }

    // Subscription tier from /user (GrokPro, etc.)
    if let Ok(user) = http_get_json(
        "https://cli-chat-proxy.grok.com/v1/user?include=subscription",
        &headers,
    ) {
        if let Some(tier) = user["subscriptionTier"]
            .as_str()
            .or_else(|| user["subscription_tier"].as_str())
        {
            meter.plan = Some(tier.to_string());
        }
        if let Some(email) = user["email"].as_str() {
            meter.account = Some(email.to_string());
        }
    }

    // --- Weekly soft credit pool (same as TUI /usage show) ---
    let mut credits_err: Option<HttpError> = None;
    match http_get_json(
        "https://cli-chat-proxy.grok.com/v1/billing?format=credits",
        &headers,
    ) {
        Ok(v) => {
            let cfg = &v["config"];
            let period = &cfg["currentPeriod"];
            let period_type = period["type"]
                .as_str()
                .unwrap_or("USAGE_PERIOD_TYPE_WEEKLY")
                .trim_start_matches("USAGE_PERIOD_TYPE_")
                .to_ascii_lowercase();
            let end = period["end"]
                .as_str()
                .or_else(|| cfg["billingPeriodEnd"].as_str())
                .and_then(parse_rfc3339);

            // Overall weekly credit %
            let overall = cfg["creditUsagePercent"].as_f64();

            // Per-product windows (GrokBuild is the coding agent pool)
            let mut build_pct = None;
            if let Some(arr) = cfg["productUsage"].as_array() {
                for p in arr {
                    let name = p["product"].as_str().unwrap_or("?");
                    let pct = p["usagePercent"].as_f64();
                    if name.eq_ignore_ascii_case("GrokBuild") {
                        build_pct = pct;
                    }
                    // Only add product rows that have a reading, or always for Build
                    if pct.is_some() || name.eq_ignore_ascii_case("GrokBuild") {
                        let label = match name {
                            "GrokBuild" => "week build".to_string(),
                            "GrokChat" => "week chat".to_string(),
                            other => format!("week {other}"),
                        };
                        let used = pct.or(if name.eq_ignore_ascii_case("GrokBuild") {
                            overall
                        } else {
                            None
                        });
                        // Weekly product pool is a soft included-credits gate;
                        // hard stop is monthly / on-demand exhaustion.
                        meter.windows.push(make_window(
                            &format!("week_{}", name.to_ascii_lowercase()),
                            &label,
                            used,
                            "soft",
                            false,
                            end,
                            None,
                            Some(name.eq_ignore_ascii_case("GrokBuild")),
                            None,
                        ));
                    }
                }
            }

            // If no product breakdown, still show weekly credits
            if meter.windows.is_empty() {
                if let Some(u) = overall.or(build_pct) {
                    let label = if period_type.is_empty() {
                        "weekly credits".into()
                    } else {
                        format!("{period_type} credits")
                    };
                    meter.windows.push(make_window(
                        "weekly",
                        &label,
                        Some(u),
                        "soft",
                        false,
                        end,
                        None,
                        Some(true),
                        None,
                    ));
                }
            }
        }
        Err(e) => {
            warn!(error = %e, "grok credits billing fetch failed");
            credits_err = Some(e);
        }
    }

    // --- Monthly hard spend limit (default billing format) ---
    let mut monthly_err: Option<HttpError> = None;
    match http_get_json("https://cli-chat-proxy.grok.com/v1/billing", &headers) {
        Ok(v) => {
            let cfg = &v["config"];
            let used = cfg["used"]["val"]
                .as_f64()
                .or_else(|| cfg["used"].as_f64());
            let limit = cfg["monthlyLimit"]["val"]
                .as_f64()
                .or_else(|| cfg["monthlyLimit"].as_f64())
                .or_else(|| cfg["maxAmountPerMonth"]["val"].as_f64())
                .or_else(|| cfg["maxAmountPerMonth"].as_f64());

            let end = cfg["billingPeriodEnd"]
                .as_str()
                .and_then(parse_rfc3339);

            if let (Some(u), Some(l)) = (used, limit) {
                if l > 0.0 {
                    let pct = (u / l * 100.0).clamp(0.0, 100.0);
                    meter.windows.push(make_window(
                        "monthly",
                        "monthly",
                        Some(pct),
                        "hard",
                        true,
                        end,
                        Some(format!("${u:.0} / ${l:.0}")),
                        None,
                        None,
                    ));
                }
            } else if let Some(u) = used {
                meter.windows.push(make_window(
                    "monthly",
                    "monthly",
                    None,
                    "hard",
                    true,
                    end,
                    Some(format!("${u:.0} used")),
                    None,
                    None,
                ));
            }

            // On-demand (paid overage) — hard once cap hit
            let od_used = cfg["onDemandUsed"]["val"]
                .as_f64()
                .or_else(|| cfg["onDemandUsed"].as_f64());
            let od_cap = cfg["onDemandCap"]["val"]
                .as_f64()
                .or_else(|| cfg["onDemandCap"].as_f64());
            if let (Some(u), Some(c)) = (od_used, od_cap) {
                if c > 0.0 || u > 0.0 {
                    let pct = if c > 0.0 {
                        Some((u / c * 100.0).clamp(0.0, 100.0))
                    } else if u > 0.0 {
                        Some(100.0)
                    } else {
                        None
                    };
                    meter.windows.push(make_window(
                        "on_demand",
                        "on-demand",
                        pct,
                        "hard",
                        true,
                        None,
                        Some(format!("${u:.0} / ${c:.0}")),
                        None,
                        None,
                    ));
                }
            }

            // Prepaid balance note
            if let Some(bal) = cfg["prepaidBalance"]["val"]
                .as_f64()
                .or_else(|| cfg["prepaidBalance"].as_f64())
            {
                if bal > 0.0 {
                    meter.detail = Some(format!("prepaid ${bal:.0}"));
                }
            }
        }
        Err(e) => {
            warn!(error = %e, "grok monthly billing fetch failed");
            monthly_err = Some(e);
        }
    }

    if meter.windows.is_empty() {
        // Prefer stale over empty error on rate limits
        let rate_limited = credits_err
            .as_ref()
            .map(|e| is_rate_limited(e))
            .unwrap_or(false)
            || monthly_err
                .as_ref()
                .map(|e| is_rate_limited(e))
                .unwrap_or(false);
        if rate_limited {
            cache_backoff("grok", 120);
        }
        if let Some(stale) = cache_get_stale("grok") {
            return stale;
        }
        meter.primary_label = Some("logged in".into());
        meter.secondary_label = meter.account.clone();
        let mut errs = Vec::new();
        if let Some(e) = credits_err {
            errs.push(format!("credits: {e}"));
        }
        if let Some(e) = monthly_err {
            errs.push(format!("monthly: {e}"));
        }
        if !errs.is_empty() {
            meter.error = Some(errs.join("; "));
        }
    } else {
        apply_windows_to_meter(&mut meter);
        if meter.detail.is_none() {
            meter.detail = meter.account.clone();
        }
        // Surface partial errors as non-fatal detail, not red offline
        if let Some(e) = credits_err.or(monthly_err) {
            let extra = format!("partial: {e}");
            meter.detail = Some(match meter.detail.take() {
                Some(d) => format!("{d} · {extra}"),
                None => extra,
            });
        }
        cache_put("grok", &meter, Duration::from_secs(0));
    }
    meter
}

/// Collect usage for all three agents (blocking; call from spawn_blocking).
pub fn collect_all() -> UsageSnapshot {
    let meters = vec![fetch_claude(), fetch_grok(), fetch_codex()];
    UsageSnapshot {
        fetched_at: Utc::now(),
        meters,
    }
}
