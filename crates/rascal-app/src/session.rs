//! One repository tab: state, Git worker events and the history/diff UI.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
};

use chrono::{Local, TimeZone};
use gpui::ClipboardItem;
use gpui::StatefulInteractiveElement;
use gpui::prelude::*;
use gpui::*;
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_component::notification::Notification;
use gpui_component::resizable::{h_resizable, resizable_panel, v_resizable};
use gpui_component::theme::ActiveTheme;
use gpui_component::{Icon, IconName, WindowExt};
use rascal_core::{
    BlobView, BranchInfo, CommitDetail, CommitSummary, DiffLayer, FileDiff, FileStatus, GraphRow,
    HISTORY_PAGE_SIZE, HeadState, HistoryPage, Oid, RefKind, RepoInfo, TreeEntries, TreeEntryKind,
    WorktreeCounts, WorktreeSnapshot, graph,
};

use crate::diff_theme::{DiffPalette, DiffTheme, palette as diff_palette};
use crate::file_history::FileHistoryView;
use crate::git_worker::{Command, Event, GitWorker};
use crate::i18n::LanguageExt;
use crate::ui::diff_view::{self, DiffDisplayRow, DiffLayout};
use crate::ui::graph::{gutter_width_for, lane_color};
use crate::ui::image_view::{self, ImageView};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileListMode {
    Changes,
    Files,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChangesView {
    List,
    Tree,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    None,
    WorkingTree,
    Commit(Oid),
}

#[derive(Clone, Copy, Debug)]
enum FileSource {
    CommitDiff { index: usize },
    Worktree { layer: DiffLayer },
    TreeFile,
}

#[derive(Clone, Debug)]
struct SelectedFile {
    path: String,
    old_path: Option<String>,
    source: FileSource,
}

#[derive(Clone, Debug)]
enum FileTarget {
    CommitFile(usize),
    WorktreeFile(usize),
    TreeDir(String),
    TreeFile(String),
}

#[derive(Clone, Debug)]
enum ViewerRow {
    Diff(DiffDisplayRow),
    Blob { number: usize, text: String },
    Info(String),
}

#[derive(Clone, Debug)]
enum HistoryRow {
    WorkingTree {
        counts: WorktreeCounts,
    },
    Commit {
        index: usize,
        oid: Oid,
        summary: String,
        author: String,
        time: i64,
        refs: Vec<rascal_core::RefLabel>,
        graph: Option<GraphRow>,
    },
    LoadMore,
}

#[derive(Clone, Debug)]
enum FileRow {
    Group(String),
    Change {
        target: FileTarget,
        path: String,
        display: String,
        depth: usize,
        old_path: Option<String>,
        status: FileStatus,
        additions: Option<usize>,
        deletions: Option<usize>,
        binary: bool,
        layer: Option<DiffLayer>,
    },
    ChangeDir {
        key: String,
        name: String,
        depth: usize,
        expanded: bool,
        count: usize,
    },
    Tree {
        target: FileTarget,
        name: String,
        depth: usize,
        is_dir: bool,
        expanded: bool,
        loading: bool,
        size: Option<u64>,
    },
    Message(String),
}

#[derive(Default)]
struct ChangeTree {
    dirs: BTreeMap<String, ChangeTree>,
    files: Vec<(String, ChangeLeaf)>,
}

#[derive(Clone)]
struct ChangeLeaf {
    target: FileTarget,
    path: String,
    old_path: Option<String>,
    status: FileStatus,
    additions: Option<usize>,
    deletions: Option<usize>,
    binary: bool,
    layer: Option<DiffLayer>,
}

impl ChangeTree {
    fn insert(&mut self, path: &str, leaf: ChangeLeaf) {
        let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        let Some((name, dirs)) = parts.split_last() else {
            return;
        };
        let mut node = self;
        for part in dirs {
            node = node.dirs.entry((*part).to_string()).or_default();
        }
        node.files.push(((*name).to_string(), leaf));
    }

    fn file_count(&self) -> usize {
        self.files.len()
            + self
                .dirs
                .values()
                .map(ChangeTree::file_count)
                .sum::<usize>()
    }
}

pub struct RepoSession {
    pub id: u64,
    pub path: PathBuf,
    info: Option<RepoInfo>,
    branches: Vec<BranchInfo>,
    opening: bool,
    open_error: Option<String>,

    history: Vec<CommitSummary>,
    graph_rows: Vec<GraphRow>,
    history_rev: String,
    history_has_more: bool,
    history_loading: bool,
    history_error: Option<String>,
    pending_history_offset: usize,

    filter_input: Entity<InputState>,
    filter: String,
    selected: Selection,

    detail: Option<CommitDetail>,
    detail_loading: bool,
    detail_error: Option<String>,
    message_expanded: bool,

    file_mode: FileListMode,
    changes_view: ChangesView,
    collapsed_change_dirs: HashSet<String>,
    worktree: Option<WorktreeSnapshot>,
    worktree_loading: bool,
    worktree_error: Option<String>,

    selected_file: Option<SelectedFile>,
    diff: Option<FileDiff>,
    blob: Option<BlobView>,
    viewer_loading: bool,
    viewer_error: Option<String>,
    viewer_rows: Rc<Vec<ViewerRow>>,
    viewer_list: ListState,
    viewer_layout: DiffLayout,
    image_view: Option<Entity<ImageView>>,
    image_loading: bool,
    image_error: Option<String>,
    image_req: u64,

    tree_cache: HashMap<String, TreeEntries>,
    tree_loading: HashSet<String>,
    expanded_dirs: HashSet<String>,
    tree_error: Option<String>,

    commands: async_channel::Sender<Command>,
    next_req: u64,
    meta_req: u64,
    history_req: u64,
    commit_req: u64,
    diff_req: u64,
    tree_req: u64,
    blob_req: u64,
    worktree_req: u64,

    history_focus: FocusHandle,
    history_scroll: UniformListScrollHandle,
    file_scroll: UniformListScrollHandle,

    _subscriptions: Vec<Subscription>,
}

impl RepoSession {
    pub fn new(id: u64, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter_placeholder = cx.tr("コミットを絞り込む");
        let filter_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx);
            state.set_placeholder(filter_placeholder.clone(), window, cx);
            state
        });

        let filter_subscription =
            cx.subscribe_in(&filter_input, window, |this, state, event, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.filter = state.read(cx).value().to_string();
                    cx.notify();
                }
            });

        let history_focus = cx.focus_handle();
        let history_scroll = UniformListScrollHandle::new();
        let file_scroll = UniformListScrollHandle::new();
        let viewer_list = ListState::new(0, ListAlignment::Top, px(1000.0));

        let activation_subscription = cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.request_meta(cx);
            }
        });

        let (worker, events) = GitWorker::spawn(path.clone());
        cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                if this
                    .update(cx, |session, cx| session.handle_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let mut session = Self {
            id,
            path,
            info: None,
            branches: Vec::new(),
            opening: true,
            open_error: None,
            history: Vec::new(),
            graph_rows: Vec::new(),
            history_rev: "HEAD".to_string(),
            history_has_more: false,
            history_loading: false,
            history_error: None,
            pending_history_offset: 0,
            filter_input,
            filter: String::new(),
            selected: Selection::None,
            detail: None,
            detail_loading: false,
            detail_error: None,
            message_expanded: false,
            file_mode: FileListMode::Changes,
            changes_view: ChangesView::List,
            collapsed_change_dirs: HashSet::new(),
            worktree: None,
            worktree_loading: false,
            worktree_error: None,
            selected_file: None,
            diff: None,
            blob: None,
            viewer_loading: false,
            viewer_error: None,
            viewer_rows: Rc::new(Vec::new()),
            viewer_list,
            viewer_layout: cx.try_global::<DiffLayout>().copied().unwrap_or_default(),
            image_view: None,
            image_loading: false,
            image_error: None,
            image_req: 0,
            tree_cache: HashMap::new(),
            tree_loading: HashSet::new(),
            expanded_dirs: HashSet::new(),
            tree_error: None,
            commands: worker.commands,
            next_req: 0,
            meta_req: 0,
            history_req: 0,
            commit_req: 0,
            diff_req: 0,
            tree_req: 0,
            blob_req: 0,
            worktree_req: 0,
            history_focus: history_focus.clone(),
            history_scroll,
            file_scroll,
            _subscriptions: vec![filter_subscription, activation_subscription],
        };

        history_focus.focus(window);
        session.request_meta(cx);
        session
    }

    pub fn tab_title(&self) -> String {
        self.info
            .as_ref()
            .map(|info| info.name.clone())
            .or_else(|| {
                self.path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "repository".to_string())
    }

    pub fn refresh(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.request_meta(cx);
        self.request_worktree(cx);
        self.request_history(0, cx);
        if let Selection::Commit(oid) = self.selected {
            self.select_commit(oid, cx);
        } else if self.selected == Selection::WorkingTree {
            self.selected_file = None;
            self.clear_viewer();
            self.collapsed_change_dirs.clear();
            self.auto_select_first_file(cx);
        }
    }

    pub fn focus_history(&self, window: &mut Window, _cx: &mut App) {
        self.history_focus.focus(window);
    }

    pub fn language_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let placeholder = cx.tr("コミットを絞り込む");
        self.filter_input.update(cx, |state, cx| {
            state.set_placeholder(placeholder.clone(), window, cx);
        });
        if let Some(diff) = self.diff.clone() {
            self.set_diff(diff, cx);
        }
        if let Some(blob) = self.blob.clone() {
            self.set_blob(blob, cx);
        }
        cx.notify();
    }

    pub fn focus_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter_input
            .update(cx, |state, cx| state.focus(window, cx));
    }

    // ---- worker plumbing -------------------------------------------------

    fn next_req(&mut self) -> u64 {
        self.next_req += 1;
        self.next_req
    }

    fn request_meta(&mut self, cx: &mut Context<Self>) {
        let req = self.next_req();
        self.meta_req = req;
        let _ = self.commands.send_blocking(Command::LoadMeta { req });
        cx.notify();
    }

    fn request_worktree(&mut self, cx: &mut Context<Self>) {
        if self.info.as_ref().is_some_and(|info| info.is_bare) {
            return;
        }
        let req = self.next_req();
        self.worktree_req = req;
        self.worktree_loading = true;
        self.worktree_error = None;
        let _ = self.commands.send_blocking(Command::LoadWorktree { req });
        cx.notify();
    }

    fn request_history(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self
            .info
            .as_ref()
            .is_some_and(|info| matches!(info.head, HeadState::Unborn))
        {
            self.history.clear();
            self.graph_rows.clear();
            self.history_loading = false;
            self.history_has_more = false;
            cx.notify();
            return;
        }

        let req = self.next_req();
        self.history_req = req;
        self.pending_history_offset = offset;
        self.history_loading = true;
        self.history_error = None;
        if offset == 0 {
            self.history.clear();
            self.graph_rows.clear();
            self.history_has_more = false;
        }
        let _ = self.commands.send_blocking(Command::LoadHistory {
            req,
            rev: self.history_rev.clone(),
            offset,
            limit: HISTORY_PAGE_SIZE,
        });
        cx.notify();
    }

    fn load_more(&mut self, cx: &mut Context<Self>) {
        if self.history_loading || !self.history_has_more {
            return;
        }
        self.request_history(self.history.len(), cx);
    }

    fn request_commit(&mut self, oid: Oid, cx: &mut Context<Self>) {
        let req = self.next_req();
        self.commit_req = req;
        self.detail_loading = true;
        let _ = self
            .commands
            .send_blocking(Command::LoadCommit { req, oid });
        cx.notify();
    }

    fn request_commit_diff(
        &mut self,
        oid: Oid,
        index: usize,
        old_path: Option<String>,
        new_path: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if new_path
            .as_deref()
            .or(old_path.as_deref())
            .is_some_and(image_view::is_image_path)
        {
            self.request_commit_image_diff(oid, old_path, new_path, cx);
            return;
        }
        let req = self.next_req();
        self.diff_req = req;
        self.viewer_loading = true;
        self.viewer_error = None;
        self.diff = None;
        self.blob = None;
        self.image_view = None;
        self.image_loading = false;
        self.image_error = None;
        self.image_req = 0;
        self.set_viewer_rows(Vec::new());
        let _ = self.commands.send_blocking(Command::LoadCommitDiff {
            req,
            oid,
            file_index: index,
            old_path,
            new_path,
        });
        cx.notify();
    }

    fn request_worktree_diff(
        &mut self,
        path: String,
        old_path: Option<String>,
        layer: DiffLayer,
        cx: &mut Context<Self>,
    ) {
        if image_view::is_image_path(&path)
            || old_path.as_deref().is_some_and(image_view::is_image_path)
        {
            self.request_worktree_image_diff(path, old_path, layer, cx);
            return;
        }
        let req = self.next_req();
        self.diff_req = req;
        self.viewer_loading = true;
        self.viewer_error = None;
        self.diff = None;
        self.blob = None;
        self.image_view = None;
        self.image_loading = false;
        self.image_error = None;
        self.image_req = 0;
        self.set_viewer_rows(Vec::new());
        let _ = self.commands.send_blocking(Command::LoadWorktreeDiff {
            req,
            path,
            old_path,
            layer,
        });
        cx.notify();
    }

    fn request_commit_image_diff(
        &mut self,
        oid: Oid,
        old_path: Option<String>,
        new_path: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let req = self.next_req();
        self.image_req = req;
        self.image_loading = true;
        self.image_error = None;
        self.image_view = None;
        self.viewer_loading = false;
        self.viewer_error = None;
        self.diff = None;
        self.blob = None;
        self.diff_req = 0;
        self.set_viewer_rows(Vec::new());
        let _ = self.commands.send_blocking(Command::LoadCommitImageDiff {
            req,
            oid,
            old_path,
            new_path,
        });
        cx.notify();
    }

    fn request_worktree_image_diff(
        &mut self,
        path: String,
        old_path: Option<String>,
        layer: DiffLayer,
        cx: &mut Context<Self>,
    ) {
        let req = self.next_req();
        self.image_req = req;
        self.image_loading = true;
        self.image_error = None;
        self.image_view = None;
        self.viewer_loading = false;
        self.viewer_error = None;
        self.diff = None;
        self.blob = None;
        self.diff_req = 0;
        self.set_viewer_rows(Vec::new());
        let _ = self.commands.send_blocking(Command::LoadWorktreeImageDiff {
            req,
            path,
            old_path,
            layer,
        });
        cx.notify();
    }

    fn request_image(&mut self, oid: Oid, path: String, cx: &mut Context<Self>) {
        let req = self.next_req();
        self.image_req = req;
        self.image_loading = true;
        self.image_error = None;
        self.image_view = None;
        self.viewer_loading = false;
        self.viewer_error = None;
        self.diff = None;
        self.blob = None;
        self.diff_req = 0;
        self.set_viewer_rows(Vec::new());
        let _ = self
            .commands
            .send_blocking(Command::LoadImage { req, oid, path });
        cx.notify();
    }

    fn request_tree(&mut self, oid: Oid, dir: String, cx: &mut Context<Self>) {
        if self.tree_cache.contains_key(&dir) || self.tree_loading.contains(&dir) {
            return;
        }
        let req = self.next_req();
        self.tree_req = req;
        self.tree_error = None;
        self.tree_loading.insert(dir.clone());
        let _ = self
            .commands
            .send_blocking(Command::LoadTree { req, oid, dir });
        cx.notify();
    }

    fn request_blob(&mut self, oid: Oid, path: String, cx: &mut Context<Self>) {
        if image_view::is_image_path(&path) {
            self.request_image(oid, path, cx);
            return;
        }
        let req = self.next_req();
        self.blob_req = req;
        self.viewer_loading = true;
        self.viewer_error = None;
        self.diff = None;
        self.blob = None;
        self.image_view = None;
        self.image_loading = false;
        self.image_error = None;
        self.image_req = 0;
        self.set_viewer_rows(Vec::new());
        let _ = self
            .commands
            .send_blocking(Command::LoadBlob { req, oid, path });
        cx.notify();
    }

    // ---- state transitions ----------------------------------------------

    fn handle_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Ready { info, branches } => {
                self.opening = false;
                self.open_error = None;
                self.info = Some(info);
                self.branches = branches;
                self.request_history(0, cx);
                self.request_worktree(cx);
            }
            Event::Failed { message } => {
                self.opening = false;
                self.open_error = Some(message);
                cx.notify();
            }
            Event::Meta {
                req,
                info,
                branches,
            } => {
                if req != self.meta_req {
                    return;
                }
                self.info = Some(info);
                self.branches = branches;
                cx.notify();
            }
            Event::FileHistory { .. } | Event::FileHistoryDiff { .. } => {}
            Event::History { req, page } => {
                if req != self.history_req {
                    return;
                }
                self.history_loading = false;
                self.history_error = None;
                let append = self.pending_history_offset > 0;
                let HistoryPage { commits, has_more } = page;
                if append {
                    for commit in commits {
                        if !self
                            .history
                            .iter()
                            .any(|existing| existing.oid == commit.oid)
                        {
                            self.history.push(commit);
                        }
                    }
                } else {
                    self.history = commits;
                }
                self.history_has_more = has_more;
                self.graph_rows = graph::layout(&self.history);

                if self.selected == Selection::None {
                    if let Some(first) = self.history.first() {
                        let oid = first.oid;
                        self.select_commit(oid, cx);
                    } else if self
                        .worktree
                        .as_ref()
                        .is_some_and(|wt| !wt.counts.is_clean())
                    {
                        self.select_worktree(cx);
                    }
                } else if let Selection::Commit(oid) = self.selected
                    && !self.history.iter().any(|commit| commit.oid == oid)
                    && let Some(first) = self.history.first()
                {
                    let oid = first.oid;
                    self.select_commit(oid, cx);
                }
                cx.notify();
            }
            Event::Commit { req, detail } => {
                if req != self.commit_req {
                    return;
                }
                self.detail_loading = false;
                self.detail_error = None;
                self.detail = Some(detail.clone());
                if self.file_mode == FileListMode::Files {
                    self.ensure_tree_root(cx);
                } else {
                    self.auto_select_first_commit_file(cx);
                }
                cx.notify();
            }
            Event::Image { req, image } => {
                if req != self.image_req {
                    return;
                }
                self.image_loading = false;
                self.image_error = None;
                self.image_view = Some(ImageView::single(image, cx));
                cx.notify();
            }
            Event::ImageDiff { req, diff } => {
                if req != self.image_req {
                    return;
                }
                self.image_loading = false;
                self.image_error = None;
                self.image_view = Some(ImageView::diff(diff, cx));
                cx.notify();
            }
            Event::CommitDiff { req, diff } => {
                if req != self.diff_req {
                    return;
                }
                self.viewer_loading = false;
                self.set_diff(diff, cx);
                cx.notify();
            }
            Event::Worktree { req, snapshot } => {
                if req != self.worktree_req {
                    return;
                }
                self.worktree_loading = false;
                self.worktree_error = None;
                let is_clean = snapshot.counts.is_clean();
                self.worktree = Some(snapshot);
                if self.selected == Selection::WorkingTree {
                    self.auto_select_first_file(cx);
                } else if self.selected == Selection::None && self.history.is_empty() && !is_clean {
                    self.select_worktree(cx);
                }
                cx.notify();
            }
            Event::WorktreeDiff { req, diff } => {
                if req != self.diff_req {
                    return;
                }
                self.viewer_loading = false;
                self.set_diff(diff, cx);
                cx.notify();
            }
            Event::Tree { req, oid, entries } => {
                let _ = req;
                if self.selected != Selection::Commit(oid) {
                    return;
                }
                self.tree_loading.remove(&entries.path);
                self.tree_cache.insert(entries.path.clone(), entries);
                self.tree_error = None;
                cx.notify();
            }
            Event::Blob { req, blob } => {
                if req != self.blob_req {
                    return;
                }
                self.viewer_loading = false;
                self.set_blob(blob, cx);
                cx.notify();
            }
            Event::Error {
                req,
                context,
                message,
            } => {
                if req == self.image_req {
                    self.image_loading = false;
                    self.image_error = Some(message);
                    cx.notify();
                    return;
                }
                let _ = req;
                match context {
                    "履歴" => {
                        self.history_loading = false;
                        self.history_error = Some(message);
                    }
                    "コミット" => {
                        self.detail_loading = false;
                        self.detail_error = Some(message);
                    }
                    "差分" => {
                        self.viewer_loading = false;
                        self.viewer_error = Some(message);
                    }
                    "ファイルツリー" => {
                        self.tree_loading.clear();
                        self.tree_error = Some(message);
                    }
                    "ファイル内容" => {
                        self.viewer_loading = false;
                        self.viewer_error = Some(message);
                    }
                    "作業ツリー" => {
                        self.worktree_loading = false;
                        self.worktree_error = Some(message);
                    }
                    "メタ情報" => {
                        // Transient metadata failures should not replace the viewer.
                    }
                    _ => {
                        self.viewer_loading = false;
                        self.viewer_error = Some(message);
                    }
                }
                cx.notify();
            }
        }
    }

    fn select_rev(&mut self, rev: String, cx: &mut Context<Self>) {
        if self.history_rev == rev {
            return;
        }
        self.history_rev = rev;
        self.selected = Selection::None;
        self.detail = None;
        self.detail_error = None;
        self.selected_file = None;
        self.clear_viewer();
        self.request_history(0, cx);
    }

    fn select_commit(&mut self, oid: Oid, cx: &mut Context<Self>) {
        if self.selected == Selection::Commit(oid) && self.detail.is_some() {
            return;
        }
        self.selected = Selection::Commit(oid);
        self.detail = None;
        self.detail_loading = true;
        self.detail_error = None;
        self.message_expanded = false;
        self.selected_file = None;
        self.clear_viewer();
        self.tree_cache.clear();
        self.expanded_dirs.clear();
        self.tree_loading.clear();
        self.tree_error = None;
        self.collapsed_change_dirs.clear();
        self.request_commit(oid, cx);
    }

    fn select_worktree(&mut self, cx: &mut Context<Self>) {
        self.selected = Selection::WorkingTree;
        self.detail = None;
        self.detail_loading = false;
        self.detail_error = None;
        self.message_expanded = false;
        self.selected_file = None;
        self.clear_viewer();
        self.collapsed_change_dirs.clear();
        if self.worktree.is_none() {
            self.request_worktree(cx);
        } else {
            self.auto_select_first_file(cx);
        }
        cx.notify();
    }

    fn clear_viewer(&mut self) {
        self.diff = None;
        self.blob = None;
        self.image_view = None;
        self.image_loading = false;
        self.image_error = None;
        self.image_req = 0;
        self.diff_req = 0;
        self.viewer_error = None;
        self.viewer_loading = false;
        self.set_viewer_rows(Vec::new());
    }

    fn set_diff(&mut self, diff: FileDiff, cx: &App) {
        let layout = diff_view::current_layout(cx);
        let rows = diff_view::build_rows(&diff, layout)
            .into_iter()
            .map(ViewerRow::Diff)
            .collect();
        self.viewer_layout = layout;
        self.diff = Some(diff);
        self.blob = None;
        self.image_view = None;
        self.image_loading = false;
        self.image_error = None;
        self.set_viewer_rows(rows);
        self.viewer_list.scroll_to(ListOffset {
            item_ix: 0,
            offset_in_item: px(0.0),
        });
    }

    fn rebuild_viewer_rows(&mut self, cx: &mut Context<Self>) {
        let layout = diff_view::current_layout(cx);
        let Some(diff) = self.diff.as_ref() else {
            return;
        };
        let rows = diff_view::build_rows(diff, layout)
            .into_iter()
            .map(ViewerRow::Diff)
            .collect();
        self.viewer_layout = layout;
        self.set_viewer_rows(rows);
        self.viewer_list.scroll_to(ListOffset {
            item_ix: 0,
            offset_in_item: px(0.0),
        });
        cx.notify();
    }

    fn set_blob(&mut self, blob: BlobView, cx: &App) {
        let mut rows = Vec::new();
        if blob.is_binary {
            let message = if cx.is_japanese() {
                format!("バイナリファイルのため表示できません ({} bytes)", blob.size)
            } else {
                format!("Binary file cannot be displayed ({} bytes).", blob.size)
            };
            rows.push(ViewerRow::Info(message));
        } else {
            rows.extend(
                blob.lines
                    .iter()
                    .enumerate()
                    .map(|(index, text)| ViewerRow::Blob {
                        number: index + 1,
                        text: text.clone(),
                    }),
            );
            if blob.truncated {
                rows.push(ViewerRow::Info(
                    cx.tr("ファイルが大きいため、先頭のみ表示しています。")
                        .to_string(),
                ));
            }
        }
        self.blob = Some(blob);
        self.diff = None;
        self.image_view = None;
        self.image_loading = false;
        self.image_error = None;
        self.set_viewer_rows(rows);
    }

    fn set_viewer_rows(&mut self, rows: Vec<ViewerRow>) {
        self.viewer_list.reset(rows.len());
        self.viewer_rows = Rc::new(rows);
    }

    fn ensure_tree_root(&mut self, cx: &mut Context<Self>) {
        if let Selection::Commit(oid) = self.selected {
            self.request_tree(oid, String::new(), cx);
        }
    }

    fn select_commit_file(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(oid) = self.selected_commit() else {
            return;
        };
        let Some(file) = self
            .detail
            .as_ref()
            .and_then(|detail| detail.files.get(index))
            .cloned()
        else {
            return;
        };
        self.selected_file = Some(SelectedFile {
            path: file.path.clone(),
            old_path: file.old_path.clone(),
            source: FileSource::CommitDiff { index },
        });
        self.request_commit_diff(
            oid,
            index,
            file.old_path.clone(),
            Some(file.path.clone()),
            cx,
        );
    }

    fn select_worktree_file(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(file) = self
            .worktree
            .as_ref()
            .and_then(|snapshot| snapshot.files.get(index))
            .cloned()
        else {
            return;
        };
        self.selected_file = Some(SelectedFile {
            path: file.path.clone(),
            old_path: file.old_path.clone(),
            source: FileSource::Worktree { layer: file.layer },
        });
        self.request_worktree_diff(file.path, file.old_path, file.layer, cx);
    }

    fn select_tree_file(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(oid) = self.selected_commit() else {
            return;
        };
        let _ = oid;
        self.selected_file = Some(SelectedFile {
            path: path.clone(),
            old_path: None,
            source: FileSource::TreeFile,
        });
        self.request_blob(oid, path, cx);
    }

    fn auto_select_first_commit_file(&mut self, cx: &mut Context<Self>) {
        if self.file_mode == FileListMode::Files {
            self.ensure_tree_root(cx);
            return;
        }
        if let Some(detail) = &self.detail {
            if !detail.files.is_empty() {
                self.select_commit_file(0, cx);
            } else {
                self.selected_file = None;
                self.clear_viewer();
            }
        }
    }

    fn auto_select_first_file(&mut self, cx: &mut Context<Self>) {
        match self.selected {
            Selection::Commit(_) => self.auto_select_first_commit_file(cx),
            Selection::WorkingTree => {
                if self.file_mode == FileListMode::Files {
                    return;
                }
                if let Some(snapshot) = &self.worktree {
                    if !snapshot.files.is_empty() {
                        self.select_worktree_file(0, cx);
                    } else {
                        self.selected_file = None;
                        self.clear_viewer();
                    }
                }
            }
            Selection::None => {}
        }
    }

    fn set_file_mode(&mut self, mode: FileListMode, cx: &mut Context<Self>) {
        if mode == FileListMode::Files && self.selected == Selection::WorkingTree {
            return;
        }
        if self.file_mode == mode {
            return;
        }
        self.file_mode = mode;
        self.selected_file = None;
        self.clear_viewer();
        match mode {
            FileListMode::Changes => self.auto_select_first_file(cx),
            FileListMode::Files => self.ensure_tree_root(cx),
        }
        cx.notify();
    }

    fn toggle_dir(&mut self, path: String, cx: &mut Context<Self>) {
        if self.expanded_dirs.contains(&path) {
            self.expanded_dirs.remove(&path);
        } else {
            self.expanded_dirs.insert(path.clone());
            if let Selection::Commit(oid) = self.selected {
                self.request_tree(oid, path, cx);
            }
        }
        cx.notify();
    }

    fn on_history_row(&mut self, row: HistoryRow, cx: &mut Context<Self>) {
        match row {
            HistoryRow::WorkingTree { .. } => self.select_worktree(cx),
            HistoryRow::Commit { oid, .. } => self.select_commit(oid, cx),
            HistoryRow::LoadMore => self.load_more(cx),
        }
    }

    fn on_file_row(&mut self, target: FileTarget, cx: &mut Context<Self>) {
        match target {
            FileTarget::CommitFile(index) => self.select_commit_file(index, cx),
            FileTarget::WorktreeFile(index) => self.select_worktree_file(index, cx),
            FileTarget::TreeDir(path) => self.toggle_dir(path, cx),
            FileTarget::TreeFile(path) => self.select_tree_file(path, cx),
        }
    }

    fn history_rev_label(&self) -> String {
        if self.history_rev == "HEAD" {
            "HEAD".to_string()
        } else {
            self.history_rev
                .strip_prefix("refs/heads/")
                .unwrap_or(&self.history_rev)
                .to_string()
        }
    }

    fn selected_commit(&self) -> Option<Oid> {
        match self.selected {
            Selection::Commit(oid) => Some(oid),
            _ => None,
        }
    }

    fn move_selection(&mut self, delta: i32, cx: &mut Context<Self>) {
        let oids: Vec<Oid> = self
            .history
            .iter()
            .filter(|commit| self.commit_matches_filter(commit))
            .map(|commit| commit.oid)
            .collect();
        if oids.is_empty() {
            return;
        }

        let current = match self.selected {
            Selection::Commit(oid) => oids.iter().position(|candidate| *candidate == oid),
            Selection::WorkingTree | Selection::None => None,
        };
        let next = match current {
            Some(index) => (index as i32 + delta).clamp(0, oids.len() as i32 - 1) as usize,
            None => 0,
        };
        let oid = oids[next];
        let row_index = next + usize::from(self.has_worktree_row());
        self.history_scroll
            .scroll_to_item(row_index, ScrollStrategy::Center);
        self.select_commit(oid, cx);
    }

    fn commit_matches_filter(&self, commit: &CommitSummary) -> bool {
        let needle = self.filter.trim().to_lowercase();
        if needle.is_empty() {
            return true;
        }
        let oid = commit.oid.to_string();
        commit.summary.to_lowercase().contains(&needle)
            || commit.author_name.to_lowercase().contains(&needle)
            || oid.starts_with(&needle)
    }

    fn has_worktree_row(&self) -> bool {
        self.info
            .as_ref()
            .map(|info| !info.counts.is_clean())
            .unwrap_or(false)
    }

    // ---- render ---------------------------------------------------------

    fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let entity = cx.entity();
        let info = self.info.as_ref();
        let head = info.map(|info| info.head.label()).unwrap_or_default();
        let counts = info.map(|info| info.counts.clone()).unwrap_or_default();
        let mut branch_rows = Vec::new();

        let head_selected = self.history_rev == "HEAD";
        branch_rows.push(
            div()
                .id(ElementId::Name(format!("branch-head-{}", self.id).into()))
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .rounded(px(5.0))
                .cursor_pointer()
                .when(head_selected, |this| this.bg(theme.list_active))
                .hover(|this| this.bg(theme.list_hover))
                .on_click({
                    let entity = entity.clone();
                    move |_, _, cx| {
                        entity.update(cx, |this, cx| this.select_rev("HEAD".into(), cx));
                    }
                })
                .child(div().w(px(7.0)).h(px(7.0)).rounded_full().bg(theme.primary))
                .child(div().flex_1().text_sm().child("HEAD"))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .truncate()
                        .child(head),
                )
                .into_any_element(),
        );

        for branch in &self.branches {
            let selected = self.history_rev == branch.full_ref;
            let full_ref = branch.full_ref.clone();
            let ahead_behind = if branch.ahead > 0 || branch.behind > 0 {
                format!("↑{} ↓{}", branch.ahead, branch.behind)
            } else {
                String::new()
            };
            branch_rows.push(
                div()
                    .id(ElementId::Name(
                        format!("branch-{}-{}", self.id, branch.name).into(),
                    ))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded(px(5.0))
                    .cursor_pointer()
                    .when(selected, |this| this.bg(theme.list_active))
                    .hover(|this| this.bg(theme.list_hover))
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            let rev = full_ref.clone();
                            entity.update(cx, |this, cx| this.select_rev(rev, cx));
                        }
                    })
                    .child(
                        div()
                            .w(px(7.0))
                            .h(px(7.0))
                            .rounded_full()
                            .bg(if branch.is_head {
                                theme.primary
                            } else {
                                theme.muted_foreground
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .truncate()
                            .child(branch.name.clone()),
                    )
                    .when(!ahead_behind.is_empty(), |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(ahead_behind.clone()),
                        )
                    })
                    .when(branch.is_head, |this| {
                        this.child(
                            div()
                                .px_1()
                                .rounded(px(3.0))
                                .bg(theme.primary.opacity(0.18))
                                .text_xs()
                                .text_color(theme.primary)
                                .child("HEAD"),
                        )
                    })
                    .into_any_element(),
            );
        }

        let status_line = if let Some(error) = &self.worktree_error {
            div()
                .px_3()
                .text_xs()
                .text_color(theme.danger)
                .child(error_text(cx, error))
                .into_any_element()
        } else {
            div()
                .px_3()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(if counts.is_clean() {
                    cx.tr("作業ツリーはクリーン").to_string()
                } else {
                    worktree_counts_text(cx, &counts)
                })
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .h_full()
            .w_full()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(
                div()
                    .px_3()
                    .py_3()
                    .border_b_1()
                    .border_color(theme.sidebar_border)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(self.tab_title()),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(self.path.to_string_lossy().into_owned()),
                    )
                    .child(div().mt_2().child(status_line)),
            )
            .child(
                div()
                    .px_3()
                    .pt_3()
                    .pb_1()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.muted_foreground)
                    .child(cx.tr("ブランチ")),
            )
            .child(
                div()
                    .id(ElementId::Name(
                        format!("branches-scroll-{}", self.id).into(),
                    ))
                    .flex_1()
                    .min_h_0()
                    .px_2()
                    .overflow_y_scroll()
                    .children(branch_rows),
            )
            .into_any_element()
    }

    fn history_rows(&self) -> Vec<HistoryRow> {
        let mut rows = Vec::new();
        if self.has_worktree_row() {
            let counts = self
                .worktree
                .as_ref()
                .map(|snapshot| snapshot.counts.clone())
                .or_else(|| self.info.as_ref().map(|info| info.counts.clone()))
                .unwrap_or_default();
            rows.push(HistoryRow::WorkingTree { counts });
        }

        for (index, commit) in self.history.iter().enumerate() {
            if !self.commit_matches_filter(commit) {
                continue;
            }
            let graph = if self.filter.trim().is_empty() {
                self.graph_rows.get(index).cloned()
            } else {
                None
            };
            rows.push(HistoryRow::Commit {
                index,
                oid: commit.oid,
                summary: commit.summary.clone(),
                author: commit.author_name.clone(),
                time: commit.author_time,
                refs: commit.refs.clone(),
                graph,
            });
        }

        if self.history_has_more {
            rows.push(HistoryRow::LoadMore);
        }
        rows
    }

    fn render_history(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let rows = self.history_rows();
        let selected = self.selected;
        let entity = cx.entity();
        let focus = self.history_focus.clone();
        let scroll = self.history_scroll.clone();
        let visible_count = rows
            .iter()
            .filter(|row| matches!(row, HistoryRow::Commit { .. }))
            .count();
        let total = self.history.len();
        let graph_lanes = self
            .graph_rows
            .iter()
            .map(|row| row.lane_count)
            .max()
            .unwrap_or(1)
            .clamp(1, rascal_core::MAX_GRAPH_LANES);

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(cx.tr("履歴")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!("{visible_count} / {total}")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(self.history_rev_label()),
                    ),
            )
            .child(
                div()
                    .w(px(230.0))
                    .child(Input::new(&self.filter_input).appearance(false)),
            );

        let body = if rows.is_empty() {
            let message = if self.history_loading {
                cx.tr("履歴を読み込み中…").to_string()
            } else if let Some(error) = &self.history_error {
                error_text(cx, error)
            } else if !self.filter.trim().is_empty() {
                cx.tr("一致するコミットがありません").to_string()
            } else {
                cx.tr("コミットがありません").to_string()
            };
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p_6()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(message.to_string())
                .into_any_element()
        } else {
            let rows_for_list = rows.clone();
            let entity_for_list = entity.clone();
            let focus_for_list = focus.clone();
            uniform_list(
                ElementId::Name(format!("history-{}", self.id).into()),
                rows.len(),
                move |range, _window, cx| {
                    let mut items = Vec::with_capacity(range.len());
                    for index in range {
                        if let Some(row) = rows_for_list.get(index) {
                            items.push(history_row_element(
                                row.clone(),
                                selected,
                                graph_lanes,
                                entity_for_list.clone(),
                                focus_for_list.clone(),
                                cx,
                            ));
                        }
                    }
                    items
                },
            )
            .track_scroll(scroll)
            .flex_1()
            .min_h_0()
            .into_any_element()
        };

        let entity_keys = entity.clone();
        let history_focus = focus.clone();
        div()
            .flex()
            .flex_col()
            .h_full()
            .min_h_0()
            .w_full()
            .bg(theme.background)
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .track_focus(&history_focus)
                    .on_key_down(move |event, _window, cx| {
                        let key = event.keystroke.key.as_str();
                        if key == "up" || key == "down" {
                            let delta = if key == "up" { -1 } else { 1 };
                            entity_keys.update(cx, |this, cx| this.move_selection(delta, cx));
                            cx.stop_propagation();
                        } else if key == "enter" {
                            entity_keys.update(cx, |this, cx| {
                                if let Selection::Commit(oid) = this.selected {
                                    this.select_commit(oid, cx);
                                }
                            });
                            cx.stop_propagation();
                        }
                    })
                    .child(body),
            )
            .when(self.history_loading && !rows.is_empty(), |this| {
                this.child(
                    div()
                        .px_3()
                        .py_1()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(cx.tr("読み込み中…")),
                )
            })
            .into_any_element()
    }

    fn file_rows(&self, cx: &App) -> Vec<FileRow> {
        if self.selected == Selection::None {
            return vec![FileRow::Message(
                cx.tr("コミットを選択してください").to_string(),
            )];
        }
        if self.effective_file_mode() == FileListMode::Files {
            return self.tree_rows(cx);
        }
        if self.changes_view == ChangesView::Tree {
            return self.changes_tree_rows(cx);
        }

        match self.selected {
            Selection::Commit(_) => {
                let Some(detail) = &self.detail else {
                    return vec![FileRow::Message(if self.detail_loading {
                        cx.tr("コミットを読み込み中…").to_string()
                    } else {
                        cx.tr("ファイルを選択できません").to_string()
                    })];
                };
                if detail.files.is_empty() {
                    return vec![FileRow::Message(
                        cx.tr("変更ファイルはありません").to_string(),
                    )];
                }
                detail
                    .files
                    .iter()
                    .enumerate()
                    .map(|(index, file)| FileRow::Change {
                        target: FileTarget::CommitFile(index),
                        display: file.path.clone(),
                        path: file.path.clone(),
                        depth: 0,
                        old_path: file.old_path.clone(),
                        status: file.status,
                        additions: file.stats_loaded.then_some(file.additions),
                        deletions: file.stats_loaded.then_some(file.deletions),
                        binary: file.is_binary,
                        layer: None,
                    })
                    .collect()
            }
            Selection::WorkingTree => {
                let Some(snapshot) = &self.worktree else {
                    return vec![FileRow::Message(
                        cx.tr("作業ツリーを読み込み中…").to_string(),
                    )];
                };
                if snapshot.files.is_empty() {
                    return vec![FileRow::Message(cx.tr("変更はありません").to_string())];
                }
                let mut rows = Vec::new();
                let mut current_layer = None;
                for (index, file) in snapshot.files.iter().enumerate() {
                    if current_layer != Some(file.layer) {
                        current_layer = Some(file.layer);
                        rows.push(FileRow::Group(file.layer.label().to_string()));
                    }
                    rows.push(FileRow::Change {
                        target: FileTarget::WorktreeFile(index),
                        display: file.path.clone(),
                        path: file.path.clone(),
                        depth: 0,
                        old_path: file.old_path.clone(),
                        status: file.status,
                        additions: None,
                        deletions: None,
                        binary: file.is_binary,
                        layer: Some(file.layer),
                    });
                }
                rows
            }
            Selection::None => vec![FileRow::Message(
                cx.tr("コミットを選択してください").to_string(),
            )],
        }
    }

    fn changes_tree_rows(&self, cx: &App) -> Vec<FileRow> {
        match self.selected {
            Selection::Commit(_) => {
                let Some(detail) = &self.detail else {
                    return vec![FileRow::Message(if self.detail_loading {
                        cx.tr("コミットを読み込み中…").to_string()
                    } else {
                        cx.tr("ファイルを選択できません").to_string()
                    })];
                };
                if detail.files.is_empty() {
                    return vec![FileRow::Message(
                        cx.tr("変更ファイルはありません").to_string(),
                    )];
                }
                let mut tree = ChangeTree::default();
                for (index, file) in detail.files.iter().enumerate() {
                    tree.insert(
                        &file.path,
                        ChangeLeaf {
                            target: FileTarget::CommitFile(index),
                            path: file.path.clone(),
                            old_path: file.old_path.clone(),
                            status: file.status,
                            additions: file.stats_loaded.then_some(file.additions),
                            deletions: file.stats_loaded.then_some(file.deletions),
                            binary: file.is_binary,
                            layer: None,
                        },
                    );
                }
                let mut rows = Vec::new();
                self.flatten_change_tree(&tree, 0, "", "commit", &mut rows);
                rows
            }
            Selection::WorkingTree => {
                let Some(snapshot) = &self.worktree else {
                    return vec![FileRow::Message(
                        cx.tr("作業ツリーを読み込み中…").to_string(),
                    )];
                };
                if snapshot.files.is_empty() {
                    return vec![FileRow::Message(cx.tr("変更はありません").to_string())];
                }

                let mut layers: Vec<(DiffLayer, ChangeTree)> = Vec::new();
                for (index, file) in snapshot.files.iter().enumerate() {
                    if layers.last().map(|(layer, _)| *layer) != Some(file.layer) {
                        layers.push((file.layer, ChangeTree::default()));
                    }
                    let tree = &mut layers.last_mut().expect("layer exists").1;
                    tree.insert(
                        &file.path,
                        ChangeLeaf {
                            target: FileTarget::WorktreeFile(index),
                            path: file.path.clone(),
                            old_path: file.old_path.clone(),
                            status: file.status,
                            additions: None,
                            deletions: None,
                            binary: file.is_binary,
                            layer: Some(file.layer),
                        },
                    );
                }

                let mut rows = Vec::new();
                for (layer, tree) in layers {
                    rows.push(FileRow::Group(layer.label().to_string()));
                    self.flatten_change_tree(&tree, 0, "", &format!("{layer:?}"), &mut rows);
                }
                rows
            }
            Selection::None => vec![FileRow::Message(
                cx.tr("コミットを選択してください").to_string(),
            )],
        }
    }

    fn flatten_change_tree(
        &self,
        tree: &ChangeTree,
        depth: usize,
        prefix: &str,
        group: &str,
        rows: &mut Vec<FileRow>,
    ) {
        for (name, child) in &tree.dirs {
            let full = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let key = format!("{group}:{full}");
            let expanded = !self.collapsed_change_dirs.contains(&key);
            rows.push(FileRow::ChangeDir {
                key: key.clone(),
                name: name.clone(),
                depth,
                expanded,
                count: child.file_count(),
            });
            if expanded {
                self.flatten_change_tree(child, depth + 1, &full, group, rows);
            }
        }

        let mut files = tree.files.clone();
        files.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
        for (name, leaf) in files {
            rows.push(FileRow::Change {
                target: leaf.target,
                path: leaf.path,
                display: name,
                depth,
                old_path: leaf.old_path,
                status: leaf.status,
                additions: leaf.additions,
                deletions: leaf.deletions,
                binary: leaf.binary,
                layer: leaf.layer,
            });
        }
    }

    fn set_changes_view(&mut self, view: ChangesView, cx: &mut Context<Self>) {
        if self.changes_view == view {
            return;
        }
        self.changes_view = view;
        self.file_scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn toggle_changes_dir(&mut self, key: String, cx: &mut Context<Self>) {
        if !self.collapsed_change_dirs.insert(key.clone()) {
            self.collapsed_change_dirs.remove(&key);
        }
        cx.notify();
    }

    fn effective_file_mode(&self) -> FileListMode {
        if self.selected == Selection::WorkingTree {
            FileListMode::Changes
        } else {
            self.file_mode
        }
    }

    fn tree_rows(&self, cx: &App) -> Vec<FileRow> {
        if self.tree_cache.is_empty() {
            return vec![FileRow::Message(if let Some(error) = &self.tree_error {
                error_text(cx, error)
            } else {
                cx.tr("ファイルツリーを読み込み中…").to_string()
            })];
        }
        let mut rows = Vec::new();
        self.collect_tree_rows("", 0, &mut rows, cx);
        rows
    }

    fn collect_tree_rows(&self, dir: &str, depth: usize, rows: &mut Vec<FileRow>, cx: &App) {
        let Some(entries) = self.tree_cache.get(dir) else {
            rows.push(FileRow::Message(cx.tr("読み込み中…").to_string()));
            return;
        };
        for entry in &entries.entries {
            let is_dir = entry.kind == TreeEntryKind::Tree;
            let expanded = self.expanded_dirs.contains(&entry.path);
            let loading = self.tree_loading.contains(&entry.path);
            rows.push(FileRow::Tree {
                target: if is_dir {
                    FileTarget::TreeDir(entry.path.clone())
                } else {
                    FileTarget::TreeFile(entry.path.clone())
                },
                name: entry.name.clone(),
                depth,
                is_dir,
                expanded,
                loading,
                size: entry.size,
            });
            if is_dir && expanded {
                self.collect_tree_rows(&entry.path, depth + 1, rows, cx);
            }
        }
        if entries.truncated {
            rows.push(FileRow::Message(
                cx.tr("ファイルが多すぎるため、一部のみ表示しています。")
                    .to_string(),
            ));
        }
    }

    fn render_file_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let mode = self.effective_file_mode();
        let rows = self.file_rows(cx);
        let entity = cx.entity();
        let files_disabled = self.selected == Selection::WorkingTree;
        let count = rows
            .iter()
            .filter(|row| matches!(row, FileRow::Change { .. } | FileRow::Tree { .. }))
            .count();

        let changes_button = mode_toggle(
            format!("files-changes-{}", self.id),
            cx.tr("変更"),
            mode == FileListMode::Changes,
            !files_disabled,
            entity.clone(),
            FileListMode::Changes,
            theme,
        );
        let view = self.changes_view;
        let view_toggles =
            div()
                .flex()
                .items_center()
                .gap_1()
                .when(mode == FileListMode::Changes, |this| {
                    this.child(changes_view_toggle(
                        format!("changes-view-list-{}", self.id),
                        cx.tr("一覧"),
                        ChangesView::List,
                        view == ChangesView::List,
                        entity.clone(),
                        theme,
                    ))
                    .child(changes_view_toggle(
                        format!("changes-view-tree-{}", self.id),
                        cx.tr("ツリー"),
                        ChangesView::Tree,
                        view == ChangesView::Tree,
                        entity.clone(),
                        theme,
                    ))
                });
        let files_button = mode_toggle(
            format!("files-tree-{}", self.id),
            cx.tr_key("ファイル", "mode.files"),
            mode == FileListMode::Files,
            !files_disabled,
            entity.clone(),
            FileListMode::Files,
            theme,
        );

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(changes_button)
                    .child(files_button),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(view_toggles)
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(file_count_text(cx, count)),
                    ),
            );

        let body = if rows.is_empty() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(cx.tr("該当するファイルがありません"))
                .into_any_element()
        } else {
            let rows_for_list = rows.clone();
            let selected_for_list = self.selected_file.clone();
            let entity_for_list = entity.clone();
            let scroll = self.file_scroll.clone();
            uniform_list(
                ElementId::Name(format!("file-list-{}", self.id).into()),
                rows.len(),
                move |range, _window, cx| {
                    let mut items = Vec::with_capacity(range.len());
                    for index in range {
                        if let Some(row) = rows_for_list.get(index) {
                            items.push(file_row_element(
                                row.clone(),
                                selected_for_list.clone(),
                                entity_for_list.clone(),
                                cx,
                            ));
                        }
                    }
                    items
                },
            )
            .track_scroll(scroll)
            .flex_1()
            .min_h_0()
            .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .h_full()
            .min_h_0()
            .w_full()
            .border_r_1()
            .border_color(theme.border)
            .child(header)
            .child(body)
            .into_any_element()
    }

    fn render_viewer(&self, cx: &mut Context<Self>) -> AnyElement {
        let header = self.viewer_header(cx);
        let theme = cx.theme();

        let body = if let Some(image) = &self.image_view {
            div()
                .flex_1()
                .min_h_0()
                .w_full()
                .child(image.clone())
                .into_any_element()
        } else if self.image_loading {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p_6()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(cx.tr("画像を読み込み中…"))
                .into_any_element()
        } else if let Some(error) = &self.image_error {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p_6()
                .text_sm()
                .text_color(theme.danger)
                .child(format!("{}: {error}", cx.tr("画像を読み込めません")))
                .into_any_element()
        } else if let Some(error) = &self.viewer_error {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p_6()
                .text_sm()
                .text_color(theme.danger)
                .child(error_text(cx, error))
                .into_any_element()
        } else if self.viewer_loading {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p_6()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(cx.tr("読み込み中…"))
                .into_any_element()
        } else if self.selected_file.is_none() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p_6()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(cx.tr("ファイルを選択してください"))
                .into_any_element()
        } else if self.viewer_rows.is_empty() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p_6()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(cx.tr("表示する内容がありません"))
                .into_any_element()
        } else {
            let rows = self.viewer_rows.clone();
            let list = list(self.viewer_list.clone(), move |index, _window, cx| {
                let row = rows.get(index).cloned();
                viewer_row_element(row, cx)
            });
            div()
                .flex_1()
                .min_h_0()
                .w_full()
                .child(list.size_full())
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .h_full()
            .min_h_0()
            .w_full()
            .bg(theme.background)
            .child(header)
            .child(body)
            .into_any_element()
    }

    fn viewer_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let layout_control = self
            .diff
            .is_some()
            .then(|| diff_view::layout_toggle("main-diff-layout", cx));
        let theme = cx.theme();
        let diff_theme = cx.try_global::<DiffTheme>().copied().unwrap_or_default();
        let diff = diff_palette(theme, diff_theme);
        let Some(selected) = &self.selected_file else {
            return div()
                .h(px(44.0))
                .flex()
                .items_center()
                .px_3()
                .border_b_1()
                .border_color(theme.border)
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(cx.tr("ビューア"))
                .into_any_element();
        };

        let status = self.diff.as_ref().map(|diff| diff.status).or_else(|| {
            self.detail.as_ref().and_then(|detail| {
                if let FileSource::CommitDiff { index } = selected.source {
                    detail.files.get(index).map(|file| file.status)
                } else {
                    None
                }
            })
        });
        let additions = self.diff.as_ref().map(|diff| diff.additions).unwrap_or(0);
        let deletions = self.diff.as_ref().map(|diff| diff.deletions).unwrap_or(0);
        let label = match selected.source {
            FileSource::CommitDiff { .. } => {
                let base = self
                    .diff
                    .as_ref()
                    .map(|diff| diff.base_label.clone())
                    .unwrap_or_else(|| "第1親".to_string());
                let base = cx.tr(&base);
                let oid = self.selected_commit().map(short_oid).unwrap_or_default();
                if cx.is_japanese() {
                    format!("差分 {base} → {oid}")
                } else {
                    format!("Diff {base} → {oid}")
                }
            }
            FileSource::Worktree { layer } => {
                let layer = cx.tr(layer.label());
                if cx.is_japanese() {
                    format!("差分 {layer}")
                } else {
                    format!("Diff {layer}")
                }
            }
            FileSource::TreeFile => {
                let oid = self.selected_commit().map(short_oid).unwrap_or_default();
                if cx.is_japanese() {
                    format!("内容 @ {oid}")
                } else {
                    format!("Content @ {oid}")
                }
            }
        };

        let mut left = div().flex().items_center().gap_2().min_w_0();
        if let Some(old_path) = &selected.old_path {
            left = left
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .truncate()
                        .child(old_path.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child("→"),
                );
        }
        left = left.child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .truncate()
                .child(selected.path.clone()),
        );

        div()
            .h(px(44.0))
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .px_3()
            .border_b_1()
            .border_color(theme.border)
            .child(left)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(label),
                    )
                    .when_some(status, |this, status| {
                        this.child(
                            div()
                                .px_1()
                                .rounded(px(3.0))
                                .bg(theme.muted)
                                .text_xs()
                                .text_color(status_color(theme, status, &diff))
                                .child(status.code()),
                        )
                    })
                    .when(additions > 0 || deletions > 0, |this| {
                        this.child(
                            div().text_xs().child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_color(diff.added_mark)
                                            .child(format!("+{additions}")),
                                    )
                                    .child(
                                        div()
                                            .text_color(diff.removed_mark)
                                            .child(format!("-{deletions}")),
                                    ),
                            ),
                        )
                    })
                    .when_some(layout_control, |this, control| this.child(control)),
            )
            .into_any_element()
    }

    fn render_commit_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let entity = cx.entity();

        let (title, subtitle, message, oid, refs) = match self.selected {
            Selection::Commit(oid) => {
                let summary = self
                    .detail
                    .as_ref()
                    .map(|detail| detail.summary.clone())
                    .or_else(|| {
                        self.history
                            .iter()
                            .find(|commit| commit.oid == oid)
                            .map(|commit| commit.summary.clone())
                    })
                    .unwrap_or_else(|| short_oid(oid));
                let (author, time, parents) = if let Some(detail) = &self.detail {
                    (
                        format!("{} <{}>", detail.author.name, detail.author.email),
                        detail.author.time,
                        detail.parents.len(),
                    )
                } else if let Some(commit) = self.history.iter().find(|commit| commit.oid == oid) {
                    (
                        commit.author_name.clone(),
                        commit.author_time,
                        commit.parents.len(),
                    )
                } else {
                    (String::new(), 0, 0)
                };
                let subtitle = if cx.is_japanese() {
                    format!(
                        "{} · {} · {} · 親 {}",
                        author,
                        absolute_time(time),
                        short_oid(oid),
                        parents
                    )
                } else if parents == 1 {
                    format!(
                        "{} · {} · {} · 1 parent",
                        author,
                        absolute_time(time),
                        short_oid(oid)
                    )
                } else {
                    format!(
                        "{} · {} · {} · {} parents",
                        author,
                        absolute_time(time),
                        short_oid(oid),
                        parents
                    )
                };
                let message = self
                    .detail
                    .as_ref()
                    .map(|detail| detail.message.trim().to_string())
                    .unwrap_or_default();
                let refs = self
                    .history
                    .iter()
                    .find(|commit| commit.oid == oid)
                    .map(|commit| commit.refs.clone())
                    .unwrap_or_default();
                (summary, subtitle, message, Some(oid), refs)
            }
            Selection::WorkingTree => {
                let counts = self
                    .worktree
                    .as_ref()
                    .map(|snapshot| snapshot.counts.clone())
                    .or_else(|| self.info.as_ref().map(|info| info.counts.clone()))
                    .unwrap_or_default();
                (
                    cx.tr("未コミットの変更").to_string(),
                    worktree_counts_text(cx, &counts),
                    String::new(),
                    None,
                    Vec::new(),
                )
            }
            Selection::None => (
                cx.tr("コミットを選択してください").to_string(),
                String::new(),
                String::new(),
                None,
                Vec::new(),
            ),
        };

        let mut title_row = div().flex().items_center().gap_2().min_w_0();
        title_row = title_row.child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .truncate()
                .child(title),
        );
        for label in refs.iter().take(4) {
            title_row = title_row.child(ref_badge(label, theme));
        }

        let has_message = message.lines().count() > 1 || message.len() > 120;
        let toggle_entity = entity.clone();
        let toggle = div()
            .id(ElementId::Name(
                format!("message-toggle-{}", self.id).into(),
            ))
            .px_2()
            .py_1()
            .rounded(px(4.0))
            .text_xs()
            .cursor_pointer()
            .text_color(theme.link)
            .hover(|this| this.bg(theme.list_hover))
            .on_click(move |_, _, cx| {
                toggle_entity.update(cx, |this, cx| {
                    this.message_expanded = !this.message_expanded;
                    cx.notify();
                });
            })
            .child(if self.message_expanded {
                cx.tr("たたむ")
            } else {
                cx.tr("全文")
            });

        let copy_button = oid.map(|oid| {
            div()
                .id(ElementId::Name(format!("copy-sha-{}", self.id).into()))
                .px_2()
                .py_1()
                .rounded(px(4.0))
                .text_xs()
                .cursor_pointer()
                .text_color(theme.muted_foreground)
                .hover(|this| this.bg(theme.list_hover))
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(oid.to_string()));
                })
                .child(cx.tr("SHAをコピー"))
        });

        let mut bar = div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w_full()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .gap_1()
                            .child(title_row)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .truncate()
                                    .child(subtitle),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .when(has_message, |this| this.child(toggle))
                            .when_some(copy_button, |this, button| this.child(button)),
                    ),
            );

        if self.message_expanded && !message.is_empty() {
            bar = bar.child(
                div()
                    .id(ElementId::Name(
                        format!("commit-message-{}", self.id).into(),
                    ))
                    .max_h(px(160.0))
                    .overflow_y_scroll()
                    .px_4()
                    .pb_2()
                    .text_sm()
                    .whitespace_normal()
                    .child(message),
            );
        }

        if let Some(error) = &self.detail_error {
            bar = bar.child(
                div()
                    .px_3()
                    .pb_2()
                    .text_xs()
                    .text_color(theme.danger)
                    .child(error_text(cx, error)),
            );
        }

        bar.into_any_element()
    }

    fn render_status(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let info = self.info.as_ref();
        let head = info.map(|info| info.head.label()).unwrap_or_default();
        let counts = info.map(|info| info.counts.clone()).unwrap_or_default();
        let mut items = Vec::new();
        items.push(div().text_xs().child(head));
        if let Some(info) = info
            && let HeadState::Attached { ahead, behind, .. } = &info.head
        {
            if *ahead > 0 {
                items.push(div().text_xs().child(format!("↑{ahead}")));
            }
            if *behind > 0 {
                items.push(div().text_xs().child(format!("↓{behind}")));
            }
        }
        if !counts.is_clean() {
            let changed = counts.staged + counts.unstaged + counts.untracked;
            items.push(div().text_xs().child(if cx.is_japanese() {
                format!("変更 {changed}")
            } else {
                format!("{changed} changed")
            }));
        }
        if self.history_loading {
            items.push(div().text_xs().child(cx.tr("読み込み中…")));
        }
        if let Some(error) = &self.history_error {
            items.push(
                div()
                    .text_xs()
                    .text_color(theme.danger)
                    .truncate()
                    .child(error_text(cx, error)),
            );
        }

        div()
            .h(px(26.0))
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .px_3()
            .bg(theme.title_bar)
            .border_t_1()
            .border_color(theme.title_bar_border)
            .child(div().flex().items_center().gap_3().children(items))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded(px(3.0))
                            .bg(theme.muted)
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(cx.tr("読み取り専用")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(commit_count_text(cx, self.history.len())),
                    ),
            )
            .into_any_element()
    }
}

