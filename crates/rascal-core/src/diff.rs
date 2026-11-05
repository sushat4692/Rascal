//! Commit detail and unified diff extraction.

use std::io::Read;

use git2::{Delta, Diff, DiffFindOptions, DiffOptions, FileMode, Patch, Repository};

use crate::error::{Error, Result};
use crate::model::{
    CommitDetail, CommitPerson, DiffBase, DiffRow, DiffRowKind, FileChange, FileDiff, FileStatus,
};
use crate::repo::{GitRepo, display_line};
use crate::{MAX_DIFF_ROWS, MAX_FILES, MAX_STAT_FILES};

impl GitRepo {
    /// Load metadata and the changed-file list for one commit.
    pub fn commit(&self, oid: crate::Oid) -> Result<CommitDetail> {
        let repo = self.repository();
        let commit = repo
            .find_commit(oid)
            .map_err(|_| Error::CommitNotFound(oid.to_string()))?;

        let parent_count = commit.parent_count();
        let base = if parent_count == 0 {
            DiffBase::Root
        } else {
            DiffBase::FirstParent
        };

        let diff = self.commit_diff_base(&commit)?;
        let delta_count = diff.deltas().len();
        let mut files = Vec::with_capacity(delta_count.min(MAX_FILES));

        for (index, delta) in diff.deltas().take(MAX_FILES).enumerate() {
            let (path, old_path) = delta_paths(&delta);
            let mut change = FileChange {
                path,
                old_path,
                status: map_delta(delta.status()),
                additions: 0,
                deletions: 0,
                is_binary: detect_binary(repo, &delta),
                is_submodule: is_submodule(&delta),
                stats_loaded: false,
            };

            if index < MAX_STAT_FILES
                && !change.is_binary
                && !change.is_submodule
                && let Ok(Some(patch)) = Patch::from_diff(&diff, index)
                && let Ok((_context, additions, deletions)) = patch.line_stats()
            {
                change.additions = additions;
                change.deletions = deletions;
                change.stats_loaded = true;
            }

            files.push(change);
        }

        let message = commit.message().unwrap_or_default();
        let message = truncate_chars(message, 100_000);
        let author = commit.author();
        let committer = commit.committer();

        Ok(CommitDetail {
            oid,
            summary: commit
                .summary()
                .ok()
                .flatten()
                .unwrap_or_default()
                .to_string(),
            message,
            author: CommitPerson {
                name: author.name().unwrap_or_default().to_string(),
                email: author.email().unwrap_or_default().to_string(),
                time: author.when().seconds(),
            },
            committer: CommitPerson {
                name: committer.name().unwrap_or_default().to_string(),
                email: committer.email().unwrap_or_default().to_string(),
                time: committer.when().seconds(),
            },
            parents: commit.parent_ids().collect(),
            files,
            files_truncated: delta_count > MAX_FILES,
            base,
        })
    }

    /// Extract the unified diff for one changed file of a commit.
    pub fn commit_diff(
        &self,
        oid: crate::Oid,
        file_index: usize,
        old_path: Option<&str>,
        new_path: Option<&str>,
    ) -> Result<FileDiff> {
        let repo = self.repository();
        let commit = repo
            .find_commit(oid)
            .map_err(|_| Error::CommitNotFound(oid.to_string()))?;
        let base_label = if commit.parent_count() == 0 {
            "空ツリー"
        } else {
            "第1親"
        };

        let diff = self.commit_diff_base(&commit)?;
        let mut delta_index = None;

        for (index, delta) in diff.deltas().enumerate() {
            let (path, _) = delta_paths(&delta);
            if new_path.is_some_and(|p| p == path.as_str())
                || old_path.is_some_and(|p| p == path.as_str())
                || (new_path.is_none() && index == file_index)
            {
                delta_index = Some(index);
                if new_path.is_some_and(|p| p == path.as_str()) {
                    break;
                }
            }
        }

        let index = delta_index.unwrap_or(file_index);
        build_file_diff(repo, &diff, index, base_label)
    }

    /// Diff for one path at a commit. Returns `None` if the commit did not
    /// change that path.
    pub fn commit_file_diff(&self, oid: crate::Oid, path: &str) -> Result<Option<FileDiff>> {
        let repo = self.repository();
        let commit = repo
            .find_commit(oid)
            .map_err(|_| Error::CommitNotFound(oid.to_string()))?;
        let base_label = if commit.parent_count() == 0 {
            "空ツリー"
        } else {
            "第1親"
        };
        let diff = self.commit_diff_base(&commit)?;

        for (index, delta) in diff.deltas().enumerate() {
            let (delta_path, old_path) = delta_paths(&delta);
            if delta_path == path || old_path.as_deref() == Some(path) {
                return build_file_diff(repo, &diff, index, base_label).map(Some);
            }
        }

        Ok(None)
    }

    pub(crate) fn commit_diff_base<'repo>(
        &'repo self,
        commit: &git2::Commit<'_>,
    ) -> Result<Diff<'repo>> {
        let repo = self.repository();
        let new_tree = commit.tree()?;
        let old_tree = if commit.parent_count() > 0 {
            Some(commit.parent(0)?.tree()?)
        } else {
            None
        };

        let mut options = diff_options();
        let diff =
            repo.diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), Some(&mut options))?;
        finish_diff(diff)
    }
}

