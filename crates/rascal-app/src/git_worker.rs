//! Dedicated Git thread for one repository tab.
//!
//! `git2` operations are synchronous and can be expensive. The UI never calls
//! them directly; a tab owns one worker thread that opens the repository once
//! and serves requests over an async channel.

use std::path::PathBuf;

use async_channel::{Receiver, Sender};
use rascal_core::{
    BlobView, BranchInfo, CommitDetail, DiffLayer, FileDiff, HistoryPage, ImageDiff, ImageFile,
    Oid, RepoInfo, TreeEntries, WorktreeSnapshot,
};

#[allow(clippy::enum_variant_names)]
pub enum Command {
    LoadMeta {
        req: u64,
    },
    LoadHistory {
        req: u64,
        rev: String,
        offset: usize,
        limit: usize,
    },
    LoadFileHistory {
        req: u64,
        rev: String,
        path: String,
        offset: usize,
        limit: usize,
    },
    LoadFileHistoryDiff {
        req: u64,
        oid: Oid,
        path: String,
    },
    LoadImage {
        req: u64,
        oid: Oid,
        path: String,
    },
    LoadCommitImageDiff {
        req: u64,
        oid: Oid,
        old_path: Option<String>,
        new_path: Option<String>,
    },
    LoadWorktreeImageDiff {
        req: u64,
        path: String,
        old_path: Option<String>,
        layer: DiffLayer,
    },
    LoadCommit {
        req: u64,
        oid: Oid,
    },
    LoadCommitDiff {
        req: u64,
        oid: Oid,
        file_index: usize,
        old_path: Option<String>,
        new_path: Option<String>,
    },
    LoadWorktree {
        req: u64,
    },
    LoadWorktreeDiff {
        req: u64,
        path: String,
        old_path: Option<String>,
        layer: DiffLayer,
    },
    LoadTree {
        req: u64,
        oid: Oid,
        dir: String,
    },
    LoadBlob {
        req: u64,
        oid: Oid,
        path: String,
    },
}

pub enum Event {
    Ready {
        info: RepoInfo,
        branches: Vec<BranchInfo>,
    },
    Failed {
        message: String,
    },
    Meta {
        req: u64,
        info: RepoInfo,
        branches: Vec<BranchInfo>,
    },
    History {
        req: u64,
        page: HistoryPage,
    },
    FileHistory {
        req: u64,
        page: HistoryPage,
    },
    FileHistoryDiff {
        req: u64,
        diff: Option<FileDiff>,
    },
    Image {
        req: u64,
        image: ImageFile,
    },
    ImageDiff {
        req: u64,
        diff: ImageDiff,
    },
    Commit {
        req: u64,
        detail: CommitDetail,
    },
    CommitDiff {
        req: u64,
        diff: FileDiff,
    },
    Worktree {
        req: u64,
        snapshot: WorktreeSnapshot,
    },
    WorktreeDiff {
        req: u64,
        diff: FileDiff,
    },
    Tree {
        req: u64,
        oid: Oid,
        entries: TreeEntries,
    },
    Blob {
        req: u64,
        blob: BlobView,
    },
    Error {
        req: u64,
        context: &'static str,
        message: String,
    },
}

pub struct GitWorker {
    pub commands: Sender<Command>,
}

impl GitWorker {
    pub fn spawn(path: PathBuf) -> (Self, Receiver<Event>) {
        let (command_tx, command_rx) = async_channel::unbounded();
        let (event_tx, event_rx) = async_channel::unbounded();

        std::thread::Builder::new()
            .name("rascal-git".to_string())
            .spawn(move || run(path, command_rx, event_tx))
            .expect("failed to spawn Git worker");

        (
            Self {
                commands: command_tx,
            },
            event_rx,
        )
    }
}

fn run(path: PathBuf, command_rx: Receiver<Command>, event_tx: Sender<Event>) {
    let repo = match rascal_core::GitRepo::open(&path) {
        Ok(repo) => repo,
        Err(err) => {
            let _ = event_tx.send_blocking(Event::Failed {
                message: err.to_string(),
            });
            return;
        }
    };

    let info = match repo.info() {
        Ok(info) => info,
        Err(err) => {
            let _ = event_tx.send_blocking(Event::Failed {
                message: err.to_string(),
            });
            return;
        }
    };
    let branches = repo.branches().unwrap_or_default();
    if event_tx
        .send_blocking(Event::Ready { info, branches })
        .is_err()
    {
        return;
    }

    while let Ok(command) = command_rx.recv_blocking() {
        handle(&repo, command, &event_tx);
    }
}

