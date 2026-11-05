//! Minimal runtime localization.
//!
//! Strings are authored in Japanese at the call site and translated through a
//! small lookup. This keeps the UI code readable and makes the English mode
//! self-contained.

use gpui::{App, Global, SharedString};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    Japanese,
    English,
}

impl Global for Language {}

impl Language {
    pub fn key(self) -> &'static str {
        match self {
            Language::Japanese => "ja",
            Language::English => "en",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "ja" => Some(Language::Japanese),
            "en" => Some(Language::English),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Language::Japanese => "日本語",
            Language::English => "English",
        }
    }
}

pub trait LanguageExt {
    fn language(&self) -> Language;
    fn is_japanese(&self) -> bool;
    fn tr(&self, japanese: &str) -> SharedString;
    fn tr_key(&self, japanese: &str, lookup: &str) -> SharedString;
}

impl LanguageExt for App {
    fn language(&self) -> Language {
        self.try_global::<Language>().copied().unwrap_or_default()
    }

    fn is_japanese(&self) -> bool {
        self.language() == Language::Japanese
    }

    fn tr(&self, japanese: &str) -> SharedString {
        if self.is_japanese() {
            japanese.to_string().into()
        } else {
            english(japanese).unwrap_or(japanese).to_string().into()
        }
    }

    fn tr_key(&self, japanese: &str, lookup: &str) -> SharedString {
        if self.is_japanese() {
            japanese.to_string().into()
        } else {
            english(lookup).unwrap_or(japanese).to_string().into()
        }
    }
}

