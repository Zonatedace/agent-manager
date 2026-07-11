// Hide the console window in Windows release builds (file logging still works).
// Debug builds keep a console for tracing. Override with --console.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod agents;
mod desktop;
mod fsbrowser;
mod git;
mod models;
mod parser;
mod scanner;
mod server;
mod sessions;
mod settings;
mod todos;
mod usage;

use clap::{Parser, ValueEnum};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum RunMode {
    /// Native desktop window (default on Windows)
    App,
    /// HTTP server only — no window (browser optional)
    Server,
}

#[derive(Parser, Debug)]
#[command(
    name = "agent-manager",
    about = "Agent Manager — local multi-repo TODO app with agents and usage meters",
    long_about = "Windows desktop app (WebView2) by default. Use --mode server for headless HTTP only."
)]
struct Args {
    /// Root directory containing project repos
    #[arg(
        short,
        long,
        default_value = r"C:\Users\Brandon\Desktop\Repos"
    )]
    root: PathBuf,

    /// HTTP port (loopback only)
    #[arg(short, long, default_value_t = 7878)]
    port: u16,

    /// Run mode: app = native window, server = HTTP only
    #[arg(long, value_enum, default_value_t = default_run_mode())]
    mode: RunMode,

    /// Shorthand for --mode server
    #[arg(long, default_value_t = false)]
    server: bool,

    /// Attach a console on Windows release builds (for debugging)
    #[arg(long, default_value_t = false)]
    console: bool,

    /// Open an external browser (server mode only; ignored in app mode)
    #[arg(long, default_value_t = true)]
    open: bool,

    /// Do not open an external browser
    #[arg(long, default_value_t = false)]
    no_open: bool,

    /// Log file path
    #[arg(long, default_value = "agent-manager.log")]
    log_file: PathBuf,

    /// Log level filter (e.g. info, debug, agent_manager=debug)
    #[arg(long, default_value = "info")]
    log_level: String,

    /// Path to settings JSON
    #[arg(long, default_value = "agent-manager.config.json")]
    config: PathBuf,

    /// Ignore saved config root and force this root
    #[arg(long)]
    force_root: Option<PathBuf>,
}

fn default_run_mode() -> RunMode {
    if cfg!(windows) {
        RunMode::App
    } else {
        RunMode::Server
    }
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
            eprintln!("WARN: could not open log file {}: {e}", log_file.display());
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
        eprintln!("PANIC: {info}");
        error!(panic = %info, "application panic");
        default(info);
    }));
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
                    eprintln!(
                        "Migrated settings {} → {}",
                        legacy.display(),
                        target.display()
                    );
                    return target;
                }
                Err(_) => return legacy,
            }
        }
    }
    configured
}

fn main() {
    let mut args = Args::parse();
    if args.server {
        args.mode = RunMode::Server;
    }
    args.config = resolve_config_path(args.config);

    attach_console_if_requested(args.console);

    // Console stdout in: debug builds, server mode, or --console
    let want_stdout =
        cfg!(debug_assertions) || args.mode == RunMode::Server || args.console;

    let _log_guard = init_logging(&args.log_file, &args.log_level, want_stdout);
    install_panic_hook();

    let mut settings = settings::load(&args.config);
    if let Some(fr) = args.force_root.clone() {
        settings.root = fr.to_string_lossy().to_string();
    } else if settings.root.is_empty() {
        settings.root = args.root.to_string_lossy().to_string();
    } else {
        let default_root = PathBuf::from(r"C:\Users\Brandon\Desktop\Repos");
        if args.root != default_root && args.root != PathBuf::from(&settings.root) {
            settings.root = args.root.to_string_lossy().to_string();
        }
    }
    settings.normalize();

    let open_browser = args.open && !args.no_open && settings.open_browser_on_start;

    info!(
        version = env!("CARGO_PKG_VERSION"),
        root = %settings.root,
        port = args.port,
        ?args.mode,
        log_file = %args.log_file.display(),
        config = %args.config.display(),
        "starting Agent Manager"
    );

    let root_path = settings.root_path();
    if let Err(e) = settings::Settings::validate_root(&root_path) {
        error!(error = %e, "invalid root");
        eprintln!("{e}");
        std::process::exit(1);
    }

    info!(path = %root_path.display(), "scanning TODOs");
    let started = std::time::Instant::now();
    let root_for_scan = root_path.clone();
    let snapshot = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        scanner::scan_repos(&root_for_scan)
    })) {
        Ok(s) => s,
        Err(e) => {
            error!(?e, "scan panicked");
            eprintln!("Scan panicked — see log for details");
            std::process::exit(1);
        }
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

    let state = Arc::new(server::AppState::new(
        root_path,
        snapshot,
        settings,
        args.config.clone(),
        sessions::SessionManager::new(),
    ));

    let addr = format!("127.0.0.1:{}", args.port);
    let url = format!("http://127.0.0.1:{}/", args.port);

    // Dedicated multi-thread runtime so the UI thread can block on the native window.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("todo-dash")
        .build()
        .expect("tokio runtime");

    let serve_state = state.clone();
    let serve_addr = addr.clone();
    rt.spawn(async move {
        if let Err(e) = server::serve(serve_state, &serve_addr).await {
            error!(error = %e, "server exited with error");
        }
    });

    if !desktop::wait_for_server(&url, std::time::Duration::from_secs(30)) {
        eprintln!("Server failed to start on {url} — see {}", args.log_file.display());
        std::process::exit(1);
    }

    info!(%addr, %url, mode = ?args.mode, "dashboard ready");

    match args.mode {
        RunMode::App => {
            if want_stdout {
                println!("Agent Manager (desktop) → {url}");
                println!("Logs: {}", args.log_file.display());
            }
            // Block until window closes
            if let Err(e) = desktop::run_main_window(&url, "Agent Manager") {
                error!(error = %e, "desktop window failed");
                eprintln!("{e}");
                // Fall back to server + browser so the user is not stuck
                warn!("falling back to server mode");
                if open_browser {
                    let _ = open::that(&url);
                }
                // Keep process alive serving HTTP
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(3600));
                }
            }
            info!("window closed — exiting");
            // Force-exit so background tokio workers / sessions don't hang the process
            std::process::exit(0);
        }
        RunMode::Server => {
            println!("Dashboard listening on {url}");
            println!("  (use 127.0.0.1 — not localhost — if the browser fails to connect)");
            println!("Logs: stdout + {}", args.log_file.display());
            println!("Config: {}", args.config.display());
            if open_browser {
                if let Err(e) = open::that(&url) {
                    warn!(error = %e, "could not open browser");
                }
            }
            // Block forever (server runs on runtime)
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        }
    }
}
