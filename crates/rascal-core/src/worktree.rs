//! Working-tree status and per-file diffs.

use git2::Diff;

use crate::diff::{build_file_diff, delta_paths, diff_options, finish_diff, map_delta};
use crate::error::{Error, Result};
use crate::model::{
    DiffLayer, FileDiff, FileStatus, WorktreeCounts, WorktreeFile, WorktreeSnapshot,
};
use crate::repo::{GitRepo, classify_status};

impl GitRepo {
    /// Snapshot the staged/unstaged/untracked files of the working tree.
    pub fn worktree(&self) -> Result<WorktreeSnapshot> {
        if self.repository().is_bare() {
            return Err(Error::Unsupported(
                "bareリポジトリには作業ツリーがありません",
            ));
        }

        let statuses = self.statuses()?;
        let mut counts = WorktreeCounts::default();
        let mut files = Vec::new();

        for entry in statuses.iter() {
            let status = entry.status();
            let (staged, unstaged, untracked) = classify_status(status);
            if !staged && !unstaged && !untracked {
                continue;
            }

            let path = match entry.path() {
                Ok(path) => path.to_string(),
                Err(_) => String::from_utf8_lossy(entry.path_bytes()).into_owned(),
            };
            if path.is_empty() {
                continue;
            }

            if staged {
                counts.staged += 1;
                let (old_path, file_status) = staged_details(entry.head_to_index().as_ref());
                files.push(WorktreeFile {
                    path: path.clone(),
                    old_path,
                    layer: DiffLayer::Staged,
                    status: file_status,
                    is_binary: false,
                });
            }

            if unstaged {
                counts.unstaged += 1;
                let (old_path, file_status) = unstaged_details(entry.index_to_workdir().as_ref());
                files.push(WorktreeFile {
                    path: path.clone(),
                    old_path,
                    layer: DiffLayer::Unstaged,
                    status: file_status,
                    is_binary: false,
                });
            }

            if untracked {
                counts.untracked += 1;
                files.push(WorktreeFile {
                    path,
                    old_path: None,
                    layer: DiffLayer::Untracked,
                    status: FileStatus::Untracked,
                    is_binary: false,
                });
            }
        }

        files.sort_by(|a, b| {
            layer_rank(a.layer)
                .cmp(&layer_rank(b.layer))
                .then_with(|| a.path.to_lowercase().cmp(&b.path.to_lowercase()))
        });

        Ok(WorktreeSnapshot { counts, files })
    }

    /// Load the unified diff for one working-tree file and layer.
    pub fn worktree_diff(
        &self,
        path: &str,
        old_path: Option<&str>,
        layer: DiffLayer,
    ) -> Result<FileDiff> {
        if self.repository().is_bare() {
            return Err(Error::Unsupported(
                "bareリポジトリには作業ツリーがありません",
            ));
        }

        let mut options = diff_options();
        options.pathspec(path);
        if let Some(old_path) = old_path.filter(|old| *old != path) {
            options.pathspec(old_path);
        }
        if layer == DiffLayer::Untracked {
            options
                .include_untracked(true)
                .recurse_untracked_dirs(true)
                .show_untracked_content(true);
        }

        let diff = match layer {
            DiffLayer::Staged => {
                let head_tree = self
                    .repository()
                    .head()
                    .ok()
                    .and_then(|head| head.peel_to_tree().ok());
                self.repository().diff_tree_to_index(
                    head_tree.as_ref(),
                    None,
                    Some(&mut options),
                )?
            }
            DiffLayer::Unstaged | DiffLayer::Untracked => self
                .repository()
                .diff_index_to_workdir(None, Some(&mut options))?,
        };
        let diff = finish_diff(diff)?;

        let index = find_delta_index(&diff, path, old_path);
        match index {
            Some(index) => build_file_diff(self.repository(), &diff, index, layer.label()),
            None => Ok(FileDiff {
                path: path.to_string(),
                old_path: old_path.map(str::to_string),
                status: status_for_layer(layer),
                is_binary: false,
                is_submodule: false,
                base_label: layer.label().to_string(),
                additions: 0,
                deletions: 0,
                rows: Vec::new(),
                truncated: false,
            }),
        }
    }
}

fn find_delta_index(diff: &Diff<'_>, path: &str, old_path: Option<&str>) -> Option<usize> {
    diff.deltas().enumerate().find_map(|(index, delta)| {
        let (new_path, delta_old_path) = delta_paths(&delta);
        if new_path == path
            || old_path.is_some_and(|old| old == new_path)
            || delta_old_path.as_deref() == old_path
            || delta_old_path.as_deref() == Some(path)
        {
            Some(index)
        } else {
            None
        }
    })
}

fn staged_details(delta: Option<&git2::DiffDelta<'_>>) -> (Option<String>, FileStatus) {
    match delta {
        Some(delta) => {
            let (path, old_path) = delta_paths(delta);
            let old = old_path.filter(|old| old != &path);
            (old, map_delta(delta.status()))
        }
        None => (None, FileStatus::Modified),
    }
}

fn unstaged_details(delta: Option<&git2::DiffDelta<'_>>) -> (Option<String>, FileStatus) {
    match delta {
        Some(delta) => {
            let (path, old_path) = delta_paths(delta);
            let old = old_path.filter(|old| old != &path);
            (old, map_delta(delta.status()))
        }
        None => (None, FileStatus::Modified),
    }
}

fn status_for_layer(layer: DiffLayer) -> FileStatus {
    match layer {
        DiffLayer::Staged => FileStatus::Modified,
        DiffLayer::Unstaged => FileStatus::Modified,
        DiffLayer::Untracked => FileStatus::Untracked,
    }
}

fn layer_rank(layer: DiffLayer) -> u8 {
    match layer {
        DiffLayer::Staged => 0,
        DiffLayer::Unstaged => 1,
        DiffLayer::Untracked => 2,
    }
}
