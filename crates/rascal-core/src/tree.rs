//! Lazy commit-tree browsing and blob content.

use std::path::Path;

use git2::{ObjectType, Tree};

use crate::error::{Error, Result};
use crate::model::{BlobView, TreeEntries, TreeEntry, TreeEntryKind};
use crate::repo::{GitRepo, expand_tabs};
use crate::{MAX_BLOB_BYTES, MAX_BLOB_LINES, MAX_TREE_ENTRIES};

impl GitRepo {
    /// List the immediate children of a directory in a commit tree.
    pub fn tree_entries(&self, oid: crate::Oid, dir: &str) -> Result<TreeEntries> {
        let repo = self.repository();
        let commit = repo
            .find_commit(oid)
            .map_err(|_| Error::CommitNotFound(oid.to_string()))?;
        let root = commit.tree()?;

        let tree: Tree<'_> = if dir.is_empty() {
            root
        } else {
            let entry = root.get_path(Path::new(dir))?;
            if entry.kind() != Some(ObjectType::Tree) {
                return Err(Error::Unsupported("指定パスはディレクトリではありません"));
            }
            entry
                .to_object(repo)?
                .into_tree()
                .map_err(|_| Error::Unsupported("指定パスはディレクトリではありません"))?
        };

        let mut entries = Vec::new();
        let mut truncated = false;

        for entry in tree.iter() {
            if entries.len() >= MAX_TREE_ENTRIES {
                truncated = true;
                break;
            }

            let Ok(name) = entry.name() else {
                continue;
            };
            let path = if dir.is_empty() {
                name.to_string()
            } else {
                format!("{dir}/{name}")
            };

            let kind = match entry.kind() {
                Some(ObjectType::Tree) => TreeEntryKind::Tree,
                Some(ObjectType::Blob) => TreeEntryKind::Blob,
                Some(ObjectType::Commit) => TreeEntryKind::Submodule,
                _ => TreeEntryKind::Other,
            };

            let size = if kind == TreeEntryKind::Blob {
                repo.find_blob(entry.id())
                    .ok()
                    .map(|blob| blob.size() as u64)
            } else {
                None
            };

            entries.push(TreeEntry {
                name: name.to_string(),
                path,
                kind,
                size,
            });
        }

        entries.sort_by(|a, b| {
            let a_dir = a.kind == TreeEntryKind::Tree;
            let b_dir = b.kind == TreeEntryKind::Tree;
            b_dir
                .cmp(&a_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        Ok(TreeEntries {
            path: dir.to_string(),
            entries,
            truncated,
        })
    }

    /// Read a file at a given commit.
    pub fn blob(&self, oid: crate::Oid, path: &str) -> Result<BlobView> {
        let repo = self.repository();
        let commit = repo
            .find_commit(oid)
            .map_err(|_| Error::CommitNotFound(oid.to_string()))?;
        let tree = commit.tree()?;
        let entry = tree.get_path(Path::new(path))?;
        let object = entry.to_object(repo)?;

        if object.kind() == Some(ObjectType::Tree) {
            return Err(Error::Unsupported("パスはファイルではありません"));
        }
        if object.kind() == Some(ObjectType::Commit) {
            return Err(Error::Unsupported("サブモジュールの内容は表示できません"));
        }

        let blob = object
            .into_blob()
            .map_err(|_| Error::Unsupported("ファイル内容を読み込めません"))?;
        let size = blob.size() as u64;
        let content = blob.content();
        let truncated = size > MAX_BLOB_BYTES;
        let visible = if truncated {
            &content[..MAX_BLOB_BYTES as usize]
        } else {
            content
        };
        let is_binary = looks_binary(visible);

        let mut lines = Vec::new();
        if !is_binary {
            let text = String::from_utf8_lossy(visible);
            for (index, line) in text.split('\n').enumerate() {
                if index >= MAX_BLOB_LINES {
                    break;
                }
                lines.push(expand_tabs(line.strip_suffix('\r').unwrap_or(line)));
            }
            // `split` yields a trailing empty line for files ending in a newline;
            // that is a real line in the editor, but showing it adds noise.
            if text.ends_with('\n') {
                lines.pop();
            }
        }

        Ok(BlobView {
            path: path.to_string(),
            oid: blob.id(),
            size,
            is_binary,
            truncated: truncated || lines.len() >= MAX_BLOB_LINES,
            lines,
        })
    }
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8_000).any(|byte| *byte == 0)
}
