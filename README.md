# Rascal

<p align="center">
  <img src="docs/icon.png" width="160" alt="Rascal app icon">
</p>

<p align="center">A lightweight, read-only Git history viewer built with GPUI.</p>

Rascal is a Git GUI for reading history and reviewing changes. It never stages,
commits, checks out, fetches, pushes, merges, or cherry-picks.

## Highlights

- **Read-only by design** — no write operations, no accidental repository changes.
- **Native and lightweight** — GPUI, no WebView, no network access, and no file watching.
- **Responsive on large repositories** — paged history, lazy trees, and a dedicated
  worktree-status worker; status scans never block history or diff interactions.

## Screenshots

| Main window | Changes tree | Dashboard |
|:---:|:---:|:---:|
| <a href="docs/screenshot.png"><img src="docs/screenshot.png" width="100%" alt="Main window"></a> | <a href="docs/changes-tree.png"><img src="docs/changes-tree.png" width="100%" alt="Changes tree"></a> | <a href="docs/dashboard.png"><img src="docs/dashboard.png" width="100%" alt="Dashboard"></a> |
| **Image diff** | **Settings** | **File history** |
| <a href="docs/image-diff.png"><img src="docs/image-diff.png" width="100%" alt="Image diff with slider comparison"></a> | <a href="docs/settings.png"><img src="docs/settings.png" width="100%" alt="Settings window"></a> | <a href="docs/file-history.png"><img src="docs/file-history.png" width="100%" alt="File history window"></a> |

## Features

- Commit graph and paged commit history
- Unified and side-by-side diff views with hunk headers and line numbers
- Rename, binary, and submodule handling
- Standard, high-contrast, and colorblind-friendly diff palettes
- Image preview and image diffs
  - Side-by-side comparison
  - Draggable slider comparison
  - Difference highlighting
- File tree at any commit
- Working-tree changes grouped by staged, unstaged, and untracked
- List and directory-tree views for changed files
- Per-file history window with its own diff pane
- Multiple repository tabs, dashboard, and session restore
- Settings window with Japanese and English UI
- App theme, diff theme, and font selection
- CLI launcher installation from the settings window
- File context menus and keyboard shortcuts
- Always-visible scrollbars in scrollable panes

## Requirements

- macOS (Apple Silicon is the primary target)
- Rust 1.92 or later
- Xcode Command Line Tools
- CMake (for the vendored libgit2 build)

GPUI is built with its `runtime_shaders` feature, so the Metal Toolchain is not
required. The first launch may still take a moment while Metal shaders are
prepared; subsequent launches are faster.

## Build and Run

### macOS app bundle (recommended)

```sh
./scripts/bundle-macos.sh
open target/release/Rascal.app
```

The bundle embeds `Rascal.icns` so the icon appears in Finder and the Dock.
To open a repository on a cold launch:

```sh
open target/release/Rascal.app --args /path/to/repository
```

To open repositories in an already-running instance, install the CLI command
from **Settings → Command line** and use `rascal /path/to/repository`.

### Development

Icons only apply to the `.app` bundle; `cargo run` is for development.

```sh
cargo run --release -p rascal-app -- /path/to/repository
```

Open multiple repositories as tabs:

```sh
cargo run --release -p rascal-app -- /path/to/repo-a /path/to/repo-b
```

Without arguments, Rascal restores the tabs that were open at the previous exit.
If there are no saved tabs, it shows the dashboard with recent repositories.
Open the dashboard at any time from the title bar or with `⌘⇧D`.

```sh
cargo run --release -p rascal-app
```

CLI help:

```text
rascal [repository ...]
rascal --version
rascal --help
```

## CLI

Install the `rascal` command from **Settings → Command line**.

- Install location: `~/.local/bin/rascal`
- No sudo and no automatic changes to shell configuration
- If `~/.local/bin` is not in `PATH`, the settings window shows a setup example

```sh
rascal
rascal /path/to/repository
```

Installation is available only when Rascal is running from its `.app` bundle.
If the app moves, reinstall the command or rely on the bundle identifier fallback.
When Rascal is already running, `rascal /path/to/repository` opens the repository
in a tab in the existing window. Relative paths, paths containing spaces, and
multiple repositories are supported.

Launchers created by older versions are upgraded automatically when Rascal starts.

## Usage

