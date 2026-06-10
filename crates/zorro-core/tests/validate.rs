//! Tests for structural (bracket-balance) validation.

use zorro_core::syntax::Language;
use zorro_core::validate::check;

#[test]
fn balanced_code_has_no_issues() {
    let src = "fn main() {\n    let v = vec![1, 2, 3];\n    foo(v);\n}\n";
    assert!(check(src, Language::Rust).is_empty());
}

#[test]
fn extra_closing_brace_is_flagged() {
    let src = "fn main() {\n}\n}\n";
    let issues = check(src, Language::Rust);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].line, 3);
}

#[test]
fn unclosed_brace_is_flagged_at_its_opening_line() {
    let src = "fn main() {\n    if x {\n}\n";
    let issues = check(src, Language::Rust);
    assert_eq!(issues.len(), 1);
    // The unclosed `{` is the one opened on line 1.
    assert_eq!(issues[0].line, 1);
}

#[test]
fn mismatched_bracket_is_flagged() {
    let src = "let a = (1, 2];\n";
    let issues = check(src, Language::Rust);
    assert_eq!(issues.len(), 1);
    assert!(issues[0].message.contains("mismatched"));
}

#[test]
fn brackets_inside_strings_are_ignored() {
    let src = "let s = \"a { b ( c\";\nlet t = \"} ) ]\";\n";
    assert!(check(src, Language::Rust).is_empty());
}

#[test]
fn brackets_inside_comments_are_ignored() {
    let src = "fn main() {\n    // closing here } does not count\n}\n";
    assert!(check(src, Language::Rust).is_empty());
}

#[test]
fn balanced_json_passes_and_unbalanced_fails() {
    assert!(check("{\n  \"a\": [1, 2, 3]\n}\n", Language::Json).is_empty());
    assert!(!check("{\n  \"a\": [1, 2, 3]\n", Language::Json).is_empty());
}

#[test]
fn python_brackets_are_checked() {
    assert!(check("def f():\n    return [1, 2]\n", Language::Python).is_empty());
    assert!(!check("def f():\n    return [1, 2\n", Language::Python).is_empty());
}

#[test]
fn non_bracket_languages_are_never_flagged() {
    // A YAML document with stray brackets shouldn't be reported.
    let yaml = "items:\n  - one ]\n  - two\n";
    assert!(check(yaml, Language::Yaml).is_empty());
    assert!(check("# heading }", Language::Markdown).is_empty());
    assert!(check("plain ) text", Language::PlainText).is_empty());
}
