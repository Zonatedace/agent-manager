//! Native desktop window shell (Windows WebView2 via wry/tao).
//!
//! The Axum server still runs locally; this module opens an OS window
//! pointed at http://127.0.0.1:<port>/ instead of an external browser.

use tracing::{error, info, warn};

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

/// Wait until the local HTTP server answers /api/health (or timeout).
pub fn wait_for_server(url_base: &str, timeout: std::time::Duration) -> bool {
    let health = format!(
        "{}api/health",
        if url_base.ends_with('/') {
            url_base.to_string()
        } else {
            format!("{url_base}/")
        }
    );
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        match ureq::get(&health).timeout(std::time::Duration::from_secs(1)).call() {
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
