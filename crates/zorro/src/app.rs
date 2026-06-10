//! The root GPUI view: a sidebar of conflicted files plus a JetBrains-style
//! full-height three-column merge for the active file — **Local · Result ·
//! Incoming**. The Result column shows the whole file (unchanged context plus
//! every conflict), with each conflict's merged text a live, editable
//! [`CodeEditor`]. Gutters between the columns carry per-hunk accept (`»`/`«`)
//! and ignore (`✕`) actions; the toolbar adds global Accept Left / Accept Right.
//!
//! Columns are laid out as five aligned vertical stacks (Local · gutter ·
//! Result · gutter · Incoming). Within a conflict every column is padded to the
//! tallest side so all five stay row-aligned and read as continuous columns.
//!
//! All conflict logic lives in [`zorro_core`]; the result editors mirror each
//! conflict's text into the tested [`MergeDocument`] so save/render is correct.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    actions, div, prelude::*, px, AnyElement, App, Context, Div, ElementId, Entity, FocusHandle,
    Render, Rgba, SharedString, Stateful, Subscription, Window,
};

use zorro_core::ai::{AiConflict, AiProvider, CliProvider};
use zorro_core::conflict::{MergeDocument, Resolution, Section};
use zorro_core::diff::line_diff_flags;
use zorro_core::git::Repository;
use zorro_core::session::{FileStatus, MergeSession};
use zorro_core::syntax::{Highlighter, Language};
use zorro_core::validate::{self, SyntaxIssue};

/// What an AI request is doing / produced.
enum AiState {
    Idle,
    /// Explaining the focused conflict.
    Explaining,
    /// Resolving every conflict in the file; `done`/`total` drives progress.
    Resolving { done: usize, total: usize },
    Explanation { text: String },
    Error { message: String },
}

use crate::editor::{CodeEditor, EditorEvent};
use crate::theme::Theme;

/// Pixel height of a single text row; shared by editor, code cells, gutters, and
/// line numbers so all five columns line up.
const ROW: f32 = 20.0;
const GUTTER_W: f32 = 52.0;
const NUM_W: f32 = 44.0;
/// Width of the Result column's action sub-gutter (holds the per-hunk undo).
const ACT_W: f32 = 22.0;

actions!(
    zorro,
    [
        NextFile,
        PrevFile,
        NextConflict,
        PrevConflict,
        AcceptCurrent,
        AcceptIncoming,
        AcceptBoth,
        AcceptAllLeft,
        AcceptAllRight,
        SaveFile,
    ]
);

pub struct Zorro {
    focus_handle: FocusHandle,
    session: MergeSession,
    repo_root: Option<PathBuf>,
    file_idx: usize,
    /// Section index of the focused conflict (for highlight + keyboard apply).
    active_section: usize,
    theme: Theme,
    status: Option<SharedString>,
    /// Whether to show the "switch to diff3" banner.
    offer_diff3: bool,
    /// The AI backend (Claude Code by default).
    provider: Arc<dyn AiProvider>,
    /// Current AI request state for the focused conflict.
    ai: AiState,
    /// One result editor per section of the active file (rebuilt on file switch).
    editors: Vec<Entity<CodeEditor>>,
    _subscriptions: Vec<Subscription>,
}

impl Zorro {
    pub fn new(
        session: MergeSession,
        repo_root: Option<PathBuf>,
        offer_diff3: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);

        let file_idx = session.first_unresolved().unwrap_or(0);
        let theme = Theme::dark();

        let texts = initial_texts(&session, file_idx);
        let language = active_language(&session, file_idx);
        let (editors, subscriptions) = build_editors(texts, language, theme, cx);
        let active_section = first_conflict_section(&session, file_idx);

