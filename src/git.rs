use crate::models::{GitCommit, GitInfo, GitSummary};
use crate::process_util;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

fn run_git(cwd: &Path, args: &[&str]) -> Option<String> {
    // CREATE_NO_WINDOW: avoid a console flash per git call under the GUI app.
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(cwd);
    process_util::hide_console(&mut cmd);
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Find the git worktree root for a project path, if any.
pub fn find_git_root(project_path: &Path) -> Option<PathBuf> {
    // Prefer walking up for speed (no process spawn when not a repo).
    let mut cur = if project_path.is_file() {
        project_path.parent()?.to_path_buf()
    } else {
        project_path.to_path_buf()
    };

    loop {
        if cur.join(".git").exists() {
            return Some(cur);
        }
        if !cur.pop() {
            break;
        }
    }

    // Fallback: git rev-parse (handles worktrees / .git files)
    run_git(project_path, &["rev-parse", "--show-toplevel"]).map(PathBuf::from)
}

/// Convert common remote URL forms into a browser-friendly https URL.
pub fn remote_to_web_url(remote: &str) -> Option<String> {
    let r = remote.trim().trim_end_matches(".git");

    // git@github.com:user/repo
    if let Some(rest) = r.strip_prefix("git@") {
        if let Some((host, path)) = rest.split_once(':') {
            return Some(format!("https://{host}/{path}"));
        }
    }

    // ssh://git@github.com/user/repo
    if let Some(rest) = r.strip_prefix("ssh://git@") {
        return Some(format!("https://{rest}"));
    }
    if let Some(rest) = r.strip_prefix("ssh://") {
        // ssh://github.com/user/repo
        return Some(format!("https://{rest}"));
    }

    // https:// or http://
    if r.starts_with("https://") || r.starts_with("http://") {
        return Some(r.to_string());
    }

    None
}

fn parse_ahead_behind(branch_line: &str) -> (Option<u32>, Option<u32>) {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"\[(?:ahead (\d+))?(?:, )?(?:behind (\d+))?\]").unwrap()
    });

    if let Some(caps) = re.captures(branch_line) {
        let ahead = caps.get(1).and_then(|m| m.as_str().parse().ok());
        let behind = caps.get(2).and_then(|m| m.as_str().parse().ok());
        return (ahead, behind);
    }
    (None, None)
}

fn parse_branch_name(branch_line: &str) -> Option<String> {
    // ## main...origin/main [ahead 3]
    // ## HEAD (no branch)
    // ## feature/foo
    // ## No commits yet on master
    // ## Initial commit on main
    let line = branch_line.trim();
    let line = line.strip_prefix("## ")?;
    if line.starts_with("HEAD") {
        return Some("HEAD".to_string());
    }
    if let Some(rest) = line.strip_prefix("No commits yet on ") {
        let name = rest.split_whitespace().next().unwrap_or(rest);
        return Some(name.to_string());
    }
    if let Some(rest) = line.strip_prefix("Initial commit on ") {
        let name = rest.split_whitespace().next().unwrap_or(rest);
        return Some(name.to_string());
    }
    let name = line
        .split("...")
        .next()
        .unwrap_or(line)
        .split_whitespace()
        .next()
        .unwrap_or(line);
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    let ca = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
    let cb = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
    ca == cb
}

/// Collect git metadata for a project directory.
///
/// Returns `None` if there is no git worktree, or if the only enclosing repo is
/// the multi-project `scan_root` (so folders under a monorepo root aren't all
/// labeled as the same giant repo).
pub fn collect_git_info(project_path: &Path, scan_root: &Path) -> Option<GitInfo> {
    let root = find_git_root(project_path)?;
    // Prefer project-local repos; ignore the umbrella repos root if that's the only hit.
    if same_path(&root, scan_root) {
        return None;
    }
    let root_str = root.to_string_lossy().to_string();

    // Single porcelain status with branch header
    let status = run_git(&root, &["status", "--porcelain=v1", "-b"]).unwrap_or_default();
    let mut lines = status.lines();
    let branch_line = lines.next().unwrap_or("").to_string();
    let branch = parse_branch_name(&branch_line)
        .or_else(|| run_git(&root, &["rev-parse", "--abbrev-ref", "HEAD"]));

    let (ahead, behind) = parse_ahead_behind(&branch_line);

    let mut staged = 0u32;
    let mut unstaged = 0u32;
    let mut untracked = 0u32;
    for line in status.lines().skip(1) {
        if line.len() < 2 {
            continue;
        }
        let b = line.as_bytes();
        let x = b[0] as char;
        let y = b[1] as char;
        if x == '?' && y == '?' {
            untracked += 1;
            continue;
        }
        if x != ' ' && x != '?' {
            staged += 1;
        }
        if y != ' ' && y != '?' {
            unstaged += 1;
        }
    }
    let dirty = staged > 0 || unstaged > 0 || untracked > 0;

    let remote_url = run_git(&root, &["remote", "get-url", "origin"]);
    let web_url = remote_url.as_deref().and_then(remote_to_web_url);

    let last_commit = run_git(
        &root,
        &[
            "log",
            "-1",
            "--format=%H%x1f%h%x1f%an%x1f%cI%x1f%s",
        ],
    )
    .and_then(|s| {
        let parts: Vec<&str> = s.split('\u{1f}').collect();
        if parts.len() < 5 {
            return None;
        }
        Some(GitCommit {
            hash: parts[0].to_string(),
            short_hash: parts[1].to_string(),
            author: parts[2].to_string(),
            date: parts[3].to_string(),
            subject: parts[4].to_string(),
        })
    });

    // Upstream tracking name if any
    let upstream = run_git(&root, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]);

    Some(GitInfo {
        is_repo: true,
        root: root_str,
        branch,
        dirty,
        staged,
        unstaged,
        untracked,
        ahead,
        behind,
        remote_url,
        web_url,
        upstream,
        last_commit,
    })
}