impl Render for RepoSession {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let layout = diff_view::current_layout(cx);
        if self.diff.is_some() && self.viewer_layout != layout {
            self.rebuild_viewer_rows(cx);
        }
        if let Some(error) = &self.open_error {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .p_8()
                .child(
                    div()
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(cx.tr("リポジトリを開けませんでした")),
                )
                .child(
                    div()
                        .max_w(px(620.0))
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(error.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.path.to_string_lossy().into_owned()),
                )
                .into_any_element();
        }

        if self.opening {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(cx.tr("リポジトリを読み込み中…"))
                .into_any_element();
        }

        let sidebar = self.render_sidebar(cx);
        let history = self.render_history(window, cx);
        let files = self.render_file_list(cx);
        let viewer = self.render_viewer(cx);
        let commit_bar = self.render_commit_bar(cx);
        let status = self.render_status(cx);

        let right = v_resizable(ElementId::Name(format!("right-split-{}", self.id).into()))
            .child(
                resizable_panel()
                    .size(px(260.0))
                    .size_range(px(160.0)..px(720.0))
                    .child(history),
            )
            .child(
                resizable_panel().child(
                    div()
                        .flex()
                        .flex_col()
                        .size_full()
                        .min_h_0()
                        .child(commit_bar)
                        .child(
                            div().flex_1().min_h_0().child(
                                h_resizable(ElementId::Name(
                                    format!("bottom-split-{}", self.id).into(),
                                ))
                                .child(
                                    resizable_panel()
                                        .size(px(360.0))
                                        .size_range(px(280.0)..px(680.0))
                                        .child(files),
                                )
                                .child(resizable_panel().child(viewer)),
                            ),
                        ),
                ),
            );

        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .child(
                div().flex_1().min_h_0().child(
                    h_resizable(ElementId::Name(format!("main-split-{}", self.id).into()))
                        .child(
                            resizable_panel()
                                .size(px(232.0))
                                .size_range(px(180.0)..px(380.0))
                                .child(sidebar),
                        )
                        .child(resizable_panel().child(right)),
                ),
            )
            .child(status)
            .into_any_element()
    }
}

