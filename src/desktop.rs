//! Native desktop window shell (Windows WebView2 via wry/tao).
//!
//! - **App mode**: local Axum server + window pointed at loopback.
//! - **Client mode**: thin window that connects to a configured server URL.
//!   If no server URL is set, a setup page prompts until the user provides one.

use std::path::{Path, PathBuf};
use tracing::{error, info, warn};

use crate::client_config::{self, ClientConfig};

/// Block the calling thread with a native window until the user closes it.
/// Returns when the window is closed (caller should then exit the process).
#[cfg(windows)]
pub fn run_main_window(url: &str, title: &str) -> Result<(), String> {
    use tao::{
        dpi::LogicalSize,
        event::{Event, WindowEvent},
        event_loop::{ControlFlow, EventLoop},
        window::WindowBuilder,
    };
    use wry::WebViewBuilder;

    info!(%url, "opening desktop window (WebView2)");

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title(title)
        .with_inner_size(LogicalSize::new(1360.0, 900.0))
        .with_min_inner_size(LogicalSize::new(900.0, 600.0))
        .with_resizable(true)
        .build(&event_loop)
        .map_err(|e| format!("window: {e}"))?;

    let _webview = WebViewBuilder::new()
        .with_url(url)
        .with_devtools(cfg!(debug_assertions))
        .build(&window)
        .map_err(|e| {
            format!(
                "WebView2 failed to start ({e}). Install the Microsoft Edge WebView2 Runtime \
                 from https://developer.microsoft.com/microsoft-edge/webview2/ if needed."
            )
        })?;

    let mut exit_logged = false;
    // tao 0.32: run(event, target, control_flow) — never returns.
    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if !exit_logged {
                    info!("window close requested — shutting down");
                    exit_logged = true;
                }
                *control_flow = ControlFlow::Exit;
            }
            Event::LoopDestroyed => {
                info!("desktop event loop destroyed");
            }
            _ => {}
        }
    });
}

#[cfg(not(windows))]
pub fn run_main_window(url: &str, title: &str) -> Result<(), String> {
    let _ = (url, title);
    Err("desktop window mode is only supported on Windows".into())
}

/// Options for the thin client window (connects to a remote/local server URL).
pub struct ClientWindowOpts {
    /// Path to `agent-manager.client.json`
    pub config_path: PathBuf,
    /// Resolved server URL if already known (may still fail health check).
    pub server_url: Option<String>,
    pub title: String,
    /// When true, always show the setup form first (e.g. --reset-server-url).
    pub force_setup: bool,
}

/// Thin client: prompt for server URL until set, then navigate to the dashboard.
/// Does not start a local HTTP server.
#[cfg(windows)]
pub fn run_client_window(opts: ClientWindowOpts) -> Result<(), String> {
    use tao::{
        dpi::LogicalSize,
        event::{Event, WindowEvent},
        event_loop::{ControlFlow, EventLoopBuilder},
        window::WindowBuilder,
    };
    use wry::{http::Request, WebViewBuilder};

    #[derive(Debug)]
    enum ClientEvent {
        /// User submitted a server URL from the setup page.
        Connect(String),
        /// Re-open the setup form (e.g. change server).
        ShowSetup { prefill: String, error: String },
    }

    let config_path = opts.config_path;
    let title = opts.title;

    let initial_url = opts
        .server_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| client_config::normalize_server_url(s).ok());

    let mut startup_error = String::new();
    let show_setup = if opts.force_setup {
        true
    } else if let Some(ref url) = initial_url {
        // Probe once so a stale URL re-prompts instead of a blank/error page.
        match client_config::probe_server(url, std::time::Duration::from_secs(3)) {
            Ok(()) => false,
            Err(e) => {
                warn!(%url, error = %e, "saved server URL not reachable; prompting");
                startup_error = e;
                true
            }
        }
    } else {
        info!("no server URL configured — prompting user");
        true
    };

    info!(
        show_setup,
        has_url = initial_url.is_some(),
        config = %config_path.display(),
        "opening client window (WebView2)"
    );

    let event_loop = EventLoopBuilder::<ClientEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window = WindowBuilder::new()
        .with_title(&title)
        .with_inner_size(LogicalSize::new(1360.0, 900.0))
        .with_min_inner_size(LogicalSize::new(720.0, 520.0))
        .with_resizable(true)
        .build(&event_loop)
        .map_err(|e| format!("window: {e}"))?;

    let ipc_proxy = proxy.clone();
    let handler = move |req: Request<String>| {
        let body = req.body().trim();
        // Shared dashboard (index.html) can request setup via window.ipc.postMessage.
        // Browser has no ipc — same HTML works for web and Windows client.
        if let Some(rest) = body.strip_prefix("connect:") {
            let _ = ipc_proxy.send_event(ClientEvent::Connect(rest.to_string()));
        } else if body == "show-setup" {
            let _ = ipc_proxy.send_event(ClientEvent::ShowSetup {
                prefill: String::new(),
                error: String::new(),
            });
        } else if let Some(rest) = body.strip_prefix("show-setup:") {
            let _ = ipc_proxy.send_event(ClientEvent::ShowSetup {
                prefill: rest.to_string(),
                error: String::new(),
            });
        }
    };

    let mut builder = WebViewBuilder::new()
        .with_devtools(cfg!(debug_assertions))
        .with_ipc_handler(handler);

    if show_setup {
        let prefill = initial_url.clone().unwrap_or_default();
        builder = builder.with_html(setup_html(&prefill, &startup_error));
        window.set_title(&format!("{title} — Server URL"));
    } else {
        let url = initial_url.clone().unwrap();
        builder = builder.with_url(&url);
        window.set_title(&format!("{title} — {url}"));
    }

    let webview = builder.build(&window).map_err(|e| {
        format!(
            "WebView2 failed to start ({e}). Install the Microsoft Edge WebView2 Runtime \
             from https://developer.microsoft.com/microsoft-edge/webview2/ if needed."
        )
    })?;

    let mut exit_logged = false;

    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if !exit_logged {
                    info!("client window close requested — shutting down");
                    exit_logged = true;
                }
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(ClientEvent::Connect(raw)) => {
                match try_connect_and_save(&raw, &config_path) {
                    Ok(url) => {
                        info!(%url, "connecting client to server");
                        if let Err(e) = webview.load_url(&url) {
                            error!(error = %e, "load_url failed");
                            show_setup_page(
                                &webview,
                                &window,
                                &title,
                                &url,
                                &format!("Failed to navigate: {e}"),
                            );
                        } else {
                            window.set_title(&format!("{title} — {url}"));
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "connect rejected");
                        show_setup_page(&webview, &window, &title, raw.trim(), &e);
                    }
                }
            }
            Event::UserEvent(ClientEvent::ShowSetup { prefill, error }) => {
                show_setup_page(&webview, &window, &title, &prefill, &error);
            }
            Event::LoopDestroyed => {
                info!("client event loop destroyed");
            }
            _ => {}
        }
    });
}

