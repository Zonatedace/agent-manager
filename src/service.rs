//! Windows Service host for Agent Manager (`--mode service`).
//!
//! SCM starts the process with ImagePath args; we register a control handler
//! and run the HTTP server until Stop is requested.

#![cfg(windows)]

use std::ffi::OsString;
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::Duration;
use tracing::{error, info};
use windows_service::define_windows_service;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
    ServiceType,
};
use windows_service::service_control_handler::{
    self, ServiceControlHandlerResult, ServiceStatusHandle,
};
use windows_service::service_dispatcher;

/// Must match the name used by `install-service.ps1` / `sc create`.
pub const SERVICE_NAME: &str = "AgentManager";
pub const SERVICE_DISPLAY_NAME: &str = "Agent Manager";
pub const SERVICE_DESCRIPTION: &str =
    "Agent Manager HTTP API — multi-repo TODOs, agents, and usage meters";

define_windows_service!(ffi_service_main, service_main);

static STATUS_HANDLE: OnceLock<ServiceStatusHandle> = OnceLock::new();

/// Block until the service stops. Call from `main` when `--mode service`.
pub fn run_service_dispatcher() -> Result<(), String> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .map_err(|e| format!("service dispatcher: {e}"))
}

fn service_main(_arguments: Vec<OsString>) {
    if let Err(e) = run_service() {
        eprintln!("Agent Manager service error: {e}");
        error!(error = %e, "service main failed");
    }
}

fn run_service() -> Result<(), String> {
    let (stop_tx, stop_rx) = mpsc::channel::<()>();

    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = stop_tx.send(());
                set_stop_pending();
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)
        .map_err(|e| format!("register service control handler: {e}"))?;

    let _ = STATUS_HANDLE.set(status_handle.clone());

    // Lengthy scan: tell SCM we need time to reach Running
    status_handle
        .set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: ServiceState::StartPending,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 1,
            wait_hint: Duration::from_secs(60),
            process_id: None,
        })
        .map_err(|e| format!("set StartPending: {e}"))?;

    let boot_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::run_server_headless(Some(stop_rx))
    }));

    let exit_code = match boot_result {
        Ok(Ok(())) => {
            info!("service server exited cleanly");
            ServiceExitCode::Win32(0)
        }
        Ok(Err(e)) => {
            error!(error = %e, "service server failed");
            ServiceExitCode::ServiceSpecific(1)
        }
        Err(panic) => {
            let msg = panic
                .downcast_ref::<String>()
                .map(|s| s.as_str())
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap_or("panic");
            error!(panic = %msg, "service panicked");
            ServiceExitCode::ServiceSpecific(2)
        }
    };

    let _ = status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code,
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    });

    Ok(())
}

/// After HTTP is bound and health-ready, report Running to SCM.
pub fn set_running_status() {
    if let Some(h) = STATUS_HANDLE.get() {
        if let Err(e) = h.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: ServiceState::Running,
            controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        }) {
            error!(error = %e, "failed to set service Running");
        } else {
            info!("service status = Running");
        }
    }
}

fn set_stop_pending() {
    if let Some(h) = STATUS_HANDLE.get() {
        let _ = h.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: ServiceState::StopPending,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 1,
            wait_hint: Duration::from_secs(15),
            process_id: None,
        });
    }
}
