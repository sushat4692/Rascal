//! Repository metadata and branch listing.

use std::path::{Path, PathBuf};

use git2::{BranchType, Repository, Status, StatusOptions};

use crate::error::{Error, Result};
use crate::model::{BranchInfo, HeadState, RepoInfo, WorktreeCounts};

/// A read-only handle to a Git repository.
///
/// The handle is intentionally not exposed to the UI; a dedicated worker
/// thread owns it and only sends plain data across the channel.
pub struct GitRepo {
    repo: Repository,
}

impl GitRepo {
    /// Open a repository, discovering it from a directory inside the worktree.
    pub fn open(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Err(Error::PathNotFound(path.to_path_buf()));
        }

        let repo =
            Repository::discover(path).map_err(|_| Error::NotARepository(path.to_path_buf()))?;
        Ok(Self { repo })
    }

    pub fn repository(&self) -> &Repository {
        &self.repo
    }

    pub fn info(&self) -> Result<RepoInfo> {
        self.info_inner(true)
    }

    /// Load repository metadata without scanning the working tree.
    ///
    /// Use this for the first paint of a tab. `info` scans every tracked file
    /// for status, which can be slow on large repositories.
    pub fn info_fast(&self) -> Result<RepoInfo> {
        self.info_inner(false)
    }

    fn info_inner(&self, include_counts: bool) -> Result<RepoInfo> {
        let head = self.head_state()?;
        let workdir = self.repo.workdir().map(Path::to_path_buf);
        let name = repo_name(&self.repo, &workdir);
        let counts = if include_counts && !self.repo.is_bare() {
            self.worktree_counts()?
        } else {
            WorktreeCounts::default()
        };

        Ok(RepoInfo {
            name,
            workdir,
            git_dir: self.repo.path().to_path_buf(),
            head,
            is_bare: self.repo.is_bare(),
            counts,
        })
    }

    pub fn head_state(&self) -> Result<HeadState> {
        let head = match self.repo.head() {
            Ok(head) => head,
            Err(err)
                if err.code() == git2::ErrorCode::UnbornBranch
                    || err.code() == git2::ErrorCode::NotFound =>
            {
                return Ok(HeadState::Unborn);
            }
            Err(err) => return Err(err.into()),
        };

        if head.is_branch() {
            let shorthand = head
                .shorthand()
                .map_err(|_| Error::NonUtf8Reference("HEAD".to_string()))?
                .to_string();
            let oid = head
                .target()
                .or_else(|| head.peel_to_commit().ok().map(|c| c.id()))
                .ok_or(Error::NoHead)?;

            let (upstream, ahead, behind) =
                match self.repo.find_branch(&shorthand, BranchType::Local) {
                    Ok(branch) => upstream_info(&self.repo, &branch, oid),
                    Err(_) => (None, 0, 0),
                };

            Ok(HeadState::Attached {
                branch: shorthand,
                oid,
                upstream,
                ahead,
                behind,
            })
        } else {
            let oid = head
                .target()
                .or_else(|| head.peel_to_commit().ok().map(|c| c.id()))
                .ok_or(Error::NoHead)?;
            Ok(HeadState::Detached { oid })
        }
    }

    pub fn branches(&self) -> Result<Vec<BranchInfo>> {
        let mut branches = Vec::new();
        let iter = self.repo.branches(Some(BranchType::Local))?;
        for item in iter {
            let (branch, _) = item?;
            let Some(name) = branch.name().ok().flatten().map(str::to_string) else {
                continue;
            };
            let Ok(reference) = branch.get().resolve() else {
                continue;
            };
            let Ok(full_ref) = reference.name().map(str::to_string) else {
                continue;
            };
            let Ok(commit) = reference.peel_to_commit() else {
                continue;
            };
            let tip = commit.id();
            let (upstream, ahead, behind) = upstream_info(&self.repo, &branch, tip);

            branches.push(BranchInfo {
                name,
                full_ref,
                tip,
                is_head: branch.is_head(),
                upstream,
                ahead,
                behind,
            });
        }

        branches.sort_by(|a, b| {
            b.is_head
                .cmp(&a.is_head)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(branches)
    }

    pub fn worktree_counts(&self) -> Result<WorktreeCounts> {
        let mut counts = WorktreeCounts::default();
        for entry in self.statuses()?.iter() {
            let (staged, unstaged, untracked) = classify_status(entry.status());
            counts.staged += usize::from(staged);
            counts.unstaged += usize::from(unstaged);
            counts.untracked += usize::from(untracked);
        }
        Ok(counts)
    }

    /// Count working-tree changes without index-to-workdir rename detection.
    ///
    /// Full rename detection is expensive on large repositories. This variant
    /// is used for the first paint, while the complete snapshot is loaded on
    /// demand.
    pub fn worktree_counts_fast(&self) -> Result<WorktreeCounts> {
        let mut options = StatusOptions::new();
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .renames_head_to_index(true)
            .renames_index_to_workdir(false)
            .include_ignored(false);
        let statuses = self.repo.statuses(Some(&mut options))?;
        let mut counts = WorktreeCounts::default();
        for entry in statuses.iter() {
            let (staged, unstaged, untracked) = classify_status(entry.status());
            counts.staged += usize::from(staged);
            counts.unstaged += usize::from(unstaged);
            counts.untracked += usize::from(untracked);
        }
        Ok(counts)
    }

    pub(crate) fn statuses(&self) -> Result<git2::Statuses<'_>> {
        let mut options = StatusOptions::new();
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .renames_head_to_index(true)
            .renames_index_to_workdir(true)
            .include_ignored(false);
        Ok(self.repo.statuses(Some(&mut options))?)
    }
}

