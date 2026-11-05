//! Application shell: window title bar, repository tabs and the welcome view.

use std::path::{Path, PathBuf};

use chrono::TimeZone;

use gpui::StatefulInteractiveElement;
use gpui::hsla;
use gpui::prelude::*;
use gpui::*;
use gpui_component::IndexPath;
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::notification::Notification;
use gpui_component::select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState};
use gpui_component::theme::{ActiveTheme, Theme, ThemeMode};
use gpui_component::{Disableable, Icon, IconName, Selectable, Sizable, TitleBar, WindowExt};

use crate::diff_theme::DiffTheme;
use crate::i18n::{Language, LanguageExt};
use crate::session::RepoSession;
use crate::storage::{AppState, Storage};
use crate::ui::diff_view::DiffLayout;
use crate::ui::image_view::ImageDiffMode;

actions!(
    rascal,
    [
        OpenRepository,
        Refresh,
        CloseTab,
        ToggleTheme,
        ShowDashboard,
        OpenSettings,
        FocusFilter,
        Quit,
        About,
        CloseWindow,
    ]
);

type FontSelect = SelectState<SearchableVec<FontItem>>;

#[derive(Clone)]
struct FontItem {
    name: String,
    label: SharedString,
}

impl FontItem {
    fn new(name: impl Into<String>, label: impl Into<SharedString>) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
        }
    }
}

impl SelectItem for FontItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.name
    }
}

const CONTEXT: &str = "Rascal";

pub struct RascalApp {
    tabs: Vec<Entity<RepoSession>>,
    active: usize,
    storage: Storage,
    state: AppState,
    next_tab_id: u64,
    showing_dashboard: bool,
    showing_settings: bool,
    settings_return_dashboard: bool,
    ui_font_select: Option<Entity<FontSelect>>,
    mono_font_select: Option<Entity<FontSelect>>,
    font_select_subscriptions: Vec<Subscription>,
    settings_scroll: ScrollHandle,
    focus_handle: FocusHandle,
    _diff_layout_subscription: Option<Subscription>,
    _image_diff_mode_subscription: Option<Subscription>,
}

impl RascalApp {
    pub fn init(cx: &mut App) {
        // The language is persisted, and it must be available before the
        // menus are installed.
        let state = Storage::new().load();
        let language = state
            .language
            .as_deref()
            .and_then(Language::from_key)
            .unwrap_or_default();
        cx.set_global(language);

        cx.bind_keys([
            KeyBinding::new("cmd-o", OpenRepository, Some(CONTEXT)),
            KeyBinding::new("cmd-r", Refresh, Some(CONTEXT)),
            KeyBinding::new("cmd-w", CloseWindow, Some(CONTEXT)),
            KeyBinding::new("cmd-shift-w", CloseTab, Some(CONTEXT)),
            KeyBinding::new("cmd-shift-t", ToggleTheme, Some(CONTEXT)),
            KeyBinding::new("cmd-shift-d", ShowDashboard, Some(CONTEXT)),
            KeyBinding::new("cmd-,", OpenSettings, Some(CONTEXT)),
            KeyBinding::new("cmd-f", FocusFilter, Some(CONTEXT)),
            KeyBinding::new("cmd-q", Quit, Some(CONTEXT)),
        ]);

        install_menus(cx);
    }

    pub fn new(initial: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let storage = Storage::new();
        let state = storage.load();

        match state.theme.as_deref() {
            Some("dark") => Theme::change(ThemeMode::Dark, Some(window), cx),
            Some("light") => Theme::change(ThemeMode::Light, Some(window), cx),
            _ => Theme::sync_system_appearance(Some(window), cx),
        }
        apply_theme_tweaks(cx);
        let language = state
            .language
            .as_deref()
            .and_then(Language::from_key)
            .unwrap_or_default();
        cx.set_global(language);
        let initial_ui_font = state.ui_font.clone();
        let initial_mono_font = state.mono_font.clone();
        apply_fonts(cx, initial_ui_font.as_deref(), initial_mono_font.as_deref());

        let diff_theme = state
            .diff_theme
            .as_deref()
            .and_then(DiffTheme::from_key)
            .unwrap_or_default();
        cx.set_global(diff_theme);
        let diff_layout = state
            .diff_layout
            .as_deref()
            .and_then(DiffLayout::from_key)
            .unwrap_or_default();
        cx.set_global(diff_layout);
        let image_diff_mode = state
            .image_diff_mode
            .as_deref()
            .and_then(ImageDiffMode::from_key)
            .unwrap_or_default();
        cx.set_global(image_diff_mode);

        let focus_handle = cx.focus_handle();
        let mut app = Self {
            tabs: Vec::new(),
            active: 0,
            storage,
            state,
            next_tab_id: 1,
            showing_dashboard: true,
            showing_settings: false,
            settings_return_dashboard: true,
            ui_font_select: None,
            mono_font_select: None,
            font_select_subscriptions: Vec::new(),
            settings_scroll: ScrollHandle::new(),
            focus_handle,
            _diff_layout_subscription: None,
            _image_diff_mode_subscription: None,
        };
        app._diff_layout_subscription = Some(cx.observe_global::<DiffLayout>(|this, cx| {
            let layout = *cx.global::<DiffLayout>();
            this.state.diff_layout = Some(layout.key().to_string());
            let _ = this.storage.save(&this.state);
        }));
        app._image_diff_mode_subscription = Some(cx.observe_global::<ImageDiffMode>(|this, cx| {
            let mode = *cx.global::<ImageDiffMode>();
            this.state.image_diff_mode = Some(mode.key().to_string());
            let _ = this.storage.save(&this.state);
        }));

        let valid = |path: &PathBuf| path.exists() && rascal_core::GitRepo::open(path).is_ok();
        let explicit: Vec<PathBuf> = initial.into_iter().filter(|path| valid(path)).collect();
        let explicit_request = !explicit.is_empty();
        let saved_open = app.state.open_repos.clone();
        let saved_active = app.state.active_repo.clone();
        let saved_last = app.state.last_repo.clone();

        let restore = if explicit_request {
            explicit
        } else if let Some(open) = saved_open {
            open.into_iter().filter(|path| valid(path)).collect()
        } else {
            saved_last.into_iter().filter(|path| valid(path)).collect()
        };

        for path in restore {
            app.open_repo_internal(path, explicit_request, window, cx);
        }

        if !explicit_request
            && let Some(active) = saved_active
            && let Some(index) = app.tabs.iter().position(|tab| tab.read(cx).path == active)
        {
            app.active = index;
        }

        app.showing_dashboard = app.tabs.is_empty();
        app.persist_session(cx);
        app.update_window_title(window, cx);
        if app.showing_dashboard {
            app.focus_handle.focus(window);
        }
        app
    }