// ---- pure element helpers ------------------------------------------------

const HISTORY_ROW_HEIGHT: f32 = 36.0;

fn history_row_element(
    row: HistoryRow,
    selected: Selection,
    graph_lanes: usize,
    entity: Entity<RepoSession>,
    focus: FocusHandle,
    cx: &mut App,
) -> AnyElement {
    let theme = cx.theme();
    let row_for_click = row.clone();
    let is_selected = match (&row, selected) {
        (HistoryRow::WorkingTree { .. }, Selection::WorkingTree) => true,
        (HistoryRow::Commit { oid, .. }, Selection::Commit(selected_oid)) => *oid == selected_oid,
        _ => false,
    };

    let row_id = match &row {
        HistoryRow::WorkingTree { .. } => "history-row-worktree".to_string(),
        HistoryRow::Commit { index, oid, .. } => {
            format!("history-row-{index}-{}", short_oid(*oid))
        }
        HistoryRow::LoadMore => "history-row-more".to_string(),
    };

    let content = match &row {
        HistoryRow::WorkingTree { counts } => div()
            .flex()
            .items_center()
            .gap_2()
            .w_full()
            .child(div().w(px(7.0)).h(px(7.0)).rounded_full().bg(theme.warning))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(cx.tr("未コミットの変更")),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(worktree_counts_text(cx, counts)),
            )
            .into_any_element(),
        HistoryRow::Commit {
            summary,
            author,
            time,
            refs,
            graph,
            oid,
            ..
        } => {
            let left = div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap_2()
                .children(refs.iter().take(3).map(|label| ref_badge(label, theme)))
                .when(refs.len() > 3, |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!("+{}", refs.len() - 3)),
                    )
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .truncate()
                        .child(summary.clone()),
                );

            div()
                .flex()
                .items_center()
                .h_full()
                .w_full()
                .when_some(graph.as_ref(), |this, graph| {
                    this.child(render_graph_gutter(graph, graph_lanes, theme))
                })
                .child(left)
                .child(commit_meta_columns(author.clone(), *time, *oid, theme))
                .into_any_element()
        }
        HistoryRow::LoadMore => div()
            .w_full()
            .flex()
            .items_center()
            .justify_center()
            .text_xs()
            .text_color(theme.link)
            .child(cx.tr("さらに読み込む"))
            .into_any_element(),
    };

    div()
        .id(ElementId::Name(row_id.into()))
        .h(px(HISTORY_ROW_HEIGHT))
        .w_full()
        .px_3()
        .flex()
        .items_center()
        .border_b_1()
        .border_color(theme.border.opacity(0.45))
        .cursor_pointer()
        .when(is_selected, |this| this.bg(theme.list_active))
        .hover(|this| this.bg(theme.list_hover))
        .on_click(move |_, window, cx| {
            focus.focus(window);
            entity.update(cx, |this, cx| {
                this.on_history_row(row_for_click.clone(), cx)
            });
        })
        .child(content)
        .into_any_element()
}

