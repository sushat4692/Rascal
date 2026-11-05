//! Plain data types shared with the UI.

use std::path::PathBuf;

use crate::Oid;

#[derive(Clone, Debug, Default)]
pub struct WorktreeCounts {
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
}

impl WorktreeCounts {
    pub fn total(&self) -> usize {
        self.staged + self.unstaged + self.untracked
    }

    pub fn is_clean(&self) -> bool {
        self.total() == 0
    }
}

#[derive(Clone, Debug)]
pub enum HeadState {
    Attached {
        branch: String,
        oid: Oid,
        upstream: Option<String>,
        ahead: usize,
        behind: usize,
    },
    Detached {
        oid: Oid,
    },
    Unborn,
}

impl HeadState {
    pub fn oid(&self) -> Option<Oid> {
        match self {
            HeadState::Attached { oid, .. } | HeadState::Detached { oid } => Some(*oid),
            HeadState::Unborn => None,
        }
    }

    pub fn label(&self) -> String {
        match self {
            HeadState::Attached { branch, .. } => branch.clone(),
            HeadState::Detached { oid } => format!("detached @ {}", short_oid(*oid)),
            HeadState::Unborn => "unborn".to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RepoInfo {
    pub name: String,
    pub workdir: Option<PathBuf>,
    pub git_dir: PathBuf,
    pub head: HeadState,
    pub is_bare: bool,
    pub counts: WorktreeCounts,
}

#[derive(Clone, Debug)]
pub struct BranchInfo {
    pub name: String,
    pub full_ref: String,
    pub tip: Oid,
    pub is_head: bool,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    Head,
    LocalBranch,
    RemoteBranch,
    Tag,
}

#[derive(Clone, Debug)]
pub struct RefLabel {
    pub kind: RefKind,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct CommitSummary {
    pub oid: Oid,
    pub summary: String,
    pub author_name: String,
    pub author_email: String,
    pub author_time: i64,
    pub committer_time: i64,
    pub parents: Vec<Oid>,
    pub refs: Vec<RefLabel>,
}

#[derive(Clone, Debug)]
pub struct HistoryPage {
    pub commits: Vec<CommitSummary>,
    pub has_more: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
    Copied,
    TypeChange,
    Untracked,
    Unmodified,
    Other,
}

impl FileStatus {
    pub fn code(self) -> &'static str {
        match self {
            FileStatus::Added => "A",
            FileStatus::Deleted => "D",
            FileStatus::Modified => "M",
            FileStatus::Renamed => "R",
            FileStatus::Copied => "C",
            FileStatus::TypeChange => "T",
            FileStatus::Untracked => "?",
            FileStatus::Unmodified => " ",
            FileStatus::Other => "•",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct CommitPerson {
    pub name: String,
    pub email: String,
    pub time: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffBase {
    Root,
    FirstParent,
}

#[derive(Clone, Debug)]
pub struct FileChange {
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub additions: usize,
    pub deletions: usize,
    pub is_binary: bool,
    pub is_submodule: bool,
    pub stats_loaded: bool,
}

#[derive(Clone, Debug)]
pub struct CommitDetail {
    pub oid: Oid,
    pub summary: String,
    pub message: String,
    pub author: CommitPerson,
    pub committer: CommitPerson,
    pub parents: Vec<Oid>,
    pub files: Vec<FileChange>,
    pub files_truncated: bool,
    pub base: DiffBase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffRowKind {
    HunkHeader,
    Context,
    Added,
    Removed,
    NoNewline,
    Meta,
}

#[derive(Clone, Debug)]
pub struct DiffRow {
    pub kind: DiffRowKind,
    pub old_lineno: Option<u32>,
    pub new_lineno: Option<u32>,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct ImageFile {
    pub path: String,
    pub oid: Option<Oid>,
    pub size: u64,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct ImageDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub base_label: String,
    pub old: Option<ImageFile>,
    pub new: Option<ImageFile>,
}

#[derive(Clone, Debug)]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub is_binary: bool,
    pub is_submodule: bool,
    pub base_label: String,
    pub additions: usize,
    pub deletions: usize,
    pub rows: Vec<DiffRow>,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLayer {
    Staged,
    Unstaged,
    Untracked,
}

impl DiffLayer {
    pub fn label(self) -> &'static str {
        match self {
            DiffLayer::Staged => "ステージ済み",
            DiffLayer::Unstaged => "未ステージ",
            DiffLayer::Untracked => "未追跡",
        }
    }
}

#[derive(Clone, Debug)]
pub struct WorktreeFile {
    pub path: String,
    pub old_path: Option<String>,
    pub layer: DiffLayer,
    pub status: FileStatus,
    pub is_binary: bool,
}

#[derive(Clone, Debug)]
pub struct WorktreeSnapshot {
    pub counts: WorktreeCounts,
    pub files: Vec<WorktreeFile>,
}

#[derive(Clone, Debug)]
pub struct WorktreeFileRef {
    pub path: String,
    pub old_path: Option<String>,
    pub layer: DiffLayer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeEntryKind {
    Tree,
    Blob,
    Submodule,
    Other,
}

#[derive(Clone, Debug)]
pub struct TreeEntry {
    pub name: String,
    pub path: String,
    pub kind: TreeEntryKind,
    pub size: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct TreeEntries {
    pub path: String,
    pub entries: Vec<TreeEntry>,
    pub truncated: bool,
}

#[derive(Clone, Debug)]
pub struct BlobView {
    pub path: String,
    pub oid: Oid,
    pub size: u64,
    pub is_binary: bool,
    pub truncated: bool,
    pub lines: Vec<String>,
}

pub fn short_oid(oid: Oid) -> String {
    let mut s = oid.to_string();
    s.truncate(7);
    s
}

pub fn short_oid_str(oid: &str) -> String {
    oid.chars().take(7).collect()
}
