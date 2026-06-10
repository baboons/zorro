//! Tests for token-level diffing.

use zorro_core::diff::{diff, line_diff_flags, DiffSpan, DiffTag, Granularity};

fn span(tag: DiffTag, text: &str) -> DiffSpan {
    DiffSpan {
        tag,
        text: text.to_string(),
    }
}

#[test]
fn identical_strings_are_all_equal() {
    let spans = diff("hello world", "hello world", Granularity::Word);
    assert_eq!(spans, vec![span(DiffTag::Equal, "hello world")]);
}

#[test]
fn word_diff_isolates_changed_word() {
    let spans = diff("the quick fox", "the slow fox", Granularity::Word);
    assert_eq!(
        spans,
        vec![
            span(DiffTag::Equal, "the "),
            span(DiffTag::Delete, "quick"),
            span(DiffTag::Insert, "slow"),
            span(DiffTag::Equal, " fox"),
        ]
    );
}

#[test]
fn word_diff_pure_insertion() {
    let spans = diff("a b", "a b c", Granularity::Word);
    assert_eq!(
        spans,
        vec![
            span(DiffTag::Equal, "a b"),
            span(DiffTag::Insert, " c"),
        ]
    );
}

#[test]
fn char_diff_highlights_single_char() {
    let spans = diff("color", "colour", Granularity::Char);
    assert_eq!(
        spans,
        vec![
            span(DiffTag::Equal, "colo"),
            span(DiffTag::Insert, "u"),
            span(DiffTag::Equal, "r"),
        ]
    );
}

#[test]
fn reconstructs_both_sides() {
    // The Equal+Delete spans must rebuild the left; Equal+Insert the right.
    let left = "func handleRequest(id int)";
    let right = "func handleRequest(id int, ctx Context)";
    let spans = diff(left, right, Granularity::Word);

    let rebuilt_left: String = spans
        .iter()
        .filter(|s| s.tag != DiffTag::Insert)
        .map(|s| s.text.as_str())
        .collect();
    let rebuilt_right: String = spans
        .iter()
        .filter(|s| s.tag != DiffTag::Delete)
        .map(|s| s.text.as_str())
        .collect();

    assert_eq!(rebuilt_left, left);
    assert_eq!(rebuilt_right, right);
}

fn lines(text: &str) -> Vec<String> {
    text.lines().map(|s| s.to_string()).collect()
}

#[test]
fn line_diff_marks_removed_and_added() {
    // left has B removed (no D), right adds D; A and C are common.
    let left = lines("A\nB\nC");
    let right = lines("A\nC\nD");
    let (removed, added) = line_diff_flags(&left, &right);
    // left: A common, B removed, C common
    assert_eq!(removed, vec![false, true, false]);
    // right: A common, C common, D added
    assert_eq!(added, vec![false, false, true]);
}

#[test]
fn line_diff_identical_blocks_have_no_flags() {
    let a = lines("x\ny\nz");
    let (removed, added) = line_diff_flags(&a, &a);
    assert!(removed.iter().all(|f| !f));
    assert!(added.iter().all(|f| !f));
}

#[test]
fn line_diff_disjoint_blocks_flag_everything() {
    let left = lines("a\nb");
    let right = lines("c\nd\ne");
    let (removed, added) = line_diff_flags(&left, &right);
    assert_eq!(removed, vec![true, true]);
    assert_eq!(added, vec![true, true, true]);
}

#[test]
fn handles_unicode_chars() {
    let spans = diff("café", "cafe", Granularity::Char);
    let rebuilt_left: String = spans
        .iter()
        .filter(|s| s.tag != DiffTag::Insert)
        .map(|s| s.text.as_str())
        .collect();
    assert_eq!(rebuilt_left, "café");
}
