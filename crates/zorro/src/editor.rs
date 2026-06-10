//! A small, real **multiline** code editor built on GPUI's text-input
//! primitives (extended from gpui's single-line `input` example). It backs the
//! editable "Result" of each conflict so manual adjustments to merged text are
//! possible.
//!
//! Supports: typing, Enter/Backspace/Delete, Left/Right/Up/Down/Home/End,
//! click-to-place and click-drag selection, shift-arrow selection, Select-All,
//! copy/cut/paste, the macOS IME path, and per-line syntax highlighting via
//! [`zorro_core::syntax`].

use std::ops::Range;

use gpui::{
    actions, div, fill, point, prelude::*, px, relative, App, Bounds, ClipboardItem, Context,
    ElementId, ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle,
    Focusable, GlobalElementId, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PaintQuad, Pixels, Point, Rgba, ShapedLine, SharedString, Style, TextRun, UTF16Selection, Window,
};

use zorro_core::syntax::{Highlighter, Language};

use crate::theme::SyntaxTheme;

actions!(
    zorro_editor,
    [
        Backspace,
        DeleteForward,
        MoveLeft,
        MoveRight,
        MoveUp,
        MoveDown,
        MoveHome,
        MoveEnd,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectAll,
        Copy,
        Cut,
        Paste,
        InsertNewline,
    ]
);

/// Emitted whenever the editor's text changes, so the owning view can mark the
/// conflict resolved and refresh counts.
pub enum EditorEvent {
    Changed,
}

pub struct CodeEditor {
    focus_handle: FocusHandle,
    content: String,
    /// Selection as byte offsets into `content`. Cursor == empty range.
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    is_selecting: bool,
    language: Language,
    syntax: SyntaxTheme,
    text_color: Rgba,
    cursor_color: Rgba,
    selection_color: Rgba,
    // Layout cache populated during paint, used for mouse hit-testing.
    last_lines: Vec<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    last_line_height: Pixels,
}

impl CodeEditor {
    pub fn new(
        text: impl Into<String>,
        language: Language,
        syntax: SyntaxTheme,
        text_color: Rgba,
        cursor_color: Rgba,
        selection_color: Rgba,
        cx: &mut Context<Self>,
    ) -> Self {
        let content = text.into();
        let end = content.len();
        Self {
            focus_handle: cx.focus_handle(),
            content,
            selected_range: end..end,
            selection_reversed: false,
            marked_range: None,
            is_selecting: false,
            language,
            syntax,
            text_color,
            cursor_color,
            selection_color,
            last_lines: Vec::new(),
            last_bounds: None,
            last_line_height: px(20.),
        }
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    pub fn line_count(&self) -> usize {
        self.content.split('\n').count().max(1)
    }

    /// Replace the whole buffer *without* emitting a change event. Used when the
    /// app sets the result programmatically (accept a side / reset / switch
    /// file) — only genuine keystrokes should be reported as user edits.
    pub fn set_text_silent(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.content = text.into();
        let end = self.content.len();
        self.selected_range = end..end;
        self.selection_reversed = false;
        self.marked_range = None;
        cx.notify();
    }

    // ---- cursor / selection geometry --------------------------------------

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = self.clamp_boundary(offset);
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = self.clamp_boundary(offset);
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify();
    }

    fn line_starts(&self) -> Vec<usize> {
        let mut starts = vec![0];
        for (i, b) in self.content.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        starts
    }

    fn offset_to_row_col(&self, offset: usize) -> (usize, usize) {
        let mut row = 0;
        let mut line_start = 0;
        for (i, b) in self.content.bytes().enumerate() {
            if i >= offset {
                break;
            }
            if b == b'\n' {
                row += 1;
                line_start = i + 1;
            }
        }
        (row, offset.saturating_sub(line_start))
    }

    fn row_text(&self, row: usize) -> &str {
        self.content.split('\n').nth(row).unwrap_or("")
    }

    /// Offset one visual line above `offset`, preserving the byte column.
    fn offset_above(&self, offset: usize) -> usize {
        let (row, col) = self.offset_to_row_col(offset);
        if row == 0 {
            return 0;
        }
        let starts = self.line_starts();
        let prev_len = self.row_text(row - 1).len();
        self.clamp_boundary(starts[row - 1] + col.min(prev_len))
    }

    /// Offset one visual line below `offset`, preserving the byte column.
    fn offset_below(&self, offset: usize) -> usize {
        let (row, col) = self.offset_to_row_col(offset);
        let starts = self.line_starts();
        if row + 1 >= starts.len() {
            return self.content.len();
        }
        let next_len = self.row_text(row + 1).len();
        self.clamp_boundary(starts[row + 1] + col.min(next_len))
    }