        Self {
            focus_handle,
            session,
            repo_root,
            file_idx,
            active_section,
            theme,
            status: None,
            offer_diff3,
            provider: Arc::new(CliProvider::claude_code()),
            ai: AiState::Idle,
            editors,
            _subscriptions: subscriptions,
        }
    }

    // ---- queries -----------------------------------------------------------

    fn active_doc(&self) -> Option<&MergeDocument> {
        self.session
            .files
            .get(self.file_idx)
            .and_then(|f| f.document.as_ref())
    }

    fn active_doc_mut(&mut self) -> Option<&mut MergeDocument> {
        self.session
            .files
            .get_mut(self.file_idx)
            .and_then(|f| f.document.as_mut())
    }

    fn active_conflict_count(&self) -> usize {
        self.active_doc().map_or(0, |d| d.conflict_count())
    }

    /// The full merged Result of the active file, assembled from the editors
    /// (which are the source of truth — they carry both resolutions and any
    /// manual edits). Reconstructs the file with its original line ending.
    fn result_text(&self, cx: &App) -> String {
        let Some(doc) = self.active_doc() else {
            return String::new();
        };
        let ending = doc.line_ending.as_str();
        let mut lines: Vec<String> = Vec::new();
        for editor in &self.editors {
            for line in editor.read(cx).text().split('\n') {
                lines.push(line.to_string());
            }
        }
        let mut out = lines.join(ending);
        if doc.trailing_newline && !lines.is_empty() {
            out.push_str(ending);
        }
        out
    }

    /// The first structural problem in the active file's resolved result, if any.
    /// Only meaningful once every conflict is resolved.
    fn active_issue(&self, cx: &App) -> Option<SyntaxIssue> {
        let doc = self.active_doc()?;
        if !doc.is_fully_resolved() {
            return None;
        }
        let language = active_language(&self.session, self.file_idx);
        validate::check(&self.result_text(cx), language)
            .into_iter()
            .next()
    }

    /// Status for the sidebar. The active file is judged on its live editor
    /// result; other files fall back to the document-based check.
    fn file_status(&self, idx: usize, cx: &App) -> FileStatus {
        if idx != self.file_idx {
            return self.session.files[idx].status();
        }
        match self.active_doc() {
            Some(doc) if !doc.is_fully_resolved() => FileStatus::Unresolved,
            Some(_) => {
                let language = active_language(&self.session, idx);
                if validate::check(&self.result_text(cx), language).is_empty() {
                    FileStatus::Resolved
                } else {
                    FileStatus::Invalid
                }
            }
            None => FileStatus::Unresolved,
        }
    }

    /// Section indices that hold a conflict, in document order.
    fn conflict_sections(&self) -> Vec<usize> {
        self.active_doc().map_or_else(Vec::new, |doc| {
            doc.sections
                .iter()
                .enumerate()
                .filter(|(_, s)| matches!(s, Section::Conflict(_)))
                .map(|(i, _)| i)
                .collect()
        })
    }

    /// Next unresolved conflict's section index after `from` (wrapping).
    fn next_unresolved_section(&self, from: usize) -> Option<usize> {
        let doc = self.active_doc()?;
        let unresolved: Vec<usize> = doc
            .sections
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s, Section::Conflict(c) if !c.is_resolved()))
            .map(|(i, _)| i)
            .collect();
        unresolved
            .iter()
            .find(|&&i| i > from)
            .or_else(|| unresolved.first())
            .copied()
    }

    // ---- mutations ---------------------------------------------------------

    fn rebuild_editors(&mut self, cx: &mut Context<Self>) {
        let texts = initial_texts(&self.session, self.file_idx);
        let language = active_language(&self.session, self.file_idx);
        let (editors, subscriptions) = build_editors(texts, language, self.theme, cx);
        self.editors = editors;
        self._subscriptions = subscriptions;
        self.active_section = first_conflict_section(&self.session, self.file_idx);
    }

    fn select_file(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx < self.session.files.len() && idx != self.file_idx {
            self.file_idx = idx;
            self.status = None;
            self.rebuild_editors(cx);
            cx.notify();
        }
    }

    fn select_next_file(&mut self, cx: &mut Context<Self>) {
        if self.file_idx + 1 < self.session.files.len() {
            self.select_file(self.file_idx + 1, cx);
        }
    }

    fn select_prev_file(&mut self, cx: &mut Context<Self>) {
        if self.file_idx > 0 {
            self.select_file(self.file_idx - 1, cx);
        }
    }

    fn focus_next_conflict(&mut self, cx: &mut Context<Self>) {
        let sections = self.conflict_sections();
        if sections.is_empty() {
            return;
        }
        let pos = sections.iter().position(|&i| i == self.active_section).unwrap_or(0);
        self.active_section = sections[(pos + 1) % sections.len()];
        cx.notify();
    }

    fn focus_prev_conflict(&mut self, cx: &mut Context<Self>) {
        let sections = self.conflict_sections();
        if sections.is_empty() {
            return;
        }
        let len = sections.len();
        let pos = sections.iter().position(|&i| i == self.active_section).unwrap_or(0);
        self.active_section = sections[(pos + len - 1) % len];
        cx.notify();
    }

    /// Resolve the conflict at section `sec` and push the result into its editor.
    fn apply_resolution(&mut self, sec: usize, resolution: Resolution, cx: &mut Context<Self>) {
        let joined = {
            let Some(doc) = self.active_doc_mut() else {
                return;
            };
            let Some(Section::Conflict(c)) = doc.sections.get_mut(sec) else {
                return;
            };
            c.resolution = Some(resolution);
            c.resolved_lines().map(|l| l.join("\n")).unwrap_or_default()
        };
        if let Some(editor) = self.editors.get(sec).cloned() {
            editor.update(cx, |e, cx| e.set_text_silent(joined, cx));
        }
        self.status = None;
        cx.notify();
    }

    fn apply_to_focused(&mut self, resolution: Resolution, cx: &mut Context<Self>) {
        let sec = self.active_section;
        self.apply_resolution(sec, resolution, cx);
        if let Some(next) = self.next_unresolved_section(sec) {
            self.active_section = next;
            cx.notify();
        }
    }

    /// Resolve every conflict in the active file to one side (Accept Left/Right).
    fn accept_all(&mut self, resolution: Resolution, cx: &mut Context<Self>) {
        for sec in self.conflict_sections() {
            self.apply_resolution(sec, resolution.clone(), cx);
        }
    }

    /// Clear every resolution in the active file, back to unresolved.
    fn reset_all(&mut self, cx: &mut Context<Self>) {
        for sec in self.conflict_sections() {
            self.reset_conflict(sec, cx);
        }
    }

    fn reset_conflict(&mut self, sec: usize, cx: &mut Context<Self>) {
        if let Some(doc) = self.active_doc_mut() {
            if let Some(Section::Conflict(c)) = doc.sections.get_mut(sec) {
                c.clear();
            }
        }
        if let Some(editor) = self.editors.get(sec).cloned() {
            editor.update(cx, |e, cx| e.set_text_silent("", cx));
        }
        cx.notify();
    }

    /// Mirror a directly-edited result editor back into the document: a conflict
    /// section becomes a manual resolution, an unchanged section is updated in
    /// place (the whole Result is editable).
    fn on_editor_changed(&mut self, editor: &Entity<CodeEditor>, cx: &mut Context<Self>) {
        let Some(sec) = self.editors.iter().position(|e| e == editor) else {
            return;
        };
        let lines = split_lines(editor.read(cx).text());
        if let Some(doc) = self.active_doc_mut() {
            // A conflict edit records a manual resolution. Stable edits live only
            // in the editor (the Result's source of truth) so the Local/Incoming
            // panes keep showing the original, untouched sides.
            if let Some(Section::Conflict(c)) = doc.sections.get_mut(sec) {
                c.resolution = Some(Resolution::Custom(lines));
            }
        }
        self.status = None;
        cx.notify();
    }

    /// Switch the repo to diff3 conflict markers, regenerate the current
    /// conflicts so they carry the common ancestor, and reload the session.
    fn enable_diff3(&mut self, cx: &mut Context<Self>) {
        self.offer_diff3 = false;
        let Some(root) = self.repo_root.clone() else {
            cx.notify();
            return;
        };
        match Repository::discover(&root) {
            Ok(repo) => {
                let _ = repo.set_conflict_style("diff3");
                let _ = repo.regenerate_conflicts();
                match repo.load_session() {
                    Ok(session) => {
                        self.session = session;
                        self.file_idx = self.session.first_unresolved().unwrap_or(0);
                        self.rebuild_editors(cx);
                        self.status = Some("Switched to diff3 — common ancestor now available".into());
                    }
                    Err(err) => {
                        self.status = Some(format!("Switched conflict style; reload failed: {err}").into());
                    }
                }
            }
            Err(err) => {
                self.status = Some(format!("Could not access repository: {err}").into());
            }
        }
        cx.notify();
    }

    fn dismiss_diff3(&mut self, cx: &mut Context<Self>) {
        self.offer_diff3 = false;
        cx.notify();
    }

    fn save_active(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.repo_root.clone() else {
            self.status = Some("No repository — cannot write file".into());
            cx.notify();
            return;
        };

        // Gather owned data so the (immutable) borrow ends before we touch
        // `self.status`. The Result comes from the editors, not the document.
        let prepared = {
            let Some(file) = self.session.files.get(self.file_idx) else {
                return;
            };
            let Some(doc) = file.document.as_ref() else {
                return;
            };
            if !doc.is_fully_resolved() {
                None
            } else {
                Some((file.name(), file.path.clone()))
            }
        };

        let Some((name, rel)) = prepared else {
            self.status = Some("Resolve every conflict before saving".into());
            cx.notify();
            return;
        };

        let rendered = self.result_text(cx);
        let language = active_language(&self.session, self.file_idx);
        if let Some(first) = validate::check(&rendered, language).into_iter().next() {
            self.status = Some(
                format!("Can't save — {} (line {})", first.message, first.line).into(),
            );
            cx.notify();
            return;
        }

        let abs = root.join(&rel);
        self.status = Some(match std::fs::write(&abs, rendered) {
            Ok(()) => format!("Saved {name}").into(),
            Err(err) => format!("Save failed: {err}").into(),
        });
        cx.notify();
    }

    // ---- AI assist ---------------------------------------------------------

    /// Build the AI request for the conflict at `sec`, including a little
    /// surrounding context from the neighbouring unchanged runs.
    fn build_ai_conflict(&self, sec: usize) -> Option<AiConflict> {
        let doc = self.active_doc()?;
        let Section::Conflict(c) = doc.sections.get(sec)? else {
            return None;
        };
        let path = self
            .session
            .files
            .get(self.file_idx)
            .map(|f| f.path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let context_before = match sec.checked_sub(1).and_then(|i| doc.sections.get(i)) {
            Some(Section::Stable(l)) => l.iter().rev().take(3).rev().cloned().collect(),
            _ => Vec::new(),
        };
        let context_after = match doc.sections.get(sec + 1) {
            Some(Section::Stable(l)) => l.iter().take(3).cloned().collect(),
            _ => Vec::new(),
        };
        Some(AiConflict {
            path,
            language: active_language(&self.session, self.file_idx),
            base: c.base.clone(),
            current: c.current.clone(),
            incoming: c.incoming.clone(),
            context_before,
            context_after,
        })
    }

    /// Resolve *every* conflict in the active file with the AI. Each provider
    /// call runs concurrently on the background executor; results are applied
    /// inline as they arrive (visible + undoable in the merge view, and not
    /// written to disk until Save), with live progress.
    fn resolve_all_with_ai(&mut self, cx: &mut Context<Self>) {
        let reqs: Vec<(usize, AiConflict)> = self
            .conflict_sections()
            .into_iter()
            .filter_map(|sec| self.build_ai_conflict(sec).map(|req| (sec, req)))
            .collect();
        if reqs.is_empty() {
            return;
        }
        let total = reqs.len();
        self.ai = AiState::Resolving { done: 0, total };
        cx.notify();

        let tasks: Vec<_> = reqs
            .into_iter()
            .map(|(sec, req)| {
                let provider = self.provider.clone();
                cx.background_executor()
                    .spawn(async move { (sec, provider.resolve(&req)) })
            })
            .collect();

        cx.spawn(async move |this, cx| {
            let mut applied = 0usize;
            let mut failed = 0usize;
            let mut last_error: Option<String> = None;
            for task in tasks {
                let (sec, result) = task.await;
                match result {
                    Ok(suggestion) => {
                        applied += 1;
                        let done = applied + failed;
                        let _ = this.update(cx, |this, cx| {
                            this.apply_custom(sec, suggestion.code.clone(), cx);
                            this.ai = AiState::Resolving { done, total };
                            cx.notify();
                        });
                    }
                    Err(err) => {
                        failed += 1;
                        last_error = Some(err.to_string());
                        let done = applied + failed;
                        let _ = this.update(cx, |this, cx| {
                            this.ai = AiState::Resolving { done, total };
                            cx.notify();
                        });
                    }
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.ai = AiState::Idle;
                let plural = if applied == 1 { "" } else { "s" };
                this.status = Some(if failed == 0 {
                    format!("AI resolved {applied} conflict{plural}").into()
                } else {
                    format!(
                        "AI resolved {applied}, {failed} failed{}",
                        last_error.map(|e| format!(" — {e}")).unwrap_or_default()
                    )
                    .into()
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn explain_with_ai(&mut self, cx: &mut Context<Self>) {
        let sec = self.active_section;
        let Some(req) = self.build_ai_conflict(sec) else {
            return;
        };
        self.ai = AiState::Explaining;
        cx.notify();

        let provider = self.provider.clone();
        let task = cx
            .background_executor()
            .spawn(async move { provider.explain(&req) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.ai = match result {
                    Ok(text) => AiState::Explanation { text },
                    Err(err) => AiState::Error {
                        message: err.to_string(),
                    },
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// Apply AI (or any) text as a manual resolution of conflict `sec`.
    fn apply_custom(&mut self, sec: usize, code: String, cx: &mut Context<Self>) {
        if let Some(doc) = self.active_doc_mut() {
            if let Some(Section::Conflict(c)) = doc.sections.get_mut(sec) {
                c.resolution = Some(Resolution::Custom(split_lines(&code)));
            }
        }
        if let Some(editor) = self.editors.get(sec).cloned() {
            editor.update(cx, |e, cx| e.set_text_silent(code, cx));
        }
        cx.notify();
    }

    fn dismiss_ai(&mut self, cx: &mut Context<Self>) {
        self.ai = AiState::Idle;
        cx.notify();
    }
}

impl Render for Zorro {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        div()
            .id("zorro-root")
            .key_context("Zorro")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &NextFile, _, cx| this.select_next_file(cx)))
            .on_action(cx.listener(|this, _: &PrevFile, _, cx| this.select_prev_file(cx)))
            .on_action(cx.listener(|this, _: &NextConflict, _, cx| this.focus_next_conflict(cx)))
            .on_action(cx.listener(|this, _: &PrevConflict, _, cx| this.focus_prev_conflict(cx)))
            .on_action(cx.listener(|this, _: &AcceptCurrent, _, cx| {
                this.apply_to_focused(Resolution::Current, cx)
            }))
            .on_action(cx.listener(|this, _: &AcceptIncoming, _, cx| {
                this.apply_to_focused(Resolution::Incoming, cx)
            }))
            .on_action(cx.listener(|this, _: &AcceptBoth, _, cx| {
                this.apply_to_focused(Resolution::Both { incoming_first: false }, cx)
            }))
            .on_action(cx.listener(|this, _: &AcceptAllLeft, _, cx| {
                this.accept_all(Resolution::Current, cx)
            }))
            .on_action(cx.listener(|this, _: &AcceptAllRight, _, cx| {
                this.accept_all(Resolution::Incoming, cx)
            }))
            .on_action(cx.listener(|this, _: &SaveFile, _, cx| this.save_active(cx)))
            .flex()
            .flex_col()
            .size_full()
            .bg(t.bg)
            .text_color(t.text)
            .text_size(px(14.))
            .child(self.render_header())
            .when(self.offer_diff3, |el| el.child(self.render_diff3_banner(cx)))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(cx))
                    .child(self.render_main(cx)),
            )
    }
}

impl Zorro {
    fn render_header(&self) -> impl IntoElement {
        let t = self.theme;
        let total = self.active_conflict_count();
        let resolved = self.active_doc().map_or(0, |d| d.resolved_count());
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .h(px(52.))
            .px_4()
            .bg(t.panel)
            .border_b_1()
            .border_color(t.border)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(div().font_weight(gpui::FontWeight::BOLD).child("Zorro"))
                    .child(
                        div()
                            .text_color(t.text_dim)
                            .text_size(px(12.))
                            .child(SharedString::from(self.session.summary())),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .when_some(self.status.clone(), |el, status| {
                        el.child(div().text_color(t.text_dim).text_size(px(12.)).child(status))
                    })
                    .child(
                        div()
                            .text_color(if total == resolved { t.resolved } else { t.pending })
                            .text_size(px(13.))
                            .child(SharedString::from(format!("{resolved} / {total} conflicts"))),
                    ),
            )
    }

    fn render_diff3_banner(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px_4()
            .py_2()
            .bg(t.selection)
            .border_b_1()
            .border_color(t.border)
            .child(
                div()
                    .text_color(t.text_dim)
                    .text_size(px(12.))
                    .child(
                        "This repo uses standard conflict markers. Switch to diff3 to capture the \
                         common ancestor? (regenerates the current conflict markers)",
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(
                        pill("use-diff3", "Use diff3", t.resolved, t.incoming_bg, t.resolved)
                            .on_click(cx.listener(|this, _, _, cx| this.enable_diff3(cx))),
                    )
                    .child(
                        pill("dismiss-diff3", "Dismiss", t.text_dim, t.bg, t.border)
                            .on_click(cx.listener(|this, _, _, cx| this.dismiss_diff3(cx))),
                    ),
            )
    }

    /// The AI panel shown below the top bar while a request is in flight or a
    /// result is awaiting the user's decision.
    fn render_ai_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let provider = self.provider.name().to_string();
        let bar = div()
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .py_2()
            .bg(t.selection)
            .border_b_1()
            .border_color(t.border);

        match &self.ai {
            AiState::Idle => div().into_any_element(),
            AiState::Explaining => bar
                .child(
                    div()
                        .text_color(t.text_dim)
                        .text_size(px(12.))
                        .child(SharedString::from(format!(
                            "✦ {provider} is explaining the focused conflict…"
                        ))),
                )
                .into_any_element(),
            AiState::Resolving { done, total } => bar
                .child(
                    div()
                        .text_color(t.text_dim)
                        .text_size(px(12.))
                        .child(SharedString::from(format!(
                            "✦ {provider} is resolving all conflicts… ({done}/{total})"
                        ))),
                )
                .into_any_element(),
            AiState::Error { message } => bar
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_color(t.error)
                                .text_size(px(12.))
                                .child(SharedString::from(format!("✦ AI error: {message}"))),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap_2()
                                .child(
                                    pill("ai-retry", "Retry", t.text_dim, t.bg, t.border).on_click(
                                        cx.listener(|this, _, _, cx| this.resolve_all_with_ai(cx)),
                                    ),
                                )
                                .child(
                                    pill("ai-dismiss", "Dismiss", t.text_dim, t.bg, t.border)
                                        .on_click(cx.listener(|this, _, _, cx| this.dismiss_ai(cx))),
                                ),
                        ),
                )
                .into_any_element(),
            AiState::Explanation { text } => {
                let lines: Vec<AnyElement> = text
                    .split('\n')
                    .map(|l| div().child(SharedString::from(l.to_string())).into_any_element())
                    .collect();
                bar.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .child(div().text_color(t.current).text_size(px(11.)).child("✦ EXPLANATION"))
                        .child(
                            pill("ai-dismiss", "Dismiss", t.text_dim, t.bg, t.border)
                                .on_click(cx.listener(|this, _, _, cx| this.dismiss_ai(cx))),
                        ),
                )
                .child(div().text_color(t.text).text_size(px(12.5)).children(lines))
                .into_any_element()
            }
        }
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let rows: Vec<AnyElement> = self
            .session
            .files
            .iter()
            .enumerate()
            .map(|(idx, file)| {
                let active = idx == self.file_idx;
                let status = self.file_status(idx, cx);
                let dot_color = match status {
                    FileStatus::Resolved => t.resolved,
                    FileStatus::Invalid => t.error,
                    FileStatus::Unresolved => t.pending,
                };
                let name_color = if active {
                    t.text
                } else if status == FileStatus::Invalid {
                    t.error
                } else {
                    t.text_dim
                };
                let count = file
                    .document
                    .as_ref()
                    .map(|d| format!("{}/{}", d.resolved_count(), d.conflict_count()))
                    .unwrap_or_else(|| "bin".to_string());

                div()
                    .id(("file", idx))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .text_size(px(13.))
                    .when(active, |el| el.bg(t.selection))
                    .hover(|s| s.bg(t.selection))
                    .on_click(cx.listener(move |this, _, _, cx| this.select_file(idx, cx)))
                    .child(div().text_color(dot_color).text_size(px(10.)).child("●"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(name_color)
                            .child(SharedString::from(file.name())),
                    )
                    .child(
                        div()
                            .text_color(t.text_faint)
                            .text_size(px(11.))
                            .child(SharedString::from(count)),
                    )
                    .into_any_element()
            })
            .collect();

        div()
            .flex()
            .flex_col()
            .w(px(232.))
            .h_full()
            .bg(t.sidebar)
            .border_r_1()
            .border_color(t.border)
            .child(
                div()
                    .px_4()
                    .py_2()
                    .text_color(t.text_faint)
                    .text_size(px(11.))
                    .child(SharedString::from(format!(
                        "FILES · {} / {} done",
                        self.session.completed_files(),
                        self.session.total_files()
                    ))),
            )
            .child(
                div()
                    .id("file-list")
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_2()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(rows),
            )
    }

    fn render_main(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let Some(doc) = self.active_doc() else {
            return div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .child(div().text_color(t.text_dim).child(self.empty_message()))
                .into_any_element();
        };
        let language = active_language(&self.session, self.file_idx);
        let can_save = doc.is_fully_resolved() && self.active_issue(cx).is_none();
        let file_name = self
            .session
            .files
            .get(self.file_idx)
            .map(|f| f.path.to_string_lossy().into_owned())
            .unwrap_or_default();

        // Build the five aligned vertical stacks.
        let mut left: Vec<AnyElement> = Vec::new();
        let mut left_gutter: Vec<AnyElement> = Vec::new();
        let mut middle: Vec<AnyElement> = Vec::new();
        let mut right_gutter: Vec<AnyElement> = Vec::new();
        let mut right: Vec<AnyElement> = Vec::new();

        // Each column numbers its own file independently — Local, Result, and
        // Incoming line numbers diverge after hunks of differing length.
        let mut left_no = 1usize;
        let mut mid_no = 1usize;
        let mut right_no = 1usize;
        for (sec, section) in doc.sections.iter().enumerate() {
            let editor = self.editors[sec].clone();
            let editor_lines = editor.read(cx).line_count();
            match section {
                Section::Stable(lines) if !lines.is_empty() => {
                    // Unchanged context: read-only on the sides, editable in the
                    // Result (so the whole file can be hand-edited).
                    let band = lines.len().max(editor_lines).max(1);
                    let lcode = code_cell(lines, language, &t, t.border, t.bg, band);
                    let rcode = code_cell(lines, language, &t, t.border, t.bg, band);
                    left.push(with_numbers(left_no, lines.len(), band, lcode, &t).into_any_element());
                    left_gutter.push(spacer(band));
                    middle.push(stable_editor_cell(editor, editor_lines, band, mid_no, &t));
                    right_gutter.push(spacer(band));
                    right.push(with_numbers(right_no, lines.len(), band, rcode, &t).into_any_element());
                    left_no += lines.len();
                    mid_no += editor_lines;
                    right_no += lines.len();
                }
                Section::Stable(_) => {}
                Section::Conflict(conflict) => {
                    let cur_len = conflict.current.len();
                    let inc_len = conflict.incoming.len();
                    let band = cur_len.max(inc_len).max(editor_lines).max(1);
                    let resolved = conflict.is_resolved();
                    let active = sec == self.active_section;

                    // Once resolved, blank out the side(s) that didn't win so the
                    // row reads as done — JetBrains-style.
                    let (clear_left, clear_right) = match &conflict.resolution {
                        Some(Resolution::Current) => (false, true),
                        Some(Resolution::Incoming) => (true, false),
                        Some(Resolution::Both { .. }) => (false, false),
                        Some(_) => (true, true), // Base / Custom: neither raw side verbatim
                        None => (false, false),
                    };

                    // Line-level diff between the two sides: left-only lines are
                    // removed (red), right-only lines are added (green).
                    let (removed, added) = line_diff_flags(&conflict.current, &conflict.incoming);

                    let left_content = if clear_left {
                        dimmed_cell(&conflict.current, &t, band)
                    } else if resolved {
                        code_cell(&conflict.current, language, &t, t.current, t.panel, band)
                    } else {
                        diff_code_cell(&conflict.current, &removed, language, &t, t.current, t.removed_bg, band)
                    };
                    let right_content = if clear_right {
                        dimmed_cell(&conflict.incoming, &t, band)
                    } else if resolved {
                        code_cell(&conflict.incoming, language, &t, t.incoming, t.panel, band)
                    } else {
                        diff_code_cell(&conflict.incoming, &added, language, &t, t.incoming, t.added_bg, band)
                    };

                    let undo = resolved.then(|| {
                        gutter_button(("undo", sec), "↺", t.text_dim, t)
                            .on_click(cx.listener(move |this, _, _, cx| this.reset_conflict(sec, cx)))
                    });

                    left.push(with_numbers(left_no, cur_len, band, left_content, &t).into_any_element());
                    left_gutter.push(gutter_left(sec, band, resolved, &t, cx));
                    middle.push(editor_cell(editor, editor_lines, band, mid_no, active, undo, &t));
                    right_gutter.push(gutter_right(sec, band, resolved, &t, cx));
                    right.push(with_numbers(right_no, inc_len, band, right_content, &t).into_any_element());

                    left_no += cur_len;
                    mid_no += editor_lines;
                    right_no += inc_len;
                }
            }
        }

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(self.render_top_bar(file_name))
            .when(!matches!(self.ai, AiState::Idle), |el| {
                el.child(self.render_ai_panel(cx))
            })
            .child(
                div()
                    .id("merge-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .min_h_full()
                            .child(stack(left).flex_1().min_w_0().bg(t.bg))
                            .child(stack(left_gutter).w(px(GUTTER_W)).bg(t.sidebar))
                            .child(stack(middle).flex_1().min_w_0().bg(t.panel))
                            .child(stack(right_gutter).w(px(GUTTER_W)).bg(t.sidebar))
                            .child(stack(right).flex_1().min_w_0().bg(t.bg)),
                    ),
            )
            .child(self.render_footer(can_save, cx))
            .into_any_element()
    }

    /// Top of the merge view: the file name plus the LOCAL/RESULT/INCOMING
    /// column titles.
    fn render_top_bar(&self, file_name: String) -> impl IntoElement {
        let t = self.theme;
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_4()
                    .py_1()
                    .bg(t.panel)
                    .text_color(t.text)
                    .text_size(px(13.))
                    .child(SharedString::from(file_name)),
            )
            .child(self.render_column_titles())
    }

    /// Bottom action bar — the JetBrains-style button row: status on the left,
    /// Accept Left / Accept Right / Reset / Save on the right.
    fn render_footer(&self, can_save: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let has_conflicts = self.active_conflict_count() > 0;
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px_4()
            .py_2()
            .bg(t.panel)
            .border_t_1()
            .border_color(t.border)
            // Left: the conflict-acceptance actions (JetBrains-style).
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .when(has_conflicts, |el| {
                        el.child(
                            pill("accept-left", "« Accept Left", t.current, t.current_bg, t.current)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.accept_all(Resolution::Current, cx)
                                })),
                        )
                        .child(
                            pill("accept-right", "Accept Right »", t.incoming, t.incoming_bg, t.incoming)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.accept_all(Resolution::Incoming, cx)
                                })),
                        )
                        .child(
                            pill("reset-all", "Reset", t.text_dim, t.bg, t.border)
                                .on_click(cx.listener(|this, _, _, cx| this.reset_all(cx))),
                        )
                        // AI assist on the focused conflict.
                        .child(
                            pill("ai-explain", "✦ Explain", t.text_dim, t.bg, t.border)
                                .on_click(cx.listener(|this, _, _, cx| this.explain_with_ai(cx))),
                        )
                        .child(
                            pill("ai-resolve", "✦ Resolve all with AI", t.current, t.current_bg, t.current)
                                .on_click(cx.listener(|this, _, _, cx| this.resolve_all_with_ai(cx))),
                        )
                    }),
            )
            // Right: status / validation message, then Save.
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .when_some(self.active_issue(cx), |el, issue| {
                        el.child(
                            div()
                                .text_color(t.error)
                                .text_size(px(12.))
                                .child(SharedString::from(format!(
                                    "⚠ syntax: {} (line {})",
                                    issue.message, issue.line
                                ))),
                        )
                    })
                    .when(self.active_issue(cx).is_none(), |el| {
                        el.when_some(self.status.clone(), |el, status| {
                            el.child(div().text_color(t.text_dim).text_size(px(12.)).child(status))
                        })
                    })
                    .child(
                        pill("save", "Save  ⌘S", t.bg, if can_save { t.resolved } else { t.border }, t.border)
                            .when(!can_save, |el| el.opacity(0.5))
                            .on_click(cx.listener(|this, _, _, cx| this.save_active(cx))),
                    ),
            )
    }

    fn render_column_titles(&self) -> impl IntoElement {
        let t = self.theme;
        let title = |label: &'static str, color: Rgba, end: bool| {
            div()
                .flex_1()
                .min_w_0()
                .px_3()
                .when(end, |el| el.flex().justify_end())
                .text_color(color)
                .text_size(px(11.))
                .child(label)
        };
        div()
            .flex()
            .flex_row()
            .py_1()
            .bg(t.panel)
            .border_b_1()
            .border_color(t.border)
            .child(title("LOCAL", t.current, false))
            .child(div().w(px(GUTTER_W)))
            .child(title("RESULT", t.text_dim, false))
            .child(div().w(px(GUTTER_W)))
            .child(title("INCOMING", t.incoming, true))
    }

    fn empty_message(&self) -> SharedString {
        if self.repo_root.is_none() {
            "No Git repository found here.".into()
        } else if self.session.total_files() == 0 {
            "No merge conflicts. Nothing to resolve 🎉".into()
        } else {
            "This file has no text conflicts.".into()
        }
    }
}