fn commit_meta_columns(
    author: String,
    time: i64,
    oid: Oid,
    theme: &gpui_component::theme::Theme,
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_3()
        .flex_shrink_0()
        .child(
            div().w(px(150.0)).flex().justify_end().child(
                div()
                    .max_w(px(150.0))
                    .truncate()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(author),
            ),
        )
        .child(
            div()
                .w(px(88.0))
                .flex()
                .justify_end()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(absolute_date(time)),
        )
        .child(
            div()
                .w(px(64.0))
                .flex()
                .justify_end()
                .font_family(theme.mono_font_family.clone())
                .text_size(px(11.0))
                .text_color(theme.muted_foreground)
                .child(short_oid(oid)),
        )
        .into_any_element()
}

fn render_graph_gutter(
    graph: &GraphRow,
    graph_lanes: usize,
    theme: &gpui_component::theme::Theme,
) -> AnyElement {
    let lanes = graph_lanes.clamp(1, rascal_core::MAX_GRAPH_LANES);
    let width = gutter_width_for(lanes);
    let lane_width = 14.0;
    let mid = HISTORY_ROW_HEIGHT / 2.0;
    let center = |lane: usize| lane as f32 * lane_width + 7.0;
    let mut elements: Vec<AnyElement> = Vec::new();

    for lane in 0..lanes {
        let color = lane_color(theme, (lane % 8) as u8);
        let x = center(lane) - 1.0;
        if graph.top.get(lane).copied().unwrap_or(false) {
            elements.push(
                div()
                    .absolute()
                    .left(px(x))
                    .top_0()
                    .w(px(2.0))
                    .h(px(mid))
                    .bg(color)
                    .into_any_element(),
            );
        }
        if graph.bottom.get(lane).copied().unwrap_or(false) {
            elements.push(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(mid))
                    .w(px(2.0))
                    .h(px(mid))
                    .bg(color)
                    .into_any_element(),
            );
        }
    }

    for edge in &graph.edges {
        let (from, to) = (edge.from.min(edge.to), edge.from.max(edge.to));
        elements.push(
            div()
                .absolute()
                .left(px(center(from) - 1.0))
                .top(px(mid - 1.0))
                .w(px((to - from) as f32 * lane_width + 2.0))
                .h(px(2.0))
                .bg(lane_color(theme, edge.color))
                .into_any_element(),
        );
    }

    let node_center = center(graph.node_lane);
    elements.push(
        div()
            .absolute()
            .left(px(node_center - 4.5))
            .top(px(mid - 4.5))
            .w(px(9.0))
            .h(px(9.0))
            .rounded_full()
            .bg(lane_color(theme, graph.node_color()))
            .into_any_element(),
    );

    div()
        .relative()
        .h_full()
        .w(px(width))
        .flex_shrink_0()
        .children(elements)
        .into_any_element()
}

