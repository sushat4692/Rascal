//! Commit history and reference decoration.

use std::collections::HashMap;

use git2::{Commit, Repository, Sort};

use crate::error::{Error, Result};
use crate::model::{CommitSummary, HistoryPage, RefKind, RefLabel};
use crate::repo::GitRepo;

impl GitRepo {
    /// Walk history from `rev` (`HEAD` or a full ref name).
    pub fn history(&self, rev: &str, offset: usize, limit: usize) -> Result<HistoryPage> {
        let repo = self.repository();
        let refs = collect_refs(repo)?;

        let mut walk = repo.revwalk()?;
        walk.set_sorting(Sort::TOPOLOGICAL)?;

        let pushed = if rev == "HEAD" {
            walk.push_head()
        } else {
            walk.push_ref(rev)
        };
        pushed.map_err(|err| {
            if err.code() == git2::ErrorCode::NotFound
                || err.code() == git2::ErrorCode::UnbornBranch
            {
                Error::NoHead
            } else {
                Error::Git(err)
            }
        })?;

        let mut commits = Vec::with_capacity(limit.saturating_add(1));
        for oid in walk.skip(offset).take(limit.saturating_add(1)) {
            let oid = oid?;
            let commit = repo.find_commit(oid)?;
            commits.push(summarize(&commit, &refs)?);
        }

        let has_more = commits.len() > limit;
        commits.truncate(limit);

        Ok(HistoryPage { commits, has_more })
    }

    /// Commits that touched `path`, starting at `rev`.
    ///
    /// Merge commits are compared against their first parent. Renames are not
    /// followed; use the current path.
    pub fn file_history(
        &self,
        rev: &str,
        path: &str,
        offset: usize,
        limit: usize,
    ) -> Result<HistoryPage> {
        let repo = self.repository();
        let mut walk = repo.revwalk()?;
        walk.set_sorting(Sort::TOPOLOGICAL)?;

        let pushed = if rev == "HEAD" {
            walk.push_head()
        } else if let Ok(oid) = rev.parse::<git2::Oid>() {
            walk.push(oid)
        } else {
            walk.push_ref(rev)
        };
        pushed.map_err(|err| {
            if err.code() == git2::ErrorCode::NotFound
                || err.code() == git2::ErrorCode::UnbornBranch
            {
                Error::NoHead
            } else {
                Error::Git(err)
            }
        })?;

        let empty_refs = HashMap::new();
        let mut commits = Vec::with_capacity(limit.min(super::HISTORY_PAGE_SIZE));
        let mut matched = 0usize;
        let mut has_more = false;

        for oid in walk {
            let oid = oid?;
            let commit = repo.find_commit(oid)?;
            if !commit_touches_path(repo, &commit, path)? {
                continue;
            }

            if matched < offset {
                matched += 1;
                continue;
            }

            if commits.len() >= limit {
                has_more = true;
                break;
            }

            commits.push(summarize(&commit, &empty_refs)?);
            matched += 1;
        }

        Ok(HistoryPage { commits, has_more })
    }
}

fn commit_touches_path(repo: &Repository, commit: &Commit<'_>, path: &str) -> Result<bool> {
    let new_tree = commit.tree()?;
    let old_tree = if commit.parent_count() > 0 {
        Some(commit.parent(0)?.tree()?)
    } else {
        None
    };

    let mut options = git2::DiffOptions::new();
    options
        .pathspec(path)
        .include_typechange(true)
        .include_unmodified(false);
    let diff = repo.diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), Some(&mut options))?;
    Ok(diff.deltas().next().is_some())
}

fn summarize(
    commit: &Commit<'_>,
    refs: &HashMap<crate::Oid, Vec<RefLabel>>,
) -> Result<CommitSummary> {
    let author = commit.author();
    let summary = commit
        .summary()
        .ok()
        .flatten()
        .map(str::to_string)
        .unwrap_or_else(|| {
            commit
                .message()
                .unwrap_or_default()
                .lines()
                .next()
                .unwrap_or_default()
                .to_string()
        });

    Ok(CommitSummary {
        oid: commit.id(),
        summary,
        author_name: author.name().unwrap_or_default().to_string(),
        author_email: author.email().unwrap_or_default().to_string(),
        author_time: author.when().seconds(),
        committer_time: commit.committer().when().seconds(),
        parents: commit.parent_ids().collect(),
        refs: refs.get(&commit.id()).cloned().unwrap_or_default(),
    })
}

fn collect_refs(repo: &Repository) -> Result<HashMap<crate::Oid, Vec<RefLabel>>> {
    let mut map: HashMap<crate::Oid, Vec<RefLabel>> = HashMap::new();

    for reference in repo.references()? {
        let reference = reference?;
        let Ok(name) = reference.name() else {
            continue;
        };
        let (kind, display) = if let Some(rest) = name.strip_prefix("refs/heads/") {
            (RefKind::LocalBranch, rest.to_string())
        } else if let Some(rest) = name.strip_prefix("refs/remotes/") {
            (RefKind::RemoteBranch, rest.to_string())
        } else if let Some(rest) = name.strip_prefix("refs/tags/") {
            (RefKind::Tag, rest.to_string())
        } else {
            continue;
        };

        let Ok(resolved) = reference.resolve() else {
            continue;
        };
        let Ok(commit) = resolved.peel_to_commit() else {
            continue;
        };
        map.entry(commit.id()).or_default().push(RefLabel {
            kind,
            name: display,
        });
    }

    if let Ok(head) = repo.head()
        && let Ok(commit) = head.peel_to_commit()
    {
        map.entry(commit.id()).or_default().push(RefLabel {
            kind: RefKind::Head,
            name: "HEAD".to_string(),
        });
    }

    for labels in map.values_mut() {
        labels.sort_by(|a, b| {
            ref_rank(a.kind)
                .cmp(&ref_rank(b.kind))
                .then_with(|| a.name.cmp(&b.name))
        });
        labels.dedup_by(|a, b| a.kind == b.kind && a.name == b.name);
    }

    Ok(map)
}

fn ref_rank(kind: RefKind) -> u8 {
    match kind {
        RefKind::Head => 0,
        RefKind::LocalBranch => 1,
        RefKind::RemoteBranch => 2,
        RefKind::Tag => 3,
    }
}
