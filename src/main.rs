// Hide the console window in Windows release builds (file logging still works).
// Debug builds keep a console for tracing. Override with --console.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod agents;
mod client_config;
mod desktop;
mod fsbrowser;
mod git;
mod models;
mod parser;
mod process_util;
mod scanner;
mod server;
#[cfg(windows)]
mod service;
mod sessions;
mod settings;
mod todos;
mod usage;

use clap::{Parser, ValueEnum};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum RunMode {
    /// Native desktop window + local server (default on Windows)
    App,
    /// HTTP server only — no window (browser optional)
    Server,
    /// Thin Windows client: connect to a server URL (prompted until set)
    Client,
    /// Windows Service host (SCM; headless HTTP with stop control)
    Service,
}

#[derive(Parser, Debug, Clone)]
#[command(
    name = "agent-manager",
    about = "Agent Manager — local multi-repo TODO app with agents and usage meters",
    long_about = "Windows desktop app (WebView2) by default. Use --mode server for headless HTTP, --mode service for Windows Service, or --mode client for a thin window that connects to a server URL.\n\nPaths: set AGENT_MANAGER_ROOT in .env (see .env.example). CLI flags override env; env overrides config."
)]
struct Args {
    /// Root directory containing project repos (env: AGENT_MANAGER_ROOT)
    #[arg(short, long, env = "AGENT_MANAGER_ROOT")]
    root: Option<PathBuf>,

    /// HTTP port (env: AGENT_MANAGER_PORT)
    #[arg(short, long, env = "AGENT_MANAGER_PORT", default_value_t = 7878)]
    port: u16,

    /// Bind address for the HTTP server (env: AGENT_MANAGER_HOST). Use 0.0.0.0 for LAN clients.
    #[arg(long, env = "AGENT_MANAGER_HOST", default_value = "127.0.0.1")]
    host: String,

    /// Run mode: app | server | client | service
    #[arg(long, value_enum, default_value_t = default_run_mode())]
    mode: RunMode,

    /// Shorthand for --mode server
    #[arg(long, default_value_t = false)]
    server: bool,

    /// Shorthand for --mode client
    #[arg(long, default_value_t = false)]
    client: bool,

    /// Shorthand for --mode service (Windows Service host)
    #[arg(long, default_value_t = false)]
    service: bool,

    /// Server URL for client mode (env: AGENT_MANAGER_SERVER_URL)
    #[arg(long, env = "AGENT_MANAGER_SERVER_URL")]
    server_url: Option<String>,

    /// Path to client connection config (env: AGENT_MANAGER_CLIENT_CONFIG)
    #[arg(long, env = "AGENT_MANAGER_CLIENT_CONFIG")]
    client_config: Option<PathBuf>,

    /// Force the client setup prompt (ignore saved server URL)
    #[arg(long, default_value_t = false)]
    reset_server_url: bool,

    /// Attach a console on Windows release builds (for debugging)
    #[arg(long, default_value_t = false)]
    console: bool,

    /// Open an external browser (server mode only; ignored in app/service mode)
    #[arg(long, default_value_t = true)]
    open: bool,

    /// Do not open an external browser
    #[arg(long, default_value_t = false)]
    no_open: bool,

    /// Log file path (env: AGENT_MANAGER_LOG_FILE)
    #[arg(long, env = "AGENT_MANAGER_LOG_FILE", default_value = "agent-manager.log")]
    log_file: PathBuf,

    /// Log level filter (e.g. info, debug, agent_manager=debug)
    #[arg(long, env = "AGENT_MANAGER_LOG_LEVEL", default_value = "info")]
    log_level: String,

    /// Path to settings JSON (env: AGENT_MANAGER_CONFIG)
    #[arg(
        long,
        env = "AGENT_MANAGER_CONFIG",
        default_value = "agent-manager.config.json"
    )]
    config: PathBuf,

    /// Ignore saved config root and force this root
    #[arg(long)]
    force_root: Option<PathBuf>,
}

fn default_run_mode() -> RunMode {
    // Dedicated client binary always defaults to thin-client mode.
    if exe_is_client_bin() {
        return RunMode::Client;
    }
    if cfg!(windows) {
        RunMode::App
    } else {
        RunMode::Server
    }
}

/// True when this process was launched as `agent-manager-client(.exe)`.
/// The Cargo.toml defines a second [[bin]] with the same main that only
/// differs by output name — we detect that name so double-clicking the
/// client exe never starts a local server.
fn exe_is_client_bin() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .map(|stem| {
            let s = stem.to_ascii_lowercase();
            s == "agent-manager-client" || s.ends_with("-client")
        })
        .unwrap_or(false)
}