pub fn diff_options() -> DiffOptions {
    let mut options = DiffOptions::new();
    options
        .context_lines(3)
        .interhunk_lines(1)
        .include_typechange(true);
    options
}

pub fn finish_diff<'a>(mut diff: Diff<'a>) -> Result<Diff<'a>> {
    let mut find = DiffFindOptions::new();
    find.renames(true).copies(false);
    diff.find_similar(Some(&mut find))?;
    Ok(diff)
}

pub(crate) fn build_file_diff(
    repo: &Repository,
    diff: &Diff<'_>,
    index: usize,
    base_label: &str,
) -> Result<FileDiff> {
    let delta = diff
        .get_delta(index)
        .ok_or(Error::Unsupported("差分対象のファイルが見つかりません"))?;
    let (path, old_path) = delta_paths(&delta);
    let is_binary = detect_binary(repo, &delta);
    let is_submodule = is_submodule(&delta);
    let status = map_delta(delta.status());

    let mut file = FileDiff {
        path,
        old_path,
        status,
        is_binary,
        is_submodule,
        base_label: base_label.to_string(),
        additions: 0,
        deletions: 0,
        rows: Vec::new(),
        truncated: false,
    };

    if is_binary || is_submodule {
        return Ok(file);
    }

    let Some(patch) = Patch::from_diff(diff, index)? else {
        return Ok(file);
    };

    file.additions = patch.line_stats().map(|(_, a, _)| a).unwrap_or(0);
    file.deletions = patch.line_stats().map(|(_, _, d)| d).unwrap_or(0);

    'hunks: for hunk_index in 0..patch.num_hunks() {
        let (hunk, _) = patch.hunk(hunk_index)?;
        let header = String::from_utf8_lossy(hunk.header())
            .trim_end_matches(['\n', '\r'])
            .to_string();
        file.rows.push(DiffRow {
            kind: DiffRowKind::HunkHeader,
            old_lineno: None,
            new_lineno: None,
            text: truncate_chars(&header, 2_000),
        });

        let line_count = patch.num_lines_in_hunk(hunk_index)?;
        for line_index in 0..line_count {
            if file.rows.len() >= MAX_DIFF_ROWS {
                file.truncated = true;
                break 'hunks;
            }

            let line = patch.line_in_hunk(hunk_index, line_index)?;
            let origin = line.origin();
            let kind = match origin {
                '+' => DiffRowKind::Added,
                '-' => DiffRowKind::Removed,
                '\\' => DiffRowKind::NoNewline,
                ' ' | '=' => DiffRowKind::Context,
                _ => DiffRowKind::Meta,
            };
            file.rows.push(DiffRow {
                kind,
                old_lineno: line.old_lineno(),
                new_lineno: line.new_lineno(),
                text: display_line(line.content()),
            });
        }
    }

    Ok(file)
}

pub(crate) fn delta_paths(delta: &git2::DiffDelta<'_>) -> (String, Option<String>) {
    let new_path = delta
        .new_file()
        .path()
        .map(|path| path.to_string_lossy().into_owned());
    let old_path = delta
        .old_file()
        .path()
        .map(|path| path.to_string_lossy().into_owned());
    let path = new_path
        .clone()
        .or_else(|| old_path.clone())
        .unwrap_or_default();

    let old_path = match delta.status() {
        Delta::Renamed | Delta::Copied => old_path.filter(|old| old != &path),
        _ => None,
    };

    (path, old_path)
}

pub(crate) fn map_delta(delta: Delta) -> FileStatus {
    match delta {
        Delta::Added | Delta::Untracked => FileStatus::Added,
        Delta::Deleted => FileStatus::Deleted,
        Delta::Modified => FileStatus::Modified,
        Delta::Renamed => FileStatus::Renamed,
        Delta::Copied => FileStatus::Copied,
        Delta::Typechange => FileStatus::TypeChange,
        Delta::Unmodified => FileStatus::Unmodified,
        Delta::Conflicted => FileStatus::Other,
        _ => FileStatus::Other,
    }
}

pub(crate) fn detect_binary(repo: &Repository, delta: &git2::DiffDelta<'_>) -> bool {
    if delta.flags().is_binary() || delta.new_file().is_binary() || delta.old_file().is_binary() {
        return true;
    }

    for file in [delta.new_file(), delta.old_file()] {
        let id = file.id();
        if !id.is_zero()
            && let Ok(blob) = repo.find_blob(id)
        {
            return blob.is_binary();
        }
    }

    // Untracked working-tree entries have no blob in the object database yet.
    if let Some(workdir) = repo.workdir() {
        for file in [delta.new_file(), delta.old_file()] {
            let Some(path) = file.path() else {
                continue;
            };
            let path = workdir.join(path);
            let Ok(mut handle) = std::fs::File::open(path) else {
                continue;
            };
            let mut buffer = [0u8; 8_000];
            let Ok(read) = handle.read(&mut buffer) else {
                continue;
            };
            return buffer[..read].contains(&0);
        }
    }

    false
}

pub(crate) fn is_submodule(delta: &git2::DiffDelta<'_>) -> bool {
    delta.new_file().mode() == FileMode::Commit || delta.old_file().mode() == FileMode::Commit
}

fn truncate_chars(input: &str, max: usize) -> String {
    if input.chars().count() <= max {
        return input.to_string();
    }
    let mut out: String = input.chars().take(max).collect();
    out.push('…');
    out
}