fn ref_badge(label: &rascal_core::RefLabel, theme: &gpui_component::theme::Theme) -> AnyElement {
    let dark = theme.is_dark();
    let color = match label.kind {
        RefKind::Head => theme.primary,
        RefKind::LocalBranch => {
            if dark {
                hsla(0.60, 0.90, 0.82, 1.0)
            } else {
                hsla(0.60, 0.75, 0.34, 1.0)
            }
        }
        RefKind::RemoteBranch => {
            if dark {
                hsla(0.0, 0.0, 0.84, 1.0)
            } else {
                hsla(0.0, 0.0, 0.32, 1.0)
            }
        }
        RefKind::Tag => {
            if dark {
                hsla(0.11, 0.95, 0.76, 1.0)
            } else {
                hsla(0.08, 0.85, 0.30, 1.0)
            }
        }
    };
    div()
        .px_1()
        .py_0p5()
        .rounded(px(3.0))
        .bg(color.opacity(if dark { 0.24 } else { 0.14 }))
        .border_1()
        .border_color(color.opacity(if dark { 0.45 } else { 0.30 }))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .max_w(px(150.0))
        .truncate()
        .child(label.name.clone())
        .into_any_element()
}

fn mode_toggle(
    id: String,
    label: SharedString,
    active: bool,
    enabled: bool,
    entity: Entity<RepoSession>,
    mode: FileListMode,
    theme: &gpui_component::theme::Theme,
) -> AnyElement {
    div()
        .id(ElementId::Name(id.into()))
        .px_2()
        .py_1()
        .rounded(px(4.0))
        .text_xs()
        .when(active, |this| {
            this.bg(theme.primary.opacity(0.16))
                .text_color(theme.primary)
                .font_weight(FontWeight::SEMIBOLD)
        })
        .when(!active, |this| {
            this.text_color(if enabled {
                theme.muted_foreground
            } else {
                theme.muted_foreground.opacity(0.4)
            })
        })
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(theme.list_hover))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.set_file_mode(mode, cx));
                })
        })
        .child(label)
        .into_any_element()
}