    fn active_session(&self) -> Option<Entity<RepoSession>> {
        self.tabs.get(self.active).cloned()
    }

    fn open_repo_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let app = cx.entity();
        let title = cx.tr("リポジトリを開く").to_string();
        window
            .spawn(cx, async move |window| {
                let folder = rfd::AsyncFileDialog::new()
                    .set_title(&title)
                    .pick_folder()
                    .await;
                let Some(folder) = folder else {
                    return;
                };
                let path = folder.path().to_path_buf();
                let _ = app.update_in(window, |app, window, cx| {
                    app.open_repo(path, window, cx);
                });
            })
            .detach();
    }

    fn open_repo(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.open_repo_internal(path, true, window, cx);
    }

    fn open_repo_internal(
        &mut self,
        path: PathBuf,
        record_recent: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let canonical = std::fs::canonicalize(&path).unwrap_or(path);
        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.read(cx).path == canonical)
        {
            self.active = index;
            self.showing_dashboard = false;
            self.showing_settings = false;
            if record_recent {
                let name = self.repo_name(&canonical);
                self.storage.remember(&mut self.state, &canonical, &name);
            }
            self.tabs[index].update(cx, |session, cx| session.focus_history(window, cx));
            self.persist_session(cx);
            self.update_window_title(window, cx);
            cx.notify();
            return;
        }

        if let Err(err) = rascal_core::GitRepo::open(&canonical) {
            if record_recent {
                window.push_notification(
                    Notification::error(if cx.is_japanese() {
                        format!("Gitリポジトリではありません: {err}")
                    } else {
                        format!("Not a Git repository: {err}")
                    }),
                    cx,
                );
            }
            return;
        }

        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let session = cx.new(|cx| RepoSession::new(id, canonical.clone(), window, cx));
        self.tabs.push(session);
        self.active = self.tabs.len() - 1;
        self.showing_dashboard = false;
        self.showing_settings = false;

        if record_recent {
            let name = self.repo_name(&canonical);
            self.storage.remember(&mut self.state, &canonical, &name);
        }

        self.persist_session(cx);
        self.update_window_title(window, cx);
        cx.notify();
    }

    fn repo_name(&self, path: &Path) -> String {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "repository".to_string())
    }

    fn persist_session(&mut self, cx: &App) {
        self.state.open_repos = Some(
            self.tabs
                .iter()
                .map(|tab| tab.read(cx).path.clone())
                .collect(),
        );
        self.state.active_repo = self
            .tabs
            .get(self.active)
            .map(|tab| tab.read(cx).path.clone());
        let _ = self.storage.save(&self.state);
    }

    fn show_dashboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.showing_dashboard = true;
        self.showing_settings = false;
        self.focus_handle.focus(window);
        self.update_window_title(window, cx);
        cx.notify();
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.showing_settings {
            return;
        }
        self.settings_return_dashboard = self.showing_dashboard || self.tabs.is_empty();
        self.settings_scroll.set_offset(Point::default());
        self.prepare_font_selects(window, cx);
        self.showing_settings = true;
        self.showing_dashboard = false;
        self.focus_handle.focus(window);
        self.update_window_title(window, cx);
        cx.notify();
    }

    fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.showing_settings = false;
        self.showing_dashboard = self.settings_return_dashboard || self.tabs.is_empty();
        if self.showing_dashboard {
            self.focus_handle.focus(window);
        } else if let Some(tab) = self.tabs.get(self.active).cloned() {
            tab.update(cx, |session, cx| session.focus_history(window, cx));
        }
        self.update_window_title(window, cx);
        cx.notify();
    }

    fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.showing_settings {
            self.close_settings(window, cx);
        } else {
            self.open_settings(window, cx);
        }
    }

    fn set_language(&mut self, language: Language, window: &mut Window, cx: &mut Context<Self>) {
        if cx.global::<Language>() == &language {
            return;
        }
        cx.set_global(language);
        self.state.language = Some(language.key().to_string());
        let _ = self.storage.save(&self.state);
        install_menus(cx);
        if self.showing_settings {
            self.prepare_font_selects(window, cx);
        }
        for tab in &self.tabs {
            tab.update(cx, |session, cx| session.language_changed(window, cx));
        }
        cx.refresh_windows();
        self.update_window_title(window, cx);
        cx.notify();
    }

    fn set_ui_font(&mut self, font: Option<String>, cx: &mut Context<Self>) {
        self.state.ui_font = font;
        let _ = self.storage.save(&self.state);
        apply_fonts(
            cx,
            self.state.ui_font.as_deref(),
            self.state.mono_font.as_deref(),
        );
        self.refresh_windows_and_sessions(cx);
        cx.notify();
    }

    fn set_mono_font(&mut self, font: Option<String>, cx: &mut Context<Self>) {
        self.state.mono_font = font;
        let _ = self.storage.save(&self.state);
        apply_fonts(
            cx,
            self.state.ui_font.as_deref(),
            self.state.mono_font.as_deref(),
        );
        self.refresh_windows_and_sessions(cx);
        cx.notify();
    }

    fn set_app_theme_choice(
        &mut self,
        choice: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.theme = choice.map(str::to_string);
        let _ = self.storage.save(&self.state);
        apply_theme_preference(choice, window, cx);
        apply_fonts(
            cx,
            self.state.ui_font.as_deref(),
            self.state.mono_font.as_deref(),
        );
        self.refresh_windows_and_sessions(cx);
        cx.notify();
    }

    fn prepare_font_selects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.font_select_subscriptions.clear();
        let fonts = installed_font_names(cx);

        let mut ui_items = Vec::with_capacity(fonts.len() + 1);
        ui_items.push(FontItem::new("", cx.tr("システムフォント")));
        for name in &fonts {
            ui_items.push(FontItem::new(name, name));
        }
        let ui_selected = self.state.ui_font.clone();
        if let Some(selected) = &ui_selected
            && !ui_items.iter().any(|item| item.name == *selected)
        {
            ui_items.push(FontItem::new(selected, selected));
        }
        let ui_index = ui_selected
            .as_ref()
            .and_then(|selected| ui_items.iter().position(|item| item.name == *selected))
            .unwrap_or(0);
        let ui_state = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(ui_items),
                Some(IndexPath::default().row(ui_index)),
                window,
                cx,
            )
            .searchable(true)
        });
        self.font_select_subscriptions.push(cx.subscribe_in(
            &ui_state,
            window,
            |this, _, event: &SelectEvent<SearchableVec<FontItem>>, _window, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    let font = if value.is_empty() {
                        None
                    } else {
                        Some(value.clone())
                    };
                    this.set_ui_font(font, cx);
                }
            },
        ));
        self.ui_font_select = Some(ui_state);

        let default_mono_label = if cx.is_japanese() {
            format!("{DEFAULT_MONO_FONT}（標準）")
        } else {
            format!("{DEFAULT_MONO_FONT} (default)")
        };
        let mut mono_items = Vec::with_capacity(fonts.len() + 1);
        mono_items.push(FontItem::new("", default_mono_label));
        for name in &fonts {
            mono_items.push(FontItem::new(name, name));
        }
        let mono_selected = self.state.mono_font.clone();
        if let Some(selected) = &mono_selected
            && !mono_items.iter().any(|item| item.name == *selected)
        {
            mono_items.push(FontItem::new(selected, selected));
        }
        let mono_index = mono_selected
            .as_ref()
            .and_then(|selected| mono_items.iter().position(|item| item.name == *selected))
            .unwrap_or(0);
        let mono_state = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(mono_items),
                Some(IndexPath::default().row(mono_index)),
                window,
                cx,
            )
            .searchable(true)
        });
        self.font_select_subscriptions.push(cx.subscribe_in(
            &mono_state,
            window,
            |this, _, event: &SelectEvent<SearchableVec<FontItem>>, _window, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    let font = if value.is_empty() {
                        None
                    } else {
                        Some(value.clone())
                    };
                    this.set_mono_font(font, cx);
                }
            },
        ));
        self.mono_font_select = Some(mono_state);
    }

    fn refresh_windows_and_sessions(&mut self, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            tab.update(cx, |_, cx| cx.notify());
        }
        cx.refresh_windows();
    }

    fn close_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        self.tabs.remove(self.active);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        }
        if self.tabs.is_empty() {
            self.showing_dashboard = true;
            self.focus_handle.focus(window);
        }
        self.persist_session(cx);
        self.update_window_title(window, cx);
        cx.notify();
    }

    fn update_window_title(&self, window: &mut Window, cx: &App) {
        let title = if self.showing_settings {
            format!("{} — Rascal", cx.tr("設定"))
        } else if self.tabs.is_empty() {
            "Rascal".to_string()
        } else if self.showing_dashboard {
            cx.tr("ダッシュボード — Rascal").to_string()
        } else {
            self.tabs
                .get(self.active)
                .map(|tab| format!("{} — Rascal", tab.read(cx).tab_title()))
                .unwrap_or_else(|| "Rascal".to_string())
        };
        window.set_window_title(&title);
    }

    fn toggle_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = if Theme::global(cx).is_dark() {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        Theme::change(mode, Some(window), cx);
        apply_theme_tweaks(cx);
        apply_fonts(
            cx,
            self.state.ui_font.as_deref(),
            self.state.mono_font.as_deref(),
        );
        self.storage.set_theme(&mut self.state, mode.name());
        self.refresh_windows_and_sessions(cx);
        cx.notify();
    }

    fn set_diff_theme(&mut self, diff_theme: DiffTheme, cx: &mut Context<Self>) {
        if cx.global::<DiffTheme>() == &diff_theme {
            return;
        }
        cx.set_global(diff_theme);
        self.state.diff_theme = Some(diff_theme.key().to_string());
        let _ = self.storage.save(&self.state);
        for tab in &self.tabs {
            tab.update(cx, |_, cx| cx.notify());
        }
        cx.notify();
    }

    fn show_about(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let about = cx.tr("GPUIで作られた、軽量・読み取り専用のGit履歴ビューア。");
        let read_only =
            cx.tr("履歴・差分・ファイル内容の閲覧だけを行い、リポジトリは変更しません。");
        window.open_dialog(cx, move |dialog, _window, _cx| {
            dialog.title("Rascal").child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_2()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Rascal {}", env!("CARGO_PKG_VERSION"))),
                    )
                    .child(div().text_sm().child(about.clone()))
                    .child(div().text_xs().child(read_only.clone()))
                    .child(
                        div()
                            .text_xs()
                            .child("Built with Rust, gpui and gpui-component (Apache-2.0), libgit2 (GPL-2.0 with linking exception)."),
                    ),
            )
        });
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let root = cx.entity();
        let mut tabs = Vec::new();

        for (index, tab) in self.tabs.iter().enumerate() {
            let title = tab.read(cx).tab_title();
            let tab_path = tab.read(cx).path.clone();
            let active = index == self.active && !self.showing_dashboard;
            let close_root = root.clone();
            let tab_root = root.clone();
            tabs.push(
                div()
                    .id(ElementId::Name(format!("tab-{}", index).into()))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .max_w(px(200.0))
                    .min_w(px(90.0))
                    .rounded(px(5.0))
                    .cursor_pointer()
                    .when(active, |this| {
                        this.bg(theme.tab_active)
                            .text_color(theme.tab_active_foreground)
                    })
                    .when(!active, |this| {
                        this.text_color(theme.muted_foreground)
                            .hover(|this| this.bg(theme.list_hover))
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, window, cx| {
                        tab_root.update(cx, |app, cx| {
                            app.active = index;
                            app.showing_dashboard = false;
                            app.showing_settings = false;
                            if let Some(session) = app.tabs.get(index).cloned() {
                                session.update(cx, |session, cx| session.focus_history(window, cx));
                            }
                            app.persist_session(cx);
                            app.update_window_title(window, cx);
                            cx.notify();
                        });
                    })
                    .child(
                        Icon::new(IconName::Folder)
                            .size(px(13.0))
                            .text_color(theme.muted_foreground),
                    )
                    .child(div().flex_1().text_sm().truncate().child(title))
                    .child(
                        div()
                            .id(ElementId::Name(format!("tab-close-{}", index).into()))
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(16.0))
                            .h(px(16.0))
                            .rounded(px(3.0))
                            .cursor_pointer()
                            .hover(|this| this.bg(theme.list_hover))
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                close_root.update(cx, |app, cx| {
                                    if let Some(position) =
                                        app.tabs.iter().position(|t| t.read(cx).path == tab_path)
                                    {
                                        app.active = position;
                                    }
                                    app.close_active_tab(window, cx);
                                });
                            })
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child("×"),
                            ),
                    )
                    .into_any_element(),
            );
        }

        let open_button = Button::new("open-repo")
            .small()
            .ghost()
            .icon(Icon::new(IconName::FolderOpen).size(px(14.0)))
            .tooltip_with_action(cx.tr("リポジトリを開く"), &OpenRepository, Some(CONTEXT))
            .on_click(cx.listener(|this, _, window, cx| this.open_repo_dialog(window, cx)));

        let refresh_button = Button::new("refresh")
            .small()
            .ghost()
            .icon(Icon::default().path("icons/refresh-cw.svg").size(px(14.0)))
            .tooltip_with_action(cx.tr("更新"), &Refresh, Some(CONTEXT))
            .disabled(self.tabs.is_empty())
            .on_click(cx.listener(|this, _, window, cx| {
                if let Some(session) = this.active_session() {
                    session.update(cx, |session, cx| session.refresh(window, cx));
                }
            }));

        let dashboard_active =
            self.showing_dashboard && !self.showing_settings || self.tabs.is_empty();
        let dashboard_button = Button::new("dashboard")
            .small()
            .ghost()
            .selected(dashboard_active)
            .icon(Icon::new(IconName::LayoutDashboard).size(px(14.0)))
            .tooltip_with_action(cx.tr("ダッシュボード"), &ShowDashboard, Some(CONTEXT))
            .on_click(cx.listener(|this, _, window, cx| this.show_dashboard(window, cx)));

        let settings_button = Button::new("settings")
            .small()
            .ghost()
            .selected(self.showing_settings)
            .icon(Icon::new(IconName::Settings).size(px(14.0)))
            .tooltip_with_action(cx.tr("設定"), &OpenSettings, Some(CONTEXT))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_settings(window, cx)));

        let theme_button = Button::new("theme")
            .small()
            .ghost()
            .icon(
                Icon::new(if theme.is_dark() {
                    IconName::Sun
                } else {
                    IconName::Moon
                })
                .size(px(14.0)),
            )
            .tooltip_with_action(cx.tr("テーマを切り替える"), &ToggleTheme, Some(CONTEXT))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_theme(window, cx)));

        let plus = div()
            .id("new-tab")
            .flex()
            .items_center()
            .justify_center()
            .w(px(24.0))
            .h(px(24.0))
            .rounded(px(4.0))
            .cursor_pointer()
            .hover(|this| this.bg(theme.list_hover))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| this.open_repo_dialog(window, cx)))
            .child(
                Icon::new(IconName::Plus)
                    .size(px(14.0))
                    .text_color(theme.muted_foreground),
            );

        TitleBar::new()
            .child(
                div()
                    .w_full()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pr_2()
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
                    .child(dashboard_button)
                    .child(
                        div()
                            .id("repo-tabs")
                            .flex()
                            .items_center()
                            .gap_1()
                            .flex_shrink_0()
                            .max_w(px(760.0))
                            .min_w_0()
                            .overflow_x_scroll()
                            .children(tabs),
                    )
                    .child(plus)
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(open_button)
                            .child(refresh_button)
                            .child(settings_button)
                            .child(theme_button),
                    ),
            )
            .into_any_element()
    }

    fn render_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let entity = cx.entity();
        let language = cx.language();

        let render_language_choices = |entity: Entity<RascalApp>| {
            let mut choices = Vec::new();
            for option in [Language::Japanese, Language::English] {
                let entity = entity.clone();
                choices.push(settings_choice(
                    format!("language-{}", option.key()),
                    option.label().into(),
                    language == option,
                    theme,
                    move |window, cx| {
                        entity.update(cx, |this, cx| this.set_language(option, window, cx));
                    },
                ));
            }
            choices
        };

        let render_app_theme_choices = |entity: Entity<RascalApp>| {
            let mut choices = Vec::new();
            for (label, value) in [
                ("システム", None),
                ("ライト", Some("light")),
                ("ダーク", Some("dark")),
            ] {
                let entity = entity.clone();
                let label = cx.tr(label);
                choices.push(settings_choice(
                    format!("app-theme-{label}"),
                    label,
                    self.state.theme.as_deref() == value,
                    theme,
                    move |window, cx| {
                        entity.update(cx, |this, cx| this.set_app_theme_choice(value, window, cx));
                    },
                ));
            }
            choices
        };

        let current_diff = *cx.global::<DiffTheme>();
        let mut diff_cards = Vec::new();
        for option in [
            DiffTheme::Standard,
            DiffTheme::HighContrast,
            DiffTheme::Colorblind,
        ] {
            let entity = entity.clone();
            diff_cards.push(diff_theme_card(
                option,
                current_diff == option,
                theme,
                entity,
                cx.tr(option.label()),
            ));
        }

        let ui_font_preview = div()
            .flex()
            .flex_col()
            .gap_2()
            .rounded(px(6.0))
            .bg(theme.muted.opacity(0.35))
            .border_1()
            .border_color(theme.border)
            .p_4()
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(cx.tr("プレビュー")),
            )
            .child(
                div()
                    .text_sm()
                    .font_family(theme.font_family.clone())
                    .child("The quick brown fox jumps over the lazy dog. 0123456789"),
            )
            .child(
                div()
                    .text_sm()
                    .font_family(theme.font_family.clone())
                    .child(cx.tr("コミット履歴・ブランチ・差分プレビュー")),
            )
            .child(
                div()
                    .mt_2()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(theme.mono_font_size)
                    .child("+ added line  /  - removed line  /  @@ hunk @@"),
            );

        let header = div()
            .w(px(820.0))
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::BOLD)
                            .child(cx.tr("設定")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(cx.tr("設定は自動的に保存されます。")),
                    ),
            )
            .child(
                Button::new("settings-close")
                    .ghost()
                    .label(cx.tr("閉じる"))
                    .on_click(cx.listener(|this, _, window, cx| this.close_settings(window, cx))),
            );

        div()
            .id("settings-scroll")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.settings_scroll)
            .flex()
            .flex_col()
            .items_center()
            .gap_6()
            .px_8()
            .pt(px(48.0))
            .pb(px(48.0))
            .bg(theme.background)
            .child(header)
            .child(settings_section(
                cx.tr("言語"),
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .children(render_language_choices(entity.clone()))
                    .into_any_element(),
                theme,
            ))
            .child(settings_section(
                cx.tr("アプリテーマ"),
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .children(render_app_theme_choices(entity.clone()))
                    .into_any_element(),
                theme,
            ))
            .child(settings_section(
                cx.tr("差分テーマ"),
                div()
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .children(diff_cards)
                    .into_any_element(),
                theme,
            ))
            .child(settings_section(
                cx.tr("フォント"),
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(font_select_row(
                        cx.tr("UIフォント"),
                        self.ui_font_select.as_ref(),
                        theme,
                        cx.tr("フォントを検索"),
                    ))
                    .child(font_select_row(
                        cx.tr("コード・差分フォント"),
                        self.mono_font_select.as_ref(),
                        theme,
                        cx.tr("フォントを検索"),
                    ))
                    .child(ui_font_preview)
                    .into_any_element(),
                theme,
            ))
            .child(
                div()
                    .w(px(820.0))
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("Rascal {} · GPUI", env!("CARGO_PKG_VERSION"))),
            )
            .into_any_element()
    }

    fn render_dashboard(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let root = cx.entity();
        let open_paths: Vec<PathBuf> = self
            .tabs
            .iter()
            .map(|tab| tab.read(cx).path.clone())
            .collect();
        let mut recent = Vec::new();

        for entry in &self.state.recent {
            let exists = entry.path.exists();
            let is_open = open_paths.iter().any(|path| path == &entry.path);
            let path = entry.path.clone();
            let root = root.clone();
            let last_opened = format_recent_time(entry.last_opened);

            recent.push(
                div()
                    .id(ElementId::Name(
                        format!("recent-{}", entry.path.display()).into(),
                    ))
                    .w(px(720.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .px_4()
                    .py_3()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(theme.border)
                    .cursor_pointer()
                    .when(exists, |this| this.hover(|this| this.bg(theme.list_hover)))
                    .on_click(move |_, window, cx| {
                        if !path.exists() {
                            window.push_notification(
                                Notification::error(cx.tr("リポジトリが見つかりません")),
                                cx,
                            );
                            return;
                        }
                        let path = path.clone();
                        root.update(cx, |app, cx| app.open_repo(path, window, cx));
                    })
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .flex_col()
                            .gap_1()
                            .min_w_0()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::MEDIUM)
                                            .truncate()
                                            .child(entry.name.clone()),
                                    )
                                    .when(is_open, |this| {
                                        this.child(
                                            div()
                                                .px_1()
                                                .rounded(px(3.0))
                                                .bg(theme.primary.opacity(0.16))
                                                .text_xs()
                                                .text_color(theme.primary)
                                                .child(cx.tr("開いています")),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .truncate()
                                    .child(entry.path.to_string_lossy().into_owned()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_4()
                            .flex_shrink_0()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(last_opened),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(if exists {
                                        theme.muted_foreground
                                    } else {
                                        theme.danger
                                    })
                                    .child(if exists {
                                        cx.tr("開く")
                                    } else {
                                        cx.tr("見つかりません")
                                    }),
                            ),
                    )
                    .into_any_element(),
            );
        }

        let open_note = if self.tabs.is_empty() {
            cx.tr("軽量・高速なGit履歴ビューア")
        } else if cx.is_japanese() {
            format!("{} 個のリポジトリを開いています", self.tabs.len()).into()
        } else {
            format!(
                "{} {}",
                self.tabs.len(),
                if self.tabs.len() == 1 {
                    "repository open"
                } else {
                    "repositories open"
                }
            )
            .into()
        };

        div()
            .id("dashboard-scroll")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .items_center()
            .gap_6()
            .px_8()
            .pt(px(64.0))
            .pb(px(48.0))
            .bg(theme.background)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_3xl()
                            .font_weight(FontWeight::BOLD)
                            .child("Rascal"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(open_note),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("dashboard-open")
                                    .primary()
                                    .icon(Icon::new(IconName::FolderOpen).size(px(14.0)))
                                    .label(cx.tr("リポジトリを開く"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_repo_dialog(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("dashboard-settings")
                                    .ghost()
                                    .icon(Icon::new(IconName::Settings).size(px(14.0)))
                                    .label(cx.tr("設定"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_settings(window, cx)
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .w(px(720.0))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme.muted_foreground)
                                    .child(cx.tr("最近のリポジトリ")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(if cx.is_japanese() {
                                    format!("{} 件", self.state.recent.len())
                                } else if self.state.recent.len() == 1 {
                                    "1 repository".to_string()
                                } else {
                                    format!("{} repositories", self.state.recent.len())
                                }),
                            ),
                    )
                    .when(self.state.recent.is_empty(), |this| {
                        this.child(
                            div()
                                .w_full()
                                .rounded(px(6.0))
                                .border_1()
                                .border_color(theme.border)
                                .px_4()
                                .py_6()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(cx.tr("まだ履歴がありません。リポジトリを開くと、ここに表示されます。")),
                        )
                    })
                    .children(recent),
            )
            .into_any_element()
    }
}

const DEFAULT_UI_FONT: &str = ".SystemUIFont";
#[cfg(target_os = "macos")]
const DEFAULT_MONO_FONT: &str = "Menlo";
#[cfg(not(target_os = "macos"))]
const DEFAULT_MONO_FONT: &str = "DejaVu Sans Mono";

fn apply_fonts(cx: &mut App, ui_font: Option<&str>, mono_font: Option<&str>) {
    let theme = Theme::global_mut(cx);
    theme.font_family = ui_font.unwrap_or(DEFAULT_UI_FONT).to_string().into();
    theme.mono_font_family = mono_font.unwrap_or(DEFAULT_MONO_FONT).to_string().into();
}

fn apply_theme_preference(choice: Option<&str>, window: &mut Window, cx: &mut App) {
    match choice {
        Some("dark") => Theme::change(ThemeMode::Dark, Some(window), cx),
        Some("light") => Theme::change(ThemeMode::Light, Some(window), cx),
        _ => Theme::sync_system_appearance(Some(window), cx),
    }
    apply_theme_tweaks(cx);
}

fn font_select_row(
    title: SharedString,
    select: Option<&Entity<FontSelect>>,
    theme: &gpui_component::theme::Theme,
    search_placeholder: SharedString,
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
        .child(div().w(px(420.0)).child(if let Some(select) = select {
            Select::new(select)
                .search_placeholder(search_placeholder)
                .w_full()
                .into_any_element()
        } else {
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child("…")
                .into_any_element()
        }))
        .into_any_element()
}

fn settings_section(
    title: SharedString,
    body: AnyElement,
    theme: &gpui_component::theme::Theme,
) -> AnyElement {
    div()
        .w(px(820.0))
        .flex()
        .flex_col()
        .gap_3()
        .pb_5()
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.muted_foreground)
                .child(title),
        )
        .child(body)
        .into_any_element()
}

fn settings_choice(
    id: String,
    label: SharedString,
    selected: bool,
    theme: &gpui_component::theme::Theme,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(ElementId::Name(id.into()))
        .px_3()
        .py_2()
        .rounded(px(5.0))
        .border_1()
        .border_color(if selected {
            theme.primary
        } else {
            theme.border
        })
        .when(selected, |this| {
            this.bg(theme.primary.opacity(0.10))
                .text_color(theme.foreground)
        })
        .when(!selected, |this| {
            this.text_color(theme.muted_foreground)
                .bg(theme.background)
                .hover(|this| this.bg(theme.list_hover))
        })
        .cursor_pointer()
        .text_sm()
        .on_click(move |_, window, cx| on_click(window, cx))
        .child(label)
        .into_any_element()
}

fn diff_theme_card(
    diff_theme: DiffTheme,
    selected: bool,
    theme: &gpui_component::theme::Theme,
    entity: Entity<RascalApp>,
    label: SharedString,
) -> AnyElement {
    let palette = crate::diff_theme::palette(theme, diff_theme);
    let mono = theme.mono_font_family.clone();
    let preview = div()
        .w_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .rounded(px(4.0))
        .border_1()
        .border_color(theme.border)
        .child(
            div()
                .w_full()
                .px_2()
                .py_0p5()
                .bg(palette.hunk_bg)
                .text_color(palette.hunk_fg)
                .font_family(mono.clone())
                .text_size(px(10.0))
                .child("@@ -1,3 +1,3 @@"),
        )
        .child(
            div()
                .w_full()
                .px_2()
                .py_0p5()
                .text_color(palette.context_fg)
                .font_family(mono.clone())
                .text_size(px(10.0))
                .child(" fn main() {"),
        )
        .child(
            div()
                .w_full()
                .px_2()
                .py_0p5()
                .bg(palette.removed_bg)
                .text_color(palette.removed_fg)
                .font_family(mono.clone())
                .text_size(px(10.0))
                .child("- old_value"),
        )
        .child(
            div()
                .w_full()
                .px_2()
                .py_0p5()
                .bg(palette.added_bg)
                .text_color(palette.added_fg)
                .font_family(mono)
                .text_size(px(10.0))
                .child("+ new_value"),
        );

    div()
        .id(ElementId::Name(
            format!("diff-theme-{}", diff_theme.key()).into(),
        ))
        .w(px(252.0))
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .rounded(px(6.0))
        .border_1()
        .border_color(if selected {
            theme.primary
        } else {
            theme.border
        })
        .when(selected, |this| this.bg(theme.primary.opacity(0.06)))
        .cursor_pointer()
        .hover(|this| this.bg(theme.list_hover))
        .on_click(move |_, _, cx| {
            entity.update(cx, |this, cx| this.set_diff_theme(diff_theme, cx));
        })
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(label))
                .when(selected, |this| {
                    this.child(div().text_xs().text_color(theme.primary).child("✓"))
                }),
        )
        .child(preview)
        .into_any_element()
}

