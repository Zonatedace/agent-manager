use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct FsEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_git: bool,
    pub has_todo: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FsListing {
    pub path: String,
    pub parent: Option<String>,
    pub entries: Vec<FsEntry>,
    /// True when listing drive roots (Windows) or filesystem roots
    pub at_system_root: bool,
}

fn entry_flags(path: &Path) -> (bool, bool) {
    let is_git = path.join(".git").exists();
    let has_todo = ["TODO.md", "todo.md", "TODOS.md", "todos.md"]
        .iter()
        .any(|n| path.join(n).is_file());
    (is_git, has_todo)
}

/// List directory contents. Empty path / "drives" lists Windows drives (or `/` on Unix).
pub fn list_dir(path: &str) -> Result<FsListing, String> {
    let path = path.trim();
    if path.is_empty() || path.eq_ignore_ascii_case("drives") || path == "/" && cfg!(windows) {
        return list_system_roots();
    }

    let p = PathBuf::from(path);
    if !p.exists() {
        return Err(format!("path does not exist: {path}"));
    }
    if !p.is_dir() {
        return Err(format!("not a directory: {path}"));
    }

    let mut entries = Vec::new();
    let rd = fs::read_dir(&p).map_err(|e| format!("read_dir {}: {e}", p.display()))?;
    for ent in rd.flatten() {
        let meta = match ent.file_type() {
            Ok(m) => m,
            Err(_) => continue,
        };
        // Only show directories in the folder picker (repos are folders)
        if !meta.is_dir() {
            continue;
        }
        let name = ent.file_name().to_string_lossy().to_string();
        // Skip heavy / internal noise in the picker
        if matches!(
            name.as_str(),
            "node_modules"
                | "target"
                | "dist"
                | "build"
                | "__pycache__"
                | ".git"
                | ".venv"
                | "venv"
        ) {
            continue;
        }

        let full = ent.path();
        let (is_git, has_todo) = entry_flags(&full);
        entries.push(FsEntry {
            name,
            path: full.to_string_lossy().to_string(),
            is_dir: true,
            is_git,
            has_todo,
        });
    }

    entries.sort_by(|a, b| {
        // Prefer git / todo folders first, then alpha
        b.is_git
            .cmp(&a.is_git)
            .then(b.has_todo.cmp(&a.has_todo))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let parent = p.parent().map(|x| x.to_string_lossy().to_string());
    // On Windows, parent of `C:\` is None — treat as system root next
    let parent = if parent.as_ref().map(|s| s.is_empty()).unwrap_or(true) {
        None
    } else {
        parent
    };

    Ok(FsListing {
        path: p.to_string_lossy().to_string(),
        parent,
        entries,
        at_system_root: false,
    })
}

#[cfg(windows)]
fn list_system_roots() -> Result<FsListing, String> {
    let mut entries = Vec::new();
    for letter in b'A'..=b'Z' {
        let drive = format!("{}:\\", letter as char);
        let p = PathBuf::from(&drive);
        if p.is_dir() {
            entries.push(FsEntry {
                name: drive.clone(),
                path: drive,
                is_dir: true,
                is_git: false,
                has_todo: false,
            });
        }
    }
    Ok(FsListing {
        path: "Drives".into(),
        parent: None,
        entries,
        at_system_root: true,
    })
}

#[cfg(not(windows))]
fn list_system_roots() -> Result<FsListing, String> {
    list_dir("/")
}

/// Resolve and validate a working directory path.
pub fn resolve_cwd(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path.trim());
    if path.trim().is_empty() {
        return Err("working directory is empty".into());
    }
    let canon = p.canonicalize().unwrap_or(p.clone());
    if !canon.exists() {
        return Err(format!("path does not exist: {}", canon.display()));
    }
    if !canon.is_dir() {
        return Err(format!("not a directory: {}", canon.display()));
    }
    Ok(canon)
}
