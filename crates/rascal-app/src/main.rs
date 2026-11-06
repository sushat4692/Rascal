use std::path::PathBuf;

use gpui::*;
use gpui_component::Root;
use gpui_component::TitleBar;

mod app;
mod assets;
mod cli;
mod diff_theme;
mod file_history;
mod git_worker;
mod i18n;
mod session;
mod settings;
mod storage;
mod ui;

use app::RascalApp;

fn main() {
    let mut paths = Vec::new();
    for arg in std::env::args_os().skip(1) {
        if arg == "--version" || arg == "-V" {
            println!("rascal {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        if arg == "--help" || arg == "-h" {
            println!("rascal {}", env!("CARGO_PKG_VERSION"));
            println!("Usage: rascal [repository ...]");
            println!();
            println!("Opens one or more Git repositories as tabs. Without arguments,");
            println!("reopens the last repository and otherwise shows the welcome screen.");
            return;
        }
        paths.push(PathBuf::from(arg));
    }

    let initial = paths;
    let app = Application::new().with_assets(assets::Assets);

    app.run(move |cx| {
        gpui_component::init(cx);
        RascalApp::init(cx);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let initial = initial.clone();
        cx.spawn(async move |cx| {
            let options = WindowOptions {
                titlebar: Some(TitleBar::title_bar_options()),
                window_min_size: Some(size(px(960.0), px(600.0))),
                focus: true,
                ..Default::default()
            };
            cx.open_window(options, |window, cx| {
                let view = cx.new(|cx| RascalApp::new(initial.clone(), window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })?;

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}
