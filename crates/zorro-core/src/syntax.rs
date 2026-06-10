//! A small, dependency-free syntax highlighter.
//!
//! This is deliberately *not* a full tree-sitter integration (the eventual goal
//! per the spec) — it is a fast, line-oriented lexer that produces a contiguous
//! tiling of [`Token`]s per line, good enough to colour keywords, types,
//! strings, comments, numbers, and function calls across the languages Zorro
//! targets. The UI maps [`TokenKind`] to colours.
//!
//! The lexer carries block-comment state across lines, so callers feed lines in
//! order through a single [`Highlighter`].

use std::ops::Range;

/// The semantic class of a run of characters, used to pick a colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Plain,
    Keyword,
    Type,
    String,
    Comment,
    Number,
    Function,
    Punctuation,
}

/// A contiguous run of one [`TokenKind`] within a single line, by byte range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub range: Range<usize>,
    pub kind: TokenKind,
}

/// Languages Zorro can highlight. [`Language::PlainText`] disables highlighting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Go,
    Python,
    CSharp,
    Java,
    Json,
    Yaml,
    Markdown,
    PlainText,
}

impl Language {
    /// Guess the language from a file path's extension.
    pub fn from_path(path: &std::path::Path) -> Language {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        match ext.as_deref() {
            Some("rs") => Language::Rust,
            Some("ts" | "tsx") => Language::TypeScript,
            Some("js" | "jsx" | "mjs" | "cjs") => Language::JavaScript,
            Some("go") => Language::Go,
            Some("py" | "pyi") => Language::Python,
            Some("cs") => Language::CSharp,
            Some("java") => Language::Java,
            Some("json") => Language::Json,
            Some("yaml" | "yml") => Language::Yaml,
            Some("md" | "markdown") => Language::Markdown,
            _ => Language::PlainText,
        }
    }

    fn spec(self) -> LangSpec {
        // Common C-family string delimiters.
        const C_STRINGS: &[char] = &['"', '\''];
        match self {
            Language::Rust => LangSpec {
                line_comment: Some("//"),
                block_comment: Some(("/*", "*/")),
                strings: &['"'],
                keywords: &[
                    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else",
                    "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop",
                    "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self",
                    "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
                    "while",
                ],
            },
            Language::TypeScript | Language::JavaScript => LangSpec {
                line_comment: Some("//"),
                block_comment: Some(("/*", "*/")),
                strings: &['"', '\'', '`'],
                keywords: &[
                    "abstract", "any", "as", "async", "await", "boolean", "break", "case", "catch",
                    "class", "const", "continue", "debugger", "default", "delete", "do", "else",
                    "enum", "export", "extends", "false", "finally", "for", "from", "function",
                    "if", "implements", "import", "in", "instanceof", "interface", "let", "new",
                    "null", "number", "of", "private", "protected", "public", "readonly", "return",
                    "static", "string", "super", "switch", "this", "throw", "true", "try", "type",
                    "typeof", "undefined", "var", "void", "while", "yield",
                ],
            },
            Language::Go => LangSpec {
                line_comment: Some("//"),
                block_comment: Some(("/*", "*/")),
                strings: &['"', '`'],
                keywords: &[
                    "break", "case", "chan", "const", "continue", "default", "defer", "else",
                    "fallthrough", "for", "func", "go", "goto", "if", "import", "interface", "map",
                    "package", "range", "return", "select", "struct", "switch", "type", "var",
                    "nil", "true", "false",
                ],
            },
            Language::Python => LangSpec {
                line_comment: Some("#"),
                block_comment: None,
                strings: &['"', '\''],
                keywords: &[
                    "and", "as", "assert", "async", "await", "break", "class", "continue", "def",
                    "del", "elif", "else", "except", "False", "finally", "for", "from", "global",
                    "if", "import", "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass",
                    "raise", "return", "True", "try", "while", "with", "yield",
                ],
            },
            Language::CSharp => LangSpec {
                line_comment: Some("//"),
                block_comment: Some(("/*", "*/")),
                strings: C_STRINGS,
                keywords: &[
                    "abstract", "as", "base", "bool", "break", "case", "catch", "class", "const",
                    "continue", "default", "do", "else", "enum", "false", "finally", "for",
                    "foreach", "if", "in", "interface", "internal", "is", "namespace", "new",
                    "null", "out", "override", "private", "protected", "public", "readonly",
                    "return", "sealed", "static", "string", "struct", "switch", "this", "throw",
                    "true", "try", "using", "var", "virtual", "void", "while",
                ],
            },
            Language::Java => LangSpec {
                line_comment: Some("//"),
                block_comment: Some(("/*", "*/")),
                strings: &['"'],
                keywords: &[
                    "abstract", "boolean", "break", "byte", "case", "catch", "char", "class",
                    "const", "continue", "default", "do", "double", "else", "enum", "extends",
                    "final", "finally", "float", "for", "if", "implements", "import", "instanceof",
                    "int", "interface", "long", "new", "null", "package", "private", "protected",
                    "public", "return", "static", "super", "switch", "this", "throw", "throws",
                    "true", "false", "try", "void", "while",
                ],
            },
            Language::Json => LangSpec {
                line_comment: None,
                block_comment: None,
                strings: &['"'],
                keywords: &["true", "false", "null"],
            },
            Language::Yaml => LangSpec {
                line_comment: Some("#"),
                block_comment: None,
                strings: &['"', '\''],
                keywords: &["true", "false", "null", "yes", "no"],
            },
            Language::Markdown | Language::PlainText => LangSpec {
                line_comment: None,
                block_comment: None,
                strings: &[],
                keywords: &[],
            },
        }
    }
}

