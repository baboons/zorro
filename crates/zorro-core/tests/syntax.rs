//! Tests for the line-oriented syntax highlighter.

use std::path::Path;

use zorro_core::syntax::{Highlighter, Language, TokenKind};

/// Collect (text, kind) pairs for a single line, for readable assertions.
fn classify(lang: Language, line: &str) -> Vec<(String, TokenKind)> {
    let mut hl = Highlighter::new(lang);
    hl.highlight_line(line)
        .into_iter()
        .map(|t| (line[t.range].to_string(), t.kind))
        .collect()
}

/// The kind covering the first occurrence of `needle` in `line`.
fn kind_of(lang: Language, line: &str, needle: &str) -> TokenKind {
    let start = line.find(needle).expect("needle present");
    let mut hl = Highlighter::new(lang);
    hl.highlight_line(line)
        .into_iter()
        .find(|t| t.range.start <= start && start < t.range.end)
        .map(|t| t.kind)
        .expect("token covering needle")
}

#[test]
fn language_from_extension() {
    assert_eq!(Language::from_path(Path::new("a/b/foo.rs")), Language::Rust);
    assert_eq!(Language::from_path(Path::new("x.tsx")), Language::TypeScript);
    assert_eq!(Language::from_path(Path::new("x.py")), Language::Python);
    assert_eq!(Language::from_path(Path::new("x.json")), Language::Json);
    assert_eq!(Language::from_path(Path::new("README")), Language::PlainText);
}

#[test]
fn tiling_is_contiguous_and_covers_the_line() {
    let line = "let x = foo(1, \"hi\"); // trailing";
    let mut hl = Highlighter::new(Language::Rust);
    let tokens = hl.highlight_line(line);
    // Tokens must tile [0, line.len()) with no gaps or overlaps.
    let mut cursor = 0;
    for t in &tokens {
        assert_eq!(t.range.start, cursor, "gap or overlap before {t:?}");
        cursor = t.range.end;
    }
    assert_eq!(cursor, line.len());
}

#[test]
fn rust_keyword_string_number_comment() {
    let line = "let n = 42; // note";
    assert_eq!(kind_of(Language::Rust, line, "let"), TokenKind::Keyword);
    assert_eq!(kind_of(Language::Rust, line, "42"), TokenKind::Number);
    assert_eq!(kind_of(Language::Rust, line, "// note"), TokenKind::Comment);

    let s = "let s = \"hello\";";
    assert_eq!(kind_of(Language::Rust, s, "\"hello\""), TokenKind::String);
}

#[test]
fn function_call_and_type_heuristics() {
    let line = "let p = Point::new()";
    assert_eq!(kind_of(Language::Rust, line, "Point"), TokenKind::Type);
    assert_eq!(kind_of(Language::Rust, line, "new"), TokenKind::Function);
}

#[test]
fn block_comment_spans_multiple_lines() {
    let mut hl = Highlighter::new(Language::Rust);
    let l1 = hl.highlight_line("code /* start");
    let l2 = hl.highlight_line("still comment");
    let l3 = hl.highlight_line("end */ more");

    // Line 1: the `/* start` portion is a comment.
    assert!(l1.iter().any(|t| t.kind == TokenKind::Comment));
    // Line 2: entirely comment.
    assert_eq!(l2.len(), 1);
    assert_eq!(l2[0].kind, TokenKind::Comment);
    // Line 3: comment ends, then real code resumes.
    assert_eq!(l3[0].kind, TokenKind::Comment);
    assert!(l3.iter().any(|t| t.kind != TokenKind::Comment));
}

#[test]
fn python_uses_hash_comments() {
    let line = "x = 1  # comment";
    assert_eq!(kind_of(Language::Python, line, "# comment"), TokenKind::Comment);
    // `//` is NOT a comment in Python.
    let not = "a // b";
    assert_ne!(kind_of(Language::Python, not, "//"), TokenKind::Comment);
}

#[test]
fn plain_text_has_no_highlighting() {
    let spans = classify(Language::PlainText, "fn main() { 42 }");
    assert!(spans.iter().all(|(_, k)| *k == TokenKind::Plain));
}

#[test]
fn empty_line_yields_no_tokens() {
    let mut hl = Highlighter::new(Language::Rust);
    assert!(hl.highlight_line("").is_empty());
}
