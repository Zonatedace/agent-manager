use crate::git;
use crate::models::{ProjectTodos, Snapshot};
use crate::parser;
use chrono::{DateTime, Utc};
use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use walkdir::WalkDir;

/// Directory names we never descend into while looking for TODO files.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    "dist",
    "build",
    ".next",
    "vendor",
    "__pycache__",
    ".venv",
    "venv",
    "obj",
    "bin",
    ".cache",
    "coverage",
    "mcps",
    "agent-tools",
    "terminals",
    "packages",
    ".turbo",
    ".cargo",
    ".kilo",
    ".claude",
    ".grok",
    "worktrees",
];

const TODO_NAMES: &[&str] = &["TODO.md", "todo.md", "TODOS.md", "todos.md"];

fn is_skip_dir(name: &str) -> bool {
    SKIP_DIRS.iter().any(|s| s.eq_ignore_ascii_case(name))
}

fn is_todo_filename(name: &str) -> bool {
    TODO_NAMES.iter().any(|s| s.eq_ignore_ascii_case(name))
}

fn system_time_to_utc(t: SystemTime) -> Option<DateTime<Utc>> {
    let duration = t.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    DateTime::from_timestamp(duration.as_secs() as i64, duration.subsec_nanos())
}

/// Collect candidate TODO.md paths under `root`, skipping heavy dirs.
fn find_todo_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();

    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.file_type().is_dir() {
                let name = e.file_name().to_string_lossy();
                // Always allow the root itself
                if e.depth() == 0 {
                    return true;
                }
                return !is_skip_dir(&name);
            }
            true
        });

    for entry in walker.filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if is_todo_filename(&name) {
            files.push(entry.path().to_path_buf());
        }
    }

    files
}

fn project_id_from_path(root: &Path, todo_file: &Path) -> String {
    let parent = todo_file.parent().unwrap_or(todo_file);
    parent
        .strip_prefix(root)
        .unwrap_or(parent)
        .to_string_lossy()
        .replace('\\', "/")
        .trim_matches('/')
        .to_string()
}

fn project_name_from_id(id: &str) -> String {
    if id.is_empty() {
        return "root".to_string();
    }
    // Use the full relative path so nested TODOs stay distinguishable (e.g. k3s/docs)
    id.to_string()
}

fn load_project(root: &Path, todo_file: &Path) -> Option<ProjectTodos> {
    let content = fs::read_to_string(todo_file).ok()?;
    let items = parser::parse_todos(&content);
    // Skip empty TODO files (no checkboxes)
    if items.is_empty() {
        return None;
    }

    let id = project_id_from_path(root, todo_file);
    let name = project_name_from_id(&id);
    let open = items.iter().filter(|i| !i.done).count();
    let done = items.iter().filter(|i| i.done).count();
    let modified = fs::metadata(todo_file)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(system_time_to_utc);

    let project_path = todo_file.parent().unwrap_or(todo_file);
    let git_info = git::collect_git_info(project_path, root);

    Some(ProjectTodos {
        id,
        name,
        path: project_path.to_string_lossy().to_string(),
        todo_file: todo_file.to_string_lossy().to_string(),
        open,
        done,
        total: items.len(),
        items,
        modified,
        git: git_info,
    })
}

/// Prefer root-level TODO.md over nested duplicates when both exist for the same project.
/// Strategy: keep all files; if two share the same top-level project folder and one is
/// deeper, we keep both as separate project ids (e.g. k3s and k3s/docs) — that's useful.
pub fn scan_repos(root: &Path) -> Snapshot {
    let files = find_todo_files(root);

    let mut projects: Vec<ProjectTodos> = files
        .par_iter()
        .filter_map(|f| load_project(root, f))
        .collect();

    projects.sort_by(|a, b| {
        b.open
            .cmp(&a.open)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let total_open: usize = projects.iter().map(|p| p.open).sum();
    let total_done: usize = projects.iter().map(|p| p.done).sum();
    let total_items: usize = projects.iter().map(|p| p.total).sum();
    let git_repo_count = projects.iter().filter(|p| p.git.is_some()).count();

    Snapshot {
        scanned_at: Utc::now(),
        root: root.to_string_lossy().to_string(),
        projects,
        total_open,
        total_done,
        total_items,
        git_repo_count,
    }
}