    fn clamp_boundary(&self, mut offset: usize) -> usize {
        offset = offset.min(self.content.len());
        while offset > 0 && !self.content.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        let mut o = offset;
        while o > 0 {
            o -= 1;
            if self.content.is_char_boundary(o) {
                break;
            }
        }
        o
    }

    fn next_boundary(&self, offset: usize) -> usize {
        let mut o = (offset + 1).min(self.content.len());
        while o < self.content.len() && !self.content.is_char_boundary(o) {
            o += 1;
        }
        o
    }

    // ---- key actions -------------------------------------------------------

    fn left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let off = self.previous_boundary(self.cursor_offset());
            self.move_to(off, cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    fn right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let off = self.next_boundary(self.cursor_offset());
            self.move_to(off, cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    fn up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        let off = self.offset_above(self.cursor_offset());
        self.move_to(off, cx);
    }

    fn down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        let off = self.offset_below(self.cursor_offset());
        self.move_to(off, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        let off = self.previous_boundary(self.cursor_offset());
        self.select_to(off, cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        let off = self.next_boundary(self.cursor_offset());
        self.select_to(off, cx);
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let off = self.offset_above(self.cursor_offset());
        self.select_to(off, cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let off = self.offset_below(self.cursor_offset());
        self.select_to(off, cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        cx.notify();
    }

    fn home(&mut self, _: &MoveHome, _: &mut Window, cx: &mut Context<Self>) {
        let (row, _) = self.offset_to_row_col(self.cursor_offset());
        let starts = self.line_starts();
        self.move_to(starts[row], cx);
    }

    fn end(&mut self, _: &MoveEnd, _: &mut Window, cx: &mut Context<Self>) {
        let (row, _) = self.offset_to_row_col(self.cursor_offset());
        let starts = self.line_starts();
        let line_len = self.row_text(row).len();
        self.move_to(starts[row] + line_len, cx);
    }

    fn newline(&mut self, _: &InsertNewline, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_text_in_range(None, "\n", window, cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let prev = self.previous_boundary(self.cursor_offset());
            if prev == self.cursor_offset() {
                return;
            }
            self.selected_range = prev..self.cursor_offset();
            self.selection_reversed = false;
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete_forward(&mut self, _: &DeleteForward, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let next = self.next_boundary(self.cursor_offset());
            if next == self.cursor_offset() {
                return;
            }
            self.selected_range = self.cursor_offset()..next;
            self.selection_reversed = false;
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        self.is_selecting = true;
        let offset = self.index_for_position(event.position);
        if event.modifiers.shift {
            self.select_to(offset, cx);
        } else {
            self.move_to(offset, cx);
        }
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            let offset = self.index_for_position(event.position);
            self.select_to(offset, cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn index_for_position(&self, position: Point<Pixels>) -> usize {
        let Some(bounds) = self.last_bounds.as_ref() else {
            return self.cursor_offset();
        };
        if self.last_lines.is_empty() {
            return 0;
        }
        let rel_y = (position.y - bounds.top()).max(px(0.));
        let row = (f32::from(rel_y) / f32::from(self.last_line_height).max(1.0)) as usize;
        let row = row.min(self.last_lines.len() - 1);
        let starts = self.line_starts();
        let col = self.last_lines[row].closest_index_for_x(position.x - bounds.left());
        self.clamp_boundary(starts[row] + col)
    }

    // ---- utf16 conversions (required by EntityInputHandler) ----------------

    fn offset_from_utf16(&self, target: usize) -> usize {
        let mut utf8 = 0;
        let mut utf16 = 0;
        for ch in self.content.chars() {
            if utf16 >= target {
                break;
            }
            utf16 += ch.len_utf16();
            utf8 += ch.len_utf8();
        }
        utf8
    }

    fn offset_to_utf16(&self, target: usize) -> usize {
        let mut utf16 = 0;
        let mut utf8 = 0;
        for ch in self.content.chars() {
            if utf8 >= target {
                break;
            }
            utf8 += ch.len_utf8();
            utf16 += ch.len_utf16();
        }
        utf16
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }
}

impl EventEmitter<EditorEvent> for CodeEditor {}

impl Focusable for CodeEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for CodeEditor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range.as_ref().map(|r| self.range_to_utf16(r))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());

        self.content =
            self.content[0..range.start].to_owned() + new_text + &self.content[range.end..];
        let caret = range.start + new_text.len();
        self.selected_range = caret..caret;
        self.selection_reversed = false;
        self.marked_range = None;
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());

        self.content =
            self.content[0..range.start].to_owned() + new_text + &self.content[range.end..];
        self.marked_range =
            (!new_text.is_empty()).then(|| range.start..range.start + new_text.len());
        let new_caret = new_selected_range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .map(|r| r.start + range.start..r.end + range.start)
            .unwrap_or_else(|| {
                let c = range.start + new_text.len();
                c..c
            });
        self.selected_range = new_caret;
        self.selection_reversed = false;
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);
        let (row, _) = self.offset_to_row_col(range.start);
        let line = self.last_lines.get(row)?;
        let starts = self.line_starts();
        let col = range.start.saturating_sub(starts[row]);
        let x = line.x_for_index(col);
        let y = bounds.top() + self.last_line_height * row as f32;
        Some(Bounds::from_corners(
            point(bounds.left() + x, y),
            point(bounds.left() + x, y + self.last_line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.offset_to_utf16(self.index_for_position(point)))
    }
}

impl Render for CodeEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("CodeEditor")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete_forward))
            .on_action(cx.listener(Self::newline))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .size_full()
            .line_height(px(20.))
            .text_size(px(12.5))
            .font_family("Menlo")
            .child(EditorElement {
                editor: cx.entity(),
            })
    }
}

/// The custom element that shapes and paints the editor's lines, selection, and
/// cursor.
struct EditorElement {
    editor: Entity<CodeEditor>,
}

struct EditorPrepaint {
    lines: Vec<ShapedLine>,
    selection: Vec<PaintQuad>,
    cursor: Option<PaintQuad>,
}

impl IntoElement for EditorElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = EditorPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let line_count = self.editor.read(cx).line_count();
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = (window.line_height() * line_count as f32).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let editor = self.editor.read(cx);
        let style = window.text_style();
        let font = style.font();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let selection = editor.selected_range.clone();

