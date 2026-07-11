mod agents;
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

use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[derive(Parser, Debug)]
#[command(
    name = "todo-dashboard",
    about = "Scan project TODO.md files and serve a local dashboard"
)]
struct Args {
    /// Root directory containing project repos
    #[arg(
        short,
        long,
        default_value = r"C:\Users\Brandon\Desktop\Repos"
    )]
    root: PathBuf,

    /// HTTP port
    #[arg(short, long, default_value_t = 7878)]
    port: u16,

    /// Open the browser automatically
    #[arg(long, default_value_t = true)]
    open: bool,

    /// Do not open the browser
    #[arg(long, default_value_t = false)]
    no_open: bool,

    /// Log file path (also always logs to stdout)
    #[arg(long, default_value = "todo-dashboard.log")]
    log_file: PathBuf,

    /// Log level filter (e.g. info, debug, todo_dashboard=debug)
    #[arg(long, default_value = "info")]
    log_level: String,

    /// Path to settings JSON (persists root folder and preferences)
    #[arg(long, default_value = "todo-dashboard.config.json")]
    config: PathBuf,

    /// Ignore saved config root and force this root (one-shot override)
    #[arg(long)]
    force_root: Option<PathBuf>,
}

fn init_logging(log_file: &PathBuf, log_level: &str) -> tracing_appender::non_blocking::WorkerGuard {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(log_level));

    // Ensure parent dir exists for log file
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
            // Fallback: open a local default
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("todo-dashboard.log")
                .expect("could not open fallback log file")
        });

    let (non_blocking, guard) = tracing_appender::non_blocking(file);

    let stdout_layer = fmt::layer()
        .with_target(true)
        .with_thread_ids(false)
        .with_writer(std::io::stdout);

    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_writer(non_blocking);

    tracing_subscriber::registry()
        .with(filter)
        .with(stdout_layer)
        .with(file_layer)
        .init();

    guard
}

fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Always print to stderr even if tracing fails
        eprintln!("PANIC: {info}");
        error!(panic = %info, "application panic");
        default(info);
    }));
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let open_browser = args.open && !args.no_open;

    // Keep guard alive for the process lifetime so file logs flush
    let _log_guard = init_logging(&args.log_file, &args.log_level);
    install_panic_hook();

    let mut settings = settings::load(&args.config);
    // CLI --root always applied when using default flow; --force-root wins; else config root
    if let Some(fr) = args.force_root.clone() {
        settings.root = fr.to_string_lossy().to_string();
    } else if settings.root.is_empty() {
        settings.root = args.root.to_string_lossy().to_string();
    } else if args.root.to_string_lossy() != r"C:\Users\Brandon\Desktop\Repos"
        && args.root != PathBuf::from(&settings.root)
    {
        // Explicit non-default --root on CLI overrides config for this run and is saved later only if user uses settings UI
        // If user passed --root different from default, prefer it
        let default_root = PathBuf::from(r"C:\Users\Brandon\Desktop\Repos");
        if args.root != default_root {
            settings.root = args.root.to_string_lossy().to_string();
        }
    }
    settings.normalize();

    // Prefer settings open_browser unless --no-open
    let open_browser = open_browser && settings.open_browser_on_start;

    info!(
        version = env!("CARGO_PKG_VERSION"),
        root = %settings.root,
        port = args.port,
        log_file = %args.log_file.display(),
        config = %args.config.display(),
        "starting todo-dashboard"
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

    // Persist config on first run so UI edits have a file to update
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
    // Bind IPv4 loopback explicitly. Prefer opening http://127.0.0.1 (not localhost)
    // so browsers don't try IPv6 ::1 first and get "connection refused".
    let addr = format!("127.0.0.1:{}", args.port);
    let url = format!("http://127.0.0.1:{}/", args.port);

    info!(%addr, %url, "dashboard listening");
    println!("Dashboard listening on {url}");
    println!("  (use 127.0.0.1 — not localhost — if the browser fails to connect)");
    println!("Logs: stdout + {}", args.log_file.display());
    println!("Config: {}", args.config.display());

    if open_browser {
        if let Err(e) = open::that(&url) {
            warn!(error = %e, "could not open browser");
        }
    }

    if let Err(e) = server::serve(state, &addr).await {
        error!(error = %e, "server exited with error");
        eprintln!("Server error: {e}");
        std::process::exit(1);
    }
}