fn init_logging(
    log_file: &PathBuf,
    log_level: &str,
    want_stdout: bool,
) -> tracing_appender::non_blocking::WorkerGuard {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));

    if let Some(parent) = log_file.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file)
        .unwrap_or_else(|e| {
            let _ = writeln_safe_stderr(&format!(
                "WARN: could not open log file {}: {e}",
                log_file.display()
            ));
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("agent-manager.log")
                .expect("could not open fallback log file")
        });

    let (non_blocking, guard) = tracing_appender::non_blocking(file);

    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_writer(non_blocking);

    let registry = tracing_subscriber::registry().with(filter).with(file_layer);

    if want_stdout {
        let stdout_layer = fmt::layer()
            .with_target(true)
            .with_thread_ids(false)
            .with_writer(std::io::stdout);
        registry.with(stdout_layer).init();
    } else {
        registry.init();
    }

    guard
}

fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Never use println!/eprintln! after detach — broken stdout panics on Windows.
        let _ = writeln_safe_stderr(&format!("PANIC: {info}"));
        error!(panic = %info, "application panic");
        default(info);
    }));
}

/// Write a line to stdout without panicking when the pipe is closed (detached
/// WMI / Start-Process / service-adjacent launches on Windows).
fn writeln_safe_stdout(msg: &str) {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{msg}");
    let _ = out.flush();
}

fn writeln_safe_stderr(msg: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut err = std::io::stderr();
    writeln!(err, "{msg}")?;
    err.flush()
}

/// Allocate a console on Windows when running as a GUI-subsystem binary.
#[cfg(windows)]
fn attach_console_if_requested(requested: bool) {
    if !requested {
        return;
    }
    // SAFETY: process-wide console attach once at startup before logging.
    unsafe {
        type BOOL = i32;
        extern "system" {
            fn AttachConsole(dw_process_id: u32) -> BOOL;
            fn AllocConsole() -> BOOL;
        }
        const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            let _ = AllocConsole();
        }
    }
}

#[cfg(not(windows))]
fn attach_console_if_requested(_requested: bool) {}

/// Prefer agent-manager.config.json; migrate from legacy todo-dashboard.config.json if needed.
fn resolve_config_path(configured: PathBuf) -> PathBuf {
    if configured.exists() {
        return configured;
    }
    // Default path missing — try legacy name next to it
    if let Some(parent) = configured.parent() {
        let legacy = parent.join("todo-dashboard.config.json");
        if legacy.exists() {
            let target = if configured.file_name().is_some() {
                configured.clone()
            } else {
                parent.join("agent-manager.config.json")
            };
            match std::fs::copy(&legacy, &target) {
                Ok(_) => {
                    let _ = writeln_safe_stderr(&format!(
                        "Migrated settings {} → {}",
                        legacy.display(),
                        target.display()
                    ));
                    return target;
                }
                Err(_) => return legacy,
            }
        }
    }
    configured
}

/// Load `.env` before clap so `AGENT_MANAGER_*` vars are available.
/// Does not override variables already set in the process environment.
fn load_dotenv() {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join(".env"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(".env"));
            // target/release/agent-manager.exe → repo root
            if let Some(release_dir) = dir.parent() {
                if let Some(target_dir) = release_dir.parent() {
                    candidates.push(target_dir.join(".env"));
                }
            }
        }
    }
    for path in candidates {
        if path.is_file() && dotenvy::from_path(&path).is_ok() {
            return;
        }
    }
    // Best-effort cwd lookup (no-op if missing)
    let _ = dotenvy::dotenv();
}

/// Resolve scan root: --force-root > --root / AGENT_MANAGER_ROOT > config > portable default.
fn resolve_root(args: &Args, settings: &settings::Settings) -> PathBuf {
    if let Some(fr) = &args.force_root {
        return fr.clone();
    }
    if let Some(r) = &args.root {
        return r.clone();
    }
    if !settings.root.trim().is_empty() {
        return PathBuf::from(settings.root.trim());
    }
    settings::default_scan_root()
}

/// When running as a service, SCM sets cwd to System32 — prefer the install/repo dir.
fn chdir_to_install_dir() {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // Prefer repo root when running from target/release
            let install = if dir.ends_with("release") || dir.ends_with("debug") {
                dir.parent()
                    .and_then(|p| p.parent())
                    .unwrap_or(dir)
                    .to_path_buf()
            } else {
                dir.to_path_buf()
            };
            if let Err(e) = std::env::set_current_dir(&install) {
                let _ = writeln_safe_stderr(&format!(
                    "WARN: could not chdir to {}: {e}",
                    install.display()
                ));
            }
        }
    }
}

