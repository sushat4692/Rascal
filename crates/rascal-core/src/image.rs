//! Image blobs and image-to-image diffs.

use std::path::Path;

use git2::{Diff, ObjectType, Repository};

use crate::error::{Error, Result};
use crate::model::{DiffLayer, FileStatus, ImageDiff, ImageFile};
use crate::repo::GitRepo;
use crate::{MAX_IMAGE_BYTES, diff::delta_paths, diff::diff_options, diff::finish_diff};

impl GitRepo {
    /// Read one image blob from a commit tree.
    pub fn image_file(&self, oid: crate::Oid, path: &str) -> Result<ImageFile> {
        let repo = self.repository();
        let commit = repo
            .find_commit(oid)
            .map_err(|_| Error::CommitNotFound(oid.to_string()))?;
        let tree = commit.tree()?;
        let entry = tree.get_path(Path::new(path))?;
        let object = entry.to_object(repo)?;
        if object.kind() != Some(ObjectType::Blob) {
            return Err(Error::Unsupported("画像ファイルではありません"));
        }
        let blob = object
            .into_blob()
            .map_err(|_| Error::Unsupported("画像を読み込めません"))?;
        read_blob(repo, path, blob.id(), blob.size() as u64)
    }

    /// Load old/new image blobs for a commit.
    pub fn commit_image_diff(
        &self,
        oid: crate::Oid,
        old_path: Option<&str>,
        new_path: Option<&str>,
    ) -> Result<ImageDiff> {
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
        let Some((index, delta)) = find_delta(&diff, old_path, new_path) else {
            return Err(Error::Unsupported("画像の差分が見つかりません"));
        };
        let (path, old_path) = delta_paths(&delta);
        let status = crate::diff::map_delta(delta.status());
        let old = read_blob_side(repo, &delta, true)?;
        let new = read_blob_side(repo, &delta, false)?;
        let _ = index;
        Ok(ImageDiff {
            path,
            old_path,
            status,
            base_label: base_label.to_string(),
            old,
            new,
        })
    }

    /// Load old/new image data for a working tree file.
    pub fn worktree_image_diff(
        &self,
        path: &str,
        old_path: Option<&str>,
        layer: DiffLayer,
    ) -> Result<ImageDiff> {
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

        let repo = self.repository();
        let diff = match layer {
            DiffLayer::Staged => {
                let head_tree = repo.head().ok().and_then(|head| head.peel_to_tree().ok());
                repo.diff_tree_to_index(head_tree.as_ref(), None, Some(&mut options))?
            }
            DiffLayer::Unstaged | DiffLayer::Untracked => {
                repo.diff_index_to_workdir(None, Some(&mut options))?
            }
        };
        let diff = finish_diff(diff)?;

        let Some((_, delta)) = find_delta(&diff, old_path, Some(path)) else {
            return Err(Error::Unsupported("画像の差分が見つかりません"));
        };
        let (new_path, old_path) = delta_paths(&delta);
        let status = crate::diff::map_delta(delta.status());

        let old = if layer == DiffLayer::Untracked {
            None
        } else {
            read_blob_side(repo, &delta, true)?
        };
        let new = if matches!(status, FileStatus::Deleted) {
            None
        } else {
            match read_blob_side(repo, &delta, false)? {
                Some(image) => Some(image),
                None => {
                    let workdir = repo
                        .workdir()
                        .ok_or(Error::Unsupported("作業ツリーがありません"))?;
                    read_worktree_image(workdir, &new_path)?
                }
            }
        };

        Ok(ImageDiff {
            path: new_path,
            old_path,
            status,
            base_label: layer.label().to_string(),
            old,
            new,
        })
    }
}

fn find_delta<'a>(
    diff: &'a Diff<'a>,
    old_path: Option<&str>,
    new_path: Option<&str>,
) -> Option<(usize, git2::DiffDelta<'a>)> {
    diff.deltas().enumerate().find_map(|(index, delta)| {
        let (path, delta_old) = delta_paths(&delta);
        let matches = new_path.is_some_and(|p| p == path)
            || old_path.is_some_and(|p| p == path)
            || delta_old.as_deref().is_some_and(|p| {
                new_path.is_some_and(|n| n == p) || old_path.is_some_and(|o| o == p)
            });
        matches.then_some((index, delta))
    })
}

fn read_blob_side(
    repo: &Repository,
    delta: &git2::DiffDelta<'_>,
    old: bool,
) -> Result<Option<ImageFile>> {
    let file = if old {
        delta.old_file()
    } else {
        delta.new_file()
    };
    let id = file.id();
    if id.is_zero() {
        return Ok(None);
    }
    let path = file
        .path()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    if path.is_empty() {
        return Ok(None);
    }
    let blob = repo.find_blob(id)?;
    read_blob(repo, &path, id, blob.size() as u64).map(Some)
}

fn read_blob(_repo: &Repository, path: &str, oid: crate::Oid, size: u64) -> Result<ImageFile> {
    if size > MAX_IMAGE_BYTES {
        return Err(Error::Unsupported("画像が大きすぎます"));
    }
    let blob = _repo.find_blob(oid)?;
    if blob.size() as u64 > MAX_IMAGE_BYTES {
        return Err(Error::Unsupported("画像が大きすぎます"));
    }
    Ok(ImageFile {
        path: path.to_string(),
        oid: Some(oid),
        size: blob.size() as u64,
        bytes: blob.content().to_vec(),
    })
}

fn read_worktree_image(workdir: &Path, path: &str) -> Result<Option<ImageFile>> {
    let full = workdir.join(path);
    let Ok(metadata) = std::fs::metadata(&full) else {
        return Ok(None);
    };
    if !metadata.is_file() {
        return Ok(None);
    }
    if metadata.len() > MAX_IMAGE_BYTES {
        return Err(Error::Unsupported("画像が大きすぎます"));
    }
    let bytes = std::fs::read(&full)?;
    Ok(Some(ImageFile {
        path: path.to_string(),
        oid: None,
        size: bytes.len() as u64,
        bytes,
    }))
}