        let mut highlighter = Highlighter::new(editor.language);
        let mut lines = Vec::new();
        let mut selection_quads = Vec::new();
        let mut line_start = 0usize;

        for (row, line) in editor.content.split('\n').enumerate() {
            let line_end = line_start + line.len();

            // Build syntax-coloured runs; advance the highlighter even for blank
            // lines so multi-line comments stay tracked.
            let tokens = highlighter.highlight_line(line);
            let (display, runs): (SharedString, Vec<TextRun>) = if line.is_empty() {
                (
                    " ".into(),
                    vec![TextRun {
                        len: 1,
                        font: font.clone(),
                        color: editor.text_color.into(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                )
            } else {
                let runs = tokens
                    .iter()
                    .map(|t| TextRun {
                        len: t.range.len(),
                        font: font.clone(),
                        color: editor.syntax.color(t.kind).into(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    })
                    .collect();
                (line.to_owned().into(), runs)
            };

            let shaped = window
                .text_system()
                .shape_line(display, font_size, &runs, None);

            // Selection highlight for this row.
            if selection.start < selection.end && selection.start <= line_end && selection.end >= line_start {
                let a = selection.start.clamp(line_start, line_end) - line_start;
                let b = selection.end.clamp(line_start, line_end) - line_start;
                let includes_newline = selection.end > line_end;
                let x0 = shaped.x_for_index(a);
                let mut x1 = shaped.x_for_index(b);
                if includes_newline {
                    x1 = shaped.x_for_index(line.len()) + px(6.);
                }
                if x1 > x0 {
                    let y = bounds.top() + line_height * row as f32;
                    selection_quads.push(fill(
                        Bounds::from_corners(
                            point(bounds.left() + x0, y),
                            point(bounds.left() + x1, y + line_height),
                        ),
                        editor.selection_color,
                    ));
                }
            }

            lines.push(shaped);
            line_start = line_end + 1;
        }

        let cursor_off = editor.cursor_offset();
        let (row, col) = editor.offset_to_row_col(cursor_off);
        let cursor = lines.get(row).map(|line| {
            let x = line.x_for_index(col);
            let y = bounds.top() + line_height * row as f32;
            fill(
                Bounds::new(point(bounds.left() + x, y), gpui::size(px(2.), line_height)),
                editor.cursor_color,
            )
        });

        EditorPrepaint {
            lines,
            selection: selection_quads,
            cursor,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.editor.read(cx).focus_handle.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );

        for quad in prepaint.selection.drain(..) {
            window.paint_quad(quad);
        }

        let line_height = window.line_height();
        let lines = std::mem::take(&mut prepaint.lines);
        for (i, line) in lines.iter().enumerate() {
            let origin = point(bounds.left(), bounds.top() + line_height * i as f32);
            line.paint(origin, line_height, gpui::TextAlign::Left, None, window, cx)
                .ok();
        }

        if focus.is_focused(window) {
            if let Some(cursor) = prepaint.cursor.take() {
                window.paint_quad(cursor);
            }
        }

        self.editor.update(cx, |editor, _| {
            editor.last_lines = lines;
            editor.last_bounds = Some(bounds);
            editor.last_line_height = line_height;
        });
    }
}
