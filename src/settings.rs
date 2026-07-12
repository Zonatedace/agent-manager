use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Root folder containing project repos
    pub root: String,
    /// Default agent CLI: grok | claude | codex
    #[serde(default = "default_cli")]
    pub default_cli: String,
    #[serde(default)]
    pub default_model: String,
    #[serde(default)]
    pub default_effort: String,
    #[serde(default = "default_mode")]
    pub default_mode: String,
    #[serde(default)]
    pub default_always_approve: bool,
    /// Open browser when the server starts (CLI can still override)
    #[serde(default = "default_true")]
    pub open_browser_on_start: bool,
    /// UI health poll interval in seconds (min 5)
    #[serde(default = "default_health_poll")]
    pub health_poll_seconds: u64,
    /// Auto re-scan interval in minutes (0 = off)
    #[serde(default)]
    pub auto_refresh_minutes: u64,
    /// Default filter chip: open | done | all
    #[serde(default = "default_item_status")]
    pub default_item_status: String,
    /// When starting agent from global view, use this as cwd if no override
    /// (defaults to root at runtime when empty)
    #[serde(default)]
    pub default_agent_cwd: String,
    /// User preference: enable coding-agent features in the app UI
    /// (CLIs run where available — never inside the Docker server image).
    /// Authentication is performed on the front-end / local machine.
    #[serde(default)]
    pub agents_enabled: bool,
    /// When agents are enabled, collect Claude usage
    #[serde(default = "default_true")]
    pub agents_claude: bool,
    /// When agents are enabled, collect Grok usage / launch Grok
    #[serde(default = "default_true")]
    pub agents_grok: bool,
    /// When agents are enabled, collect Codex usage / launch Codex
    #[serde(default = "default_true")]
    pub agents_codex: bool,
}

fn default_cli() -> String {
    "grok".into()
}
fn default_mode() -> String {
    "interactive".into()
}
fn default_true() -> bool {
    true
}
fn default_health_poll() -> u64 {
    15
}
fn default_item_status() -> String {
    "open".into()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            // Empty until resolved from CLI / .env / portable default at startup
            root: String::new(),
            default_cli: default_cli(),
            default_model: String::new(),
            default_effort: String::new(),
            default_mode: default_mode(),
            default_always_approve: false,
            open_browser_on_start: true,
            health_poll_seconds: 15,
            auto_refresh_minutes: 0,
            default_item_status: default_item_status(),
            default_agent_cwd: String::new(),
            agents_enabled: false,
            agents_claude: true,
            agents_grok: true,
            agents_codex: true,
        }
    }
}

/// Whether this process is allowed to host agent CLIs at all.
/// Docker sets `AGENT_MANAGER_ALLOW_AGENTS=0` — agents are app/client-side only.
pub fn allow_agents_from_env() -> bool {
    match std::env::var("AGENT_MANAGER_ALLOW_AGENTS") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            !(v == "0" || v == "false" || v == "no" || v == "off")
        }
        // Default: allow on native hosts; Docker compose always sets 0.
        Err(_) => true,
    }
}

/// User home directory (`USERPROFILE` on Windows, `HOME` elsewhere).
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// Portable default scan root when no CLI / env / config root is set.
/// Prefers common repo parent folders under the home directory, then cwd.
pub fn default_scan_root() -> PathBuf {
    if let Some(home) = home_dir() {
        let candidates = [
            home.join("Desktop").join("Repos"),
            home.join("Desktop").join("repos"),
            home.join("repos"),
            home.join("Projects"),
            home.join("projects"),
            home.join("dev"),
            home.join("code"),
            home.join("src"),
        ];
        for p in candidates {
            if p.is_dir() {
                return p;
            }
        }
        return home;
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

impl Settings {
    pub fn root_path(&self) -> PathBuf {
        PathBuf::from(&self.root)
    }

    pub fn normalize(&mut self) {
        self.default_cli = self.default_cli.trim().to_ascii_lowercase();
        if !matches!(
            self.default_cli.as_str(),
            "grok" | "claude" | "codex"
        ) {
            self.default_cli = default_cli();
        }
        if !matches!(self.default_mode.as_str(), "interactive" | "headless") {
            self.default_mode = default_mode();
        }
        if !matches!(
            self.default_item_status.as_str(),
            "open" | "done" | "all"
        ) {
            self.default_item_status = default_item_status();
        }
        if self.health_poll_seconds < 5 {
            self.health_poll_seconds = 5;
        }
        // Trim paths
        self.root = self.root.trim().to_string();
        self.default_agent_cwd = self.default_agent_cwd.trim().to_string();
    }

    pub fn validate_root(path: &Path) -> Result<(), String> {
        if !path.exists() {
            return Err(format!("path does not exist: {}", path.display()));
        }
        if !path.is_dir() {
            return Err(format!("path is not a directory: {}", path.display()));
        }
        Ok(())
    }
}

pub fn load(path: &Path) -> Settings {
    if !path.exists() {
        info!(path = %path.display(), "no config file; using defaults");
        let mut s = Settings::default();
        s.normalize();
        return s;
    }
    match fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<Settings>(&text) {
            Ok(mut s) => {
                s.normalize();
                info!(path = %path.display(), root = %s.root, "loaded settings");
                s
            }
            Err(e) => {
                warn!(error = %e, path = %path.display(), "invalid config; using defaults");
                let mut s = Settings::default();
                s.normalize();
                s
            }
        },
        Err(e) => {
            warn!(error = %e, path = %path.display(), "could not read config; using defaults");
            let mut s = Settings::default();
            s.normalize();
            s
        }
    }
}

pub fn save(path: &Path, settings: &Settings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| format!("create config dir: {e}"))?;
        }
    }
    let text = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("serialize settings: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &text).map_err(|e| format!("write config temp: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("replace config: {e}")
    })?;
    info!(path = %path.display(), "settings saved");
    Ok(())
}

/// Merge CLI overrides into loaded settings (CLI wins for root when provided explicitly).
pub fn apply_cli_overrides(settings: &mut Settings, root: Option<PathBuf>) {
    if let Some(r) = root {
        settings.root = r.to_string_lossy().to_string();
    }
    settings.normalize();
}
