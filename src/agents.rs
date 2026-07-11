use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
// Path used by codex config helpers

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentCli {
    Grok,
    Claude,
    Codex,
}

impl AgentCli {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentCli::Grok => "grok",
            AgentCli::Claude => "claude",
            AgentCli::Codex => "codex",
        }
    }

    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "grok" => Some(AgentCli::Grok),
            "claude" => Some(AgentCli::Claude),
            "codex" => Some(AgentCli::Codex),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentMode {
    /// Open a new interactive terminal window
    Interactive,
    /// Run non-interactively (print/exec), keep a log
    Headless,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRequest {
    /// Optional project id (used when `cwd` is not set)
    #[serde(default)]
    pub project_id: String,
    /// Explicit working directory override (absolute path)
    #[serde(default)]
    pub cwd: Option<String>,
    pub cli: String,
    pub prompt: String,
    #[serde(default)]
    pub model: String,
    /// Claude: --effort, Grok: --reasoning-effort, Codex: model_reasoning_effort
    #[serde(default)]
    pub effort: String,
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Grok --always-approve / Claude --dangerously-skip-permissions / Codex approval never
    #[serde(default)]
    pub always_approve: bool,
    #[serde(default)]
    pub extra_args: Vec<String>,
    /// Optional display name shown in the terminal title
    #[serde(default)]
    pub name: String,
}

fn default_mode() -> String {
    "interactive".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliStatus {
    pub name: String,
    pub available: bool,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelOption {
    /// Value passed to --model
    pub id: String,
    /// Human-readable label for the picker
    pub label: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliModelCatalog {
    pub models: Vec<ModelOption>,
    pub default_model: Option<String>,
    pub efforts: Vec<String>,
    /// How the list was obtained: "cli", "cache", "fallback"
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsByCli {
    pub grok: CliModelCatalog,
    pub claude: CliModelCatalog,
    pub codex: CliModelCatalog,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDiscovery {
    pub clis: Vec<CliStatus>,
    /// Rich per-CLI model catalogs (preferred by UI)
    pub models: ModelsByCli,
    /// Flat id lists (compat / simple clients)
    pub model_presets: ModelPresets,
    pub effort_presets: EffortPresets,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPresets {
    pub grok: Vec<String>,
    pub claude: Vec<String>,
    pub codex: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffortPresets {
    pub grok: Vec<String>,
    pub claude: Vec<String>,
    pub codex: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchResult {
    pub ok: bool,
    pub cli: String,
    pub mode: String,
    pub cwd: String,
    pub command_preview: String,
    pub log_path: Option<String>,
    pub message: String,
}

fn which_cli(name: &str) -> Option<PathBuf> {
    // Try `where` on Windows first for .cmd/.ps1 shims
    #[cfg(windows)]
    {
        if let Ok(output) = Command::new("where").arg(name).output() {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if let Some(line) = stdout.lines().next() {
                    let p = PathBuf::from(line.trim());
                    if p.exists() {
                        return Some(p);
                    }
                }
            }
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(output) = Command::new("which").arg(name).output() {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if let Some(line) = stdout.lines().next() {
                    let p = PathBuf::from(line.trim());
                    if p.exists() {
                        return Some(p);
                    }
                }
            }
        }
    }
    None
}

fn run_cli_capture(cli: &str, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new(cli);
    cmd.args(args);
    // Avoid interactive TUI refusals for tools that check TERM
    cmd.env("TERM", "dumb");
    cmd.env("NO_COLOR", "1");
    cmd.env("CI", "1");
    let output = cmd.output().ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = if stdout.trim().is_empty() {
        stderr
    } else if stderr.trim().is_empty() {
        stdout
    } else {
        format!("{stdout}\n{stderr}")
    };
    if combined.trim().is_empty() {
        None
    } else {
        Some(combined)
    }
}

fn catalog_ids(cat: &CliModelCatalog) -> Vec<String> {
    cat.models.iter().map(|m| m.id.clone()).collect()
}

fn parse_grok_models(text: &str) -> CliModelCatalog {
    let mut default_model: Option<String> = None;
    let mut models: Vec<ModelOption> = Vec::new();

    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Default model:") {
            let id = rest.trim().to_string();
            if !id.is_empty() {
                default_model = Some(id);
            }
            continue;
        }
        // "* grok-4.5 (default)" or "- grok-composer-2.5-fast"
        let item = t
            .strip_prefix('*')
            .or_else(|| t.strip_prefix('-'))
            .map(|s| s.trim());
        let Some(item) = item else { continue };
        if item.is_empty() {
            continue;
        }
        let is_default = item.contains("(default)");
        let id = item
            .split_whitespace()
            .next()
            .unwrap_or(item)
            .trim_matches(|c| c == '(' || c == ')')
            .to_string();
        if id.is_empty() || id == "Available" || id.eq_ignore_ascii_case("models") {
            continue;
        }
        // Avoid duplicating
        if models.iter().any(|m| m.id == id) {
            continue;
        }
        if is_default {
            default_model = Some(id.clone());
        }
        models.push(ModelOption {
            label: if is_default {
                format!("{id} (default)")
            } else {
                id.clone()
            },
            id,
            is_default,
        });
    }

    if let Some(ref d) = default_model {
        for m in &mut models {
            m.is_default = m.id == *d;
            if m.is_default && !m.label.contains("default") {
                m.label = format!("{} (default)", m.id);
            }
        }
    }

    // Put default first
    models.sort_by(|a, b| b.is_default.cmp(&a.is_default).then(a.id.cmp(&b.id)));

    if models.is_empty() {
        return CliModelCatalog {
            models: vec![
                ModelOption {
                    id: "grok-4.5".into(),
                    label: "grok-4.5 (fallback)".into(),
                    is_default: true,
                },
                ModelOption {
                    id: "grok-composer-2.5-fast".into(),
                    label: "grok-composer-2.5-fast".into(),
                    is_default: false,
                },
            ],
            default_model: Some("grok-4.5".into()),
            efforts: vec![
                "low".into(),
                "medium".into(),
                "high".into(),
            ],
            source: "fallback".into(),
        };
    }

    CliModelCatalog {
        default_model,
        models,
        efforts: vec![
            "low".into(),
            "medium".into(),
            "high".into(),
        ],
        source: "cli:grok models".into(),
    }
}

fn parse_claude_models_from_text(text: &str) -> Vec<ModelOption> {
    let mut models = Vec::new();
    // Match `claude-…` model ids in backticks or bare
    let re = regex::Regex::new(r"`?(claude-[a-z0-9][a-z0-9.\-]*)`?").unwrap();
    for cap in re.captures_iter(text) {
        let id = cap.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
        if id.is_empty() || models.iter().any(|m: &ModelOption| m.id == id) {
            continue;
        }
        models.push(ModelOption {
            label: id.clone(),
            id,
            is_default: false,
        });
    }
    models
}

fn discover_claude_models() -> CliModelCatalog {
    // Prefer short aliases from CLI help, then full IDs when we can harvest them.
    let mut models: Vec<ModelOption> = vec![
        ModelOption {
            id: "sonnet".into(),
            label: "sonnet (alias - latest Sonnet)".into(),
            is_default: true,
        },
        ModelOption {
            id: "opus".into(),
            label: "opus (alias - latest Opus)".into(),
            is_default: false,
        },
        ModelOption {
            id: "haiku".into(),
            label: "haiku (alias - latest Haiku)".into(),
            is_default: false,
        },
    ];

    // `claude models` is not a real subcommand in current builds — it may invoke the model.
    // Prefer help text / version banners; fall back to known current full IDs.
    let mut source = "aliases+fallback".to_string();
    if let Some(help) = run_cli_capture("claude", &["--help"]) {
        let from_help = parse_claude_models_from_text(&help);
        for m in from_help {
            if !models.iter().any(|x| x.id == m.id) {
                models.push(m);
            }
        }
    }

    // Known current full IDs (kept as selectable options)
    for id in [
        "claude-sonnet-4-6",
        "claude-opus-4-7",
        "claude-opus-4-6",
        "claude-haiku-4-5-20251001",
        "claude-haiku-4-5",
    ] {
        if !models.iter().any(|m| m.id == id) {
            models.push(ModelOption {
                id: id.into(),
                label: id.into(),
                is_default: false,
            });
        }
    }

    // Optional: try `claude models` only if CLAUDE_ENUMERATE_MODELS=1 (slow / may bill)
    if std::env::var("CLAUDE_ENUMERATE_MODELS").ok().as_deref() == Some("1") {
        if let Some(out) = run_cli_capture("claude", &["models"]) {
            let extra = parse_claude_models_from_text(&out);
            if !extra.is_empty() {
                source = "cli:claude models".into();
                for m in extra {
                    if !models.iter().any(|x| x.id == m.id) {
                        models.push(m);
                    }
                }
            }
        }
    }

    CliModelCatalog {
        models,
        default_model: Some("sonnet".into()),
        efforts: vec![
            "low".into(),
            "medium".into(),
            "high".into(),
            "xhigh".into(),
            "max".into(),
        ],
        source,
    }
}

fn discover_codex_models() -> CliModelCatalog {
    // Prefer ~/.codex/models_cache.json (populated by Codex CLI)
    let home = dirs_home();
    let cache = home.join(".codex").join("models_cache.json");
    if let Ok(text) = std::fs::read_to_string(&cache) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            let mut models = Vec::new();
            let mut efforts_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            let mut default_model: Option<String> = None;

            if let Some(arr) = v.get("models").and_then(|m| m.as_array()) {
                for m in arr {
                    let visibility = m
                        .get("visibility")
                        .and_then(|x| x.as_str())
                        .unwrap_or("list");
                    // Skip hidden internal models
                    if visibility.eq_ignore_ascii_case("hide")
                        || visibility.eq_ignore_ascii_case("hidden")
                    {
                        continue;
                    }
                    let id = m
                        .get("slug")
                        .or_else(|| m.get("id"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    if id.is_empty() {
                        continue;
                    }
                    let display = m
                        .get("display_name")
                        .and_then(|x| x.as_str())
                        .unwrap_or(&id)
                        .to_string();
                    let priority = m.get("priority").and_then(|x| x.as_u64()).unwrap_or(999);
                    if default_model.is_none() && priority <= 2 {
                        default_model = Some(id.clone());
                    }
                    if let Some(levels) = m
                        .get("supported_reasoning_levels")
                        .and_then(|x| x.as_array())
                    {
                        for lvl in levels {
                            if let Some(e) = lvl.get("effort").and_then(|x| x.as_str()) {
                                efforts_set.insert(e.to_string());
                            }
                        }
                    }
                    models.push((priority, ModelOption {
                        label: if display != id {
                            format!("{display} ({id})")
                        } else {
                            id.clone()
                        },
                        id,
                        is_default: false,
                    }));
                }
            }

            models.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.id.cmp(&b.1.id)));
            let mut models: Vec<ModelOption> = models.into_iter().map(|(_, m)| m).collect();

            // Prefer config.toml model as default when present
            let config_model = read_codex_config_model(&home);
            if let Some(ref cm) = config_model {
                default_model = Some(cm.clone());
            }
            if default_model.is_none() {
                default_model = models.first().map(|m| m.id.clone());
            }
            if let Some(ref d) = default_model {
                for m in &mut models {
                    m.is_default = m.id == *d;
                    if m.is_default && !m.label.contains("default") {
                        m.label = format!("{} - default", m.label);
                    }
                }
                // Move default to front
                models.sort_by(|a, b| b.is_default.cmp(&a.is_default));
            }

            if !models.is_empty() {
                let efforts: Vec<String> = if efforts_set.is_empty() {
                    vec![
                        "low".into(),
                        "medium".into(),
                        "high".into(),
                        "xhigh".into(),
                    ]
                } else {
                    // stable order
                    let order = ["low", "medium", "high", "xhigh", "max", "ultra"];
                    let mut e: Vec<String> = order
                        .iter()
                        .filter(|x| efforts_set.contains(**x))
                        .map(|s| (*s).to_string())
                        .collect();
                    for extra in efforts_set {
                        if !e.iter().any(|x| x == &extra) {
                            e.push(extra);
                        }
                    }
                    e
                };
                return CliModelCatalog {
                    models,
                    default_model,
                    efforts,
                    source: format!("cache:{}", cache.display()),
                };
            }
        }
    }

    // Fallback presets
    let config_model = read_codex_config_model(&home);
    let mut models = vec![
        "gpt-5.5",
        "gpt-5.4",
        "gpt-5.4-mini",
        "gpt-5.6-sol",
        "gpt-5.6-terra",
        "gpt-5.6-luna",
        "o3",
        "o4-mini",
    ]
    .into_iter()
    .map(|id| ModelOption {
        id: id.into(),
        label: id.into(),
        is_default: false,
    })
    .collect::<Vec<_>>();
    let default_model = config_model.or_else(|| Some("gpt-5.5".into()));
    if let Some(ref d) = default_model {
        for m in &mut models {
            m.is_default = m.id == *d;
        }
        models.sort_by(|a, b| b.is_default.cmp(&a.is_default));
    }
    CliModelCatalog {
        models,
        default_model,
        efforts: vec![
            "low".into(),
            "medium".into(),
            "high".into(),
            "xhigh".into(),
        ],
        source: "fallback".into(),
    }
}

fn dirs_home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn read_codex_config_model(home: &Path) -> Option<String> {
    let path = home.join(".codex").join("config.toml");
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("model") {
            let rest = rest.trim();
            if let Some(rest) = rest.strip_prefix('=') {
                let v = rest.trim().trim_matches('"').trim_matches('\'').to_string();
                if !v.is_empty() {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn discover_grok_models() -> CliModelCatalog {
    if which_cli("grok").is_none() {
        return parse_grok_models(""); // fallback
    }
    match run_cli_capture("grok", &["models"]) {
        Some(out) => parse_grok_models(&out),
        None => parse_grok_models(""),
    }
}

pub fn discover() -> AgentDiscovery {
    let names = ["grok", "claude", "codex"];
    let clis = names
        .iter()
        .map(|n| {
            let path = which_cli(n);
            CliStatus {
                name: n.to_string(),
                available: path.is_some(),
                path: path.map(|p| p.to_string_lossy().to_string()),
            }
        })
        .collect();

    let grok = discover_grok_models();
    let claude = discover_claude_models();
    let codex = discover_codex_models();

    let model_presets = ModelPresets {
        grok: catalog_ids(&grok),
        claude: catalog_ids(&claude),
        codex: catalog_ids(&codex),
    };
    let effort_presets = EffortPresets {
        grok: grok.efforts.clone(),
        claude: claude.efforts.clone(),
        codex: codex.efforts.clone(),
    };

    tracing::info!(
        grok_models = grok.models.len(),
        grok_source = %grok.source,
        claude_models = claude.models.len(),
        claude_source = %claude.source,
        codex_models = codex.models.len(),
        codex_source = %codex.source,
        "agent model discovery complete"
    );

    AgentDiscovery {
        clis,
        models: ModelsByCli {
            grok,
            claude,
            codex,
        },
        model_presets,
        effort_presets,
    }
}

fn quote_ps(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Build argv for the agent CLI (not including shell wrapper).
pub fn build_args(cli: AgentCli, req: &AgentRequest, mode: AgentMode) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    let prompt = req.prompt.trim();

    match cli {
        AgentCli::Grok => {
            if mode == AgentMode::Headless {
                // Single-turn headless
                if !req.model.trim().is_empty() {
                    args.push("--model".into());
                    args.push(req.model.trim().into());
                }
                if !req.effort.trim().is_empty() {
                    args.push("--reasoning-effort".into());
                    args.push(req.effort.trim().into());
                }
                if req.always_approve {
                    args.push("--always-approve".into());
                }
                args.extend(req.extra_args.iter().cloned().filter(|s| !s.is_empty()));
                args.push("-p".into());
                args.push(prompt.into());
            } else {
                if !req.model.trim().is_empty() {
                    args.push("--model".into());
                    args.push(req.model.trim().into());
                }
                if !req.effort.trim().is_empty() {
                    args.push("--reasoning-effort".into());
                    args.push(req.effort.trim().into());
                }
                if req.always_approve {
                    args.push("--always-approve".into());
                } else {
                    // Sensible default for coding work
                    args.push("--permission-mode".into());
                    args.push("acceptEdits".into());
                }
                args.extend(req.extra_args.iter().cloned().filter(|s| !s.is_empty()));
                if !prompt.is_empty() {
                    args.push(prompt.into());
                }
            }
        }
        AgentCli::Claude => {
            if mode == AgentMode::Headless {
                args.push("-p".into());
            }
            if !req.model.trim().is_empty() {
                args.push("--model".into());
                args.push(req.model.trim().into());
            }
            if !req.effort.trim().is_empty() {
                args.push("--effort".into());
                args.push(req.effort.trim().into());
            }
            if !req.name.trim().is_empty() {
                args.push("--name".into());
                args.push(req.name.trim().into());
            }
            if req.always_approve {
                args.push("--dangerously-skip-permissions".into());
            }
            args.extend(req.extra_args.iter().cloned().filter(|s| !s.is_empty()));
            if !prompt.is_empty() {
                args.push(prompt.into());
            }
        }
        AgentCli::Codex => {
            if mode == AgentMode::Headless {
                args.push("exec".into());
                args.push("--skip-git-repo-check".into());
                args.push("--ask-for-approval".into());
                args.push(if req.always_approve {
                    "never".into()
                } else {
                    "on-request".into()
                });
            }
            if !req.model.trim().is_empty() {
                args.push("--model".into());
                args.push(req.model.trim().into());
            }
            if !req.effort.trim().is_empty() {
                args.push("-c".into());
                args.push(format!(
                    "model_reasoning_effort=\"{}\"",
                    req.effort.trim().replace('"', "")
                ));
            }
            args.extend(req.extra_args.iter().cloned().filter(|s| !s.is_empty()));
            if !prompt.is_empty() {
                args.push(prompt.into());
            }
        }
    }
    args
}

pub fn command_preview(cli: AgentCli, args: &[String], cwd: &Path) -> String {
    let mut parts = vec![cli.as_str().to_string()];
    for a in args {
        if a.chars().any(|c| c.is_whitespace()) {
            parts.push(format!("\"{}\"", a.replace('"', "\\\"")));
        } else {
            parts.push(a.clone());
        }
    }
    format!("cd {} && {}", cwd.display(), parts.join(" "))
}

/// Windows process creation flags so the agent is not tied to the server console/job.
#[cfg(windows)]
mod win_flags {
    pub const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    pub const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    pub const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    pub const DETACHED_PROCESS: u32 = 0x0000_0008;
}

/// Write a small .ps1 launcher so long prompts / quoting never hit cmd.exe limits,
/// then spawn a brand-new console that is not waited on by the server.
#[cfg(windows)]
pub fn launch_interactive(cli: AgentCli, args: &[String], cwd: &Path, title: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    let script_path = write_agent_script(cli, args, cwd, title)?;
    let script_str = script_path.to_string_lossy().to_string();
    let cwd_str = cwd.to_string_lossy().to_string();
    let mut last_err = String::new();

    // 1) Preferred: own console window, don't wait, don't share our console
    let direct_flags: &[u32] = &[
        win_flags::CREATE_NEW_CONSOLE | win_flags::CREATE_NEW_PROCESS_GROUP,
        win_flags::CREATE_NEW_CONSOLE,
    ];
    for flags in direct_flags {
        let mut direct = Command::new("powershell.exe");
        direct
            .args([
                "-NoExit",
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                &script_str,
            ])
            .current_dir(cwd)
            .stdin(std::process::Stdio::null())
            // Leave stdout/stderr inherited into the NEW console (CREATE_NEW_CONSOLE)
            .creation_flags(*flags);
        match direct.spawn() {
            Ok(child) => {
                // Intentionally leak/drop without wait — agent is independent
                drop(child);
                tracing::info!(
                    script = %script_path.display(),
                    cwd = %cwd.display(),
                    cli = cli.as_str(),
                    flags,
                    "spawned agent via CREATE_NEW_CONSOLE"
                );
                return Ok(());
            }
            Err(e) => {
                last_err = e.to_string();
                tracing::warn!(error = %e, flags, "direct console spawn failed");
            }
        }
    }

    // 2) Start-Process from a hidden powershell (separate process tree)
    let launch_ps = format!(
        "Start-Process -FilePath {ps} -WorkingDirectory {cwd} -WindowStyle Normal -ArgumentList @('-NoExit','-NoProfile','-ExecutionPolicy','Bypass','-File',{script})",
        ps = quote_ps("powershell.exe"),
        cwd = quote_ps(&cwd_str),
        script = quote_ps(&script_str),
    );
    let launcher_flags: &[u32] = &[
        win_flags::CREATE_NO_WINDOW | win_flags::CREATE_NEW_PROCESS_GROUP,
        win_flags::CREATE_NO_WINDOW,
        0,
    ];
    for flags in launcher_flags {
        let mut child = Command::new("powershell.exe");
        child
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &launch_ps,
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        if *flags != 0 {
            child.creation_flags(*flags);
        }
        match child.spawn() {
            Ok(mut c) => {
                let _ = c.try_wait();
                tracing::info!(
                    script = %script_path.display(),
                    flags,
                    "spawned agent via Start-Process launcher"
                );
                return Ok(());
            }
            Err(e) => {
                last_err = e.to_string();
                tracing::warn!(error = %e, flags, "launcher spawn attempt failed");
            }
        }
    }

    // 3) Last resort: cmd /c start
    let title_safe = title.replace('"', "");
    let mut cmd = Command::new("cmd.exe");
    cmd.args([
        "/C",
        "start",
        &title_safe,
        "powershell.exe",
        "-NoExit",
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        &script_str,
    ])
    .current_dir(cwd)
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null());

    match cmd.spawn() {
        Ok(mut c) => {
            let _ = c.try_wait();
            tracing::info!(script = %script_path.display(), "spawned agent via cmd start");
            Ok(())
        }
        Err(e) => Err(format!(
            "failed to spawn agent terminal (last error: {last_err}; cmd start: {e})"
        )),
    }
}

/// Persist a launcher script under the system temp dir (not under the project).
#[cfg(windows)]
fn write_agent_script(
    cli: AgentCli,
    args: &[String],
    cwd: &Path,
    title: &str,
) -> Result<PathBuf, String> {
    let safe_title = title
        .chars()
        .map(|c| if r#"<>:"/\|?*"#.contains(c) { '-' } else { c })
        .collect::<String>();
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S-%3f");
    let path = std::env::temp_dir().join(format!(
        "agent-manager-agent-{}-{}.ps1",
        cli.as_str(),
        stamp
    ));

    let mut lines = Vec::new();
    lines.push("$ErrorActionPreference = 'Continue'".to_string());
    lines.push(format!(
        "try {{ $Host.UI.RawUI.WindowTitle = {} }} catch {{}}",
        quote_ps(&safe_title)
    ));
    lines.push(format!(
        "Set-Location -LiteralPath {}",
        quote_ps(&cwd.to_string_lossy())
    ));
    lines.push(format!(
        "Write-Host {} -ForegroundColor Cyan",
        quote_ps(&format!("agent-manager → starting {} in {}", cli.as_str(), cwd.display()))
    ));
    // Prefer PATH resolution inside the new shell
    let mut invoke = format!("& {}", quote_ps(cli.as_str()));
    for a in args {
        invoke.push(' ');
        invoke.push_str(&quote_ps(a));
    }
    lines.push(invoke);
    lines.push("Write-Host ''".into());
    lines.push(
        "Write-Host 'Agent exited. Press Enter to close.' -ForegroundColor DarkGray".into(),
    );
    lines.push("[void][System.Console]::ReadLine()".into());

    let body = lines.join("\r\n");
    std::fs::write(&path, body.as_bytes())
        .map_err(|e| format!("failed to write agent script {}: {e}", path.display()))?;
    Ok(path)
}

#[cfg(not(windows))]
pub fn launch_interactive(cli: AgentCli, args: &[String], cwd: &Path, title: &str) -> Result<(), String> {
    let _ = title;
    let mut cmd = Command::new(cli.as_str());
    cmd.args(args).current_dir(cwd);
    // Best-effort: try common terminal emulators
    for term in ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"] {
        let mut t = Command::new(term);
        if term == "gnome-terminal" {
            t.args(["--", "bash", "-lc"]);
            let inner = format!(
                "cd {} && {} {}; exec bash",
                shell_escape(&cwd.to_string_lossy()),
                cli.as_str(),
                args.iter().map(|a| shell_escape(a)).collect::<Vec<_>>().join(" ")
            );
            t.arg(inner);
        } else {
            t.arg("-e");
            // fallback simplistic
            continue;
        }
        if t.spawn().is_ok() {
            return Ok(());
        }
    }
    // Last resort: run detached without terminal (not ideal)
    cmd.spawn()
        .map_err(|e| format!("failed to spawn agent: {e}"))?;
    Ok(())
}

#[cfg(not(windows))]
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Launch headless agent, logging stdout/stderr to a file under the project (or temp).
pub fn launch_headless(
    cli: AgentCli,
    args: &[String],
    cwd: &Path,
    log_dir: &Path,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(log_dir).map_err(|e| format!("log dir: {e}"))?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let log_path = log_dir.join(format!("agent-{}-{}.log", cli.as_str(), stamp));

    let log_file = std::fs::File::create(&log_path)
        .map_err(|e| format!("create log: {e}"))?;
    let log_err = log_file
        .try_clone()
        .map_err(|e| format!("clone log: {e}"))?;

    let mut cmd = Command::new(cli.as_str());
    cmd.args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(log_file)
        .stderr(log_err);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Prefer no-window + new group; skip breakaway (often Access Denied under jobs)
        cmd.creation_flags(win_flags::CREATE_NO_WINDOW | win_flags::CREATE_NEW_PROCESS_GROUP);
    }

    match cmd.spawn() {
        Ok(_) => Ok(log_path),
        Err(e) => {
            // Retry with no special flags
            let log_file = std::fs::OpenOptions::new()
                .append(true)
                .open(&log_path)
                .map_err(|e2| format!("reopen log: {e2}"))?;
            let log_err = log_file.try_clone().map_err(|e2| format!("clone log: {e2}"))?;
            Command::new(cli.as_str())
                .args(args)
                .current_dir(cwd)
                .stdin(std::process::Stdio::null())
                .stdout(log_file)
                .stderr(log_err)
                .spawn()
                .map_err(|e2| format!("failed to spawn headless agent ({e} / retry {e2})"))?;
            Ok(log_path)
        }
    }
}

pub fn start_agent(
    req: &AgentRequest,
    cwd: &Path,
) -> Result<LaunchResult, String> {
    let cli = AgentCli::from_str_loose(&req.cli)
        .ok_or_else(|| format!("unknown cli: {}", req.cli))?;

    if which_cli(cli.as_str()).is_none() {
        return Err(format!(
            "{} CLI not found on PATH. Install it or open a shell where it is available.",
            cli.as_str()
        ));
    }

    if req.prompt.trim().is_empty() {
        // Interactive empty prompt is OK (opens session); headless needs prompt
        if req.mode.eq_ignore_ascii_case("headless") {
            return Err("prompt is required for headless mode".into());
        }
    }

    let mode = if req.mode.eq_ignore_ascii_case("headless") {
        AgentMode::Headless
    } else {
        AgentMode::Interactive
    };

    let args = build_args(cli, req, mode);
    let preview = command_preview(cli, &args, cwd);

    match mode {
        AgentMode::Interactive => {
            let title = if req.name.trim().is_empty() {
                format!("{} · {}", cli.as_str(), cwd.file_name().and_then(|s| s.to_str()).unwrap_or("project"))
            } else {
                req.name.trim().to_string()
            };
            launch_interactive(cli, &args, cwd, &title)?;
            Ok(LaunchResult {
                ok: true,
                cli: cli.as_str().into(),
                mode: "interactive".into(),
                cwd: cwd.to_string_lossy().into(),
                command_preview: preview,
                log_path: None,
                message: "Agent started in a new terminal window".into(),
            })
        }
        AgentMode::Headless => {
            let log_dir = cwd.join(".agent-manager-logs");
            let log_path = launch_headless(cli, &args, cwd, &log_dir)?;
            Ok(LaunchResult {
                ok: true,
                cli: cli.as_str().into(),
                mode: "headless".into(),
                cwd: cwd.to_string_lossy().into(),
                command_preview: preview,
                log_path: Some(log_path.to_string_lossy().into()),
                message: format!("Headless agent started; logging to {}", log_path.display()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_claude_interactive() {
        let req = AgentRequest {
            project_id: "x".into(),
            cwd: None,
            cli: "claude".into(),
            prompt: "fix the bug".into(),
            model: "sonnet".into(),
            effort: "high".into(),
            mode: "interactive".into(),
            always_approve: false,
            extra_args: vec![],
            name: "".into(),
        };
        let args = build_args(AgentCli::Claude, &req, AgentMode::Interactive);
        assert!(args.contains(&"--model".into()));
        assert!(args.contains(&"sonnet".into()));
        assert!(args.contains(&"fix the bug".into()));
        assert!(!args.iter().any(|a| a == "-p"));
    }

    #[test]
    fn builds_grok_always_approve() {
        let req = AgentRequest {
            project_id: "x".into(),
            cwd: None,
            cli: "grok".into(),
            prompt: "do it".into(),
            model: "".into(),
            effort: "".into(),
            mode: "interactive".into(),
            always_approve: true,
            extra_args: vec![],
            name: "".into(),
        };
        let args = build_args(AgentCli::Grok, &req, AgentMode::Interactive);
        assert!(args.iter().any(|a| a == "--always-approve"));
    }

    #[test]
    fn parses_grok_models_output() {
        let sample = r#"
You are logged in with grok.com.

Default model: grok-4.5

Available models:
  * grok-4.5 (default)
  - grok-composer-2.5-fast
"#;
        let cat = parse_grok_models(sample);
        assert_eq!(cat.default_model.as_deref(), Some("grok-4.5"));
        assert!(cat.models.iter().any(|m| m.id == "grok-4.5" && m.is_default));
        assert!(cat.models.iter().any(|m| m.id == "grok-composer-2.5-fast"));
    }
}