// ---- editor wiring ---------------------------------------------------------

/// One seed string per *section* (in document order): unchanged runs seed with
/// their text; conflicts seed with their resolved text (empty if unresolved).
/// The editor index therefore lines up 1:1 with `doc.sections`.
fn initial_texts(session: &MergeSession, idx: usize) -> Vec<String> {
    match session.files.get(idx).and_then(|f| f.document.as_ref()) {
        Some(doc) => doc
            .sections
            .iter()
            .map(|section| match section {
                Section::Stable(lines) => lines.join("\n"),
                Section::Conflict(c) => {
                    c.resolved_lines().map(|l| l.join("\n")).unwrap_or_default()
                }
            })
            .collect(),
        None => Vec::new(),
    }
}

/// Section index of the first conflict (or 0 if none), used to pick the focused
/// conflict.
fn first_conflict_section(session: &MergeSession, idx: usize) -> usize {
    session
        .files
        .get(idx)
        .and_then(|f| f.document.as_ref())
        .and_then(|d| d.sections.iter().position(|s| matches!(s, Section::Conflict(_))))
        .unwrap_or(0)
}

fn active_language(session: &MergeSession, idx: usize) -> Language {
    session
        .files
        .get(idx)
        .map(|f| Language::from_path(&f.path))
        .unwrap_or(Language::PlainText)
}

