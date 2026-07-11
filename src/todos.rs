use crate::git;
use crate::models::{ProjectTodos, TodoItem};
use crate::parser;
use regex::Regex;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

fn checkbox_line_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Capture indent, bullet, checkbox mark, rest of line
        Regex::new(r"(?i)^(\s*(?:[-*+]|\d+\.)\s+)\[([ xX])\](\s+.*)$").unwrap()
    })
}

/// Toggle or set the checkbox state on a specific 1-based line of a markdown file.
/// Returns the new done state and updated item text.
pub fn set_todo_done(todo_file: &Path, line: usize, done: bool) -> Result<(bool, String), String> {
    if line == 0 {
        return Err("line must be 1-based".into());
    }
    let content = fs::read_to_string(todo_file)
        .map_err(|e| format!("failed to read {}: {e}", todo_file.display()))?;

    // Preserve original line endings style when possible
    let uses_crlf = content.contains("\r\n");
    let mut lines: Vec<String> = content
        .lines()
        .map(|l| l.to_string())
        .collect();

    // If file ended with trailing newline, lines() drops empty last; we'll restore below
    let ends_with_newline = content.ends_with('\n');

    if line > lines.len() {
        return Err(format!(
            "line {line} out of range (file has {} lines)",
            lines.len()
        ));
    }

    let idx = line - 1;
    let original = &lines[idx];
    let Some(caps) = checkbox_line_re().captures(original) else {
        return Err(format!("line {line} is not a checkbox item"));
    };

    let prefix = caps.get(1).map(|m| m.as_str()).unwrap_or("");
    let rest = caps.get(3).map(|m| m.as_str()).unwrap_or("");
    let mark = if done { "x" } else { " " };
    let new_line = format!("{prefix}[{mark}]{rest}");
    let text = rest.trim().to_string();
    lines[idx] = new_line;

    let joiner = if uses_crlf { "\r\n" } else { "\n" };
    let mut out = lines.join(joiner);
    if ends_with_newline {
        out.push_str(joiner);
    }

    atomic_write(todo_file, &out)?;
    Ok((done, text))
}

fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = parent.join(format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("todo")
    ));
    {
        let mut f = fs::File::create(&tmp)
            .map_err(|e| format!("failed to create temp file: {e}"))?;
        f.write_all(content.as_bytes())
            .map_err(|e| format!("failed to write temp file: {e}"))?;
        f.sync_all().ok();
    }
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("failed to replace {}: {e}", path.display())
    })?;
    Ok(())
}

/// Re-read a project's TODO file and refresh items/counts (keeps git as-is).
pub fn reload_project_items(project: &mut ProjectTodos) -> Result<(), String> {
    let content = fs::read_to_string(&project.todo_file)
        .map_err(|e| format!("failed to re-read {}: {e}", project.todo_file))?;
    let items = parser::parse_todos(&content);
    project.open = items.iter().filter(|i| !i.done).count();
    project.done = items.iter().filter(|i| i.done).count();
    project.total = items.len();
    project.items = items;
    Ok(())
}

/// Apply a todo toggle for a project under the scan root.
pub fn toggle_project_todo(
    scan_root: &Path,
    project: &mut ProjectTodos,
    line: usize,
    done: bool,
) -> Result<TodoItem, String> {
    let todo_path = PathBuf::from(&project.todo_file);
    let safe = git::path_under_root(scan_root, &todo_path)
        .ok_or_else(|| "todo file is outside repos root".to_string())?;

    let (new_done, text) = set_todo_done(&safe, line, done)?;
    reload_project_items(project)?;

    // Prefer item from re-parse if present
    if let Some(item) = project.items.iter().find(|i| i.line == line).cloned() {
        return Ok(item);
    }
    Ok(TodoItem {
        text,
        done: new_done,
        line,
        section: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn toggles_checkbox() {
        let dir = std::env::temp_dir().join(format!("todo-dash-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("TODO.md");
        {
            let mut f = fs::File::create(&path).unwrap();
            writeln!(f, "# TODO").unwrap();
            writeln!(f, "").unwrap();
            writeln!(f, "- [ ] first item").unwrap();
            writeln!(f, "- [x] second").unwrap();
        }
        let (done, text) = set_todo_done(&path, 3, true).unwrap();
        assert!(done);
        assert_eq!(text, "first item");
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("- [x] first item"));
        let (done2, _) = set_todo_done(&path, 3, false).unwrap();
        assert!(!done2);
        let content2 = fs::read_to_string(&path).unwrap();
        assert!(content2.contains("- [ ] first item"));
        let _ = fs::remove_dir_all(&dir);
    }
}