impl GitInfo {
    pub fn to_summary(&self) -> GitSummary {
        GitSummary {
            is_repo: true,
            branch: self.branch.clone(),
            dirty: self.dirty,
            ahead: self.ahead,
            behind: self.behind,
            web_url: self.web_url.clone(),
        }
    }
}

/// Ensure a path is under the allowed repos root (canonicalized when possible).
pub fn path_under_root(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let root = root.canonicalize().ok().unwrap_or_else(|| root.to_path_buf());
    let cand = candidate
        .canonicalize()
        .ok()
        .unwrap_or_else(|| candidate.to_path_buf());
    if cand.starts_with(&root) {
        Some(cand)
    } else {
        None
    }
}

/// Open a folder in the OS file manager (Explorer on Windows).
pub fn open_folder(path: &Path) -> Result<(), String> {
    open::that(path).map_err(|e| format!("failed to open folder: {e}"))
}

/// Open a URL in the default browser.
pub fn open_url(url: &str) -> Result<(), String> {
    // Basic safety: only http(s)
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("only http(s) URLs are allowed".into());
    }
    open::that(url).map_err(|e| format!("failed to open url: {e}"))
}

/// Open a shell in the project directory (Windows: PowerShell).
#[cfg(windows)]
pub fn open_terminal(path: &Path) -> Result<(), String> {
    // Use -WorkingDirectory so paths with spaces/quotes are handled by PowerShell itself.
    Command::new("powershell")
        .args([
            "-NoExit",
            "-WorkingDirectory",
            &path.to_string_lossy(),
        ])
        .spawn()
        .map_err(|e| format!("failed to open terminal: {e}"))?;
    Ok(())
}

#[cfg(not(windows))]
pub fn open_terminal(path: &Path) -> Result<(), String> {
    // Best-effort on Unix
    let candidates = ["xdg-terminal", "gnome-terminal", "x-terminal-emulator", "open"];
    for cmd in candidates {
        if Command::new(cmd)
            .arg(path)
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }
    Err("could not find a terminal launcher".into())
}

/// Optional: run `git fetch --dry-run` style not needed; provide live status refresh only.
#[allow(dead_code)]
pub fn with_timeout_note() {
    let _ = Duration::from_secs(5);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_ssh_remote() {
        assert_eq!(
            remote_to_web_url("git@github.com:example/my-repo.git").as_deref(),
            Some("https://github.com/example/my-repo")
        );
    }

    #[test]
    fn converts_https_remote() {
        assert_eq!(
            remote_to_web_url("https://github.com/example/my-app.git").as_deref(),
            Some("https://github.com/example/my-app")
        );
    }

    #[test]
    fn parses_ahead_behind() {
        let (a, b) = parse_ahead_behind("## main...origin/main [ahead 3, behind 2]");
        assert_eq!(a, Some(3));
        assert_eq!(b, Some(2));
        let (a2, b2) = parse_ahead_behind("## main...origin/main [ahead 3]");
        assert_eq!(a2, Some(3));
        assert_eq!(b2, None);
    }

    #[test]
    fn parses_branch() {
        assert_eq!(
            parse_branch_name("## feat/foo...origin/feat/foo [ahead 1]").as_deref(),
            Some("feat/foo")
        );
        assert_eq!(
            parse_branch_name("## No commits yet on master").as_deref(),
            Some("master")
        );
    }
}