/// Make relative config/log paths absolute against the current working directory
/// (after chdir for service mode).
fn absolutize_path(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        return path;
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(&path))
        .unwrap_or(path)
}

fn main() {
    // Service mode must chdir early so relative paths and .env resolve correctly.
    // We only peek argv for --mode service / --service before full parse.
    let early_service = std::env::args().any(|a| a == "--service")
        || std::env::args()
            .collect::<Vec<_>>()
            .windows(2)
            .any(|w| w[0] == "--mode" && w[1] == "service");
    if early_service {
        chdir_to_install_dir();
    }

    load_dotenv();

    let mut args = Args::parse();
    if args.server {
        args.mode = RunMode::Server;
    }
    if args.client {
        args.mode = RunMode::Client;
    }
    if args.service {
        args.mode = RunMode::Service;
    }
    // Dedicated agent-manager-client.exe always runs as thin client (no local server).
    if exe_is_client_bin() {
        args.mode = RunMode::Client;
        // Prefer a separate log file when using the client binary defaults.
        if std::env::var_os("AGENT_MANAGER_LOG_FILE").is_none()
            && !std::env::args().any(|a| a == "--log-file")
            && args.log_file.as_os_str() == "agent-manager.log"
        {
            args.log_file = PathBuf::from("agent-manager-client.log");
        }
    }

    if args.mode == RunMode::Service {
        chdir_to_install_dir();
    }

    args.config = resolve_config_path(absolutize_path(args.config));
    args.log_file = absolutize_path(args.log_file);
    if let Some(ref mut r) = args.root {
        *r = absolutize_path(r.clone());
    }
    if let Some(ref mut fr) = args.force_root {
        *fr = absolutize_path(fr.clone());
    }

    attach_console_if_requested(args.console);

    // Console stdout in: debug builds, server mode (not service), or --console
    let want_stdout = cfg!(debug_assertions)
        || args.mode == RunMode::Server
        || args.console;

    let _log_guard = init_logging(&args.log_file, &args.log_level, want_stdout);
    install_panic_hook();

    // Windows Service: hand off to SCM dispatcher (blocks until stop).
    if args.mode == RunMode::Service {
        #[cfg(windows)]
        {
            info!(
                version = env!("CARGO_PKG_VERSION"),
                log_file = %args.log_file.display(),
                config = %args.config.display(),
                "Agent Manager service starting (SCM dispatcher)"
            );
            if let Err(e) = service::run_service_dispatcher() {
                error!(error = %e, "service dispatcher failed");
                let _ = writeln_safe_stderr(&e);
                std::process::exit(1);
            }
            return;
        }
        #[cfg(not(windows))]
        {
            let _ = writeln_safe_stderr("--mode service is only supported on Windows");
            std::process::exit(1);
        }
    }

    // Client mode: no local scan/server — only a WebView shell + server URL setting.
    if args.mode == RunMode::Client {
        run_client_mode(&args, want_stdout);
        return;
    }

    if args.mode == RunMode::Server {
        if let Err(e) = run_server_headless(None) {
            error!(error = %e, "server failed");
            let _ = writeln_safe_stderr(&e);
            std::process::exit(1);
        }
        return;
    }

    // App mode: local server + desktop window
    if let Err(e) = run_app_mode(&args, want_stdout) {
        error!(error = %e, "app failed");
        let _ = writeln_safe_stderr(&e);
        std::process::exit(1);
    }
}