fn build_editors(
    texts: Vec<String>,
    language: Language,
    theme: Theme,
    cx: &mut Context<Zorro>,
) -> (Vec<Entity<CodeEditor>>, Vec<Subscription>) {
    let mut editors = Vec::with_capacity(texts.len());
    let mut subscriptions = Vec::with_capacity(texts.len());
    for text in texts {
        let editor = cx.new(|cx| {
            CodeEditor::new(
                text,
                language,
                theme.syntax,
                theme.text,
                theme.current,
                theme.text_selection,
                cx,
            )
        });
        let sub = cx.subscribe(&editor, |this, editor, _: &EditorEvent, cx| {
            this.on_editor_changed(&editor, cx)
        });
        subscriptions.push(sub);
        editors.push(editor);
    }
    (editors, subscriptions)
}

fn split_lines(text: &str) -> Vec<String> {
    text.split('\n').map(|s| s.to_string()).collect()
}

// ---- cell rendering --------------------------------------------------------

/// Wrap a list of cells into a vertical stack column.
fn stack(children: Vec<AnyElement>) -> Div {
    div().flex().flex_col().children(children)
}

/// A blank spacer of `rows` text-rows, used in the gutter columns opposite a
/// stable run.
fn spacer(rows: usize) -> AnyElement {
    div().h(px(rows as f32 * ROW)).into_any_element()
}