fn changes_view_toggle(
    id: String,
    label: SharedString,
    view: ChangesView,
    active: bool,
    entity: Entity<RepoSession>,
    theme: &gpui_component::theme::Theme,
) -> AnyElement {
    div()
        .id(ElementId::Name(id.into()))
        .px_1()
        .py_0p5()
        .rounded(px(4.0))
        .text_xs()
        .when(active, |this| {
            this.bg(theme.primary.opacity(0.16))
                .text_color(theme.primary)
                .font_weight(FontWeight::SEMIBOLD)
        })
        .when(!active, |this| {
            this.text_color(theme.muted_foreground)
                .cursor_pointer()
                .hover(|this| this.bg(theme.list_hover))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.set_changes_view(view, cx));
                })
        })
        .child(label)
        .into_any_element()
}

#[derive(Clone)]
struct FileContextData {
    repo_path: PathBuf,
    relative: String,
    absolute: PathBuf,
    rev: String,
}

fn file_context_data(entity: &Entity<RepoSession>, relative: String, cx: &App) -> FileContextData {
    let session = entity.read(cx);
    let root = session
        .info
        .as_ref()
        .and_then(|info| info.workdir.clone())
        .unwrap_or_else(|| session.path.clone());
    let rev = session
        .selected_commit()
        .map(|oid| oid.to_string())
        .unwrap_or_else(|| "HEAD".to_string());
    FileContextData {
        repo_path: session.path.clone(),
        absolute: root.join(&relative),
        relative,
        rev,
    }
}