/// Shared boot for headless HTTP (server mode and Windows Service).
///
/// When `stop_rx` is `Some`, the process shuts down gracefully on receive
/// (service Stop/Shutdown). When `None`, blocks forever after bind.
pub(crate) fn run_server_headless(
    stop_rx: Option<mpsc::Receiver<()>>,
) -> Result<(), String> {
    // Re-parse so service thread sees the same ImagePath args as the process.
    let mut args = Args::parse_from(std::env::args_os());
    if args.server {
        args.mode = RunMode::Server;
    }
    if args.service {
        args.mode = RunMode::Service;
    }
    if args.mode == RunMode::Service {
        chdir_to_install_dir();
    }
    args.config = resolve_config_path(absolutize_path(args.config));
    args.log_file = absolutize_path(args.log_file);

    // Logging may already be initialized from main (server mode). Service mode
    // initializes in main before dispatcher, so skip re-init if already set.
    // tracing subscriber can only init once — main already did it.

    let is_service = args.mode == RunMode::Service || stop_rx.is_some();

    let mut settings = settings::load(&args.config);
    let root = resolve_root(&args, &settings);
    settings.root = root.to_string_lossy().to_string();
    settings.normalize();

    let open_browser = !is_service
        && args.open
        && !args.no_open
        && settings.open_browser_on_start;

    info!(
        version = env!("CARGO_PKG_VERSION"),
        root = %settings.root,
        host = %args.host,
        port = args.port,
        mode = ?args.mode,
        log_file = %args.log_file.display(),
        config = %args.config.display(),
        "starting Agent Manager (headless)"
    );

    let root_path = settings.root_path();
    if let Err(e) = settings::Settings::validate_root(&root_path) {
        return Err(e);
    }

    info!(path = %root_path.display(), "scanning TODOs");
    let started = std::time::Instant::now();
    let root_for_scan = root_path.clone();
    let snapshot = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        scanner::scan_repos(&root_for_scan)
    })) {
        Ok(s) => s,
        Err(_) => return Err("Scan panicked — see log for details".into()),
    };
    info!(
        projects = snapshot.projects.len(),
        open = snapshot.total_open,
        done = snapshot.total_done,
        git_repos = snapshot.git_repo_count,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "scan complete"
    );

    if !args.config.exists() {
        if let Err(e) = settings::save(&args.config, &settings) {
            warn!(error = %e, "could not write initial config");
        }
    }

    let allow_agents = settings::allow_agents_from_env();
    if !allow_agents {
        info!("coding agents disabled for this process (AGENT_MANAGER_ALLOW_AGENTS)");
    }

    let state = Arc::new(server::AppState::new(
        root_path,
        snapshot,
        settings,
        args.config.clone(),
        sessions::SessionManager::new(),
        allow_agents,
    ));

    let bind_host = args.host.trim();
    let addr = format!("{bind_host}:{}", args.port);
    let connect_host = if bind_host == "0.0.0.0" || bind_host == "::" {
        "127.0.0.1"
    } else {
        bind_host
    };
    let url = format!("http://{connect_host}:{}/", args.port);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("agent-mgr")
        .build()
        .map_err(|e| format!("tokio runtime: {e}"))?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let serve_state = state.clone();
    let serve_addr = addr.clone();
    let serve_handle = rt.spawn(async move {
        let shutdown = async move {
            let _ = shutdown_rx.await;
            info!("graceful shutdown signal received");
        };
        if let Err(e) = server::serve_with_shutdown(serve_state, &serve_addr, shutdown).await {
            error!(error = %e, "server exited with error");
        }
    });

    if !desktop::wait_for_server(&url, std::time::Duration::from_secs(30)) {
        let _ = shutdown_tx.send(());
        return Err(format!(
            "Server failed to start on {url} — see {}",
            args.log_file.display()
        ));
    }

    info!(%addr, %url, mode = ?args.mode, "dashboard ready");

    #[cfg(windows)]
    if is_service {
        service::set_running_status();
    }

    if !is_service {
        // Safe prints: detached launches (restart.ps1 / ensure-running.ps1 via WMI)
        // close stdout and `println!` would panic the whole process (os error 232).
        writeln_safe_stdout(&format!("Dashboard listening on {url}"));
        if bind_host != "127.0.0.1" && bind_host != "localhost" {
            writeln_safe_stdout(&format!("  bound on {addr}"));
        }
        writeln_safe_stdout(
            "  (use 127.0.0.1 — not localhost — if the browser fails to connect)",
        );
        writeln_safe_stdout(&format!("Logs: stdout + {}", args.log_file.display()));
        writeln_safe_stdout(&format!("Config: {}", args.config.display()));
        if open_browser {
            if let Err(e) = open::that(&url) {
                warn!(error = %e, "could not open browser");
            }
        }
    }

    // Block until stop (service) or forever (interactive server)
    if let Some(rx) = stop_rx {
        let _ = rx.recv();
        info!("stop requested — shutting down HTTP server");
        let _ = shutdown_tx.send(());
        // Give axum a moment to drain
        let _ = rt.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(10), serve_handle).await
        });
    } else {
        // Keep runtime alive forever (server mode)
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }

    Ok(())
}