/// A read-only, syntax-highlighted code cell, exactly `pad_to` rows tall (blank
/// filler beyond the content) with a coloured left border.
fn code_cell(
    lines: &[String],
    language: Language,
    t: &Theme,
    accent: Rgba,
    bg: Rgba,
    pad_to: usize,
) -> Div {
    let mut highlighter = Highlighter::new(language);
    let mut rows: Vec<AnyElement> = Vec::new();
    for line in lines {
        let tokens = highlighter.highlight_line(line);
        let row = if line.is_empty() {
            div().h(px(ROW)).child(SharedString::from(" "))
        } else {
            let spans: Vec<AnyElement> = tokens
                .iter()
                .map(|tok| {
                    div()
                        .text_color(t.syntax.color(tok.kind))
                        .child(SharedString::from(line[tok.range.clone()].to_string()))
                        .into_any_element()
                })
                .collect();
            div().h(px(ROW)).flex().flex_row().children(spans)
        };
        rows.push(row.into_any_element());
    }
    for _ in lines.len()..pad_to {
        rows.push(div().h(px(ROW)).into_any_element());
    }

    div()
        .flex()
        .flex_col()
        .min_w_0()
        .px_3()
        .bg(bg)
        .border_l_2()
        .border_color(accent)
        .overflow_hidden()
        .font_family("Menlo")
        .text_size(px(12.5))
        .line_height(px(ROW))
        .text_color(t.text)
        .children(rows)
}