| Action | Shortcut |
| --- | --- |
| Open repository | `⌘O` |
| Refresh | `⌘R` |
| Dashboard | `⌘⇧D` |
| Settings | `⌘,` |
| Filter commits | `⌘F` |
| Close tab | `⌘W` |
| Close window | `⌘⇧W` |
| Toggle theme | `⌘⇧T` |
| Navigate history | `↑` / `↓` |
| Quit | `⌘Q` |

- Select a recent repository on the dashboard. Open tabs are saved at exit and
  restored on the next launch.
- Click **HEAD** or a branch in the left sidebar to show its history. Checkout is
  never performed.
- Click the **Uncommitted changes** row to view the working-tree diff.
- Use **Changes / Files** to switch between a commit's changed files and the
  commit's file tree. In **Changes**, switch between list and directory-tree views.
- Use **Copy SHA** to copy a commit hash.
- Image files can be previewed directly. Committed and working-tree image changes
  can be compared in three modes: **Side by side**, **Slider**, and **Difference**.
  The comparison mode is shared across the app. Zoom and pan are not supported.
- `⌘W` closes the current tab. Closing the last tab shows the dashboard.
  With the dashboard visible or no tabs open, `⌘W` closes the window.
- `⌘⇧W` closes the window regardless of open tabs. Closing the last window quits
  the app.
- In the settings and file-history windows, `⌘W` and `⌘⇧W` close that window only.
- Use the diff header to switch between unified and side-by-side views. The choice
  is shared across the app and applies to both the main viewer and file history.
- Right-click a file in the file list or file tree to open it, open it with a
  chosen application, show it in Finder, copy its path, or open its file history.
- Merge commits are diffed against their first parent.
- Open the settings window from the title-bar gear icon or **Rascal → Settings…**
  (`⌘,`). It can also be opened from the dashboard.
- The settings window is a single window. Opening it again brings the existing
  window to the front. It configures the UI language (Japanese / English), app
  theme, diff theme, UI font, and code/diff font. Changes are saved automatically.
- Font lists are loaded when the settings window opens, not during app startup.

## Architecture

```text
crates/rascal-core/   Read-only Git engine (independent of GPUI)
  repo.rs             Repository metadata, branches, status
  history.rs          Revwalk, ref decoration, paging
  diff.rs             Commit details and unified diffs
  worktree.rs         Staged / unstaged / untracked changes
  tree.rs             Lazy trees and blob contents
  graph.rs            Commit-graph lane layout (pure functions)

crates/rascal-app/    GPUI UI and application binary
  app.rs              Title bar, tabs, dashboard, menus
  session.rs          Per-repository state, rendering, file context menu
  settings.rs         Settings window
  file_history.rs     File history window
  git_worker.rs       Per-tab Git workers
  storage.rs          Recent repositories, tab session, theme state
  ui/                 Graph, diff, image, and scrollbar helpers

scripts/
  bundle-macos.sh     Builds the .app bundle (icon, Info.plist, ad-hoc signing)
```

- Each tab has an interactive Git worker and a separate worktree/status worker.
  The UI thread never touches `git2::Repository`; results are sent as plain data
  over `async-channel`.
- The first tab paint uses lightweight metadata. Branches and worktree counts are
  loaded afterward, and the full worktree snapshot is fetched only when needed.
- History is paged at 300 commits per page. Diffs, file trees, and file contents
  are loaded lazily when selected.
- Large inputs are capped: 20,000 diff lines, 2 MB or 50,000 lines per text blob,
  and 20 MiB or 8,192 px per image. The UI indicates when content is truncated.
- State is stored at `~/Library/Application Support/Rascal/state.json`: up to 20
  recent repositories, open tabs, active tab, app theme, diff theme, language,
  UI/code fonts, diff layout, and image comparison mode.

## Testing

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

`rascal-core` tests create repositories with git2 under `tempfile` and cover
history, refs, diffs, renames, binary files, worktrees, trees, and blobs.

## Limitations

- Rascal is read-only; it never modifies a repository.
- Merge commits show only the first-parent diff (no combined diff).
- Syntax highlighting is not supported.
- Image preview and comparison are supported, but zoom, pan, and thumbnail lists
  are not.
- The image slider is a visual comparison tool; the pixel-difference threshold
  is fixed.
- Images are limited to 20 MiB and 8,192 px. Difference computation is limited
  to 16 million pixels.
- Git LFS pointers are not fetched or rendered as images.
- File watching is not used. Press `⌘R` after external changes; window activation
  refreshes metadata only.
- The UI supports Japanese and English.

## License

MIT. See the `Cargo.toml` metadata. Dependency licenses follow their respective
projects.