fn handle(repo: &rascal_core::GitRepo, command: Command, event_tx: &Sender<Event>) {
    match command {
        Command::LoadMeta { req } => match (repo.info(), repo.branches()) {
            (Ok(info), Ok(branches)) => send(
                event_tx,
                Event::Meta {
                    req,
                    info,
                    branches,
                },
            ),
            (Err(err), _) | (_, Err(err)) => send_error(event_tx, req, "メタ情報", err.to_string()),
        },
        Command::LoadHistory {
            req,
            rev,
            offset,
            limit,
        } => match repo.history(&rev, offset, limit) {
            Ok(page) => send(event_tx, Event::History { req, page }),
            Err(err) => send_error(event_tx, req, "履歴", err.to_string()),
        },
        Command::LoadFileHistory {
            req,
            rev,
            path,
            offset,
            limit,
        } => match repo.file_history(&rev, &path, offset, limit) {
            Ok(page) => send(event_tx, Event::FileHistory { req, page }),
            Err(err) => send_error(event_tx, req, "ファイル編集履歴", err.to_string()),
        },
        Command::LoadFileHistoryDiff { req, oid, path } => {
            match repo.commit_file_diff(oid, &path) {
                Ok(diff) => send(event_tx, Event::FileHistoryDiff { req, diff }),
                Err(err) => send_error(event_tx, req, "ファイル編集履歴", err.to_string()),
            }
        }
        Command::LoadImage { req, oid, path } => match repo.image_file(oid, &path) {
            Ok(image) => send(event_tx, Event::Image { req, image }),
            Err(err) => send_error(event_tx, req, "ファイル内容", err.to_string()),
        },
        Command::LoadCommitImageDiff {
            req,
            oid,
            old_path,
            new_path,
        } => match repo.commit_image_diff(oid, old_path.as_deref(), new_path.as_deref()) {
            Ok(diff) => send(event_tx, Event::ImageDiff { req, diff }),
            Err(err) => send_error(event_tx, req, "差分", err.to_string()),
        },
        Command::LoadWorktreeImageDiff {
            req,
            path,
            old_path,
            layer,
        } => match repo.worktree_image_diff(&path, old_path.as_deref(), layer) {
            Ok(diff) => send(event_tx, Event::ImageDiff { req, diff }),
            Err(err) => send_error(event_tx, req, "差分", err.to_string()),
        },
        Command::LoadCommit { req, oid } => match repo.commit(oid) {
            Ok(detail) => send(event_tx, Event::Commit { req, detail }),
            Err(err) => send_error(event_tx, req, "コミット", err.to_string()),
        },
        Command::LoadCommitDiff {
            req,
            oid,
            file_index,
            old_path,
            new_path,
        } => match repo.commit_diff(oid, file_index, old_path.as_deref(), new_path.as_deref()) {
            Ok(diff) => send(event_tx, Event::CommitDiff { req, diff }),
            Err(err) => send_error(event_tx, req, "差分", err.to_string()),
        },
        Command::LoadWorktree { req } => match repo.worktree() {
            Ok(snapshot) => send(event_tx, Event::Worktree { req, snapshot }),
            Err(err) => send_error(event_tx, req, "作業ツリー", err.to_string()),
        },
        Command::LoadWorktreeDiff {
            req,
            path,
            old_path,
            layer,
        } => match repo.worktree_diff(&path, old_path.as_deref(), layer) {
            Ok(diff) => send(event_tx, Event::WorktreeDiff { req, diff }),
            Err(err) => send_error(event_tx, req, "差分", err.to_string()),
        },
        Command::LoadTree { req, oid, dir } => match repo.tree_entries(oid, &dir) {
            Ok(entries) => send(event_tx, Event::Tree { req, oid, entries }),
            Err(err) => send_error(event_tx, req, "ファイルツリー", err.to_string()),
        },
        Command::LoadBlob { req, oid, path } => match repo.blob(oid, &path) {
            Ok(blob) => send(event_tx, Event::Blob { req, blob }),
            Err(err) => send_error(event_tx, req, "ファイル内容", err.to_string()),
        },
    }
}

fn send(event_tx: &Sender<Event>, event: Event) {
    let _ = event_tx.send_blocking(event);
}

fn send_error(event_tx: &Sender<Event>, req: u64, context: &'static str, message: String) {
    let _ = event_tx.send_blocking(Event::Error {
        req,
        context,
        message,
    });
}
