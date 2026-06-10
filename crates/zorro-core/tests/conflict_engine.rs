//! End-to-end tests for the conflict parsing / resolution / render pipeline.

use zorro_core::conflict::{LineEnding, MergeDocument, Resolution, Section, Side};

const MERGE_STYLE: &str = "\
fn main() {
<<<<<<< HEAD
    println!(\"current\");
=======
    println!(\"incoming\");
>>>>>>> feature/login
}
";

const DIFF3_STYLE: &str = "\
config:
<<<<<<< HEAD
  timeout = 30
||||||| merged common ancestors
  timeout = 10
=======
  timeout = 60
>>>>>>> origin/main
done
";

#[test]
fn parses_merge_style_conflict() {
    let doc = MergeDocument::parse(MERGE_STYLE);
    assert_eq!(doc.conflict_count(), 1);

    let c = doc.conflict(0).unwrap();
    assert_eq!(c.current, vec!["    println!(\"current\");"]);
    assert_eq!(c.incoming, vec!["    println!(\"incoming\");"]);
    assert!(!c.has_base());
    assert_eq!(c.labels.current, "HEAD");
    assert_eq!(c.labels.incoming, "feature/login");
}

#[test]
fn parses_diff3_style_with_base() {
    let doc = MergeDocument::parse(DIFF3_STYLE);
    let c = doc.conflict(0).unwrap();
    assert!(c.has_base());
    assert_eq!(c.side(Side::Base), &["  timeout = 10".to_string()]);
    assert_eq!(c.labels.base.as_deref(), Some("merged common ancestors"));
}

#[test]
fn unresolved_document_round_trips_exactly() {
    for input in [MERGE_STYLE, DIFF3_STYLE] {
        let doc = MergeDocument::parse(input);
        assert_eq!(doc.render(), input, "round trip must be lossless");
    }
}

#[test]
fn accept_current_renders_current_side() {
    let mut doc = MergeDocument::parse(MERGE_STYLE);
    doc.resolve(0, Resolution::Current);
    assert!(doc.is_fully_resolved());
    assert_eq!(
        doc.render(),
        "fn main() {\n    println!(\"current\");\n}\n"
    );
}

#[test]
fn accept_incoming_renders_incoming_side() {
    let mut doc = MergeDocument::parse(MERGE_STYLE);
    doc.resolve(0, Resolution::Incoming);
    assert_eq!(
        doc.render(),
        "fn main() {\n    println!(\"incoming\");\n}\n"
    );
}

#[test]
fn accept_both_orders() {
    let mut doc = MergeDocument::parse(MERGE_STYLE);
    doc.resolve(0, Resolution::Both { incoming_first: false });
    assert_eq!(
        doc.render(),
        "fn main() {\n    println!(\"current\");\n    println!(\"incoming\");\n}\n"
    );

    let mut doc = MergeDocument::parse(MERGE_STYLE);
    doc.resolve(0, Resolution::Both { incoming_first: true });
    assert_eq!(
        doc.render(),
        "fn main() {\n    println!(\"incoming\");\n    println!(\"current\");\n}\n"
    );
}

#[test]
fn accept_base_uses_ancestor() {
    let mut doc = MergeDocument::parse(DIFF3_STYLE);
    doc.resolve(0, Resolution::Base);
    assert_eq!(doc.render(), "config:\n  timeout = 10\ndone\n");
}

#[test]
fn custom_resolution_replaces_region() {
    let mut doc = MergeDocument::parse(MERGE_STYLE);
    doc.resolve(0, Resolution::Custom(vec!["    println!(\"merged\");".to_string()]));
    assert_eq!(
        doc.render(),
        "fn main() {\n    println!(\"merged\");\n}\n"
    );
}

#[test]
fn clearing_a_resolution_restores_markers() {
    let mut doc = MergeDocument::parse(MERGE_STYLE);
    doc.resolve(0, Resolution::Current);
    doc.conflict_mut(0).unwrap().clear();
    assert_eq!(doc.unresolved_count(), 1);
    assert_eq!(doc.render(), MERGE_STYLE);
}

#[test]
fn multiple_conflicts_are_independent() {
    let input = "\
a
<<<<<<< HEAD
one-current
=======
one-incoming
>>>>>>> b
middle
<<<<<<< HEAD
two-current
=======
two-incoming
>>>>>>> b
z
";
    let mut doc = MergeDocument::parse(input);
    assert_eq!(doc.conflict_count(), 2);

    doc.resolve(0, Resolution::Current);
    assert_eq!(doc.unresolved_count(), 1);
    assert!(!doc.is_fully_resolved());

    doc.resolve(1, Resolution::Incoming);
    assert!(doc.is_fully_resolved());
    assert_eq!(
        doc.render(),
        "a\none-current\nmiddle\ntwo-incoming\nz\n"
    );
}

#[test]
fn file_without_conflicts_parses_and_round_trips() {
    let input = "just\nsome\nplain\ntext\n";
    let doc = MergeDocument::parse(input);
    assert_eq!(doc.conflict_count(), 0);
    assert!(doc.is_fully_resolved());
    assert_eq!(doc.render(), input);
}

#[test]
fn preserves_missing_trailing_newline() {
    let input = "a\n<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> b"; // no final newline
    let doc = MergeDocument::parse(input);
    assert!(!doc.trailing_newline);
    assert_eq!(doc.render(), input);
}

#[test]
fn preserves_crlf_line_endings() {
    let input = "a\r\n<<<<<<< HEAD\r\nx\r\n=======\r\ny\r\n>>>>>>> b\r\nz\r\n";
    let doc = MergeDocument::parse(input);
    assert_eq!(doc.line_ending, LineEnding::Crlf);
    assert_eq!(doc.render(), input);

    let mut doc = doc;
    doc.resolve(0, Resolution::Current);
    assert_eq!(doc.render(), "a\r\nx\r\nz\r\n");
}

#[test]
fn equals_underline_outside_conflict_is_plain_text() {
    // A markdown setext underline of `=` must NOT be mistaken for a separator.
    let input = "Title\n=======\nbody\n";
    let doc = MergeDocument::parse(input);
    assert_eq!(doc.conflict_count(), 0);
    assert_eq!(doc.render(), input);
}

#[test]
fn unterminated_conflict_is_kept_as_text() {
    // No closing marker: we must not lose the content.
    let input = "before\n<<<<<<< HEAD\ndangling\n";
    let doc = MergeDocument::parse(input);
    assert_eq!(doc.conflict_count(), 0);
    assert!(matches!(doc.sections.as_slice(), [Section::Stable(_)]));
    assert_eq!(doc.render(), input);
}

#[test]
fn empty_sides_are_handled() {
    // Deleting on one side yields an empty run.
    let input = "<<<<<<< HEAD\n=======\nadded\n>>>>>>> b\n";
    let mut doc = MergeDocument::parse(input);
    let c = doc.conflict(0).unwrap();
    assert!(c.current.is_empty());
    assert_eq!(c.incoming, vec!["added"]);

    doc.resolve(0, Resolution::Current);
    assert_eq!(doc.render(), "");
}
