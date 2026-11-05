//! Shared unified / split diff rows and rendering.

use gpui::prelude::*;
use gpui::*;
use gpui_component::theme::ActiveTheme;
use rascal_core::{DiffRow, DiffRowKind, FileDiff};

use crate::diff_theme::{DiffPalette, DiffTheme, palette as diff_palette};
use crate::i18n::LanguageExt;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiffLayout {
    #[default]
    Unified,
    Split,
}

impl Global for DiffLayout {}

impl DiffLayout {
    pub fn key(self) -> &'static str {
        match self {
            DiffLayout::Unified => "unified",
            DiffLayout::Split => "split",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "unified" => Some(DiffLayout::Unified),
            "split" => Some(DiffLayout::Split),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DiffLayout::Unified => "1ペイン",
            DiffLayout::Split => "2ペイン",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffInfo {
    Submodule,
    Binary,
    NoContentChange,
    Truncated,
    NotChanged,
}

#[derive(Clone, Debug)]
pub struct SplitPair {
    pub old: Option<DiffRow>,
    pub new: Option<DiffRow>,
}

#[derive(Clone, Debug)]
pub enum DiffDisplayRow {
    Info(DiffInfo),
    Unified(DiffRow),
    Full(DiffRow),
    Split(SplitPair),
}

pub fn current_layout(cx: &App) -> DiffLayout {
    cx.try_global::<DiffLayout>().copied().unwrap_or_default()
}

pub fn set_layout(layout: DiffLayout, cx: &mut App) {
    if current_layout(cx) == layout {
        return;
    }
    cx.set_global(layout);
    cx.refresh_windows();
}

pub fn build_rows(diff: &FileDiff, layout: DiffLayout) -> Vec<DiffDisplayRow> {
    let mut rows = Vec::new();

    if diff.is_submodule {
        rows.push(DiffDisplayRow::Info(DiffInfo::Submodule));
        return rows;
    }
    if diff.is_binary {
        rows.push(DiffDisplayRow::Info(DiffInfo::Binary));
        return rows;
    }
    if diff.rows.is_empty() {
        rows.push(DiffDisplayRow::Info(DiffInfo::NoContentChange));
        return rows;
    }

    match layout {
        DiffLayout::Unified => {
            rows.extend(diff.rows.iter().cloned().map(|row| {
                if is_full_row(row.kind) {
                    DiffDisplayRow::Full(row)
                } else {
                    DiffDisplayRow::Unified(row)
                }
            }));
        }
        DiffLayout::Split => build_split_rows(&diff.rows, &mut rows),
    }

    if diff.truncated {
        rows.push(DiffDisplayRow::Info(DiffInfo::Truncated));
    }

    rows
}

fn is_full_row(kind: DiffRowKind) -> bool {
    matches!(
        kind,
        DiffRowKind::HunkHeader | DiffRowKind::Meta | DiffRowKind::NoNewline
    )
}

fn build_split_rows(input: &[DiffRow], output: &mut Vec<DiffDisplayRow>) {
    let mut removed = Vec::new();
    let mut added = Vec::new();

    for row in input {
        match row.kind {
            DiffRowKind::Removed => removed.push(row.clone()),
            DiffRowKind::Added => added.push(row.clone()),
            DiffRowKind::Context => {
                flush_pending(output, &mut removed, &mut added);
                output.push(DiffDisplayRow::Split(SplitPair {
                    old: Some(row.clone()),
                    new: Some(row.clone()),
                }));
            }
            DiffRowKind::HunkHeader | DiffRowKind::Meta | DiffRowKind::NoNewline => {
                flush_pending(output, &mut removed, &mut added);
                output.push(DiffDisplayRow::Full(row.clone()));
            }
        }
    }

    flush_pending(output, &mut removed, &mut added);
}

fn flush_pending(
    output: &mut Vec<DiffDisplayRow>,
    removed: &mut Vec<DiffRow>,
    added: &mut Vec<DiffRow>,
) {
    let count = removed.len().max(added.len());
    for index in 0..count {
        output.push(DiffDisplayRow::Split(SplitPair {
            old: removed.get(index).cloned(),
            new: added.get(index).cloned(),
        }));
    }
    removed.clear();
    added.clear();
}

pub fn render_row(row: DiffDisplayRow, cx: &mut App) -> AnyElement {
    let theme = cx.theme();
    let diff_theme = cx.try_global::<DiffTheme>().copied().unwrap_or_default();
    let diff = diff_palette(theme, diff_theme);

    match row {
        DiffDisplayRow::Info(info) => info_row(info, cx),
        DiffDisplayRow::Unified(row) => unified_row(row, theme, &diff),
        DiffDisplayRow::Full(row) => full_row(row, theme, &diff),
        DiffDisplayRow::Split(pair) => split_row(pair, theme, &diff),
    }
}

fn info_row(info: DiffInfo, cx: &App) -> AnyElement {
    let message = match info {
        DiffInfo::Submodule => cx.tr("サブモジュールのため差分を表示できません"),
        DiffInfo::Binary => cx.tr("バイナリファイルのため表示できません"),
        DiffInfo::NoContentChange => cx.tr("内容の変更はありません（モード変更など）"),
        DiffInfo::Truncated => cx.tr("差分が大きいため、途中まで表示しています。"),
        DiffInfo::NotChanged => cx.tr("このコミットではこのファイルは変更されていません"),
    };
    div()
        .w_full()
        .px_4()
        .py_2()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(message)
        .into_any_element()
}

fn full_row(row: DiffRow, theme: &gpui_component::theme::Theme, diff: &DiffPalette) -> AnyElement {
    let (background, text_color) = match row.kind {
        DiffRowKind::HunkHeader => (diff.hunk_bg, diff.hunk_fg),
        DiffRowKind::Meta | DiffRowKind::NoNewline => {
            (theme.muted.opacity(0.35), theme.muted_foreground)
        }
        _ => (theme.transparent, diff.context_fg),
    };

    div()
        .w_full()
        .px_3()
        .py_1()
        .bg(background)
        .font_family(theme.mono_font_family.clone())
        .text_size(theme.mono_font_size)
        .text_color(text_color)
        .whitespace_normal()
        .child(row.text)
        .into_any_element()
}

fn unified_row(
    row: DiffRow,
    theme: &gpui_component::theme::Theme,
    diff: &DiffPalette,
) -> AnyElement {
    let (background, text_color, sign, sign_color) = match row.kind {
        DiffRowKind::HunkHeader => (diff.hunk_bg, diff.hunk_fg, String::new(), diff.hunk_fg),
        DiffRowKind::Added => (
            diff.added_bg,
            diff.added_fg,
            "+".to_string(),
            diff.added_mark,
        ),
        DiffRowKind::Removed => (
            diff.removed_bg,
            diff.removed_fg,
            "-".to_string(),
            diff.removed_mark,
        ),
        DiffRowKind::NoNewline => (
            theme.muted.opacity(0.35),
            theme.muted_foreground,
            String::new(),
            theme.muted_foreground,
        ),
        DiffRowKind::Context => (
            theme.transparent,
            diff.context_fg,
            " ".to_string(),
            theme.muted_foreground,
        ),
        DiffRowKind::Meta => (
            theme.muted.opacity(0.35),
            theme.muted_foreground,
            String::new(),
            theme.muted_foreground,
        ),
    };

    if row.kind == DiffRowKind::HunkHeader {
        return div()
            .w_full()
            .px_3()
            .py_1()
            .bg(background)
            .font_family(theme.mono_font_family.clone())
            .text_size(theme.mono_font_size)
            .text_color(text_color)
            .whitespace_normal()
            .child(row.text)
            .into_any_element();
    }

    div()
        .w_full()
        .flex()
        .items_start()
        .bg(background)
        .font_family(theme.mono_font_family.clone())
        .text_size(theme.mono_font_size)
        .text_color(text_color)
        .child(line_number(row.old_lineno, diff.line_no))
        .child(line_number(row.new_lineno, diff.line_no))
        .child(
            div()
                .w(px(16.0))
                .flex_shrink_0()
                .text_color(sign_color)
                .child(sign),
        )
        .child(div().flex_1().min_w_0().whitespace_normal().child(row.text))
        .into_any_element()
}

fn split_row(
    pair: SplitPair,
    theme: &gpui_component::theme::Theme,
    diff: &DiffPalette,
) -> AnyElement {
    div()
        .w_full()
        .flex()
        .child(split_cell(pair.old.as_ref(), SplitSide::Old, theme, diff))
        .child(split_cell(pair.new.as_ref(), SplitSide::New, theme, diff))
        .into_any_element()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SplitSide {
    Old,
    New,
}

fn split_cell(
    row: Option<&DiffRow>,
    side: SplitSide,
    theme: &gpui_component::theme::Theme,
    diff: &DiffPalette,
) -> AnyElement {
    let (background, text_color, sign, sign_color) = match (side, row.map(|row| row.kind)) {
        (SplitSide::Old, Some(DiffRowKind::Removed)) => (
            diff.removed_bg,
            diff.removed_fg,
            "-".to_string(),
            diff.removed_mark,
        ),
        (SplitSide::New, Some(DiffRowKind::Added)) => (
            diff.added_bg,
            diff.added_fg,
            "+".to_string(),
            diff.added_mark,
        ),
        (_, Some(DiffRowKind::Context)) => (
            theme.transparent,
            diff.context_fg,
            " ".to_string(),
            theme.muted_foreground,
        ),
        (_, Some(DiffRowKind::Meta | DiffRowKind::NoNewline)) => (
            theme.muted.opacity(0.35),
            theme.muted_foreground,
            String::new(),
            theme.muted_foreground,
        ),
        (_, None) => (
            theme.muted.opacity(0.08),
            theme.muted_foreground,
            " ".to_string(),
            theme.muted_foreground,
        ),
        _ => (
            theme.transparent,
            diff.context_fg,
            " ".to_string(),
            theme.muted_foreground,
        ),
    };

    let number = row.map(|row| match side {
        SplitSide::Old => row.old_lineno,
        SplitSide::New => row.new_lineno,
    });

    div()
        .flex_1()
        .min_w_0()
        .bg(background)
        .when(side == SplitSide::Old, |this| {
            this.border_r_1().border_color(theme.border.opacity(0.5))
        })
        .flex()
        .items_start()
        .font_family(theme.mono_font_family.clone())
        .text_size(theme.mono_font_size)
        .text_color(text_color)
        .child(line_number(number.flatten(), diff.line_no))
        .child(
            div()
                .w(px(14.0))
                .flex_shrink_0()
                .text_color(sign_color)
                .child(sign),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .whitespace_normal()
                .child(row.map(|row| row.text.clone()).unwrap_or_default()),
        )
        .into_any_element()
}

fn line_number(number: Option<u32>, color: Hsla) -> AnyElement {
    div()
        .w(px(42.0))
        .flex_shrink_0()
        .flex()
        .justify_end()
        .pr_2()
        .text_color(color)
        .child(number.map(|number| number.to_string()).unwrap_or_default())
        .into_any_element()
}

pub fn layout_toggle(id: &'static str, cx: &mut App) -> AnyElement {
    let theme = cx.theme();
    let current = current_layout(cx);
    let mut group = div()
        .flex()
        .items_center()
        .rounded(px(4.0))
        .border_1()
        .border_color(theme.border)
        .overflow_hidden();

    for layout in [DiffLayout::Unified, DiffLayout::Split] {
        let selected = current == layout;
        group = group.child(
            div()
                .id(ElementId::Name(format!("{id}-{}", layout.key()).into()))
                .px_2()
                .py_0p5()
                .text_xs()
                .cursor_pointer()
                .when(selected, |this| {
                    this.bg(theme.primary.opacity(0.16))
                        .text_color(theme.foreground)
                })
                .when(!selected, |this| {
                    this.text_color(theme.muted_foreground)
                        .hover(|this| this.bg(theme.list_hover))
                })
                .on_click(move |_, _, cx| set_layout(layout, cx))
                .child(cx.tr(layout.label())),
        );
    }

    group.into_any_element()
}

#[cfg(test)]
mod tests;