/// Like [`code_cell`] but colours individual lines per a diff `flags` vector:
/// flagged lines get `hl_bg` (red for removed / green for added), the rest are
/// the neutral panel background. Padded to `pad_to` rows.
fn diff_code_cell(
    lines: &[String],
    flags: &[bool],
    language: Language,
    t: &Theme,
    accent: Rgba,
    hl_bg: Rgba,
    pad_to: usize,
) -> Div {
    let mut highlighter = Highlighter::new(language);
    let mut rows: Vec<AnyElement> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let row_bg = if flags.get(i).copied().unwrap_or(false) {
            hl_bg
        } else {
            t.panel
        };
        let base = div().h(px(ROW)).w_full().px_3().bg(row_bg);
        let row = if line.is_empty() {
            base.child(SharedString::from(" "))
        } else {
            let tokens = highlighter.highlight_line(line);
            let spans: Vec<AnyElement> = tokens
                .iter()
                .map(|tok| {
                    div()
                        .text_color(t.syntax.color(tok.kind))
                        .child(SharedString::from(line[tok.range.clone()].to_string()))
                        .into_any_element()
                })
                .collect();
            base.flex().flex_row().children(spans)
        };
        rows.push(row.into_any_element());
    }
    for _ in lines.len()..pad_to {
        rows.push(div().h(px(ROW)).into_any_element());
    }

    div()
        .flex()
        .flex_col()
        .min_w_0()
        .bg(t.panel)
        .border_l_2()
        .border_color(accent)
        .overflow_hidden()
        .font_family("Menlo")
        .text_size(px(12.5))
        .line_height(px(ROW))
        .text_color(t.text)
        .children(rows)
}