fn with_file_context_menu(
    element: impl ParentElement + Styled + IntoElement + 'static,
    data: FileContextData,
) -> AnyElement {
    element
        .context_menu(move |menu, _window, cx| {
            let exists = data.absolute.exists();
            let parent_exists = data.absolute.parent().is_some_and(|parent| parent.exists());
            let mut menu = menu.min_w(px(260.0));

            {
                let data = data.clone();
                menu = menu.item(
                    PopupMenuItem::new(cx.tr("開く"))
                        .disabled(!exists)
                        .on_click(move |_, window, cx| {
                            open_path(&data.absolute, window, cx);
                        }),
                );
            }
            {
                let data = data.clone();
                menu = menu.item(
                    PopupMenuItem::new(cx.tr("アプリを指定して開く"))
                        .disabled(!exists)
                        .on_click(move |_, window, cx| {
                            open_with_application(data.clone(), window, cx);
                        }),
                );
            }
            {
                let data = data.clone();
                menu = menu.item(
                    PopupMenuItem::new(cx.tr("対象フォルダをFinderで開く"))
                        .disabled(!parent_exists)
                        .on_click(move |_, window, cx| {
                            if let Some(parent) = data.absolute.parent() {
                                open_path(parent, window, cx);
                            }
                        }),
                );
            }
            menu = menu.separator();
            {
                let data = data.clone();
                menu = menu.item(PopupMenuItem::new(cx.tr("絶対パスをコピー")).on_click(
                    move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(
                            data.absolute.to_string_lossy().into_owned(),
                        ));
                    },
                ));
            }
            {
                let data = data.clone();
                menu = menu.item(PopupMenuItem::new(cx.tr("相対パスをコピー")).on_click(
                    move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(data.relative.clone()));
                    },
                ));
            }
            menu = menu.separator();
            {
                let data = data.clone();
                menu = menu.item(PopupMenuItem::new(cx.tr("ファイル編集履歴")).on_click(
                    move |_, window, cx| {
                        open_file_history_window(
                            data.repo_path.clone(),
                            data.rev.clone(),
                            data.relative.clone(),
                            window,
                            cx,
                        );
                    },
                ));
            }
            menu
        })
        .into_any_element()
}

fn open_path(path: &Path, window: &mut Window, cx: &mut App) {
    if let Err(err) = std::process::Command::new("open").arg(path).spawn() {
        window.push_notification(
            Notification::error(format!("{}: {err}", cx.tr("ファイルを開けませんでした"))),
            cx,
        );
    }
}

fn open_with_application(data: FileContextData, window: &mut Window, cx: &mut App) {
    let title = cx.tr("アプリを指定して開く").to_string();
    let error_prefix = cx.tr("ファイルを開けませんでした").to_string();
    window
        .spawn(cx, async move |window| {
            let application = rfd::AsyncFileDialog::new()
                .set_title(&title)
                .set_directory("/Applications")
                .add_filter("Application", &["app"])
                .pick_file()
                .await;
            let Some(application) = application else {
                return;
            };
            if let Err(err) = std::process::Command::new("open")
                .arg("-a")
                .arg(application.path())
                .arg(&data.absolute)
                .spawn()
            {
                let message = format!("{error_prefix}: {err}");
                let _ = window.update(|window, cx| {
                    window.push_notification(Notification::error(message), cx);
                });
            }
        })
        .detach();
}

