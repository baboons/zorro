//! Zorro — a native macOS Git merge-conflict resolver built on GPUI.
//!
//! The binary discovers the conflicted files in the repository at the current
//! directory (or a path passed as the first argument), then opens a window that
//! drives [`zorro_core`]'s conflict engine.

mod app;
mod editor;
mod theme;

use std::path::PathBuf;

use gpui::{
    App, AppContext, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};
use gpui_platform::application;

use zorro_core::git::Repository;
use zorro_core::session::{MergeSession, Workflow};

use app::{
    AcceptBoth, AcceptCurrent, AcceptIncoming, NextConflict, NextFile, PrevConflict, PrevFile,
    SaveFile, Zorro,
};
use editor::{
    Backspace, Copy, Cut, DeleteForward, InsertNewline, MoveDown, MoveEnd, MoveHome, MoveLeft,
    MoveRight, MoveUp, Paste, SelectAll, SelectDown, SelectLeft, SelectRight, SelectUp,
};

fn main() {
    let (session, repo_root, offer_diff3) = load_session();

    application().run(move |cx: &mut App| {
        // Keyboard-first: every primary action is bound. No key context, so the
        // bindings fire whenever the (single) root view holds focus.
        cx.bind_keys([
            KeyBinding::new("f7", NextConflict, None),
            KeyBinding::new("shift-f7", PrevConflict, None),
            KeyBinding::new("alt-left", AcceptCurrent, None),
            KeyBinding::new("alt-right", AcceptIncoming, None),
            KeyBinding::new("alt-down", AcceptBoth, None),
            KeyBinding::new("cmd-down", NextFile, None),
            KeyBinding::new("cmd-up", PrevFile, None),
            KeyBinding::new("cmd-enter", SaveFile, None),
            // Result editor — only active while a CodeEditor holds focus.
            KeyBinding::new("left", MoveLeft, Some("CodeEditor")),
            KeyBinding::new("right", MoveRight, Some("CodeEditor")),
            KeyBinding::new("up", MoveUp, Some("CodeEditor")),
            KeyBinding::new("down", MoveDown, Some("CodeEditor")),
            KeyBinding::new("home", MoveHome, Some("CodeEditor")),
            KeyBinding::new("end", MoveEnd, Some("CodeEditor")),
            KeyBinding::new("shift-left", SelectLeft, Some("CodeEditor")),
            KeyBinding::new("shift-right", SelectRight, Some("CodeEditor")),
            KeyBinding::new("shift-up", SelectUp, Some("CodeEditor")),
            KeyBinding::new("shift-down", SelectDown, Some("CodeEditor")),
            KeyBinding::new("cmd-a", SelectAll, Some("CodeEditor")),
            KeyBinding::new("cmd-c", Copy, Some("CodeEditor")),
            KeyBinding::new("cmd-x", Cut, Some("CodeEditor")),
            KeyBinding::new("cmd-v", Paste, Some("CodeEditor")),
            KeyBinding::new("backspace", Backspace, Some("CodeEditor")),
            KeyBinding::new("delete", DeleteForward, Some("CodeEditor")),
            KeyBinding::new("enter", InsertNewline, Some("CodeEditor")),
        ]);

        let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
        let opened = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Zorro".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Zorro::new(session, repo_root, offer_diff3, window, cx)),
        );

        if let Err(err) = opened {
            eprintln!("zorro: failed to open window: {err}");
            cx.quit();
            return;
        }
        cx.activate(true);
    });
}

/// Discover the repository and load its conflicted files. Falls back to an empty
/// session (which renders a friendly empty state) when there is no repo or no
/// conflicts. The third value is whether to offer switching to diff3 markers
/// (true when the repo isn't using diff3/zdiff3 and no conflict carries a base).
fn load_session() -> (MergeSession, Option<PathBuf>, bool) {
    let start = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));

    match Repository::discover(&start) {
        Ok(repo) => {
            let root = repo.root().to_path_buf();
            let session = repo
                .load_session()
                .unwrap_or_else(|_| MergeSession::new(Workflow::Unknown, Vec::new()));
            let style = repo.conflict_style();
            let already_diff3 = matches!(style.as_deref(), Some("diff3") | Some("zdiff3"));
            let has_base = session
                .files
                .iter()
                .filter_map(|f| f.document.as_ref())
                .any(|d| d.conflicts().any(|c| c.has_base()));
            let offer_diff3 = !already_diff3 && !has_base && session.total_files() > 0;
            (session, Some(root), offer_diff3)
        }
        Err(_) => (MergeSession::new(Workflow::Unknown, Vec::new()), None, false),
    }
}