struct LangSpec {
    line_comment: Option<&'static str>,
    block_comment: Option<(&'static str, &'static str)>,
    strings: &'static [char],
    keywords: &'static [&'static str],
}

/// Stateful, line-by-line highlighter. Create one per pass and feed lines in
/// document order so multi-line block comments are tracked.
pub struct Highlighter {
    spec: LangSpec,
    plain: bool,
    in_block_comment: bool,
}

impl Highlighter {
    pub fn new(language: Language) -> Highlighter {
        let spec = language.spec();
        let plain = spec.keywords.is_empty()
            && spec.line_comment.is_none()
            && spec.block_comment.is_none()
            && spec.strings.is_empty();
        Highlighter {
            spec,
            plain,
            in_block_comment: false,
        }
    }

    /// Tokenize one line into a contiguous, gap-free tiling of tokens (Plain
    /// runs fill anything not otherwise classified).
    pub fn highlight_line(&mut self, line: &str) -> Vec<Token> {
        // Languages with nothing to highlight (plain text / markdown) get a
        // single plain run so callers never colour them.
        if self.plain {
            return if line.is_empty() {
                Vec::new()
            } else {
                vec![Token {
                    range: 0..line.len(),
                    kind: TokenKind::Plain,
                }]
            };
        }

        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut tokens: Vec<Token> = Vec::new();
        let mut i = 0;

        let push = |tokens: &mut Vec<Token>, start: usize, end: usize, kind: TokenKind| {
            if end > start {
                tokens.push(Token {
                    range: start..end,
                    kind,
                });
            }
        };

        while i < len {
            // Inside a block comment: consume until the terminator.
            if self.in_block_comment {
                let (_, close) = self.spec.block_comment.unwrap();
                if let Some(end_rel) = line[i..].find(close) {
                    let end = i + end_rel + close.len();
                    push(&mut tokens, i, end, TokenKind::Comment);
                    self.in_block_comment = false;
                    i = end;
                } else {
                    push(&mut tokens, i, len, TokenKind::Comment);
                    i = len;
                }
                continue;
            }

            let rest = &line[i..];

            // Line comment to end of line.
            if let Some(lc) = self.spec.line_comment {
                if rest.starts_with(lc) {
                    push(&mut tokens, i, len, TokenKind::Comment);
                    i = len;
                    continue;
                }
            }

            // Block comment open.
            if let Some((open, close)) = self.spec.block_comment {
                if rest.starts_with(open) {
                    if let Some(end_rel) = line[i + open.len()..].find(close) {
                        let end = i + open.len() + end_rel + close.len();
                        push(&mut tokens, i, end, TokenKind::Comment);
                        i = end;
                    } else {
                        push(&mut tokens, i, len, TokenKind::Comment);
                        self.in_block_comment = true;
                        i = len;
                    }
                    continue;
                }
            }

            let ch = bytes[i];

            // String literal (single line; unterminated runs to end of line).
            if self.spec.strings.contains(&(ch as char)) {
                let quote = ch;
                let mut j = i + 1;
                while j < len {
                    if bytes[j] == b'\\' {
                        j += 2;
                        continue;
                    }
                    if bytes[j] == quote {
                        j += 1;
                        break;
                    }
                    j += 1;
                }
                let end = j.min(len);
                push(&mut tokens, i, end, TokenKind::String);
                i = end;
                continue;
            }

            // Number.
            if ch.is_ascii_digit() {
                let mut j = i + 1;
                while j < len
                    && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'.' || bytes[j] == b'_')
                {
                    j += 1;
                }
                push(&mut tokens, i, j, TokenKind::Number);
                i = j;
                continue;
            }

            // Identifier / keyword / type / function.
            if ch == b'_' || ch.is_ascii_alphabetic() {
                let mut j = i + 1;
                while j < len && (bytes[j] == b'_' || bytes[j].is_ascii_alphanumeric()) {
                    j += 1;
                }
                let word = &line[i..j];
                let kind = if self.spec.keywords.contains(&word) {
                    TokenKind::Keyword
                } else if word.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                    TokenKind::Type
                } else if line[j..].trim_start().starts_with('(') {
                    TokenKind::Function
                } else {
                    TokenKind::Plain
                };
                push(&mut tokens, i, j, kind);
                i = j;
                continue;
            }

            // Punctuation vs plain whitespace.
            if !ch.is_ascii_whitespace() && ch.is_ascii_punctuation() {
                push(&mut tokens, i, i + 1, TokenKind::Punctuation);
            } else {
                push(&mut tokens, i, i + 1, TokenKind::Plain);
            }
            i += 1;
        }

        coalesce(tokens, len)
    }
}

/// Merge adjacent same-kind tokens and ensure the tiling covers `0..len`.
fn coalesce(tokens: Vec<Token>, len: usize) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    let mut cursor = 0;
    for token in tokens {
        // Fill any gap (shouldn't happen, but keeps the tiling contiguous).
        if token.range.start > cursor {
            push_or_extend(&mut out, cursor..token.range.start, TokenKind::Plain);
        }
        cursor = token.range.end;
        push_or_extend(&mut out, token.range.clone(), token.kind);
    }
    if cursor < len {
        push_or_extend(&mut out, cursor..len, TokenKind::Plain);
    }
    out
}

fn push_or_extend(out: &mut Vec<Token>, range: Range<usize>, kind: TokenKind) {
    if range.is_empty() {
        return;
    }
    match out.last_mut() {
        Some(last) if last.kind == kind && last.range.end == range.start => {
            last.range.end = range.end;
        }
        _ => out.push(Token { range, kind }),
    }
}
