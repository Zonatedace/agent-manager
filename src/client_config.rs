//! Client-side connection settings (Windows client → Agent Manager server).
//!
//! Separate from server `Settings` so a thin client can point at any reachable
//! server without scanning repos or starting HTTP locally.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Persistent client connection config (`agent-manager.client.json`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClientConfig {
    /// Base URL of the Agent Manager server, e.g. `http://127.0.0.1:7878/`
    #[serde(default)]
    pub server_url: String,
}

impl ClientConfig {
    pub fn has_server_url(&self) -> bool {
        !self.server_url.trim().is_empty()
    }

    pub fn normalize(&mut self) {
        self.server_url = match normalize_server_url(&self.server_url) {
            Ok(u) => u,
            Err(_) => self.server_url.trim().to_string(),
        };
    }
}

/// Normalize and validate a server URL.
///
/// - Trims whitespace
/// - Requires `http://` or `https://`
/// - Ensures a trailing `/` for consistent navigation
pub fn normalize_server_url(raw: &str) -> Result<String, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("Server URL is required".into());
    }
    let lower = s.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err("Server URL must start with http:// or https://".into());
    }
    // Reject obvious junk (spaces, newlines)
    if s.chars().any(|c| c.is_whitespace()) {
        return Err("Server URL must not contain whitespace".into());
    }
    let mut url = s.to_string();
    if !url.ends_with('/') {
        url.push('/');
    }
    Ok(url)
}

/// Build `/api/health` URL from a normalized base URL.
pub fn health_url(base: &str) -> String {
    let base = if base.ends_with('/') {
        base.to_string()
    } else {
        format!("{base}/")
    };
    format!("{base}api/health")
}

/// Quick health probe (blocking). Returns Ok(()) on 2xx.
pub fn probe_server(base: &str, timeout: std::time::Duration) -> Result<(), String> {
    let url = health_url(base);
    match ureq::get(&url).timeout(timeout).call() {
        Ok(resp) if (200..300).contains(&resp.status()) => Ok(()),
        Ok(resp) => Err(format!("Server returned HTTP {}", resp.status())),
        Err(ureq::Error::Status(code, _)) => Err(format!("Server returned HTTP {code}")),
        Err(e) => Err(format!("Cannot reach server: {e}")),
    }
}

pub fn load(path: &Path) -> ClientConfig {
    if !path.exists() {
        info!(path = %path.display(), "no client config; server URL will be prompted");
        return ClientConfig::default();
    }
    match fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<ClientConfig>(&text) {
            Ok(mut c) => {
                c.normalize();
                info!(
                    path = %path.display(),
                    server_url = %c.server_url,
                    "loaded client config"
                );
                c
            }
            Err(e) => {
                warn!(error = %e, path = %path.display(), "invalid client config; starting empty");
                ClientConfig::default()
            }
        },
        Err(e) => {
            warn!(error = %e, path = %path.display(), "could not read client config");
            ClientConfig::default()
        }
    }
}

pub fn save(path: &Path, config: &ClientConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| format!("create client config dir: {e}"))?;
        }
    }
    let text = serde_json::to_string_pretty(config)
        .map_err(|e| format!("serialize client config: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &text).map_err(|e| format!("write client config temp: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("replace client config: {e}")
    })?;
    info!(path = %path.display(), server_url = %config.server_url, "client config saved");
    Ok(())
}

/// Default client config path next to the server config file.
pub fn default_path_near(server_config: &Path) -> PathBuf {
    if let Some(parent) = server_config.parent() {
        parent.join("agent-manager.client.json")
    } else {
        PathBuf::from("agent-manager.client.json")
    }
}