fn run_app_mode(args: &Args, want_stdout: bool) -> Result<(), String> {
    let mut settings = settings::load(&args.config);
    let root = resolve_root(args, &settings);
    settings.root = root.to_string_lossy().to_string();
    settings.normalize();

    let open_browser = args.open && !args.no_open && settings.open_browser_on_start;

    info!(
        version = env!("CARGO_PKG_VERSION"),
        root = %settings.root,
        host = %args.host,
        port = args.port,
        ?args.mode,
        log_file = %args.log_file.display(),
        config = %args.config.display(),
        "starting Agent Manager"
    );

    let root_path = settings.root_path();
    settings::Settings::validate_root(&root_path)?;

    info!(path = %root_path.display(), "scanning TODOs");
    let started = std::time::Instant::now();
    let root_for_scan = root_path.clone();
    let snapshot = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        scanner::scan_repos(&root_for_scan)
    })) {
        Ok(s) => s,
        Err(_) => return Err("Scan panicked — see log for details".into()),
    };
    info!(
        projects = snapshot.projects.len(),
        open = snapshot.total_open,
        done = snapshot.total_done,
        git_repos = snapshot.git_repo_count,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "scan complete"
    );

    if !Path::new(&args.config).exists() {
        if let Err(e) = settings::save(&args.config, &settings) {
            warn!(error = %e, "could not write initial config");
        }
    }

    let allow_agents = settings::allow_agents_from_env();
    let state = Arc::new(server::AppState::new(
        root_path,
        snapshot,
        settings,
        args.config.clone(),
        sessions::SessionManager::new(),
        allow_agents,
    ));

    let bind_host = args.host.trim();
    let addr = format!("{bind_host}:{}", args.port);
    let connect_host = if bind_host == "0.0.0.0" || bind_host == "::" {
        "127.0.0.1"
    } else {
        bind_host
    };
    let url = format!("http://{connect_host}:{}/", args.port);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("todo-dash")
        .build()
        .map_err(|e| format!("tokio runtime: {e}"))?;

    let serve_state = state.clone();
    let serve_addr = addr.clone();
    rt.spawn(async move {
        if let Err(e) = server::serve(serve_state, &serve_addr).await {
            error!(error = %e, "server exited with error");
        }
    });

    if !desktop::wait_for_server(&url, std::time::Duration::from_secs(30)) {
        return Err(format!(
            "Server failed to start on {url} — see {}",
            args.log_file.display()
        ));
    }

    info!(%addr, %url, mode = ?args.mode, "dashboard ready");

    if want_stdout {
        writeln_safe_stdout(&format!("Agent Manager (desktop) → {url}"));
        writeln_safe_stdout(&format!("Logs: {}", args.log_file.display()));
    }

    if let Err(e) = desktop::run_main_window(&url, "Agent Manager") {
        error!(error = %e, "desktop window failed");
        warn!("falling back to server mode");
        if open_browser {
            let _ = open::that(&url);
        }
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
    info!("window closed — exiting");
    std::process::exit(0);
}

/// Thin Windows client: resolve server URL (CLI → env → client config), prompt until set.
fn run_client_mode(args: &Args, want_stdout: bool) {
    let client_config_path = args
        .client_config
        .clone()
        .unwrap_or_else(|| client_config::default_path_near(&args.config));

    let mut client_cfg = client_config::load(&client_config_path);

    if let Some(ref url) = args.server_url {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            match client_config::normalize_server_url(trimmed) {
                Ok(u) => client_cfg.server_url = u,
                Err(e) => {
                    warn!(error = %e, "invalid --server-url; will prompt");
                    client_cfg.server_url.clear();
                }
            }
        }
    }

    info!(
        version = env!("CARGO_PKG_VERSION"),
        mode = "client",
        server_url = %client_cfg.server_url,
        client_config = %client_config_path.display(),
        log_file = %args.log_file.display(),
        "starting Agent Manager client"
    );

    if want_stdout {
        writeln_safe_stdout("Agent Manager (client)");
        if client_cfg.has_server_url() && !args.reset_server_url {
            writeln_safe_stdout(&format!("  server: {}", client_cfg.server_url));
        } else {
            writeln_safe_stdout("  server URL not set — setup prompt will open");
        }
        writeln_safe_stdout(&format!(
            "  client config: {}",
            client_config_path.display()
        ));
        writeln_safe_stdout(&format!("Logs: {}", args.log_file.display()));
    }

    let server_url = if client_cfg.has_server_url() {
        Some(client_cfg.server_url.clone())
    } else {
        None
    };

    if let Err(e) = desktop::run_client_window(desktop::ClientWindowOpts {
        config_path: client_config_path,
        server_url,
        title: "Agent Manager".into(),
        force_setup: args.reset_server_url,
    }) {
        error!(error = %e, "client window failed");
        let _ = writeln_safe_stderr(&e);
        std::process::exit(1);
    }
    info!("client window closed — exiting");
    std::process::exit(0);
}