fn upstream_info(
    repo: &Repository,
    branch: &git2::Branch<'_>,
    local_oid: crate::Oid,
) -> (Option<String>, usize, usize) {
    let Ok(upstream) = branch.upstream() else {
        return (None, 0, 0);
    };
    let name = upstream.name().ok().flatten().map(str::to_string);
    let ahead_behind = upstream
        .get()
        .target()
        .and_then(|upstream_oid| repo.graph_ahead_behind(local_oid, upstream_oid).ok())
        .unwrap_or((0, 0));
    (name, ahead_behind.0, ahead_behind.1)
}

fn repo_name(repo: &Repository, workdir: &Option<PathBuf>) -> String {
    if let Some(workdir) = workdir {
        return workdir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "repository".to_string());
    }

    repo.path()
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repository".to_string())
}

/// Returns (staged, unstaged, untracked) for a status entry.
pub(crate) fn classify_status(status: Status) -> (bool, bool, bool) {
    let staged_flags = Status::INDEX_NEW
        | Status::INDEX_MODIFIED
        | Status::INDEX_DELETED
        | Status::INDEX_RENAMED
        | Status::INDEX_TYPECHANGE;

    let unstaged_flags = Status::WT_MODIFIED
        | Status::WT_DELETED
        | Status::WT_RENAMED
        | Status::WT_TYPECHANGE
        | Status::CONFLICTED;

    let untracked = status.contains(Status::WT_NEW) && !status.contains(Status::INDEX_NEW);
    let staged = status.intersects(staged_flags);
    let unstaged =
        status.intersects(unstaged_flags) || (status.contains(Status::WT_NEW) && !untracked);

    (staged, unstaged, untracked)
}

pub(crate) fn expand_tabs(input: &str) -> String {
    if !input.contains('\t') {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len() + 8);
    for ch in input.chars() {
        if ch == '\t' {
            out.push_str("    ");
        } else {
            out.push(ch);
        }
    }
    out
}

pub(crate) fn display_line(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text: &str = text.strip_suffix('\n').unwrap_or(&text);
    let text: &str = text.strip_suffix('\r').unwrap_or(text);
    let mut expanded = expand_tabs(text);
    const MAX_CHARS: usize = 2_000;
    if expanded.chars().count() > MAX_CHARS {
        expanded = expanded.chars().take(MAX_CHARS).collect::<String>();
        expanded.push('…');
    }
    expanded
}
