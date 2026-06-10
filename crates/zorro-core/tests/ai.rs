//! Tests for the AI-assist plumbing: prompt building, response parsing,
//! confidence heuristics, and the CLI provider mechanism (via a fake CLI that
//! reads the prompt on stdin and prints a canned response).

use zorro_core::ai::{
    explain_prompt, heuristic_confidence, parse_suggestion, resolve_prompt, AiConflict, AiError,
    AiProvider, CliProvider, Confidence,
};
use zorro_core::syntax::Language;

fn conflict() -> AiConflict {
    AiConflict {
        path: "src/auth.ts".into(),
        language: Language::TypeScript,
        base: Some(vec!["return { token };".into()]),
        current: vec!["return { token, ttl: 3600 };".into()],
        incoming: vec!["return { token, refresh: true };".into()],
        context_before: vec!["function login() {".into()],
        context_after: vec!["}".into()],
    }
}

#[test]
fn resolve_prompt_includes_all_sides() {
    let p = resolve_prompt(&conflict());
    assert!(p.contains("src/auth.ts"));
    assert!(p.contains("CURRENT (ours):"));
    assert!(p.contains("INCOMING (theirs):"));
    assert!(p.contains("BASE (common ancestor):"));
    assert!(p.contains("ttl: 3600"));
    assert!(p.contains("refresh: true"));
    assert!(p.contains("Confidence:"));
}

#[test]
fn explain_prompt_is_prose_oriented() {
    let p = explain_prompt(&conflict());
    assert!(p.contains("Explain"));
    assert!(p.contains("CURRENT:"));
    assert!(p.contains("INCOMING:"));
}

#[test]
fn parse_extracts_confidence_and_code_block() {
    let raw = "Confidence: High\n\
               Here is the merge:\n\
               ```ts\n\
               return { token, ttl: 3600, refresh: true };\n\
               ```\n";
    let s = parse_suggestion(raw, Confidence::Low);
    assert_eq!(s.confidence, Confidence::High);
    assert_eq!(s.code, "return { token, ttl: 3600, refresh: true };");
    assert_eq!(s.notes.as_deref(), Some("Here is the merge:"));
}

#[test]
fn parse_falls_back_when_no_fence_or_confidence() {
    let raw = "just some merged code\nwith two lines";
    let s = parse_suggestion(raw, Confidence::Medium);
    assert_eq!(s.confidence, Confidence::Medium);
    assert_eq!(s.code, "just some merged code\nwith two lines");
}

#[test]
fn parse_handles_multiline_code_block() {
    let raw = "Confidence: medium\n```rust\nfn a() {}\nfn b() {}\n```";
    let s = parse_suggestion(raw, Confidence::Low);
    assert_eq!(s.confidence, Confidence::Medium);
    assert_eq!(s.code, "fn a() {}\nfn b() {}");
}

#[test]
fn heuristic_flags_import_only_as_high() {
    let c = AiConflict {
        current: vec!["import java.util.Objects;".into()],
        incoming: vec!["import java.util.Set;".into()],
        ..conflict()
    };
    assert_eq!(heuristic_confidence(&c), Confidence::High);
}

#[test]
fn heuristic_scales_with_size() {
    let small = AiConflict {
        current: vec!["a()".into()],
        incoming: vec!["b()".into()],
        ..conflict()
    };
    assert_eq!(heuristic_confidence(&small), Confidence::High);

    let big = AiConflict {
        current: (0..6).map(|i| format!("let x{i} = {i};")).collect(),
        incoming: (0..6).map(|i| format!("let y{i} = {i};")).collect(),
        ..conflict()
    };
    assert_eq!(heuristic_confidence(&big), Confidence::Low);
}

/// Drive the real CLI-provider machinery with a fake "AI" shell command that
/// reads the prompt on stdin and prints a canned, fenced response.
#[test]
fn cli_provider_runs_command_and_parses_output() {
    if std::process::Command::new("sh").arg("-c").arg("exit 0").status().is_err() {
        eprintln!("skipping: no sh");
        return;
    }
    // `cat >/dev/null` consumes the prompt; then we print the canned answer.
    let script = "cat >/dev/null; printf 'Confidence: High\\n```\\nmerged();\\n```\\n'";
    let provider = CliProvider::new("Fake", "sh", vec!["-c".into(), script.into()]);

    let suggestion = provider.resolve(&conflict()).expect("resolve");
    assert_eq!(suggestion.confidence, Confidence::High);
    assert_eq!(suggestion.code, "merged();");
}

#[test]
fn cli_provider_reports_missing_binary() {
    let provider = CliProvider::new("Nope", "zorro-no-such-binary-xyz", vec![]);
    match provider.resolve(&conflict()) {
        Err(AiError::CliMissing(name)) => assert!(name.contains("zorro-no-such-binary")),
        other => panic!("expected CliMissing, got {other:?}"),
    }
}