fn installed_font_names(cx: &App) -> Vec<String> {
    let mut fonts = cx.text_system().all_font_names();
    fonts.retain(|name| !name.is_empty() && !name.starts_with('.'));
    fonts.sort_by_key(|name| name.to_lowercase());
    fonts.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    fonts
}

fn install_menus(cx: &mut App) {
    cx.set_menus(vec![
        Menu {
            name: "Rascal".into(),
            items: vec![
                MenuItem::action(cx.tr("Rascalについて"), About),
                MenuItem::action(cx.tr("設定…"), OpenSettings),
                MenuItem::separator(),
                MenuItem::action(cx.tr("Rascalを終了"), Quit),
            ],
        },
        Menu {
            name: cx.tr_key("ファイル", "menu.file"),
            items: vec![
                MenuItem::action(cx.tr("リポジトリを開く"), OpenRepository),
                MenuItem::action(cx.tr("更新"), Refresh),
                MenuItem::separator(),
                MenuItem::action(cx.tr("タブを閉じる"), CloseTab),
                MenuItem::action(cx.tr("ウィンドウを閉じる"), CloseWindow),
            ],
        },
        Menu {
            name: cx.tr("表示"),
            items: vec![
                MenuItem::action(cx.tr("ダッシュボード"), ShowDashboard),
                MenuItem::separator(),
                MenuItem::action(cx.tr("コミットを絞り込む"), FocusFilter),
                MenuItem::separator(),
                MenuItem::action(cx.tr("アプリテーマを切り替える"), ToggleTheme),
            ],
        },
    ]);
}