/// The rejected side of a *resolved* conflict: its original lines are still
/// shown, but greyed out in a muted box so it's clear what was dropped and why.
fn dimmed_cell(lines: &[String], t: &Theme, pad_to: usize) -> Div {
    let mut rows: Vec<AnyElement> = lines
        .iter()
        .map(|line| {
            let content = if line.is_empty() {
                SharedString::from(" ")
            } else {
                SharedString::from(line.clone())
            };
            div().h(px(ROW)).child(content).into_any_element()
        })
        .collect();
    for _ in lines.len()..pad_to {
        rows.push(div().h(px(ROW)).into_any_element());
    }
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .px_3()
        .bg(t.selection)
        .border_l_2()
        .border_color(t.border)
        .overflow_hidden()
        .font_family("Menlo")
        .text_size(px(12.5))
        .line_height(px(ROW))
        .text_color(t.text_faint)
        .children(rows)
}

/// A right-aligned column of line numbers: `count` numbered rows from `start`,
/// then blank rows up to `pad_to` (alignment filler beyond the real lines).
fn line_numbers(start: usize, count: usize, pad_to: usize, t: &Theme) -> Div {
    let mut rows: Vec<AnyElement> = (0..count)
        .map(|i| {
            div()
                .h(px(ROW))
                .child(SharedString::from((start + i).to_string()))
                .into_any_element()
        })
        .collect();
    for _ in count..pad_to {
        rows.push(div().h(px(ROW)).into_any_element());
    }
    div()
        .flex()
        .flex_col()
        .items_end()
        .w(px(NUM_W))
        .px_2()
        .bg(t.sidebar)
        .text_color(t.text_faint)
        .text_size(px(11.))
        .line_height(px(ROW))
        .font_family("Menlo")
        .children(rows)
}

