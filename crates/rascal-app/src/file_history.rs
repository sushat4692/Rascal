//! Standalone two-pane window: commits touching one file and their diff.

use std::{path::PathBuf, rc::Rc};

use chrono::{Local, TimeZone};
use gpui::prelude::*;
use gpui::*;
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::theme::ActiveTheme;
use gpui_component::{Disableable, v_flex};
use rascal_core::{CommitSummary, FileDiff, HISTORY_PAGE_SIZE, Oid, short_oid};

use crate::app::{CloseWindow, Quit};
use crate::git_worker::{Command, Event, GitWorker};
use crate::i18n::LanguageExt;
use crate::ui::diff_view::{self, DiffDisplayRow, DiffInfo, DiffLayout};
use crate::ui::image_view::{self, ImageView};

const LEFT_PANE_WIDTH: f32 = 360.0;
const HISTORY_ROW_HEIGHT: f32 = 76.0;

pub struct FileHistoryView {
    repo_name: String,
    rev: String,
    path: String,
    commits: Vec<CommitSummary>,
    selected: Option<Oid>,
    loading: bool,
    has_more: bool,
    error: Option<String>,
    commands: async_channel::Sender<Command>,
    next_req: u64,
    pending_req: u64,

    diff: Option<FileDiff>,
    diff_rows: Rc<Vec<DiffDisplayRow>>,
    diff_list: ListState,
    diff_layout: DiffLayout,
    diff_loading: bool,
    diff_error: Option<String>,
    diff_req: u64,
    image_view: Option<Entity<ImageView>>,
    image_loading: bool,
    image_error: Option<String>,
    image_req: u64,
    focus_handle: FocusHandle,
}

impl FileHistoryView {
    pub fn new(
        repo_path: PathBuf,
        rev: String,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let repo_name = repo_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "repository".to_string());
        let (worker, events) = GitWorker::spawn(repo_path);
        let commands = worker.commands;

        let _ = commands.try_send(Command::LoadFileHistory {
            req: 1,
            rev: rev.clone(),
            path: path.clone(),
            offset: 0,
            limit: HISTORY_PAGE_SIZE,
        });

        let mut view = Self {
            repo_name,
            rev,
            path,
            commits: Vec::new(),
            selected: None,
            loading: true,
            has_more: false,
            error: None,
            commands,
            next_req: 1,
            pending_req: 1,
            diff: None,
            diff_rows: Rc::new(Vec::new()),
            diff_list: ListState::new(0, ListAlignment::Top, px(1000.0)),
            diff_layout: diff_view::current_layout(cx),
            diff_loading: false,
            diff_error: None,
            diff_req: 0,
            image_view: None,
            image_loading: false,
            image_error: None,
            image_req: 0,
            focus_handle: cx.focus_handle(),
        };

        cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = events.recv().await {
                if this
                    .update(cx, |view, cx| view.handle_event(event, cx))
                    .is_err()
                {
                    break;
                }
                let _ = cx.update(|window, _| window.refresh());
            }
        })
        .detach();

        view.focus_handle.focus(window);
        view.loading = true;
        view
    }

    fn handle_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::FileHistory { req, page } => {
                if req != self.pending_req {
                    return;
                }
                self.loading = false;
                self.error = None;
                self.has_more = page.has_more;
                self.commits.extend(page.commits);
                if self.selected.is_none()
                    && let Some(first) = self.commits.first()
                {
                    let oid = first.oid;
                    self.select_commit(oid, cx);
                }
                cx.notify();
            }
            Event::FileHistoryDiff { req, diff } => {
                if req != self.diff_req {
                    return;
                }
                self.diff_loading = false;
                self.diff_error = None;
                self.set_diff(diff, cx);
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
            Event::Failed { message } => {
                self.loading = false;
                self.error = Some(message);
                cx.notify();
            }
            Event::Error { req, message, .. } => {
                if req == self.image_req {
                    self.image_loading = false;
                    self.image_error = Some(message);
                } else if req == self.diff_req {
                    self.diff_loading = false;
                    self.diff_error = Some(message);
                } else if req == self.pending_req {
                    self.loading = false;
                    self.error = Some(message);
                }
                cx.notify();
            }
            _ => {}
        }
    }

    fn select_commit(&mut self, oid: Oid, cx: &mut Context<Self>) {
        if self.selected == Some(oid) {
            return;
        }
        self.selected = Some(oid);
        self.image_view = None;
        self.image_loading = false;
        self.image_error = None;
        if image_view::is_image_path(&self.path) {
            self.diff_loading = false;
            self.diff_error = None;
            self.diff = None;
            self.diff_req = 0;
            self.set_diff_rows(Vec::new());
            self.image_loading = true;
            self.image_req += 1;
            let _ = self.commands.try_send(Command::LoadCommitImageDiff {
                req: self.image_req,
                oid,
                old_path: None,
                new_path: Some(self.path.clone()),
            });
        } else {
            self.diff_loading = true;
            self.diff_error = None;
            self.diff = None;
            self.image_req = 0;
            self.set_diff_rows(Vec::new());
            self.diff_req += 1;
            let _ = self.commands.try_send(Command::LoadFileHistoryDiff {
                req: self.diff_req,
                oid,
                path: self.path.clone(),
            });
        }
        cx.notify();
    }

    fn set_diff(&mut self, diff: Option<FileDiff>, cx: &App) {
        let layout = diff_view::current_layout(cx);
        let rows = match &diff {
            Some(diff) => diff_view::build_rows(diff, layout),
            None => vec![DiffDisplayRow::Info(DiffInfo::NotChanged)],
        };
        self.diff_layout = layout;
        self.diff = diff;
        self.image_view = None;
        self.image_loading = false;
        self.image_error = None;
        self.set_diff_rows(rows);
        self.diff_list.scroll_to(ListOffset {
            item_ix: 0,
            offset_in_item: px(0.0),
        });
    }

    fn rebuild_diff_rows(&mut self, cx: &mut Context<Self>) {
        let layout = diff_view::current_layout(cx);
        let rows = match &self.diff {
            Some(diff) => diff_view::build_rows(diff, layout),
            None => vec![DiffDisplayRow::Info(DiffInfo::NotChanged)],
        };
        self.diff_layout = layout;
        self.set_diff_rows(rows);
        self.diff_list.scroll_to(ListOffset {
            item_ix: 0,
            offset_in_item: px(0.0),
        });
        cx.notify();
    }

    fn set_diff_rows(&mut self, rows: Vec<DiffDisplayRow>) {
        self.diff_list.reset(rows.len());
        self.diff_rows = Rc::new(rows);
    }

    fn load_more(&mut self, cx: &mut Context<Self>) {
        if self.loading || !self.has_more {
            return;
        }
        self.next_req += 1;
        self.pending_req = self.next_req;
        self.loading = true;
        let _ = self.commands.try_send(Command::LoadFileHistory {
            req: self.next_req,
            rev: self.rev.clone(),
            path: self.path.clone(),
            offset: self.commits.len(),
            limit: HISTORY_PAGE_SIZE,
        });
        cx.notify();
    }

    fn selected_commit(&self) -> Option<&CommitSummary> {
        let oid = self.selected?;
        self.commits.iter().find(|commit| commit.oid == oid)
    }

    fn render_left(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let entity = cx.entity();
        let selected = self.selected;
        let commits = self.commits.clone();

        let header = div()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
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
                            .child(self.commit_count_text(cx)),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .truncate()
                    .child(self.path.clone()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("{} · {}", self.repo_name, self.rev_short())),
            );

        let body = if let Some(error) = &self.error {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                .text_xs()
                .text_color(theme.danger)
                .child(format!("{}: {error}", cx.tr("読み込みに失敗しました")))
                .into_any_element()
        } else if self.loading && commits.is_empty() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(cx.tr("読み込み中…"))
                .into_any_element()
        } else if commits.is_empty() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(cx.tr("このファイルを変更したコミットはありません"))
                .into_any_element()
        } else {
            uniform_list(
                "file-history-list",
                commits.len(),
                move |range, _window, cx| {
                    let theme = cx.theme();
                    range
                        .filter_map(|index| {
                            commits.get(index).map(|commit| {
                                history_row(commit, selected == Some(commit.oid), &entity, theme)
                            })
                        })
                        .collect::<Vec<_>>()
                },
            )
            .flex_1()
            .min_h_0()
            .into_any_element()
        };

        let footer = if self.has_more {
            Some(
                div()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        Button::new("file-history-more")
                            .ghost()
                            .label(cx.tr("さらに読み込む"))
                            .disabled(self.loading)
                            .on_click(cx.listener(|this, _, _, cx| this.load_more(cx))),
                    ),
            )
        } else {
            None
        };

        v_flex()
            .w(px(LEFT_PANE_WIDTH))
            .flex_shrink_0()
            .h_full()
            .border_r_1()
            .border_color(theme.border)
            .child(header)
            .child(body)
            .when_some(footer, |this, footer| this.child(footer))
            .into_any_element()
    }

    fn render_right(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let header = self.render_diff_header(cx);
        let body = self.render_diff_body(cx);
        let theme = cx.theme();

        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(theme.background)
            .child(header)
            .child(body)
            .into_any_element()
    }

    fn render_diff_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(commit) = self.selected_commit() else {
            let theme = cx.theme();
            return div()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(theme.border)
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(cx.tr("コミットを選択してください"))
                .into_any_element();
        };
        let layout_control =
            (self.image_view.is_none() && !self.image_loading && self.image_error.is_none())
                .then(|| diff_view::layout_toggle("history-diff-layout", cx));
        let theme = cx.theme();

        let additions = self.diff.as_ref().map(|diff| diff.additions).unwrap_or(0);
        let deletions = self.diff.as_ref().map(|diff| diff.deletions).unwrap_or(0);

        div()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.muted_foreground)
                            .child(cx.tr("差分")),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when_some(layout_control, |this, control| this.child(control))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.success)
                                    .child(format!("+{additions}")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.danger)
                                    .child(format!("-{deletions}")),
                            ),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .truncate()
                    .child(commit.summary.clone()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!(
                        "{} · {} · {}",
                        commit.author_name,
                        absolute_date(commit.author_time),
                        short_oid(commit.oid)
                    )),
            )
            .into_any_element()
    }

    fn render_diff_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();

        if let Some(image) = &self.image_view {
            return div()
                .flex_1()
                .min_h_0()
                .w_full()
                .child(image.clone())
                .into_any_element();
        }

        if self.image_loading {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p_6()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(cx.tr("画像を読み込み中…"))
                .into_any_element();
        }

        if let Some(error) = &self.image_error {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p_6()
                .text_sm()
                .text_color(theme.danger)
                .child(format!("{}: {error}", cx.tr("画像を読み込めません")))
                .into_any_element();
        }

        if let Some(error) = &self.diff_error {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p_6()
                .text_sm()
                .text_color(theme.danger)
                .child(format!("{}: {error}", cx.tr("読み込みに失敗しました")))
                .into_any_element();
        }

        if self.diff_loading {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p_6()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(cx.tr("読み込み中…"))
                .into_any_element();
        }

        if self.diff_rows.is_empty() {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p_6()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(cx.tr("表示する内容がありません"))
                .into_any_element();
        }

        let rows = self.diff_rows.clone();
        let list = list(self.diff_list.clone(), move |index, _window, cx| match rows
            .get(index)
            .cloned()
        {
            Some(row) => diff_view::render_row(row, cx),
            None => div().into_any_element(),
        });
        div()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(list.size_full())
            .into_any_element()
    }

    fn rev_short(&self) -> String {
        if self.rev == "HEAD" {
            "HEAD".to_string()
        } else {
            self.rev.chars().take(7).collect()
        }
    }

    fn commit_count_text(&self, cx: &App) -> String {
        if cx.is_japanese() {
            format!("{} 件", self.commits.len())
        } else if self.commits.len() == 1 {
            "1 commit".to_string()
        } else {
            format!("{} commits", self.commits.len())
        }
    }
}

