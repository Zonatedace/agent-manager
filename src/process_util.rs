//! Helpers for spawning child processes without UI side effects.
//!
//! On Windows, a GUI app (`windows_subsystem = "windows"`) that runs console
//! tools like `git`, `where`, or agent CLIs will briefly flash a black terminal
//! window for every spawn unless `CREATE_NO_WINDOW` is set.

use std::process::Command;

/// Flag: do not allocate a console for this child (Windows).
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Configure a command that should run in the background with no visible console.
///
/// Call this for every capture/headless spawn (`output()`, piped sessions, etc.).
/// Do **not** use this for intentional interactive terminals (`open_terminal`,
/// external agent in a new console window).
pub fn hide_console(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = cmd;
}