fn english(japanese: &str) -> Option<&'static str> {
    Some(match japanese {
        // Application / menus
        "Rascalについて" => "About Rascal",
        "Rascalを終了" => "Quit Rascal",
        "設定…" => "Settings…",
        "ファイル" => "File",
        "menu.file" => "File",
        "mode.files" => "Files",
        "リポジトリを開く" => "Open Repository",
        "更新" => "Refresh",
        "タブを閉じる" => "Close Tab",
        "ウィンドウを閉じる" => "Close Window",
        "表示" => "View",
        "ダッシュボード" => "Dashboard",
        "コミットを絞り込む" => "Filter commits",
        "アプリテーマを切り替える" => "Toggle App Theme",
        "テーマを切り替える" => "Toggle Theme",
        "設定" => "Settings",
        "1ペイン" => "Unified",
        "2ペイン" => "Split",

        // About
        "GPUIで作られた、軽量・読み取り専用のGit履歴ビューア。" => {
            "A lightweight, read-only Git history viewer built with GPUI."
        }
        "履歴・差分・ファイル内容の閲覧だけを行い、リポジトリは変更しません。" => {
            "It only views history, diffs and file contents; repositories are never modified."
        }

        // Dashboard
        "軽量・高速なGit履歴ビューア" => "A lightweight, fast Git history viewer",
        "{} 個のリポジトリを開いています" => "{} repositories open",
        "最近のリポジトリ" => "Recent Repositories",
        "まだ履歴がありません。リポジトリを開くと、ここに表示されます。" => {
            "No history yet. Repositories you open will appear here."
        }
        "コミット履歴・ブランチ・差分プレビュー" => {
            "Commit history, branches and diff preview"
        }
        "開いています" => "Open",
        "開く" => "Open",
        "アプリを指定して開く" => "Open With…",
        "対象フォルダをFinderで開く" => "Show Folder in Finder",
        "絶対パスをコピー" => "Copy Absolute Path",
        "相対パスをコピー" => "Copy Relative Path",
        "ファイル編集履歴" => "File History",
        "ファイルを開けませんでした" => "Could not open file",
        "ウィンドウを開けませんでした" => "Could not open window",
        "作者" => "Author",
        "日付" => "Date",
        "SHA" => "SHA",
        "このファイルを変更したコミットはありません" => {
            "No commits changed this file."
        }
        "見つかりません" => "Not found",
        "リポジトリが見つかりません" => "Repository not found",
        "Gitリポジトリではありません: {err}" => "Not a Git repository: {err}",
        "ダッシュボード — Rascal" => "Dashboard — Rascal",

        // Settings
        "閉じる" => "Close",
        "言語" => "Language",
        "外観" => "Appearance",
        "アプリテーマ" => "App theme",
        "システム" => "System",
        "ライト" => "Light",
        "ダーク" => "Dark",
        "差分テーマ" => "Diff theme",
        "標準" => "Standard",
        "高コントラスト" => "High contrast",
        "色覚配慮" => "Colorblind friendly",
        "フォント" => "Fonts",
        "フォントを検索" => "Search fonts",
        "UIフォント" => "UI font",
        "コード・差分フォント" => "Code & diff font",
        "プレビュー" => "Preview",
        "設定は自動的に保存されます。" => "Settings are saved automatically.",
        "元に戻す" => "Reset",
        "この画面について" => "About this screen",
        "システムフォント" => "System font",
        "ヒラギノ角ゴシック" => "Hiragino Sans",
        "ヒラギノ角ゴ ProN" => "Hiragino Kaku Gothic ProN",
        "等幅フォント" => "Monospaced font",

        // History
        "履歴" => "History",
        "未コミットの変更" => "Uncommitted changes",
        "ステージ {} · 未ステージ {} · 未追跡 {}" => {
            "Staged {} · Unstaged {} · Untracked {}"
        }
        "さらに読み込む" => "Load more",
        "履歴を読み込み中…" => "Loading history…",
        "一致するコミットがありません" => "No matching commits",
        "コミットがありません" => "No commits",
        "読み込み中…" => "Loading…",
        "ブランチ" => "Branches",
        "作業ツリーはクリーン" => "Working tree clean",
        "変更" => "Changes",
        "一覧" => "List",
        "ツリー" => "Tree",
        "{} 件" => "{} files",
        "コミットを読み込み中…" => "Loading commit…",
        "ファイルを選択できません" => "No files available",
        "変更ファイルはありません" => "No changed files",
        "作業ツリーを読み込み中…" => "Loading working tree…",
        "変更はありません" => "No changes",
        "コミットを選択してください" => "Select a commit",
        "ファイルツリーを読み込み中…" => "Loading file tree…",
        "ファイルが多すぎるため、一部のみ表示しています。" => {
            "Too many files; showing a subset."
        }
        "該当するファイルがありません" => "No files",
        "bin" => "bin",
        "ビューア" => "Viewer",
        "ファイルを選択してください" => "Select a file",
        "表示する内容がありません" => "Nothing to display",
        "差分" => "Diff",
        "内容" => "Content",
        "第1親" => "first parent",
        "空ツリー" => "empty tree",
        "ステージ済み" => "Staged",
        "未ステージ" => "Unstaged",
        "未追跡" => "Untracked",
        "全文" => "Full message",
        "たたむ" => "Collapse",
        "SHAをコピー" => "Copy SHA",
        "親 {}" => "{} parent(s)",
        "読み取り専用" => "Read-only",
        "{} commits" => "{} commits",
        "{} コミット" => "{} commits",
        "変更 {}" => "{} changed",
        "バイナリファイルのため表示できません" => {
            "Binary file cannot be displayed."
        }
        "バイナリファイルのため表示できません ({} bytes)" => {
            "Binary file cannot be displayed ({} bytes)."
        }
        "サブモジュールのため差分を表示できません" => {
            "Submodule diff cannot be displayed."
        }
        "内容の変更はありません（モード変更など）" => {
            "No content changes (mode change, etc.)."
        }
        "差分が大きいため、途中まで表示しています。" => {
            "The diff is large; showing a truncated view."
        }
        "ファイルが大きいため、先頭のみ表示しています。" => {
            "The file is large; showing the beginning only."
        }
        "読み込みに失敗しました" => "Failed to load",
        "リポジトリを開けませんでした" => "Could not open repository",
        "リポジトリ情報を読み込めませんでした" => {
            "Could not load repository information"
        }
        "ファイルツリー" => "File tree",
        "作業ツリー" => "Working tree",
        "コミット" => "Commit",
        "メタ情報" => "Metadata",
        "差分を読み込めませんでした" => "Could not load diff",
        "履歴を読み込めませんでした" => "Could not load history",
        "コミットを読み込めませんでした" => "Could not load commit",
        "ファイル内容を読み込めませんでした" => "Could not load file content",
        "ファイルツリーを読み込めませんでした" => "Could not load file tree",
        "作業ツリーを読み込めませんでした" => "Could not load working tree",
        _ => return None,
    })
}
