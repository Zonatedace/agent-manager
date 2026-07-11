use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub text: String,
    pub done: bool,
    /// 1-based line number in the source file
    pub line: usize,
    /// Section heading this item belongs under (if any)
    pub section: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitCommit {
    pub hash: String,
    pub short_hash: String,
    pub author: String,
    pub date: String,
    pub subject: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitInfo {
    pub is_repo: bool,
    pub root: String,
    pub branch: Option<String>,
    pub dirty: bool,
    pub staged: u32,
    pub unstaged: u32,
    pub untracked: u32,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub remote_url: Option<String>,
    pub web_url: Option<String>,
    pub upstream: Option<String>,
    pub last_commit: Option<GitCommit>,
}

/// Compact git fields for lists / global cards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitSummary {
    pub is_repo: bool,
    pub branch: Option<String>,
    pub dirty: bool,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub web_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectTodos {
    /// Relative path from the repos root (e.g. "costdb" or "k3s/docs")
    pub id: String,
    pub name: String,
    pub path: String,
    pub todo_file: String,
    pub open: usize,
    pub done: usize,
    pub total: usize,
    pub items: Vec<TodoItem>,
    pub modified: Option<DateTime<Utc>>,
    pub git: Option<GitInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub scanned_at: DateTime<Utc>,
    pub root: String,
    pub projects: Vec<ProjectTodos>,
    pub total_open: usize,
    pub total_done: usize,
    pub total_items: usize,
    pub git_repo_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub open: usize,
    pub done: usize,
    pub total: usize,
    pub modified: Option<DateTime<Utc>>,
    pub git: Option<GitSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalView {
    pub scanned_at: DateTime<Utc>,
    pub root: String,
    pub total_open: usize,
    pub total_done: usize,
    pub total_items: usize,
    pub project_count: usize,
    pub git_repo_count: usize,
    pub projects: Vec<ProjectSummary>,
    /// Flattened open items with project context (for global board)
    pub open_items: Vec<GlobalItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalItem {
    pub project_id: String,
    pub project_name: String,
    pub text: String,
    pub line: usize,
    pub section: Option<String>,
}

impl Snapshot {
    pub fn to_global_view(&self) -> GlobalView {
        let projects: Vec<ProjectSummary> = self
            .projects
            .iter()
            .map(|p| ProjectSummary {
                id: p.id.clone(),
                name: p.name.clone(),
                open: p.open,
                done: p.done,
                total: p.total,
                modified: p.modified,
                git: p.git.as_ref().map(|g| g.to_summary()),
            })
            .collect();

        let mut open_items: Vec<GlobalItem> = self
            .projects
            .iter()
            .flat_map(|p| {
                p.items.iter().filter(|i| !i.done).map(move |i| GlobalItem {
                    project_id: p.id.clone(),
                    project_name: p.name.clone(),
                    text: i.text.clone(),
                    line: i.line,
                    section: i.section.clone(),
                })
            })
            .collect();

        open_items.sort_by(|a, b| {
            a.project_name
                .to_lowercase()
                .cmp(&b.project_name.to_lowercase())
                .then(a.line.cmp(&b.line))
        });

        GlobalView {
            scanned_at: self.scanned_at,
            root: self.root.clone(),
            total_open: self.total_open,
            total_done: self.total_done,
            total_items: self.total_items,
            project_count: self.projects.len(),
            git_repo_count: self.git_repo_count,
            projects,
            open_items,
        }
    }

    pub fn project_by_id(&self, id: &str) -> Option<&ProjectTodos> {
        self.projects.iter().find(|p| p.id == id)
    }

    pub fn project_by_id_mut(&mut self, id: &str) -> Option<&mut ProjectTodos> {
        self.projects.iter_mut().find(|p| p.id == id)
    }
}