fn history_row(
    commit: &CommitSummary,
    selected: bool,
    entity: &Entity<FileHistoryView>,
    theme: &gpui_component::theme::Theme,
) -> AnyElement {
    let oid = commit.oid;
    let entity = entity.clone();

    div()
        .id(ElementId::Name(
            format!("file-history-{}", commit.oid).into(),
        ))
        .w_full()
        .h(px(HISTORY_ROW_HEIGHT))
        .px_3()
        .py_2()
        .flex()
        .flex_col()
        .justify_center()
        .gap_0p5()
        .text_color(theme.foreground)
        .cursor_pointer()
        .when(selected, |this| this.bg(theme.list_active))
        .hover(|this| this.bg(theme.list_hover))
        .on_click(move |_, _, cx| {
            entity.update(cx, |this, cx| this.select_commit(oid, cx));
        })
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(commit.author_name.clone()),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(absolute_date(commit.author_time)),
                ),
        )
        .child(
            div()
                .w_full()
                .flex_shrink_0()
                .truncate()
                .text_sm()
                .text_color(theme.foreground)
                .child(commit.summary.clone()),
        )
        .child(
            div()
                .text_xs()
                .font_family(theme.mono_font_family.clone())
                .text_color(theme.muted_foreground)
                .child(short_oid(commit.oid)),
        )
        .into_any_element()
}

fn absolute_date(timestamp: i64) -> String {
    Local
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|time| time.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

impl Focusable for FileHistoryView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for FileHistoryView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if window.focused(cx).is_none() {
            self.focus_handle.focus(window);
        }
        let font_size = cx.theme().font_size;
        window.set_rem_size(font_size);
        let layout = diff_view::current_layout(cx);
        if self.diff_layout != layout {
            self.rebuild_diff_rows(cx);
        }
        let left = self.render_left(cx);
        let right = self.render_right(cx);
        let theme = cx.theme();

        div()
            .key_context("Rascal")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|_this, _: &CloseWindow, window, _cx| {
                window.remove_window();
            }))
            .on_action(cx.listener(|_this, _: &Quit, _window, cx| {
                cx.quit();
            }))
            .size_full()
            .flex()
            .font_family(theme.font_family.clone())
            .text_color(theme.foreground)
            .bg(theme.background)
            .child(left)
            .child(right)
    }
}