fn format_recent_time(timestamp: i64) -> String {
    if timestamp <= 0 {
        return String::new();
    }
    chrono::Local
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|time| time.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

fn apply_theme_tweaks(cx: &mut App) {
    let theme = Theme::global_mut(cx);
    if !theme.is_dark() {
        return;
    }

    // The stock dark palette is intentionally low-contrast. Lift the text,
    // borders and selection surfaces a little for a long reading session.
    theme.colors.foreground = hsla(0.0, 0.0, 0.98, 1.0);
    theme.colors.muted_foreground = hsla(0.0, 0.0, 0.72, 1.0);
    theme.colors.border = hsla(0.0, 0.0, 0.27, 1.0);
    theme.colors.sidebar_border = theme.colors.border;
    theme.colors.title_bar_border = theme.colors.border;
    theme.colors.input = theme.colors.border;
    theme.colors.table_row_border = hsla(0.0, 0.0, 0.25, 0.85);
    theme.colors.list_hover = hsla(0.0, 0.0, 0.18, 1.0);
    theme.colors.list_active = hsla(0.60, 0.60, 0.35, 0.45);
    theme.colors.muted = hsla(0.0, 0.0, 0.20, 1.0);
    theme.colors.secondary = hsla(0.0, 0.0, 0.12, 1.0);
    theme.colors.secondary_hover = hsla(0.0, 0.0, 0.18, 1.0);
    theme.colors.secondary_active = hsla(0.0, 0.0, 0.22, 1.0);
    theme.colors.sidebar_accent = hsla(0.0, 0.0, 0.17, 1.0);
    theme.colors.scrollbar_thumb = hsla(0.0, 0.0, 0.45, 1.0);
    theme.colors.scrollbar_thumb_hover = hsla(0.0, 0.0, 0.58, 1.0);
}

impl Focusable for RascalApp {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RascalApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if (self.showing_dashboard || self.showing_settings) && window.focused(cx).is_none() {
            self.focus_handle.focus(window);
        }
        let title_bar = self.render_title_bar(cx);
        let content = if self.showing_settings {
            self.render_settings(cx)
        } else if self.showing_dashboard || self.tabs.is_empty() {
            self.render_dashboard(cx)
        } else {
            self.tabs[self.active].clone().into_any_element()
        };

        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .on_action(cx.listener(|this, _: &OpenRepository, window, cx| {
                this.open_repo_dialog(window, cx);
            }))
            .on_action(cx.listener(|this, _: &Refresh, window, cx| {
                if let Some(session) = this.active_session() {
                    session.update(cx, |session, cx| session.refresh(window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                this.close_active_tab(window, cx);
            }))
            .on_action(cx.listener(|_this, _: &CloseWindow, window, _cx| {
                window.remove_window();
            }))
            .on_action(cx.listener(|this, _: &ToggleTheme, window, cx| {
                this.toggle_theme(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowDashboard, window, cx| {
                this.show_dashboard(window, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                this.open_settings(window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusFilter, window, cx| {
                if let Some(session) = this.active_session() {
                    session.update(cx, |session, cx| session.focus_filter(window, cx));
                }
            }))
            .on_action(cx.listener(|_this, _: &Quit, _window, cx| {
                cx.quit();
            }))
            .on_action(cx.listener(|this, _: &About, window, cx| {
                this.show_about(window, cx);
            }))
            .child(title_bar)
            .child(div().flex_1().min_h_0().flex().flex_col().child(content))
    }
}