/// Wrap a content cell with a line-number gutter on its left.
fn with_numbers(start: usize, count: usize, pad_to: usize, content: Div, t: &Theme) -> Div {
    div()
        .flex()
        .flex_row()
        .min_w_0()
        .child(line_numbers(start, count, pad_to, t))
        .child(content.flex_1().min_w_0())
}

/// The Result cell for an unchanged run: line numbers + the editable editor.
/// Neutral styling (no conflict accent). Editing it edits the file in place.
fn stable_editor_cell(
    editor: Entity<CodeEditor>,
    editor_lines: usize,
    band: usize,
    start_no: usize,
    t: &Theme,
) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .w_full()
        .min_w_0()
        .min_h(px(band as f32 * ROW))
        .bg(t.panel)
        .border_l_2()
        .border_color(t.border)
        .child(line_numbers(start_no, editor_lines, editor_lines, t))
        .child(act_gutter(None))
        .child(div().flex_1().min_w_0().child(editor))
        .into_any_element()
}

/// The Result cell for a conflict: line numbers + the editable editor, padded to
/// `band` rows. A coloured left border flags resolved (green) vs pending (amber).
fn editor_cell(
    editor: Entity<CodeEditor>,
    editor_lines: usize,
    band: usize,
    start_no: usize,
    active: bool,
    undo: Option<Stateful<Div>>,
    t: &Theme,
) -> AnyElement {
    // The undo button is present exactly when the conflict is resolved.
    let accent = if undo.is_some() {
        t.resolved
    } else {
        t.pending
    };
    div()
        .flex()
        .flex_row()
        .w_full()
        .min_w_0()
        .min_h(px(band as f32 * ROW))
        .bg(if active { t.selection } else { t.panel })
        .border_l_2()
        .border_color(accent)
        .child(line_numbers(start_no, editor_lines, editor_lines, t))
        .child(act_gutter(undo))
        .child(div().flex_1().min_w_0().child(editor))
        .into_any_element()
}

/// The Result column's thin action sub-gutter (right of the line numbers),
/// holding the per-hunk undo button when present.
fn act_gutter(button: Option<Stateful<Div>>) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .w(px(ACT_W))
        .pt_1()
        .when_some(button, |el, b| el.child(b))
}

/// Left gutter for a conflict. While unresolved: ignore-current (`✕` → keep
/// incoming) and accept-current (`»`). Once resolved: empty (the undo lives in
/// the Result column).
fn gutter_left(n: usize, band: usize, resolved: bool, t: &Theme, cx: &mut Context<Zorro>) -> AnyElement {
    let row = div()
        .h(px(band as f32 * ROW))
        .flex()
        .flex_row()
        .justify_center()
        .pt_1()
        .gap_1();
    if resolved {
        return row.into_any_element();
    }
    row.child(
        gutter_button(("xl", n), "✕", t.text_faint, *t)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.apply_resolution(n, Resolution::Incoming, cx)
            })),
    )
    .child(
        gutter_button(("takel", n), "»", t.current, *t)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.apply_resolution(n, Resolution::Current, cx)
            })),
    )
    .into_any_element()
}

/// Right gutter for a conflict. While unresolved: accept-incoming (`«`) and
/// ignore-incoming (`✕` → keep current). Once resolved: empty.
fn gutter_right(n: usize, band: usize, resolved: bool, t: &Theme, cx: &mut Context<Zorro>) -> AnyElement {
    let row = div()
        .h(px(band as f32 * ROW))
        .flex()
        .flex_row()
        .justify_center()
        .pt_1()
        .gap_1();
    if resolved {
        return row.into_any_element();
    }
    row.child(
        gutter_button(("taker", n), "«", t.incoming, *t)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.apply_resolution(n, Resolution::Incoming, cx)
            })),
    )
    .child(
        gutter_button(("xr", n), "✕", t.text_faint, *t)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.apply_resolution(n, Resolution::Current, cx)
            })),
    )
    .into_any_element()
}

/// A small square gutter action button.
fn gutter_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    fg: Rgba,
    t: Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .w(px(20.))
        .h(px(18.))
        .rounded_md()
        .bg(t.bg)
        .text_color(fg)
        .text_size(px(12.))
        .cursor_pointer()
        .hover(|s| s.bg(t.selection))
        .child(label.into())
}

/// A pill button (text + border). The caller attaches `.on_click(...)`.
fn pill(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    fg: Rgba,
    bg: Rgba,
    border: Rgba,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .px_3()
        .py_1()
        .rounded_md()
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_color(fg)
        .text_size(px(12.))
        .cursor_pointer()
        .hover(|s| s.border_color(fg))
        .child(label.into())
}