fn open_file_history_window(
    repo_path: PathBuf,
    rev: String,
    path: String,
    window: &mut Window,
    cx: &mut App,
) {
    let title = format!("{} — {}", path, cx.tr("ファイル編集履歴"));
    let parent = window.bounds();
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            appears_transparent: false,
            traffic_light_position: None,
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(parent.origin.x + px(48.0), parent.origin.y + px(48.0)),
            size: size(px(1240.0), px(700.0)),
        })),
        window_min_size: Some(size(px(820.0), px(480.0))),
        focus: true,
        ..Default::default()
    };

    let result = cx.open_window(options, move |window, cx| {
        cx.new(|cx| FileHistoryView::new(repo_path.clone(), rev.clone(), path.clone(), window, cx))
    });

    if let Ok(handle) = &result {
        let _ = handle.update(cx, |_, window, _| window.activate_window());
    }
    cx.activate(true);

    if let Err(err) = result {
        window.push_notification(
            Notification::error(format!("{}: {err}", cx.tr("ウィンドウを開けませんでした"))),
            cx,
        );
    }
}

fn file_row_element(
    row: FileRow,
    selected: Option<SelectedFile>,
    entity: Entity<RepoSession>,
    cx: &mut App,
) -> AnyElement {
    let theme = cx.theme();
    let diff_theme = cx.try_global::<DiffTheme>().copied().unwrap_or_default();
    let diff = diff_palette(theme, diff_theme);
    let is_selected = match (&row, &selected) {
        (
            FileRow::Change {
                target: FileTarget::CommitFile(index),
                ..
            },
            Some(SelectedFile {
                source:
                    FileSource::CommitDiff {
                        index: selected_index,
                    },
                ..
            }),
        ) => index == selected_index,
        (
            FileRow::Change {
                path,
                layer,
                target: FileTarget::WorktreeFile(_),
                ..
            },
            Some(SelectedFile {
                path: selected_path,
                source:
                    FileSource::Worktree {
                        layer: selected_layer,
                    },
                ..
            }),
        ) => path == selected_path && layer.as_ref() == Some(selected_layer),
        (
            FileRow::Tree {
                target: FileTarget::TreeFile(path),
                ..
            },
            Some(SelectedFile {
                path: selected_path,
                source: FileSource::TreeFile,
                ..
            }),
        ) => path == selected_path,
        _ => false,
    };
    match row {
        FileRow::Group(label) => div()
            .h(px(30.0))
            .w_full()
            .px_3()
            .flex()
            .items_center()
            .bg(theme.muted.opacity(0.4))
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.muted_foreground)
            .child(label)
            .into_any_element(),
        FileRow::Message(message) => div()
            .h(px(30.0))
            .w_full()
            .px_3()
            .flex()
            .items_center()
            .text_xs()
            .text_color(theme.muted_foreground)
            .truncate()
            .child(message)
            .into_any_element(),
        FileRow::ChangeDir {
            key,
            name,
            depth,
            expanded,
            count,
        } => {
            let key_for_click = key.clone();
            div()
                .id(ElementId::Name(format!("changes-dir-{key}").into()))
                .h(px(30.0))
                .w_full()
                .pr_3()
                .pl(px(8.0 + depth as f32 * 14.0))
                .flex()
                .items_center()
                .gap_1()
                .cursor_pointer()
                .hover(|this| this.bg(theme.list_hover))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.toggle_changes_dir(key_for_click.clone(), cx)
                    });
                })
                .child(
                    Icon::new(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size(px(12.0))
                    .text_color(theme.muted_foreground),
                )
                .child(
                    Icon::new(if expanded {
                        IconName::FolderOpen
                    } else {
                        IconName::Folder
                    })
                    .size(px(13.0))
                    .text_color(theme.info),
                )
                .child(div().flex_1().text_sm().truncate().child(name))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format!("{count}")),
                )
                .into_any_element()
        }
        FileRow::Change {
            target,
            path,
            display,
            depth,
            old_path,
            status,
            additions,
            deletions,
            binary,
            ..
        } => {
            let context = file_context_data(&entity, path.clone(), cx);
            let target_for_click = target.clone();
            let mut label = div().flex().flex_1().items_center().gap_2().min_w_0();
            label = label.child(
                div()
                    .w(px(14.0))
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(status_color(theme, status, &diff))
                    .child(status.code()),
            );
            if let Some(old_path) = old_path {
                label = label
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(old_path),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("→"),
                    );
            }
            label = label.child(div().flex_1().text_sm().truncate().child(display));
            if binary {
                label = label.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child("bin"),
                );
            } else if let (Some(additions), Some(deletions)) = (additions, deletions) {
                label = label
                    .child(
                        div()
                            .text_xs()
                            .text_color(diff.added_mark)
                            .child(format!("+{additions}")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(diff.removed_mark)
                            .child(format!("-{deletions}")),
                    );
            }

            let row_id = match &target_for_click {
                FileTarget::CommitFile(index) => format!("file-commit-{index}"),
                FileTarget::WorktreeFile(index) => format!("file-worktree-{index}"),
                FileTarget::TreeDir(path) => format!("file-dir-{path}"),
                FileTarget::TreeFile(path) => format!("file-blob-{path}"),
            };
            let row = div()
                .id(ElementId::Name(row_id.into()))
                .h(px(30.0))
                .w_full()
                .pr_3()
                .pl(px(8.0 + depth as f32 * 14.0))
                .flex()
                .items_center()
                .cursor_pointer()
                .when(is_selected, |this| this.bg(theme.list_active))
                .hover(|this| this.bg(theme.list_hover))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.on_file_row(target_for_click.clone(), cx)
                    });
                })
                .child(label);
            with_file_context_menu(row, context)
        }
        FileRow::Tree {
            target,
            name,
            depth,
            is_dir,
            expanded,
            loading,
            size,
            ..
        } => {
            let context = if !is_dir {
                if let FileTarget::TreeFile(path) = &target {
                    Some(file_context_data(&entity, path.clone(), cx))
                } else {
                    None
                }
            } else {
                None
            };
            let target_for_click = target.clone();
            let icon = if is_dir {
                if expanded {
                    IconName::FolderOpen
                } else {
                    IconName::Folder
                }
            } else {
                IconName::File
            };
            let row_id = match &target_for_click {
                FileTarget::TreeDir(path) => format!("tree-dir-{path}"),
                FileTarget::TreeFile(path) => format!("tree-file-{path}"),
                FileTarget::CommitFile(index) => format!("tree-commit-{index}"),
                FileTarget::WorktreeFile(index) => format!("tree-worktree-{index}"),
            };
            let row = div()
                .id(ElementId::Name(row_id.into()))
                .h(px(30.0))
                .w_full()
                .pr_3()
                .pl(px(8.0 + depth as f32 * 14.0))
                .flex()
                .items_center()
                .gap_1()
                .cursor_pointer()
                .when(is_selected, |this| this.bg(theme.list_active))
                .hover(|this| this.bg(theme.list_hover))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.on_file_row(target_for_click.clone(), cx)
                    });
                })
                .when(is_dir, |this| {
                    this.child(
                        Icon::new(if expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(px(12.0))
                        .text_color(theme.muted_foreground),
                    )
                })
                .when(!is_dir, |this| this.child(div().w(px(12.0))))
                .child(Icon::new(icon).size(px(13.0)).text_color(if is_dir {
                    theme.info
                } else {
                    theme.muted_foreground
                }))
                .child(div().flex_1().text_sm().truncate().child(name))
                .when(loading, |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("…"),
                    )
                })
                .when_some(size, |this, size| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format_size(size)),
                    )
                });
            if let Some(context) = context {
                with_file_context_menu(row, context)
            } else {
                row.into_any_element()
            }
        }
    }
}

fn viewer_row_element(row: Option<ViewerRow>, cx: &mut App) -> AnyElement {
    let Some(row) = row else {
        return div().into_any_element();
    };

    match row {
        ViewerRow::Diff(row) => diff_view::render_row(row, cx),
        ViewerRow::Info(message) => div()
            .w_full()
            .px_4()
            .py_2()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(message)
            .into_any_element(),
        ViewerRow::Blob { number, text } => {
            let theme = cx.theme();
            div()
                .w_full()
                .flex()
                .items_start()
                .font_family(theme.mono_font_family.clone())
                .text_size(theme.mono_font_size)
                .child(
                    div()
                        .w(px(52.0))
                        .flex_shrink_0()
                        .flex()
                        .justify_end()
                        .pr_2()
                        .text_color(theme.muted_foreground)
                        .child(number.to_string()),
                )
                .child(div().flex_1().min_w_0().whitespace_normal().child(text))
                .into_any_element()
        }
    }
}

fn worktree_counts_text(cx: &App, counts: &WorktreeCounts) -> String {
    if cx.is_japanese() {
        format!(
            "ステージ {} · 未ステージ {} · 未追跡 {}",
            counts.staged, counts.unstaged, counts.untracked
        )
    } else {
        format!(
            "Staged {} · Unstaged {} · Untracked {}",
            counts.staged, counts.unstaged, counts.untracked
        )
    }
}

fn file_count_text(cx: &App, count: usize) -> String {
    if cx.is_japanese() {
        format!("{count} 件")
    } else if count == 1 {
        "1 file".to_string()
    } else {
        format!("{count} files")
    }
}

fn commit_count_text(cx: &App, count: usize) -> String {
    if cx.is_japanese() {
        format!("{count} コミット")
    } else if count == 1 {
        "1 commit".to_string()
    } else {
        format!("{count} commits")
    }
}

fn error_text(cx: &App, message: &str) -> String {
    format!("{}: {message}", cx.tr("読み込みに失敗しました"))
}

fn status_color(
    theme: &gpui_component::theme::Theme,
    status: FileStatus,
    diff: &DiffPalette,
) -> Hsla {
    match status {
        FileStatus::Added | FileStatus::Untracked => diff.added_mark,
        FileStatus::Deleted => diff.removed_mark,
        FileStatus::Renamed | FileStatus::Copied => theme.warning,
        FileStatus::TypeChange => theme.warning,
        FileStatus::Modified => theme.info,
        _ => theme.muted_foreground,
    }
}

fn short_oid(oid: Oid) -> String {
    let mut text = oid.to_string();
    text.truncate(7);
    text
}

fn absolute_time(timestamp: i64) -> String {
    Local
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|time| time.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

fn absolute_date(timestamp: i64) -> String {
    Local
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|time| time.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

fn format_size(size: u64) -> String {
    if size < 1_024 {
        format!("{size} B")
    } else if size < 1_024 * 1_024 {
        format!("{:.1} KB", size as f64 / 1_024.0)
    } else {
        format!("{:.1} MB", size as f64 / (1_024.0 * 1_024.0))
    }
}
