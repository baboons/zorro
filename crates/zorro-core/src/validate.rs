//! Lightweight structural validation of a resolved file.
//!
//! Zorro is a merge tool, not a compiler — but a botched resolution usually
//! breaks a file in one very detectable way: the brackets no longer balance
//! (you took two sides whose `{`/`}` don't add up). This module checks that
//! `()`, `[]`, and `{}` are balanced and correctly nested, reusing the
//! [`crate::syntax`] tokenizer so brackets inside strings and comments are
//! ignored. It is intentionally conservative: a clean file never reports an
//! issue, so a reported issue is a real "you must fix this" signal.

use crate::syntax::{Highlighter, Language, TokenKind};

/// A single structural problem found in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxIssue {
    /// 1-based line number where the problem was detected.
    pub line: usize,
    pub message: String,
}

/// Check `source` for unbalanced brackets, returning every issue found (empty
/// means "looks structurally sound"). Languages without C-style bracketing
/// (YAML, Markdown, plain text) are never flagged.
pub fn check(source: &str, language: Language) -> Vec<SyntaxIssue> {
    if !uses_brackets(language) {
        return Vec::new();
    }

    let mut issues = Vec::new();
    let mut stack: Vec<(char, usize)> = Vec::new();
    let mut highlighter = Highlighter::new(language);

    for (idx, line) in source.split('\n').enumerate() {
        let lineno = idx + 1;
        for token in highlighter.highlight_line(line) {
            // Brackets inside strings and comments are not structural.
            if matches!(token.kind, TokenKind::String | TokenKind::Comment) {
                continue;
            }
            for ch in line[token.range].chars() {
                match ch {
                    '(' | '[' | '{' => stack.push((ch, lineno)),
                    ')' | ']' | '}' => match stack.pop() {
                        Some((open, _)) if matches_pair(open, ch) => {}
                        Some((open, open_line)) => issues.push(SyntaxIssue {
                            line: lineno,
                            message: format!(
                                "mismatched '{ch}' (opened '{open}' at line {open_line})"
                            ),
                        }),
                        None => issues.push(SyntaxIssue {
                            line: lineno,
                            message: format!("unmatched closing '{ch}'"),
                        }),
                    },
                    _ => {}
                }
            }
        }
    }

    for (open, open_line) in stack {
        issues.push(SyntaxIssue {
            line: open_line,
            message: format!("unclosed '{open}'"),
        });
    }

    issues
}

/// Whether a language uses C-style bracket nesting worth balancing.
fn uses_brackets(language: Language) -> bool {
    matches!(
        language,
        Language::Rust
            | Language::TypeScript
            | Language::JavaScript
            | Language::Go
            | Language::CSharp
            | Language::Java
            | Language::Json
            | Language::Python
    )
}

fn matches_pair(open: char, close: char) -> bool {
    matches!((open, close), ('(', ')') | ('[', ']') | ('{', '}'))
}
