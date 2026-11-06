//! Standalone settings window.

use gpui::prelude::*;
use gpui::*;

use crate::app::{CONTEXT, CloseTab, CloseWindow, OpenSettings, Quit, RascalApp};
use crate::i18n::LanguageExt;

pub struct SettingsView {
    root: Entity<RascalApp>,
    focus_handle: FocusHandle,
}

impl SettingsView {
    pub fn new(root: Entity<RascalApp>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window);
        cx.defer_in(window, move |view, window, cx| {
            view.root
                .update(cx, |app, cx| app.prepare_settings_window(window, cx));
            cx.notify();
        });
        Self { root, focus_handle }
    }
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = format!("{} — Rascal", cx.tr("設定"));
        window.set_window_title(&title);
        let content = self.root.read(cx).render_settings(self.root.clone(), cx);
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .on_action(cx.listener(|_, _: &CloseTab, window, _| {
                window.remove_window();
            }))
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| {
                window.remove_window();
            }))
            .on_action(cx.listener(|_, _: &OpenSettings, window, _| {
                window.activate_window();
            }))
            .on_action(cx.listener(|_, _: &Quit, _window, cx| {
                cx.quit();
            }))
            .child(content)
            .into_any_element()
    }
}