#[cfg(windows)]
fn try_connect_and_save(raw: &str, config_path: &Path) -> Result<String, String> {
    let url = client_config::normalize_server_url(raw)?;
    // Require a live server before persisting so a typo does not stick permanently
    // without re-prompt (failed probe still re-shows setup with the attempted URL).
    client_config::probe_server(&url, std::time::Duration::from_secs(5))?;
    let mut cfg = ClientConfig {
        server_url: url.clone(),
    };
    cfg.normalize();
    client_config::save(config_path, &cfg)?;
    Ok(url)
}

#[cfg(windows)]
fn show_setup_page(
    webview: &wry::WebView,
    window: &tao::window::Window,
    title: &str,
    prefill: &str,
    error: &str,
) {
    window.set_title(&format!("{title} — Server URL"));
    if let Err(e) = webview.load_html(&setup_html(prefill, error)) {
        error!(error = %e, "failed to show setup page");
    }
}

#[cfg(not(windows))]
pub fn run_client_window(_opts: ClientWindowOpts) -> Result<(), String> {
    Err("client window mode is only supported on Windows".into())
}

/// Escape text for safe embedding in HTML attribute/text contexts.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn setup_html(prefill: &str, error: &str) -> String {
    let prefill = html_escape(prefill);
    let error_block = if error.is_empty() {
        String::new()
    } else {
        format!(
            r#"<div class="err" id="err">{}</div>"#,
            html_escape(error)
        )
    };
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Agent Manager — Server URL</title>
  <style>
    :root {{
      color-scheme: dark;
      --bg: #0f1419;
      --card: #1a2332;
      --border: #2d3a4d;
      --text: #e7ecf3;
      --muted: #8b9bb4;
      --accent: #3b82f6;
      --accent-hover: #2563eb;
      --danger: #f87171;
      --danger-bg: rgba(248, 113, 113, 0.12);
    }}
    * {{ box-sizing: border-box; }}
    body {{
      margin: 0;
      min-height: 100vh;
      font-family: "Segoe UI", system-ui, -apple-system, sans-serif;
      background: radial-gradient(1200px 600px at 20% -10%, #1e3a5f 0%, transparent 50%),
                  radial-gradient(900px 500px at 100% 100%, #1a2f1a 0%, transparent 45%),
                  var(--bg);
      color: var(--text);
      display: flex;
      align-items: center;
      justify-content: center;
      padding: 1.5rem;
    }}
    .card {{
      width: min(480px, 100%);
      background: var(--card);
      border: 1px solid var(--border);
      border-radius: 14px;
      padding: 1.75rem 1.5rem 1.5rem;
      box-shadow: 0 16px 48px rgba(0,0,0,0.35);
    }}
    .logo {{
      font-size: 0.75rem;
      font-weight: 600;
      letter-spacing: 0.08em;
      text-transform: uppercase;
      color: var(--muted);
      margin-bottom: 0.5rem;
    }}
    h1 {{
      margin: 0 0 0.5rem;
      font-size: 1.35rem;
      font-weight: 650;
    }}
    p {{
      margin: 0 0 1.25rem;
      color: var(--muted);
      font-size: 0.95rem;
      line-height: 1.45;
    }}
    label {{
      display: block;
      font-size: 0.8rem;
      font-weight: 600;
      color: var(--muted);
      margin-bottom: 0.4rem;
    }}
    input[type="url"], input[type="text"] {{
      width: 100%;
      padding: 0.7rem 0.85rem;
      border-radius: 8px;
      border: 1px solid var(--border);
      background: #0c1118;
      color: var(--text);
      font-size: 0.95rem;
      outline: none;
    }}
    input:focus {{
      border-color: var(--accent);
      box-shadow: 0 0 0 3px rgba(59, 130, 246, 0.25);
    }}
    .err {{
      margin: 0.85rem 0 0;
      padding: 0.65rem 0.75rem;
      border-radius: 8px;
      background: var(--danger-bg);
      color: var(--danger);
      font-size: 0.88rem;
      line-height: 1.35;
    }}
    .actions {{
      margin-top: 1.15rem;
      display: flex;
      gap: 0.6rem;
      flex-wrap: wrap;
    }}
    button {{
      appearance: none;
      border: none;
      border-radius: 8px;
      padding: 0.65rem 1.1rem;
      font-size: 0.92rem;
      font-weight: 600;
      cursor: pointer;
      font-family: inherit;
    }}
    button.primary {{
      background: var(--accent);
      color: #fff;
    }}
    button.primary:hover {{ background: var(--accent-hover); }}
    button.primary:disabled {{
      opacity: 0.55;
      cursor: not-allowed;
    }}
    .hint {{
      margin-top: 1.1rem;
      font-size: 0.8rem;
      color: var(--muted);
      line-height: 1.4;
    }}
    code {{
      font-family: ui-monospace, Consolas, monospace;
      font-size: 0.85em;
      background: #0c1118;
      padding: 0.1em 0.35em;
      border-radius: 4px;
    }}
  </style>
</head>
<body>
  <div class="card">
    <div class="logo">Agent Manager</div>
    <h1>Connect to server</h1>
    <p>
      Enter the URL of a running Agent Manager server. This is required before
      the Windows client can open the dashboard.
    </p>
    <form id="form" autocomplete="on">
      <label for="url">Server URL</label>
      <input
        id="url"
        name="server_url"
        type="url"
        inputmode="url"
        spellcheck="false"
        placeholder="http://127.0.0.1:7878/"
        value="{prefill}"
        required
        autofocus
      />
      {error_block}
      <div class="actions">
        <button type="submit" class="primary" id="btn">Connect</button>
      </div>
    </form>
    <p class="hint">
      Start the server with <code>agent-manager --mode server</code>, then use
      something like <code>http://127.0.0.1:7878/</code>. The URL is saved in
      <code>agent-manager.client.json</code>.
    </p>
  </div>
  <script>
    const form = document.getElementById("form");
    const input = document.getElementById("url");
    const btn = document.getElementById("btn");
    function setBusy(busy) {{
      btn.disabled = busy;
      btn.textContent = busy ? "Connecting…" : "Connect";
    }}
    form.addEventListener("submit", (e) => {{
      e.preventDefault();
      const v = (input.value || "").trim();
      if (!v) {{
        let err = document.getElementById("err");
        if (!err) {{
          err = document.createElement("div");
          err.className = "err";
          err.id = "err";
          form.insertBefore(err, form.querySelector(".actions"));
        }}
        err.textContent = "Server URL is required.";
        input.focus();
        return;
      }}
      setBusy(true);
      try {{
        window.ipc.postMessage("connect:" + v);
      }} catch (err) {{
        setBusy(false);
        alert("IPC unavailable: " + err);
      }}
    }});
    input.addEventListener("input", () => {{
      const err = document.getElementById("err");
      if (err) err.remove();
    }});
    input.focus();
    input.select();
  </script>
</body>
</html>"#
    )
}

/// Wait until the local HTTP server answers /api/health (or timeout).
pub fn wait_for_server(url_base: &str, timeout: std::time::Duration) -> bool {
    let health = client_config::health_url(url_base);
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        match ureq::get(&health)
            .timeout(std::time::Duration::from_secs(1))
            .call()
        {
            Ok(resp) if (200..300).contains(&resp.status()) => {
                info!(elapsed_ms = start.elapsed().as_millis() as u64, "server ready");
                return true;
            }
            Ok(resp) => {
                warn!(status = resp.status(), "health not ready");
            }
            Err(e) => {
                // Connection refused while server boots is normal
                let _ = e;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    error!(%health, "server did not become ready in time");
    false
}
