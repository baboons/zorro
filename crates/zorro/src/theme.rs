//! Dark-first color palette. Kept as a plain value type so views can hold a copy
//! and a future light theme is a second constructor away.

use gpui::{rgb, Rgba};
use zorro_core::syntax::TokenKind;

#[derive(Clone, Copy)]
pub struct Theme {
    pub bg: Rgba,
    pub panel: Rgba,
    pub sidebar: Rgba,
    pub border: Rgba,
    pub text: Rgba,
    pub text_dim: Rgba,
    pub text_faint: Rgba,
    /// "Ours" / current accent + tinted background.
    pub current: Rgba,
    pub current_bg: Rgba,
    /// "Theirs" / incoming accent + tinted background.
    pub incoming: Rgba,
    pub incoming_bg: Rgba,
    /// Per-line diff backgrounds: a line added on a side (green) / removed (red).
    pub added_bg: Rgba,
    pub removed_bg: Rgba,
    pub selection: Rgba,
    /// Highlight behind selected text in the editor.
    pub text_selection: Rgba,
    pub resolved: Rgba,
    pub pending: Rgba,
    /// Error / invalid state (red) — e.g. a file that won't parse.
    pub error: Rgba,
    pub syntax: SyntaxTheme,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            syntax: SyntaxTheme::dark(),
            text_selection: rgb(0x2f5074),
            bg: rgb(0x1b1b1f),
            panel: rgb(0x232329),
            sidebar: rgb(0x1f1f24),
            border: rgb(0x34343c),
            text: rgb(0xe7e7ea),
            text_dim: rgb(0xa0a0a8),
            text_faint: rgb(0x6b6b74),
            current: rgb(0x6cb6ff),
            current_bg: rgb(0x142338),
            incoming: rgb(0x6bd089),
            incoming_bg: rgb(0x12301f),
            added_bg: rgb(0x1d3a26),
            removed_bg: rgb(0x3d2226),
            selection: rgb(0x2d2d36),
            resolved: rgb(0x6bd089),
            pending: rgb(0xe0b341),
            error: rgb(0xe0625f),
        }
    }
}

/// Colours for each syntax [`TokenKind`] (Darcula-flavoured).
#[derive(Clone, Copy)]
pub struct SyntaxTheme {
    pub plain: Rgba,
    pub keyword: Rgba,
    pub type_name: Rgba,
    pub string: Rgba,
    pub comment: Rgba,
    pub number: Rgba,
    pub function: Rgba,
    pub punctuation: Rgba,
}

impl SyntaxTheme {
    pub fn dark() -> Self {
        Self {
            plain: rgb(0xe7e7ea),
            keyword: rgb(0xcc7832),
            type_name: rgb(0x4ec9b0),
            string: rgb(0x6a8759),
            comment: rgb(0x6a737d),
            number: rgb(0x6897bb),
            function: rgb(0xdcdcaa),
            punctuation: rgb(0xa6accd),
        }
    }

    pub fn color(&self, kind: TokenKind) -> Rgba {
        match kind {
            TokenKind::Plain => self.plain,
            TokenKind::Keyword => self.keyword,
            TokenKind::Type => self.type_name,
            TokenKind::String => self.string,
            TokenKind::Comment => self.comment,
            TokenKind::Number => self.number,
            TokenKind::Function => self.function,
            TokenKind::Punctuation => self.punctuation,
        }
    }
}
